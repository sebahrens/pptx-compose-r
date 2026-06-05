# ralph-scripts — autonomous loop runner for pptx-compose

`loop.sh` drives an agent (Claude or Codex) over this repo's beads (`bd`) backlog,
one bounded iteration at a time. Ported from the asr ralph-scripts and adapted for
this **cleanroom Rust** PPTX engine.

## Two modes

| Mode | Prompt | What it does |
|------|--------|--------------|
| **plan** | `PROMPT_plan.md` | Multi-round, stateful drill-down: **one epic per round**, decomposed into **true atomic beads** — each carrying parent epic, spec ref, target crate/files, the exact Rust change, and a single `cargo` acceptance check. **Bounded and self-stopping** (see below). |
| **build** | `PROMPT_build.md` | Implements exactly one ready `tier:task` in Rust per iteration (`cargo build`/`test`/`clippy` must pass), closes the bead, commits. Epics are skipped; if nothing buildable is ready it auto-pivots to plan mode. |

## Usage

```bash
./loop.sh plan            # decompose backlog to atomic (the usual first step)
./loop.sh plan 3          # plan, max 3 rounds
./loop.sh                 # build loop, unlimited iterations
./loop.sh 5               # build loop, max 5 iterations
./loop.sh include-tests   # build + `cargo test` + `cargo clippy` each pass
./loop.sh codex plan 2    # use OpenAI Codex instead of Claude
./loop.sh --help          # full flag reference
```

Stop early: `touch .ralph-exit` in the repo root (consumed after the current round).
Override the target repo with `PROJECT_DIR=/path/to/repo`.

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
- **The `cargo` test phase is skipped** while the repo is still spec-only (no
  `Cargo.toml`/`crates/`). It activates automatically once the workspace exists.
- **Cleanroom discipline** is baked into both prompts: implement from the spec
  suite / OOXML behavior, never port the legacy TypeScript line-by-line.

## Files

- `loop.sh` — the runner (engine dispatch, iteration control, beads integration).
- `PROMPT_plan.md` — planning/decomposition prompt (epic → atomic task).
- `PROMPT_build.md` — build prompt (implement one Rust task).
