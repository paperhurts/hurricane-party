# hurricane-party

Offline-first media library and player. Save YouTube playlists, videos, and MP3s to disk so they're watchable when the internet is down during a hurricane. Classic Winamp-style skinnable windows that magnetize to each other.

Windows-first. Tauri v2 + Svelte 5 + Vite. SQLite. yt-dlp, ffmpeg, and deno as sidecars.

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
| `.claude/skills/*/SKILL.md` | Before running a workflow command. `/brief`, `/implement`, `/delegate`, `/land`, `/audit`, `/decide`; the table under Workflow says which does what |

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

**v0.0 through v0.4a are built.** The window-engine spike returned **go** on the bond model (D45); its `bond.rs` was ported byte-identical, the spike repo has been archived, and `bond.rs` now evolves under its own tests like any other module (D66). Currently starting **v0.4b** — the skin renderer, EQ and analyser, briefed in `docs/v0.4-brief.md` and tracked as the v0.4b milestone on GitHub.

---

## Working agreements

- **Branch per issue, pull request, merge commit on GitHub.** Never commit straight to `main`; `tools/git-hooks/pre-push` refuses it. Squash and rebase are disabled on the repo, so the merge bubble is still the record of what changed together. The owner merges, after the hand test
- **Nothing lives only in a conversation.** Decisions go in `decisions.md`, findings go in a doc, working agreements go here. A session should be disposable
- **Nothing personal and no session links in git.** The repo is public. Commits end with the `Co-Authored-By` line only, never a `Claude-Session:` trailer; PR and issue text carries no session URL and no generated-with footer; paths are repo-relative, never a machine path; no emails or account details anywhere
- **Test the interaction the way a user performs it, at least once per stage.** The v0.0 spike's scripted sweeps passed at 0 px error while the real interaction was dead, because an invisible window was eating the clicks (D43). Scripted paths and real input are not the same test
- **Verify against the real binary before believing a flag.** `--embed-thumbnail` looked right and hard-errors on webm; the fix only surfaced by running it
- `.sid/` is a scratch folder for screenshots. Gitignored, never referenced by code

## Workflow

GitHub issues are the work list; the decision log is not. Every unit of work is an issue, briefed before it is built, and merged through a pull request the owner reviews on GitHub. Planning happens in one session (Fable), execution in another (Opus) or in a delegated subagent, so a session stays disposable and the issue carries the contract.

| Step | Who | How |
|---|---|---|
| Plan and scope | planning session | `/brief <n>` writes the contract into the issue: outcome, scope, decisions implicated, acceptance, hand test |
| Decide | the owner | `/decide` appends to `docs/decisions.md`. A choice the brief needs but nobody has made stops the work |
| Build | an execution session, or `/delegate <n>` from the planning session | `/implement <n>`: branch `<type>/<n>-<slug>`, code, tests, gates, then `/land` opens the PR |
| Review | planning session | `/audit <pr>` posts the decision-auditor's verdict on the PR |
| Hand test and merge | the owner | On a real screen, then merge on GitHub. Nothing merges from a session |

Gates: `cargo fmt --check`, `cargo clippy --all-targets`, and `cargo test` on both crates, then `pnpm check`. Warnings are errors in both crates (`[lints]` in each `Cargo.toml`) and the compiler is pinned by `rust-toolchain.toml`, so the local run and CI agree by construction; a new Rust release reaches the build only when the pin is bumped. `/land` runs them; CI (`.github/workflows/ci.yml`) repeats them on every PR. A shipping exe comes from the `release-exe` workflow on demand.

Hooks in `.claude/settings.json` refuse a hardcoded colour under `src/` (a `tokens-exempt: <why>` comment on the line is the escape hatch), a `#[cfg(windows)]` outside `platform/`, and a whole-file rewrite of `decisions.md`. They load at session start; restart after changing them.

Decisions carry provenance. A **(req)** row is the owner's business requirement; an **(adv)** row is advice the owner accepted. A requirement the owner states beats an (adv) row, and the fix is a superseding row via `/decide`, never a silent contradiction in code. `decision-auditor` raises these instead of failing them.

## Conventions

- Platform-specific calls (`windows-rs`, HWND owner tricks) go behind a trait. Don't scatter `#[cfg(windows)]`
- Scripts are Windows PowerShell 5.1, run as `powershell -NoProfile -ExecutionPolicy Bypass -File`. `pwsh` is not installed on the dev machine and nothing may assume it
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
