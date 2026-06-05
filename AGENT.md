# AGENT.md

## Read This First

This repo is not meant to remain a TypeScript PPTX-to-JSON package. It is being reverse engineered into a cleanroom Rust implementation for AI-agent-safe PPTX round-tripping.

The existing `src/`, `lib/`, `bin/convert`, and Jest tests are legacy observation material only. The TypeScript implementation will be deleted ASAP once the Rust replacement is viable. Do not build new product behavior in TypeScript unless the task explicitly says to measure or preserve current observable behavior.

## Authority Order

1. [`SPEC.md`](SPEC.md)
2. Linked specs under [`specs/`](specs/)
3. Existing README/tests/fixtures as observable legacy behavior only
4. Existing TypeScript source as last-resort context, never as architecture to copy

## The Point of the Rewrite

Agents like Claude and Codex need to:

1. read a `.pptx`,
2. understand slides/elements/media through bounded semantic JSON,
3. produce explicit guarded patches,
4. validate/dry-run those patches,
5. apply changes atomically,
6. write a valid `.pptx`,
7. preserve unsupported content.

This is an OPC/PPTX package engine plus agent runtime contract, not a generic XML-to-JSON toy.

## QA Agent Mandate

If you are reviewing or testing this project, your job is to prove the future Rust crate can be safely used by agents through CLI or MCP. Focus on the specs, not on preserving the old TypeScript structure.

Ask these questions:

- Can an agent inspect a large deck without receiving an unbounded full-deck dump?
- Can it target an element using stable IDs, selectors, and guards?
- Can it stage image bytes through `media_ref` without inline binary overload?
- Can it dry-run and get a structured report before mutation?
- Can it recover from stale revisions, missing media, ambiguous selectors, and unsupported edits?
- Does writing validate by default and avoid partial/corrupt outputs?
- Does no-edit round-trip preserve unknown content and unmodified bytes as required?
- Do CLI and MCP wrappers expose structured outputs and actionable errors?

If the answer depends on manually editing raw XML or legacy path-keyed JSON for a supported V1 operation, the design failed.

## Non-Negotiable Guardrails

- Do not port TypeScript line by line.
- Do not treat `xml2js` shape as the native model.
- Do not rewrite unmodified XML by default.
- Do not drop unknown parts or relationships.
- Do not add media without updating content types, relationships, and slide XML.
- Do not expose unbounded binary payloads to agents by default.
- Do not let raw XML replacement be a default MCP/CLI workflow.
- Do not call TypeScript test success sufficient QA for the Rust rewrite.

## Specs QA Should Prioritize

- [`specs/001-goals-and-scope.md`](specs/001-goals-and-scope.md)
- [`specs/003-cleanroom-process.md`](specs/003-cleanroom-process.md)
- [`specs/010-opc-package-model.md`](specs/010-opc-package-model.md)
- [`specs/020-xml-handling.md`](specs/020-xml-handling.md)
- [`specs/040-agent-json-format.md`](specs/040-agent-json-format.md)
- [`specs/041-agent-edit-operations.md`](specs/041-agent-edit-operations.md)
- [`specs/042-agent-protocol-schemas.md`](specs/042-agent-protocol-schemas.md)
- [`specs/043-media-staging-and-refs.md`](specs/043-media-staging-and-refs.md)
- [`specs/044-results-validation-errors.md`](specs/044-results-validation-errors.md)
- [`specs/045-diffs-previews-journals.md`](specs/045-diffs-previews-journals.md)
- [`specs/071-cli-agent-contract.md`](specs/071-cli-agent-contract.md)
- [`specs/072-mcp-server-contract.md`](specs/072-mcp-server-contract.md)
- [`specs/073-runtime-safety-and-permissions.md`](specs/073-runtime-safety-and-permissions.md)
- [`specs/080-testing-and-fixtures.md`](specs/080-testing-and-fixtures.md)
- [`specs/081-agent-runtime-evals.md`](specs/081-agent-runtime-evals.md)

## Legacy Commands

Only for observing the current package:

```bash
npm test
npm run build
```

These are not the future acceptance criteria. The future acceptance criteria are the spec invariants, CLI/MCP contracts, agent schemas, and runtime evals.
