# 080. Testing and Fixtures

The Rust rewrite is now tested through manifest-backed fixtures, crate-level
unit/integration tests, CLI/MCP eval corpora, and Ralph E2E harnesses. Legacy
TypeScript compatibility tests are no longer a planning target; the Rust
agent-facing surface is the source of truth.

## Fixture Layout

```text
fixtures/
  manifest.toml
  legacy/
  minimal.pptx
  minimal/
  powerpoint/
  libreoffice/
  google-slides/
  media/
  charts/
  embedded/
  malformed/
  construction/
  real-world/
```

`fixtures/manifest.toml` is the registry for fixture-driven tests. Each entry
records the fixture path, source application, notable features, expected warning
codes, required invariants, and intended consuming tests. Tests that iterate the
corpus must read the manifest instead of globbing arbitrary `.pptx` files.

The real-world corpus contains public consulting/economic decks plus localized
derived variants. Those localized variants are intentionally useful for text
rewrite and translation-fidelity stress tests, but manifest entries must not
reference files that are absent from a clean checkout.

## Package and Round-Trip Tests

- Open every non-malformed manifest fixture.
- Enumerate all parts and parse `[Content_Types].xml` plus package/part
  relationships.
- Resolve relative relationship targets and reject unsafe ZIP paths.
- No-edit `parse -> write -> parse` succeeds.
- Output contains every original part.
- Unchanged binary media and clean XML parts are byte-identical in preserve mode.
- Package validation passes after write, with only manifest-declared warnings.

## Edit Tests

- Replace whole-element text, including explicit `formatting_simplified`
  handling for rich text.
- Replace run-scoped text without dropping sibling runs, bullets, hyperlinks,
  fields, or paragraph structure.
- Replace table-cell and supported notes text through `replace_text`.
- Add text box.
- Move/resize text boxes, shapes, pictures, and supported graphic frames.
- Set alt text on any element with a `p:cNvPr`.
- Set core document metadata fields supported by `set_document_metadata`.
- Add image and assert media part, content type, relationship, and slide XML
  reference.
- Replace image and preserve picture element metadata.

## Deferred Edit Tests

When slide lifecycle operations are added, test:

- Add slide with valid layout relationship.
- Duplicate slide while preserving dependencies.
- Delete slide without leaving required dangling references.
- Move slide while preserving slide IDs and order.

When template-population support lands, add fit/alignment tests described in
[082. Template population and text fitting](082-template-population-and-text-fitting.md).

## Validation Tests

- Detect missing content type.
- Detect missing internal relationship target.
- Detect duplicate relationship IDs.
- Detect duplicate slide IDs.
- Detect duplicate drawing IDs in every slide part, not only dirty parts.
- Preserve unsupported parts and warn on unchecked external relationships.

## Ralph E2E Harnesses

`ralph-scripts/pptx_roundtrip_e2e.py` runs the bounded no-edit agent flow for
every non-malformed manifest fixture:

```text
inspect -> empty-patch apply --dry-run -> empty-patch apply --output -> validate
```

It compares XML/package/media bytes, optionally performs visual comparison when
LibreOffice + `pdftoppm` + Pillow are available, writes
`.ralph-roundtrip-e2e/roundtrip-summary.json`, and files deduplicated
`defect:roundtrip-e2e` Beads when defects are detected.

`ralph-scripts/pptx_edit_e2e.py` exercises representative V1 edit operations on
real fixtures. It currently verifies targeted changes, unrelated-part
preservation, validation, and render sanity; defect filing and negative error-code
assertions are tracked separately in `pptx-compose-roxl`.

`ralph-scripts/translation_fidelity.py` analyzes localized fixture outputs for
remaining untranslated text classes such as chart/SmartArt/table content and
line-break fidelity. It should be used when regenerating localized fixtures and
when validating the V1 requirement that existing visible chart/SmartArt text is
inspectable and editable while chart data authoring and SmartArt structure
authoring remain unsupported.

Generated Ralph directories (`.ralph-roundtrip-e2e/`, `.ralph-edit-e2e/`) are
scratch outputs and should remain ignored.

## Agent Protocol Tests

- Agent JSON validates against published schemas.
- Patch JSON validates against published schemas.
- Unknown operation fields are rejected by default.
- Stale `base_revision` and wrong `document_id` are rejected.
- Selector unresolved, ambiguous, and guard-failed cases return expected error
  codes.
- `find-text` cursors are rejected when reused with a different query or scope.
- Dry-run returns the same validation/diff shape as apply without mutating the
  document.
- Failed multi-operation patches leave package state unchanged.
- `replace_image` on shared media retargets only one picture.
- Missing `media_ref`, content-type mismatch, and checksum mismatch fail before
  mutation.

## CLI and MCP Contract Tests

- `inspect --output -` emits valid `AgentView` JSON and no non-JSON stdout.
- `find-text --output -` emits valid `FindText` JSON and no non-JSON stdout.
- `validate --report -` emits valid `ValidationReport` JSON.
- `apply --dry-run` emits valid `PatchReport` and creates no PPTX output.
- `apply --dry-run --diff -` is rejected unless `--report` writes to a file; no
  command may emit two machine JSON documents to stdout.
- `apply` writes output atomically and emits valid `PatchReport`.
- `--json-errors` emits exactly one valid error envelope for each failure class.
- MCP tools return structured content matching their schemas and advertised
  bounds, including the shared page limit.
- MCP eval fixture path substitutions must point to files that exist in a clean
  checkout.
- MCP sessions reject stale revisions and clean up uploaded media on close/expiry.
- Permission-denied path reads/writes fail with structured errors.

Every documented CLI example and MCP eval transcript should be executable as a
fixture-backed test.
