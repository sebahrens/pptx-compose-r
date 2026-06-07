# 071. CLI Agent Contract

The CLI is a stable, scriptable protocol for agents. Human-friendly output is secondary to deterministic machine behavior.

## Global Rules

- No command may prompt interactively.
- Human progress logs go to stderr only.
- Primary JSON output goes to stdout or an explicit output path.
- Commands must never mix human prose into JSON streams.
- `-` means stdin/stdout where safe.
- Every machine-readable JSON document includes `schema`, `version`, and `status` or a schema-specific equivalent.
- Failed commands with `--json-errors` emit exactly one [error envelope](044-results-validation-errors.md#error-envelope) to stderr.
- Commands must not write outside explicit output paths, `--workspace`, or `--temp-dir`.

## Global Flags

Required global flags:

```bash
pptx-compose --version
pptx-compose --help
pptx-compose --json-errors
pptx-compose --quiet
pptx-compose --verbose
pptx-compose --no-color
pptx-compose --workspace DIR
pptx-compose --temp-dir DIR
pptx-compose --max-uncompressed-bytes N
pptx-compose --max-part-count N
```

## Commands

### `inspect`

```bash
pptx-compose inspect INPUT.pptx --format agent-json --output deck.view.json --report inspect.report.json
pptx-compose inspect INPUT.pptx --slides 1-5 --detail summary --output - --json-errors
```

`inspect` must not modify input or write PPTX output. It emits an agent view matching [agent protocol schemas](042-agent-protocol-schemas.md).

### `validate`

```bash
pptx-compose validate INPUT.pptx --report validation.json
pptx-compose validate INPUT.pptx --report - --json-errors
```

Validation emits a [validation report](044-results-validation-errors.md#validation-report).

### `apply --dry-run`

```bash
pptx-compose apply INPUT.pptx PATCH.json --dry-run --media-manifest media.json --media-root assets --media hero=override.png --report dry-run.report.json --diff diff.json
```

Dry-run validates and computes reports/diffs without creating PPTX output or mutating state.

### `apply`

```bash
pptx-compose apply INPUT.pptx PATCH.json --media-manifest media.json --output OUTPUT.pptx --report apply.report.json
pptx-compose apply INPUT.pptx PATCH.json --in-place --no-backup --report apply.report.json
```

Flags:

| Flag | Meaning |
| --- | --- |
| `--dry-run` | Validate the patch and compute reports/diffs without writing PPTX output. |
| `--media-manifest PATH` | Load a media manifest that maps patch `media_ref` values to input media. |
| `--media-root DIR` | Resolve relative media paths in `--media-manifest` against `DIR` instead of the manifest's parent directory. Requires `--media-manifest`. |
| `--media MEDIA_REF=PATH` | Bind one media reference directly to a file path. May be repeated. Explicit bindings override manifest bindings for the same `MEDIA_REF`. Duplicate explicit bindings for the same `MEDIA_REF` fail. |
| `--output PATH` | Write the edited PPTX to `PATH`. Required unless `--dry-run` or `--in-place` is selected. |
| `--report PATH` | Write the patch report JSON to `PATH` or `-` for stdout where safe. |
| `--diff PATH` | Write the patch diff JSON to `PATH` or `-` for stdout where safe. |
| `--overwrite` | Permit replacing an existing `--output`, `--report`, or `--diff` path. |
| `--in-place` | Write back to `INPUT.pptx` atomically. If `--output` is present it must be the same path as `INPUT.pptx`. |
| `--no-backup` | Suppress the default `INPUT.pptx.bak` backup. Valid only with `--in-place`. |
| `--deterministic` | Accepted for CLI stability; deterministic ZIP framing/order/metadata is already the default write profile. |

Rules:

- Must be atomic: never leave a partial output at `--output`.
- Must fail if output exists unless `--overwrite` is explicit.
- `--in-place` sets the output path to `INPUT.pptx`; if `--output` is also present it must equal the input path.
- `--no-backup` is meaningful only with `--in-place`; non-in-place writes never create an input backup.
- `--media-root` is valid only with `--media-manifest`.
- If both `--media-manifest` and repeatable `--media MEDIA_REF=PATH` bindings are present, each explicit `--media` binding overrides the manifest entry with the same `MEDIA_REF`; unrelated manifest bindings remain available.
- Must validate edited output before final write by default.
- Must write deterministic ZIP framing/order/metadata by default, while still raw-copying clean part payload bytes.
- Must emit a patch report with operations, changed parts, generated IDs, warnings, and output document fingerprint.

### `media`

```bash
pptx-compose media list INPUT.pptx --json
pptx-compose media get INPUT.pptx ppt/media/image1.png --output image1.png --report media.report.json
```

Media extraction must sanitize package paths and write only under explicit output paths.

### `schema`

```bash
pptx-compose schema agent-view-v1
pptx-compose schema patch-v1
pptx-compose schema media-manifest-v1
pptx-compose schema patch-report-v1
pptx-compose schema validation-report-v1
pptx-compose schema result-v1
pptx-compose schema error-v1
pptx-compose schema capabilities-v1
```

`schema` prints JSON Schema to stdout.

### Legacy conversion commands (out of core scope)

`to-json`, `to-pptx`, and the `convert` alias exist for legacy parity with the TypeScript package's whole-deck JSON round-trip. They are intentionally **outside** this normative agent contract: their behavior is pinned by observable current behavior ([002](002-observable-current-behavior.md)) and the legacy fixtures in [080](080-testing-and-fixtures.md), not by the command rules above, and they are scoped as a separate compatibility epic (see [070](070-public-api-and-cli.md#legacy-conversion-commands-separate-compat-scope)). Agents performing V1 edits must use `inspect`/`apply`, never the legacy conversion path.

## Exit Codes

Exit codes are coarse buckets; agents should read the JSON `error.code` ([044](044-results-validation-errors.md#error-envelope)) for precise semantics. This table is the **sole normative** CLI exit mapping (the summary in [070](070-public-api-and-cli.md) is non-normative and defers here). Each row lists the 044 error codes that roll up into that exit.

| Exit | Meaning | 044 error codes |
| --- | --- | --- |
| `0` | success | — |
| `1` | command-line usage error | (argument parsing; emitted before a structured envelope, or `invalid_input` for a bad flag value) |
| `2` | input file not found/unreadable | `invalid_input` (missing/unreadable input path) |
| `3` | unsafe path / permission violation | `unsafe_path`, `permission_denied` |
| `10` | parse/open failure | `parse_error` |
| `11` | unsupported/encrypted package | `unsupported_package` |
| `12` | resource limit exceeded | `resource_limit_exceeded` |
| `20` | patch invalid (schema or operation precondition) | `invalid_input` (patch fails schema), `invalid_bounds` |
| `21` | stale document/revision | `stale_patch` |
| `22` | selector resolution failure | `selector_not_found`, `selector_ambiguous`, `selector_guard_failed` |
| `23` | media resolution failure | `missing_media_ref`, `media_checksum_mismatch`, `unsupported_media_type` |
| `24` | unsupported operation | `unsupported_edit` |
| `30` | validation failure | `validation_failed` |
| `40` | write failure | `write_failed` |
| `50` | internal error | `internal_error` |

`invalid_input` is the only code that spans multiple exits (1/2/20) by sub-cause: a bad CLI flag is exit 1, an unreadable input file is exit 2, and a patch document that fails schema validation is exit 20. Agents should branch on `error.code`, not the exit, to disambiguate.

## Idempotency and Retries

- `inspect`, `validate`, and dry-run are read-only and idempotent.
- `apply` is idempotent only for the same input bytes, patch bytes, media bytes, deterministic options, and `client_request_id`.
- Existing outputs are not overwritten unless `--overwrite` is explicit.
- `--if-output-document-id sha256:...` may allow conditional overwrite.
