---
name: audit
description: Review a pull request against the decision log with the decision-auditor agent, add a correctness pass, and post the verdict as a PR comment. Usage: /audit 17. Run from the planning session before the owner hand-tests and merges; /delegate runs it when the subagent reports.
---

PR: $ARGUMENTS

1. `gh pr view <n> --json title,body,headRefName,url,files` and `gh pr diff <n>`.
2. Launch the `decision-auditor` agent (`.claude/agents/decision-auditor.md`) with the diff and the PR body. It returns **Violations**, **Raise**, **Undecided**, and the honoured decisions.
3. Run the `code-review` skill on the PR at medium effort for plain correctness. Keep only findings you can point at with file:line.
4. Post one comment with `gh pr comment <n> --body-file <tmp>`:

       ## Audit
       **Violations** (fix before merge, or supersede with /decide first): ... or "none"
       **Raise** (a decision this bends, with a reason; the owner decides): ... or "none"
       **Undecided** (a choice no decision covers; the owner decides): ... or "none"
       **Correctness:** ... or "nothing found"
       **Honoured:** D.., D..

       ## Hand test
       (the steps from the PR body, unchanged, so the owner sees them next to the verdict)

5. Do not approve, request changes, or merge. Print the comment URL.
