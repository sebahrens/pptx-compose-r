#!/bin/bash

# pptx-compose — outer test/triage/build runner
#
# This script wraps loop.sh with an outer cycle:
#   1. run E2E tests that file Beads,
#   2. consolidate/investigate/update those Beads,
#   3. build one Bead at a time until no open Beads remain or progress stalls,
#   4. repeat until tests create no more Beads or MAX_OUTER_CYCLES is reached.

set -e

unset ANTHROPIC_API_KEY

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="${PROJECT_DIR:-$HOME/projects/pptx-compose}"
PREFIX="pptx-compose"
ENGINE="${RALPH_ENGINE:-claude}"
MAX_OUTER_CYCLES="${MAX_OUTER_CYCLES:-3}"
BUILD_STALL_LIMIT="${BUILD_STALL_LIMIT:-2}"
CONSOLIDATE_BEADS="${CONSOLIDATE_BEADS:-true}"
RUN_ROUNDTRIP_E2E="${RUN_ROUNDTRIP_E2E:-true}"
CODEX_BUILD_MODEL="${CODEX_BUILD_MODEL:-}"
BUILD_MODEL="opus"

print_help() {
    cat <<'EOF'
pptx-compose outer runner — test, file Beads, consolidate, build, repeat.

USAGE:
  ./run.sh [claude|codex] [N]
  ./run.sh --help | -h

POSITIONAL ARGS:
  claude|codex   Agent engine for consolidation and build loop. Default claude.
  N              Maximum outer test/triage/build cycles. Default 3.

ENVIRONMENT:
  PROJECT_DIR          Repo root. Default ~/projects/pptx-compose.
  MAX_OUTER_CYCLES     Outer cycle cap when N is omitted. Default 3.
  BUILD_STALL_LIMIT    Stop build phase after this many non-decreasing open-Bead
                       counts. Default 2.
  CONSOLIDATE_BEADS    true|false. Run agent triage prompt. Default true.
  RUN_ROUNDTRIP_E2E    true|false. Run test/file-Beads phase. Default true.
  RALPH_ENGINE         Default engine when no positional engine is supplied.

STOP CONDITIONS:
  - No open Beads remain and the next test phase creates no new Beads.
  - The test phase creates no new Beads and the build phase cannot reduce open
    Bead count.
  - The outer cycle reaches MAX_OUTER_CYCLES.
EOF
}

for arg in "$@"; do
    case "$arg" in
        -h|--help|help) print_help; exit 0 ;;
        claude|codex) ENGINE="$arg" ;;
        ''|*[!0-9]*) echo "Error: unknown argument '$arg'" >&2; exit 1 ;;
        *) MAX_OUTER_CYCLES="$arg" ;;
    esac
done

if [ ! -d "$PROJECT_DIR" ]; then
    echo "Error: PROJECT_DIR does not exist: $PROJECT_DIR" >&2
    exit 1
fi
if ! command -v bd >/dev/null 2>&1; then
    echo "Error: bd is required for run.sh orchestration" >&2
    exit 1
fi
case "$ENGINE" in
    claude|codex) ;;
    *) echo "Error: unknown ENGINE '$ENGINE'" >&2; exit 1 ;;
esac
if ! command -v "$ENGINE" >/dev/null 2>&1; then
    echo "Error: '$ENGINE' CLI not found on PATH" >&2
    exit 1
fi

open_bead_count() {
    ( cd "$PROJECT_DIR" && bd list --status=open --limit 0 --json 2>/dev/null ) \
        | grep -v 'auto-import' \
        | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))' 2>/dev/null || echo "0"
}

run_tests_and_file_beads() {
    if [ "$RUN_ROUNDTRIP_E2E" != true ]; then
        echo "  Test/file-Beads phase skipped (RUN_ROUNDTRIP_E2E=$RUN_ROUNDTRIP_E2E)"
        return 0
    fi

    echo "  Test/file-Beads phase: roundtrip-e2e"
    local log="$PROJECT_DIR/.ralph-run-roundtrip.log"
    set +e
    ( cd "$PROJECT_DIR" && python3 "$SCRIPT_DIR/pptx_roundtrip_e2e.py" --project-dir "$PROJECT_DIR" --file-beads ) >"$log" 2>&1
    local rc=$?
    set -e
    tail -20 "$log" | sed 's/^/    /'
    rm -f "$log"
    if [ $rc -ne 0 ]; then
        echo "  roundtrip-e2e found issues and filed/deduped Beads where possible"
    else
        echo "  roundtrip-e2e found no issues"
    fi
    return 0
}

run_agent_prompt() {
    local prompt_file="$1"
    local log="$2"
    if [ "$ENGINE" = "codex" ]; then
        local model_args=()
        [ -n "$CODEX_BUILD_MODEL" ] && model_args=(--model "$CODEX_BUILD_MODEL")
        cat "$prompt_file" | env -u OPENAI_API_KEY NO_COLOR=1 codex exec \
            --cd "$PROJECT_DIR" \
            --skip-git-repo-check \
            --dangerously-bypass-approvals-and-sandbox \
            --color never \
            "${model_args[@]}" >"$log" 2>&1
    else
        cat "$prompt_file" | env -u ANTHROPIC_API_KEY claude -p \
            --dangerously-skip-permissions \
            --model "$BUILD_MODEL" >"$log" 2>&1
    fi
}

consolidate_and_investigate_beads() {
    if [ "$CONSOLIDATE_BEADS" != true ]; then
        echo "  Consolidation phase skipped (CONSOLIDATE_BEADS=$CONSOLIDATE_BEADS)"
        return 0
    fi
    if [ "$(open_bead_count)" -eq 0 ]; then
        echo "  Consolidation phase skipped: no open Beads"
        return 0
    fi

    echo "  Consolidation phase: investigate/update Beads"
    local prompt="$SCRIPT_DIR/PROMPT_consolidate.md"
    local log="$PROJECT_DIR/.ralph-run-consolidate.log"
    if [ ! -f "$prompt" ]; then
        echo "  ⚠ consolidation prompt missing: $prompt"
        return 0
    fi
    set +e
    run_agent_prompt "$prompt" "$log"
    local rc=$?
    set -e
    if [ $rc -ne 0 ]; then
        echo "  ⚠ consolidation agent exited $rc"
    fi
    tail -20 "$log" | sed 's/^/    /'
    rm -f "$log"
}

run_build_until_no_open_beads() {
    echo "  Build phase: loop.sh build until no open Beads or progress stalls"
    local previous current stalls
    previous=$(open_bead_count)
    stalls=0
    while [ "$previous" -gt 0 ]; do
        "$SCRIPT_DIR/loop.sh" "$ENGINE" include-tests skip-roundtrip-e2e 1 || true
        current=$(open_bead_count)
        echo "  Open Beads after build iteration: $current"
        if [ "$current" -eq 0 ]; then
            return 0
        fi
        if [ "$current" -ge "$previous" ]; then
            stalls=$((stalls + 1))
            echo "  Build phase did not reduce open Beads (stall $stalls/$BUILD_STALL_LIMIT)"
            if [ "$stalls" -ge "$BUILD_STALL_LIMIT" ]; then
                return 1
            fi
        else
            stalls=0
        fi
        previous="$current"
    done
}

echo "=== pptx-compose Outer Runner ==="
echo "Engine: $ENGINE"
echo "Project: $PROJECT_DIR"
echo "Max outer cycles: $MAX_OUTER_CYCLES"
echo ""

cycle=0
while [ "$cycle" -lt "$MAX_OUTER_CYCLES" ]; do
    cycle=$((cycle + 1))
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Outer cycle $cycle / $MAX_OUTER_CYCLES"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    before_tests=$(open_bead_count)
    echo "  Open Beads before tests: $before_tests"
    run_tests_and_file_beads
    after_tests=$(open_bead_count)
    echo "  Open Beads after tests: $after_tests"

    if [ "$after_tests" -gt "$before_tests" ]; then
        echo "  New Beads were created by tests: $((after_tests - before_tests))"
    else
        echo "  No new beads were created by tests"
    fi

    consolidate_and_investigate_beads
    after_consolidate=$(open_bead_count)
    echo "  Open Beads after consolidation: $after_consolidate"

    set +e
    run_build_until_no_open_beads
    build_rc=$?
    set -e
    after_build=$(open_bead_count)
    echo "  Open Beads after build phase: $after_build"

    if [ "$after_build" -eq 0 ]; then
        echo "  No open Beads remain; running one more outer cycle will verify tests cannot create more."
        if [ "$after_tests" -le "$before_tests" ]; then
            echo "  No new beads were created and no open Beads remain. Stopping."
            break
        fi
    fi

    if [ "$after_tests" -le "$before_tests" ] && [ "$after_build" -ge "$after_consolidate" ] && [ $build_rc -ne 0 ]; then
        echo "  No new beads were created and build made no progress. Stopping."
        break
    fi
done

echo "=== Outer runner completed ==="
echo "Outer cycles: $cycle"
echo "Open Beads: $(open_bead_count)"
