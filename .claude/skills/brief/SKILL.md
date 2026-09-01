---
name: brief
description: Write the implementation brief for a GitHub issue into the issue itself, so a cold session on any model can implement it. Usage: /brief 12  or  /brief "title of a new issue". Run from the planning session before anyone implements.
disable-model-invocation: true
allowed-tools: Read, Grep, Glob, Bash
---

Target: $ARGUMENTS

A number is an existing issue. Text is a new one: create it first with `gh issue create --title "<text>" --label <bug|feature|chore|documentation|debt> --body "Brief follows."`, and add `--milestone` when the work clearly belongs to one in the D27 table.

## Before writing a word

1. `gh issue view <n> --json title,body,labels,milestone,comments`. Read every comment; later comments override the body.
2. Read `docs/decisions.md` for every decision this touches, then the spec the work lives in (`docs/windows.md`, `skin-manifest.md`, `control-api.md`, `architecture.md`, `theme.md`, `v0.4-brief.md`). Then read the code that will change. A brief written from the issue text alone is a guess.
3. If the work needs a decision nobody has made, stop and ask the owner. The brief must not smuggle one in. Once the owner decides, `/decide` records it and the brief cites it.
4. Check each decision's provenance (preamble of `decisions.md`). An advisory decision that the owner's stated requirement contradicts is a supersession to raise, not a constraint to design around.

## Write the brief

Replace or add a `## Brief` section at the end of the issue body, keeping everything above it: write the full body to a temp file and `gh issue edit <n> --body-file <file>`. Every section present, even when short.

    ## Brief
    **Outcome.** One or two sentences in business terms, reviewable by anyone.
    **Scope.** What changes, as bullets. Modules and files likely touched.
    **Out of scope.** The tempting adjacent change, named, and that it is not this issue.
    **Decisions.** D-numbers this relies on, one line each on *how* the decision constrains the code. Mark any that are (adv) and might reasonably bend.
    **Acceptance.** A checklist a reviewer can tick from the diff and CI alone.
    **Hand test.** The interaction, performed the way a user performs it, monitor layout included when it matters. Screenshot to `.sid/`, attached to the PR. Automated tests are not this (D43).
    **Notes for the implementer.** Gotchas, the order that matters, what to run first. Assume a cold session with no memory of this one.
    **Size.** S / M / L, and whether this is a good first `/implement` for a fresh session.

Print the issue URL. Do not start implementing.
