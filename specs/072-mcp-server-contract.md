# 072. MCP Server Contract

The MCP server is a first-class public agent interface, not a thin wrapper around CLI strings. It must expose bounded, structured, validated operations that preserve the crate's safety guarantees.

## Design Goals

- Agents can inspect, patch, validate, and export decks without raw XML mutation.
- Tool outputs are structured, bounded, paginated, and schema-versioned.
- Mutating tools enforce revisions and validation.
- Media bytes are handled through scoped handles, not unbounded JSON blobs by default.
- Raw XML tools are advanced/debug only and disabled by default.

## Tools

Minimum tool set:

| Tool | Purpose | Read-only | Destructive | Idempotent |
| --- | --- | --- | --- | --- |
| `pptx_open` | Open a deck into a session | yes | no | no |
| `pptx_get_document_summary` | Return deck summary and capabilities | yes | no | yes |
| `pptx_list_slides` | Page through slide summaries | yes | no | yes |
| `pptx_get_slide` | Return one slide view | yes | no | yes |
| `pptx_list_elements` | Page through slide elements | yes | no | yes |
| `pptx_get_element` | Return one element detail | yes | no | yes |
| `pptx_find_text` | Search text in scoped slides | yes | no | yes |
| `pptx_import_media` | Stage media bytes/path as `media_ref` | no | session only | no |
| `pptx_validate_patch` | Dry-run patch | yes | no | yes |
| `pptx_apply_patch` | Apply atomic patch to session | no | session | no |
| `pptx_validate` | Validate current session | yes | no | yes |
| `pptx_export` | Write or return PPTX output | no | filesystem/resource | no |
| `pptx_close` | Release session resources | no | session | yes |

Each tool must define input schema, structured output schema, annotations, max output size, pagination behavior, and error envelope behavior.

For V1, `pptx_list_elements` is slide-scoped: its input schema requires
`slide_id`, and `cursor`/`limit` paginate that slide's element collection in a
`slide_detail` view. Deck-wide element listing requires a separate explicit
collection mode with its own stable ordering and cursor scope.

## Sessions and Revisions

`pptx_open` returns:

```json
{
  "session_id": "sess_abc",
  "document_id": "sha256:...",
  "revision": 1,
  "slide_count": 12,
  "expires_at": "2026-06-05T00:00:00Z"
}
```

Rules:

- Every mutating tool requires `session_id` and `expected_revision` or a patch `base_revision`.
- `base_revision` is the revision field inside a patch document; `expected_revision` is the equivalent guard passed as a tool/CLI parameter for non-patch operations. They carry the same value and meaning.
- Successful non-dry-run patch application increments revision.
- Stale revisions return `stale_patch` ([044](044-results-validation-errors.md#error-envelope) canonical code); no silent rebase.
- Concurrent applies to one session must be serialized.
- Sessions have TTL, memory limits, and cleanup behavior.
- Separate sessions from the same source do not share mutable state unless explicitly documented.

## Media Handles

`pptx_import_media` returns a session-scoped `media_ref`:

```json
{
  "media_ref": "media_1",
  "content_type": "image/png",
  "sha256": "sha256:...",
  "byte_length": 12345,
  "dimensions_px": { "width": 1024, "height": 768 }
}
```

The server must enforce media size limits and content-type validation. Media handles expire with the session.

## Resources

Recommended resource URI patterns:

```text
pptx://sessions/{session_id}/summary
pptx://sessions/{session_id}/slides
pptx://sessions/{session_id}/slides/{slide_id}
pptx://sessions/{session_id}/elements/{element_id}
pptx://sessions/{session_id}/media/{media_id}/metadata
pptx://sessions/{session_id}/validation/latest
pptx://schemas/agent-view/v1
pptx://schemas/patch/v1
pptx://schemas/patch-report/v1
pptx://schemas/error/v1
```

Resources are read-only. Large resources must be paginated or scoped. Binary resources are not embedded in JSON unless explicitly requested and size-limited.

## Prompts

Recommended MCP prompts:

- `inspect_deck`: guide agents to open and inspect scoped views.
- `edit_deck_safely`: guide inspect → patch → dry-run → apply → export.
- `replace_text_across_deck`: produce guarded text replacement patches.
- `add_image_to_slide`: stage media and produce add-image patches.
- `explain_validation_errors`: turn validation reports into next safe actions.

Prompts should teach agents to use scoped tools first, validate before apply, refuse unsupported raw package/XML mutation, and report warnings.

## Raw XML Scope

Raw package/XML inspection and mutation tools are out of scope for V1. Agents must use the bounded inspection, validation, patch, and export tools for supported edits.
