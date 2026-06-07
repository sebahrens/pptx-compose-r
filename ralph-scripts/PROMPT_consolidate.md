# pptx-compose — Bead Consolidation / Investigation Mode

You are an AI agent triaging the pptx-compose Beads backlog after an automated
test pass filed defects.

## Goal

Consolidate, investigate, and improve the open Beads so the build loop can work
on the smallest correct set of actionable issues.

## Required workflow

1. Inspect open testing/round-trip/cargo defect Beads first:
   ```bash
   bd list --status=open --limit 0
   bd list --status=open --label defect:roundtrip-e2e --limit 0
   bd list --status=open --label defect:cargo-failure --limit 0
   ```
2. Read the associated logs/reports when a Bead mentions artifacts such as
   `.ralph-roundtrip-e2e/roundtrip-summary.json` or fixture-specific logs.
3. Consolidate duplicates:
   - Prefer one Bead per root cause when evidence shows multiple fixtures fail
     for the same underlying code path.
   - Update the kept Bead with affected fixtures, logs, exact failure signals,
     suspected files, and acceptance criteria.
   - Close duplicate Beads with a clear reason pointing to the kept Bead.
4. Improve weak Beads instead of leaving vague work:
   - Add where to look, what command reproduced it, what output proves the bug,
     and what acceptance check should pass.
5. If investigation reveals a distinct new bug, file a new Bead with full
   reproduction details.

## Rules

- Use `bd` only; do not create markdown TODO files.
- Do not edit production code in this mode.
- Do not close a Bead unless it is genuinely duplicate, obsolete, or already
  fixed by current code.
- Keep changes conservative: the next build pass should see fewer, clearer,
  more actionable Beads.

## Done condition

Stop after one bounded triage pass. Summarize:

- Beads updated
- Beads closed as duplicates/obsolete
- New Beads filed
- Any uncertainty the build loop should preserve
