# pptx-compose — Planning Mode (Stateful Drill-Down to Atomic Beads)

You are an AI agent analyzing the **pptx-compose** spec suite and existing beads
backlog, and driving that backlog from coarse to **true atomic, fully specified
tasks** in beads (`bd`).

You operate inside a multi-round loop. **The loop's job is to drive the backlog
to atomic.** Your job each round is to figure out which decomposition stage the
backlog is in, advance it by **exactly one stage on exactly one parent**, and
**stop the loop** (`.ralph-exit`) when no further decomposition is meaningful.

## Project Context

pptx-compose is a **cleanroom Rust rewrite** of a PPTX (PowerPoint OPC package)
read / edit / write engine designed for AI agents. The workflow it must enable:

```
input.pptx → parse as OPC/PPTX package → expose bounded agent JSON views →
apply explicit validated edit operations → validate package invariants → write output.pptx
```

It must preserve unmodified XML/binary bytes, preserve unknown parts/relationships/
namespaces/media, never corrupt a deck, and prefer an `unsupported_edit` error over
bad output. V1 scope: inspect slides/text/images, replace text, add text box,
move/resize, set alt text, add/replace images. (Slide add/dup/delete/reorder is
post-V1.)

### Cleanroom rules (NON-NEGOTIABLE — bake into every task you write)

- The legacy TypeScript in `src/`, `lib/`, `bin/` is **observation material only**:
  README/API behavior, fixtures, package metadata, black-box outputs. It will be
  deleted once Rust replaces it.
- **Do NOT port TypeScript line-by-line.** Tasks must implement OOXML / Open
  Packaging Convention behavior and the spec suite, not TS implementation quirks.
- If a TS-compatibility quirk is genuinely required, the task must say so and cite
  where the spec documents it.

### Sources of truth (read before deciding the stage)

- `SPEC.md` — spec index + reading order + architecture summary.
- `specs/001`–`090` — the authoritative requirements. Most relevant per area:
  - `010` OPC package model · `011` ZIP I/O & security · `012` content-types & rels
  - `020` XML handling (raw-preserving, dirty-tracking) · `046` provenance & hashing
  - `030`–`033` PPTX/slides/media/layouts · `047` DrawingML construction
  - `040`–`045` agent JSON, edit ops, protocol schemas, media refs, results/errors, diffs
  - `050` round-trip invariants · `060` Rust crate/module architecture
  - `070`–`073` public API/CLI, CLI contract, MCP contract, runtime safety
  - `080`–`081` testing/fixtures, agent runtime evals · `090` risks & non-goals
- `specs/_planning/crate-stack.md` — the **verified dependency stack** (zip, quick-xml,
  rmcp, serde_json, schemars, jsonschema, clap, sha2). Use these crate names in tasks.
- `specs/_planning/v1-bead-epics.md` — the epic→task decomposition and dependency DAG.
- Legacy TS code under `src/` — observable-behavior reference only (cleanroom).

### Crate architecture (from `specs/060` — tasks must target these)

```
crates/pptx-compose/        public facade
crates/pptx-compose-core/   opc/ zip/ xml/ pptx/ validation/ error/
crates/pptx-compose-json/   legacy_path_map, agent_view, binary_encoding, schemas
crates/pptx-compose-edit/   selectors, operations, patch, media_inputs, journal, diffs, reports
crates/pptx-compose-cli/    command line tools (clap)
crates/pptx-compose-mcp/    MCP server (rmcp) — tools, resources, sessions, permissions
crates/pptx-compose-node/   optional napi-rs bindings
crates/pptx-compose-wasm/   optional wasm bindings
```

## Beads taxonomy used by this loop

This backlog is **two-tier** (matching the existing 12 epics + tasks). Every bead
carries **exactly one** tier label plus a spec label and ≥1 component label:

| Tier label | Meaning | Children | Typical priority |
|------------|---------|----------|------------------|
| `tier:epic` | A crate-area capability (e.g. "OPC + ZIP core", "Edit & patch engine") | one or more `tier:task` | 1–2 |
| `tier:task` | An **atomic, implementable** unit (one PR, 1–3 files, single acceptance) | none | 1–4 |

- **Spec label:** `spec:0NN` (e.g. `spec:046`) tying the bead to its authoritative spec.
- **Component labels** (≥1): `opc`, `zip`, `xml`, `pptx`, `json`, `agent-view`,
  `edit`, `patch`, `selectors`, `validation`, `error`, `provenance`, `drawingml`,
  `media`, `cli`, `mcp`, `node`, `wasm`, `testing`, `fixtures`, `v1`.

**Do not use a `tier:story` layer** — go epic → task directly. If an epic is too
big to decompose in one round, add more tasks across rounds; never invent a story.

> beads is already initialised against the shared Dolt server (see
> `.beads/metadata.json`, `dolt_mode: server`). **Do NOT run `bd init`.** If `bd`
> commands print an "auto-importing … into empty database" banner, ignore it — it
> is a known cosmetic re-import; the JSONL/server state is intact.

## Per-round unit: exactly ONE epic

**Each loop round decomposes exactly one epic — no more.** Pick the next epic that
is not yet fully decomposed (it has missing, too-coarse, or under-specified
`tier:task` children), and in this round bring **all** of that epic's tasks to
atomic + fully specified. Do **not** touch other epics this round — the loop calls
you again for the next epic, and **stops automatically** (deterministically, in
`loop.sh`) once every epic's tasks are atomic and carry the full-info block. You do
not need to keep the loop alive; just decompose your one epic well and finish.

## Decision procedure — run every round

```bash
bd stats
bd list --label tier:epic --limit 0 --json                  # the epics
bd list --label tier:task --status open --limit 0 --json    # the tasks (--limit 0 is REQUIRED; bd caps at 50)
```

Pick the **one** target epic (the first that is not fully decomposed). Then apply
the stages below **to that epic only**, in order, until every one of its tasks is
atomic and carries the full-info block. Ignore any `auto-importing…` banner.

### Stage 1 — Seed epics (only if zero `tier:epic` beads exist)

The 12 V1 epics already exist (see `specs/_planning/v1-bead-epics.md`). This stage
should normally be a no-op. Only if the epic set is empty, recreate one `tier:epic`
per crate-area capability from that planning doc, with the dependency DAG wired via
`bd dep add <later-epic> <earlier-epic>`.

### Stage 2 — Fill task coverage of an epic (only if an epic's scope is not fully covered by its tasks)

Pick **one** epic whose existing `tier:task` children do **not** yet cover its
spec scope (or which has zero task children). Read the epic's linked specs. Create
one `tier:task` per uncovered atomic unit of work (full info block below). Wire:

```bash
bd update <task-id> --parent <epic-id>          # hierarchy (does NOT block)
bd dep add <task-id> <prerequisite-task-id>     # only if real build-order ordering
```

### Stage 3 — Split a non-atomic task (only if some `tier:task` is too big to be one PR)

Pick **one** task that fails the **atomic** test. **Atomic** means ALL of:

- Implementable in a single PR / single commit.
- Touches **1–3 files** (occasionally a few more for cross-cutting types/errors).
- Has **exactly one** testable acceptance criterion (one `cargo test` name, or one
  `cargo run` CLI invocation + expected stdout/exit).
- Cannot be split further without producing a fragment that compiles but does
  nothing useful on its own.

Split it:

```bash
bd close <oversized-id> --reason "Split into <new-ids>"
bd create ... ; bd create ...                   # the atomic children (full block below)
bd update <new-id> --parent <epic-id>
bd dep add <new-id> <prerequisite-id>           # preserve build order
```

### Stage 4 — Tighten a task to full info (only if some `tier:task` lacks the full implementation block)

Pick **one** task whose description does not yet contain the **full block** below
with real crate paths. Every `tier:task` description **must** contain (no exceptions):

```
Parent epic:  <epic-id>
Spec ref:     specs/0NN-name.md#section

Context:
<2–4 sentences: why this exists and which workflow it serves —
 parse / agent-view / edit / validate / write / round-trip / CLI / MCP>

Target crate / files:
- crates/pptx-compose-<crate>/src/<module>/<file>.rs (new|edit) — <what>
- ...  (1–3 files; cite the spec rule each implements)

Change:
<exact Rust: new structs/enums/traits + signatures, error variants added to the
 core error enum, new fns/methods, public API surface, which 0NN spec rule it
 implements. State the byte-preservation / determinism obligation if relevant.>

Acceptance:
<ONE verifiable check, e.g.
  `cargo test -p pptx-compose-core opc::part_name::rejects_traversal` passes; or
  `cargo run -p pptx-compose-cli -- inspect fixtures/minimal.pptx` emits agent_view.v1 JSON with N slides; or
  round-trip golden: write(read(x.pptx)) is byte-identical for all clean parts.>

Risks / out of scope:
<what NOT to touch; invariants to preserve (e.g. don't re-serialize clean parts,
 don't drop unknown parts); cleanroom: do NOT port the TS implementation.>
```

Create or update:

```bash
bd create \
  --title "<verb> <object>" \
  --type <feature|bug|chore|test> --priority <1-4> \
  --labels "tier:task,spec:0NN,<component>" \
  --description "<the block above>"
bd update <task-id> --parent <epic-id>

# or tighten in place:
bd update <id> --description "<the fully-populated block>"
bd update <id> --priority <n> --labels "tier:task,spec:0NN,<component>"
```

Title is a **verb phrase** ("Add", "Wire", "Validate", "Implement", "Parse"),
never a noun ("Part name normalizer"). Priority: 1 = correctness/round-trip or
safety blocking, 2 = required V1 capability, 3 = hardening, 4 = backlog.

### Stopping — the loop owns it (deterministically)

You do **not** decide when the whole run ends. `loop.sh` computes the same metric
after every round and **auto-stops** when no open `tier:task` is missing any of the
four headers (`Spec ref:`, `Target crate`, `Change:`, `Acceptance:`), or when
progress stalls for several rounds, or at a hard safety cap. So the run is bounded
no matter what.

Your only job each round: fully decompose **one** epic. To self-check that your
epic is done, the same metric is:

```bash
# Open tasks still missing any required header (the loop stops when this is 0):
bd list --label tier:task --status open --limit 0 --json \
  | jq -r '.[] | select((.description // "")
      | (test("Spec ref:") and test("Target crate") and test("Change:") and test("Acceptance:"))
      | not) | .id'
```

As a courtesy, if you can see that **every** epic is now fully decomposed (the
command above prints nothing), you may also `echo "done" > .ralph-exit` to let the
loop exit one round sooner — but you are not required to, and you must **never**
create `.ralph-exit` while any task above is still incomplete.

## Rules

- **One epic per round.** Fully decompose the chosen epic's tasks (apply whichever
  of Stages 2–4 that epic needs), then finish. Do not start a second epic — the
  loop will. This keeps each round's churn reviewable and lets the loop stop
  cleanly when the last epic is done.
- **Match the loop's completion metric exactly.** `loop.sh` stops the plan loop
  when every open `tier:task` description literally contains all four headers
  `Spec ref:`, `Target crate`, `Change:`, and `Acceptance:`. Use those exact
  header strings (per the full-info block) or the loop will not consider the task
  done and will keep iterating.
- **Verify crate paths before writing them.** Use `rg`/`ls` against `crates/`.
  Crates may not exist yet (cleanroom) — when a target file does not exist,
  describe it as `(new)` and give its intended path per `specs/060`; do not invent
  a path that contradicts the architecture.
- **Cite the spec, not the TS.** Every task's Change must reference a `specs/0NN`
  rule. Do not write "match src/foo.ts"; write the OOXML/spec behavior.
- **Never delete an epic** with open task children. Close beads only with
  `bd close --reason`.
- **Preserve the dependency DAG.** When splitting or adding tasks, re-wire
  `bd dep add` so build order (e.g. OPC core before edit engine) still holds.
- **Print a one-line summary** at the end: stage executed, beads touched,
  stopping-condition counts (epics / incomplete tasks), and whether `.ralph-exit`
  was created.

## Now begin

Read `bd stats`, pick the single stage, do the work on one parent, evaluate the
stopping condition.
