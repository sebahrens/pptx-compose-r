# 020. XML Handling

XML handling is the highest fidelity risk in the rewrite. Generic XML-to-JSON-to-XML round-tripping is not sufficient for reliable PPTX editing.

## Core Rule

Unmodified XML parts must be written from their original raw bytes by default.

Only XML parts marked dirty by an edit should be serialized.

In preserve mode, unmodified XML parts must be emitted byte-identically from their original raw bytes. If preserve mode and deterministic mode conflict, preserve mode wins for unmodified parts; deterministic mode applies to new or modified parts and to ZIP metadata/order where configured.

Preserve vs deterministic is selected by the explicit `WriteMode` option (default `Preserve`) defined in [public API and CLI](070-public-api-and-cli.md); it is not implicit. Deterministic mode still copies clean-part payloads from `raw`, so it never changes a clean part's `part_checksum` (046) — it only normalizes ZIP framing/order and the serialization of dirty parts.

## XML Part Model

```rust
struct XmlPart {
    raw: Vec<u8>,
    parsed: Option<XmlDocument>,
    dirty: bool,
    diagnostics: Vec<XmlDiagnostic>,
}
```

## Dirty Tracking

The `dirty` flag is the single bit that decides whether a part is copied through from `raw` or re-serialized. Because every round-trip invariant in spec 050 depends on it, its transition rule is normative.

- A part starts **clean** (`dirty = false`) on open/parse. Parsing, reading, building an agent view, computing checksums, and dry-run validation **never** set `dirty`.
- `dirty` becomes `true` only when a successful mutating edit changes the element/attribute/text content of that specific part. Applying a patch operation sets `dirty` on exactly the parts whose bytes the operation changes — no others.
- `dirty` is strictly **per-part**. Editing one slide does not dirty other slides, the presentation part, layouts, masters, or themes.
- A failed or dry-run operation must leave `dirty` unchanged (patches are atomic per 041: either all operations apply and their parts are marked dirty, or none are).
- The raw escape hatch (below) that replaces a part's bytes sets `dirty` on that part.

Edits that touch package control parts mark those control parts dirty by the same rule:

- `add_image` / `replace_image` (retarget) mark the edited slide part dirty, the slide's `.rels` part dirty (new relationship), and `[Content_Types].xml` dirty **only if** a new default/override is actually added (012). Reusing an already-covered content type does not dirty the content-types stream.
- `add_text_box`, `move_resize_element`, `set_alt_text`, and `replace_text` mark only the target slide part dirty.

A clean part is written byte-for-byte from `raw` in preserve mode (see Core Rule); a dirty part is serialized. This makes `part_checksum` (046) of a clean part identical across a no-edit round trip, and changes it for exactly the parts an edit dirtied.

## Parser Requirements

- Preserve namespace prefixes where possible.
- Preserve qualified names.
- Preserve attributes and unknown attributes.
- Preserve text nodes and escape them safely when writing.
- Preserve unknown elements.
- Support `.xml` and `.rels` files.
- Support markup compatibility and extension elements such as `mc:AlternateContent`.
- Provide meaningful diagnostics with part paths.

## Writer Requirements

- Emit well-formed XML.
- Preserve required namespace declarations.
- Dirty XML parts may be serialized as whole parts in V1; subtree/token-splice writing is an optional optimization, not a V1 guarantee.
- Within dirty parts, preserve unknown sibling elements and attributes when applying targeted edits.
- Support compact output by default for modified parts.
- Support deterministic output mode.
- Preserve relationship XML semantics.

## Raw Escape Hatch

Advanced users may request raw XML for a part and replace it directly. Raw replacement must still run basic well-formedness validation and package graph validation before writing.

## Legacy JSON Compatibility

The existing JavaScript package exposes XML as `xml2js`-shaped objects. The Rust rewrite may provide a compatibility export mode, but the native XML model should not be designed around `xml2js` internals.
