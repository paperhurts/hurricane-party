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
| DB | **SQLite** (`tauri-plugin-sql` or raw `rusqlite`) | Library metadata, playlists, job queue. WAL mode for crash safety. |
| Fetch | **yt-dlp** as Tauri sidecar (`externalBin`) | 1800+ site extractors, maintained by people who fight YouTube full-time. Do not write your own. |
| Transcode | **ffmpeg** as sidecar | Extract MP3 from downloaded video, normalize, generate waveform peaks. |
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
│  · file watcher, integrity checker          │
│  · storage budget accounting                │
└──────────────┬──────────────────────────────┘
               │ sidecar spawn
        ┌──────▼──────┐  ┌────────────┐
        │   yt-dlp    │  │  ffmpeg    │
        └─────────────┘  └────────────┘
```

Job runner emits progress over Tauri events. One yt-dlp process per item, bounded concurrency (2–3 default; more will get you rate-limited, not faster).

---

## Data model

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
  last_seen_at  INTEGER
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
  duration_s    INTEGER,

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
  created_at    INTEGER NOT NULL
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
  want_video    INTEGER NOT NULL DEFAULT 1,
  want_audio    INTEGER NOT NULL DEFAULT 1,
  status        TEXT NOT NULL CHECK (status IN ('queued','running','done','failed','paused')),

  -- resume, not restart (D26). which recovery to run depends on where it died
  stage         TEXT NOT NULL DEFAULT 'probe'
                CHECK (stage IN ('probe','download','extract','verify')),
  outtmpl       TEXT,                 -- resolved output path, so --continue finds the .part
  part_path     TEXT,                 -- the partial yt-dlp was writing

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
  key           TEXT PRIMARY KEY,     -- 'storage.budget_bytes', 'chrome.scale', 'skin.active'
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

This is the v0.2 schema and there is no deployed data yet, so it ships as one initial
migration rather than a chain. The columns that exist purely to avoid a painful retrofit
later — `profile_id` (O9), `library_roots` + `relpath` (D28), `media.title` (D34) — are
the whole reason to get this right before v0.2 writes the first migration file.

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
- **Prune** a root: a rescan of a known root counts the rows whose files are gone and the
  user is offered to drop them. Nothing drops them unasked, and a root that is not
  mounted reports nothing (D28: unplugged is not missing).

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

First band as `lowshelf`, last as `highshelf`, the middle eight as `peaking` with Q around 1.0–1.4. Verify the exact frequency set against a reference before locking the UI labels.

**Range** — ±12 dB per band, ±12 dB preamp. Matching the classic range means imported `.eqf` preset files map 1:1 with no rescaling.

**Clipping is the part people get wrong.** Boosting bands and preamp together can push well past unity and the output clips audibly. Two fixes, use both:

- A **trim gain node** after the chain, automatically reduced by the maximum applied boost
- A **clip indicator** in the EQ window that lights when the analyser sees samples at ceiling

A `DynamicsCompressorNode` as a limiter is the lazier option and it colors the sound. Prefer the trim.

**Per-track EQ** — the `auto` toggle on the EQ window means "load this track's saved preset on play." That's what `media.eq_preset_id` is for. Null means use the global setting.

**`.eqf` import** — Winamp's EQ preset format is small and simple, and importing it is cheap. Worth doing alongside `.wsz` in v0.5.

**Deliberately not in the control API v1.** No `set_eq` command. The public surface stays small; EQ is an in-app control, and adding it later is additive rather than breaking.

On launch: `UPDATE jobs SET status='queued' WHERE status='running'` — recover anything interrupted.

**That means "re-enter the runner," not "start over" (D26).** The row keeps its `stage`, `outtmpl`, and `part_path`, and the runner picks the recovery that matches where it died:

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

### Phase 2 — fetch

For each selected item:

```bash
yt-dlp \
  -f "bv*+ba/b" \
  --embed-metadata --embed-thumbnail \
  --write-info-json --write-thumbnail \
  --no-playlist \
  --continue \
  --newline \
  --progress-template "download:%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.speed)s|%(progress.eta)s" \
  -o "%(extractor)s/%(title)s [%(id)s].%(ext)s" \
  "<url>"
```

**On the output template (D49):** the `%(uploader)s` level that used to sit between extractor and title is gone. On YouTube the uploader is the channel, not the artist, so it produced roughly one folder per file. It's still stored on `media.uploader`, and the library browser is a DB query (O6) rather than a directory listing.

**The trailing `[%(id)s]` is load-bearing, not decoration.** It's how a resumed job finds the file it was part-way through, without parsing yt-dlp's stdout for the name it chose — which is the same class of mistake as screen-scraping the progress bar.

**Use `--progress-template`, not screen-scraping the progress bar.** Pipe-delimited or JSON, parse it in Rust, emit a Tauri event. The human-readable bar changes between releases; the template doesn't.

### Phase 3 — derive the MP3 locally

Don't download twice. You already have the best audio stream inside the video file:

```bash
ffmpeg -i "input.mp4" -vn -c:a libmp3lame -q:a 0 "output.mp3"
```

Saves bandwidth and a round-trip. If the user only wants audio, skip video entirely with `-x --audio-format mp3 --audio-quality 0`.

### The PoToken problem — resolved as D25

YouTube requires a Proof-of-Origin Token per request now. Without it you get downgraded formats or outright failures. It's the single most likely thing to break this app six months from now.

**Decision (D25): bundle `bgutil-ytdlp-pot-provider` as a third sidecar, in script mode.**

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

Concretely: make the player chrome a fixed **275 × 116 logical-pixel canvas** with absolutely-positioned elements at Winamp's known coordinates, scaled by an integer factor (1x/2x/3x) for modern displays. The theme layer supplies *either* sprite offsets into a bitmap *or* CSS colors and vectors.

Do that and `.wsz` support later is a **loader**, not a rewrite. Skip it and you'll be trying to retrofit fixed-pixel sprite positioning onto a flexbox app, which is genuinely miserable.

Ship **your own** default skin — don't bundle third-party Winamp skins, those are other people's copyrighted art. Let users load their own `.wsz` files from disk.

---

## Hurricane-specific features (the ones that justify the name)

**Prep mode.** A pre-storm bulk screen: paste everything, see total GB, see free disk, hit go. Progress that survives reboots. This is the killer feature and nothing else on the market does it well.

**Storage budget.** Set a ceiling (say 40 GB). Show a meter. Warn before crossing. Offer "audio only" as a per-source downgrade that cuts size ~90%.

**Battery mode.** Video decode eats battery. A toggle that forces audio-only playback and kills the visualizer extends runtime meaningfully on a laptop running off an inverter.

**Integrity check.** Hash on import, verify on launch (throttled, background). Surface a "3 files failed verification" banner. Finding out mid-outage that your download truncated is the exact failure this app exists to prevent.

**Zero-network guarantee.** A tested property, not an aspiration — **D29** pins the mechanism:

- The **webview CSP forbids remote origins outright.** `connect-src`, `img-src`, `font-src`, and `script-src` allow only `self` and Tauri's `asset:` scheme. No CDN font, no remote thumbnail, no analytics can be added later by accident — it fails at load, loudly, in development
- **All egress lives in Rust**, behind a single allowlisted command for the Cone radar fetch (D19). There is exactly one function in the codebase that opens a socket to the internet, and it's greppable
- **A test runs the app with the interface down** and asserts no connection is attempted

Anything that needs the network degrades silently, not spins. The CSP is doing the real work here: it turns "we intend not to make requests" into "requests cannot be made," which is the difference between a guarantee and a habit.

---

## Other sites — mostly free

yt-dlp handles Bandcamp, SoundCloud, Vimeo, Internet Archive, Mixcloud, and ~1800 others out of the box. Your `extractor` column already stores which one was used. **The work is UI affordances, not backend** — per-site auth for things like Bandcamp purchases, and sensible format defaults per extractor. Don't build site-specific code paths until a site actually forces you to.

Also worth doing early and cheaply: **import a local folder.** Scan, read tags via ffprobe, insert into `media` with a null `source_id`. Suddenly the app is useful for the music she already owns, not just YouTube.

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
