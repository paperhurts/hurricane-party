---
name: implement
description: Implement a briefed GitHub issue end to end - branch, code, tests, gates, pull request. Refuses an issue with no ## Brief. Usage: /implement 12. Runs in any session on any model; the brief is the contract.
disable-model-invocation: true
---

Issue: $ARGUMENTS

## Ground rules

- `docs/decisions.md` outranks everything; `CLAUDE.md` has the non-negotiables and conventions. Read both before the first edit, even if you think you know them.
- The brief is the contract. Where the brief and the code disagree about what exists, the code is right and the brief is stale: say so in the PR body, do not improvise silently.
- Stop and ask when the work needs a decision the brief did not make. Do not infer one; the decision log exists so choices get made once, on purpose.
- Never commit to main. Never push main. A git hook refuses it anyway.
- No worktree: `src-tauri/binaries/` and `node_modules/` are not in git, and a worktree has neither.

## Steps

1. `gh issue view <n> --json title,body,labels,milestone,comments`. No `## Brief` section: stop and say "run /brief <n> first". Read every comment.
2. Start clean: `git status --short` empty, then `git checkout main && git pull --ff-only`.
3. Branch `<type>/<n>-<slug>`: type is `feat` | `fix` | `chore` | `docs` from the label, slug is two to four words. Example: `fix/14-clippy-warnings`.
4. Do the work in the brief's scope. Anything outside it goes in the PR's `## Noticed` list, untouched.
5. Tests: extend or add them wherever an acceptance item can be tested. Rust tests sit next to the code in `#[cfg(test)]`; run the file's tests as you go.
6. Run the gates yourself before landing, and fix what fails:

       cargo fmt --manifest-path src-tauri/Cargo.toml
       cargo fmt --manifest-path crates/hp-control/Cargo.toml
       cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
       cargo clippy --manifest-path crates/hp-control/Cargo.toml --all-targets
       cargo test --manifest-path src-tauri/Cargo.toml --lib
       cargo test --manifest-path crates/hp-control/Cargo.toml
       pnpm check

   Warnings are errors in both crates and the compiler is pinned, so a warning is a failed gate here exactly as it is in CI.

7. Commit in the repo's voice: subject `<milestone or area>: <what changed> (#<n>)`, the *why* in the body. Several commits when the work has several steps; one when it does not. End each message with the `Co-Authored-By` line only: no `Claude-Session:` trailer, no session URL, no machine paths or personal data, in commits or in the PR. The repo is public.
8. A decision made along the way went through `/decide` (the owner's call); cite the D-number in the commit body.
9. Finish by following `.claude/skills/land/SKILL.md` step by step: it pushes and opens the PR with the hand-test checklist. The hand test is the owner's, on a real screen, before merge.
