# ralph-scripts — autonomous loop runner for pptx-compose

`loop.sh` drives an agent (Claude or Codex) over this repo's beads (`bd`) backlog,
one bounded iteration at a time. Ported from the asr ralph-scripts and adapted for
this **cleanroom Rust** PPTX engine.

## Two modes

| Mode | Prompt | What it does |
|------|--------|--------------|
| **plan** | `PROMPT_plan.md` | Multi-round, stateful drill-down: **one epic per round**, decomposed into **true atomic beads** — each carrying parent epic, spec ref, target crate/files, the exact Rust change, and a single `cargo` acceptance check. **Bounded and self-stopping** (see below). |
| **build** | `PROMPT_build.md` | Opt-in automation mode that implements exactly one ready non-epic bead per iteration (`cargo build`/`test`/`clippy` must pass), closes the bead, and commits the scoped change. Epics are skipped; if nothing buildable is ready it auto-pivots to plan mode. |

## Usage

```bash
./run.sh 3               # outer loop: test/file beads → consolidate → build, max 3 cycles
./loop.sh plan            # decompose backlog to atomic (the usual first step)
./loop.sh plan 3          # plan, max 3 rounds
./loop.sh                 # build loop, unlimited iterations
./loop.sh 5               # build loop, max 5 iterations
./loop.sh include-tests   # build + cargo + PPTX round-trip E2E each pass
./loop.sh include-tests skip-roundtrip-e2e
                          # build + cargo only
./loop.sh codex plan 2    # use OpenAI Codex instead of Claude
./loop.sh --help          # full flag reference
```

Running `loop.sh` or `run.sh` in build mode is explicit authority for the spawned
agent to commit the completed bead's scoped change. Outside this opt-in ralph
build context, the repository's conservative Beads policy still applies: agents
must not commit or push unless the current user or active profile clearly allows
it. Ralph never authorizes pushing to git or Dolt remotes.

Stop early: `touch .ralph-exit` in the repo root (consumed after the current round).
Override the target repo with `PROJECT_DIR=/path/to/repo`.

## Outer run mode

`run.sh` is the highest-level orchestrator. Each outer cycle starts by running the
roundtrip E2E test/file-Beads phase, then asks an agent to consolidate and
investigate the resulting Beads using `PROMPT_consolidate.md`, then switches to
`loop.sh` build mode to implement one Bead at a time until no open Beads remain or
the build phase stops making progress. It repeats until tests create no new Beads
and build has no remaining work, or until the outer cycle threshold is reached.

```bash
./run.sh              # default max outer cycles (3)
./run.sh codex 5      # Codex engine, max 5 outer cycles
MAX_OUTER_CYCLES=10 ./run.sh claude
```

Useful controls:

- `CONSOLIDATE_BEADS=false ./run.sh 1` skips the triage agent.
- `RUN_ROUNDTRIP_E2E=false ./run.sh 1` skips the test/file-Beads phase.
- `BUILD_STALL_LIMIT=3 ./run.sh` lets build mode tolerate more non-decreasing
  open-Bead counts before stopping the current outer cycle.

## Plan mode is bounded (no infinite iteration)

`loop.sh plan` does **one epic per round** and stops **deterministically** — it
does not rely on the agent remembering to write `.ralph-exit`. After each round the
loop itself computes a completion metric via `bd … --json | jq`: the number of open
`tier:task` beads whose description is still missing any of the four required
headers (`Spec ref:`, `Target crate`, `Change:`, `Acceptance:`). It then stops when:

- that count reaches **0** (every epic fully decomposed to atomic detail), or
- the count **doesn't drop for 3 consecutive rounds** (stall guard), or
- it hits the **hard safety cap** of 40 rounds (`PLAN_SAFETY_CAP` in `loop.sh`),

whichever comes first. A `… N` iteration cap and a manual `.ralph-exit` still work
on top of these. So a bare `./loop.sh plan` is guaranteed to terminate.

## Engines

- **claude** (default) — `claude -p` on the Anthropic subscription. `ANTHROPIC_API_KEY`
  is always unset by the script so it never bills the API account. Uses Opus.
  Includes the stream-json completion-detection workaround for the Claude Code hang
  bug (GH #19060/#25629/#31050).
- **codex** — `codex exec` on the ChatGPT subscription (`OPENAI_API_KEY` unset).
  Model from `~/.codex/config.toml`, override with `CODEX_BUILD_MODEL`.

## Notes specific to this repo

- **beads runs against the shared Dolt server** (`.beads/metadata.json`,
  `dolt_mode: server`). The script never runs `bd init`. A harmless
  `auto-importing … into empty database` banner may appear — it is a known cosmetic
  re-import and does not affect the JSONL/server state.
- **bd output is parsed by issue-ID pattern, not status glyphs.** bd decorates
  listings with Unicode glyphs (`○ ◐ ● ✓`) plus a footer legend; matching on those
  is fragile and prints control-char mojibake. `loop.sh` extracts
  `pptx-compose-<id>` and strips non-printable bytes before display (`clean_line`).
- **The `include-tests` phase runs cargo and round-trip E2E checks.** After
  `cargo test` + `cargo clippy` pass, `pptx_roundtrip_e2e.py` runs the V1
  no-edit agent flow for every non-malformed fixture in `fixtures/manifest.toml`:
  `inspect`, empty-patch `apply --dry-run`, empty-patch `apply --output`, and
  `validate`. It compares XML/package/media bytes, attempts a visual comparison
  when LibreOffice + `pdftoppm` + Pillow are available, writes
  `.ralph-roundtrip-e2e/roundtrip-summary.json`, and files deduplicated
  `defect:roundtrip-e2e` beads when defects are detected. Pass
  `skip-roundtrip-e2e` to keep cargo tests but skip this heavier check.
- **The edit E2E phase exercises the V1 agent edit surface.** Immediately after
  the round-trip phase, `pptx_edit_e2e.py` runs one representative patch per V1
  op (`replace_text`, `set_alt_text`, `add_text_box`, `move_resize_element`,
  `add_image`, `replace_image`). For each it `inspect`s a real fixture slide to
  discover an editable target, builds a schema-valid `pptx-compose.patch.v1`
  carrying the reported `document_id`/`base_revision` guards, runs `apply`, then
  asserts the targeted part(s) actually changed (with a greppable marker) while
  every unrelated part stays byte-identical and the embedded validation is
  clean. It then runs an edit-aware visual stage (soffice + pdftoppm + Pillow):
  the edited deck must still render, keep the same slide count, and produce no
  blank slides. The visual stage degrades to `inconclusive`/`skipped` (never a
  failure) when LibreOffice is absent or a synthetic fixture will not render.
  Summary at `.ralph-edit-e2e/edit-summary.json`.
- **The `cargo`/E2E test phase is skipped** while the repo is still spec-only (no
  `Cargo.toml`/`crates/`). It activates automatically once the workspace exists.
- **Cleanroom discipline** is baked into both prompts: implement from the spec
  suite / OOXML behavior, never port the legacy TypeScript line-by-line.

## Files

- `loop.sh` — the runner (engine dispatch, iteration control, beads integration).
- `run.sh` — outer runner (test/file Beads → consolidate → build → repeat).
- `pptx_roundtrip_e2e.py` — loop-integrated no-edit V1 agent flow checker.
- `pptx_edit_e2e.py` — loop-integrated edit round-trip: apply each V1 edit op to
  a real fixture, assert targeted-change + byte-preservation + validation, then
  run an edit-aware visual render check (degrades gracefully without LibreOffice).
- `tests/test_pptx_edit_e2e.py` — unit tests (always run) + integration tests
  (skipped unless `target/debug/pptx-compose` is built) for the edit harness.
- `PROMPT_consolidate.md` — agent prompt for investigating and deduplicating Beads.
- `PROMPT_plan.md` — planning/decomposition prompt (epic → atomic task).
- `PROMPT_build.md` — build prompt (implement one Rust task).
