---
name: decide
description: Append a numbered decision to docs/decisions.md. Use when a call made while building should outlive the PR. Append-only; a superseded row is struck through with a pointer, never rewritten. Usage: /decide <the decision, and why>
allowed-tools: Read, Edit, Grep, Glob
---

Record: $ARGUMENTS

`docs/decisions.md` is the authoritative log (CLAUDE.md precedence). The session writes the row; the owner reads it in the PR diff and overrules there if it's wrong.

1. **Next number.** Grep `^\| ~?~?D[0-9]+ \|` across the file and take the highest plus one. The table is not in numeric order (D46–D64 sit between D25 and D26), so the last row's number is wrong by construction. Same for O-numbers: `^\| ~?~?O[0-9]+`.
2. **Decision or question?** An open question with no resolution goes in the "Still open" table as O## instead. Questions are promoted to D-numbers when resolved.
3. **Append the row** at the end of the main D-table, matching the existing shape: `| D## | Topic | Resolution |`. What was decided, why, and the road not taken, in the voice of the rows around it. End with the provenance tag: **(req)** when the owner asked for it, **(adv)** when the session proposed it and the owner went along.
4. **Superseding an older row:** wrap its text in `~~ ~~` and end it with `— **superseded by D##**`. Never delete a row, never renumber, never reorder.
5. **Stale statements.** Grep `CLAUDE.md`, `README.md`, and `docs/*.md` for statements this decision makes wrong and fix them in the same PR.

The row ships in the PR that carries the work. A separate `docs/` branch only when there is no work, just the decision.
