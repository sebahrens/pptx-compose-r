# 046. Provenance and Hashing

Every safety mechanism in this engine — stale-patch rejection, selector guards, revision tracking, no-edit round-trip tests — depends on comparing identity values (`document_id`, `revision`, `part_checksum`, `fingerprint`, `text_hash`, `view_id`) and agent IDs (`slide-*`, `*:shape-*`, `*:p*`, `*:r*`). Specs 040, 041, 042, and 050 reference these values but none defines how they are produced. This spec is the single normative source for their derivation.

The governing requirement: **two conformant implementations, and the same implementation across a no-edit round trip, must produce identical values for identical input.** Any value defined here must be reproducible from the package bytes alone, with no dependence on wall-clock time, iteration order of hash maps, locale, or filesystem ordering.

## Hash Primitive

- The hash algorithm for all provenance values in V1 is **SHA-256**.
- Hash values are serialized as the lowercase ASCII string `sha256:` followed by 64 lowercase hexadecimal characters, e.g. `sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- The placeholder `sha256:...` used in other specs is illustrative only; conformant output must contain the full digest.
- Implementations may expose additional algorithms behind an explicit option later; V1 readers and writers must default to SHA-256 so values are comparable across tools.

## Canonical Preimage Encoding

Several values hash a structured preimage rather than raw bytes. To keep digests stable across implementations, structured preimages use **Canonical Preimage JSON (CPJ)**:

- UTF-8 encoded.
- Object keys sorted by Unicode code point, ascending.
- No insignificant whitespace (no spaces, no newlines) between tokens.
- Strings use minimal JSON escaping (only `"`, `\`, and control characters `U+0000`–`U+001F`); non-ASCII is emitted as raw UTF-8, not `\u` escapes.
- Integers are emitted in base-10 with no leading zeros and no `+`; arrays preserve their given order.
- No floating-point values appear in any preimage defined here.

A digest of a CPJ value is `sha256:` + hex(SHA-256(CPJ-bytes)).

## `part_checksum`

`part_checksum` is the digest of a single part's **raw stored bytes**:

- The input is the part's uncompressed byte payload — the bytes the ZIP entry decompresses to (or the literal bytes for a stored/uncompressed entry).
- It is **not** computed over re-serialized XML, over compressed bytes, or over a canonicalized form. For an unmodified part this is the original payload; for a dirty part it is the bytes the writer will emit for that part.
- It covers `.xml`, `.rels`, media, and any other ordinary part identically — there is no XML-specific normalization.
- The `[Content_Types].xml` stream is checksummed by this same rule when it participates in `document_id` (below).

This pins B-blocker B1's "raw vs re-serialized" question to **raw**, which makes `part_checksum` stable across a no-edit round trip without requiring byte-identical re-serialization of XML.

## `document_id`

`document_id` identifies the exact package content. It is the digest of a CPJ manifest of every ordinary part plus the content-types stream:

1. Build the set of entries:
   - One entry per ordinary package part: `{ "name": <canonical PartName>, "checksum": <part_checksum> }`.
   - One entry for the content-types stream: `{ "name": "/[Content_Types].xml", "checksum": <part_checksum of the stream bytes> }`.
   - Relationship parts (`*.rels` and `/_rels/.rels`) are ordinary parts for this purpose and are included.
   - ZIP directory entries (names ending in `/`) and any entry excluded from the part store are **not** included.
2. `<canonical PartName>` is the internal canonical part name from spec 010 (leading slash, normalized) — see [OPC package model](010-opc-package-model.md).
3. Sort entries by `name` (Unicode code point, ascending). Names are unique by the OPC duplicate-name rule, so the sort is total.
4. The preimage is the CPJ document `{ "parts": [ <sorted entries> ], "schema": "pptx-compose.document_id.v1" }`.
5. `document_id = sha256:` + hex(SHA-256(CPJ-bytes)).

Properties this guarantees:

- A no-edit round trip yields the same `document_id` (it depends only on raw part payloads and names, both preserved).
- Reordering ZIP entries, changing compression, or normalizing timestamps does **not** change `document_id`.
- Adding, removing, or mutating any part's bytes **does** change it.

## `revision`

`revision` is a monotonic integer scoped to an open document instance:

- On a fresh open/parse, `revision = 1`.
- Each successful **non-dry-run** `apply_patch` that writes at least one part increments `revision` by exactly 1. The post-apply value appears as `new_revision` in the patch report (044).
- A dry-run, a failed apply, or an apply that the engine determines is a no-op does **not** increment `revision`.
- `revision` is in-memory session state, not persisted into the `.pptx`. It is meaningful only together with `document_id`: a patch's `(document_id, base_revision)` pair must match the current document state or the patch is rejected with `stale_patch` (044).
- `document_id` is the durable identity (survives reopen); `revision` is the cheap monotonic guard within a session. Both are emitted in views (040/042) and patch envelopes (041/042).

## `text_hash`

`text_hash` is the digest of an element's **normalized text projection**:

1. Compute the `normalized` text exactly as exposed in the text element view (042 "Text Element View"): the concatenation of run text in document order, with the normalization rule below.
2. Normalization: Unicode NFC; collapse each run of XML whitespace (`U+0020`, `U+0009`, `U+000A`, `U+000D`) to a single `U+0020`; trim leading and trailing whitespace of the whole projection. Soft line breaks (`a:br`) project as a single `U+000A` that is preserved (not collapsed) so multi-line text remains distinguishable.
3. `text_hash = sha256:` + hex(SHA-256(UTF-8 bytes of the normalized projection)).

The same normalization defines the value of the `normalized` field in 042, so the field and its hash are always consistent. `plain` (042) is the unnormalized concatenation and is never hashed.

## `fingerprint`

`fingerprint` is an element's structural identity digest, used as a selector guard (041/042) to detect that the element a stale ID points at is no longer the element the agent saw:

1. Build the CPJ preimage:
   ```
   {
     "kind": <element kind, e.g. "text_box" | "image" | "shape" | "group">,
     "part": <canonical PartName of the slide part>,
     "sp_tree_path": <array of integers, the spTree child-index path>,
     "group_path": <array of integers, empty when top-level>,
     "cnvpr_id": <integer cNvPr id, or null if the element has no cNvPr>,
     "text_hash": <text_hash string, or null for non-text elements>,
     "schema": "pptx-compose.fingerprint.v1"
   }
   ```
2. `fingerprint = sha256:` + hex(SHA-256(CPJ-bytes)).

Notes:

- `sp_tree_path` / `group_path` use the same indexing as `xml_location` in 042 (positional child indices within `p:spTree`, descending through `p:grpSp`). They locate the element; `cnvpr_id` and `text_hash` detect substitution at that location.
- `cnvpr_id` is included for substitution detection only. It is **not** the agent ID and must never be used to derive one (cNvPr ids are reassignable and may collide across the deck).

## `view_id`

`view_id` identifies a specific materialized view so a later patch can be checked against the exact projection the agent read:

1. Preimage CPJ:
   ```
   {
     "document_id": <document_id>,
     "revision": <revision>,
     "mode": <view mode, e.g. "deck_summary" | "slide_detail" | "element_detail">,
     "scope": <CPJ of the view's scoping parameters: slide ids, element id, pagination cursor/limit, detail level>,
     "schema": "pptx-compose.view_id.v1"
   }
   ```
2. `view_id = sha256:` + hex(SHA-256(CPJ-bytes)).

Two views with identical document, revision, mode, and scope share a `view_id`; any difference yields a different one.

## Agent ID Derivation

Agent IDs are stable string identifiers for the exported `(document_id, revision)`. They are derived from **stable structural position**, never from reassignable OOXML ids.

### Slide IDs

- A slide's agent ID is `slide-{n}` where `n` is the 1-based position of the slide in presentation order (`<p:sldIdLst>` order in `ppt/presentation.xml`), i.e. `n = index + 1`.
- The slide's OOXML `<p:sldId>` value is exposed separately as `ppt_slide_id` (042) and must **not** be used to form the agent ID.

### Element IDs

- An element's agent ID is `{slide_id}:{kind_prefix}-{key}` where:
  - `slide_id` is the containing slide's agent ID.
  - `kind_prefix` is a fixed token per kind: `shape` (autoshape/text box / placeholder shape), `pic` (picture), `group` (group shape), `graphic` (graphicFrame: chart/table/SmartArt), `cxn` (connector), `oth` (any other/unknown shape kind).
  - `key` is the element's `p:cNvPr/@id` value when present. For malformed or unknown `p:spTree` child elements that lack `cNvPr`, implementations may fall back to the dotted `sp_tree_path` so the element remains addressable within the exported revision.
- Rationale: `cNvPr/@id` is the DrawingML non-visual drawing property ID and remains stable under reordering of siblings. After an edit that changes or allocates a drawing ID, a new export produces a new `(document_id, revision)` and IDs are recomputed.
- The illustrative examples in 040/042 (`slide-1:shape-4`, `slide-1:pic-7`) are pinned to this rule: the numeric suffix is the `cNvPr/@id` value, not the `p:spTree` child index.

### Paragraph and Run IDs

- A paragraph's agent ID is `{element_id}:p{i}` where `i` is the 0-based index of the `a:p` within the element's `a:txBody`.
- A run's agent ID is `{paragraph_id}:r{j}` where `j` is the 0-based index of the `a:r` within that paragraph. Non-run paragraph children (`a:br`, `a:fld`) do not consume run indices but are preserved in the underlying XML.

## Stability Guarantees

Conformant implementations must satisfy, and 080 must test:

- **No-edit stability:** `read(input).write(output)` preserves `document_id`, every `part_checksum`, every agent ID, and every `fingerprint`/`text_hash`. Re-opening `output` yields the same `document_id`.
- **Cross-implementation determinism:** identical input bytes yield identical `document_id`, `part_checksum`, `text_hash`, `fingerprint`, and agent IDs across conformant implementations.
- **Edit locality:** an edit changes exactly the `part_checksum` of dirty parts (031/020 dirty-tracking) and therefore `document_id`; it changes a given element's agent ID only if the edit changes that element's `cNvPr/@id` or the fallback structural path for an element without `cNvPr`.
- **Guard soundness:** a selector whose `fingerprint` and `text_hash` guards both match the current element is treated as resolving to the same element the agent saw; a mismatch yields `selector_guard_failed` (044) rather than a best-effort edit.

See [agent JSON format](040-agent-json-format.md), [agent edit operations](041-agent-edit-operations.md), [agent protocol schemas](042-agent-protocol-schemas.md), and [round-trip invariants](050-roundtrip-invariants.md).
