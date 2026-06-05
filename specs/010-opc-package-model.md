# 010. OPC Package Model

PPTX files are Open Packaging Convention packages: ZIP archives containing named parts, content types, and relationships. The OPC layer must not know PowerPoint-specific semantics.

## Core Types

```rust
struct Package {
    parts: PartStore,
    content_types: ContentTypes,
    relationships: RelationshipGraph,
    metadata: PackageMetadata,
    warnings: Vec<PackageWarning>,
}

enum PartData {
    Xml(XmlPart),
    Binary(BinaryPart),
}

struct Part {
    name: PartName,
    content_type: Option<ContentType>,
    data: PartData,
    zip_metadata: ZipEntryMetadata,
}
```

## Part Names

Requirements:

- Use normalized OPC part names internally.
- Preserve the original display/path form for writing when safe.
- Reject path traversal and absolute paths.
- Reject duplicate normalized names.
- Treat package **part-name path segments** as case-sensitive (`/ppt/Slides/slide1.xml` and `/ppt/slides/slide1.xml` are distinct parts).
- This case-sensitivity applies to part names only. Content-type **Default extension** matching is ASCII case-insensitive (so `image1.PNG` resolves against Default `png`); see the resolution algorithm in [content types and relationships](012-content-types-and-relationships.md).

Examples:

- Valid: `/ppt/slides/slide1.xml`
- Valid external relationship target: `https://example.com/image.png`
- Invalid part name: `../evil.xml`
- Invalid part name: `/ppt/../evil.xml`

Normative path conventions:

| Context | Form |
| --- | --- |
| Internal canonical `PartName` | Leading slash, e.g. `/ppt/slides/slide1.xml` |
| ZIP entry name | No leading slash, e.g. `ppt/slides/slide1.xml` |
| Legacy JSON path | No leading slash, matching ZIP entry names |
| Agent JSON path | No leading slash unless explicitly labeled `part_name` |
| Relationship `Target` attribute | Preserve as authored when possible; resolve relative to source part |

`[Content_Types].xml` is the package content-type stream and is parsed into `ContentTypes`; it is not validated as an ordinary content-typed part. Relationship parts are parsed into `RelationshipGraph`; if retained in the raw part store for preservation, they must be marked as OPC control parts.

### Part-Name Normalization

A ZIP entry name is normalized into a canonical `PartName` by this exact procedure; any step that fails rejects the part with `unsafe_path`:

1. Convert backslashes (`\`) to forward slashes, then percent-decode (`%XX`) once. A literal `%` that is not a valid escape is an error.
2. Prepend a leading `/` if absent. The canonical form always has exactly one leading slash.
3. Split on `/` into segments. Reject if any segment is empty (collapsed `//`), `.`, or `..`, or if the name has a trailing slash (a part name never names a directory).
4. Reject absolute drive/UNC forms surviving step 1 (e.g. a segment containing `:` at position 0, or a leading `//`).
5. Segment comparison is **case-sensitive** (see above). Normalization does **not** lowercase. The only case-insensitive comparison anywhere in part handling is the content-type Default-extension match in [012](012-content-types-and-relationships.md).

Two ZIP entries that normalize to the same `PartName` are a duplicate-part error. The original ZIP entry name (no leading slash) is retained for byte-faithful writing; normalization affects identity and lookup only.

### Relative Target Resolution

A relationship `Target` with `TargetMode=Internal` is resolved relative to the **base** of its source part — the source part name with its last segment removed. Resolution is `../`-aware:

1. Start from the source base segments (e.g. source `/ppt/slides/slide1.xml` → base `/ppt/slides`).
2. Split the `Target` on `/`. For each segment: `.` is skipped; `..` pops one segment from the accumulated path (popping past the root is `unsafe_path`); any other segment is pushed.
3. A `Target` beginning with `/` resolves from the package root instead of the source base.
4. The result is normalized as a `PartName` and must resolve to an existing part (else `dangling_internal_relationship`, [044](044-results-validation-errors.md#validation-finding-codes)).

Worked example: source part `/ppt/slides/slide1.xml`, relationship `Target="../media/image1.png"` → base `/ppt/slides` → pop to `/ppt` → push `media`, `image1.png` → `/ppt/media/image1.png`.

`TargetMode=External` targets (e.g. `https://…`) are not resolved against the package and are preserved verbatim.

## Relationships

Relationships are first-class graph edges:

```rust
struct Relationship {
    source: RelationshipSource,
    id: RelationshipId,
    rel_type: RelationshipType,
    target: RelationshipTarget,
    target_mode: TargetMode,
}
```

Relationship sources:

- Package root: `/_rels/.rels`
- Part-specific: e.g. `/ppt/slides/_rels/slide1.xml.rels`

Relationship targets must resolve relative to the source part unless marked external.

## Package Invariants

- Every internal relationship target resolves to an existing part.
- Every ordinary package part has a content type through `[Content_Types].xml` defaults or overrides.
- Relationship IDs are unique within their relationship part.
- Unknown parts and relationships are preserved.
- Directory entries are not semantically required but may be preserved for fidelity.
- **Orphan parts are preserved, never auto-pruned in V1.** A part that is not reachable through the relationship graph (no relationship targets it, transitively from the package root) is still a valid package member: it is written through unchanged and reported as an `orphan_part` info finding ([044](044-results-validation-errors.md#validation-finding-codes)), not an error. Editing operations must not prune orphans; e.g. `replace_image` may leave the prior media part orphaned, and that orphan is preserved. Pruning is deferred to post-V1 slide-delete work.

## Required APIs

- `Package::open_reader(reader, options) -> Result<Package>`
- `Package::from_bytes(bytes, options) -> Result<Package>`
- `Package::part(name) -> Option<&Part>`
- `Package::part_mut(name) -> Option<&mut Part>`
- `Package::relationships_for(source) -> &[Relationship]`
- `Package::validate() -> ValidationReport`
- `Package::write(writer, options) -> Result<()>`
