# 050. Round-Trip Invariants

## No-Edit Round Trip

For `read(input).write(output)` with no edits:

- Output must be a valid ZIP/PPTX package.
- All original parts must be present.
- No unknown parts may be dropped.
- Unmodified binary parts must be byte-identical.
- Unmodified XML parts must be byte-identical in preserve mode.
- Relationship graph must remain valid.

## Package Completeness

- Every part must have a content type.
- Every internal relationship target must exist.
- External relationships must remain marked external.
- Relationship IDs must be unique within each relationship file.

## Slide Integrity

- Slide order must match `ppt/presentation.xml` unless explicitly changed.
- Slide IDs must be unique.
- Drawing non-visual IDs must be unique within each slide.
- Shape tree order must be preserved except where an edit intentionally inserts/moves elements.

## Media Integrity

- Added media parts must have correct content types.
- Slide XML must reference the correct relationship ID.
- Existing media bytes must remain unchanged unless explicitly replaced.

## XML Integrity

- Written XML must be well-formed.
- Required namespaces must be declared.
- Text must be escaped correctly.
- Unknown XML must be preserved unless explicitly removed by an operation.

## Agent Patch Integrity

- Exported IDs must remain stable for the exported `document_id` and revision.
- Failed patches must not partially corrupt the package.
- Patch results must include changed parts and warnings.
- Validation errors must identify the affected slide, part, relationship, or element.

## Validation-on-Write

For edited or dirty documents, `write_*` must run package validation by default and fail on validation errors. Callers may opt out only through an explicit unsafe/debug option that is not exposed to default agent workflows.

For no-edit round trips, existing validation warnings may be reported without blocking writes unless the package is structurally unreadable or unsafe.
