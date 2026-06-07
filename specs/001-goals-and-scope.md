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

Any target whose correctness depends on a cache, a proprietary layout algorithm, a separate embedded workbook, or a cross-part reference graph (charts, SmartArt, table styling/merge topology, slide lifecycle) fails one of these conditions and is therefore preserve-only or deferred. Every such target returns `unsupported_edit` at apply time rather than producing a corrupt deck.

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

**Edit (the six confirmed operations):**

- `replace_text` — replace the text of a `text_box`/`shape`. Whole-element, plain-text replacement only; multi-run styling collapses and `a:hlinkClick`/`a:fld`/`a:br` are not preserved through a paragraph rewrite. This loss is reported via the `formatting_simplified` warning and is the documented V1 contract.
- `add_text_box` — append a simple text box to a slide with basic font/fill/alignment style fields.
- `move_resize_element` — move or resize a `text_box`, `shape`, `picture`, or `graphic_frame`. Graphic-frame geometry edits are faithful because they touch only the frame-level `p:xfrm`, never the chart/SmartArt cache.
- `set_alt_text` — set title/alt-text metadata on any element with a `p:cNvPr`.
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

- **Document metadata** (`docProps/core.xml`: `dc:title`/`dc:subject`/`dc:creator`/`cp:keywords`) — slide-model-independent, no run model, no rels/content-type coupling. The single best first post-V1 edit op.
- **Non-destructive run-scoped text replacement** — a new `replace_text` mode that mutates individual `a:r`/`a:t` in place and preserves sibling runs, `a:hlinkClick`, `a:fld`, and `a:br`. This is the gate for all further text-bearing breadth; nothing below ships before it.
- **Speaker-notes text replacement** — pre-authorized by this spec for "simple note text". Needs slide→rels→`notesSlideN.xml` resolution (absent today) plus the run-scoped primitive.
- **Run-property overrides in `replace_text`** (size/bold/italic/color/family/align) — only on the run-scoped mode, never on the destructive whole-element path.
- **Table cell-text replacement on non-merged cells** — requires the run-scoped primitive and a table-style inheritance read model; reads `gridSpan`/`rowSpan`/`vMerge` only in order to refuse merged cells; never touches `a:tblGrid`.
- **Table structural edits** (rows/cols, merge/unmerge, widths, borders, fills) — needs a merge-semantics spec first.
- **Slide lifecycle** (add/delete/duplicate/reorder) — needs rels + content-type + layout wiring, unique `p:cNvPr` re-id, media clone/share decisions, position-based agent-ID invalidation handling, and the GC validation prerequisite.
- **Orphaned-relationship / dangling-comment-author-ref / unreferenced-media GC validation** — harmless in V1 (no delete ops) but a hard prerequisite for lifecycle; build before any delete op.

## Preserve-Only Areas in V1

These must be byte-preserved through any edit. Some are preserve-only permanently; others are merely not editable yet (see Deferred above):

- **Charts and embedded workbooks** — preserve-only permanently. Chart text lives in two places (`c:numCache`/`c:v` and the embedded `c:f` workbook in `ppt/embeddings/*.xlsx`); editing the cache without the workbook is silently overwritten by PowerPoint on reopen.
- **SmartArt body and structure** — preserve-only permanently. `dgm:pt` data must stay consistent with PowerPoint's proprietary layout cache (`ppt/drawings/drawing#.xml`), which cannot be regenerated in open source. (Detection, `move_resize`, and `set_alt_text` on the frame are de-facto V1 surface and are faithful because they never touch the cache.)
- **Tables beyond detection and geometry** — preserve-only in V1; cell-text editing is deferred (see above).
- **Animations and transitions** — preserve-only permanently in V1+; `p:timing` references `p:cNvPr` ids and there is no reference garbage collection.
- **Comments** (modern `ppt/comments/*` and `commentAuthors.xml`, legacy comment shapes).
- **Speaker notes** — preserve-only until the deferred notes-text op ships.
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
- Full chart/table/SmartArt editing in V1 (charts and SmartArt body/structure are non-goals permanently for V1+).
- Slide add/duplicate/delete/reorder in V1.
- Run-property styling layered onto the destructive whole-element `replace_text` path (must wait for the run-scoped primitive).
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
