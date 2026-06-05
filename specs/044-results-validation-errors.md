# 044. Results, Validation, and Errors

Agents need structured results to recover from failures. Human-readable logs are insufficient.

## Standard Result Envelope

Successful machine-readable outputs should use this envelope shape:

```json
{
  "schema": "pptx-compose.result.v1",
  "version": 1,
  "status": "success",
  "result": {},
  "warnings": [],
  "next_cursor": null
}
```

## Patch Report

Dry-run and apply use the same report schema:

```json
{
  "schema": "pptx-compose.patch_report.v1",
  "version": 1,
  "status": "applied",
  "dry_run": false,
  "document_id": "sha256:old",
  "base_revision": 1,
  "new_document_id": "sha256:new",
  "new_revision": 2,
  "operation_reports": [
    {
      "operation_id": "op-1",
      "op": "replace_text",
      "status": "applied",
      "target": {
        "slide_id": "slide-1",
        "element_id": "slide-1:shape-4",
        "part": "ppt/slides/slide1.xml"
      },
      "changed_parts": ["ppt/slides/slide1.xml"],
      "created_element_ids": [],
      "warnings": []
    }
  ],
  "changed_parts": ["ppt/slides/slide1.xml"],
  "validation": { "status": "valid", "errors": 0, "warnings": 0 }
}
```

Patch statuses: `dry_run_success`, `dry_run_failed`, `applied`, `failed`.

Operation statuses: `validated`, `applied`, `skipped`, `failed`.

## Validation Report

Validation reports must include stable finding codes and machine-readable locations:

```json
{
  "schema": "pptx-compose.validation_report.v1",
  "version": 1,
  "document_id": "sha256:...",
  "revision": 1,
  "status": "valid",
  "summary": { "fatal": 0, "errors": 0, "warnings": 1, "info": 3 },
  "findings": [
    {
      "id": "finding-1",
      "severity": "warning",
      "category": "relationship",
      "code": "external_relationship_not_checked",
      "message": "External relationship was preserved but not fetched",
      "blocking": false,
      "location": { "part": "ppt/slides/slide1.xml", "relationship_id": "rId5" },
      "suggested_action": null
    }
  ]
}
```

Severity levels: `info`, `warning`, `error`, `fatal`.

Edited documents with any `error` or `fatal` finding must not be written by default. No-edit documents may write with existing warnings unless structurally unsafe. "Structurally unsafe" means any `fatal` finding: a `fatal` finding blocks writing even for a no-edit round trip.

### Validation Finding Codes

`code` is a stable identifier; renaming a code is a breaking change. Every validation invariant in [round-trip invariants](050-roundtrip-invariants.md) and [content types and relationships](012-content-types-and-relationships.md) maps to exactly one finding code. The registry below is the canonical set; implementations must not invent ad-hoc codes for these conditions.

| `code` | `category` | default `severity` | Invariant / trigger (spec) |
| --- | --- | --- | --- |
| `missing_content_type` | `content_type` | `error` | An ordinary part resolves to no content type (010, 012 resolution algorithm, 050). |
| `media_content_type_mismatch` | `content_type` | `error` | A media part's declared content type disagrees with its sniffed magic-byte type (012, 032). |
| `dangling_internal_relationship` | `relationship` | `error` | An internal relationship `Target` does not resolve to an existing part (010, 050). |
| `unresolved_relationship_reference` | `relationship` | `error` | Slide/part XML references an `r:id`/`rId` not present in that part's `.rels` (012, 050). |
| `duplicate_relationship_id` | `relationship` | `error` | Two relationships in one relationship part share an `Id` (010, 012, 050). |
| `external_relationship_not_checked` | `relationship` | `warning` | An external relationship was preserved but not fetched (010). |
| `duplicate_slide_id` | `presentation` | `error` | Two `p:sldId` entries share an id (050). |
| `slide_order_mismatch` | `presentation` | `error` | Slide order diverges from `p:sldIdLst` without an explicit reorder edit (050). |
| `duplicate_drawing_id` | `slide` | `error` | Two `p:cNvPr` ids collide within one slide shape tree (050). |
| `invalid_bounds` | `slide` | `error` | An inserted/moved element's bounds fall outside the valid EMU range (047). |
| `malformed_xml` | `xml` | `fatal` | A part that must be written is not well-formed XML (050). |
| `missing_namespace_declaration` | `xml` | `error` | A dirty part uses a prefix whose namespace is not declared in scope (047, 050). |
| `part_dropped` | `package` | `fatal` | An original or unknown part would be lost on write (050). |
| `orphan_part` | `package` | `info` | A part is unreachable through the relationship graph; preserved, never auto-pruned in V1 (010). |
| `signature_invalidated_by_edit` | `signature` | `warning` | A mutating edit to a signed deck would invalidate an existing digital signature (073, 090). |

Notes:

- `severity` is the **default**; a surface may escalate (never silently downgrade) a finding given more context.
- Byte-preservation guarantees (binary/XML byte-identity in preserve mode, shape-tree order, unknown-XML preservation) are **test-level** invariants checked by the fixture suite (080), not runtime validation findings — except where a violation surfaces as `part_dropped` or `malformed_xml`.

## Error Envelope

All CLI/MCP failures should expose this shape:

```json
{
  "schema": "pptx-compose.error.v1",
  "version": 1,
  "status": "error",
  "error": {
    "code": "stale_patch",
    "message": "Patch base_revision does not match current revision.",
    "severity": "error",
    "category": "patch",
    "retryable": false,
    "state_changed": false,
    "location": { "operation_id": "op-1", "element_id": "slide-1:shape-4" },
    "suggestions": ["Inspect the deck again and regenerate the patch."]
  }
}
```

This spec is the **single normative source** for error-code names. Other specs must reference these codes rather than defining their own vocabulary: [Rust architecture](060-rust-architecture.md) error kinds, [CLI agent contract](071-cli-agent-contract.md) exit codes, and [MCP server contract](072-mcp-server-contract.md) tool errors all map onto this set. The code → CLI exit-code mapping lives in [071](071-cli-agent-contract.md#exit-codes).

Minimum stable error codes:

- `invalid_input`
- `unsafe_path`
- `resource_limit_exceeded`
- `unsupported_package`
- `unsupported_edit`
- `unsupported_media_type`
- `invalid_bounds`
- `parse_error`
- `validation_failed`
- `stale_patch`
- `selector_not_found`
- `selector_ambiguous`
- `selector_guard_failed`
- `missing_media_ref`
- `media_checksum_mismatch`
- `permission_denied`
- `write_failed`
- `internal_error`

Error messages must be written for an agent deciding the next tool call, not only for a human reading logs.
