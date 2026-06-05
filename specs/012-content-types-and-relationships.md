# 012. Content Types and Relationships

Content types and relationships are required for valid PPTX editing. Adding a ZIP file alone is not enough.

## Content Types

The package must parse `/[Content_Types].xml` into:

- Defaults by extension.
- Overrides by full part name.

Requirements:

- Every part must resolve to a content type.
- New XML presentation parts usually need explicit overrides.
- New binary media may use defaults if the extension content type already exists.
- Removing parts should remove stale overrides when safe.
- Deterministic mode should order defaults and overrides stably.

### Content-Type Resolution Algorithm

For a given ordinary part name, the content type is resolved in this exact order:

1. **Override (exact match):** if `[Content_Types].xml` has an `<Override PartName="...">` whose `PartName` equals the part's canonical name (leading slash, per 010), that content type wins.
2. **Default (extension match):** otherwise, take the part name's extension (the substring after the final `.`) and match it against `<Default Extension="...">` entries by **ASCII case-insensitive** comparison (`PNG`, `png`, and `Png` all match Default `png`). The matched default content type applies.
3. **Untyped:** if neither matches, the part has no content type. Validation emits `missing_content_type` (044), which is an `error` and blocks edited writes.

Override beats Default whenever both could apply. The extension comparison is the **only** case-insensitive comparison in part handling; part-name path segments remain case-sensitive (010).

## Relationships

The package must parse all `.rels` files as relationship sets.

Requirements:

- Preserve `Id`, `Type`, `Target`, and `TargetMode`.
- Resolve internal targets relative to the source part.
- Preserve external relationships without fetching them.
- Allocate non-conflicting relationship IDs when adding relationships (see policy below).
- Validate that every internal relationship target exists.

### Relationship ID Allocation

When adding a relationship to a relationship part, the new `Id` is allocated deterministically:

1. Scan existing `Id` values in that relationship part. Parse those matching `^rId(\d+)$` into their integer suffix `n`.
2. The new id is `rId{m+1}` where `m` is the maximum parsed `n` (or `m = 0` if none match the pattern).
3. The result must not collide with any existing `Id` in that part, including non-conforming ids that do not match `^rId(\d+)$`; if `rId{m+1}` already exists as a literal non-conforming id, increment until free.
4. Allocation is per relationship part and deterministic: the same starting state and the same sequence of additions always yields the same ids, so tests can assert a specific `rId`.

This is the policy referenced by `add_image`, `add_text_box`, and `replace_image` (041) and by DrawingML construction (047).

## PPTX Discovery Rules

A reader must discover the presentation through the OPC relationship graph rather than hardcoding `ppt/presentation.xml` as the only possible path:

1. Read package root relationships from `/_rels/.rels`.
2. Find the Office document relationship whose target is the presentation part.
3. Read slide order from `<p:sldIdLst>` in the presentation part.
4. Resolve each slide through the slide relationship IDs in the presentation relationship part.
5. Resolve each slide layout through the slide's own relationship part.
6. Preserve unknown root and part relationships.

Required V1 relationship/content-type categories include office document/presentation, slide, slide layout, slide master, theme, image, relationship parts, XML parts, PNG, JPEG, and GIF. SVG/EMF/WMF may be preserved as existing media but are not required for V1 image insertion unless explicitly supported by MIME/type detection.

## Adding an Image

Adding an image to a slide requires all of these steps:

1. Add image bytes as a media part, e.g. `/ppt/media/image7.png`.
2. Ensure `[Content_Types].xml` covers the image content type.
3. Add a relationship from the slide part to the media part.
4. Insert a `p:pic` element into the slide shape tree referencing the relationship ID.
5. Allocate a unique drawing non-visual ID for the picture.
6. Validate the package graph.

## Post-V1: Adding a Slide

Slide creation is not a V1 requirement. When added in a later phase, it requires all of these steps:

1. Create `/ppt/slides/slideN.xml`.
2. Create `/ppt/slides/_rels/slideN.xml.rels`.
3. Add a slide relationship in `/ppt/_rels/presentation.xml.rels`.
4. Add a `<p:sldId>` entry in `/ppt/presentation.xml`.
5. Add a content type override for the slide part.
6. Link to a valid slide layout.
7. Allocate unique slide relationship and slide IDs.
8. Validate slide order and relationship targets.
