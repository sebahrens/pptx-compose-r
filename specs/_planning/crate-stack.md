# V1 Dependency Stack (verified 2026-06-05)

Recommended Rust crate stack for the cleanroom PPTX engine. Every row below was confirmed live against crates.io / docs.rs / the project repo on 2026-06-05. Pin exact versions in `Cargo.toml`; pre-1.0 crates (`jsonschema`) especially.

| Concern | Crate | Version (2026-06-05) | License | Verdict |
| --- | --- | --- | --- | --- |
| ZIP read/write | **`zip`** (zip-rs/zip2) | 8.6.x | MIT | **Use.** `ZipWriter::raw_copy_file` = verbatim passthrough of unmodified parts. ⚠ CVE-2025-29787 (patched) — still add our own path-traversal defense. |
| XML parse/serialize | **`quick-xml`** | 0.40.x | MIT | **Use.** Namespace-aware; `BytesText::from_escaped` = raw-byte text round-trip. Handle `GeneralRef` events (0.37+) and preserve prefixes via raw bytes. |
| MCP server | **`rmcp`** | 1.7.0 | Apache-2.0 | **Use.** Official MCP Rust SDK (`modelcontextprotocol/rust-sdk`), tokio-based, `#[tool]`/`#[tool_router]`/`#[tool_handler]` + `ServerHandler`, optional `schemars` feature. Actively maintained (biweekly releases). Commits us to **tokio**. |
| JSON values | **`serde_json`** | 1.0.150 | MIT OR Apache-2.0 | **Use.** Canonical. |
| Schema generation | **`schemars`** | 1.2.x | MIT | **Use.** `#[derive(JsonSchema)]` + `schema_for!`; honors serde attrs → schemas match serialization. Stable 1.x. |
| Schema validation | **`jsonschema`** | 0.46.x | MIT | **Use, pin tightly.** Validates `serde_json::Value`; supports Draft 2020-12. Still 0.x → breaking minors. |
| CLI | **`clap`** | 4.6.x | MIT OR Apache-2.0 | **Use.** Derive API (`#[derive(Parser)]`). |
| Checksums | **`sha2`** | 0.11.0 | MIT OR Apache-2.0 | **Use.** RustCrypto SHA-256 — matches the `sha256:` digests pinned in spec 046. Pure-Rust, no_std-capable. (Just moved to 0.11 major — review digest-trait API on upgrade.) |
| Checksums (alt) | `blake3` | 1.8.x | CC0/Apache-2.0/LLVM-exc | **Skip for V1.** Faster, but spec 046 pins SHA-256; only relevant if the hash scheme is deliberately redefined. |

## Build in-house (no adequate crate exists)

The research found **no drop-in Rust OOXML/PPTX foundation** (`office_oxide` v0.1.2 mirrors our architecture but is text-edit-only and too immature; `ooxml`/`ooxml-rs` is xlsx-only; `ppt-rs` is generation-first; `ooxmlsdk` normalizes bytes and kills losslessness). So we build:

- OPC package model: `[Content_Types].xml` + relationship graph (specs 010, 012).
- Byte-preserving round-trip guarantees + their tests — `zip`/`quick-xml` give primitives but **no documented determinism/preservation guarantee** (specs 020, 050, 046).
- Bounded/paginated agent-JSON view + DocumentIR (specs 040, 042).
- Validated patch operations with document/revision guards + operation IDs (specs 041, 042, 047).
- OPC validation (content-type coverage, rel resolution, unique rel IDs) (specs 044, 050).
- Zip-bomb / path-traversal / resource-limit defenses (spec 011) — do **not** rely solely on `zip` for path safety.

## Reference only

`office_oxide`'s `to_ir()` / CLI / MCP-server shape is a useful architectural template, not a dependency.
