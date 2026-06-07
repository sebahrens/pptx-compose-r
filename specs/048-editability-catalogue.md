# 048. Editability Catalogue

This is the exhaustive, normative enumeration of every editable feature class in a PPTX and its status in this rewrite. It is the companion to [001. Goals and scope](001-goals-and-scope.md): 001 sets the scope boundary in prose; this file lists every feature against that boundary so nothing is implicitly in or out.

Cleanroom rule: every status below is derived from the OOXML/Open Packaging Convention model and this spec suite, not from the legacy TypeScript implementation.

## Status Vocabulary

- **v1-edit** — an editable V1 operation ships for this feature.
- **v1-read** — exposed in the agent view in V1; not editable.
- **deferred** — editable in principle, explicitly out of V1, scheduled in a later phase with a stated prerequisite.
- **preserve-only** — never edited in V1; byte-preserved through edits. Some are preserve-only permanently (charts, SmartArt body, animations, masters), others only until a deferred op lands.
- **reject-at-parse** — cannot be opened for editing; must fail with a clear error (e.g. encryption).

## Editability Gate

A feature is **v1-edit** only if both hold:

- **(a)** the edit rewrites one part, or the fixed atomic set `{ media + slideN.xml.rels + [Content_Types].xml }`, while preserving internal and cross-part invariants; and
- **(b)** the touched part re-serializes faithfully (no silent loss of content present in it).

Anything whose correctness depends on a cache, a proprietary layout algorithm, a separate embedded workbook, or a cross-part reference graph fails (a) or (b) and cannot be **v1-edit**. The universal escape hatch is `unsupported_edit` at apply time; the rejection is never a corrupt deck.

## Text

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Whole-element text replace (text box / shape) | `p:sp/p:txBody/a:p/a:r/a:t` | v1-edit (`replace_text`) | Gated to `text_box`/`shape`. Plain-text, whole-element rewrite. | High: nuke-and-rebuild collapses multi-run `rPr` to the first run and drops `a:fld`/`a:hlinkClick`/`a:hlinkMouseOver`/`a:br`. Reported via `formatting_simplified`. |
| Run-scoped in-place text replace | `a:r/a:t` (single run) | deferred (Phase 2 — the gate) | Mutates one run, preserves siblings + hyperlinks/fields/breaks. | Low by design; this primitive exists to remove the high risk above. |
| Run properties write (size/bold/italic/color/family) | `a:rPr @sz @b @i`, `a:solidFill/a:srgbClr`, `a:latin@typeface` | v1-edit on new text only; deferred elsewhere | Settable in `add_text_box`. Overrides in `replace_text` only on the run-scoped mode. | Medium: writing onto the whole-element path would widen the lossy blast radius — forbidden. |
| Run properties read | `a:rPr @sz @b @i @u`, `a:solidFill/a:srgbClr`, `a:latin@typeface`, `@lang` | v1-read | Surfaced as a run-style summary for context. | None (read-only). |
| Advanced run props (strike/cap/baseline/char-spacing) | `a:rPr @strike @cap @baseline @spc` | preserve-only | Not read or written; `@dirty` defaults to 0 on synthesized runs. | Preserved only if the run is untouched. |
| Paragraph props (align/indent/spacing/line-spacing) | `a:pPr @algn @marL @lvl @indent`, `a:lnSpc`, `a:spcBef/spcAft` | preserve-only (align writable on new boxes) | Only `algn` is writable, and only in `add_text_box`. | Lost if a paragraph is rewritten by whole-element `replace_text`. |
| Bullets / numbering / list styles | `a:pPr/a:buChar|buAutoNum|buNone`, `a:txBody/a:lstStyle` | deferred / preserve-only | Auto-numbering needs cross-paragraph sync. `add_text_box` writes an empty `lstStyle`. | Lost if a paragraph is rewritten. |
| Hyperlinks / fields / soft breaks | `a:r/a:hlinkClick`, `a:p/a:fld`, `a:p/a:br` | preserve-only (deferred edit via Phase 2) | Detected to raise `formatting_simplified`; survive only when the run structure is not rewritten. Hyperlink editing also needs rel management. | High: lost through a whole-element paragraph rewrite. |
| Text body props (wrap/anchor/inset/autofit) | `a:txBody/a:bodyPr`, `a:spAutoFit/normAutoFit` | preserve-only | Not read; `add_text_box` hardcodes `wrap=square` + `spAutoFit`. | Preserved on existing shapes. |

## Geometry

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Move / resize text box, shape, picture | `p:sp/p:spPr/a:xfrm`, `p:pic/p:spPr/a:xfrm` | v1-edit (`move_resize_element`) | EMU bounds validated; overflow guarded. Preserves `rot`/`flipH`/`flipV`. | Low: rewrites only `a:off`/`a:ext`. |
| Move / resize graphic frame (chart/table/SmartArt) | `p:graphicFrame/p:xfrm` | v1-edit (`move_resize_element`) | Frame-level transform, never the cache. | Low: faithful because the chart/diagram cache is untouched. |
| Add text box geometry | `p:spTree` append `p:sp` with `a:xfrm` | v1-edit (`add_text_box`) | New shape; bounds required. | Low. |
| Z-order on insert | `p:spTree` child order | v1-edit hygiene fix | `insert.z_order` must be honored (front/back/index) or removed from the schema; no silently-ignored knob. | Low. |
| Reorder existing element z-order | `p:spTree` child order | deferred | No reorder op in V1. | Low (read-only exposure only). |
| Rotation / flip | `a:xfrm @rot @flipH @flipV` | preserve-only (read) | Read and preserved through `move_resize`; no rotate op. | Low; cheap post-V1 stretch. |
| Preset geometry + adjustments | `p:spPr/a:prstGeom@prst`, `a:avLst/a:gd` | preserve-only | New elements emit `prst=rect` + empty `avLst`; existing geometry preserved. | Preserved on unmodified shapes. |
| Connectors / endpoints | `p:cxnSp`, `cxnSpLocks` | preserve-only | Detected as `connector`; `move_resize` rejects. | Preserved. |
| Groups + nested children | `p:spTree/p:grpSp`, `group_path` | preserve-only / partial | Detected and traversable; `move_resize` rejects group bounds; `add_*` into a group is `unsupported_edit`. | Preserved. |
| Shape fills / outline / effects | `p:spPr/a:gradFill|pattFill|blipFill|noFill`, `a:ln`, `a:effectLst` | preserve-only | Not read or written. | Byte-preserved in unmodified parts. |

## Accessibility

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Title / alt text | `p:cNvPr@title`, `p:cNvPr@descr` | v1-edit (`set_alt_text`) | Any element with a `p:cNvPr`; `alt_text` maps to `descr`. Works on chart/table/SmartArt frames. | Low: single attribute edit. |

## Media / Images

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Add image | `p:spTree` (`p:pic`), `ppt/media/imageN.ext`, `slideN.xml.rels`, `[Content_Types].xml` | v1-edit (`add_image`) | Atomic fixed cross-part set. Public V1 patches expose no `fit` or `dedupe` fields; the operation emits `a:stretch` fill and creates a new media part. | Medium: the canonical multi-part edit; consistency validated atomically. |
| Replace image | `p:pic/p:blipFill/a:blip@r:embed` | v1-edit (`replace_image`) | Retargets `r:embed`; preserves `a:srcRect`, fill, effects, `a:xfrm`. Refcount-deletes old media at refcount 0. Rejects `r:link`. | Low: only the blip target changes. |
| Intrinsic dimension read (PNG/JPEG/GIF) | media binary headers | v1-read | Reads PNG IHDR, JPEG SOF, GIF header. | None. |
| GIF capability advertisement | agent view default capabilities | v1-edit hygiene fix | Operations accept GIF; advertised set must match (advertise `image/gif`, recommended, or strip from ops). | None. |
| External / linked pictures | `p:pic/p:blipFill/a:blip@r:link`, rels `TargetMode=External` | preserve-only | `replace_image` rejects with `unsupported_edit`; agent view must report `image.supported=false`, `reason=external_link`. | Preserved. |
| Picture crop / tile / recolor | `p:pic/p:blipFill/a:srcRect`, `a:tile`, `alphaModFix` | preserve-only | Preserved across `replace_image`; no crop/fill edit op. | Preserved (replace may leave crop geometrically wrong if dims differ — accepted post-V1 concern). |
| Slide background image / fill | `p:cSld/p:bg|p:bgPr/a:blipFill` | preserve-only / deferred | Distinct from `p:pic`; needs background insertion logic. | Preserved. |
| Audio / video | `a:videoFile`/`a:audioFile`, `ppt/media/video*` | preserve-only | Opaque binary; no detect/edit. | Preserved. |

## Tables

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Detection + geometry | `p:graphicFrame/a:graphic/a:graphicData[@uri=…table]/a:tbl` | v1-read (+ `move_resize` on frame) | Classified as `table` (kind fix); frame geometry editable. | Low (frame only). |
| Cell text read | `a:tbl/a:tr/a:tc/a:txBody` | deferred | The slide text reader is cell-agnostic and reusable for read. | None. |
| Cell text replace (non-merged) | `a:tc/a:txBody` | v1-edit (`replace_text` with `cell`) | Uses the run-scoped primitive + a table-style inheritance read model (`a:tblStyleLst`/`a:tblStyle`/`a:tblPr`); reads `gridSpan`/`rowSpan`/`vMerge` only to refuse merged cells. | High without the style model: clone-first-`rPr`-or-default would fabricate overriding `rPr`. Never touch `a:tblGrid`. |
| Structural edits (rows/cols/merge/widths/borders/fills) | `a:tblGrid/a:gridCol`, `a:tr@h`, `a:tc@gridSpan`, `a:vMerge`, `a:tcPr`, `a:tblPr` | deferred (very high) | Merge topology and `tblGrid`↔cell-count consistency must be validated atomically or PowerPoint rejects/misrenders. Needs a merge-semantics spec first. | Very high. |

## Charts

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Detection + geometry + alt text | `p:graphicFrame[graphicData uri=…chart]` | v1-read (+ `move_resize`, `set_alt_text`) | Classified as `chart`; frame geometry and alt text editable. | Low (frame/cNvPr only). |
| Title / axis / series / category text + values | `ppt/charts/chartN.xml` `c:tx`/`c:numCache`/`c:strCache`/`c:v`, `c:f` | preserve-only (permanent for V1+) | Dual-location: cache `c:v` vs embedded workbook via `c:f` in `ppt/embeddings/*.xlsx`. Editing the cache without the workbook is silently overwritten by PowerPoint on reopen. | Fatal to faithfulness. Chart-text READ is a possible inspection-only post-V1 feature. |
| Embedded workbooks / OLE objects | `ppt/embeddings/*.xlsx|oleObject*.bin`, `p:oleObj` | preserve-only (permanent) | Opaque nested OPC/CFBF blobs; never executed (security). | Byte-preserved. |

## SmartArt

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Detection + geometry + alt text | `p:graphicFrame[graphicData uri=…diagram]`, `ppt/diagrams/*` | v1-read (+ `move_resize`, `set_alt_text`) | Classified as `diagram`; frame geometry and alt text editable; all diagram parts byte-preserved. | Low (frame/cNvPr only). |
| Text / node / layout / color editing | `ppt/diagrams/data#.xml` `dgm:pt/a:txBody`, `layout#.xml`, `colors#.xml`, cached `ppt/drawings/drawing#.xml` | preserve-only (permanent for V1+) | Requires data↔layout↔cached-drawing consistency + PowerPoint's proprietary layout algorithm and cache regeneration (infeasible in OSS). | Editing without regen breaks rendering in non-PowerPoint viewers. Node-text READ is the only plausible post-V1 step. |

## Deck / Layout

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Placeholder detection | `p:nvSpPr/p:nvPr/p:ph@type@idx` | v1-read | Read into shape context; placeholder text edits go through normal `replace_text`. Exposing role/type is a cheap read stretch. | None. |
| Masters / layouts / themes | `ppt/slideMasters/*`, `ppt/slideLayouts/*`, `ppt/theme/*` | preserve-only | Byte-preserved; refs exposed for context only. Editing needs the full inheritance model. | Preserved. |
| Slide lifecycle (add/delete/duplicate/reorder) | `presentation.xml` `p:sldIdLst`, `p:sectionLst`, `p:custShowLst`; slide parts + rels + content-types | deferred (epic, Phase 5) | Reorder invalidates position-based agent IDs (`slide_order_mismatch`). Add/dup need rels + content-type + layout wiring, unique `p:cNvPr` re-id, media clone/share. Gated behind GC validation. | High. |
| Presentation properties | `p:sldSz`, `p:notesSz`, `p:hf`, `p:showPr`, `p:prnPr`, `p:embeddedFontLst` | preserve-only (read stretch) | Parsed where useful; editing `sldSz` would require reflowing all slide geometry. | Preserved. |
| Transitions / animations | `p:cSld` sibling `p:transition`; `p:sld/p:timing` | preserve-only (permanent for V1+) | `p:timing` references `p:cNvPr` ids; editing/deleting shapes can orphan refs (no GC). | Preserved through text/geometry edits. |

## Auxiliary Parts

| Feature | OOXML location | Status | Rationale | Preservation risk |
| --- | --- | --- | --- | --- |
| Document metadata | `docProps/core.xml` (`dc:title`/`dc:subject`/`dc:creator`/`cp:keywords`; deferred fields `cp:category`/`dc:description`) | deferred (FIRST post-V1 edit op: `set_document_metadata`) | Standalone part: no `txBody`, no run model, no rels/content-type coupling. Uses a part-scoped `core_properties` selector, `operation_id`, patch-level document/revision guards, and optional field-value/`part_checksum` guards. Initial settable fields are title, subject, creator, and keywords; category and description stay reserved until their schema task opts them in. | Low — does not need the run-scoped primitive. |
| App / custom metadata | `docProps/app.xml` (application stats), `docProps/custom.xml` | preserve-only | `app.xml` stats go stale after lifecycle ops and require a separate regeneration policy; custom properties need a typed property schema. `set_document_metadata` must not touch either part. | Preserved as blobs. |
| Speaker notes | `ppt/notesSlides/notesSlideN.xml` | deferred (Phase 3) | Pre-authorized by 001 for "simple note text". Needs slide→rels→notes resolution (absent today) + the run-scoped primitive. | Mirrors slide `txBody`; inherits whole-element risk until Phase 2. |
| Notes / handout masters | `ppt/notesMasters/*`, `ppt/handoutMasters/*` | preserve-only | Master edits out of scope. | Preserved. |
| Comments | `ppt/comments/comment#.xml`, `ppt/commentAuthors.xml`; legacy comment shapes | preserve-only | Byte-preserved; no validation that author refs resolve (post-V1 validation gap). | Preserved. |
| Custom XML / `mc:AlternateContent` / external rels / signatures | `customXml/*`, `mc:AlternateContent`, rels `TargetMode=External`, signature parts | preserve-only | Opaque parts. Edits invalidate package signatures (must warn per spec 090). External rels not target-validated (warning only). | Preserved. |

## Security

| Feature | OOXML location | Status | Rationale |
| --- | --- | --- | --- |
| Encrypted / password-protected packages | OPC encryption layer | reject-at-parse | Spec 090: fail with a clear error at parse. Not editable. |
| Signed / macro-containing decks | signature parts, `vbaProject.bin` | preserve-only + warn | Edits invalidate signatures; macros are never executed. |

## Cross-Cutting Validation Debt

The following do not exist today and are harmless in V1 (no delete ops), but are hard prerequisites for any deferred lifecycle work and are listed here so they are not forgotten:

- Orphaned-relationship detection.
- Dangling comment-author reference detection.
- Unreferenced-media detection after deletion.

Build this garbage-collection validation **before** any delete/lifecycle operation.

See [001. Goals and scope](001-goals-and-scope.md) for the scope boundary, [041. Agent edit operations](041-agent-edit-operations.md) for operation schemas and phasing, and [050. Round-trip invariants](050-roundtrip-invariants.md) for the preservation guarantees these statuses rely on.
