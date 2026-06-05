# 032. Media and Images

## Image Reading

The parser should identify slide pictures and expose:

- Agent element ID.
- Slide ID.
- Picture name/title/alt text.
- Bounds and transform.
- Relationship ID.
- Resolved media part path.
- Content type.
- Byte length and optional checksum.

Image references usually flow through a slide relationship from a picture element's `r:embed` value to a `/ppt/media/...` part.

## Add Image Operation

Adding an image requires:

- Accept bytes and content type, or infer content type from trusted input.
- Verify declared content type against magic bytes where feasible.
- Choose a deterministic media part name.
- Add or reuse a content-type default/override.
- Add a slide relationship to the media part.
- Insert a valid picture element into the slide shape tree.
- Allocate unique relationship and drawing IDs.
- Validate the package graph.

## Replace Image Operation

Replacing an image should prefer preserving the existing picture XML:

- Keep the same picture element.
- Keep the same relationship ID when possible.
- Retarget only the selected picture to a new media part by default.
- Replace the target media part bytes only when it is not shared, or when the caller explicitly opts into shared-media mutation.
- Update content types if the extension/content type changes.
- Preserve crop, transform, and picture effects by leaving existing picture XML intact whenever possible.

The agent view must expose `shared_media_ref_count` for image elements. If an image points at externally linked media (`r:link` rather than `r:embed`), V1 replacement should return `unsupported_edit` unless a later spec explicitly supports external image retargeting.

## Supported V1 Input Types

V1 image insertion supports PNG and JPEG. GIF may be supported if bytes can be preserved and content types are handled. SVG, TIFF, EMF, WMF, HEIC, remote URLs, and externally linked images are preserve-only unless a later phase expands support.

See [media staging and references](043-media-staging-and-refs.md) for how CLI, MCP, and Rust APIs bind `media_ref` values to bytes.

## Deduplication

Deduplication by checksum may be offered as an option. It must never silently merge separate media parts by default because Office-generated decks may intentionally contain duplicate files.
