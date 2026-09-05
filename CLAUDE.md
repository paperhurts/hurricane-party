# hurricane-party

Offline-first media library and player. Save YouTube playlists, videos, and MP3s to disk so they're watchable when the internet is down during a hurricane. Classic Winamp-style skinnable windows that magnetize to each other.

Windows-first. Tauri v2 + Svelte 5 + Vite. SQLite. yt-dlp, ffmpeg, and deno as sidecars.

---

## Precedence — when sources disagree

1. **`docs/decisions.md`** — the decision log. If something contradicts it, the other thing is wrong or stale
2. **`docs/*.md`** — the specs
3. **`design/`** — visual reference **only**

The prototype is not the spec. Design exports show intent, not contract. If the prototype has a control the specs don't mention, say so rather than building it unasked. If a measurement conflicts, the spec wins.

If no decision covers a choice, make the obvious call, write it down (a row in `decisions.md`, in the same PR), and say so in the PR. Ask only when the options lead to materially different work, or when it's a taste call that's the owner's: what it looks like, what it's called, what it's for. A decision that turns out wrong gets superseded by a new row in the PR that changes course. That's the whole procedure.

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
| `.claude/skills/*/SKILL.md` | `/land` runs the gates and opens the PR; `/decide` appends a decision row. Those are the only workflow commands |

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

**v0.0 through v0.4a are built, and most of v0.4b.** The window-engine spike returned **go** on the bond model (D45); its `bond.rs` was ported byte-identical, the spike repo has been archived, and `bond.rs` now evolves under its own tests like any other module (D66). v0.4b so far: chrome drawn from tokens in CSS (D72), Main owns playback and shows the spectrum on the radar ramp (D74), the 10-band EQ (D75), the playlist window, seams that glow and discharge (#9), 2x chrome (D76), the playlist's corner grip. Left in v0.4b: the viz stream (#6, #7), the windowshade contents (#8), and the sprite renderer (#3), which waits on the hand-drawn sheet (D73). Tracked as the v0.4b milestone on GitHub.

---

## How we work

Say what you want. The session builds it and lands it.

1. **Branch off `main`.** `<type>/<n>-<slug>` when there's an issue, `<type>/<slug>` when there isn't. Never commit to `main`; `tools/git-hooks/pre-push` refuses it anyway
2. **Build it.** Make the routine calls yourself and note them in the PR. A call worth keeping past this PR gets a row in `docs/decisions.md` (`/decide` appends it; it ships in the same PR as the code)
3. **Land it.** `/land` runs the gates, pushes, and opens the PR. The body says what changed and, when there's something to see on a screen, how to hand-test it
4. **The owner hand-tests and merges** on GitHub. Nothing merges from a session. Squash and rebase are disabled, so the merge bubble is the record of what changed together

Any session, any model. There is no planning session and no execution session; a session is disposable and the branch and PR carry everything.

**Gates:** `cargo fmt --check`, `cargo clippy --all-targets`, and `cargo test` on both crates, then `pnpm check`. Warnings are errors in both crates (`[lints]` in each `Cargo.toml`) and the compiler is pinned by `rust-toolchain.toml`, so the local run and CI (`.github/workflows/ci.yml`) agree by construction. A shipping exe comes from the `release-exe` workflow on demand.

**Issues** are the list of work that isn't happening yet. An issue body is the spec; there is no brief format to fill in first. Work that's happening now doesn't need an issue; the PR is enough.

**Hooks** in `.claude/settings.json` refuse a hardcoded colour under `src/` (a `tokens-exempt: <why>` comment on the line is the escape hatch), a `#[cfg(windows)]` outside `platform/`, and a whole-file rewrite of `decisions.md`. They load at session start; restart after changing them.

**Reviewers on request, not on schedule.** For a large or decision-heavy change, ask for the `decision-auditor` agent on the PR. For anything under `skins/`, `design/`, or the skin renderer, `skin-reviewer`. Neither is a step; a bug fix doesn't need an audit.

### Things that have burned us

- **Test the interaction the way a user performs it** for anything on screen. The v0.0 spike's scripted sweeps passed at 0 px error while the real interaction was dead, because an invisible window was eating the clicks (D43). Scripted paths and real input are not the same test. `tools/shot.ps1` screenshots a running window and `tools/input.ps1` clicks, double-clicks and drags at physical coordinates; a real double-click is how the dead bottom seams were found after every scripted check passed
- **Leave the app running from the session, on the PR branch.** A background `pnpm tauri dev` on the branch means the owner tests the build the PR describes; hot reload carries frontend edits in and a Rust change rebuilds and relaunches. Never two dev servers: the second fails on the port but still starts a second exe on top of the first, sharing the database
- **Verify against the real binary before believing a flag.** `--embed-thumbnail` looked right and hard-errors on webm; the fix only surfaced by running it
- **Leave the tree on the PR branch and say so** before a hand test. There's no worktree: `src-tauri/binaries/` and `node_modules/` aren't in git, and a hand test on the wrong branch tests nothing
- **Nothing lives only in a conversation.** A decision goes in `decisions.md`, a finding in a doc or on the issue. The next session starts cold
- **Nothing personal and no session links in git.** The repo is public. Commits end with the `Co-Authored-By` line only, never a `Claude-Session:` trailer; PR and issue text carries no session URL and no generated-with footer; paths are repo-relative, never a machine path; no emails or account details anywhere
- `.sid/` is a scratch folder for screenshots. Gitignored, never referenced by code

## Conventions

- Platform-specific calls (`windows-rs`, HWND owner tricks) go behind a trait. Don't scatter `#[cfg(windows)]`
- Scripts are Windows PowerShell 5.1, run as `powershell -NoProfile -ExecutionPolicy Bypass -File`. `pwsh` is not installed on the dev machine and nothing may assume it
- Snap and bond math is in **physical pixels**, converted at the boundaries. Mixing logical and physical here produces bugs that only appear on a second monitor
- Parse yt-dlp with `--progress-template`. Never scrape the human-readable progress bar
- Tauri events: a `listen()` with no target receives **every** emit, including an `emitTo` aimed at another window. Any event that more than one window listens to is listened to with `{ target: { kind: "WebviewWindow", label } }`. Main once wore the playlist's seam edges because of this
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
