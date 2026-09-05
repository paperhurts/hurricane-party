//! The viz hub: the Main window's analyser in, one binary pipe per subscriber
//! out (`docs/control-api.md`, D15).
//!
//! The analyser lives in the webview (D5), so the spectrum crosses the IPC
//! boundary once per source frame: Main reads its stream analyser at the
//! highest rate any subscriber asked for and invokes `viz_frame` with the raw
//! FFT bins as the request body, bytes rather than JSON. This side maps the
//! bins onto each subscriber's bands, encodes the frame (`hp_control::viz`),
//! and hands it to that subscriber's writer task.
//!
//! **Drop, never buffer.** Each writer sits on a `watch` channel that holds
//! only the newest frame, and its pipe's outbound buffer is two frames deep,
//! so a client that stalls sees a gap and then the present, never a replay.
//!
//! **Demand.** Capturing at 60 Hz for nobody is waste, so the hub tells Main
//! when to start and stop (`viz:capture`) and Main asks on mount
//! (`viz_demand`), which is what keeps a reload of the Main window from
//! silently ending a rig's stream.

use hp_control::{viz, Frame, VizParams};
use serde::Serialize;
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::http::HeaderMap;
use tauri::ipc::InvokeBody;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::platform::pipe;

/// How long a subscriber has to open its pipe after `subscribe_viz` replies.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The band range, the same one the on-screen bars use (`spectrum.ts`).
const F_MIN: f64 = 50.0;
const F_MAX: f64 = 16000.0;

/// The bins the beat detector listens to: kick and bass.
const BEAT_LO_HZ: f64 = 40.0;
const BEAT_HI_HZ: f64 = 160.0;

/// What the source should be doing. Sent to Main as `viz:capture` whenever it
/// changes; answered by `viz_demand` when Main mounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Demand {
    pub active: bool,
    pub rate_hz: u32,
}

/// One frame from the source, as `viz_frame` received it.
pub struct SourceFrame<'a> {
    /// `getByteFrequencyData` of the stream analyser: linear bins, 0..255.
    pub bins: &'a [u8],
    pub sample_rate: u32,
    /// Microseconds since the epoch on the webview's clock, when read.
    pub timestamp_us: u64,
    pub level_peak: u8,
    pub level_rms: u8,
    /// Diagnostics, for the trace only: ticks the source dropped so far
    /// because the previous frame's IPC had not returned, and how late this
    /// tick ran after it was due.
    pub source_dropped: u64,
    pub late_ms: f32,
    /// Also diagnostics: the source's running frame number, how many of its
    /// IPC calls have rejected, and what the last rejection said.
    pub seq: u64,
    pub source_failed: u64,
    pub source_err: String,
    /// `AudioContext.outputLatency + baseLatency` in ms: how far ahead of
    /// the speaker the analyser reads. The doc's speaker-side number is the
    /// pipe latency minus this.
    pub output_latency_ms: f32,
}

/// The shape of the source a band mapping was computed for: (bins, rate).
type SourceShape = (usize, u32);
/// Half-open bin ranges, one per band.
type Edges = Vec<(u16, u16)>;

struct Sub {
    id: u32,
    params: VizParams,
    tx: watch::Sender<Arc<Vec<u8>>>,
    /// Band edges for the source shape last seen; recomputed if it changes.
    edges: Option<(SourceShape, Edges)>,
    scratch: Vec<f32>,
    out: Vec<u8>,
}

struct Inner {
    subs: Vec<Sub>,
    next_id: u32,
    tick: u64,
    demand: Demand,
    beat: BeatDetector,
    trace: Option<Trace>,
}

/// Managed state. One per app.
pub struct VizHub(Arc<Mutex<Inner>>);

impl Default for VizHub {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            subs: Vec::new(),
            next_id: 1,
            tick: 0,
            demand: Demand::default(),
            beat: BeatDetector::default(),
            // HP_VIZ_TRACE=1 prints the webview-to-Rust hop and the source
            // cadence once a second: the harness behind #7's number.
            trace: std::env::var_os("HP_VIZ_TRACE").map(|_| Trace::default()),
        })))
    }
}

fn compute_demand(inner: &Inner) -> Demand {
    Demand {
        active: !inner.subs.is_empty(),
        rate_hz: inner
            .subs
            .iter()
            .map(|s| s.params.rate_hz)
            .max()
            .unwrap_or(0),
    }
}

/// Re-derive demand from the subscriber list and tell Main if it changed.
fn notify_demand(app: &AppHandle, hub: &Arc<Mutex<Inner>>) {
    let (changed, d) = {
        let mut g = hub.lock().unwrap();
        let d = compute_demand(&g);
        let changed = d != g.demand;
        g.demand = d;
        if changed && !d.active {
            g.beat.reset();
            if let Some(t) = &mut g.trace {
                t.reset();
            }
        }
        (changed, d)
    };
    if changed {
        eprintln!(
            "hp-viz: capture {} at {} Hz",
            if d.active { "on" } else { "off" },
            d.rate_hz
        );
        let _ = app.emit_to("main", "viz:capture", d);
    }
}

/// What Main should be doing right now.
pub fn demand(app: &AppHandle) -> Demand {
    app.state::<VizHub>().0.lock().unwrap().demand
}

/// `subscribe_viz`: create the subscriber's pipe, start its writer, and
/// return the pipe's name. The pipe exists before the reply goes out, so the
/// client never finds nothing listening (control-api.md).
pub fn subscribe(app: &AppHandle, params: VizParams) -> Result<String, String> {
    let hub = app.state::<VizHub>().0.clone();
    let (id, name) = {
        let mut g = hub.lock().unwrap();
        let id = g.next_id;
        g.next_id = g.next_id.wrapping_add(1).max(1);
        (id, viz::pipe_name(id))
    };

    let max_frame = viz::HEADER_LEN + params.bands as usize * params.depth.bytes_per_band();
    let listener = pipe::listen(
        &name,
        pipe::ListenOptions {
            out_buffer: (2 * max_frame) as u32,
        },
    )
    .map_err(|e| format!("can't create viz pipe: {e}"))?;

    let (tx, mut rx) = watch::channel(Arc::new(Vec::new()));
    hub.lock().unwrap().subs.push(Sub {
        id,
        params,
        tx,
        edges: None,
        scratch: Vec::new(),
        out: Vec::new(),
    });
    notify_demand(app, &hub);
    eprintln!(
        "hp-viz: #{id:04x} subscribed: {} bands at {} Hz, {:?}",
        params.bands, params.rate_hz, params.depth
    );

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match tokio::time::timeout(CONNECT_TIMEOUT, listener.accept()).await {
            Ok(Ok(mut conn)) => {
                eprintln!("hp-viz: #{id:04x} connected");
                loop {
                    // The sender goes away when the hub drops the subscriber.
                    if rx.changed().await.is_err() {
                        break;
                    }
                    let frame = rx.borrow_and_update().clone();
                    if frame.is_empty() {
                        continue;
                    }
                    // A stalled client parks us here; meanwhile the watch
                    // keeps only the newest frame, which is the drop policy.
                    if conn.write_all(&frame).await.is_err() {
                        break;
                    }
                }
                eprintln!("hp-viz: #{id:04x} gone");
            }
            Ok(Err(e)) => eprintln!("hp-viz: #{id:04x} accept failed: {e}"),
            Err(_) => eprintln!("hp-viz: #{id:04x} never connected; dropped"),
        }
        hub.lock().unwrap().subs.retain(|s| s.id != id);
        notify_demand(&app, &hub);
    });

    Ok(name)
}

/// The `viz_frame` command body: headers for the scalars, raw bytes for the
/// bins. JSON would turn 1 KB of bytes into 4 KB of digits sixty times a
/// second, on the one path where latency is the product.
pub fn on_request(app: &AppHandle, req: &tauri::ipc::Request<'_>) -> Result<(), String> {
    fn hdr<T: FromStr>(h: &HeaderMap, name: &str) -> Result<T, String> {
        h.get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("viz_frame: bad or missing header {name}"))
    }
    let bins = match req.body() {
        InvokeBody::Raw(b) => b.as_slice(),
        InvokeBody::Json(_) => return Err("viz_frame: expected a raw body".into()),
    };
    let h = req.headers();
    on_frame(
        app,
        SourceFrame {
            bins,
            sample_rate: hdr(h, "x-hp-rate")?,
            timestamp_us: hdr(h, "x-hp-ts")?,
            level_peak: hdr(h, "x-hp-peak")?,
            level_rms: hdr(h, "x-hp-rms")?,
            source_dropped: hdr(h, "x-hp-dropped").unwrap_or(0),
            late_ms: hdr(h, "x-hp-late").unwrap_or(0.0),
            seq: hdr(h, "x-hp-seq").unwrap_or(0),
            source_failed: hdr(h, "x-hp-failed").unwrap_or(0),
            source_err: h
                .get("x-hp-err")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string(),
            output_latency_ms: hdr(h, "x-hp-outlat").unwrap_or(0.0),
        },
    );
    Ok(())
}

/// One source frame in; one encoded frame out to every subscriber due one.
pub fn on_frame(app: &AppHandle, src: SourceFrame<'_>) {
    let hub = app.state::<VizHub>();
    let mut g = hub.0.lock().unwrap();
    let g = &mut *g;
    if !g.demand.active {
        // The tail of a loop that has already been told to stop.
        return;
    }
    g.tick = g.tick.wrapping_add(1);
    let tick = g.tick;
    let source_rate = g.demand.rate_hz.max(1);

    let beat = g
        .beat
        .feed(low_energy(src.bins, src.sample_rate), source_rate);
    if let Some(t) = &mut g.trace {
        t.record(&src);
    }

    for s in &mut g.subs {
        // A 15 Hz subscriber under a 60 Hz source takes every fourth frame.
        let every = (source_rate / s.params.rate_hz.max(1)).max(1) as u64;
        if tick % every != 0 {
            continue;
        }
        let key = (src.bins.len(), src.sample_rate);
        if s.edges.as_ref().map(|(k, _)| *k != key).unwrap_or(true) {
            s.edges = band_edges(
                s.params.bands as usize,
                src.bins.len(),
                src.sample_rate as f64,
                F_MIN,
                F_MAX,
            )
            .ok()
            .map(|e| (key, e));
        }
        let inc = s.params.include;
        s.scratch.clear();
        if inc.spectrum {
            if let Some((_, edges)) = &s.edges {
                reduce_bands(src.bins, edges, &mut s.scratch);
            }
        }
        let frame = Frame {
            timestamp_us: src.timestamp_us,
            depth: s.params.depth,
            beat: inc.beat && beat,
            level_peak: if inc.level { src.level_peak } else { 0 },
            level_rms: if inc.level { src.level_rms } else { 0 },
            spectrum: std::mem::take(&mut s.scratch),
        };
        frame.encode_into(&mut s.out);
        s.scratch = frame.spectrum;
        s.tx.send_replace(Arc::new(s.out.clone()));
    }
}

// ---- the arithmetic, pure and tested --------------------------------------

/// Log-spaced bands over `f_min..f_max` Hz mapped onto `bin_count` FFT bins.
/// A port of `bandEdges` in `src/lib/spectrum.ts`, so a 19-band subscriber
/// sees the bars the Main window draws. Every band gets at least one bin,
/// the bands are contiguous, and bin 0 (DC) is skipped.
pub fn band_edges(
    bars: usize,
    bin_count: usize,
    sample_rate: f64,
    f_min: f64,
    f_max: f64,
) -> Result<Vec<(u16, u16)>, &'static str> {
    if bars < 1 {
        return Err("bars must be >= 1");
    }
    if bin_count < bars + 1 || bin_count > u16::MAX as usize {
        return Err("not enough bins for that many bars");
    }
    let bin_hz = sample_rate / (2.0 * bin_count as f64);
    let top = bin_count.min((f_max / bin_hz).floor() as usize);
    if top < bars + 1 {
        return Err("not enough bins below f_max for that many bars");
    }
    let mut edges = Vec::with_capacity(bars);
    let mut lo = 1usize.max((f_min / bin_hz).floor() as usize);
    for i in 0..bars {
        let f = f_min * (f_max / f_min).powf((i + 1) as f64 / bars as f64);
        let remaining = bars - i - 1;
        let mut hi = (f / bin_hz).floor() as usize;
        hi = hi.max(lo + 1);
        hi = hi.min(top - remaining);
        edges.push((lo as u16, hi as u16));
        lo = hi;
    }
    Ok(edges)
}

/// Loudest bin in each band, 0..1. `out` is cleared and refilled.
pub fn reduce_bands(bins: &[u8], edges: &[(u16, u16)], out: &mut Vec<f32>) {
    out.clear();
    for &(lo, hi) in edges {
        let max = bins
            .get(lo as usize..(hi as usize).min(bins.len()))
            .and_then(|s| s.iter().max())
            .copied()
            .unwrap_or(0);
        out.push(max as f32 / 255.0);
    }
}

/// Mean squared magnitude of the bins between `BEAT_LO_HZ` and `BEAT_HI_HZ`.
pub fn low_energy(bins: &[u8], sample_rate: u32) -> f32 {
    if bins.len() < 2 || sample_rate == 0 {
        return 0.0;
    }
    let bin_hz = sample_rate as f64 / (2.0 * bins.len() as f64);
    let lo = 1usize.max((BEAT_LO_HZ / bin_hz).floor() as usize);
    let hi = bins.len().min((BEAT_HI_HZ / bin_hz).ceil() as usize + 1);
    if hi <= lo {
        return 0.0;
    }
    let sum: f32 = bins[lo..hi]
        .iter()
        .map(|&b| {
            let v = b as f32 / 255.0;
            v * v
        })
        .sum();
    sum / (hi - lo) as f32
}

/// Onset detection on low-band energy: a frame well above the last second's
/// average, not too soon after the last one. A heuristic, and flagged as one
/// in the doc; a client that wants better runs its own on the spectrum.
#[derive(Default)]
pub struct BeatDetector {
    history: VecDeque<f32>,
    cap: usize,
    rate_hz: u32,
    since_last: u64,
}

impl BeatDetector {
    const RATIO: f32 = 1.5;
    const FLOOR: f32 = 0.01;
    const WINDOW_S: f32 = 1.0;
    const HOLD_S: f32 = 0.2;

    pub fn feed(&mut self, energy: f32, rate_hz: u32) -> bool {
        if rate_hz != self.rate_hz {
            self.reset();
            self.rate_hz = rate_hz;
            self.cap = ((rate_hz as f32 * Self::WINDOW_S) as usize).max(2);
        }
        self.since_last = self.since_last.saturating_add(1);
        let avg = if self.history.is_empty() {
            energy
        } else {
            self.history.iter().sum::<f32>() / self.history.len() as f32
        };
        let hold = (rate_hz as f32 * Self::HOLD_S) as u64;
        let warm = self.history.len() >= self.cap / 2;
        let beat =
            warm && energy > Self::FLOOR && energy > Self::RATIO * avg && self.since_last >= hold;
        if beat {
            self.since_last = 0;
        }
        self.history.push_back(energy);
        while self.history.len() > self.cap {
            self.history.pop_front();
        }
        beat
    }

    pub fn reset(&mut self) {
        self.history.clear();
        self.since_last = u64::MAX / 2;
    }
}

/// The #7 harness. Once a second: how far behind the webview's read each
/// frame arrived here, and how even the source cadence was.
struct Trace {
    hops_us: Vec<i64>,
    gaps_us: Vec<u64>,
    last_ts: Option<u64>,
    late_max_ms: f32,
    source_dropped: u64,
    source_failed: u64,
    source_err: String,
    last_seq: u64,
    seq_missing: u64,
    output_latency_ms: f32,
    since: Instant,
}

impl Default for Trace {
    fn default() -> Self {
        Self {
            hops_us: Vec::new(),
            gaps_us: Vec::new(),
            last_ts: None,
            late_max_ms: 0.0,
            source_dropped: 0,
            source_failed: 0,
            source_err: String::new(),
            last_seq: 0,
            seq_missing: 0,
            output_latency_ms: 0.0,
            since: Instant::now(),
        }
    }
}

fn wall_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn percentile<T: Copy + Ord>(sorted: &[T], p: f64) -> T {
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

impl Trace {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn record(&mut self, src: &SourceFrame<'_>) {
        let ts_us = src.timestamp_us;
        self.hops_us.push(wall_us() as i64 - ts_us as i64);
        if let Some(last) = self.last_ts {
            self.gaps_us.push(ts_us.saturating_sub(last));
        }
        self.last_ts = Some(ts_us);
        self.late_max_ms = self.late_max_ms.max(src.late_ms);
        self.source_dropped = src.source_dropped;
        self.source_failed = src.source_failed;
        self.output_latency_ms = src.output_latency_ms;
        if !src.source_err.is_empty() {
            self.source_err = src.source_err.clone();
        }
        if self.last_seq != 0 && src.seq > self.last_seq + 1 {
            self.seq_missing += src.seq - self.last_seq - 1;
        }
        self.last_seq = src.seq;
        if self.since.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.hops_us.sort_unstable();
        self.gaps_us.sort_unstable();
        let ms = |us: i64| us as f64 / 1000.0;
        let n = self.hops_us.len();
        if n > 0 {
            let hop = |p| ms(percentile(&self.hops_us, p));
            let mut line = format!(
                "hp-viz: {n} frames | hop webview->rust p50 {:.2} ms p95 {:.2} max {:.2}",
                hop(0.5),
                hop(0.95),
                hop(1.0)
            );
            if !self.gaps_us.is_empty() {
                let gap = |p| ms(percentile(&self.gaps_us, p) as i64);
                line.push_str(&format!(
                    " | cadence p50 {:.1} ms p95 {:.1} max {:.1}",
                    gap(0.5),
                    gap(0.95),
                    gap(1.0)
                ));
            }
            line.push_str(&format!(
                " | source: tick late max {:.1} ms, dropped {} / failed {} total, seq missing {}",
                self.late_max_ms, self.source_dropped, self.source_failed, self.seq_missing
            ));
            if !self.source_err.is_empty() {
                line.push_str(&format!(" | last error: {}", self.source_err));
            }
            line.push_str(&format!(
                " | audio output latency {:.1} ms",
                self.output_latency_ms
            ));
            eprintln!("{line}");
        }
        self.hops_us.clear();
        self.gaps_us.clear();
        self.late_max_ms = 0.0;
        self.seq_missing = 0;
        self.since = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cases `spectrum.test.ts` runs, so the two mappers stay one.
    #[test]
    fn band_edges_are_contiguous_non_empty_and_in_range() {
        for (bars, bins, rate) in [
            (19, 1024, 44100.0),
            (19, 512, 44100.0),
            (32, 1024, 48000.0),
            (24, 1024, 44100.0),
            (64, 1024, 48000.0),
            (128, 1024, 48000.0),
            (8, 128, 22050.0),
        ] {
            let e = band_edges(bars, bins, rate, F_MIN, F_MAX).unwrap();
            assert_eq!(e.len(), bars);
            assert!(e[0].0 >= 1, "never DC");
            for i in 0..bars {
                let (lo, hi) = e[i];
                assert!(hi > lo, "at least one bin: {bars}/{bins}/{rate} band {i}");
                if i > 0 {
                    assert_eq!(lo, e[i - 1].1, "contiguous");
                }
            }
            assert!(e[bars - 1].1 as usize <= bins);
        }
    }

    /// Same numbers as the TypeScript for the bars Main draws: 19 over 1024
    /// bins at 44.1 kHz starts at bin 2 (50 Hz / 21.5 Hz) and ends under 743
    /// (16 kHz).
    #[test]
    fn band_edges_match_the_screen() {
        let e = band_edges(19, 1024, 44100.0, 50.0, 16000.0).unwrap();
        assert_eq!(e[0].0, 2);
        assert!(e[18].1 <= 743);
        let width = |i: usize| e[i].1 - e[i].0;
        assert!(width(18) > width(9));
        assert!(width(9) >= width(0));
    }

    #[test]
    fn band_edges_refuse_shapes_that_cannot_give_every_band_a_bin() {
        assert!(band_edges(0, 1024, 44100.0, F_MIN, F_MAX).is_err());
        assert!(band_edges(200, 100, 44100.0, F_MIN, F_MAX).is_err());
    }

    #[test]
    fn reduce_bands_takes_the_loudest_bin_in_each_band() {
        let edges = [(1, 3), (3, 6)];
        let data = [255, 10, 51, 0, 255, 102, 255];
        let mut out = vec![9.0; 7];
        reduce_bands(&data, &edges, &mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 51.0 / 255.0).abs() < 1e-6);
        assert!((out[1] - 1.0).abs() < 1e-6, "bin 4, not bin 6");
    }

    #[test]
    fn reduce_bands_survives_edges_past_the_data() {
        let mut out = Vec::new();
        reduce_bands(&[0, 200], &[(1, 2), (5, 9)], &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0] > 0.7);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn low_energy_reads_only_the_bass_bins() {
        // 1024 bins at 48 kHz: 23.4 Hz each, so 40..160 Hz is bins 1..=7.
        let mut bins = [0u8; 1024];
        bins[500] = 255; // 11.7 kHz: not bass
        assert_eq!(low_energy(&bins, 48000), 0.0);
        bins[3] = 255;
        assert!(low_energy(&bins, 48000) > 0.0);
        assert_eq!(low_energy(&[], 48000), 0.0);
        assert_eq!(low_energy(&bins, 0), 0.0);
    }

    #[test]
    fn silence_never_beats() {
        let mut d = BeatDetector::default();
        for _ in 0..300 {
            assert!(!d.feed(0.0, 60));
        }
    }

    #[test]
    fn a_kick_after_quiet_beats_once_then_holds() {
        let mut d = BeatDetector::default();
        for _ in 0..60 {
            d.feed(0.02, 60);
        }
        assert!(d.feed(0.5, 60), "the onset");
        assert!(!d.feed(0.5, 60), "not twice in a row");
        for _ in 0..5 {
            assert!(!d.feed(0.5, 60), "held for 200 ms");
        }
        // Back to quiet, then another kick after the hold has passed.
        for _ in 0..30 {
            d.feed(0.02, 60);
        }
        assert!(d.feed(0.5, 60));
    }

    #[test]
    fn a_first_loud_frame_with_no_history_is_not_a_beat() {
        let mut d = BeatDetector::default();
        assert!(!d.feed(0.9, 30));
        assert!(!d.feed(0.9, 30));
    }

    #[test]
    fn percentile_covers_the_ends() {
        let v = [1, 2, 3, 4, 5];
        assert_eq!(percentile(&v, 0.0), 1);
        assert_eq!(percentile(&v, 0.5), 3);
        assert_eq!(percentile(&v, 1.0), 5);
        assert_eq!(percentile(&[7], 0.95), 7);
    }
}
