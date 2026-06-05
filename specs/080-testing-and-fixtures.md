# 080. Testing and Fixtures

The current tests are smoke tests. The Rust rewrite needs structural, invariant, compatibility, and real-world tests.

## Fixture Layout

```text
fixtures/
  legacy/
  minimal/
  powerpoint/
  libreoffice/
  google-slides/
  media/
  charts/
  embedded/
  malformed/
```

The existing `sample.pptx`, `sample.zip`, and `sample.jpg` should be migrated into `fixtures/legacy/`.

## Package Tests

- Opens sample PPTX.
- Enumerates all parts.
- Parses `[Content_Types].xml`.
- Parses root and part relationships.
- Resolves relative relationship targets.
- Rejects unsafe ZIP paths.

## Round-Trip Tests

- No-edit `parse -> write -> parse` succeeds.
- Output contains every original part.
- Unchanged binary media is byte-identical.
- Unchanged XML is byte-identical in preserve mode.
- Package validation passes after write.

## Edit Tests

- Replace title text.
- Replace body text.
- Add text box.
- Move/resize element.
- Add image and assert media part, content type, relationship, and slide XML reference.
- Replace image and preserve picture element metadata.

## Post-V1 Edit Tests

When slide lifecycle operations are added, test:

- Add slide with valid layout relationship.
- Duplicate slide while preserving dependencies.
- Delete slide without leaving required dangling references.
- Move slide while preserving slide IDs and order.

## Validation Tests

- Detect missing content type.
- Detect missing internal relationship target.
- Detect duplicate relationship IDs.
- Detect duplicate slide IDs.
- Detect duplicate drawing IDs when feasible.

## Compatibility Tests

- Existing sample PPTX exports expected key set in legacy JSON mode.
- Legacy JSON can be written back to PPTX.
- Existing media insertion smoke behavior still works in compatibility mode.
- CLI compatibility alias behaves predictably.

## Fixture Manifest

Each fixture corpus entry should have a manifest row describing source application, notable features, expected parse warnings, and required invariants. Corpus creation is an implementation task; the current repository only supplies legacy smoke fixtures under `src/__tests__/fixtures/`.

## External Smoke Tests

When available in CI, use LibreOffice headless or another OpenXML-compatible validator to confirm edited outputs open/convert.

## Agent Protocol Tests

- Agent JSON validates against published schemas.
- Patch JSON validates against published schemas.
- Unknown operation fields are rejected by default.
- Stale `base_revision` is rejected.
- Wrong `document_id` is rejected.
- Selector unresolved, ambiguous, and guard-failed cases return expected error codes.
- Dry-run returns the same validation/diff shape as apply without mutating the document.
- Failed multi-operation patches leave package state unchanged.
- `replace_image` on shared media defaults to retargeting only one picture.
- Missing `media_ref`, content-type mismatch, and checksum mismatch fail before mutation.

## CLI and MCP Contract Tests

- `inspect --output -` emits valid `AgentView` JSON and no non-JSON stdout.
- `validate --report -` emits valid `ValidationReport` JSON.
- `apply --dry-run` emits valid `PatchReport` and creates no PPTX output.
- `apply` writes output atomically and emits valid `PatchReport`.
- `--json-errors` emits exactly one valid error envelope for each failure class.
- MCP tools return structured content matching their schemas.
- MCP sessions reject stale revisions and clean up uploaded media on close/expiry.
- Permission-denied path reads/writes fail with structured errors.

Every documented CLI example and MCP eval transcript should be executable as a fixture-backed test.
