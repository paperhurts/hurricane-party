---
name: decide
description: Append a numbered decision to docs/decisions.md. Append-only; superseded rows are struck through with a pointer, never rewritten. Use when a choice has been made that must outlive this session. Usage: /decide <the decision, and why>
disable-model-invocation: true
allowed-tools: Read, Edit, Grep, Glob
---

Record: $ARGUMENTS

`docs/decisions.md` is the authoritative log (CLAUDE.md precedence). Read its preamble first; it carries the append-only and provenance rules this skill enforces.

1. **Find the next number.** Grep `^\| D[0-9]+ \|` across the file and take the highest plus one. The table is not in numeric order (D46–D64 sit between D25 and D26), so the last row's number is wrong by construction. Same for O-numbers: `^\| ~?~?O[0-9]+`.
2. **Decision or question?** If $ARGUMENTS reads as an open question with no resolution, add it to the "Still open" table as O## instead and say so. Questions are promoted to D-numbers when resolved.
3. **Append the row** at the end of the main D-table, matching the existing shape: `| D## | Topic | Resolution |`. The resolution says what was decided, why, and the road not taken, in the voice of the existing rows. End it with the provenance tag: **(req)** for the owner's business requirement, **(adv)** for a recommendation the owner accepted. Ask which if the conversation does not make it obvious.
4. **Superseding an older row:** wrap its text in `~~ ~~` and end it with `— **superseded by D##**`. Never delete a row, never renumber, never reorder.
5. **Stale statements.** Grep `CLAUDE.md`, `README.md`, and `docs/*.md` for statements this decision makes wrong. List them as `file:line`. Do not edit them here; they are fixed in the PR that carries the work, or in a docs PR when there is no work.
6. **Where the row lands.** If this session is also doing the work, leave the row uncommitted; it goes in the work's PR. If the work belongs to another session (the usual case: the planning session decides, an execution session implements), commit the row on its own branch `docs/d##-<slug>`, push, and open a PR labelled `documentation`, so `/implement` starts from a main that already carries it. Then comment on the issue: the decision is recorded, and which acceptance item it satisfies. Never commit on main.
