# 001. Goals and Scope

## Goal

Build a cleanroom Rust implementation that can read `.pptx` files, expose their content to AI agents, apply safe modifications or additions, and write valid `.pptx` files.

The rewrite should graduate from the current raw ZIP/XML JSON concept to a layered PPTX system:

1. Lossless OPC package handling.
2. Namespace-aware XML handling.
3. PPTX-aware presentation, slide, text, shape, and media models.
4. Agent-friendly JSON projection.
5. Explicit validated edit operations.

## Primary Users

- Rust applications that need PPTX inspection and mutation.
- Node/JavaScript users migrating from the existing package.
- AI agents that need a stable read/modify/write protocol.
- CLI users converting, inspecting, validating, or patching decks.

## V1 Supported Operations

V1 should support a narrow but reliable set of edits:

- Open a PPTX from a path, bytes, or reader.
- List slides in presentation order.
- Extract slide text, shape names, basic bounds, and image references.
- Replace existing text while preserving formatting where possible.
- Add a simple text box to a slide.
- Move or resize supported text, shape, and image elements.
- Set element title/alt text metadata.
- Read image placements and media part references.
- Add an image to a slide.
- Replace an existing image while preserving the surrounding picture element.
- Validate the package before writing.
- Write a `.pptx` while preserving unsupported content.

Slide lifecycle operations are not V1 requirements. Adding, duplicating, deleting, and reordering slides are post-V1 operations unless a later implementation plan explicitly expands scope and adds the corresponding dependency-copying and cleanup rules.

## Preserve-Only Areas in V1

These must be preserved, but do not need full semantic editing in V1:

- Charts and embedded workbooks.
- Tables beyond basic detection.
- SmartArt.
- Animations and transitions.
- Comments.
- Speaker notes, unless simple note text support is explicitly added.
- Slide masters, layouts, and themes.
- Custom XML parts.
- Unknown OOXML extension elements.
- Embedded OLE objects and files.

## Non-Goals

- A full PowerPoint renderer.
- Pixel-perfect layout computation.
- Complete ECMA-376 object model coverage.
- Full chart/table/SmartArt editing in V1.
- Slide add/duplicate/delete/reorder in V1.
- Generating arbitrary presentations from high-level prose in V1.
- Byte-for-byte output equality for the entire ZIP when edits are made.
- Preserving historical TypeScript bugs unless a compatibility spec explicitly requires them.

## Success Criteria

- No-edit round trips preserve all parts and copy unmodified part bytes wherever possible.
- Common text/image edits produce PPTX files that open in PowerPoint-compatible tools.
- Added content updates every required OPC dependency: content type, relationship, and XML reference.
- Agent patches are explicit, validated, and fail atomically.
- Unknown deck content survives edits.
