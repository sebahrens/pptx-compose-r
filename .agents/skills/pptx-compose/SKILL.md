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

`find-text --slides` is currently single-slide scoped. Use a 1-based number or canonical `slide-N` id; omit the flag for a deck-wide search.

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
