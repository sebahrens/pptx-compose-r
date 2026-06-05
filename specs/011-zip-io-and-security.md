# 011. ZIP I/O and Security

## Input Forms

The Rust API should support:

- `open_path(path)`
- `from_bytes(&[u8])`
- `open_reader(Read + Seek)`

Optional async support may be a feature flag after the synchronous API is stable.

## Package Format Detection (Pre-ZIP Sniff)

Before any ZIP parsing, the input layer must sniff the leading bytes and reject non-PPTX containers with a clear, fixture-testable error rather than failing deep in the ZIP reader. A `.pptx` is a ZIP package (local file header magic `50 4B 03 04`, i.e. `PK\x03\x04`).

| Leading bytes / condition | Meaning | Result |
| --- | --- | --- |
| `D0 CF 11 E0 A1 B1 1A E1` | OLE/CFBF compound file — this is how an **encrypted** OOXML deck or a legacy binary `.ppt` is stored | `unsupported_package`, exit `11`, no ZIP parse attempted |
| `50 4B 03 04` ZIP that contains an `EncryptionInfo` and/or `EncryptedPackage` stream/part | Agile/standard **encrypted** OOXML | `unsupported_package`, exit `11` |
| `50 4B 03 04` ZIP, otherwise | Candidate OPC package | proceed to ZIP/OPC parsing |
| anything else | Not a recognized package | `unsupported_package`, exit `11` |

The encrypted-deck case must be detected and reported, never silently mis-parsed. V1 does not decrypt; a clear `unsupported_package` error is the contract. This pins the encrypted-deck negative eval (081) and the "encrypted deck" QA case to a concrete trigger.

## Output Forms

The Rust API should support:

- `write_path(path)`
- `write_vec() -> Vec<u8>`
- `write_writer(Write + Seek)` where supported by the ZIP writer.

## Security Requirements

- Do not extract package entries directly to a filesystem directory by default.
- Reject or quarantine path traversal entries.
- Enforce configurable resource limits. Each limit has a concrete V1 default; callers may override via `OpenOptions`. Exceeding any limit aborts parsing with `resource_limit_exceeded` (exit `12`).

  | Limit | V1 default | Notes |
  | --- | --- | --- |
  | maximum compressed bytes (whole package) | 524288000 (500 MiB) | size of the input stream |
  | maximum uncompressed bytes (whole package) | 2147483648 (2 GiB) | summed across all entries |
  | maximum part count | 10000 | ordinary parts + control parts |
  | maximum single part uncompressed size | 268435456 (256 MiB) | per entry |
  | maximum media part size | 67108864 (64 MiB) | per media entry; also `--max-media-bytes` (071) |
  | maximum per-entry compression ratio | 200:1 | uncompressed ÷ compressed, evaluated during streaming inflate |
  | maximum XML element depth | 256 | nesting depth |
  | maximum XML node count (per part) | 5000000 | elements + text nodes |

- **Zip-bomb defense:** the per-entry compression ratio and the per-entry/whole-package uncompressed limits must be enforced **during** streaming decompression and abort as soon as a running total crosses the threshold — never by decompressing fully and checking afterward.
- Do not fetch external relationships.
- Treat embedded packages and OLE objects as opaque binary unless explicitly opened by a caller.

## Preservation Requirements

- Preserve unmodified binary payloads exactly.
- Preserve unmodified XML raw bytes in preserve mode.
- Preserve unknown ZIP entries as parts or raw entries when safe.
- Preserve original part ordering where practical in preserve mode.

## Deterministic Output

Deterministic mode should provide stable testable output:

- Stable ZIP entry order.
- Stable compression settings.
- Stable timestamps or normalized timestamps.
- Stable relationship and content-type ordering.

Whole-file byte equality is not required unless a special normalized test mode is used.
