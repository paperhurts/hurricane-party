---
name: land
description: Run every gate, push the branch, and open or update the pull request with the hand-test checklist. Never merges; that happens on GitHub after the hand test. Usage: /land [issue number]
disable-model-invocation: true
---

## State
- Branch: !`git branch --show-current`
- Changes: !`git status --short`
- Ahead of main: !`git log --oneline main..HEAD`

Issue: $ARGUMENTS (when omitted, the number in the branch name `<type>/<n>-...`)

1. **Refuse on main.** Refuse if there are changes you did not make in this session and cannot explain.
2. **Gates**, in this order, stopping at the first failure and fixing it. Formatting is applied, not only checked:

       cargo fmt --manifest-path src-tauri/Cargo.toml
       cargo fmt --manifest-path crates/hp-control/Cargo.toml
       cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
       cargo clippy --manifest-path crates/hp-control/Cargo.toml --all-targets
       cargo test --manifest-path src-tauri/Cargo.toml --lib
       cargo test --manifest-path crates/hp-control/Cargo.toml
       pnpm check

   Warnings are errors in both crates (`[lints]` in each `Cargo.toml`) and the compiler is pinned by `rust-toolchain.toml`, so this list is exactly what CI runs; no flags. If fmt changed files, commit that on its own: `chore: rustfmt`.
3. **Commit** uncommitted work in the repo's voice, `<area>: <what> (#n)`, the why in the body. End the message with the `Co-Authored-By` line only: no `Claude-Session:` trailer, no session URL, no machine paths or personal data. The repo is public.
4. `git push -u origin <branch>`.
5. **Pull request.** None open yet: `gh pr create --title "<subject>" --body-file <tmp>`, plus `--milestone` from the issue, and `--label needs-hand-test` only when the brief's hand test names something to perform on a screen; a brief whose hand test is "None" gets no label. The body follows `.github/pull_request_template.md`: `Closes #n`; what changed, outcome first; the gates ticked, because you ran them; the **Hand test** copied from the issue's brief as an unticked checklist; the decisions relied on; a `## Noticed` list of what was seen and left alone. No session link and no generated-with footer in the body, and paths are repo-relative. A PR already open: the push is enough; comment only if the hand-test steps changed.
6. Print the PR URL and one line of state: "gates green; awaiting hand test and merge on GitHub".

Never `gh pr merge`. Never `git push origin main`.
