# pptx-compose Rust Rewrite Spec Index

This document is the entry point for a cleanroom Rust rewrite of `pptx-compose`.

The rewrite target is not a line-for-line port of the current TypeScript package. The target is a lossless, OPC-aware PPTX engine that lets AI agents such as Claude or Codex read presentations, make explicit validated edits, add content, and write valid `.pptx` files while preserving unsupported content.

## Primary Goal

Enable this workflow:

```text
input.pptx
  -> parse as an OPC/PPTX package
  -> expose an agent-friendly JSON view
  -> apply validated edit operations
  -> validate package invariants
  -> write output.pptx
```

The default behavior must preserve the original deck as much as possible. Unchanged XML and binary parts should be copied through without being parsed and serialized again.

## Cleanroom Position

The existing TypeScript implementation may be used only as an observable behavior reference: public README behavior, tests, package metadata, fixtures, and black-box outputs. Rust implementation details should be independently designed from the OOXML/Open Packaging Convention model and this spec suite.

See [cleanroom process](specs/003-cleanroom-process.md).

## Spec Reading Order for Agents

1. [Goals and scope](specs/001-goals-and-scope.md)
2. [Observable current behavior](specs/002-observable-current-behavior.md)
3. [Cleanroom process](specs/003-cleanroom-process.md)
4. [OPC package model](specs/010-opc-package-model.md)
5. [ZIP I/O and security](specs/011-zip-io-and-security.md)
6. [Content types and relationships](specs/012-content-types-and-relationships.md)
7. [XML handling](specs/020-xml-handling.md)
8. [PPTX presentation model](specs/030-pptx-presentation-model.md)
9. [Slides, shapes, and text](specs/031-slides-shapes-and-text.md)
10. [Media and images](specs/032-media-images.md)
11. [Layouts, masters, and themes](specs/033-layouts-masters-themes.md)
12. [Agent JSON format](specs/040-agent-json-format.md)
13. [Agent edit operations](specs/041-agent-edit-operations.md)
14. [Agent protocol schemas](specs/042-agent-protocol-schemas.md)
15. [Media staging and references](specs/043-media-staging-and-refs.md)
16. [Results, validation, and errors](specs/044-results-validation-errors.md)
17. [Diffs, previews, and journals](specs/045-diffs-previews-journals.md)
18. [Provenance and hashing](specs/046-provenance-and-hashing.md)
19. [DrawingML construction](specs/047-drawingml-construction.md)
20. [Round-trip invariants](specs/050-roundtrip-invariants.md)
21. [Rust architecture](specs/060-rust-architecture.md)
22. [Public API and CLI](specs/070-public-api-and-cli.md)
23. [CLI agent contract](specs/071-cli-agent-contract.md)
24. [MCP server contract](specs/072-mcp-server-contract.md)
25. [Runtime safety and permissions](specs/073-runtime-safety-and-permissions.md)
26. [Testing and fixtures](specs/080-testing-and-fixtures.md)
27. [Agent runtime evals](specs/081-agent-runtime-evals.md)
28. [Known risks and non-goals](specs/090-known-risks-and-non-goals.md)

## Spec Map

| Spec | Purpose |
| --- | --- |
| [001-goals-and-scope.md](specs/001-goals-and-scope.md) | Defines v1 goals, supported edits, and non-goals. |
| [002-observable-current-behavior.md](specs/002-observable-current-behavior.md) | Documents the current package at the API/behavior level. |
| [003-cleanroom-process.md](specs/003-cleanroom-process.md) | Sets rules for using the old repo without copying implementation. |
| [010-opc-package-model.md](specs/010-opc-package-model.md) | Defines the ZIP/OPC package, parts, relationships, and validation model. |
| [011-zip-io-and-security.md](specs/011-zip-io-and-security.md) | Defines ZIP I/O, deterministic output, and resource limits. |
| [012-content-types-and-relationships.md](specs/012-content-types-and-relationships.md) | Defines `[Content_Types].xml` and `.rels` behavior for edits. |
| [020-xml-handling.md](specs/020-xml-handling.md) | Defines namespace-aware XML parsing and raw-preserving writes. |
| [030-pptx-presentation-model.md](specs/030-pptx-presentation-model.md) | Defines presentation, slide, shape, text, media, layout, and theme models. |
| [031-slides-shapes-and-text.md](specs/031-slides-shapes-and-text.md) | Defines v1 slide/text/shape read and edit behavior. |
| [032-media-images.md](specs/032-media-images.md) | Defines reading, replacing, and adding images. |
| [033-layouts-masters-themes.md](specs/033-layouts-masters-themes.md) | Defines preservation and minimal read support for layouts/masters/themes. |
| [040-agent-json-format.md](specs/040-agent-json-format.md) | Defines the compact JSON view intended for LLM agents. |
| [041-agent-edit-operations.md](specs/041-agent-edit-operations.md) | Defines explicit patch operations agents should emit. |
| [042-agent-protocol-schemas.md](specs/042-agent-protocol-schemas.md) | Defines normative schemas for agent views, selectors, patches, and reports. |
| [043-media-staging-and-refs.md](specs/043-media-staging-and-refs.md) | Defines how CLI, MCP, and Rust APIs bind `media_ref` to bytes. |
| [044-results-validation-errors.md](specs/044-results-validation-errors.md) | Defines machine-readable result, validation, and error envelopes. |
| [045-diffs-previews-journals.md](specs/045-diffs-previews-journals.md) | Defines semantic diffs, preview levels, and transaction journals. |
| [046-provenance-and-hashing.md](specs/046-provenance-and-hashing.md) | Defines derivation of document_id, revision, checksums, fingerprints, text hashes, and agent IDs. |
| [047-drawingml-construction.md](specs/047-drawingml-construction.md) | Defines `p:pic`/`p:sp` templates, image rel URIs, EMU mapping, and default text-box style for inserted elements. |
| [050-roundtrip-invariants.md](specs/050-roundtrip-invariants.md) | Defines correctness rules for parse/write/edit cycles. |
| [060-rust-architecture.md](specs/060-rust-architecture.md) | Defines crate/module boundaries for the Rust rewrite. |
| [070-public-api-and-cli.md](specs/070-public-api-and-cli.md) | Defines library, Node-compatible, and CLI surfaces. |
| [071-cli-agent-contract.md](specs/071-cli-agent-contract.md) | Defines stable scriptable CLI behavior for agents. |
| [072-mcp-server-contract.md](specs/072-mcp-server-contract.md) | Defines MCP tools, resources, prompts, sessions, and structured outputs. |
| [073-runtime-safety-and-permissions.md](specs/073-runtime-safety-and-permissions.md) | Defines filesystem, overwrite, raw XML, and sensitive-content policies. |
| [080-testing-and-fixtures.md](specs/080-testing-and-fixtures.md) | Defines required fixtures, golden tests, and validation tests. |
| [081-agent-runtime-evals.md](specs/081-agent-runtime-evals.md) | Defines CLI/MCP evals proving agents can use the runtime contract. |
| [090-known-risks-and-non-goals.md](specs/090-known-risks-and-non-goals.md) | Captures risks, guardrails, and deliberately unsupported work. |

## Architecture Summary

```text
┌───────────────────────┐
│ Public API / CLI       │
└───────────┬───────────┘
            │
┌───────────▼───────────┐
│ Agent JSON + Patches   │  Safe operations for Claude/Codex
└───────────┬───────────┘
            │
┌───────────▼───────────┐
│ PPTX Domain Model      │  Presentation, slides, shapes, text, images
└───────────┬───────────┘
            │
┌───────────▼───────────┐
│ OPC Package Model      │  Parts, content types, relationships
└───────────┬───────────┘
            │
┌───────────▼───────────┐
│ ZIP + XML I/O          │  Lossless preservation where possible
└───────────────────────┘
```

## Hard Guardrails

- Never rewrite unmodified XML parts by default.
- Never drop unknown parts, relationships, namespaces, or extension elements.
- Never add media by only adding a file; update content types, relationships, and slide XML.
- Never make raw XML/JSON mutation the primary editing API for agents.
- Never expose MCP or CLI workflows that require agents to mutate raw XML/legacy JSON for supported V1 edits.
- Never return unbounded full-deck or binary payloads by default through agent-facing tools.
- Always validate package graph invariants before writing edited files.
- Prefer a clear unsupported-edit error over producing a corrupt deck.
