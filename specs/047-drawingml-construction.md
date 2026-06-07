# 047. DrawingML Construction

V1 inserts two kinds of new elements into a slide shape tree: a picture (`add_image`) and a text box (`add_text_box`). The guardrail "never add media by only adding a file; update content types, relationships, and slide XML" makes the inserted XML load-bearing, yet no spec defined the element skeletons, the image relationship type URI, the EMU mapping for `bounds`, or the default text-box style. This spec is the normative source for constructing those elements. It governs only **newly constructed** elements; unmodified shape XML is preserved per spec 020.

All inserted XML must satisfy the round-trip and validation invariants in 050 and must be produced deterministically (same inputs → same bytes) so golden tests in 080 are possible.

## Namespaces

Constructed elements use these prefixes, declared on the existing slide root (`p:sld`) where not already present. Implementations must not redeclare a prefix already bound on an ancestor.

| Prefix | Namespace |
| --- | --- |
| `p` | `http://schemas.openxmlformats.org/presentationml/2006/main` |
| `a` | `http://schemas.openxmlformats.org/drawingml/2006/main` |
| `r` | `http://schemas.openxmlformats.org/officeDocument/2006/relationships` |

## Bounds and EMU Mapping

`bounds` in patches is `{ "x": emu, "y": emu, "cx": emu, "cy": emu }`. It maps to an `a:xfrm` under the element's shape properties:

```xml
<a:xfrm>
  <a:off x="{x}" y="{y}"/>
  <a:ext cx="{cx}" cy="{cy}"/>
</a:xfrm>
```

- All four values are non-negative 64-bit integers in EMUs (914400 EMU = 1 inch).
- `cx` and `cy` must be `> 0`; `x`/`y` must be `>= 0`. A violation fails dry-run validation with `invalid_bounds` (see 044 error-code additions).
- For top-level shapes (`group_path` empty), bounds are absolute slide coordinates. Construction of children inside an existing group is **post-V1**; `add_*` targeting a group child returns `unsupported_edit`.
- No `rot`, `flipH`, or `flipV` is emitted on newly constructed elements unless a future spec adds it.

## Drawing Non-Visual ID Allocation

Both elements require a `cNvPr` with `id` and `name`:

- `id` is allocated as `max(existing cNvPr id on the slide) + 1`, scanning the slide's `p:spTree` (including grouped descendants). Allocation must be deterministic and must not collide with any existing id on that slide (050 "Drawing non-visual IDs must be unique within each slide").
- `name` defaults to `"{Kind} {id}"` (`"Picture {id}"` or `"TextBox {id}"`) when the patch supplies no `name`; otherwise the patch `name` is used verbatim (XML-escaped).
- `descr` / `title` are set only when the patch supplies `alt_text` / `title` (see set_alt_text mapping in 041); otherwise omitted.

## Shape-Tree Insertion Order

`add_image` and `add_text_box` insert one new top-level child into the target
slide's `p:spTree`. V1 does not construct children inside an existing group.
The optional `insert.z_order` field in 041 maps to `p:spTree` child placement:

- omitted or `"front"` inserts immediately after the last real shape-tree child (`p:sp`, `p:pic`, `p:grpSp`, `p:graphicFrame`, or `p:cxnSp`).
- `"back"` inserts immediately before the first real shape-tree child, while preserving required non-visual group properties (`p:nvGrpSpPr`) and group properties (`p:grpSpPr`) at the beginning of `p:spTree`.
- integer `N` inserts at the 1-based agent element ordinal `N`, counting XML element children in `p:spTree`; `N` must fall between the current back insertion ordinal and the current front insertion ordinal, inclusive.

The inserted element's agent id uses its resulting ordinal (`slide-N:shape-M`).
Existing elements' raw XML bytes remain unchanged, but element ids derived from
shape-tree ordinals can shift for elements after the insertion point. Agents
must refresh the agent view before targeting those shifted elements.

## Relationship Type URIs

| Purpose | Relationship `Type` |
| --- | --- |
| Slide → image media (`r:embed`) | `http://schemas.openxmlformats.org/officeDocument/2006/relationships/image` |

Relationship `Id` allocation follows the rId policy in [content types and relationships](012-content-types-and-relationships.md). The relationship `Target` is authored relative to the slide part (e.g. `../media/image7.png`) with `TargetMode` internal.

## `p:pic` Template (add_image / replace_image retarget)

`add_image` inserts this element into `p:spTree`; `replace_image` reuses the existing `p:pic` and only retargets `r:embed`/bounds (041), so this template is the canonical shape `add_image` produces:

```xml
<p:pic>
  <p:nvPicPr>
    <p:cNvPr id="{id}" name="{name}"/>            <!-- + descr/title when alt_text/title supplied -->
    <p:cNvPicPr>
      <a:picLocks noChangeAspect="1"/>
    </p:cNvPicPr>
    <p:nvPr/>
  </p:nvPicPr>
  <p:blipFill>
    <a:blip r:embed="{rId}"/>
    <a:stretch>
      <a:fillRect/>
    </a:stretch>
  </p:blipFill>
  <p:spPr>
    <a:xfrm>
      <a:off x="{x}" y="{y}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="rect">
      <a:avLst/>
    </a:prstGeom>
  </p:spPr>
</p:pic>
```

- `fit: stretch` (041 default) is realized by the `a:stretch`/`a:fillRect` fill above. `contain`, `cover`, and `original_size` are not realized in V1 and must be rejected with `unsupported_edit` rather than emitting a different fill.
- `noChangeAspect="1"` is fixed for V1-inserted pictures.

## `p:sp` Template (add_text_box)

```xml
<p:sp>
  <p:nvSpPr>
    <p:cNvPr id="{id}" name="{name}"/>            <!-- + descr/title when alt_text/title supplied -->
    <p:cNvSpPr txBox="1"/>
    <p:nvPr/>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm>
      <a:off x="{x}" y="{y}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="rect">
      <a:avLst/>
    </a:prstGeom>
  </p:spPr>
  <p:txBody>
    <a:bodyPr wrap="square" rtlCol="0">
      <a:spAutoFit/>
    </a:bodyPr>
    <a:lstStyle/>
    <a:p>
      <a:r>
        <a:rPr lang="en-US" dirty="0"/>
        <a:t>{text}</a:t>
      </a:r>
    </a:p>
  </p:txBody>
</p:sp>
```

### Deterministic Default Text-Box Style

This is the "deterministic default style" referenced by 031 and the resolution of the `add_text_box.style` whitelist gap in 041:

- The element carries **no explicit fill, line, or shadow** (`p:spPr` contains only `a:xfrm` + `a:prstGeom`), so the text box is transparent and unbordered.
- The single run carries `lang="en-US"` and no explicit font size, color, or typeface; rendered appearance therefore inherits from the slide layout/master placeholder defaults and theme. This is intentional and deterministic: the emitted bytes do not vary by environment.
- `a:bodyPr` uses `wrap="square"` with `a:spAutoFit`.

### V1 `style` Field Whitelist

`add_text_box.style` accepts only the following fields in V1. Any other key fails dry-run validation with `unsupported_edit` (041: "Unknown style fields fail validation rather than being silently ignored"):

| Field | Type | Effect |
| --- | --- | --- |
| `font_size_pt` | number (pt) | Sets `a:rPr/@sz` = `round(font_size_pt * 100)` on the run. |
| `bold` | boolean | Sets `a:rPr/@b` = `1`/`0`. |
| `italic` | boolean | Sets `a:rPr/@i` = `1`/`0`. |
| `font_family` | string | Adds `<a:latin typeface="{font_family}"/>` under `a:rPr`. |
| `color_hex` | string `RRGGBB` | Adds `<a:solidFill><a:srgbClr val="{color_hex}"/></a:solidFill>` under `a:rPr`. |
| `align` | `left`\|`center`\|`right` | Sets `a:p/a:pPr/@algn` = `l`\|`ctr`\|`r`. |

These same fields are the only ones reported as supported in the `capabilities` block (042). The read-side `style_summary` (042) exposes the corresponding `font_size_pt`/`bold` for inspection.

## Text and Newline Mapping

For both `add_text_box.text` and `replace_text.text`, newline handling is fixed for V1:

- `\n` maps to a new paragraph (`a:p`), not a soft break, so each line is an independent paragraph. The chosen mapping is reported in the patch report `newline_mapping: "paragraph"` field (044).
- `a:t` text is XML-escaped per normal XML rules; this is a constructed (dirty) element, so escaping applies (contrast 020's raw preservation for unmodified parts).

## Validation of Constructed Elements

After insertion, the package must pass (050):

- The new `r:embed` (pictures) resolves to a slide relationship whose internal target exists and whose media part has a content type (012/032).
- The new `cNvPr id` is unique within the slide.
- The slide part remains well-formed and required namespaces are declared.

A single golden fixture per element (a minimal deck plus the exact expected inserted bytes) is required by 080.

See [slides, shapes, and text](031-slides-shapes-and-text.md), [media and images](032-media-images.md), [agent edit operations](041-agent-edit-operations.md), and [content types and relationships](012-content-types-and-relationships.md).
