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
       cargo clippy --manifest-path src-tauri/Cargo.toml --lib
       cargo test --manifest-path src-tauri/Cargo.toml --lib
       cargo test --manifest-path crates/hp-control/Cargo.toml
       pnpm check

   If fmt changed files, commit that on its own: `chore: rustfmt`.
3. **Commit** uncommitted work in the repo's voice, `<area>: <what> (#n)`, the why in the body.
4. `git push -u origin <branch>`.
5. **Pull request.** None open yet: `gh pr create --title "<subject>" --body-file <tmp> --label needs-hand-test`, plus `--milestone` from the issue. The body follows `.github/pull_request_template.md`: `Closes #n`; what changed, outcome first; the gates ticked, because you ran them; the **Hand test** copied from the issue's brief as an unticked checklist; the decisions relied on; a `## Noticed` list of what was seen and left alone. A PR already open: the push is enough; comment only if the hand-test steps changed.
6. Print the PR URL and one line of state: "gates green; awaiting hand test and merge on GitHub".

Never `gh pr merge`. Never `git push origin main`.
