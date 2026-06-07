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
  "schema": "pptx-compose.agent_view.v1",
  "version": 1,
  "document_id": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "revision": 1,
  "view_id": "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
  "view": {
    "mode": "slide_detail",
    "limit": 20,
    "next_cursor": null,
    "truncated": false
  },
  "omitted_count": 0,
  "capabilities": {
    "operations": [
      "replace_text",
      "add_text_box",
      "move_resize_element",
      "set_alt_text",
      "add_image",
      "replace_image"
    ],
    "media_content_types": ["image/png", "image/jpeg", "image/gif"],
    "units": "emu"
  },
  "presentation": {
    "part": "ppt/presentation.xml",
    "slide_count": 1
  },
  "slides": [
    {
      "id": "slide-1",
      "index": 0,
      "ppt_slide_id": 256,
      "part": "ppt/slides/slide1.xml",
      "relationship_id": "rId7",
      "layout_part": "ppt/slideLayouts/slideLayout1.xml",
      "part_checksum": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
      "elements": [
        {
          "id": "slide-1:shape-4",
          "kind": "text_box",
          "slide_id": "slide-1",
          "part": "ppt/slides/slide1.xml",
          "xml_location": {
            "sp_tree_path": [3],
            "group_path": [],
            "element_tag": "p:sp",
            "cnvpr_id": 4,
            "cnvpr_name": "Title 1"
          },
          "z_order": 3,
          "bounds": { "x": 914400, "y": 457200, "cx": 7315200, "cy": 914400 },
          "editable": {
            "text": { "supported": true },
            "bounds": { "supported": true },
            "image": { "supported": false, "reason": "not_picture" }
          },
          "fingerprint": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
          "text": {
            "plain": "Quarterly Results",
            "normalized": "Quarterly Results",
            "paragraphs": [
              {
                "text": "Quarterly Results",
                "runs": [
                  {
                    "text": "Quarterly Results",
                    "style_summary": { "font_size_pt": 32, "bold": false }
                  }
                ]
              }
            ],
            "text_hash": "sha256:3333333333333333333333333333333333333333333333333333333333333333"
          }
        },
        {
          "id": "slide-1:pic-5",
          "kind": "image",
          "slide_id": "slide-1",
          "part": "ppt/slides/slide1.xml",
          "xml_location": {
            "sp_tree_path": [4],
            "group_path": [],
            "element_tag": "p:pic",
            "cnvpr_id": 5,
            "cnvpr_name": "Picture 1"
          },
          "z_order": 4,
          "bounds": { "x": 914400, "y": 1524000, "cx": 3657600, "cy": 2743200 },
          "editable": {
            "text": { "supported": false, "reason": "not_text" },
            "bounds": { "supported": true },
            "image": { "supported": true }
          },
          "fingerprint": "sha256:4444444444444444444444444444444444444444444444444444444444444444",
          "image": {
            "relationship_id": "rId2",
            "media_part": "ppt/media/image1.png",
            "content_type": "image/png",
            "byte_length": 12345,
            "checksum": "sha256:5555555555555555555555555555555555555555555555555555555555555555",
            "intrinsic_size_px": { "width": 800, "height": 600 },
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
