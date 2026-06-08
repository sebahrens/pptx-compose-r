# 001. Goals and Scope

## Goal

Build a cleanroom Rust implementation that can read `.pptx` files, expose their content to AI agents, apply safe modifications or additions, and write valid `.pptx` files.

The rewrite should graduate from the current raw ZIP/XML JSON concept to a layered PPTX system:

1. Lossless OPC package handling.
2. Namespace-aware XML handling.
3. PPTX-aware presentation, slide, text, shape, and media models.
4. Agent-friendly JSON projection.
5. Explicit validated edit operations.

## Primary Users

- Rust applications that need PPTX inspection and mutation.
- Node/JavaScript users migrating from the existing package.
- AI agents that need a stable read/modify/write protocol.
- CLI users converting, inspecting, validating, or patching decks.

## Editability Model

Editing is allowed only where two conditions both hold:

- **(a) Local cross-part consistency** — applying the edit by rewriting one part, or one fixed atomic set of parts, preserves every internal and cross-part invariant. The only sanctioned multi-part set in V1 is `{ media part + slideN.xml.rels + [Content_Types].xml }` for image insertion/replacement.
- **(b) Faithful re-serialization** — the touched part can be re-serialized without silently corrupting the content that exists in it.

Any target whose correctness depends on a cache, a proprietary layout algorithm, a separate embedded workbook, or a cross-part reference graph (chart data/workbook authoring, SmartArt structure/layout/cache authoring, table styling/merge topology, slide lifecycle) fails one of these conditions and is therefore preserve-only or deferred. Existing visible chart and SmartArt text is the narrow V1 exception: it is in scope only when inspection and editing can keep the required chart XML/workbook or diagram data/cache parts consistent. Unsupported chart/SmartArt authoring returns `unsupported_edit` at apply time rather than producing a corrupt deck.

The full feature-by-feature breakdown lives in [048. Editability catalogue](048-editability-catalogue.md). This section is the normative scope boundary; 048 is the exhaustive enumeration that must agree with it.

## V1 Supported Operations

V1 supports a narrow but reliable set of edits plus the read surface that makes them targetable:

**Read / inspect (always available):**

- Open a PPTX from a path, bytes, or reader.
- List slides in presentation order.
- Extract slide text, shape names, basic bounds, and image references.
- Read run-level style summaries (size, bold, italic, underline, color, typeface, language) for context.
- Read image placements, intrinsic dimensions, and media part references.
- Classify graphic frames into chart, table, diagram/SmartArt, and OLE kinds from `a:graphicData/@uri` (so the agent view stops collapsing every graphic frame to `chart`).
- Inspect existing visible chart and SmartArt text with stable provenance, while
  keeping chart data authoring and SmartArt structure/layout authoring distinct
  from text editing.

**Edit (the confirmed operations):**

- `replace_text` — replace text in text-capable shapes/text boxes, supported
  table cells, supported notes text, and existing visible chart/SmartArt text
  when the target exposes stable text selectors. Whole-element replacement is a
  plain-text rewrite that may collapse rich text and reports
  `formatting_simplified`; `run_scoped` replacement edits one inspected run in
  place and preserves sibling formatting/structure.
- `add_text_box` — append a simple text box to a slide with basic font/fill/alignment style fields.
- `move_resize_element` — move or resize a `text_box`, `shape`, `picture`, or `graphic_frame`. Graphic-frame geometry edits are faithful because they touch only the frame-level `p:xfrm`, never the chart/SmartArt cache.
- `set_alt_text` — set title/alt-text metadata on any element with a `p:cNvPr`.
- `set_document_metadata` — set supported core properties in `docProps/core.xml`.
- `add_image` — add an image to a slide, atomically updating the media part, relationships, and content types.
- `replace_image` — retarget the embedded blip of an existing picture while preserving crop, fill, effects, and transform. Linked (`r:link`) pictures are rejected.

**V1 read-honesty / hygiene fixes (required to ship V1; they touch no edit path):**

These remove dishonest capability surfaces so the advertised contract equals the accepted contract:

1. **Graphic-frame kind classification** — emit `chart`/`table`/`diagram`/`ole` from `a:graphicData/@uri` instead of collapsing all graphic frames to `chart`. Wires the previously-dead `table` kind. Read-only.
2. **GIF advertisement alignment** — the advertised media capability set must equal the set the operations actually accept. Either advertise `image/gif` (recommended; sniffing already supports it) or strip GIF from the operations. Read-only / capability-only.
3. **External-link image honesty** — pictures backed by `r:link` must report `editable.image.supported = false` with `reason = external_link`, matching the apply-time rejection. Read-only.
4. **`insert.z_order` resolution** — `add_text_box` and `add_image` must either honor `z_order` (front/back/index placement) or the field must be removed from the schema. No silently-ignored knobs.

Slide lifecycle operations are not V1 requirements. Adding, duplicating, deleting, and reordering slides are post-V1 operations and are gated behind orphaned-relationship / dangling-reference garbage-collection validation; see [048](048-editability-catalogue.md) and [041. Agent edit operations](041-agent-edit-operations.md).

## Deferred Editable Areas (post-V1, with reasons)

These are editable in principle but explicitly out of V1. Each is scheduled in a phase (see [041](041-agent-edit-operations.md) and [048](048-editability-catalogue.md)) and stays preserve-only until then:

- **Table structural edits** (rows/cols, merge/unmerge, widths, borders, fills) — needs a merge-semantics spec first.
- **Slide lifecycle** (add/delete/duplicate/reorder) — needs rels + content-type + layout wiring, unique `p:cNvPr` re-id, media clone/share decisions, position-based agent-ID invalidation handling, and the GC validation prerequisite.
- **Template population and text fitting** — placeholder metadata, fit estimation,
  fit policies, and alignment rules are specified in [082](082-template-population-and-text-fitting.md)
  but not yet implemented.
- **Orphaned-relationship / dangling-comment-author-ref / unreferenced-media GC validation** — harmless in V1 (no delete ops) but a hard prerequisite for lifecycle; build before any delete op.

## Preserve-Only Areas in V1

These must be byte-preserved through any edit. Some are preserve-only permanently; others are merely not editable yet (see Deferred above):

- **Charts and embedded workbooks** — chart data/workbook authoring remains
  preserve-only; existing visible chart text inspection/editing is in V1 only
  through text-specific selectors that keep chart XML and backing workbook label
  cells consistent.
- **SmartArt body and structure** — data/layout/cache authoring remains
  preserve-only; existing visible SmartArt text inspection/editing is in V1 only
  through text-specific selectors that keep diagram data and rendered/cache text
  consistent.
- **Tables beyond detection, geometry, and supported cell text** — preserve-only
  in V1.
- **Animations and transitions** — preserve-only permanently in V1+; `p:timing` references `p:cNvPr` ids and there is no reference garbage collection.
- **Comments** (modern `ppt/comments/*` and `commentAuthors.xml`, legacy comment shapes).
- **Unsupported speaker-notes structures** beyond the supported simple note text
  replacement path.
- **Slide masters, layouts, and themes** (including `clrScheme`/`fontScheme` inheritance).
- **Slide backgrounds and fills** (`p:bg`/`p:bgPr`).
- **Shape-level fills, lines, and effects** (gradient/pattern/blip fills, `a:ln`, `a:effectLst`); preset geometry and adjustment handles; connectors and groups.
- **Custom XML parts, `mc:AlternateContent`, external relationships, package signatures.**
- **Unknown OOXML extension elements.**
- **Embedded OLE objects, audio, and video** — opaque binary; never executed.
- **Presentation properties** (`p:sldSz`, `p:notesSz`, `p:hf`, `p:showPr`, `p:prnPr`, embedded fonts) — parsed where useful for context but not edited; editing `sldSz` would require reflowing all slide geometry.

## Non-Goals

- A full PowerPoint renderer.
- Pixel-perfect layout computation.
- Complete ECMA-376 object model coverage.
- Full chart/table/SmartArt editing in V1. Chart data/workbook authoring,
  SmartArt structure/layout/cache editing, and table structural edits remain
  out of scope even if later text-specific support is added.
- Slide add/duplicate/delete/reorder in V1.
- Run-property styling layered onto the destructive whole-element `replace_text` path.
- Generating arbitrary presentations from high-level prose in V1.
- Byte-for-byte output equality for the entire ZIP when edits are made.
- Image EXIF/metadata stripping, format conversion, or recompression.
- Preserving historical TypeScript bugs unless a compatibility spec explicitly requires them.

## Success Criteria

- No-edit round trips preserve all parts and copy unmodified part bytes wherever possible.
- Common text/image edits produce PPTX files that open in PowerPoint-compatible tools.
- Added content updates every required OPC dependency: content type, relationship, and XML reference.
- Agent patches are explicit, validated, and fail atomically.
- Unknown deck content survives edits.
