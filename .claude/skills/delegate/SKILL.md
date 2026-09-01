---
name: delegate
description: Hand a briefed issue to an Opus subagent to implement in the background while this session stays the planner. Usage: /delegate 12. Same contract as /implement, different executor.
disable-model-invocation: true
---

Issue: $ARGUMENTS

Check first: `gh issue view <n> --json body --jq .body` contains `## Brief`. If not, stop: "run /brief <n> first".

Spawn one subagent with the Agent tool: `subagent_type: general-purpose`, `model: opus`. No worktree isolation; the sidecars and node_modules are not in git. Its prompt, verbatim apart from the number:

> You are implementing GitHub issue #<n> in the repository at the current working directory. Read `CLAUDE.md`, then `docs/decisions.md`, then `.claude/skills/implement/SKILL.md`, and follow that skill file step by step exactly as if the user had typed `/implement <n>`, including the land step at the end. Stop and report instead of guessing if: the issue has no `## Brief`; the brief needs a decision nobody has made; a gate fails for a reason that is not yours to fix. Report back with the branch name, the PR URL, the gates' results, and everything you noticed but left alone.

While it runs, do not touch the working tree; the subagent owns the checkout. Write the next brief instead.

When it reports: run `/audit <pr>` and relay the verdict, then tell the owner the PR is ready for the hand test.
