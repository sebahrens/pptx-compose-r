# 082. Template Population and Text Fitting

Template population is the next agent-facing layer on top of bounded inspection
and patch operations. V1 already exposes placeholder/layout metadata and
conservative `replace_text.fit_policy` handling; the remaining goal is a
first-class multi-placeholder operation that fills existing template
placeholders without silently overflowing text or destabilizing visual alignment
when text scales or boxes resize.

## Problem Statement

Agents can inspect element bounds, placeholder metadata, direct text-body layout,
and replace text with conservative fit warnings or rejection. They still cannot
answer every template-population question reliably:

1. Which placeholder should be selected for a requested content field when
   several layout, slide, and inherited candidates exist?
2. How much text can fit in that placeholder with the current style and body
   layout?
3. If the text does not fit, which safe policy should be applied without moving
   visually aligned elements out of place?

Without that model, agents either overfill placeholders or compensate by resizing
elements in ways that break title/body/footer alignment and grouped layouts.

## Current Capabilities

- Agent views expose stable slide/element identities, part provenance, element
  bounds in EMUs, kind, editability, text paragraphs/runs, and direct run style
  summaries.
- Shape parsing exposes placeholder metadata from `p:nvPr/p:ph` as
  `ElementView.placeholder`, including type, optional index, source, and resolved
  layout part when known.
- Agent views expose direct text-body layout summaries as
  `ElementView.text_layout`, including `a:bodyPr` wrap/anchor/insets/autofit,
  paragraph default alignment, and `style_confidence`.
- Slide views expose `layout_part`, while master/theme/style inheritance remains
  intentionally shallow.
- `replace_text` supports lossy `whole_element` replacement and faithful
  `run_scoped` replacement. It accepts `fit_policy` with `preserve`,
  `fail_if_overflow`, and degenerate V1 `shrink_text` behavior.
- `move_resize_element` can change geometry, but it has no alignment policy.
- `add_text_box` supports explicit bodyPr/autofit-related style options:
  `autofit`, `vertical_anchor`, and body inset EMU fields.
- Validation is structural/package-level; it does not detect visual overflow.

## V1 Read Model Extension

V1 exposes optional template metadata in `ElementView` for shapes and
text-capable elements:

```json
{
  "placeholder": {
    "type": "title",
    "idx": 1,
    "source": "slide",
    "layout_part": "ppt/slideLayouts/slideLayout1.xml"
  },
  "text_layout": {
    "body_pr": {
      "wrap": "square",
      "anchor": "top",
      "inset_l": 91440,
      "inset_r": 91440,
      "inset_t": 45720,
      "inset_b": 45720,
      "autofit": "no_autofit"
    },
    "paragraph_defaults": {
      "align": "left"
    },
    "style_confidence": "direct_only"
  }
}
```

Current fields:

- placeholder `type` and optional `idx`, using OOXML placeholder vocabulary where
  possible;
- direct `a:bodyPr` wrap, vertical anchor, insets, and autofit kind
  (`no_autofit`, `norm_auto_fit`, `shape_auto_fit`, or `unknown`);
- paragraph alignment and direct run font sizes where available;
- confidence indicating whether style data is direct (`direct_only`) or unknown.

Do not block this read model on full master/theme inheritance. Emit partial data
with `style_confidence` rather than pretending to know exact PowerPoint layout.

## Fit Estimation

V1 uses a conservative, non-rendering estimate in `replace_text` dry-run/apply
reports. The estimate is deterministic and intentionally approximate:

```json
{
  "fit": {
    "status": "fits",
    "confidence": "medium",
    "estimated_lines": 3,
    "available_height_emu": 720000,
    "scale_needed": 0.92,
    "suggested_font_size_pt": 18,
    "reason": null
  }
}
```

Estimator outline:

1. Compute available text rectangle from element bounds minus body insets.
2. Resolve font size from direct run style, then placeholder/layout defaults, then
   conservative fallback by placeholder type.
3. Estimate average glyph width by script: Latin roughly `0.50–0.55em`, CJK
   roughly `1.0em`, mixed/complex scripts lower confidence.
4. Wrap text by available width when `wrap=square`; preserve explicit paragraphs
   and soft breaks.
5. Estimate line height as `1.15–1.25 * font_size`, plus paragraph spacing when
   known.
6. Binary-search font size down to the policy minimum to find the smallest scale
   that fits.

Fit reports should include confidence when exposed. Commands fail only when the
caller selects `fit_policy.mode: fail_if_overflow` or V1 `shrink_text` would
require actual shrinking.

## Fit Policies

`replace_text` includes `fit_policy` before introducing a new
template-population operation:

```json
{
  "fit_policy": {
    "mode": "preserve|fail_if_overflow|shrink_text",
    "min_font_size_pt": 14
  }
}
```

- `preserve`: current behavior; dry-run/apply may emit `text_overflow_risk`.
- `fail_if_overflow`: reject when the estimator predicts overflow with
  sufficient confidence.
- `shrink_text`: accepted by the V1 schema only when no shrink is required. If
  the estimate predicts overflow, V1 rejects rather than mutating run sizes.
  Uniform direct run-size reduction down to `min_font_size_pt` is post-V1.

Keep placeholder bounds fixed by default. Prefer shrinking text within the
existing box over resizing the shape, because resizing often breaks visual
alignment and may overlap siblings.

## Alignment and Autoscaling Rules

- Preserve original shape geometry unless the caller explicitly permits resize.
- Prefer existing `normAutoFit` semantics or explicit run-size shrink. Avoid
  `spAutoFit` for template population because it can change shape dimensions at
  render time.
- If resize is explicitly allowed, require an anchor policy:
  - titles: usually preserve center or top-left according to paragraph alignment;
  - body placeholders: preserve top-left and width, grow downward if within slide
    bounds;
  - footers/date/slide numbers: preserve bottom/baseline alignment and grow
    upward or shrink text.
- Grouped targets should warn or fail on resize until group bounds and sibling
  constraints are modeled.
- Any resize policy should report geometry deltas and overlap risk; do not hide
  layout changes in a text replacement report.

## Future `populate_placeholders` Operation

With placeholder metadata and fit advisory available in V1, the next layer is a
first-class operation for atomic multi-placeholder population:

```json
{
  "op": "populate_placeholders",
  "operation_id": "op-fill-slide-3",
  "slide_id": "slide-3",
  "bindings": [
    {
      "placeholder": { "type": "title", "idx": 1 },
      "text": "Market outlook",
      "fit_policy": { "mode": "shrink_text", "min_font_size_pt": 18 }
    }
  ],
  "alignment_policy": {
    "preserve_placeholder_center": true,
    "preserve_group_bounds": true,
    "avoid_overlap": "warn"
  }
}
```

The report must name the selected element for each binding, the replacement mode
used, fit action taken, warnings, and any geometry delta.

## Tests and Evals

- Inspect emits placeholder type/index and bodyPr autofit/wrap/insets/anchor.
- Fit estimator unit tests cover short title, long title, multiline body,
  mixed-size runs, CJK text, and low-confidence inherited styles.
- Dry-run with `fit_policy: fail_if_overflow` rejects obvious overflow.
- Dry-run with `fit_policy: preserve` succeeds with `text_overflow_risk`.
- V1 `shrink_text` succeeds only when no shrinking is required and rejects
  predicted overflow; post-V1 real shrinking must preserve sibling run
  formatting and respect `min_font_size_pt`.
- Visual evals cover before/after clipping, overlap, and alignment drift on
  PowerPoint-, Google Slides-, and LibreOffice-authored templates.

## Follow-Up Work

Completed implementation beads established the V1 substrate:
placeholder/layout metadata, conservative `replace_text.fit_policy`, explicit
`add_text_box` bodyPr/autofit options, and chart/SmartArt visible-text scope.
Remaining follow-up work should be tracked as new beads for the specific
post-V1 gaps: first-class `populate_placeholders`, richer inherited style
resolution, real `shrink_text` run-size mutation, and alignment/overlap policy
reports.
