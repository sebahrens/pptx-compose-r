# V1 Bead Epic Scaffold

> **Historical reference only. Do not execute.**
>
> This document preserves the original V1 decomposition for design archaeology,
> but it is not current Beads workflow guidance. Do not create `tier:epic`
> Beads, do not use `bd update --parent`, and do not recreate parent-child
> edges from this file or from [`v1-epics.graph.json`](v1-epics.graph.json).
> On the Beads version used by this repository, parent-child edges block leaf
> tasks and have already caused tracker deadlocks. Current work should be filed
> as standalone atomic tasks, with any epic context kept in descriptions or
> labels only.

Decomposition of the V1 spec suite into epics and atomic tasks, aligned to the crate/module boundaries in [060](../060-rust-architecture.md). The adjacent machine-readable file is retained as non-executable historical data only.

## Status gate (read first)

The adversarial spec review verdict is **ready-with-fixes after Phase A + Phase B**.

- **Phase A (8 blockers): DONE** — provenance/hashing (046), DrawingML construction (047), dirty-tracking (020), write modes (070/020), content-type resolution + rId allocation (012), case carve-out (010), encryption sniff + resource limits (011).
- **Phase B (registries): NOT DONE** — validation finding-code table (044), error-code reconciliation across 044/060/071, CLI exit-code/command-surface dedup (070/071). **Epics E7, E9, and the CLI/MCP error surfaces (E10/E11) must not start until Phase B lands**, or their acceptance criteria are unstable. They are included here so the DAG is complete, but marked `blocked:phase-b`.

Current tracker rule: do not recreate these epic nodes, do not recreate
parent-child edges, and do not convert the historical graph back into an import
file.

## Epics (→ crate)

| Key | Epic | Crate / module | Specs | Blocking deps |
| --- | --- | --- | --- | --- |
| `pc-e1` | OPC + ZIP core (lossless package I/O) | core `opc/`, `zip/` | 010, 011, 050 | — |
| `pc-e2` | Content types & relationships | core `opc/content_types`, `relationships` | 012 | e1 |
| `pc-e3` | XML model, dirty-tracking & round-trip | core `xml/` | 020, 050 | e1 |
| `pc-e4` | Provenance & hashing | core `pptx/ids`, `opc` | 046 | e1, e2, e3, e5 |
| `pc-e5` | PPTX domain model | core `pptx/` | 030, 031, 032, 033 | e1, e2, e3 |
| `pc-e6` | Agent JSON view | json `agent_view`, `schemas` | 040, 042 | e4, e5, e9 |
| `pc-e7` | Edit & patch engine | edit `selectors`, `operations`, `patch` | 041, 042, 047, 050 | e4, e5, e6, e8, e9 · `blocked:phase-b` |
| `pc-e8` | Media staging & refs | edit `media_inputs`, core `media` | 032, 043 | e1, e2 |
| `pc-e9` | Results, validation & errors | core `validation/`, `error/`, edit `reports` | 044, 045, 050 | e1 · `blocked:phase-b` |
| `pc-e10` | CLI | `cli/` | 070, 071 | e6, e7, e9 · `blocked:phase-b` |
| `pc-e11` | MCP server | `mcp/` | 072, 073 | e6, e7, e9 · `blocked:phase-b` |
| `pc-e12` | Testing, fixtures & evals | tests, `tests/fixtures` | 080, 081 | cross-cutting (start fixtures early) |

## Dependency DAG (blocking order)

```text
        e1 ──┬─► e2 ──┐
             ├─► e3 ──┼─► e5 ──┬─► e4 ─────┐
             └─► e8 ──┘        └─► e6 ◄── e9 (phase-b)
                                   │        │
                                   └─► e7 ◄─┘ (phase-b)
                                        │
                                 ┌──────┴──────┐
                                 ▼             ▼
                               e10           e11   (both phase-b)
        e12 (fixtures) feeds e1/e3/e5 early; eval tasks gate on e10/e11.
```

## Atomic tasks per epic

### pc-e1 — OPC + ZIP core
- `pc-e1-t1` ZIP reader over `zip` crate: enumerate entries, expose raw uncompressed bytes per entry, capture `ZipEntryMetadata` (order, method, timestamps). *(011 Input Forms; 010 Part)*
- `pc-e1-t2` Pre-ZIP format sniff: CFBF magic + `EncryptionInfo`/`EncryptedPackage` → `unsupported_package`/exit 11. *(011 Package Format Detection)*
- `pc-e1-t3` Resource-limit enforcement during streaming inflate: all 8 limits + 200:1 ratio, abort → `resource_limit_exceeded`/exit 12. *(011)*
- `pc-e1-t4` Path-traversal/absolute-path rejection independent of the `zip` crate (CVE-2025-29787). *(011, 010 Part Names)*
- `pc-e1-t5` `PartStore` + canonical `PartName` (leading-slash, normalize, reject dup/traversal, case-sensitive segments). *(010)*
- `pc-e1-t6` ZIP writer: `raw_copy_file` passthrough for clean parts; preserve entry order in preserve mode. *(011, 020 Core Rule)*
- `pc-e1-t7` Deterministic write mode: stable order/timestamps/compression without re-serializing clean payloads. *(011 Deterministic Output, 070 WriteMode)*

### pc-e2 — Content types & relationships
- `pc-e2-t1` Parse `[Content_Types].xml` into defaults+overrides. *(012)*
- `pc-e2-t2` Content-type resolution algorithm (Override exact → Default ASCII-case-insensitive ext → untyped). *(012, 010 carve-out)*
- `pc-e2-t3` Parse all `.rels` into a `RelationshipGraph`; preserve Id/Type/Target/TargetMode; resolve internal targets relative to source. *(012, 010)*
- `pc-e2-t4` rId allocation policy (`rId{max+1}`, collision-safe, deterministic). *(012)*
- `pc-e2-t5` Presentation discovery via OPC relationship graph (root rels → office-doc → sldIdLst → slides → layouts). *(012 PPTX Discovery)*
- `pc-e2-t6` Package invariant checks: every internal rel target resolves; rel IDs unique per part; every ordinary part has a content type. *(010, 012, 050)*

### pc-e3 — XML model, dirty-tracking & round-trip
- `pc-e3-t1` `XmlPart` over `quick-xml`: raw bytes + lazy parse, namespace/prefix/unknown-element preservation, `mc:AlternateContent` support. *(020 Parser)*
- `pc-e3-t2` Dirty-tracking: clean on open; set only on successful mutating edit; strictly per-part. *(020 Dirty Tracking)*
- `pc-e3-t3` XML writer for dirty parts: well-formed, namespaces declared, deterministic serialization profile. *(020 Writer)*
- `pc-e3-t4` Preserve-mode write: clean parts byte-identical from raw. *(020, 050 No-Edit Round Trip)*
- `pc-e3-t5` Raw escape hatch: replace a part's bytes, mark dirty, still run well-formedness + graph validation. *(020 Raw Escape Hatch)*
- `pc-e3-t6` No-edit round-trip test harness: all parts present, unknown parts kept, bytes identical, rel graph valid. *(050)*

### pc-e4 — Provenance & hashing
- `pc-e4-t1` `part_checksum` over raw stored bytes. *(046)*
- `pc-e4-t2` Canonical Preimage JSON (CPJ) encoder. *(046)*
- `pc-e4-t3` `document_id` over sorted CPJ part manifest + content-types stream. *(046)*
- `pc-e4-t4` `revision` lifecycle (1 on open, +1 per successful non-dry-run apply). *(046)*
- `pc-e4-t5` `text_hash` (NFC + whitespace normalization, `a:br`→`\n`). *(046)*
- `pc-e4-t6` `fingerprint` CPJ over {kind,part,sp_tree_path,group_path,cnvpr_id,text_hash}. *(046)*
- `pc-e4-t7` `view_id` over {document_id,revision,mode,scope}. *(046)*
- `pc-e4-t8` Agent ID derivation: slide-{n}, element {kind_prefix}-{spTree path}, p{i}/r{j}. *(046)*
- `pc-e4-t9` Stability tests: no-edit + cross-run determinism + edit-locality. *(046, 050)*

### pc-e5 — PPTX domain model
- `pc-e5-t1` Presentation + slide model; slide order from `sldIdLst`. *(030, 031)*
- `pc-e5-t2` Shape reading: id/name/kind/bounds(EMU)/rot/flip/placeholder/alt. *(031)*
- `pc-e5-t3` Text reading: paragraphs/runs with positional identity + normalized/plain projections. *(031, 042)*
- `pc-e5-t4` Picture reading: r:embed → media part, content type, byte length, shared count, intrinsic size. *(032)*
- `pc-e5-t5` `spTree` positional indexing (sp_tree_path/group_path) feeding IDs + fingerprints. *(046, 042)*
- `pc-e5-t6` Layouts/masters/themes: preserve + minimal read. *(033)*

### pc-e6 — Agent JSON view
- `pc-e6-t1` `schemars`-derived schema-versioned types for agent_view/slide/element/text/image. *(042)*
- `pc-e6-t2` View modes (deck_summary, slide_page, slide_detail, element_detail, media_metadata, validation_report). *(040)*
- `pc-e6-t3` Opaque-cursor pagination + truncation markers + default-limit table. *(040, 042)*
- `pc-e6-t4` No-inline-binary rule; media-by-reference. *(040 Binary Handling)*
- `pc-e6-t5` Legacy path-keyed JSON compat mode with `$xml`/`$binary` envelopes. *(040 Legacy JSON)*

### pc-e7 — Edit & patch engine `(blocked:phase-b)`
- `pc-e7-t1` Selector resolution + guard evaluation (element_id/slide_id/media_part; fingerprint/text_hash guards → `selector_guard_failed`). *(041, 042, 046)*
- `pc-e7-t2` Patch envelope validation + stale-patch rejection (document_id/base_revision). *(041, 046)*
- `pc-e7-t3` Atomic apply (all-or-nothing) + operation reports. *(041, 044)*
- `pc-e7-t4` `replace_text` (whole_element, format_policy, `\n`→`a:p`, newline_mapping report). *(041, 047)*
- `pc-e7-t5` `add_text_box` (p:sp template, default style, style whitelist, invalid_bounds). *(047, 041)*
- `pc-e7-t6` `move_resize_element` (a:xfrm mapping, kind matrix, preserve rot/flip). *(041, 047)*
- `pc-e7-t7` `set_alt_text` (cNvPr descr/title mapping). *(041)*
- `pc-e7-t8` `add_image` (p:pic template, rel URI, content-type, drawing-id alloc, wiring). *(047, 032, 012)*
- `pc-e7-t9` `replace_image` (retarget_picture default, shared-media guard, r:link → unsupported_edit). *(032, 041)*
- `pc-e7-t10` Validation-on-write integration (block on error/fatal). *(050)*

### pc-e8 — Media staging & refs
- `pc-e8-t1` `media_ref` binding across Rust/CLI/MCP (manifest + inline). *(043)*
- `pc-e8-t2` Magic-byte content-type verification (PNG/JPEG/GIF); declared≠sniffed → code. *(032, 043)*
- `pc-e8-t3` Checksum handling + `media_checksum_mismatch` before mutation. *(043, 044)*
- `pc-e8-t4` Deterministic media part naming + dedup (opt-in, never silent). *(032)*
- `pc-e8-t5` Media size limit (`--max-media-bytes`) → `resource_limit_exceeded`. *(011, 043)*

### pc-e9 — Results, validation & errors `(blocked:phase-b)`
- `pc-e9-t1` Result/patch-report/validation-report/error envelopes (schema-versioned). *(044)*
- `pc-e9-t2` Validation finding-code registry (code+category+severity per 050/012 invariant). *(044 — Phase B)*
- `pc-e9-t3` Error-code enum reconciled across 044/060/071. *(044 — Phase B)*
- `pc-e9-t4` Each 050 invariant as an atomic validator emitting its registry code. *(050, 044)*
- `pc-e9-t5` Semantic diff / preview / journal schemas. *(045)*

### pc-e10 — CLI `(blocked:phase-b)`
- `pc-e10-t1` `clap` command surface (inspect/apply/validate/schema + compat) per 071. *(070, 071)*
- `pc-e10-t2` JSON stdout discipline, JSON errors, no prompts. *(071)*
- `pc-e10-t3` Exit-code map (single source = 071). *(071 — Phase B)*
- `pc-e10-t4` Atomic writes + no implicit overwrite + `--deterministic`. *(071, 070)*
- `pc-e10-t5` `--dry-run`/`--report`/`--diff` wiring to report schemas. *(071, 044, 045)*

### pc-e11 — MCP server `(blocked:phase-b)`
- `pc-e11-t1` `rmcp` server scaffold + scoped tool set; raw-XML tools off by default. *(072, 073)*
- `pc-e11-t2` Sessions + revisions + media handles + governance limits (TTL/memory/max). *(072)*
- `pc-e11-t3` Structured outputs via schemars; error→tool-result mapping. *(072, 044)*
- `pc-e11-t4` Per-surface stale-patch rule (MCP rejects on mismatch). *(072, 041)*
- `pc-e11-t5` Permission/safety enforcement (filesystem scope, overwrite, sensitive-content). *(073)*

### pc-e12 — Testing, fixtures & evals
- `pc-e12-t1` Fixture-manifest corpus (filename, source app, structural features, consuming test). *(080 — needs Phase-C/D fixture table)*
- `pc-e12-t2` Golden round-trip tests (no-edit byte identity, unknown-part preservation incl. `mc:AlternateContent`). *(050, 080)*
- `pc-e12-t3` Golden construction fixtures (p:pic, p:sp expected bytes). *(047, 080)*
- `pc-e12-t4` Negative-case suite (stale revision, bad element id, missing/mismatched media, encrypted, zip-bomb, unsafe path, r:link image). *(080, 011)*
- `pc-e12-t5` Agent runtime evals (CLI + MCP transcripts proving the contract). *(081)*

## Historical Blocking Order

The original import plan also included epic-level blocking relationships. Those
commands were intentionally removed because they were copy-pastable and could
recreate the old tracker deadlock. Treat the dependency DAG above as historical
planning context only; create current Beads as standalone atomic tasks.
