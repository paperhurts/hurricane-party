---
name: land
description: Run the gates, push the branch, and open or update the pull request. Never merges; that happens on GitHub after the hand test. Usage: /land [issue number]
disable-model-invocation: true
---

## State
- Branch: !`git branch --show-current`
- Changes: !`git status --short`
- Ahead of main: !`git log --oneline main..HEAD`

Issue: $ARGUMENTS (when omitted, the number in the branch name `<type>/<n>-...`, if there is one)

1. **Refuse on main.**
2. **Gates**, stopping at the first failure and fixing it. Formatting is applied, not only checked:

       cargo fmt --manifest-path src-tauri/Cargo.toml
       cargo fmt --manifest-path crates/hp-control/Cargo.toml
       cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
       cargo clippy --manifest-path crates/hp-control/Cargo.toml --all-targets
       cargo test --manifest-path src-tauri/Cargo.toml --lib
       cargo test --manifest-path crates/hp-control/Cargo.toml
       pnpm check

   Warnings are errors in both crates and the compiler is pinned, so this list is exactly what CI runs. A docs-only diff skips the Rust gates.
3. **Commit** in the repo's voice: `<area>: <what> (#n)`, the why in the body. End the message with the `Co-Authored-By` line only: no `Claude-Session:` trailer, no session URL, no machine paths or personal data. The repo is public.
4. `git push -u origin <branch>`.
5. **Pull request.** None open yet: `gh pr create --title "<subject>" --body-file <tmp>`, `--milestone` from the issue when there is one, `--label needs-hand-test` when there is something to see on a screen. The body follows `.github/pull_request_template.md`: what changed, outcome first; `Closes #n` when there is an issue; the hand test as an unticked checklist when there is one; any decision made along the way, by D-number. No session link, no generated-with footer, repo-relative paths. A PR already open: the push is enough; comment only if the hand-test steps changed.
6. Print the PR URL. Leave the tree on this branch so the owner can hand-test, and say so.

Never `gh pr merge`. Never `git push origin main`.
