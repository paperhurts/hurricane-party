# hurricane-party

Offline-first media library and player. Grab it before the storm, play it when the grid's down.

---

## The actual design constraint

This isn't "a media player that happens to work offline." It's **a player that assumes the network is gone at playback time.** That flips a lot of defaults:

- No CDN fonts, no remote album art, no telemetry, no update check on launch
- Thumbnails, metadata, and lyrics get cached **at import**, never fetched at play time
- The job queue survives a hard power loss — SQLite WAL, not in-memory state
- Storage awareness is a first-class feature, not a settings-page afterthought
- Integrity check on launch, so you find the corrupt file the day before, not during

Everything below follows from that.

---

## Stack

| Layer | Choice | Why |
|---|---|---|
| Shell | **Tauri v2** | ~10 MB bundle vs Electron's ~150 MB. "Lightweight" is in the brief. Also: you already know it from doc-md. |
| UI | **Svelte 5 + Vite** | Same reason. No reason to learn a second frontend stack. |
| DB | **SQLite** through `rusqlite` (bundled) | Library metadata, playlists, job queue, settings. WAL mode for crash safety. |
| Fetch | **yt-dlp** as Tauri sidecar (`externalBin`), with **deno** beside it for YouTube's JS challenges (D46) | 1800+ site extractors, maintained by people who fight YouTube full-time. Do not write your own. |
| Transcode | **ffmpeg** as sidecar: yt-dlp's own build, pinned by checksum, or a person's own copy (D133) | Derive the MP3 from the downloaded file and attach its cover art. |
| Playback | **HTML5 `<audio>` / `<video>`** via `convertFileSrc()` | See below — this is a real decision, not a default. |

### Why HTML5 audio instead of Rust-side (rodio/symphonia)

Because you want a Winamp-style spectrum analyzer. `AnalyserNode` from the Web Audio API gives you FFT bins for free, and `BiquadFilterNode` gives you the classic 10-band EQ for free. Doing that in Rust means writing your own FFT plumbing and piping bins across the IPC boundary at 60fps. Not worth it.

Route: `<audio>` element → `MediaElementAudioSourceNode` → EQ filter chain → `AnalyserNode` → destination.

---

## Process architecture

```
┌─────────────────────────────────────────────┐
│  Webview (Svelte)                           │
│  · player chrome, skin renderer             │
│  · Web Audio graph (EQ + analyser)          │
│  · library browser, playlist editor         │
└──────────────┬──────────────────────────────┘
               │ Tauri IPC + events
┌──────────────▼──────────────────────────────┐
│  Rust core                                  │
│  · SQLite (library, playlists, job queue)   │
│  · job runner: spawns yt-dlp / ffmpeg       │
│  · library scan, control pipe, viz stream   │
│  · v0.6: watcher, integrity, storage budget │
└──────────────┬──────────────────────────────┘
               │ sidecar spawn
        ┌──────▼──────┐  ┌────────────┐
        │   yt-dlp    │  │  ffmpeg    │
        └─────────────┘  └────────────┘
```

Job runner emits progress over Tauri events. One yt-dlp process per item, bounded concurrency (default 2, adjustable 1–4 in the library header; more gets you rate-limited, not faster — O12).

---

## Data model

The schema that runs is `SCHEMA` in `src-tauri/src/db.rs`; this is the same shape with the reasons written beside it. When the two disagree, the code is what is on disk and this block is stale.

```sql
-- what you imported from
CREATE TABLE sources (
  id            INTEGER PRIMARY KEY,
  url           TEXT UNIQUE NOT NULL,
  extractor     TEXT NOT NULL,        -- 'youtube', 'bandcamp', 'soundcloud'
  title         TEXT,
  uploader      TEXT,
  upload_date   TEXT,
  duration_s    INTEGER,
  thumb_path    TEXT,                 -- cached locally at import
  info_json     TEXT,                 -- full yt-dlp dump, keep it
  added_at      INTEGER NOT NULL
);

-- where files live. many roots from day one (D28) — the external drive case is real
CREATE TABLE library_roots (
  id            INTEGER PRIMARY KEY,
  label         TEXT NOT NULL,        -- 'Internal SSD', 'Storm drive'
  path          TEXT UNIQUE NOT NULL, -- absolute, resolved at mount time
  is_removable  INTEGER NOT NULL DEFAULT 0,
  last_seen_at  INTEGER,
  volume        TEXT,                 -- the drive's serial, which follows it to a new letter (D143)
  volume_rel    TEXT                  -- where on that drive the root is
);

-- the files on disk. one source can have several (video + extracted mp3)
CREATE TABLE media (
  id            INTEGER PRIMARY KEY,
  source_id     INTEGER REFERENCES sources(id) ON DELETE CASCADE,  -- NULL for local import
  root_id       INTEGER NOT NULL REFERENCES library_roots(id),
  relpath       TEXT NOT NULL,        -- relative to the root. NEVER store an absolute path (D28)
  kind          TEXT NOT NULL CHECK (kind IN ('audio','video')),

  -- denormalized (D34). local imports have no source row, so without these
  -- the library browser cannot render a row for a file she already owned
  title         TEXT NOT NULL,
  uploader      TEXT,
  duration_s    REAL,                 -- seconds, fractional, as the probe reports them

  container     TEXT,                 -- 'mp3', 'mp4', 'opus'
  bitrate_kbps  INTEGER,
  filesize      INTEGER,
  sha256        TEXT,                 -- integrity check
  verified_at   INTEGER,
  eq_preset_id  INTEGER REFERENCES eq_presets(id),   -- per-track EQ, the 'auto' toggle
  added_at      INTEGER NOT NULL,

  UNIQUE (root_id, relpath)
);

CREATE TABLE playlists (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  is_smart      INTEGER DEFAULT 0,
  rule_json     TEXT,                 -- for smart playlists
  profile_id    INTEGER NOT NULL DEFAULT 1,   -- O9. free now, a migration later
  created_at    INTEGER NOT NULL,
  position      INTEGER               -- the order a person arranged the lists in (D116)
);

CREATE TABLE playlist_items (
  playlist_id   INTEGER REFERENCES playlists(id) ON DELETE CASCADE,
  media_id      INTEGER REFERENCES media(id) ON DELETE CASCADE,
  position      INTEGER NOT NULL,
  PRIMARY KEY (playlist_id, position)
);
-- NOTE: the unique (playlist_id, position) invariant is worth keeping, but it means a
-- reorder cannot be a naive sequence of UPDATEs — it collides mid-statement. SQLite has
-- no deferred UNIQUE. Reorder in one transaction, shifting affected rows to negative
-- positions first, then writing final values. Write it once, in a helper, and test it.

-- survives power loss. this is the point.
CREATE TABLE jobs (
  id            INTEGER PRIMARY KEY,
  url           TEXT NOT NULL,
  want_video    INTEGER NOT NULL DEFAULT 0,   -- audio unless the video box is ticked
  want_audio    INTEGER NOT NULL DEFAULT 1,
  status        TEXT NOT NULL CHECK (status IN ('queued','running','done','failed','paused')),

  -- resume, not restart (D26). which recovery to run depends on where it died
  stage         TEXT NOT NULL DEFAULT 'probe'
                CHECK (stage IN ('probe','download','extract','verify')),
  title         TEXT,                 -- from the probe, so the row reads before it lands
  video_id      TEXT,                 -- the source's id, the [id] in the file name
  outtmpl       TEXT,                 -- resolved output path, so --continue finds the .part

  progress      REAL NOT NULL DEFAULT 0,
  bytes_done    INTEGER NOT NULL DEFAULT 0,
  bytes_total   INTEGER,
  error         TEXT,
  attempts      INTEGER NOT NULL DEFAULT 0,
  playlist_id   INTEGER REFERENCES playlists(id) ON DELETE SET NULL,  -- auto-add on completion
  profile_id    INTEGER NOT NULL DEFAULT 1,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE play_history (
  media_id      INTEGER REFERENCES media(id) ON DELETE CASCADE,
  played_at     INTEGER NOT NULL,
  completed     INTEGER,
  profile_id    INTEGER NOT NULL DEFAULT 1    -- O9
);

-- settings live in the DB, not a side file (D32), so a hard power loss
-- can't desync them from the library they describe
CREATE TABLE settings (
  key           TEXT PRIMARY KEY,     -- 'skin.current', 'chrome.glow', 'play.shuffle', 'ytdlp.cookies'
  value         TEXT NOT NULL         -- JSON
);

-- window geometry and the bond graph survive restart (D33).
-- physical pixels, with the monitor recorded, so a group doesn't restore offscreen.
CREATE TABLE window_layout (
  window_id     TEXT PRIMARY KEY,     -- 'main' | 'eq' | 'playlist' | 'library' | ...
  x             INTEGER NOT NULL,
  y             INTEGER NOT NULL,
  w             INTEGER NOT NULL,
  h             INTEGER NOT NULL,
  shaded        INTEGER NOT NULL DEFAULT 0,
  visible       INTEGER NOT NULL DEFAULT 1,
  monitor_id    TEXT
);

CREATE TABLE window_bonds (
  a             TEXT NOT NULL,
  b             TEXT NOT NULL,
  edge          TEXT NOT NULL,        -- A's edge that touches B: 'right' | 'bottom' | ...
  span_start    INTEGER NOT NULL,     -- overlapping extent of the shared boundary
  span_end      INTEGER NOT NULL,
  PRIMARY KEY (a, b)
);

-- EQ. Was missing from the first draft of this schema.
CREATE TABLE eq_presets (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  preamp_db     REAL NOT NULL DEFAULT 0,
  bands_db      TEXT NOT NULL,        -- JSON array of 10 floats, -12..+12
  is_builtin    INTEGER DEFAULT 0,
  created_at    INTEGER NOT NULL
);

-- per-track EQ, for the "auto" toggle, is now `media.eq_preset_id` in the CREATE above
-- rather than a trailing ALTER. Create `eq_presets` before `media` when running the
-- schema, since media references it.
```

### Migration note

This was the v0.2 schema, written before there was any deployed data, so it shipped as
one `CREATE TABLE IF NOT EXISTS` batch rather than a chain of migrations. The columns
that exist purely to avoid a painful retrofit later — `profile_id` (O9), `library_roots` +
`relpath` (D28), `media.title` (D34) — are why it was worth getting right first.

There is deployed data now (the v0.4 Releases), so a column added since goes through
`db::migrate`, which runs on every open and adds what an older database lacks:
`playlists.position` (D116) is the first.

The one data fix since is `db::normalize_roots`, run on every open (D83): a root the
scanner stored in Windows' verbatim form (`\\?\C:\…`) is folded into the plain-path twin
the download pipeline stored, and the twin keeps the rows. Roots are stored plain from
then on.

### Leaving the library

Three verbs, in `library.rs`, kept apart on purpose (D83, #78):

- **Remove** a row: the row goes, its playlist memberships go (the cascade, then each list
  closes its gap), the file stays. No undo; adding the folder or the URL again brings it
  back.
- **Delete the file**: a separate call, made after the row is gone, from the notice that
  has already printed the path, behind a warning dialog. Rust refuses any path outside a
  library root. The only destructive action in the app.
- **Prune** a root: a rescan of a known root (a click on the root in the library's sidebar, or Add folder on the same folder; D95) counts the rows whose files are gone and the
  user is offered to drop them. Nothing drops them unasked, and a root that is not
  mounted reports nothing (D28: unplugged is not missing).

A root on a drive that is out keeps its rows, and the library window leaves them out of
what it lists and what plays, with one line to show them greyed (D143). The watcher says
within seconds when a drive goes or comes back. A root that is there remembers its drive by
the volume's serial number, so a flash drive back under another letter takes its root with
it (`drives.rs`).

### Equalizer spec

The EQ window is in the design brief and the window inventory, but the audio side needs pinning down before build.

**Topology** — a `BiquadFilterNode` chain between the source and the analyser:

```
<audio> → MediaElementSource → [preamp Gain]
        → lowshelf → peaking ×8 → highshelf
        → [trim Gain] → AnalyserNode → destination
```

**Bands** — the classic ten, in Hz:

`60 · 170 · 310 · 600 · 1k · 3k · 6k · 12k · 14k · 16k`

First band as `lowshelf`, last as `highshelf`, the middle eight as `peaking` at Q 1.2 (D75, `src/lib/audio.ts`). The frequency set is verified against the classic (D21, D31).

**Range** — ±12 dB per band, ±12 dB preamp. Matching the classic range means imported `.eqf` preset files map 1:1 with no rescaling.

**Clipping is the part people get wrong.** Boosting bands and preamp together can push well past unity and the output clips audibly. Two fixes, use both:

- A **trim gain node** after the chain, automatically reduced by the maximum applied boost
- A **clip indicator** in the EQ window that lights when the analyser sees samples at ceiling

A `DynamicsCompressorNode` as a limiter is the lazier option and it colors the sound. Prefer the trim.

**Per-track EQ** — the `auto` toggle on the EQ window means "load this track's saved preset on play." That's what `media.eq_preset_id` is for. Null means use the global setting.

**`.eqf` import** — Winamp's EQ preset format is small and simple, and importing it is cheap. Built at v0.5 (#145, D128): the EQ preset menu imports any number of `.eqf` files and saves the EQ under a name, into `eq_presets`, beside the four presets that ship. D31 has the byte layout.

**Deliberately not in the control API v1.** No `set_eq` command. The public surface stays small; EQ is an in-app control, and adding it later is additive rather than breaking.

### Recovering the job queue

On launch: `UPDATE jobs SET status='queued' WHERE status='running'` — recover anything interrupted.

**That means "re-enter the runner," not "start over" (D26).** The row keeps its `stage`, `outtmpl` and `video_id`, and the runner picks the recovery that matches where it died:

| Died in | Recovery |
|---|---|
| `probe` | Re-probe. Cheap, no partial state |
| `download` | Re-invoke yt-dlp with `--continue` against the preserved `.part` |
| `extract` | Delete the truncated MP3 and re-run ffmpeg. The source video is intact |
| `verify` | Re-hash |

**Never delete `.part` files on startup.** A 2 GB video interrupted at 90% is the case this whole property exists for, and a well-meaning "clean up temp files on launch" pass silently converts resume into restart.

---

## Import pipeline

**Two-phase. Always show the user what they're about to download before downloading it.**

### Phase 1 — probe (no download)

```bash
yt-dlp -J --flat-playlist "<url>"
```

Returns JSON. For a playlist you get the item list without hitting every video. Parse it, show a checklist with estimated sizes, let her deselect. *This is the difference between a good app and a frustrating one* — a 200-video playlist that starts downloading on paste is hostile.

**Built at v0.5** (#137, D114). `probe_playlist` is that one call — the only one that drops `--no-playlist`, so the list expands there and nowhere else. The picker shows every entry with its duration and marks what the library already has, the kept entries queue as one job each, and the list becomes a playlist they are filed into as they finish (`jobs.playlist_id`). A `RD…` mix is refused: YouTube makes those up per person, so there is nothing to snapshot.

### Phase 2 — fetch

For each selected item:

```bash
yt-dlp \
  --ffmpeg-location <ffmpeg in use: the bundled one, or the person's own (D133)> \
  -f "bv*+ba/b" \                      # "bestaudio/best" when only audio is wanted
  --embed-metadata \
  --write-thumbnail --convert-thumbnails jpg \
  --no-playlist \
  --continue \
  --newline \
  --progress-template "download:HPPROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.speed)s|%(progress.eta)s|%(progress.status)s" \
  --merge-output-format mp4 \
  -o "<root>/%(extractor)s/%(title)s [%(id)s].%(ext)s" \
  -- "<url>"
```

The arguments that run are in `src-tauri/src/pipeline.rs`. Two differ from the first draft of this page, both found by running them: **no `--embed-thumbnail`**, which hard-errors on the webm/opus intermediate and fails a job after a good download (the cover is attached at the ffmpeg step instead, where the container is MP3), and a **`HPPROG` marker** at the front of the progress template, so a progress line can never be mistaken for anything else yt-dlp prints. The `cookies` arguments below are prepended to all of it when set.

**On the output template (D49):** the `%(uploader)s` level that used to sit between extractor and title is gone. On YouTube the uploader is the channel, not the artist, so it produced roughly one folder per file. It's still stored on `media.uploader`, and the library browser is a DB query (O6) rather than a directory listing.

**The trailing `[%(id)s]` is load-bearing, not decoration.** It's how a resumed job finds the file it was part-way through, without parsing yt-dlp's stdout for the name it chose — which is the same class of mistake as screen-scraping the progress bar.

**Signing in, for the videos that need it (D112).** Some videos — age-restricted, members-only — are refused to a signed-out request, and yt-dlp says so in a wall of text about cookies. A person can point the app at a `cookies.txt` they exported from their own browser (*Cookies…* in the library header); the path is a setting, and `--cookies <path>` is then prepended to **every** yt-dlp call, probe included, since the probe is what fails first. The app stores the path and nothing else: the file is never read by this process, never copied into the library, never logged. Unset, which is the default, means no authentication at all. The same header also offers *From a browser…*, which runs yt-dlp once with `--cookies-from-browser <b> --cookies <jar>` and points the setting at the jar it writes (D113) — yt-dlp dumps the store it loaded, so no third-party extension ever touches a person's cookies. That run is given `https://cookies.invalid/`, a name RFC 2606 guarantees cannot resolve, so it reaches nobody; the jar is written regardless and the exit code is ignored. The list is an allowlist in Rust, and it offers **profiles**, not browsers (D115): `--cookies-from-browser chrome` means `chrome:Default`, and a second profile is where a YouTube sign-in often lives. Enumerating them opens no cookie database — Chromium's `User Data/*` plus the display names in `Local State`, Firefox's `profiles.ini` — and Firefox's spec carries the profile's **path**, since yt-dlp resolves a bare name against a directory of that exact name and fails. It is also the one place the app holds a credential rather than a path to one: the jar sits in per-user app data because someone clicked for it, and the app only ever reports how many cookies it contains. What that route cannot do is decrypt Chrome or Edge on Windows (App-Bound Encryption) or read Brave while it is running, and the refusal says which wall it hit.

**Use `--progress-template`, not screen-scraping the progress bar.** Pipe-delimited or JSON, parse it in Rust, emit a Tauri event. The human-readable bar changes between releases; the template doesn't.

### Phase 3 — derive the MP3 locally

Don't download twice. You already have the best audio stream inside the video file:

```bash
ffmpeg -i "<downloaded file>" [-i cover.jpg -map 1 ...] -map 0:a -map_metadata 0 \
       -c:a libmp3lame -q:a 2 -metadata title=... -metadata artist=... "<scratch>.mp3"
```

Saves bandwidth and a round-trip. When only audio is wanted, the download is already `bestaudio/best` rather than a video, and the same step turns it into the MP3; yt-dlp's own `-x` is not used, so there is one extraction path. The written thumbnail becomes the MP3's front cover, and the title and artist come from the probe rather than whatever tags the stream carried.

### The PoToken problem — D25, superseded by D46

**This section is history.** D25 bundled a PO token provider; tested on 2026-08-30, yt-dlp needed no PO token at all, and what it did need was a JavaScript runtime for YouTube's challenges. So the third sidecar is **deno** (D46), not a token provider, and the reasoning below survives only as the case for keeping any such helper pinned and swappable.

YouTube requires a Proof-of-Origin Token per request now. Without it you get downgraded formats or outright failures. It's the single most likely thing to break this app six months from now.

**Decision (D25, superseded): bundle `bgutil-ytdlp-pot-provider` as a third sidecar, in script mode.**

Script mode invokes the provider per request rather than running a long-lived HTTP listener. Server mode is marginally faster across a big prep run, but it means a localhost socket sitting open — and D11's zero-network guarantee is worth more as a by-construction property than as a benchmark. A listener that's "only local" is exactly the kind of thing that erodes into an exception.

Consequences to build for:

- **Three sidecars now**, not two: yt-dlp, ffmpeg, and the POT provider. Bundling is per-platform and fiddly; budget for it in v0.1, because v0.1's definition of done is a YouTube URL producing an MP3
- **The provider is swappable.** Put it behind the same seam as the extractor backend. When it breaks — and it will — replacing it should be config, not a refactor
- **Health shows in the diagnostics panel** alongside yt-dlp and ffmpeg. "Which of my three helpers is broken" is the first question at 2am
- **Pinned, never silently auto-updated** (O11). This applies to the provider as much as to yt-dlp. A surprise bump the day before a storm is the wrong failure

Also surface yt-dlp's stderr to the diagnostics panel and ship an in-app "check for update" button. An offline app that can't fetch anymore is a brick.

---

## The skinning decision (read this one carefully)

You said "like old Winamp." There are two very different things that could mean, and **picking the wrong one costs you a rewrite.**

**Option A — CSS themes.** Custom properties, a `theme.json`, done in an afternoon. Flexible layout, responsive, modern. Not actually Winamp.

**Option B — real `.wsz` skin support.** Classic Winamp skins are ZIPs of BMPs with fixed sprite-sheet layouts: `MAIN.BMP`, `CBUTTONS.BMP`, `TITLEBAR.BMP`, `NUMBERS.BMP`, `TEXT.BMP`, `VOLUME.BMP`, `POSBAR.BMP`, plus `PLEDIT.TXT` and `VISCOLOR.TXT` for palettes and `REGION.TXT` for non-rectangular windows. Sprite coordinates are conventional, not declared. Webamp (MIT, github.com/captbaritone/webamp) already implements the whole format in JS — worth studying even if you don't vendor it.

### My actual recommendation

**Build the sprite-sheet abstraction from day one, ship CSS themes first.**

Concretely: make the player chrome a fixed **275 × 116 logical-pixel canvas** with absolutely-positioned elements at Winamp's known coordinates, scaled by an integer factor (1x or 2x, O3) for modern displays. The theme layer supplies *either* sprite offsets into a bitmap *or* CSS colors and vectors.

Do that and `.wsz` support later is a **loader**, not a rewrite. Skip it and you'll be trying to retrofit fixed-pixel sprite positioning onto a flexbox app, which is genuinely miserable.

Ship **your own** default skin — don't bundle third-party Winamp skins, those are other people's copyrighted art. Let users load their own `.wsz` files from disk.

**What was built.** The recommendation held. The three classic windows are drawn from `hp-skin/1` manifests (`docs/skin-manifest.md`): Eyewall, the skin that ships, is mask sheets tinted from the theme (D73, D90); a `.wsz` is mapped into the same format by an importer, supported **to a degree** — it wears, and what it draws that this app has no feature for is said out loud (D102–D111); and **making your own is the headline** (D110): *Make a skin…* turns any picture into a skin with Eyewall's chrome in the picture's colours and the picture behind the windows (D122).

---

## Hurricane-specific features (the ones that justify the name)

**Prep mode.** A pre-storm bulk screen: paste everything, see total GB, see free disk, hit go. Progress that survives reboots. This is the killer feature and nothing else on the market does it well.

*As built* (#163, D140): *Hurricane Party Planning*, a window of its own from the library header or the tray. One paste box sorts its lines, reads lists without downloading, and shows the run in audio and in video against the drive; one press queues it as a batch through the ordinary queue, and the run's progress, the radar pre-cache, and a download's wait for a lost connection (D141) all outlast a restart.

**Storage budget.** Set a ceiling (say 40 GB). Show a meter. Warn before crossing. Offer "audio only" as a per-source downgrade that cuts size ~90%.

*As built* (#162, D138): the meter is in the library footer, beside the download folder, with each root's size in the roots list. The ceiling is optional and counts the whole library. It warns at 85% of the download drive or past the ceiling, and never holds a download. The audio-only saving is measured from the library's own files rather than assumed.

**Battery mode.** Video decode eats battery. A toggle that forces audio-only playback and kills the visualizer extends runtime meaningfully on a laptop running off an inverter.

**Integrity check.** Hash on import, verify on launch (throttled, background). Surface a "3 files failed verification" banner. Finding out mid-outage that your download truncated is the exact failure this app exists to prevent.

*As built* (#164, D142): a download is hashed as it lands and checked against the length the site gave. The library is read back quietly twenty seconds after launch, never at it, a quarter second between files, four gigabytes a launch, skipping unplugged drives. A file that changed, went short or stopped opening is marked on its row; the library says how many failed, and each row offers Accept or Download again. **Check files** in the footer reads everything now.

**Zero-network guarantee.** A tested property, not an aspiration — **D29** pins the mechanism:

- The **webview CSP forbids remote origins outright.** `connect-src`, `img-src`, `font-src`, and `script-src` allow only `self` and Tauri's `asset:` scheme. No CDN font, no remote thumbnail, no analytics can be added later by accident — it fails at load, loudly, in development
- **All egress lives in Rust**, behind a single allowlisted command for the Cone radar fetch (D19). *As built* (#85, D135), that is `egress::get` in `src-tauri/src/egress.rs`, to `mapservices.weather.noaa.gov` and `api.weather.gov` over https only. There is exactly one function in the codebase that opens a socket to the internet, and it's greppable
- **A test runs the app with the interface down** and asserts no connection is attempted. *That test is not written yet*; the CSP (`src-tauri/tauri.conf.json`) is what enforces the property today

Anything that needs the network degrades silently, not spins. The CSP is doing the real work here: it turns "we intend not to make requests" into "requests cannot be made," which is the difference between a guarantee and a habit.

---

## Other sites — mostly free

yt-dlp handles Bandcamp, SoundCloud, Vimeo, Internet Archive, Mixcloud, and ~1800 others out of the box. Your `extractor` column already stores which one was used. **The work is UI affordances, not backend** — per-site auth for things like Bandcamp purchases, and sensible format defaults per extractor. Don't build site-specific code paths until a site actually forces you to.

Also worth doing early and cheaply: **import a local folder.** Scan, read tags via ffprobe, insert into `media` with a null `source_id`. Suddenly the app is useful for the music she already owns, not just YouTube. *Built at v0.3*, as *Add folder* in the library; a click on a root rescans it (D95).

---

## Build order

**See the milestone table in `decisions.md`, which is canonical (D27).** This document used to carry its own copy; it drifted, so it's gone rather than maintained in parallel.

Get to v0.1 in a weekend. Everything after that is incremental and shippable.

---

## Where I'd push back

**Winamp skin fidelity is a tarpit.** The format has undocumented quirks, and every hour spent on pixel-perfect `.wsz` parity is an hour not spent on the offline features that are the actual point. Sprite-ready architecture now, full loader at v0.5, and be willing to ship "most skins work" rather than "all skins work."

**Don't add a plugin system.** You'll want to. Resist until v1.0 — it's the classic way a personal tool becomes an unshippable platform.

**On legality**, briefly, then I'll drop it: downloading for personal offline use is a YouTube ToS matter, not a criminal one, and yt-dlp itself is a neutral tool that GitHub reinstated specifically because it has legitimate uses. Your call, your risk tolerance, you know the terrain. I'd just say: don't build a "share library" feature, because that's where the character of the thing changes.

---

## Open questions

All three original questions here are resolved. Kept with their answers because the reasoning is load-bearing:

1. ~~Windows-only, or MINERVA + laptop + anything else?~~ → **O7.** Windows first; keep the code portable, don't test elsewhere until needed. Note this got more expensive with D25 — sidecar bundling is per-platform and there are now three of them.
2. ~~Single library folder, or multiple roots?~~ → **O8, and D28 for the consequence.** Many roots from day one, and `media` stores `(root_id, relpath)` so a drive returning under a different letter doesn't orphan the library.
3. ~~Does Rowan get his own playlists?~~ → **O9.** Skip the feature, carry `profile_id` on `playlists`, `play_history`, and `jobs` defaulting to 1.
