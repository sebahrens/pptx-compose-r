---
name: pptx-compose
description: Use when inspecting, editing, validating, or exporting PPTX files with this repository's pptx-compose CLI or MCP server. Guides agents through the bounded inspect -> find-text -> patch -> dry-run -> apply -> validate workflow.
---

# pptx-compose

Use pptx-compose as a bounded PPTX editing protocol. Do not mutate package internals, raw XML, ZIP entries, relationships, or content types directly. Prefer a supported patch operation; if the requested edit is not supported, report `unsupported_edit` instead of producing a corrupt deck.

## CLI Workflow

Start by confirming the built surface and the supported operations:

```bash
pptx-compose --version
pptx-compose capabilities | jq -r '.supported_operations[].op'
```

Do not hard-code supported operation names from memory. Read `capabilities.supported_operations` from the current binary, then build patches only with operations it advertises.

Inspect a scoped view first:

```bash
pptx-compose inspect IN.pptx --slides 1 --output deck.view.json --report inspect.report.json --json-errors
```

Use `find-text` to get ready-to-paste guarded selectors for text edits:

```bash
pptx-compose find-text IN.pptx "QUERY" --slides slide-1 --limit 25 --output matches.json --json-errors
```

Each `matches[].selector` is the selector to paste into a patch operation. It has this shape:

```json
{
  "type": "element_id",
  "id": "slide-1:shape-1",
  "guards": {
    "slide_id": "slide-1",
    "kind": "shape",
    "part": "ppt/slides/slide1.xml",
    "text_hash": "sha256:...",
    "fingerprint": "sha256:..."
  }
}
```

Drop that selector into a `pptx-compose.patch.v1` envelope. A complete
single-edit patch (a run-scoped text replacement) looks like this:

```json
{
  "schema": "pptx-compose.patch.v1",
  "version": 1,
  "document_id": "sha256:7252a8...",
  "base_revision": 1,
  "client_request_id": "your-request-id",
  "operations": [
    {
      "op": "replace_text",
      "operation_id": "op-1",
      "mode": "run_scoped",
      "selector": {
        "type": "element_id",
        "id": "slide-1:shape-1",
        "guards": {
          "slide_id": "slide-1",
          "kind": "shape",
          "part": "ppt/slides/slide1.xml",
          "text_hash": "sha256:...",
          "fingerprint": "sha256:..."
        },
        "run": { "paragraph_index": 0, "run_index": 0 }
      },
      "text": "replacement text"
    }
  ]
}
```

`document_id` and `base_revision` are revision guards: copy `document_id` and
`revision` from the `inspect`/`find-text` output verbatim (a stale
`base_revision` is rejected with `stale_patch`). `client_request_id` and each
`operation_id` are caller-chosen idempotency keys. For `run_scoped` edits, add
`selector.run` with the `paragraph_index`/`run_index` you read from `inspect
--detail full`. The guards object is exactly the `find-text` selector's
`guards` — paste it unchanged.

Validate the patch without writing an output deck:

```bash
pptx-compose apply IN.pptx PATCH.json --dry-run --report dry-run.report.json --diff diff.json --json-errors
```

Apply only after the dry run succeeds:

```bash
pptx-compose apply IN.pptx PATCH.json --output OUT.pptx --report apply.report.json --json-errors
pptx-compose validate OUT.pptx --report validation.json --json-errors
```

For intentional input replacement, use in-place mode. It writes back to `INPUT.pptx` atomically and creates `INPUT.pptx.bak` unless `--no-backup` is supplied:

```bash
pptx-compose apply IN.pptx PATCH.json --in-place --report apply.report.json --json-errors
```

## Slide Scopes

`inspect --slides` accepts 1-based numbers, canonical slide ids, comma lists, and numeric ranges:

```bash
--slides 1
--slides slide-1
--slides 1,3,5
--slides 2-4
```

`find-text --slides` is currently single-slide scoped. Use a 1-based number or canonical `slide-N` id; omit the flag for a deck-wide search. A comma list or range is rejected with `invalid_input: find-text --slides currently accepts exactly one slide`. Like `inspect`, `find-text` paginates large result sets: if the output carries a non-null `next_cursor`, pass it back via `--cursor` until it is null.

## Patch Inputs

Use schemas from the current binary when constructing or validating JSON:

```bash
pptx-compose schema patch-v1
pptx-compose schema media-manifest-v1
pptx-compose schema find-text-v1
pptx-compose schema capabilities-v1
```

For image operations, bind media through the CLI or a manifest:

```bash
pptx-compose apply IN.pptx PATCH.json --media hero=assets/hero.png --dry-run --report r.json --diff d.json --json-errors
pptx-compose apply IN.pptx PATCH.json --media-manifest media.json --media-root assets --output OUT.pptx --json-errors
```

Never add an image by only adding a file to the ZIP. A valid image edit must update `[Content_Types].xml`, relationships, and slide XML together through a supported operation.

To inspect what media a deck already contains (the read side, distinct from the
`--media` staging flags above), use the `media` subcommands:

```bash
pptx-compose media list IN.pptx --json-errors
pptx-compose media get IN.pptx ppt/media/image1.png --output extracted.png --json-errors
```

`media list` reports the media parts as JSON on stdout; `media get` extracts one
part (by its package path) to `--output`, which is bound by the same workspace
rules as other output paths.

## Error Handling

For automation, pass `--json-errors`. Failed commands then emit one JSON error envelope to stderr. Branch on `error.code`, not the process exit code, because exits are coarse buckets and some codes, such as `invalid_input`, can appear under multiple exits.

Primary machine JSON goes to stdout or an explicit output path. Human logs belong on stderr; do not parse prose.

## MCP Workflow

Use the MCP server for session-based agent workflows:

```text
pptx_open
-> pptx_get_document_summary / pptx_list_slides / pptx_get_slide
-> pptx_list_elements / pptx_get_element / pptx_find_text
-> pptx_validate_patch
-> pptx_apply_patch
-> pptx_export
-> pptx_close
```

Stage media with `pptx_import_media`; use the returned session-scoped `media_ref` in patches. Mutating tools require `session_id` plus `expected_revision` or a patch `base_revision`; stale revisions return `stale_patch`.

Raw package/XML mutation is outside the V1 agent surface. Use bounded inspection tools, guarded selectors, patch validation, apply, export, and validate.

## Field Notes (verified 2026-06-11 against the built CLI and specs)

These are non-obvious behaviors that bit real agents. Trust the built binary's
`--help`/`schema` over older examples.

### Inspect / output paths
- `inspect` has **no `--format` flag**. The agent view (`schema:
  pptx-compose.agent_view.v1`) is the only output. Passing `--format agent-json`
  errors with `invalid_input: unexpected argument '--format'`.
- **All output paths** (`--output`, `--report`, `--diff`) must be inside the
  workspace or `--temp-dir`. `/tmp` (resolves outside) and `/dev/stdout` are
  rejected with `permission_denied`. Write to a dir under the repo, or pass
  `--workspace`/`--temp-dir`. Re-running needs `--overwrite`; `inspect`,
  `find-text`, `apply`, and `validate` all expose it for existing output paths.
- **The input deck is bound to the same workspace root**, which defaults to the
  current directory. Pointing `--workspace` at a scratch dir that does not
  contain the input fails with `invalid_input: Could not resolve readable input
  PPTX path` even though the file plainly exists — the message reads like a
  missing file but the real cause is the workspace boundary. Keep `--workspace`
  at a directory that contains *both* the input deck and your output paths.
- `inspect --limit` is a **slides-per-page** budget, range 1..100 (not element
  count). Big decks paginate: read `view.next_cursor` and pass it back via
  `--cursor` until it is null. `--detail full` gives per-run text.

### Selectors and guards (important for batch edits)
- Use the guarded selectors emitted by `inspect --detail full` and `find-text`.
  `guards.text_hash` protects the target's normalized text projection, while
  `guards.fingerprint` protects structural identity. Fingerprints are not salted
  by sibling text changes, and should not be stripped for normal batch edits.
- For run-scoped translation, keep the run selector plus the relevant run or
  element text guard. Bare `element_id` run selectors are not enough evidence
  for real-world translation fixtures.

### replace_text modes — choose by fidelity need
- **whole_element** (default): replaces the element's entire text. Newlines in
  `text` map to **new paragraphs** (warning `newline_mapping: paragraph`). It
  **refuses** if the edit would simplify rich text unless you set
  `allow_formatting_simplification: true`. `format_policy: preserve_first_run`
  then collapses every run to the first run's style — so **per-run bold/color is
  lost** and the whole box takes the first run's formatting. Paragraph-level
  bullet/auto-numbering is also not reliably preserved. Use only for plain,
  single-style text.
- **run_scoped**: edits one run in place. Requires a run selector — either
  `selector.run: {paragraph_index, run_index}` (preferred) or the op-level
  `run` field. The `text` **must not contain `\n` or `\r`** (rejected as
  `invalid_input`). For an in-run line break use a **vertical tab**
  (accepted, renders as a soft break). run_scoped **preserves** paragraph
  numbering/bullets (`buAutoNum`) and each run's own style. **This is the
  faithful mode for translation / text rewrites** of richly formatted decks:
  inspect with `--detail full`, then emit one run_scoped op per non-empty run.

### Chart and SmartArt text boundaries
- **Charts** (`kind: chart`) and **SmartArt diagrams** (`kind: diagram`) can
  expose V1-supported visible text selectors when the engine can keep companion
  state consistent. Use the selectors and guards from `inspect` or `find-text`;
  chart/diagram text replacement requires `mode: run_scoped`.
- Chart cache or workbook-backed labels are editable only when the required chart
  XML cache and workbook label state can be updated together. SmartArt text is
  editable only when diagram data and rendered drawing-cache text can be mapped
  safely. Ambiguous mappings, chart data/value authoring, workbook authoring,
  SmartArt structure/layout/cache authoring, and node add/remove/reorder edits
  must fail with `unsupported_edit`.
- **Tables** (`kind: table`) and **groups** (`kind: group`) report
  `editable.text.supported` falsey at the element level. Edit **table cells**
  via a `cell: {row, col}` (or run selector) on the table element; group
  **children are listed as separate top-level editable shapes**, so editing all
  `editable.text.supported == true` shapes/text_boxes already covers group text.
- Filter targets by `editable.text.supported == true` rather than merely "has a
  `text` field" — shapes/diagrams/tables can carry text that is not editable.
