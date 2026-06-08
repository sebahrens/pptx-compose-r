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
- `set_document_metadata`
- `add_image`
- `replace_image`

Post-V1 operations are not advertised in V1 `capabilities.operations` until
their implementation task ships.

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

Selectors must resolve to exactly one target unless the operation explicitly supports multiple targets. Fuzzy matching is forbidden by default. Guard failures return structured stale/guard error codes rather than applying best-effort edits. `guards.text_hash` is the content guard for the element's normalized text projection; `guards.fingerprint` is a stable structural identity guard and does not change only because that element's text, or a sibling element's text in the same part, changed.

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
- `mode`: `whole_element` by default; `run_scoped` edits one inspected run in
  place using `selector.run` or the operation-level `run` coordinate.
- `format_policy`: `preserve_existing_runs`, `preserve_first_run`, or `single_run_default_style`.
- `allow_formatting_simplification`: explicit confirmation for lossy
  whole-element rewrites. Implementations with `run_scoped` support MUST reject
  lossy `whole_element` rewrites unless it is `true`.
- `run_style`: accepted only with `mode: run_scoped`; supported fields are
  `font_size_pt`, `bold`, `italic`, `color_hex`, `font_family`, and paragraph
  `align` (`left`, `center`, `right`).

`replace_text` has two shipped text-body modes. `whole_element` is a plain-text,
potentially lossy rewrite of the selected text body. The implementation filters
the existing `a:txBody` down to `bodyPr`/`lstStyle`, clones the first run's
`a:rPr` onto every rebuilt run when the format policy preserves formatting, and
resynthesizes `a:p`/`a:r`/`a:t` by splitting the new text. As a consequence:

- Multi-run paragraphs collapse to a single run style.
- `a:pPr`, `a:fld`, `a:hlinkClick`, `a:hlinkMouseOver`, `a:br`, and literal
  line-break characters inside `a:t` are detected but **not** preserved through
  the rewrite; they survive only incidentally and are lost when paragraphs are
  rewritten.

These losses are reported via the `formatting_simplified` warning, which is the
documented V1 contract. The warning MUST be emitted whenever the original
`a:txBody` has `run_count > 1` or contains any detected rich construct
(`a:pPr`, `a:fld`, `a:hlinkClick`, `a:hlinkMouseOver`, `a:br`, or literal
line-break characters inside `a:t`) that the rewrite cannot preserve.
Run-property overrides (size/bold/italic/color/family/align) MUST NOT be added
to this whole-element path; on `mode: whole_element` they fail validation.

Newlines in whole-element replacement text must have a documented mapping. V1
maps `\n` to PowerPoint paragraphs (hard breaks), never soft `a:br` breaks, and
reports the chosen mapping in the patch report. For `mode: run_scoped`, `text`
is literal run text except that U+000B vertical tab is the V1 in-run soft-break
sentinel and serializes as paragraph-level `<a:br/>` between cloned run
segments. Raw U+000B MUST NOT be written into XML text. Line-break characters
`\n` and `\r` remain invalid in `run_scoped` text.

Targets include text-capable `text_box`/`shape` elements, table cells addressed
with `cell: { "row": N, "col": N }`, supported slide speaker-notes text, and
existing visible chart/SmartArt text addressed by their text-specific selectors.
Chart data/workbook authoring, SmartArt structure/layout/cache authoring, OLE
payloads, groups, and connectors remain unsupported for `replace_text`.

### `add_text_box`

Required fields:

- `slide_id`
- `text`
- `bounds`: `{ "x": emu, "y": emu, "cx": emu, "cy": emu }`

Optional fields:

- `name`
- `alt_text`
- `style`: implementation-defined basic font/fill and text-body options. V1 validates `font_size_pt`, `bold`, `italic`, `color_hex`, `font_family`, `align` (`left`/`center`/`right`), `autofit` (`no_autofit`/`norm_auto_fit`/`shape_auto_fit`), `vertical_anchor` (`top`/`middle`/`bottom`), and body inset EMU fields `inset_l`, `inset_r`, `inset_t`, and `inset_b`.
- `insert`: `{ "z_order": "front" }` by default.

Supported V1 style fields must be explicit in implementation docs. Unknown style fields fail validation rather than being silently ignored.

`insert.z_order` is honored for V1 `add_text_box` and `add_image` by choosing
the insertion position in the target slide's top-level `p:spTree`:

- omitted or `"front"`: insert after the last real shape-tree child, so the new element is visually in front of existing shapes.
- `"back"`: insert before the first real shape-tree child, after required shape-tree properties such as `p:nvGrpSpPr` and `p:grpSpPr`, so the new element is visually behind existing shapes.
- integer `N`: insert at agent element ordinal `N`, using the same 1-based ordinal space as `slide-N:shape-N` element ids and `Element View.z_order`. `N` must be between the current back ordinal and the current front insertion ordinal, inclusive; out-of-range values fail validation with `invalid_input`.

Insertion does not rewrite existing shape XML, but it can shift the ordinal
component of later agent element ids in the same `p:spTree`. Agents that need to
target pre-existing elements after an insertion must refresh the agent view or
use selector guards; the inserted element's id is assigned from its resulting
ordinal.

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
- `insert`: same `z_order` semantics as `add_text_box`; omitted means `"front"`.

V1 always emits a stretched fill and always creates a new media part. Public V1
patches do not include `fit` or `dedupe` fields until non-stretch sizing or
checksum reuse is implemented.

### `replace_image`

Required fields:

- `element_id`: target picture element.
- `media_ref`: key for replacement bytes.
- `content_type`

The implementation keeps the existing picture element and relationship ID while retargeting that relationship to replacement media. It must update content types and relationships atomically.

V1 `replace_image` behavior is always `retarget_picture`: create a media part and update only the selected picture relationship. It never mutates existing media bytes in place, including when the old media part is unshared.

`replace_image` rejects pictures backed by `a:blip@r:link` (external links) with `unsupported_edit`. The agent view MUST report such pictures as `editable.image.supported = false` with `reason = external_link` so agents do not attempt an edit that will be rejected at apply.

See [agent protocol schemas](042-agent-protocol-schemas.md) and [media staging and references](043-media-staging-and-refs.md) for normative schema details.

## Specialized Operation Semantics

This section records operation details that were originally written as phase
gates. The current implementation advertises `set_document_metadata` and
supports `replace_text` run-scoped mode, table-cell text, and notes text. Treat
those entries as current V1 semantics, not deferred work. Later operation phases
must not contradict these shipped contracts. The [editability catalogue](048-editability-catalogue.md)
is the exhaustive feature-by-feature mapping.

### `replace_text` run-scoped mode

`mode: run_scoped` for `replace_text` edits one inspected run in place. Do not
define a separate run-preserving replacement mode.

Scope:

- Applies to inspected runs in text-capable shapes and text boxes. Table cells
  and notes use the same single-run coordinate model when the operation target
  identifies that text body. Existing visible chart and SmartArt text may use
  the same model when the agent view exposes stable text provenance. Groups, OLE
  payloads, connectors, chart data/workbook authoring, and SmartArt
  structure/layout/cache authoring remain unsupported.
- Targets one `a:r` within one `a:p`, not the whole `a:txBody`. Run ranges are
  reserved until they are implemented and advertised by schema/capabilities.

Selector addressing:

- Run-scoped replacement extends the resolved element model with a run
  coordinate:
  `{ "paragraph_index": u32, "run_index": u32 }` for a single run.
- The coordinate lives in the canonical selector as `selector.run`, not as a
  fuzzy text query. `selector.type` remains `element_id`, and the element-level
  guards (`slide_id`, `kind`, `part`, `fingerprint`) still identify the owning
  shape.
- `selector.run` indexes only literal `a:r` children. `a:br`, `a:fld`, and other
  paragraph children are not counted as runs. A coordinate that names a missing
  paragraph or run returns `selector_not_found`.

Mutation contract:

- A single-run replacement MUST mutate only that run's `a:t` text in place.
  Existing `a:rPr` and other children of the selected `a:r`, including
  `a:hlinkClick` and `a:hlinkMouseOver`, MUST remain unchanged in the XML tree;
  serialization may only change escaping required inside the dirty `a:t` text
  node.
- The operation MUST preserve sibling runs and their `a:rPr`, sibling
  `a:hlinkClick`/`a:hlinkMouseOver`, sibling `a:fld`, and sibling `a:br`
  untouched. It MUST NOT rewrite `a:p`, `a:txBody`, `bodyPr`, `lstStyle`, or
  unrelated paragraph children.
- Replacement text in `run_scoped` mode is literal run text, with U+000B
  vertical tab reserved as the V1 soft-break sentinel. Implementations MUST
  serialize that sentinel as `<a:br/>` between cloned run segments and MUST NOT
  write raw U+000B into an XML text node. `\n` and `\r` are invalid and MUST NOT
  be mapped to paragraphs or `a:br` by this mode.

Guard semantics:

- `selector.guards.text_hash` remains an element-level guard over the normalized
  text projection for the whole text body.
- `selector.guards.fingerprint` remains an element identity guard and is not
  salted by the element's text. Agents that need stale-content protection should
  include `text_hash` as well as `fingerprint`.
- Run-scoped patches MAY instead use `selector.run.text_hash`, computed over the
  normalized text of exactly the selected run before
  mutation. When both element-level and run-level hashes are present, both MUST
  match or validation fails with `selector_guard_failed`.
- The legacy operation-level `match` field guards the whole element text in
  `whole_element` mode. In `run_scoped` mode it guards the selected run text,
  not the whole element; mismatch returns `selector_guard_failed`.

Whole-element refusal with run-scoped support:

- Since `run_scoped` is implemented, `mode: whole_element` remains available only
  for intentionally plain-text rewrites. If the source `a:txBody` has
  `run_count > 1` or contains `a:pPr`, `a:fld`, `a:hlinkClick`,
  `a:hlinkMouseOver`, `a:br`, or literal line-break characters inside `a:t`,
  validation MUST reject the operation with `unsupported_edit` unless
  `allow_formatting_simplification: true` is present.
- When explicitly confirmed, the operation still emits the
  `formatting_simplified` warning and performs the documented lossy
  whole-element rewrite. Confirmation is not allowed to silence the warning.

### `set_document_metadata`

- Required: `operation_id`, `op: "set_document_metadata"`, and at least one
  field under `metadata`.
- Patch-level `document_id` and `base_revision` are required by the patch
  envelope and guard the whole package/revision as usual. This operation also
  resolves a part-scoped target for `docProps/core.xml`; it is not an element
  selector and MUST NOT resolve through `p:spTree`.
- Canonical target selector:
  `{ "type": "core_properties", "part": "docProps/core.xml", "guards": { ... } }`.
  The selector resolves to a new `ResolvedTarget::CoreProperties` variant whose
  report target is `{ "part": "docProps/core.xml" }`.
- Initial settable fields, each mapping to one `docProps/core.xml` element:
  `title` -> `dc:title`, `subject` -> `dc:subject`, `creator` -> `dc:creator`,
  and `keywords` -> `cp:keywords`.
- Deferred fields, accepted only after their implementation task extends this
  operation schema: `category` -> `cp:category` and `description` ->
  `dc:description`. Unknown metadata fields are rejected by default.
- Guard options:
  - `match` may name any settable metadata field and its expected current
    string value. A mismatch returns `selector_guard_failed`.
  - `selector.guards.part_checksum` MAY guard the raw current bytes of
    `docProps/core.xml`, using the `part_checksum` format from
    [046](046-provenance-and-hashing.md). A mismatch returns
    `selector_guard_failed`.
  - When both `match` and `part_checksum` are present, both MUST match before
    any mutation is allowed.
- Missing `docProps/core.xml` returns `selector_not_found` unless a later spec
  explicitly allows creating the core-properties part.
- The operation touches exactly one part (`docProps/core.xml`). It does not
  update relationships, `[Content_Types].xml`, slide XML, or agent element IDs.
  The only expected changed part in the report is `docProps/core.xml`.
- `docProps/app.xml` and `docProps/custom.xml` remain preserve-only. In
  particular, app.xml statistics are not regenerated by this operation because
  they can become stale after lifecycle edits and need a separate policy.

Example:

```json
{
  "operation_id": "op-metadata-title",
  "op": "set_document_metadata",
  "selector": {
    "type": "core_properties",
    "part": "docProps/core.xml",
    "guards": {
      "part_checksum": "sha256:example-core-properties-checksum"
    }
  },
  "match": {
    "title": "Old deck title"
  },
  "metadata": {
    "title": "New deck title",
    "creator": "Research Team"
  }
}
```

`replace_text` with a slide selector for supported notes text:

- Required: `slide_id` or `selector: { "type": "slide_id", ... }`, `text`, `run`.
  Optional: `match`, `mode`, `format_policy` (same semantics as
  element-targeted `replace_text`).
- Resolution adds a new target variant: `slide_id` → slide rels → `ppt/notesSlides/notesSlideN.xml`. The notes body reuses the `p:sp/a:txBody` shape, so it reuses the run-scoped path.
- Newline mapping and the `formatting_simplified` contract are identical to `replace_text`.

Run-property overrides in `replace_text`:

- Optional `run_style` overrides (size/bold/italic/color/family/align) are accepted **only** when `mode: run_scoped`. On `whole_element` they fail validation. This is a hard boundary, not a default.
- Supported keys are `font_size_pt`, `bold`, `italic`, `color_hex`,
  `font_family`, and `align`. Unknown keys fail validation rather than being
  ignored. `color_hex` is exactly six hexadecimal characters (`RRGGBB`).
  `align` uses the same values as `add_text_box.style.align` and maps to the
  selected paragraph's `a:pPr/@algn`; all other fields apply to the first
  selected run's `a:rPr`.

### Table cell text

`replace_text` with a table-cell selector:

- Required: `element_id` or `selector: { "type": "element_id", ... }` for the
  graphic-frame table, `cell`: `{ "row": u32, "col": u32 }`, `text`.
- Adds a `(row, col)` cell-coordinate selector variant; the V1 `sp_tree_path` cannot express a path into `a:tbl`.
- MUST read `gridSpan`/`rowSpan`/`vMerge` and refuse merged cells with `unsupported_edit`. MUST NOT touch `a:tblGrid`.
- Requires a table-style inheritance read model (`a:tblStyleLst`/`a:tblStyle`/`a:tblPr`) so the run rewrite does not fabricate overriding `rPr` where the cell relies on inherited style.

### Deferred validation hardening, then slide lifecycle

`add_slide`, `duplicate_slide`, `delete_slide`, `move_slide`:

- Gated behind orphaned-relationship / dangling-comment-author-ref / unreferenced-media garbage-collection validation, which MUST be built first.
- `move_slide` invalidates position-based agent IDs; output validation must flag `slide_order_mismatch`.
- `add_slide`/`duplicate_slide` require rels + content-type + layout wiring, unique `p:cNvPr` re-id, and explicit media clone/share decisions.

### Preserve-only complex payloads

Chart data/value/workbook authoring, SmartArt structure/layout/cache authoring, masters/layouts/themes, transitions/animations, comments, OLE/embeddings, audio/video, external `r:link` media, and slide backgrounds remain preserve-only. Existing visible chart and SmartArt text inspection/editing is V1 scope when text-specific selectors can keep the required companion parts consistent; unsupported chart data or SmartArt authoring edits are not a blanket decision about all visible chart/SmartArt text. See [048](048-editability-catalogue.md) for rationale.

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
