//! Cone's radar (#85, D19, D135): the last four hours of NOAA reflectivity
//! around a person's radar, cached on disk and drawn behind the classic
//! windows, with the Weather Service's active alerts for its state.
//!
//! The network rules are the theme's, and they are strict. It fetches on a
//! ten-minute timer, only while Cone is the theme and a radar is picked,
//! never because something is playing, and never before the windows are up.
//! Every request goes through `egress::get`. A fetch that fails changes
//! nothing but the age the windows show: the cache is what they draw.
//!
//! The service sends pictures already coloured in NOAA's reflectivity scale.
//! Each frame is redrawn here in the analyser's own 24-step ramp, so heavy
//! rain and bass are the same magenta (theme.md, D20): every colour the
//! service uses maps back to a reflectivity (`radar_colours.json`, read off
//! its frames), and that to a step of the ramp. Light returns under 15 dBZ,
//! the drizzle and clear-air haze, are left transparent.

use crate::egress;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Which radar a person picked, by its id (`KJAX`). Unset until they pick.
pub const SITE_SETTING: &str = "radar.site";

/// `design/tokens.json` → `themes.cone.radar`: how often to fetch, and how
/// many frames make the loop. A test holds these to the tokens.
pub const REFRESH_SECS: u64 = 600;
pub const CACHE_FRAMES: usize = 24;

/// The loop's length, and the least time between two of its frames. The
/// service publishes a frame every six to eight minutes; the loop keeps one
/// about every ten.
const WINDOW_MS: i64 = 4 * 60 * 60 * 1000;
const SPACING_MS: i64 = 9 * 60 * 1000;

const SERVICE: &str =
    "https://mapservices.weather.noaa.gov/eventdriven/rest/services/radar/radar_base_reflectivity_time/ImageServer";

/// A frame's size in pixels: the three classic windows stacked, Main and the
/// EQ at 116 and a playlist 290 tall, at 2x.
pub const FRAME_W: u32 = 550;
pub const FRAME_H: u32 = 1044;
/// How far down the stack the radar sits, in logical pixels: the middle of
/// the three windows at their base sizes, so it is in view.
pub const SITE_Y: f64 = 188.0;
/// Ground distance per frame pixel: a frame is about 460 km across, twice
/// the radar's own reach.
const KM_PER_PX: f64 = 0.84;

/// Reflectivity below which nothing is drawn, and at which the ramp tops out.
const FLOOR_DBZ: f32 = 15.0;
const TOP_DBZ: f32 = 70.0;
/// The ramp step light rain starts on. The analyser's first steps are its
/// quietest bars, nearly black, and rain drawn in them vanished into the
/// window: radar starts where the ramp is plainly green.
const FIRST_STEP: usize = 6;

/// One WSR-88D radar the picker offers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Site {
    pub id: String,
    pub name: String,
    /// Two letters, for the alerts area.
    pub state: String,
    pub lat: f64,
    pub lon: f64,
    /// Which of the service's mosaics covers it.
    pub region: String,
}

/// Every radar the picker offers, ordered by state and name. Read from the
/// National Weather Service's station list once, with each site's state.
/// The three overseas military sites are left out: no mosaic covers them.
pub fn sites() -> Vec<Site> {
    serde_json::from_str(include_str!("radar_sites.json")).expect("radar_sites.json is valid")
}

pub fn site(id: &str) -> Option<Site> {
    sites().into_iter().find(|s| s.id == id)
}

/// The frame's box in Web Mercator metres, `[xmin, ymin, xmax, ymax]`: the
/// radar centred across, `SITE_Y` down the stack.
pub fn bbox(site: &Site) -> [f64; 4] {
    const R: f64 = 6_378_137.0;
    let lat = site.lat.to_radians();
    let x = R * site.lon.to_radians();
    let y = R * (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln();
    // Mercator stretches distance by 1/cos(latitude).
    let per_px = KM_PER_PX * 1000.0 / lat.cos();
    let half_w = f64::from(FRAME_W) / 2.0 * per_px;
    let above = SITE_Y * 2.0 * per_px;
    let below = (f64::from(FRAME_H) - SITE_Y * 2.0) * per_px;
    [x - half_w, y - below, x + half_w, y + above]
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn catalog_url(region: &str) -> String {
    format!(
        "{SERVICE}/query?where={}&outFields=idp_validtime&returnGeometry=false&orderByFields=idp_validtime&f=json",
        encode(&format!("name LIKE '{region}%'"))
    )
}

fn frame_url(site: &Site, time_ms: i64) -> String {
    let [a, b, c, d] = bbox(site);
    format!(
        "{SERVICE}/exportImage?bbox={a:.0},{b:.0},{c:.0},{d:.0}&bboxSR=3857&imageSR=3857&size={FRAME_W},{FRAME_H}\
         &format=png32&transparent=true&time={time_ms}&interpolation=RSP_NearestNeighbor&f=image"
    )
}

fn alerts_url(state: &str) -> String {
    format!(
        "https://api.weather.gov/alerts/active?area={}",
        encode(state)
    )
}

/// The frame times the service has for a mosaic, oldest first.
fn times_from(v: &serde_json::Value) -> Vec<i64> {
    let mut t: Vec<i64> = v
        .get("features")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    f.pointer("/attributes/idp_validtime")
                        .and_then(|x| x.as_i64())
                })
                .collect()
        })
        .unwrap_or_default();
    t.sort_unstable();
    t.dedup();
    t
}

/// Which of the service's frames to fetch: the newest first, each at least
/// `SPACING_MS` from every frame kept or cached, within `WINDOW_MS` of the
/// newest, no more than `CACHE_FRAMES` in all. Oldest first.
fn pick_frames(available: &[i64], cached: &[i64]) -> Vec<i64> {
    let Some(newest) = available.iter().chain(cached).max().copied() else {
        return Vec::new();
    };
    let start = newest - WINDOW_MS;
    let mut kept: Vec<i64> = cached.iter().copied().filter(|t| *t >= start).collect();
    let mut fetch = Vec::new();
    for &t in available.iter().rev().filter(|t| **t >= start) {
        if kept.iter().all(|k| (k - t).abs() >= SPACING_MS) {
            kept.push(t);
            fetch.push(t);
        }
    }
    // Only as many as the loop holds, the newest.
    kept.sort_unstable_by(|a, b| b.cmp(a));
    let keep: Vec<i64> = kept.into_iter().take(CACHE_FRAMES).collect();
    let mut fetch: Vec<i64> = fetch.into_iter().filter(|t| keep.contains(t)).collect();
    fetch.sort_unstable();
    fetch
}

/// Which cached frames have fallen out of the loop.
fn stale_frames(cached: &[i64]) -> Vec<i64> {
    let Some(newest) = cached.iter().max().copied() else {
        return Vec::new();
    };
    let mut by_age: Vec<i64> = cached.to_vec();
    by_age.sort_unstable_by(|a, b| b.cmp(a));
    by_age
        .iter()
        .enumerate()
        .filter(|(i, t)| *i >= CACHE_FRAMES || **t < newest - WINDOW_MS)
        .map(|(_, t)| *t)
        .collect()
}

/// The analyser's ramp, from the tokens: Cone draws Eyewall's.
fn ramp() -> [[u8; 3]; 24] {
    let tokens: serde_json::Value = serde_json::from_str(include_str!("../../design/tokens.json"))
        .expect("tokens.json is valid");
    let list = tokens
        .pointer("/themes/eyewall/visualizer/palette")
        .and_then(|p| p.as_array())
        .expect("Eyewall has a ramp");
    let mut out = [[0u8; 3]; 24];
    for (i, hex) in list.iter().take(24).enumerate() {
        out[i] = rgb(hex.as_str().unwrap_or("#000000")).unwrap_or([0, 0, 0]);
    }
    out
}

fn rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// The service's colours back to reflectivity, and reflectivity to the ramp.
struct Recolour {
    table: Vec<([u8; 3], f32)>,
    ramp: [[u8; 3]; 24],
    seen: HashMap<[u8; 3], Option<usize>>,
}

impl Recolour {
    fn new() -> Self {
        let raw: Vec<(String, f32)> = serde_json::from_str(include_str!("radar_colours.json"))
            .expect("radar_colours.json is valid");
        let table = raw
            .iter()
            .filter_map(|(h, d)| rgb(h).map(|c| (c, *d)))
            .collect();
        Recolour {
            table,
            ramp: ramp(),
            seen: HashMap::new(),
        }
    }

    /// The ramp step a service colour stands for, None below the floor. A
    /// colour not in the table (an edge the service smoothed) takes its
    /// nearest neighbour's reflectivity.
    fn step(&mut self, c: [u8; 3]) -> Option<usize> {
        if let Some(s) = self.seen.get(&c) {
            return *s;
        }
        let d2 = |t: &[u8; 3]| {
            t.iter()
                .zip(c.iter())
                .map(|(a, b)| (i32::from(*a) - i32::from(*b)).pow(2))
                .sum::<i32>()
        };
        let dbz = self
            .table
            .iter()
            .min_by_key(|(t, _)| d2(t))
            .map(|(_, d)| *d)
            .unwrap_or(-99.0);
        let s = step_for(dbz);
        self.seen.insert(c, s);
        s
    }

    /// A frame from the service, redrawn: the same size, transparent where
    /// there is nothing to show, the ramp's colour everywhere else.
    fn frame(&mut self, png_bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().map_err(|e| format!("not a picture: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("picture too large")?];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("picture would not decode: {e}"))?;
        let channels = match info.color_type {
            png::ColorType::Rgba => 4,
            png::ColorType::Rgb => 3,
            other => return Err(format!("unexpected picture format {other:?}")),
        };
        let (w, h) = (info.width, info.height);
        let mut out = vec![0u8; (w * h * 4) as usize];
        for (i, px) in buf[..(w * h) as usize * channels]
            .chunks_exact(channels)
            .enumerate()
        {
            if channels == 4 && px[3] == 0 {
                continue;
            }
            if let Some(s) = self.step([px[0], px[1], px[2]]) {
                let c = self.ramp[s];
                let o = &mut out[i * 4..i * 4 + 4];
                o[..3].copy_from_slice(&c);
                // The faintest steps are the most see-through.
                o[3] = (150 + 105 * s / 23) as u8;
            }
        }
        let mut png_out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().map_err(|e| e.to_string())?;
            wr.write_image_data(&out).map_err(|e| e.to_string())?;
        }
        Ok(png_out)
    }
}

/// Reflectivity to a step of the 24-step ramp, None under the floor.
fn step_for(dbz: f32) -> Option<usize> {
    if dbz < FLOOR_DBZ {
        return None;
    }
    let t = ((dbz - FLOOR_DBZ) / (TOP_DBZ - FLOOR_DBZ)).clamp(0.0, 1.0);
    Some(FIRST_STEP + (t * (23 - FIRST_STEP) as f32).round() as usize)
}

/// An active watch, warning or advisory, as the Weather Service states it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alert {
    pub event: String,
    pub severity: String,
    pub headline: Option<String>,
    /// When it was issued, and when it runs out, as the service writes them.
    pub sent: Option<String>,
    pub expires: Option<String>,
}

/// The alerts in a Weather Service answer, the most severe first, then the
/// most recently issued.
fn alerts_from(v: &serde_json::Value) -> Vec<Alert> {
    let text =
        |p: &serde_json::Value, k: &str| p.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let mut out: Vec<Alert> = v
        .get("features")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|f| f.get("properties"))
                .filter_map(|p| {
                    Some(Alert {
                        event: text(p, "event")?,
                        severity: text(p, "severity").unwrap_or_else(|| "Unknown".into()),
                        headline: text(p, "headline"),
                        sent: text(p, "sent"),
                        expires: text(p, "expires"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let rank = |s: &str| match s {
        "Extreme" => 0,
        "Severe" => 1,
        "Moderate" => 2,
        "Minor" => 3,
        _ => 4,
    };
    out.sort_by(|a, b| {
        rank(&a.severity)
            .cmp(&rank(&b.severity))
            .then(b.sent.cmp(&a.sent))
    });
    out
}

// ---- disk -------------------------------------------------------------------

fn root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("radar"))
        .map_err(|e| format!("no app data dir: {e}"))
}

fn site_dir(app: &AppHandle, site: &Site) -> Result<PathBuf, String> {
    let dir = root(app)?.join(&site.id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make the radar folder: {e}"))?;
    Ok(dir)
}

fn cached_times(dir: &Path) -> Vec<i64> {
    let mut t: Vec<i64> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    name.strip_suffix(".png")?.parse().ok()
                })
                .collect()
        })
        .unwrap_or_default();
    t.sort_unstable();
    t
}

/// What the last attempt did, kept on disk so an app started with no
/// connection still knows how old its loop is and that it could not refresh.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Record {
    last_attempt_ms: Option<i64>,
    last_ok_ms: Option<i64>,
    last_error: Option<String>,
    alerts_ms: Option<i64>,
    alerts: Vec<Alert>,
}

fn record_path(app: &AppHandle, site: &Site) -> Result<PathBuf, String> {
    Ok(site_dir(app, site)?.join("record.json"))
}

fn read_record(app: &AppHandle, site: &Site) -> Record {
    record_path(app, site)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_record(app: &AppHandle, site: &Site, r: &Record) {
    if let (Ok(p), Ok(json)) = (record_path(app, site), serde_json::to_vec_pretty(r)) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---- what the windows see ---------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    pub time_ms: i64,
    /// Absolute path; the webview turns it into an asset URL.
    pub path: String,
}

/// Everything a window needs to draw Cone and say how old it is.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub site: Option<Site>,
    pub frames: Vec<Frame>,
    pub last_attempt_ms: Option<i64>,
    pub last_ok_ms: Option<i64>,
    pub last_error: Option<String>,
    pub alerts: Vec<Alert>,
    pub alerts_ms: Option<i64>,
    pub frame_size: [u32; 2],
    pub site_y: f64,
}

pub fn current_site(app: &AppHandle) -> Option<Site> {
    let state = app.state::<crate::Db>();
    let id = {
        let conn = state.0.lock().unwrap();
        crate::db::get_setting(&conn, SITE_SETTING)
    }?;
    site(&id)
}

pub fn status(app: &AppHandle) -> Status {
    let site = current_site(app);
    let (frames, rec) = match &site {
        Some(s) => {
            let dir = site_dir(app, s).ok();
            let frames = dir
                .as_ref()
                .map(|d| {
                    cached_times(d)
                        .into_iter()
                        .map(|t| Frame {
                            time_ms: t,
                            path: d.join(format!("{t}.png")).to_string_lossy().into_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            (frames, read_record(app, s))
        }
        None => (Vec::new(), Record::default()),
    };
    Status {
        site,
        frames,
        last_attempt_ms: rec.last_attempt_ms,
        last_ok_ms: rec.last_ok_ms,
        last_error: rec.last_error,
        alerts: rec.alerts,
        alerts_ms: rec.alerts_ms,
        frame_size: [FRAME_W, FRAME_H],
        site_y: SITE_Y,
    }
}

// ---- the fetch --------------------------------------------------------------

/// One refresh: the frames the loop is missing, the old ones dropped, then
/// the alerts. Written to a scratch name and renamed, so a window never
/// reads half a frame.
async fn refresh(app: &AppHandle, site: &Site) -> Result<(), String> {
    let dir = site_dir(app, site)?;
    let catalog = egress::get(&catalog_url(&site.region)).await?;
    let available = times_from(
        &serde_json::from_slice(&catalog)
            .map_err(|e| format!("the radar service's list was not JSON: {e}"))?,
    );
    let wanted = pick_frames(&available, &cached_times(&dir));
    let mut recolour = Recolour::new();
    for t in wanted {
        let bytes = egress::get(&frame_url(site, t)).await?;
        if !bytes.starts_with(b"\x89PNG") {
            return Err("the radar service sent something other than a picture".into());
        }
        let out = recolour.frame(&bytes)?;
        let tmp = dir.join(format!("{t}.part"));
        std::fs::write(&tmp, out).map_err(|e| format!("could not save a radar frame: {e}"))?;
        std::fs::rename(&tmp, dir.join(format!("{t}.png")))
            .map_err(|e| format!("could not save a radar frame: {e}"))?;
    }
    for t in stale_frames(&cached_times(&dir)) {
        let _ = std::fs::remove_file(dir.join(format!("{t}.png")));
    }
    Ok(())
}

async fn refresh_alerts(site: &Site) -> Result<Vec<Alert>, String> {
    let body = egress::get(&alerts_url(&site.state)).await?;
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("the alerts were not JSON: {e}"))?;
    Ok(alerts_from(&v))
}

/// Wakes the timer early: a radar was picked, or Cone was put on.
#[derive(Default)]
pub struct Wake(pub Arc<tokio::sync::Notify>);

pub fn wake(app: &AppHandle) {
    app.state::<Wake>().0.notify_one();
}

fn wanted(app: &AppHandle) -> Option<Site> {
    let (theme, precaching) = {
        let state = app.state::<crate::Db>();
        let conn = state.0.lock().unwrap();
        (crate::db::theme(&conn), crate::prep::precaching(&conn))
    };
    // A prep run fills the loop whatever the theme, so it is there when the
    // lights go out (#163, D140). Still only with a radar picked, still on
    // the ten-minute timer, still through `egress::get` (D29).
    if theme == "cone" || precaching {
        current_site(app)
    } else {
        None
    }
}

/// The timer. Started once the windows are up; does nothing at all while
/// another theme is on or no radar is picked.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let wake = app.state::<Wake>().0.clone();
        // The windows draw the cache first (theme.md): no request before
        // they have had a moment to come up.
        tokio::time::sleep(Duration::from_secs(3)).await;
        loop {
            if let Some(site) = wanted(&app) {
                let mut rec = read_record(&app, &site);
                rec.last_attempt_ms = Some(now_ms());
                match refresh(&app, &site).await {
                    Ok(()) => {
                        rec.last_ok_ms = rec.last_attempt_ms;
                        rec.last_error = None;
                    }
                    Err(e) => rec.last_error = Some(e),
                }
                if let Ok(alerts) = refresh_alerts(&site).await {
                    rec.alerts = alerts;
                    rec.alerts_ms = Some(now_ms());
                }
                write_record(&app, &site, &rec);
                let _ = app.emit("radar:updated", status(&app));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(REFRESH_SECS)) => {}
                _ = wake.notified() => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_radar_has_an_id_a_state_and_a_mosaic() {
        let all = sites();
        assert!(all.len() > 150, "{}", all.len());
        let mut ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "ids are unique");
        for s in &all {
            assert_eq!(s.state.len(), 2, "{s:?}");
            assert!(
                ["CONUS", "ALASKA", "HAWAII", "CARIB", "GUAM"].contains(&s.region.as_str()),
                "{s:?}"
            );
            assert!(
                (-90.0..90.0).contains(&s.lat) && (-180.0..180.0).contains(&s.lon),
                "{s:?}"
            );
        }
        assert_eq!(site("KJAX").map(|s| s.state), Some("FL".into()));
        assert_eq!(site("TJUA").map(|s| s.region), Some("CARIB".into()));
    }

    #[test]
    fn the_refresh_and_the_loop_are_the_tokens() {
        let tokens: serde_json::Value =
            serde_json::from_str(include_str!("../../design/tokens.json")).unwrap();
        let radar = tokens.pointer("/themes/cone/radar").unwrap();
        assert_eq!(radar["refreshSeconds"].as_u64(), Some(REFRESH_SECS));
        assert_eq!(radar["cacheFrames"].as_u64(), Some(CACHE_FRAMES as u64));
        assert_eq!(radar["blockStartup"].as_bool(), Some(false));
    }

    #[test]
    fn the_frame_is_about_460_km_across_with_the_radar_in_view() {
        let jax = site("KJAX").unwrap();
        let [x0, y0, x1, y1] = bbox(&jax);
        let across_km = (x1 - x0) * jax.lat.to_radians().cos() / 1000.0;
        assert!((across_km - 462.0).abs() < 1.0, "{across_km}");
        // The radar is SITE_Y logical (2x pixels) from the top.
        let per_px = (y1 - y0) / f64::from(FRAME_H);
        let y = 6_378_137.0
            * (std::f64::consts::FRAC_PI_4 + jax.lat.to_radians() / 2.0)
                .tan()
                .ln();
        assert!(((y1 - y) / per_px - SITE_Y * 2.0).abs() < 0.5);
        assert!(((x0 + x1) / 2.0 - 6_378_137.0 * jax.lon.to_radians()).abs() < 1.0);
    }

    #[test]
    fn frames_are_about_ten_minutes_apart_within_four_hours_and_no_more_than_the_loop() {
        let m = 60_000;
        // The service's cadence: every six to eight minutes for two hours.
        let mut t = 0;
        let mut available = Vec::new();
        for i in 0..18 {
            t += if i % 2 == 0 { 6 * m } else { 8 * m };
            available.push(t);
        }
        let first = pick_frames(&available, &[]);
        assert!(!first.is_empty());
        for pair in first.windows(2) {
            assert!(pair[1] - pair[0] >= SPACING_MS, "{pair:?}");
        }
        assert_eq!(
            *first.last().unwrap(),
            *available.last().unwrap(),
            "the newest is always taken"
        );
        // Nothing new: nothing to fetch.
        assert!(pick_frames(&available, &first).is_empty());
        // One newer frame, far enough on: just that one.
        let later = available.last().unwrap() + 10 * m;
        assert_eq!(pick_frames(&[later], &first), vec![later]);
        // Too close to a cached one: skipped.
        assert!(pick_frames(&[later + 3 * m], &[later]).is_empty());
        // Frames past four hours are not fetched, and the loop never grows
        // past CACHE_FRAMES.
        let long: Vec<i64> = (0..60).map(|i| i * 10 * m).collect();
        let picked = pick_frames(&long, &[]);
        assert!(picked.len() <= CACHE_FRAMES);
        assert!(picked
            .iter()
            .all(|t| *t >= long.last().unwrap() - WINDOW_MS));
    }

    #[test]
    fn frames_past_the_loop_are_dropped() {
        let m = 60_000;
        let cached: Vec<i64> = (0..30).map(|i| i * 10 * m).collect();
        let gone = stale_frames(&cached);
        let newest = *cached.last().unwrap();
        for t in &cached {
            let kept = !gone.contains(t);
            let rank = cached.iter().filter(|x| *x > t).count();
            assert_eq!(kept, rank < CACHE_FRAMES && *t >= newest - WINDOW_MS, "{t}");
        }
        assert!(stale_frames(&[]).is_empty());
    }

    #[test]
    fn reflectivity_maps_onto_the_analysers_ramp() {
        assert_eq!(step_for(5.0), None, "drizzle and haze are not drawn");
        assert_eq!(step_for(15.0), Some(FIRST_STEP));
        assert_eq!(step_for(70.0), Some(23));
        assert_eq!(step_for(90.0), Some(23));
        let mut r = Recolour::new();
        // The service's own colours: a blue is light, a green is rain, red
        // is heavy rain, white is the top of the scale.
        let blue = r.step([0x46, 0x66, 0xA4]);
        let green = r.step([0x0E, 0xD6, 0x14]).unwrap();
        let yellow = r.step([0xFF, 0xDD, 0x00]).unwrap();
        let red = r.step([0xFF, 0x00, 0x00]).unwrap();
        let white = r.step([0xFF, 0xFF, 0xFF]).unwrap();
        assert_eq!(blue, None);
        assert!(
            green < yellow && yellow < red && red <= white,
            "{green} {yellow} {red} {white}"
        );
        // A colour the table does not list takes its nearest neighbour's.
        assert_eq!(r.step([0xFE, 0x01, 0x01]), Some(red));
        // The ramp is the tokens'.
        assert_eq!(r.ramp[23], rgb("#FF7BEC").unwrap());
    }

    #[test]
    fn a_frame_is_redrawn_in_the_ramp_and_nothing_else_is_kept() {
        // A 3x1 picture: transparent, a heavy-rain red, a light blue.
        let mut src = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut src, 3, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 0, 0, 0, 255, 0, 0, 255, 0x46, 0x66, 0xA4, 255])
                .unwrap();
        }
        let mut r = Recolour::new();
        let out = r.frame(&src).unwrap();
        let mut dec = png::Decoder::new(std::io::Cursor::new(out));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut buf).unwrap();
        assert_eq!(buf[3], 0, "transparent stays transparent");
        let step = r.step([255, 0, 0]).unwrap();
        assert_eq!(
            &buf[4..7],
            &r.ramp[step],
            "red becomes the ramp's colour for it"
        );
        assert!(buf[7] > 150);
        assert_eq!(buf[11], 0, "light returns are left out");
        assert!(r.frame(b"not a png").is_err());
    }

    #[test]
    fn alerts_are_read_most_severe_first() {
        let v = serde_json::json!({"features": [
            {"properties": {"event": "Flood Watch", "severity": "Moderate", "sent": "2026-09-13T10:00:00-04:00"}},
            {"properties": {"event": "Hurricane Warning", "severity": "Extreme", "sent": "2026-09-13T09:00:00-04:00",
                            "headline": "Hurricane Warning issued", "expires": "2026-09-14T09:00:00-04:00"}},
            {"properties": {"severity": "Minor"}}
        ]});
        let a = alerts_from(&v);
        assert_eq!(a.len(), 2, "an alert with no event is not one");
        assert_eq!(a[0].event, "Hurricane Warning");
        assert_eq!(a[0].sent.as_deref(), Some("2026-09-13T09:00:00-04:00"));
        assert_eq!(a[1].event, "Flood Watch");
        assert!(alerts_from(&serde_json::json!({})).is_empty());
    }

    /// The real service, once: the newest frame around a radar, redrawn, and
    /// written to the temp folder to look at. Needs the network, so it only
    /// runs when asked: `cargo test live_frame -- --ignored`.
    #[test]
    #[ignore]
    fn live_frame() {
        tauri::async_runtime::block_on(async {
            let site = site(&std::env::var("HP_RADAR").unwrap_or_else(|_| "KJAX".into())).unwrap();
            let catalog = egress::get(&catalog_url(&site.region)).await.unwrap();
            let times = times_from(&serde_json::from_slice(&catalog).unwrap());
            assert!(times.len() > 5, "{} frames listed", times.len());
            let picked = pick_frames(&times, &[]);
            let newest = *picked.last().unwrap();
            let bytes = egress::get(&frame_url(&site, newest)).await.unwrap();
            let out = Recolour::new().frame(&bytes).unwrap();
            let dir = std::env::temp_dir().join("hp-radar-live");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("source.png"), &bytes).unwrap();
            std::fs::write(dir.join("redrawn.png"), &out).unwrap();
            // With HP_RADAR_DIR, the whole loop the first refresh would fetch.
            if let Ok(loop_dir) = std::env::var("HP_RADAR_DIR") {
                let mut r = Recolour::new();
                for t in &picked {
                    let b = egress::get(&frame_url(&site, *t)).await.unwrap();
                    std::fs::write(
                        Path::new(&loop_dir).join(format!("{t}.png")),
                        r.frame(&b).unwrap(),
                    )
                    .unwrap();
                }
            }
            let alerts = egress::get(&alerts_url(&site.state)).await.unwrap();
            let alerts = alerts_from(&serde_json::from_slice(&alerts).unwrap());
            eprintln!(
                "{} frames to fetch of {}, {} alerts, written to {}",
                picked.len(),
                times.len(),
                alerts.len(),
                dir.display()
            );
        });
    }

    #[test]
    fn the_urls_go_only_where_egress_allows() {
        let jax = site("KJAX").unwrap();
        for u in [
            catalog_url("CONUS"),
            frame_url(&jax, 1_789_316_528_000),
            alerts_url("FL"),
        ] {
            assert!(egress::allowed(&u).is_ok(), "{u}");
        }
        assert!(catalog_url("CONUS").contains("name+LIKE+%27CONUS%25%27"));
    }
}
