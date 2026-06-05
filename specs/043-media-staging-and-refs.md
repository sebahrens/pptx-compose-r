# 043. Media Staging and References

Patch operations use `media_ref` keys rather than embedding bytes directly. This spec defines how those keys resolve in Rust API, CLI, and MCP workflows.

## Media Ref Rules

- `media_ref` values are scoped to one apply/dry-run request or one MCP session.
- Missing media refs fail before any package mutation.
- Extra media refs should produce a warning by default or fail in strict mode.
- Duplicate media refs are invalid.
- Declared content types must be checked against magic bytes where feasible.
- Checksum mismatch is a hard error.
- Inline base64 is opt-in and size-limited.

## CLI Media Manifest

CLI patch workflows may bind media through a manifest:

```json
{
  "schema": "pptx-compose.media_manifest.v1",
  "version": 1,
  "media": {
    "input-image-1": {
      "path": "assets/chart.png",
      "content_type": "image/png",
      "sha256": "sha256:...",
      "byte_length": 12345
    }
  }
}
```

Rules:

- Relative paths resolve relative to the manifest file unless `--media-root` is supplied.
- Paths must not escape `--media-root` or the configured workspace.
- Absolute media paths are rejected by default unless an explicit unsafe/debug option is used.
- `media_ref` in patches must match manifest keys or explicit CLI `--media key=path` bindings.

## CLI Inline Media

Inline media is optional and should be avoided for large assets:

```json
{
  "media": {
    "input-image-1": {
      "inline": {
        "encoding": "base64",
        "content_type": "image/png",
        "data": "..."
      },
      "sha256": "sha256:..."
    }
  }
}
```

## MCP Media Handles

MCP workflows should import media before patch application. The import tool returns a session-scoped handle:

```json
{
  "media_ref": "media_upload_1",
  "content_type": "image/png",
  "byte_length": 12345,
  "sha256": "sha256:...",
  "dimensions_px": { "width": 1024, "height": 768 }
}
```

Media handles expire when the session closes or reaches its TTL.

## Rust API Media Inputs

Rust APIs should expose a `MediaInputs` type that can bind refs to bytes, paths, or readers. The API must validate content type, checksum, and size before mutating the deck.

## Deterministic Media Part Naming

New media part names must be deterministic and collision-safe. A recommended policy is `ppt/media/image{next_available}.{ext}` with collision checks against existing parts and previously generated additions.
