#!/bin/bash

# pptx-compose — Ralph Loop Runner
#
# This script and its PROMPT_*.md prompts live in <repo>/ralph-scripts/ and
# operate on the pptx-compose repo at $PROJECT_DIR (default ~/projects/pptx-compose,
# override with PROJECT_DIR=/path/to/repo).
#
# pptx-compose is a cleanroom Rust rewrite of a PPTX read/edit/write engine for
# AI agents. The backlog is tracked in beads (bd). This loop drives that backlog:
#   - plan mode   — decompose epics into TRUE ATOMIC tasks with full implementation
#                   info (target crate/files, exact change, cargo acceptance check).
#   - build mode  — implement exactly one ready atomic task per iteration in Rust.
#
# Usage:
#   ./loop.sh                    - Run build loop (build only)
#   ./loop.sh plan               - Run planning loop (decompose backlog)
#   ./loop.sh N                  - Run build loop for max N iterations
#   ./loop.sh plan N             - Run planning loop for max N iterations
#   ./loop.sh include-tests      - Run build loop with cargo test/clippy each pass
#   ./loop.sh include-tests N    - Run build loop with tests for max N iterations
#   ./loop.sh codex              - Run the loop with OpenAI Codex instead of Claude
#   ./loop.sh codex plan N       - Flags compose: engine + mode + iteration cap
#
# Engine (claude default, or codex):
#   claude — uses `claude -p` (Anthropic subscription; ANTHROPIC_API_KEY unset).
#   codex  — uses `codex exec` (ChatGPT subscription; OPENAI_API_KEY unset so it
#            never falls through to API-key billing). Select with the `codex`
#            arg or RALPH_ENGINE=codex.
#
# Models:
#   claude — Build/plan use Opus (--model opus).
#   codex  — Uses the model from ~/.codex/config.toml by default. Override with
#            CODEX_BUILD_MODEL.
#
# By default only the build phase runs. The cargo test/clippy phase is opt-in via
# the "include-tests" flag, and is skipped gracefully while the repo is still in
# the cleanroom spec-only state (no Cargo workspace yet).
#
# FIX for Claude Code hang bug (GitHub #19060, #25629, #31050):
# Claude completes work but never calls process.exit(). The process hangs
# indefinitely at 0% CPU with stdout open. Using --output-format stream-json
# lets us detect the {"type":"result"} event and kill the process ourselves.

set -e

# Safety: always unset ANTHROPIC_API_KEY so every claude invocation in this
# script (and any subprocess it spawns) uses the subscription, never API credits.
unset ANTHROPIC_API_KEY

MODE="build"
INCLUDE_TESTS=false
MAX_ITERATIONS=0
ITERATION=0
ENGINE="${RALPH_ENGINE:-claude}"              # claude (default) or codex
BUILD_MODEL="opus"                            # claude build/plan model
CODEX_BUILD_MODEL="${CODEX_BUILD_MODEL:-}"    # empty => codex config.toml default
HARD_TIMEOUT=2700  # 45min safety net (should never hit with stream-json detection)

# --help / -h : print usage and exit (before any side effects).
print_help() {
    cat <<'EOF'
pptx-compose Ralph Loop Runner — drives an agent (Claude or Codex) over the
pptx-compose repo, iterating on beads-tracked work until done or a cap is hit.

USAGE:
  ./loop.sh [claude|codex] [plan] [include-tests] [N]
  ./loop.sh --help | -h

POSITIONAL ARGS (order-independent, all optional, compose freely):
  plan            Run the planning loop (decompose specs/epics into true atomic
                  tasks) instead of the default build loop. Uses PROMPT_plan.md.
  include-tests   After each build iteration, also run `cargo test` and
                  `cargo clippy`. Opt-in; off by default. Skipped automatically
                  while there is no Cargo workspace yet. Ignored in plan mode.
  claude          Use the Claude engine (`claude -p`). This is the default.
  codex           Use the OpenAI Codex engine (`codex exec`) instead of Claude.
                  Equivalent to RALPH_ENGINE=codex.
  N               A bare integer caps the run at N iterations. 0 / omitted means
                  run until an exit signal or no work remains.

  Examples:
    ./loop.sh                  Build loop, Claude, unlimited iterations
    ./loop.sh plan             Planning loop (decompose backlog to atomic)
    ./loop.sh 5                Build loop, stop after 5 iterations
    ./loop.sh plan 3           Planning loop, max 3 iterations
    ./loop.sh include-tests    Build loop + cargo test/clippy each pass
    ./loop.sh codex plan 2     Codex engine, planning loop, max 2 iterations

ENGINES:
  claude  `claude -p` on the Anthropic subscription (ANTHROPIC_API_KEY is
          unset so it never bills the API account).
  codex   `codex exec` on the ChatGPT subscription (OPENAI_API_KEY is unset
          for the same reason). Called as the supported headless CLI directly.

MODELS:
  claude  Build/plan = Opus (--model opus).
  codex   Uses ~/.codex/config.toml default unless overridden via
          CODEX_BUILD_MODEL.

ENVIRONMENT VARIABLES:
  PROJECT_DIR         Repo to run against (default: ~/projects/pptx-compose).
  RALPH_ENGINE        Default engine when no claude/codex arg is given
                      (claude | codex; default claude). A positional arg wins.
  CODEX_BUILD_MODEL   Override the codex model for the build/plan phase
                      (default: codex config.toml default).
  ANTHROPIC_API_KEY   Always unset by this script (forces subscription auth).
  OPENAI_API_KEY      Unset for codex runs (forces ChatGPT-subscription auth).

BEHAVIOUR NOTES:
  - In build mode, only tier:task beads are picked up; epics are skipped. If no
    buildable task is ready, the iteration auto-pivots to plan mode to decompose.
  - Create a file named .ralph-exit in PROJECT_DIR to stop the loop after the
    current iteration (the file is consumed on exit). The plan prompt creates
    this itself once the backlog is fully atomic.
  - HARD_TIMEOUT (2700s) is a per-phase watchdog safety net.
EOF
}
for arg in "$@"; do
    case "$arg" in
        -h|--help|help) print_help; exit 0 ;;
    esac
done

# Absolute paths
# - SCRIPT_DIR: where loop.sh + PROMPT_*.md live (this directory)
# - PROJECT_DIR: the pptx-compose repo to run against; override with PROJECT_DIR=...
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="${PROJECT_DIR:-$HOME/projects/pptx-compose}"
if [ ! -d "$PROJECT_DIR" ]; then
    echo "Error: PROJECT_DIR does not exist: $PROJECT_DIR" >&2
    exit 1
fi
TEMP_OUTPUT=$(mktemp)
trap "rm -f $TEMP_OUTPUT" EXIT

# Kill any orphaned Claude processes from previous runs
cleanup_orphan_claude_processes() {
    local current_ppid=$$
    ps aux | grep -E "claude.*-p.*--dangerously-skip-permissions" | grep -v grep | while read -r line; do
        local pid=$(echo "$line" | awk '{print $2}')
        if [ "$pid" != "$current_ppid" ]; then
            kill "$pid" 2>/dev/null || true
        fi
    done
}

# Kill any orphaned Codex processes from previous runs (codex exec exits cleanly,
# so these are rare, but a watchdog-killed run can leave a stray child behind).
cleanup_orphan_codex_processes() {
    local current_ppid=$$
    ps aux | grep -E "codex exec.*--dangerously-bypass-approvals-and-sandbox" | grep -v grep | while read -r line; do
        local pid=$(echo "$line" | awk '{print $2}')
        if [ "$pid" != "$current_ppid" ]; then
            kill "$pid" 2>/dev/null || true
        fi
    done
}
cleanup_orphan_claude_processes
cleanup_orphan_codex_processes

# Run claude with stream-json and detect completion via result event.
# Returns 0 on successful result, 1 on timeout/no result.
run_claude_with_completion_detection() {
    local prompt_file="$1"
    local model="$2"
    local temp_out="$3"
    local err_log="${temp_out}.err"

    > "$temp_out"
    > "$err_log"

    # Start claude in background with stream-json output.
    # Prompt piped via stdin to handle large prompts; stdout=json, stderr=separate log.
    # Unset ANTHROPIC_API_KEY so claude uses the subscription, not the API billing account.
    cd "$PROJECT_DIR" && cat "$prompt_file" \
        | env -u ANTHROPIC_API_KEY claude -p --dangerously-skip-permissions --verbose \
            --output-format stream-json --model "$model" \
            > "$temp_out" 2>"$err_log" &
    local claude_pid=$!

    # Hard timeout watchdog (kills claude if stream-json detection fails)
    ( sleep $HARD_TIMEOUT; kill $claude_pid 2>/dev/null ) &
    local watchdog_pid=$!

    # Monitor stream-json output for the result event
    local result_received=false
    while kill -0 $claude_pid 2>/dev/null; do
        if grep -q '"type":"result"' "$temp_out" 2>/dev/null; then
            result_received=true
            # Give claude 3s to exit cleanly, then force kill
            ( sleep 3; kill $claude_pid 2>/dev/null ) &
            local killer_pid=$!
            wait $claude_pid 2>/dev/null
            kill $killer_pid 2>/dev/null
            break
        fi
        sleep 1
    done

    # Clean up watchdog
    kill $watchdog_pid 2>/dev/null
    wait $watchdog_pid 2>/dev/null
    wait $claude_pid 2>/dev/null

    # Final check: process may have exited (e.g. hook crash) after emitting the result
    # but before our polling loop caught it
    if [ "$result_received" = false ] && grep -q '"type":"result"' "$temp_out" 2>/dev/null; then
        result_received=true
    fi

    # Extract and display ONLY the final result text. The stream-json output is
    # JSONL; parse it as JSON (jq) and pull `.result` from the result event —
    # never echo raw stream lines, which carry decorated/ANSI content and are the
    # source of the "control chars / \033[0m on every line" terminal noise.
    local result_text
    if command -v jq >/dev/null 2>&1; then
        result_text=$(grep '"type":"result"' "$temp_out" \
            | jq -r 'select(.type=="result") | .result // empty' 2>/dev/null \
            | head -c 500)
    else
        result_text=$(grep '"type":"result"' "$temp_out" | python3 -c "
import sys, json
for line in sys.stdin:
    line = line.strip()
    if line:
        try:
            obj = json.loads(line)
            if obj.get('result'):
                print(obj['result'][:500])
                break
        except: pass
" 2>/dev/null)
    fi
    [ -n "$result_text" ] && printf '%s\n' "$result_text" | strip_ansi

    if [ "$result_received" = true ]; then
        echo "  (completed via stream-json result detection)"
        rm -f "$err_log"
        return 0
    else
        # Show stderr to help diagnose failures
        if [ -s "$err_log" ]; then
            echo "  stderr output:"
            head -5 "$err_log" | sed 's/^/    /'
        fi
        echo "  (no result event received)"
        rm -f "$err_log"
        return 1
    fi
}

# Run codex non-interactively via `codex exec`.
#
# `codex exec` is the supported headless entrypoint; it EXITS cleanly when the
# turn completes (so the stream-json hang workaround the Claude path needs does
# not apply here), and it has no plugin/Node dependency. We keep the
# orchestration (iteration control, beads, exit signal) in this loop.
#
# Returns 0 on a clean exit, 1 on nonzero exit or hard-timeout kill.
run_codex_with_completion_detection() {
    local prompt_file="$1"
    local model="$2"
    local temp_out="$3"
    local err_log="${temp_out}.err"
    local last_msg="${temp_out}.last"

    > "$temp_out"
    > "$err_log"
    > "$last_msg"

    # Pass --model only when explicitly configured; otherwise codex uses its
    # config.toml default.
    local model_args=()
    [ -n "$model" ] && model_args=(--model "$model")

    # --dangerously-bypass-approvals-and-sandbox mirrors claude's
    #   --dangerously-skip-permissions (fully headless, no approval prompts).
    # env -u OPENAI_API_KEY forces ChatGPT-subscription auth (~/.codex/auth.json
    #   tokens) instead of API-key billing — the codex analogue of the
    #   ANTHROPIC_API_KEY unset above.
    # -o writes the final agent message to a file so we can echo a short summary.
    # Prompt is piped via stdin (no PROMPT arg) to handle large prompts.
    # --color never: codex's activity stream ("[02:53] #38 /bin/bash -lc …")
    #   otherwise leaks raw ANSI escapes (\033[0m) into the terminal. NO_COLOR is
    #   belt-and-suspenders for any child process codex spawns.
    cat "$prompt_file" \
        | env -u OPENAI_API_KEY NO_COLOR=1 codex exec \
            --cd "$PROJECT_DIR" \
            --skip-git-repo-check \
            --dangerously-bypass-approvals-and-sandbox \
            --color never \
            "${model_args[@]}" \
            -o "$last_msg" \
            > "$temp_out" 2>"$err_log" &
    local codex_pid=$!

    # Hard timeout watchdog (codex exec normally exits on its own; this only
    # fires on a network/stall hang).
    ( sleep $HARD_TIMEOUT; kill $codex_pid 2>/dev/null ) &
    local watchdog_pid=$!

    wait $codex_pid 2>/dev/null
    local codex_rc=$?

    # Clean up watchdog
    kill $watchdog_pid 2>/dev/null
    wait $watchdog_pid 2>/dev/null

    # Echo the final agent message (truncated), like the claude result text.
    # -o writes the clean final message (not the decorated activity stream), and
    # strip_ansi guards against any embedded escapes.
    if [ -s "$last_msg" ]; then
        head -c 500 "$last_msg" | strip_ansi
        echo ""
    fi

    if [ "$codex_rc" -eq 0 ]; then
        echo "  (completed — codex exec exited 0)"
        rm -f "$err_log" "$last_msg"
        return 0
    else
        if [ -s "$err_log" ]; then
            echo "  stderr output:"
            head -5 "$err_log" | sed 's/^/    /'
        fi
        echo "  (codex exec exited $codex_rc — nonzero or hard-timeout kill)"
        rm -f "$err_log" "$last_msg"
        return 1
    fi
}

# --- bd output sanitizing -------------------------------------------------
# bd decorates listings with Unicode status glyphs (○ ◐ ● ✓ ❄), a "←" parent
# arrow, and a trailing legend/footer ("Status: ○ open  ◐ in_progress …").
# Parsing or echoing those lines directly is fragile (the footer legend also
# contains the glyphs) and leaks control-char-looking multibyte bytes to the
# terminal. So we never grep on glyphs: we extract the stable issue-ID pattern,
# and strip non-printable bytes before printing.
PREFIX="pptx-compose"   # bd issue-id prefix for this repo

# first_id / count_ids: read a bd listing on stdin, ignore decoration + footer.
first_id() { grep -oE "${PREFIX}-[a-z0-9]+" | head -1; }
count_ids() { grep -oE "${PREFIX}-[a-z0-9]+" | sort -u | wc -l | tr -d ' '; }
# clean_line: strip ANSI + all non-printable (UTF-8 glyph) bytes, collapse/trim.
# Use for single-line bd listings we parse/print.
clean_line() { LC_ALL=C sed -E $'s/\x1b\\[[0-9;]*m//g; s/[^[:print:]\t]+/ /g; s/^[[:space:]]+//; s/[[:space:]]+$//'; }
# strip_ansi: remove ANSI/CSI + OSC escape sequences only, preserving text and
# newlines. Use for multi-line agent result text we echo (codex/claude can embed
# escapes like \033[0m). Without this the loop leaks raw color codes to the term.
strip_ansi() { LC_ALL=C sed -E $'s/\x1b\\[[0-9;?]*[ -/]*[@-~]//g; s/\x1b\\][^\x07]*(\x07|\x1b\\\\)//g; s/\x1b[@-Z\\\\-_]//g'; }

# Engine dispatcher: route a phase to the configured agent.
#   $1 prompt file  $2 temp out  $3 claude model  $4 codex model
run_agent_with_completion_detection() {
    local prompt_file="$1"
    local temp_out="$2"
    local claude_model="$3"
    local codex_model="$4"
    if [ "$ENGINE" = "codex" ]; then
        run_codex_with_completion_detection "$prompt_file" "$codex_model" "$temp_out"
    else
        run_claude_with_completion_detection "$prompt_file" "$claude_model" "$temp_out"
    fi
}

# Parse arguments
for arg in "$@"; do
    if [ "$arg" = "plan" ]; then
        MODE="plan"
    elif [ "$arg" = "include-tests" ]; then
        INCLUDE_TESTS=true
    elif [ "$arg" = "codex" ]; then
        ENGINE="codex"
    elif [ "$arg" = "claude" ]; then
        ENGINE="claude"
    elif [ "$arg" -eq "$arg" ] 2>/dev/null; then
        MAX_ITERATIONS=$arg
    fi
done

# Validate engine and that its CLI is on PATH
case "$ENGINE" in
    claude|codex) ;;
    *) echo "Error: unknown ENGINE '$ENGINE' (expected 'claude' or 'codex')" >&2; exit 1 ;;
esac
if ! command -v "$ENGINE" >/dev/null 2>&1; then
    echo "Error: '$ENGINE' CLI not found on PATH" >&2
    exit 1
fi

# Human-readable model label for the per-phase banner
if [ "$ENGINE" = "codex" ]; then
    ACTIVE_BUILD_MODEL="codex:${CODEX_BUILD_MODEL:-default}"
else
    ACTIVE_BUILD_MODEL="claude:$BUILD_MODEL"
fi

echo "=== pptx-compose Ralph Loop ==="
echo "Engine: $ENGINE"
echo "Mode: $MODE"
echo "Tests: $INCLUDE_TESTS"
echo "Project: $PROJECT_DIR"
if [ $MAX_ITERATIONS -gt 0 ]; then
    echo "Max iterations: $MAX_ITERATIONS"
fi
echo ""

# Select prompt file (prompts live alongside loop.sh in SCRIPT_DIR)
if [ "$MODE" = "plan" ]; then
    PROMPT_FILE="$SCRIPT_DIR/PROMPT_plan.md"
else
    PROMPT_FILE="$SCRIPT_DIR/PROMPT_build.md"
fi

# Check prompt file exists
if [ ! -f "$PROMPT_FILE" ]; then
    echo "Error: prompt file not found: $PROMPT_FILE"
    exit 1
fi

# Ensure beads is initialised; gracefully degrade if not available.
# NOTE: pptx-compose uses the shared Dolt server (see .beads/metadata.json).
# Do NOT `bd init` here — the repo is already initialised against the server.
HAS_BEADS=false
if command -v bd >/dev/null 2>&1; then
    if ( cd "$PROJECT_DIR" && bd list >/dev/null 2>&1 ); then
        HAS_BEADS=true
    else
        echo "  ⚠ beads (bd) installed but not usable in $PROJECT_DIR — check .beads/ and the shared Dolt server"
    fi
else
    echo "  ⚠ beads (bd) not on PATH — task tracking commands will be skipped"
fi

# Main loop
while true; do
    ITERATION=$((ITERATION + 1))
    START_EPOCH=$(date +%s)

    # Re-derive prompt file each iteration so a plan-pivot doesn't persist
    if [ "$MODE" = "plan" ]; then
        PROMPT_FILE="$SCRIPT_DIR/PROMPT_plan.md"
    else
        PROMPT_FILE="$SCRIPT_DIR/PROMPT_build.md"
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Iteration $ITERATION — $(date)"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    # Show next bead to work on. In build mode, refuse to pick non-task beads
    # (epics are aggregates — they have no implementable acceptance) and
    # auto-pivot to plan mode if nothing implementable is ready.
    if [ "$HAS_BEADS" = true ]; then
        echo ""
        IN_PROGRESS=$(cd "$PROJECT_DIR" && bd list --status=in_progress 2>/dev/null | grep -E "${PREFIX}-[a-z0-9]+" | head -1 | clean_line)
        if [ -n "$IN_PROGRESS" ]; then
            echo "Resuming in-progress bead:"
            echo "  $IN_PROGRESS"
        elif [ "$MODE" = "build" ]; then
            # Prefer atomic tasks. Skip epics — they aren't buildable.
            NEXT_BUILDABLE=$(cd "$PROJECT_DIR" && bd list --ready --label tier:task --limit 0 2>/dev/null | grep -E "${PREFIX}-[a-z0-9]+" | head -1 | clean_line)
            if [ -n "$NEXT_BUILDABLE" ]; then
                echo "Next ready buildable task:"
                echo "  $NEXT_BUILDABLE"
            else
                READY_TOTAL=$(cd "$PROJECT_DIR" && bd ready --limit 0 2>/dev/null | count_ids)
                READY_EPICS=$(cd "$PROJECT_DIR" && bd list --ready --label tier:epic --limit 0 2>/dev/null | count_ids)
                echo "⚠ No buildable task ready (ready=$READY_TOTAL of which epics=$READY_EPICS)."
                echo "  Pivoting this iteration to PLAN mode to decompose into atomic tasks."
                PROMPT_FILE="$SCRIPT_DIR/PROMPT_plan.md"
                if [ ! -f "$PROMPT_FILE" ]; then
                    echo "  ⚠ Plan prompt missing; skipping iteration." ; continue
                fi
            fi
        else
            echo "Next ready bead:"
            NEXT_ANY=$(cd "$PROJECT_DIR" && bd ready 2>/dev/null | grep -E "${PREFIX}-[a-z0-9]+" | head -1 | clean_line)
            echo "  ${NEXT_ANY:-(no ready bead)}"
        fi
        echo ""
    fi

    # Phase 1: Build/plan
    # Uses stream-json to detect completion and kill hung process (GitHub #19060 fix)
    echo "  Phase 1: $MODE ($ACTIVE_BUILD_MODEL)"
    set +e
    run_agent_with_completion_detection "$PROMPT_FILE" "$TEMP_OUTPUT" "$BUILD_MODEL" "$CODEX_BUILD_MODEL"
    BUILD_EXIT=$?
    set -e

    BUILD_ELAPSED=$(( $(date +%s) - START_EPOCH ))
    echo ""
    echo "  $MODE phase completed (exit $BUILD_EXIT, ${BUILD_ELAPSED}s)"

    # Fallback: create tracking bead if build phase crashed without creating its own beads
    if [ $BUILD_EXIT -ne 0 ] && [ "$HAS_BEADS" = true ]; then
        echo "  ⚠ $MODE phase exited $BUILD_EXIT — checking for untracked failures..."
        EXISTING=$(cd "$PROJECT_DIR" && bd list --status=open 2>/dev/null | grep -c "Loop iteration.*$MODE.*crash" || echo "0")
        if [ "${EXISTING}" = "0" ]; then
            cd "$PROJECT_DIR" && bd create \
                --title="Loop iteration $ITERATION $MODE phase crash (exit $BUILD_EXIT)" \
                --type=bug \
                --priority=1 \
                --labels="loop,build-crash" 2>/dev/null || true
            echo "  Created fallback bead for $MODE phase failure"
        fi
    fi

    # Phase 1.5: cargo test + clippy (only with include-tests, build mode).
    # Skipped gracefully while the repo is still spec-only (no Cargo workspace).
    if [ "$INCLUDE_TESTS" = true ] && [ "$MODE" = "build" ]; then
        echo ""
        if [ -f "$PROJECT_DIR/Cargo.toml" ] || [ -d "$PROJECT_DIR/crates" ]; then
            echo "  Phase 1.5: cargo test + clippy"
            CARGO_LOG="$PROJECT_DIR/.ralph-cargo.log"
            set +e
            ( cd "$PROJECT_DIR" && cargo test --workspace --quiet && cargo clippy --workspace --all-targets -- -D warnings ) >"$CARGO_LOG" 2>&1
            CARGO_EXIT=$?
            set -e

            if [ $CARGO_EXIT -ne 0 ]; then
                echo "  ⚠ cargo test/clippy FAILED (exit=$CARGO_EXIT)"
                tail -15 "$CARGO_LOG" | sed 's/^/    /'
                if [ "$HAS_BEADS" = true ]; then
                    # Stable-defect dedup: every iteration's cargo failure points at the
                    # same wording-independent bead, identified by label
                    # `defect:cargo-failure`, with tier:task so the build phase picks it up.
                    CARGO_TAIL=$(tail -30 "$CARGO_LOG")
                    EXISTING_ID=$(cd "$PROJECT_DIR" && bd list --status=open --label defect:cargo-failure --limit 0 2>/dev/null \
                        | grep -oE 'pptx-compose-[a-z0-9]+' | head -1)
                    if [ -z "$EXISTING_ID" ]; then
                        CLOSED_ID=$(cd "$PROJECT_DIR" && bd list --status=closed --label defect:cargo-failure --limit 0 2>/dev/null \
                            | grep -oE 'pptx-compose-[a-z0-9]+' | head -1)
                        if [ -n "$CLOSED_ID" ]; then
                            cd "$PROJECT_DIR" && bd reopen "$CLOSED_ID" 2>/dev/null || true
                            cd "$PROJECT_DIR" && bd note "$CLOSED_ID" "Recurred in iteration $ITERATION. Exit $CARGO_EXIT. Tail:
$CARGO_TAIL" 2>/dev/null || true
                            echo "  Reopened bead $CLOSED_ID with new iteration context"
                        else
                            cd "$PROJECT_DIR" && bd create \
                                --title="cargo test/clippy failing (defect:cargo-failure)" \
                                --type=bug \
                                --priority=1 \
                                --labels="testing,tier:task,defect:cargo-failure" \
                                --description="Exit $CARGO_EXIT. Tail:
$CARGO_TAIL" 2>/dev/null || true
                            echo "  Filed canonical bead for cargo failure"
                        fi
                    else
                        cd "$PROJECT_DIR" && bd note "$EXISTING_ID" "Recurred in iteration $ITERATION. Exit $CARGO_EXIT. Tail:
$CARGO_TAIL" 2>/dev/null || true
                        echo "  Appended iteration $ITERATION context to existing bead $EXISTING_ID"
                    fi
                fi
            else
                echo "  ✓ cargo test + clippy passed"
            fi
            rm -f "$CARGO_LOG"
        else
            echo "  Phase 1.5: skipped — no Cargo workspace yet (cleanroom spec-only state)"
        fi
    fi

    ELAPSED=$(( $(date +%s) - START_EPOCH ))
    echo ""
    echo "Iteration $ITERATION completed (total ${ELAPSED}s)"
    echo ""

    # Check for explicit exit signal (file-based)
    if [ -f "$PROJECT_DIR/.ralph-exit" ]; then
        echo "Exit signal detected (.ralph-exit file found)"
        rm -f "$PROJECT_DIR/.ralph-exit"
        break
    fi

    # Check iteration limit
    if [ $MAX_ITERATIONS -gt 0 ] && [ $ITERATION -ge $MAX_ITERATIONS ]; then
        echo "Reached maximum iterations ($MAX_ITERATIONS)"
        break
    fi

    # Small delay between iterations to avoid hammering
    sleep 2
done

echo "=== Loop completed ==="
echo "Total iterations: $ITERATION"
