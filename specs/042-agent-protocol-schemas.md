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
    "media_content_types": ["image/png", "image/jpeg", "image/gif"],
    "units": "emu"
  },
  "presentation": {
    "part": "ppt/presentation.xml",
    "slide_count": 1
  },
  "slides": []
}
```

Post-V1 metadata inspection may add this optional top-level shape:

```json
{
  "metadata": {
    "core_properties": {
      "part": "docProps/core.xml",
      "part_checksum": "sha256:...",
      "title": "Quarterly Results",
      "subject": null,
      "creator": "Research Team",
      "keywords": "finance; q4"
    }
  }
}
```

The `part_checksum` is the raw-byte checksum defined in
[046](046-provenance-and-hashing.md) and is suitable for
`set_document_metadata` selector guards. Missing core-property elements are
reported as `null` in the view.

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

The canonical element kind vocabulary is `text_box`, `shape`, `image`, `group`,
`chart`, `table`, `diagram`, and `ole`. Selector `guards.kind` uses this same
vocabulary, so an agent can copy an `Element View.kind` value directly into a
guarded element selector. Connector or otherwise unsupported drawing elements
that are exposed as generic shape elements use `shape`.

For OOXML `p:graphicFrame` elements, `kind` MUST be derived from
`a:graphic/a:graphicData/@uri`:

| `a:graphicData/@uri` | `ElementKind` |
| --- | --- |
| `http://schemas.openxmlformats.org/drawingml/2006/chart` | `chart` |
| `http://schemas.openxmlformats.org/drawingml/2006/table` | `table` |
| `http://schemas.openxmlformats.org/drawingml/2006/diagram` | `diagram` |
| `http://schemas.openxmlformats.org/presentationml/2006/ole` and other graphic-frame OLE URIs ending in `/ole` | `ole` |

The V1 agent-view schema already permits the full vocabulary above, so this
clarification does not bump `pptx-compose.agent_view.v1` or `version: 1`.
Graphic-frame subkinds are editable only at the frame level: `move_resize_element`
and `set_alt_text` are allowed for `chart`, `table`, `diagram`, and `ole`;
`replace_text` is not allowed for any graphic-frame subkind. The detailed
editability rationale is normative in
[048. Editability Catalogue](048-editability-catalogue.md).

## Text Element View

Flat `text` is convenient but insufficient for safe targeting. Text-capable elements should expose structured text:

```json
{
  "text": {
    "plain": "Quarterly Results",
    "normalized": "Quarterly Results",
    "paragraphs": [
      {
        "index": 0,
        "text": "Quarterly Results",
        "runs": [
          {
            "index": 0,
            "text": "Quarterly Results",
            "style_summary": { "font_size_pt": 32, "bold": false },
            "text_hash": "sha256:..."
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

Post-V1 operations appear in `capabilities.operations` only after their
implementation ships. The first post-V1 operation schema is
`set_document_metadata`:

```json
{
  "operation_id": "op-metadata-title",
  "op": "set_document_metadata",
  "selector": {
    "type": "core_properties",
    "part": "docProps/core.xml",
    "guards": {
      "part_checksum": "sha256:..."
    }
  },
  "match": {
    "title": "Old deck title"
  },
  "metadata": {
    "title": "New deck title",
    "subject": "Board update",
    "creator": "Research Team",
    "keywords": "finance; q4"
  }
}
```

`metadata` MUST contain at least one settable field. Initial implementation
fields are `title`, `subject`, `creator`, and `keywords`; these map to
`dc:title`, `dc:subject`, `dc:creator`, and `cp:keywords` in
`docProps/core.xml`. `category` (`cp:category`) and `description`
(`dc:description`) are reserved deferred fields and MUST be rejected until the
schema explicitly opts them in. Unknown fields are rejected by default.

`match` is optional and maps settable metadata field names to expected current
string values. Missing elements do not satisfy a string `match`; a later schema
may add an explicit null/absence guard if agents need that distinction. Guard
mismatches return `selector_guard_failed`.

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

`set_document_metadata` adds a post-V1 part-scoped selector:

```json
{
  "type": "core_properties",
  "part": "docProps/core.xml",
  "guards": {
    "part_checksum": "sha256:..."
  }
}
```

This selector resolves only the OPC core-properties part and produces a
part-scoped resolved target, not a slide, media part, or shape-tree element.
`part` MUST be `docProps/core.xml`. `guards.part_checksum`, when present, is the
current raw-byte `part_checksum` of that part as defined in [046](046-provenance-and-hashing.md).
Missing core properties return `selector_not_found`; checksum mismatch returns
`selector_guard_failed`.

Phase 2 run-scoped text replacement extends only `element_id` selectors. The
owning element is still selected by `type: "element_id"` and `id`, while the run
coordinate is carried in `run`:

```json
{
  "type": "element_id",
  "id": "slide-1:shape-4",
  "run": {
    "paragraph_index": 0,
    "run_index": 1,
    "text_hash": "sha256:..."
  },
  "guards": {
    "slide_id": "slide-1",
    "kind": "shape",
    "part": "ppt/slides/slide1.xml",
    "text_hash": "sha256:...",
    "fingerprint": "sha256:..."
  }
}
```

For a run range, add `run_end_index`; it is inclusive and MUST be in the same
paragraph as `run_index`. `run.text_hash` guards exactly the selected run or run
range. `guards.text_hash` remains the whole-element text hash. When both are
present, both must match.
