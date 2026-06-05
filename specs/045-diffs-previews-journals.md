# 045. Diffs, Previews, and Journals

Agents need to understand what changed without reading raw XML diffs. This spec defines semantic diff and transaction metadata.

## Semantic Diff

Patch reports may include or reference a semantic diff:

```json
{
  "schema": "pptx-compose.semantic_diff.v1",
  "version": 1,
  "changes": [
    {
      "kind": "text_replaced",
      "operation_id": "op-1",
      "element_id": "slide-1:shape-4",
      "before": { "text": "Quarterly Results" },
      "after": { "text": "Updated title" }
    },
    {
      "kind": "relationship_added",
      "part": "ppt/slides/_rels/slide1.xml.rels",
      "relationship_id": "rId8",
      "target": "../media/image7.png"
    }
  ],
  "changed_parts": [
    {
      "part": "ppt/slides/slide1.xml",
      "change_kind": "modified_xml",
      "before_checksum": "sha256:...",
      "after_checksum": "sha256:..."
    }
  ]
}
```

## Preview Levels

Supported preview levels:

- `json_diff`: always available.
- `package_diff`: always available.
- `validation_preview`: always available on dry-run.
- `xml_excerpt`: optional/debug; not the primary agent workflow.
- `slide_thumbnail`: optional; requires an external renderer.

Rendering is not a V1 requirement. When unavailable, previews must still provide semantic/package/validation diffs.

## Transaction Journal

Patch application should maintain an internal transaction journal so failures leave the document unchanged. CLI workflows may optionally persist a journal:

```json
{
  "schema": "pptx-compose.journal.v1",
  "version": 1,
  "transaction_id": "txn_...",
  "document_id": "sha256:old",
  "base_revision": 1,
  "status": "applied",
  "operations": ["op-1"],
  "changed_parts": [
    { "part": "ppt/slides/slide1.xml", "before_checksum": "sha256:...", "after_checksum": "sha256:..." }
  ],
  "validation_report": "validation-report-id-or-inline",
  "output_path": "output.pptx"
}
```

Rules:

- `apply_patch(dry_run)` must not dirty the document.
- Failed `apply_patch` must leave in-memory state unchanged.
- File writes must use temp file plus atomic rename where supported.
- Patch reports must include enough changed-part information for agents to explain what happened.
