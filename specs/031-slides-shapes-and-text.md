# 031. Slides, Shapes, and Text

## Text Reading

The parser should extract text from common DrawingML containers:

- Shape text bodies.
- Text boxes.
- Placeholder text.
- Grouped shapes where practical.
- Existing visible chart text such as titles, axis labels, legends, data labels,
  and category/series labels when provenance can identify the chart XML and any
  backing workbook label cells.
- Existing visible SmartArt text when provenance can identify the diagram data
  part and any rendered/cache text that must stay in sync.

Text extraction must preserve enough provenance to edit safely:

- Slide ID.
- Element ID.
- Paragraph/run identity when available.
- Source XML part.

## Text Editing

V1 operations:

- Replace all text in a text box.
- Replace text by exact match within an element.
- Replace a paragraph or run when selected by stable ID.
- Replace existing visible chart and SmartArt text only through selectors that
  preserve the required chart XML/workbook or diagram data/cache consistency.
- Add a simple text box.

Formatting requirements:

- Preserve existing run formatting when replacing text in-place.
- For whole-box replacement, reuse the first run style when practical.
- If style cannot be preserved, document the deterministic default style.
- Chart data/value authoring and SmartArt node/layout/color authoring are not
  text edits; unsupported authoring requests must fail with `unsupported_edit`.

## Shape Reading

Expose basic shape metadata:

- ID and name.
- Kind.
- Bounds: `x`, `y`, `cx`, `cy` in EMUs.
- Rotation and flip where present.
- Alt text/title when present.
- Placeholder role when detectable.

## Shape Editing

V1 shape edits:

- Add text box.
- Move/resize supported shape types.
- Set alt text/title.

Requirements:

- Drawing non-visual IDs must be unique within each slide.
- New elements must be inserted into a valid `p:spTree`.
- Unsupported shape types must be preserved unchanged.
