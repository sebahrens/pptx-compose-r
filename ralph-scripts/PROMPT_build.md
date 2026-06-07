# pptx-compose — Build Mode

You are an AI agent implementing one atomic task for **pptx-compose**, a cleanroom
**Rust** rewrite of a PPTX (PowerPoint OPC package) read / edit / write engine for
AI agents. It parses a `.pptx` as an OPC package, exposes bounded agent-facing JSON
views, applies explicit validated edit operations, validates package invariants,
and writes a valid `.pptx` — preserving unmodified bytes and unknown content.

## Cleanroom rules (NON-NEGOTIABLE)

- The legacy TypeScript in `src/`, `lib/`, `bin/` is **observation material only**
  (README/API behavior, fixtures, package metadata, black-box outputs). It will be
  deleted once Rust replaces it.
- **Do NOT port TypeScript line-by-line.** Implement OOXML / Open Packaging
  Convention behavior and the spec suite, independently designed in Rust.
- Prefer a clear `unsupported_edit` error over producing a corrupt deck.
- Never add media by only adding a file — update `[Content_Types].xml`,
  relationships, and the slide XML together (see `specs/012`, `specs/047`).
- Never re-serialize unmodified parts by default; preserve their raw bytes
  (`specs/020`, `specs/050`). Only dirty parts are serialized.

## Project Structure (target architecture — `specs/060`)

A Rust workspace (`crates/…`). Crates may not all exist yet — create them per
`specs/060` as tasks require:

- `crates/pptx-compose/` — public facade
- `crates/pptx-compose-core/` — `opc/` `zip/` `xml/` `pptx/` `validation/` `error/`
- `crates/pptx-compose-json/` — legacy + agent JSON, schemas (serde_json, schemars)
- `crates/pptx-compose-edit/` — selectors, operations, patch, media_inputs, journal, diffs, reports
- `crates/pptx-compose-cli/` — `clap`-based CLI (`specs/070`, `071`)
- `crates/pptx-compose-mcp/` — MCP server via `rmcp` (`specs/072`, `073`)
- `specs/` — authoritative spec suite (start at `SPEC.md`)
- `specs/_planning/crate-stack.md` — the verified dependency stack to use

## Verified dependency stack (`specs/_planning/crate-stack.md`)

`zip` 8.6 (raw_copy_file for clean parts) · `quick-xml` 0.40 (raw-preserving XML) ·
`rmcp` 1.7 (official MCP SDK, tokio) · `serde_json` 1 · `schemars` 1.2 · `jsonschema`
0.46 (Draft 2020-12) · `clap` 4.6 · `sha2` 0.11 (SHA-256 — `specs/046` pins it; do
NOT use blake3). Pin exact versions; pin `jsonschema` tightly (0.x).

## Workflow

### Commit authority

This prompt is used only by `ralph-scripts/loop.sh` and `run.sh` build mode.
Running those scripts in build mode is explicit opt-in authority for the spawned
agent to commit the single completed bead's scoped change after validation and
`bd close`. The authority is intentionally narrow:

- commit only the files needed for this one bead;
- do not commit unrelated working-tree changes;
- do not push git or Dolt remotes;
- outside this ralph build-mode prompt, follow the repository's conservative
  Beads policy and do not commit unless the current user or active profile
  clearly allows it.

### 1. Find your task

```bash
bd list --status in_progress
```

If any exist, resume the first (check `bd show <id>`). Otherwise:

```bash
bd ready
```

Pick the **first ready issue that is not an epic** — any non-epic type (task,
bug, feature) is buildable; only epics are aggregates with no implementable
acceptance, so skip those. Do NOT filter on a `tier:task` label: a task created
without that label is still buildable and must not be skipped. If nothing
non-epic is ready, the orchestrator loop will pivot to plan mode; just stop.

> beads runs against the shared Dolt server — **do NOT `bd init`**. Ignore any
> "auto-importing … into empty database" banner; it is a known cosmetic re-import.

### 2. Claim and understand the task

```bash
bd update <id> --status in_progress
bd show <id>
```

The description contains: parent epic, spec ref, context, target crate/files,
exact change, the single acceptance criterion, and risks/out-of-scope. Read the
cited `specs/0NN` section and the named files before changing anything.

### 3. Implement

- Read the affected files (and the spec section) before editing.
- Follow existing patterns in the crate you're modifying; keep the change within
  the task's stated scope and file list.
- Rust conventions:
  - Idiomatic, `#![deny(warnings)]`-clean code; no `unwrap()`/`panic!` on
    malformed user input (`specs/060`: "no malformed input should panic").
  - Structured errors carry the stable `code` from `specs/044` (single source of
    error-code names). Don't invent error codes absent from `044`.
  - Stable schemas via `schemars`; validate against Draft 2020-12 with `jsonschema`.
  - Preserve byte-faithfulness: clean parts copied from raw bytes; dirty parts use
    the deterministic serialization profile (`specs/020`, `046`, `050`).
  - SHA-256 digests are `sha256:` + 64 lowercase hex (`specs/046`).

### 4. Build and test

All must pass before closing the task (run from repo root):

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Add a `#[test]` (or integration test under `tests/`) for new behavior; add a
regression test when fixing a bug. The task's Acceptance line names the exact check
that must pass.

### 5. Complete the task

```bash
bd close <id> --reason "Implemented: brief description of what was done"
```

### 6. Commit

This commit step is authorized only in the opt-in ralph build-mode context
described above.

```bash
git add <changed-files-for-this-bead>
git commit -m "[<id>] Brief description of change"
```

## Rules

- **One task at a time** — finish fully before starting another.
- **Stay in scope** — only touch the files the task names (or update callers you
  must, found via `rg`).
- **Build and test before closing** — `cargo build` + `cargo test` + `cargo clippy`
  must pass.
- **Never port the TypeScript** — implement from the spec/OOXML behavior.
- **Don't break round-trip invariants** — if unsure whether a change re-serializes
  a clean part or drops unknown content, stop and re-read `specs/050`.
- **Discover work** — if you find something genuinely needed but out of scope:
  ```bash
  bd create --title "<verb> <object>" --type bug --priority 2 \
    --labels "tier:task,spec:0NN,<component>" --description "<full info block>"
  ```

## Now begin

Find the next ready task, implement it in Rust, and **stop**. Do exactly ONE task
per invocation — after closing the bead and committing, you are done. Do not look
for more work.

> **Stopping means: end your turn.** Just finish your response. The orchestrator
> loop owns iteration control. Do **NOT** create, touch, or write a `.ralph-exit`
> file (or any other loop-control file) — that is the plan-mode/user stop signal,
> not yours. Writing it in build mode wrongly kills the whole loop after one task.
