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


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:7510c1e2 -->
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

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
