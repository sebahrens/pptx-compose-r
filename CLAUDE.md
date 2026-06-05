# CLAUDE.md

## Project Direction

This repository is being reverse engineered into a cleanroom Rust rewrite for reliable PPTX round-tripping.

The current TypeScript implementation is legacy scaffolding. It exists only so agents can observe public behavior, fixtures, package metadata, and known limitations. The TypeScript code is expected to be deleted as soon as the Rust implementation can replace it. Do not extend the TypeScript implementation unless explicitly asked to preserve or measure current observable behavior.

## Source of Truth

Start with [`SPEC.md`](SPEC.md). The linked files in [`specs/`](specs/) are the authoritative requirements.

Important specs for most work:

- [`specs/001-goals-and-scope.md`](specs/001-goals-and-scope.md) — V1 scope and non-goals.
- [`specs/003-cleanroom-process.md`](specs/003-cleanroom-process.md) — what can and cannot be copied from the old implementation.
- [`specs/010-opc-package-model.md`](specs/010-opc-package-model.md) — OPC package model.
- [`specs/020-xml-handling.md`](specs/020-xml-handling.md) — raw-preserving XML strategy.
- [`specs/040-agent-json-format.md`](specs/040-agent-json-format.md) and [`specs/041-agent-edit-operations.md`](specs/041-agent-edit-operations.md) — agent-facing view and patch model.
- [`specs/042-agent-protocol-schemas.md`](specs/042-agent-protocol-schemas.md), [`specs/043-media-staging-and-refs.md`](specs/043-media-staging-and-refs.md), and [`specs/044-results-validation-errors.md`](specs/044-results-validation-errors.md) — schemas, media refs, reports, and errors.
- [`specs/071-cli-agent-contract.md`](specs/071-cli-agent-contract.md) and [`specs/072-mcp-server-contract.md`](specs/072-mcp-server-contract.md) — CLI/MCP contracts agents must be able to use.
- [`specs/080-testing-and-fixtures.md`](specs/080-testing-and-fixtures.md) and [`specs/081-agent-runtime-evals.md`](specs/081-agent-runtime-evals.md) — QA expectations.

## Cleanroom Rules

- Do not port TypeScript source line by line.
- Use the TypeScript code only for observable behavior: README/API behavior, existing tests, fixtures, package metadata, and black-box outputs.
- Prefer OOXML/Open Packaging Convention behavior and the spec suite over implementation quirks.
- If compatibility with a TypeScript quirk is required, document it explicitly in the specs before implementing it.

## What To Build Toward

The target workflow is:

```text
input.pptx
  -> parse as an OPC/PPTX package
  -> expose bounded agent JSON views
  -> apply explicit validated edit operations
  -> validate package invariants
  -> write output.pptx
```

Core guardrails:

- Preserve unmodified XML bytes by default.
- Preserve unknown parts, relationships, namespaces, extension elements, media, embeddings, charts, and unsupported content.
- Never add an image by only adding a file; update content types, relationships, and slide XML.
- Never make raw XML or legacy path-keyed JSON mutation the primary editing API for supported V1 operations.
- Validate edited documents before writing.
- Prefer `unsupported_edit` over corrupt output.

## Current Legacy Commands

These commands describe only the legacy TypeScript package:

```bash
npm test
npm run build
```

Use them only when measuring existing behavior or ensuring a temporary compatibility fixture still runs. Passing legacy Jest tests does not prove the Rust rewrite is correct.

## QA Expectations

QA must test the spec, not the old TypeScript architecture.

For any Rust rewrite, CLI, or MCP work, QA should verify:

- No-edit round trip preserves all parts and keeps unmodified XML/binary bytes where required.
- `[Content_Types].xml` covers every ordinary part.
- Internal relationships resolve and relationship IDs are unique per relationship part.
- Agent JSON is bounded, schema-versioned, paginated/truncated when needed, and does not inline binaries by default.
- Patches include document/revision guards and operation IDs.
- Dry-run, apply, validation, diff, and error outputs follow the schemas in `specs/042` through `specs/045`.
- CLI behavior follows `specs/071-cli-agent-contract.md`: JSON stdout discipline, JSON errors, no prompts, atomic writes, no implicit overwrite.
- MCP behavior follows `specs/072-mcp-server-contract.md`: scoped tools, sessions/revisions, media handles, structured outputs, raw XML tools disabled by default.
- Negative cases fail safely: stale revision, hallucinated element ID, missing media ref, media checksum mismatch, unsupported chart edit, unsafe path, encrypted deck, signed/macro-containing deck warnings.

Do not accept a QA result that only says “the TypeScript tests pass.” That is legacy smoke coverage, not proof that agents can round-trip PPTX safely.

## Implementation Bias

- Build Rust-first crate boundaries from [`specs/060-rust-architecture.md`](specs/060-rust-architecture.md).
- Keep V1 narrow: inspect slides/text/images, replace text, add text box, move/resize, set alt text, add/replace images.
- Slide add/duplicate/delete/reorder is post-V1 unless the specs are expanded first.
- Avoid overbuilding rendering, full layout, chart editing, SmartArt editing, or full ECMA-376 coverage in V1.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->
