# 042. Agent Protocol Schemas

This spec defines the normative machine-readable schemas used by agent-facing APIs. Examples in other specs are illustrative; implementations should publish JSON Schema documents for every schema listed here.

## Schema Rules

- Every agent-facing JSON document must include `schema` and `version`.
- Unknown fields are rejected by default unless a schema explicitly permits extensions.
- All IDs in patches and reports must be strings.
- EMUs are the default unit for bounds unless a field explicitly includes another unit.
- Binary bytes are never inlined by default.

## Agent View

Required top-level fields:

```json
{
  "schema": "pptx-compose.agent_view.v1",
  "version": 1,
  "document_id": "sha256:...",
  "revision": 1,
  "view_id": "sha256:...",
  "capabilities": {
    "operations": ["replace_text", "add_text_box", "move_resize_element", "set_alt_text", "add_image", "replace_image"],
    "media_content_types": ["image/png", "image/jpeg"],
    "units": "emu"
  },
  "presentation": {
    "part": "ppt/presentation.xml",
    "slide_count": 1
  },
  "slides": []
}
```

## Slide View

Slide entries must include stable identity and provenance:

```json
{
  "id": "slide-1",
  "index": 0,
  "ppt_slide_id": 256,
  "part": "ppt/slides/slide1.xml",
  "relationship_id": "rId7",
  "layout_part": "ppt/slideLayouts/slideLayout1.xml",
  "part_checksum": "sha256:...",
  "elements": []
}
```

## Element View

Element entries must include enough provenance for agents to target and recover from errors:

```json
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
  "fingerprint": "sha256:..."
}
```

## Text Element View

Flat `text` is convenient but insufficient for safe targeting. Text-capable elements should expose structured text:

```json
{
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
    "text_hash": "sha256:..."
  }
}
```

## Image Element View

Picture elements must expose media provenance:

```json
{
  "image": {
    "relationship_id": "rId2",
    "media_part": "ppt/media/image1.png",
    "content_type": "image/png",
    "byte_length": 12345,
    "checksum": "sha256:...",
    "intrinsic_size_px": { "width": 800, "height": 600 },
    "shared_media_ref_count": 1
  }
}
```

## Patch Envelope

Required fields:

```json
{
  "schema": "pptx-compose.patch.v1",
  "version": 1,
  "document_id": "sha256:...",
  "base_revision": 1,
  "client_request_id": "agent-run-001",
  "operations": []
}
```

Each operation must include `operation_id` and `op`. Patches are atomic. Dry-run and apply must use the same report schema.

## Selector Schema

Canonical element selectors use this shape:

```json
{
  "type": "element_id",
  "id": "slide-1:shape-4",
  "guards": {
    "slide_id": "slide-1",
    "kind": "text_box",
    "part": "ppt/slides/slide1.xml",
    "text_hash": "sha256:...",
    "fingerprint": "sha256:..."
  }
}
```

Supported selector types in V1:

- `slide_id`
- `element_id`
- `media_part`

Query/fuzzy selectors are post-V1 unless a later spec defines ambiguity handling.
