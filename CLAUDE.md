# hurricane-party

Offline-first media library and player. Save YouTube playlists, videos, and MP3s to disk so they're watchable when the internet is down during a hurricane. Classic Winamp-style skinnable windows that magnetize to each other.

Windows-first. Tauri v2 + Svelte 5 + Vite. SQLite. yt-dlp and ffmpeg as sidecars.

---

## Precedence — read this before resolving any ambiguity

When sources disagree, this order wins:

1. **`docs/decisions.md`** — the decision log. Authoritative. If something contradicts it, the other thing is wrong or stale
2. **`docs/*.md`** — the specs
3. **`design/`** — visual reference **only**

**The prototype is not the spec.** Design exports show intent, not contract. If the prototype has a control the specs don't mention, ask rather than implement. If a measurement conflicts, the spec wins.

If a decision genuinely hasn't been made, **stop and ask.** Don't infer one and proceed — the decision log exists specifically so choices get made once, on purpose, and stay made.

---

## Where things are

| Path | Read it when |
|---|---|
| `docs/decisions.md` | **Always.** Short. Start here |
| `docs/v0.4-brief.md` | Before starting v0.4. What ports from the spike, which findings are now requirements, and why stage 6 runs first |
| `docs/spike-v0.0.md` | Before starting the window engine spike. Stage order, pass/fail criteria, what the findings doc must answer |
| `docs/architecture.md` | Data model, import pipeline, yt-dlp/ffmpeg invocation, EQ audio graph |
| `docs/windows.md` | Window bonding, snapping, splitter resize, skin formats. Read before touching anything window-related |
| `docs/skin-manifest.md` | The `hp-skin/1` native format. Read before touching the skin renderer or either importer — both `.wsz` and `.wal` map *into* it |
| `docs/control-api.md` | The public IPC protocol and viz stream. Read before changing anything in `crates/hp-control/` |
| `docs/theme.md` | Eyewall and Cone themes, palette rationale, glow rendering |
| `docs/purricane.md` | Kaleidoscope theme + desktop kittens. v0.5 and v0.7. Not needed before then |
| `design/tokens.json` | Machine-readable palette. **Import this; never hardcode a hex value** |

---

## Non-negotiables

These have burned into the design. Don't quietly relax them.

- **Zero network at playback time.** No CDN fonts, no remote art fetch, no update check on launch. The one exception is the Cone theme's radar fetch, which runs on a timer, never blocks startup, and always displays data age
- **Dark only.** There is no light mode. Bright themes exist; a light *mode* does not
- **The job queue survives hard power loss.** SQLite WAL. On launch, `running` → `queued`
- **No in-process plugin loader.** The control API is the only external surface, and it runs in the other program's process
- **Never hardcode a color.** Two themes ship and skins are user-loadable
- **yt-dlp is the only extractor.** No custom site scrapers, ever

---

## Milestones

**The canonical table lives in `docs/decisions.md` (D27).** It used to be duplicated here and in two other docs; all three drifted, so there is now exactly one.

Work one at a time. Don't build ahead.

**v0.0 through v0.4a are built.** The window-engine spike returned **go** on the bond model (D45); its `bond.rs` and 34 tests **have now been ported byte-identical**, so the spike repo is no longer load-bearing and may be archived. Currently starting **v0.4b** — the skin renderer, EQ and analyser, briefed in `docs/v0.4-brief.md`.

---

## Working agreements

- **Branch per unit of work, merged with `--no-ff`.** Never commit straight to `main`. The merge bubble is the record of what changed together
- **Nothing lives only in a conversation.** Decisions go in `decisions.md`, findings go in a doc, working agreements go here. A session should be disposable
- **Test the interaction the way a user performs it, at least once per stage.** The v0.0 spike's scripted sweeps passed at 0 px error while the real interaction was dead, because an invisible window was eating the clicks (D43). Scripted paths and real input are not the same test
- **Verify against the real binary before believing a flag.** `--embed-thumbnail` looked right and hard-errors on webm; the fix only surfaced by running it
- `.sid/` is a scratch folder for screenshots. Gitignored, never referenced by code

## Conventions

- Platform-specific calls (`windows-rs`, HWND owner tricks) go behind a trait. Don't scatter `#[cfg(windows)]`
- Snap and bond math is in **physical pixels**, converted at the boundaries. Mixing logical and physical here produces bugs that only appear on a second monitor
- Parse yt-dlp with `--progress-template`. Never scrape the human-readable progress bar
- Every sensitive path (library roots, sidecar dirs) is configurable, never assumed

## Anti-scope

Written down so it doesn't get relitigated at midnight.

- In-process plugins or an add-ons folder
- `.wal` script (MAKI bytecode) execution — layout and art import only
- Library sharing, sync, or multi-device anything
- Streaming. This plays local files
- Custom extractors
- Mobile, cloud, accounts
- Fractional chrome scaling
- A light mode
