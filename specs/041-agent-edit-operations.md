# 041. Agent Edit Operations

Agents should modify decks through explicit operations, not arbitrary raw XML or raw JSON mutation.

## Patch Envelope

```json
{
  "schema": "pptx-compose.patch.v1",
  "version": 1,
  "document_id": "sha256:example-package-fingerprint",
  "base_revision": 1,
  "client_request_id": "agent-run-001",
  "operations": [
    {
      "operation_id": "op-1",
      "op": "replace_text",
      "element_id": "slide-1:shape-4",
      "text": "Updated title"
    }
  ]
}
```

## V1 Operations

- `replace_text`
- `add_text_box`
- `move_resize_element`
- `set_alt_text`
- `add_image`
- `replace_image`

## Operation Requirements

- Selectors must resolve uniquely unless multi-edit is explicit.
- Agent IDs are valid only for the exported `document_id` and `base_revision`; stale patches must be rejected unless selectors can be safely revalidated.
- Every operation must include `operation_id` so dry-run, apply, validation, and error reports can refer to it.
- Unknown operation fields must be rejected by default. Future compatibility modes may allow extensions only when explicitly requested.
- Every operation must have a dry-run validation path.
- Patch application must be atomic: either all operations apply or none are written.
- Every operation must produce an edit report.
- Unsupported operations must return structured errors.

## Selectors and Guards

Bare `slide_id` and `element_id` fields are allowed shorthand. Canonical patches should use selector objects when agents need stale-target protection:

```json
{
  "selector": {
    "type": "element_id",
    "id": "slide-1:shape-4",
    "guards": {
      "slide_id": "slide-1",
      "kind": "text_box",
      "part": "ppt/slides/slide1.xml",
      "text_hash": "sha256:example-text-hash",
      "fingerprint": "sha256:example-element-fingerprint"
    }
  }
}
```

Selectors must resolve to exactly one target unless the operation explicitly supports multiple targets. Fuzzy matching is forbidden by default. Guard failures return structured stale/guard error codes rather than applying best-effort edits.

Element `guards.kind` uses the same snake_case vocabulary emitted by
`Element View.kind` in the agent view schema: `text_box`, `image`, `shape`,
`group`, `table`, and `chart`. Agents may copy an element view's `kind` value
directly into selector guards. Implementations may accept legacy internal guard
aliases for compatibility, but emitted views and canonical patches should use
the agent-view vocabulary.

## V1 Operation Schemas

### `replace_text`

Required fields:

- `element_id`: target text-capable element.
- `text`: replacement text.

Optional fields:

- `match`: exact current text guard. If present and the current text differs, validation fails.
- `mode`: `whole_element` by default. V1 exposes structured paragraph/run text
  for context only; it must not emit paragraph/run IDs because no paragraph/run
  replacement mode exists yet.
- `format_policy`: `preserve_existing_runs`, `preserve_first_run`, or `single_run_default_style`.
- `overflow_policy`: `allow` by default. V1 does not guarantee rendered text fit unless an external renderer is configured.

Newlines in replacement text must have a documented mapping. V1 should map `\n` to PowerPoint line breaks or paragraphs consistently and report the chosen mapping in the patch report. Hyperlinks, fields, and mixed-format runs must be preserved when possible or return `unsupported_edit`/`formatting_simplified` warnings.

### `add_text_box`

Required fields:

- `slide_id`
- `text`
- `bounds`: `{ "x": emu, "y": emu, "cx": emu, "cy": emu }`

Optional fields:

- `name`
- `alt_text`
- `style`: implementation-defined basic font/fill options.
- `insert`: `{ "z_order": "front" }` by default.

Supported V1 style fields must be explicit in implementation docs. Unknown style fields fail validation rather than being silently ignored.

### `move_resize_element`

Required fields:

- `element_id`
- `bounds` in EMUs.

### `set_alt_text`

Required fields:

- `element_id`

Optional fields:

- `title`
- `description`
- `alt_text` as a shorthand mapped to the appropriate OOXML non-visual property.

### `add_image`

Required fields:

- `slide_id`
- `media_ref`: key for bytes supplied alongside the patch.
- `content_type`: e.g. `image/png` or `image/jpeg`.
- `bounds` in EMUs.

Optional fields:

- `name`
- `alt_text`
- `fit`: `stretch` by default. `contain`, `cover`, and `original_size` are optional and must be rejected when unsupported.
- `dedupe`: `never` by default. Checksum-based reuse is opt-in.

### `replace_image`

Required fields:

- `element_id`: target picture element.
- `media_ref`: key for replacement bytes.
- `content_type`

The implementation should keep the existing picture element and relationship ID when possible; if retargeting is required, it must update content types and relationships atomically.

Default `replace_image` behavior is `retarget_picture`: create or reuse a media part and update only the selected picture relationship. Mutating the existing media part is allowed only when the media part is unshared or when `allow_shared_mutation: true` is explicit.

See [agent protocol schemas](042-agent-protocol-schemas.md) and [media staging and references](043-media-staging-and-refs.md) for normative schema details.

## Post-V1 Operations

These operations are explicitly post-V1 unless a later implementation plan expands scope:

- `add_slide`
- `duplicate_slide`
- `delete_slide`
- `move_slide`

## Example Image Operation

```json
{
  "op": "add_image",
  "slide_id": "slide-1",
  "media_ref": "input-image-1",
  "content_type": "image/png",
  "bounds": { "x": 914400, "y": 1828800, "cx": 3657600, "cy": 2743200 },
  "alt_text": "Revenue chart screenshot"
}
```

The implementation resolves `media_ref` through API-provided bytes, then updates media parts, content types, relationships, and slide XML.
