# 040. Agent JSON Format

The agent JSON format is a compact semantic projection for LLMs. It is not a raw dump of every XML node.

## Principles

- Stable schema version.
- Stable element IDs for the exported document revision.
- Small enough for LLM context.
- No inline binary blobs by default.
- Include provenance so patches can be validated.
- Preserve unknown content by reference rather than dropping it.
- Emit only IDs that are accepted by V1 patch operations. Structured paragraph
  and run text may be exposed for context, but paragraph/run IDs must not be
  emitted until paragraph/run replacement modes are implemented.

Agent element IDs are valid for the `document_id` and `revision` that produced the JSON view. CLI workflows that export JSON and apply a later patch must include those fields and reject stale patches unless the implementation can prove the IDs still resolve to the same elements.

Full-deck export is not the primary agent read path for large decks. Agent-facing APIs must support scoped views, pagination, and truncation markers so agents can inspect only the slides and elements they need.

## Example

```json
{
  "version": 1,
  "document_id": "sha256:example-package-fingerprint",
  "revision": 1,
  "presentation": {
    "slide_count": 1
  },
  "slides": [
    {
      "id": "slide-1",
      "index": 0,
      "part": "ppt/slides/slide1.xml",
      "layout": "ppt/slideLayouts/slideLayout1.xml",
      "elements": [
        {
          "id": "slide-1:shape-4",
          "kind": "text_box",
          "name": "Title 1",
          "text": "Quarterly Results",
          "bounds": { "x": 914400, "y": 457200, "cx": 7315200, "cy": 914400 },
          "editable": { "text": { "supported": true }, "bounds": { "supported": true }, "alt_text": { "supported": true } },
          "selector_hints": {
            "slide_index": 0,
            "shape_name": "Title 1",
            "text_hash": "sha256:example-text-hash"
          }
        },
        {
          "id": "slide-1:pic-7",
          "kind": "image",
          "media_part": "ppt/media/image1.png",
          "content_type": "image/png",
          "bounds": { "x": 914400, "y": 1828800, "cx": 3657600, "cy": 2743200 },
          "editable": { "bounds": { "supported": true }, "alt_text": { "supported": true }, "image": { "supported": true } },
          "image": {
            "relationship_id": "rId2",
            "media_part": "ppt/media/image1.png",
            "content_type": "image/png",
            "byte_length": 12345,
            "checksum": "sha256:example-media-hash",
            "shared_media_ref_count": 1
          }
        }
      ]
    }
  ]
}
```

## Views, Pagination, and Truncation

Agent-facing APIs must support these view modes:

- `deck_summary`: presentation metadata, slide count, warnings, and capability summary.
- `slide_page`: pageable slide summaries without full element payloads.
- `slide_detail`: one slide with selected element details.
- `element_detail`: one element with full supported provenance.
- `media_metadata`: media inventory without binary bytes.
- `validation_report`: pageable validation findings.

Paginated outputs must use opaque cursors and explicit truncation fields:

```json
{
  "view": { "mode": "slide_page", "limit": 20, "next_cursor": "opaque", "truncated": true },
  "omitted_count": 42
}
```

Long text, notes, part lists, validation findings, and media inventories must be truncated rather than silently omitted. Truncated fields must indicate how to request more detail.

## Element Kinds

`Element.kind` uses the canonical agent-view `ElementKind` vocabulary:

- `text_box`
- `shape`
- `image`
- `group`
- `chart`
- `table`
- `diagram`
- `ole`

For OOXML `p:graphicFrame` elements, the kind derives from
`a:graphic/a:graphicData/@uri`:

- `http://schemas.openxmlformats.org/drawingml/2006/chart` -> `chart`
- `http://schemas.openxmlformats.org/drawingml/2006/table` -> `table`
- `http://schemas.openxmlformats.org/drawingml/2006/diagram` -> `diagram`
- `http://schemas.openxmlformats.org/presentationml/2006/ole` and other
  graphic-frame OLE URIs ending in `/ole` -> `ole`

Graphic-frame subkinds expose frame-level geometry and non-visual properties
only. `move_resize_element` and `set_alt_text` are supported for `chart`,
`table`, `diagram`, and `ole` because they mutate only `p:graphicFrame/p:xfrm`
or `p:cNvPr`. `replace_text` is not supported for any graphic-frame subkind;
chart data, table cells, SmartArt/diagram content, and embedded OLE payloads
follow the editability boundaries in
[048. Editability Catalogue](048-editability-catalogue.md).

## Binary Handling

Binary data should be referenced by media part, checksum, or caller-provided media handle. Inline base64 export may exist as an explicit option, not the default.

## Legacy JSON

The existing path-keyed JSON format should be a compatibility mode, separate from this agent JSON format.

Durable file-based legacy JSON must use explicit envelopes for values that cannot be represented safely as plain JSON:

```json
{
  "ppt/presentation.xml": { "$xml": "<p:presentation>...</p:presentation>" },
  "ppt/media/image1.png": {
    "$binary": {
      "encoding": "base64",
      "content_type": "image/png",
      "data": "..."
    }
  }
}
```

In-memory compatibility APIs may expose richer host-native values such as `Buffer`/`Uint8Array`, but CLI JSON must be durable and schema-versioned.
