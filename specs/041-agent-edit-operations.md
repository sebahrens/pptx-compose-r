# 041. Agent Edit Operations

Agents should modify decks through explicit operations, not arbitrary raw XML or raw JSON mutation.

## Patch Envelope

```json
{
  "schema": "pptx-compose.patch.v1",
  "version": 1,
  "document_id": "sha256:example-package-fingerprint",
  "base_revision": 1,
  "client_request_id": "agent-run-001",
  "operations": [
    {
      "operation_id": "op-1",
      "op": "replace_text",
      "element_id": "slide-1:shape-4",
      "text": "Updated title"
    }
  ]
}
```

## V1 Operations

- `replace_text`
- `add_text_box`
- `move_resize_element`
- `set_alt_text`
- `add_image`
- `replace_image`

## Operation Requirements

- Selectors must resolve uniquely unless multi-edit is explicit.
- Agent IDs are valid only for the exported `document_id` and `base_revision`; stale patches must be rejected unless selectors can be safely revalidated.
- Every operation must include `operation_id` so dry-run, apply, validation, and error reports can refer to it.
- Unknown operation fields must be rejected by default. Future compatibility modes may allow extensions only when explicitly requested.
- Every operation must have a dry-run validation path.
- Patch application must be atomic: either all operations apply or none are written.
- Every operation must produce an edit report.
- Unsupported operations must return structured errors.

## Selectors and Guards

Bare `slide_id` and `element_id` fields are allowed shorthand. Canonical patches should use selector objects when agents need stale-target protection:

```json
{
  "selector": {
    "type": "element_id",
    "id": "slide-1:shape-4",
    "guards": {
      "slide_id": "slide-1",
      "kind": "text_box",
      "part": "ppt/slides/slide1.xml",
      "text_hash": "sha256:example-text-hash",
      "fingerprint": "sha256:example-element-fingerprint"
    }
  }
}
```

Selectors must resolve to exactly one target unless the operation explicitly supports multiple targets. Fuzzy matching is forbidden by default. Guard failures return structured stale/guard error codes rather than applying best-effort edits.

Element `guards.kind` uses the same snake_case vocabulary emitted by
`Element View.kind` in the agent view schema: `text_box`, `image`, `shape`,
`group`, `table`, and `chart`. Agents may copy an element view's `kind` value
directly into selector guards. Implementations may accept legacy internal guard
aliases for compatibility, but emitted views and canonical patches should use
the agent-view vocabulary.

## V1 Operation Schemas

### `replace_text`

Required fields:

- `element_id`: target text-capable element.
- `text`: replacement text.

Optional fields:

- `match`: exact current text guard. If present and the current text differs, validation fails.
- `mode`: `whole_element` by default. V1 exposes structured paragraph/run text
  for context only; it must not emit paragraph/run IDs because no paragraph/run
  replacement mode exists yet.
- `format_policy`: `preserve_existing_runs`, `preserve_first_run`, or `single_run_default_style`.
- `overflow_policy`: `allow` by default. V1 does not guarantee rendered text fit unless an external renderer is configured.

V1 `replace_text` is **whole-element and destructive**. The implementation filters the existing `a:txBody` down to `bodyPr`/`lstStyle`, clones the first run's `a:rPr` onto every rebuilt run, and resynthesizes `a:p`/`a:r`/`a:t` by splitting the new text. As a consequence:

- Multi-run paragraphs collapse to a single run style.
- `a:fld`, `a:hlinkClick`, `a:hlinkMouseOver`, and `a:br` are detected but **not** preserved through the rewrite; they survive only incidentally and are lost when paragraphs are rewritten.

These losses are reported via the `formatting_simplified` warning, which is the documented V1 contract. Run-property overrides (size/bold/color) MUST NOT be added to this whole-element path; they are gated behind the post-V1 run-scoped mode below.

Newlines in replacement text must have a documented mapping. V1 maps `\n` to PowerPoint paragraphs (hard breaks), never soft `a:br` breaks, and reports the chosen mapping in the patch report. Targets that are not `text_box` or `shape` (graphic frames, groups, connectors) return `unsupported_edit`.

### `add_text_box`

Required fields:

- `slide_id`
- `text`
- `bounds`: `{ "x": emu, "y": emu, "cx": emu, "cy": emu }`

Optional fields:

- `name`
- `alt_text`
- `style`: implementation-defined basic font/fill options. V1 validates `font_size_pt`, `bold`, `italic`, `color_hex`, `font_family`, and `align` (`l`/`ctr`/`r`).
- `insert`: `{ "z_order": "front" }` by default.

Supported V1 style fields must be explicit in implementation docs. Unknown style fields fail validation rather than being silently ignored.

`insert.z_order` MUST NOT be a silently-ignored knob. V1 either honors it (front/back/index placement in `p:spTree`) for both `add_text_box` and `add_image`, or removes the field from the schema. A field that is accepted and dropped is a dishonest surface and is not permitted.

### `move_resize_element`

Required fields:

- `element_id`
- `bounds` in EMUs.

### `set_alt_text`

Required fields:

- `element_id`

Optional fields:

- `title`
- `description`
- `alt_text` as a shorthand mapped to the appropriate OOXML non-visual property.

### `add_image`

Required fields:

- `slide_id`
- `media_ref`: key for bytes supplied alongside the patch.
- `content_type`: e.g. `image/png`, `image/jpeg`, or `image/gif`.
- `bounds` in EMUs.

The advertised media capability set in the agent view MUST equal the set the operation accepts. If `add_image`/`replace_image` accept GIF, the agent view advertises `image/gif`; otherwise GIF is stripped from the operations. Advertising and acceptance must not diverge.

Optional fields:

- `name`
- `alt_text`
- `fit`: `stretch` by default. `contain`, `cover`, and `original_size` are optional and must be rejected when unsupported.
- `dedupe`: `never` by default. Checksum-based reuse is opt-in.

### `replace_image`

Required fields:

- `element_id`: target picture element.
- `media_ref`: key for replacement bytes.
- `content_type`

The implementation should keep the existing picture element and relationship ID when possible; if retargeting is required, it must update content types and relationships atomically.

Default `replace_image` behavior is `retarget_picture`: create or reuse a media part and update only the selected picture relationship. Mutating the existing media part is allowed only when the media part is unshared or when `allow_shared_mutation: true` is explicit.

`replace_image` rejects pictures backed by `a:blip@r:link` (external links) with `unsupported_edit`. The agent view MUST report such pictures as `editable.image.supported = false` with `reason = external_link` so agents do not attempt an edit that will be rejected at apply.

See [agent protocol schemas](042-agent-protocol-schemas.md) and [media staging and references](043-media-staging-and-refs.md) for normative schema details.

## Phased Post-V1 Operations

These operations are explicitly out of V1. They are listed here so the full editable surface is specified, but each ships only behind its own phase gate, schema, guards, and negative tests. The phases are ordered by dependency, not by priority. The [editability catalogue](048-editability-catalogue.md) is the exhaustive feature-by-feature mapping.

The hard rule: no text-bearing breadth (notes, table cells, run-property overrides) ships before the **run-scoped text mode** exists. The only post-V1 edit op that may precede it is document-metadata editing, because it has no text body.

### Phase 2 — `replace_text` run-scoped mode (the gate)

A new `mode: run_scoped` for `replace_text` (the only existing mode is `whole_element`).

- Targets a single `a:r` (run) within a resolved element, not the whole `txBody`.
- MUST mutate the run's `a:t` in place and preserve sibling runs, `a:hlinkClick`, `a:fld`, and `a:br`.
- Selecting a run requires extending the selector model: the V1 `element_id` + `sp_tree_path` cannot address a run, so a run coordinate (paragraph index, run index) is a new selector field added under `selector.guards`/`selector` rather than a fuzzy match.
- When `mode: whole_element` would lose rich constructs (`run_count > 1`, or any detected `a:fld`/`a:hlinkClick`/`a:br`), the implementation MUST harden `formatting_simplified` into a **refuse-or-confirm**: reject with `unsupported_edit` unless the patch explicitly opts into the lossy rewrite.

### Phase 3 — slide-model-independent edits

`set_document_metadata` (may precede Phase 2; needs no text body):

- Required: `document_id`, `base_revision`, `operation_id`.
- Optional fields, each mapping to one `docProps/core.xml` element: `title` → `dc:title`, `subject` → `dc:subject`, `creator` → `dc:creator`, `keywords` → `cp:keywords`.
- Guard: a match guard on the prior value of any field being changed; mismatch returns `SelectorGuardFailed`.
- Touches exactly one part (`docProps/core.xml`). No rels or content-type coupling.

`replace_notes_text` (needs Phase 2):

- Required: `slide_id`, `text`. Optional: `match`, `mode`, `format_policy`, `overflow_policy` (same semantics as `replace_text`).
- Resolution adds a new target variant: `slide_id` → slide rels → `ppt/notesSlides/notesSlideN.xml`. The notes body reuses the `p:sp/a:txBody` shape, so it reuses the run-scoped path.
- Newline mapping and the `formatting_simplified` contract are identical to `replace_text`.

Run-property overrides in `replace_text`:

- Optional `run_style` overrides (size/bold/italic/color/family/align) are accepted **only** when `mode: run_scoped`. On `whole_element` they fail validation. This is a hard boundary, not a default.

### Phase 4 — table cell text

`replace_table_cell_text` (needs Phase 2 + a table-style read model):

- Required: `element_id` (the graphic-frame table), `cell`: `{ "row": u32, "col": u32 }`, `text`.
- Adds a `(row, col)` cell-coordinate selector variant; the V1 `sp_tree_path` cannot express a path into `a:tbl`.
- MUST read `gridSpan`/`rowSpan`/`vMerge` and refuse merged cells with `unsupported_edit`. MUST NOT touch `a:tblGrid`.
- Requires a table-style inheritance read model (`a:tblStyleLst`/`a:tblStyle`/`a:tblPr`) so the run rewrite does not fabricate overriding `rPr` where the cell relies on inherited style.

### Phase 5 — validation hardening, then slide lifecycle

`add_slide`, `duplicate_slide`, `delete_slide`, `move_slide`:

- Gated behind orphaned-relationship / dangling-comment-author-ref / unreferenced-media garbage-collection validation, which MUST be built first.
- `move_slide` invalidates position-based agent IDs; output validation must flag `slide_order_mismatch`.
- `add_slide`/`duplicate_slide` require rels + content-type + layout wiring, unique `p:cNvPr` re-id, and explicit media clone/share decisions.

### Permanently preserve-only (never an edit op in V1+)

Chart text/values, SmartArt body/structure, masters/layouts/themes, transitions/animations, comments, OLE/embeddings, audio/video, external `r:link` media, and slide backgrounds remain preserve-only. Any attempt to edit them returns `unsupported_edit`. See [048](048-editability-catalogue.md) for rationale (chart dual-location workbook sync and SmartArt proprietary cache regeneration make byte-faithful edits impossible).

## Example Image Operation

```json
{
  "op": "add_image",
  "slide_id": "slide-1",
  "media_ref": "input-image-1",
  "content_type": "image/png",
  "bounds": { "x": 914400, "y": 1828800, "cx": 3657600, "cy": 2743200 },
  "alt_text": "Revenue chart screenshot"
}
```

The implementation resolves `media_ref` through API-provided bytes, then updates media parts, content types, relationships, and slide XML.
