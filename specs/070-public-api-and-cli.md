# 070. Public API and CLI

## Rust API

Recommended core API:

```rust
let mut deck = PresentationDocument::open_path_with_options("input.pptx", OpenOptions::default())?;
let view = deck.to_agent_json_with_options(AgentViewOptions::summary())?;
let media = MediaInputs::from_manifest("media.manifest.json")?;
let report = deck.apply_patch_with_options(patch, media, ApplyPatchOptions { dry_run: false, validate: true })?;
deck.write_path_with_options("output.pptx", WriteOptions { mode: WriteMode::Preserve, overwrite: false, validate: true, atomic: true })?;
```

`write_path` and `write_vec` must validate edited documents by default. Calling `validate` explicitly is still useful for preflight reports and CLI diagnostics.

## Write and Open Modes

Preserve-mode and deterministic-mode are load-bearing for the round-trip invariants (050) and the XML core rule (020), so they are explicit options with a stated default, not implicit behavior.

```rust
enum WriteMode {
    Preserve,       // default: clean parts emitted byte-for-byte from raw; dirty parts serialized
    Deterministic,  // stable ZIP order/timestamps/compression; still preserves clean part *payloads*
}

struct WriteOptions {
    mode: WriteMode,   // default WriteMode::Preserve
    overwrite: bool,   // default false
    validate: bool,    // default true; false only via explicit unsafe/debug path
    atomic: bool,      // default true
}
```

Semantics:

- The default is **`WriteMode::Preserve`**. In preserve mode every clean (non-dirty) part is written byte-for-byte from its `raw` bytes; only dirty parts (020 dirty-tracking) are serialized.
- `WriteMode::Deterministic` normalizes ZIP entry order, timestamps, compression settings, and content-type/relationship ordering (011 "Deterministic Output") and serializes dirty parts with the deterministic serialization profile. It must **never** re-serialize a clean part: clean-part *payload* bytes are still copied from `raw`, so `part_checksum`/`document_id` (046) are unchanged; only ZIP framing/order may differ. Where preserve and deterministic conflict for a clean part, preserve wins for that part's payload.
- `OpenOptions` carries the resource limits and security policy from [ZIP I/O and security](011-zip-io-and-security.md); it does not carry a write mode.
- The CLI exposes mode via `--deterministic` (selects `Deterministic`; default is `Preserve`). See [CLI agent contract](071-cli-agent-contract.md).

Required methods:

- `PresentationDocument::open_path`
- `PresentationDocument::from_bytes`
- `PresentationDocument::open_reader`
- `PresentationDocument::to_agent_json`
- `PresentationDocument::to_legacy_json`
- `PresentationDocument::apply_patch`
- `PresentationDocument::validate`
- `PresentationDocument::write_path`
- `PresentationDocument::write_vec`

Write options must include an explicit validation toggle only for unsafe/debug use. Default agent and CLI workflows must not bypass validation.

Patch application and validation APIs must return structured reports defined in [results, validation, and errors](044-results-validation-errors.md), not just `Result<()>`.

## Node Compatibility API

Optional Node bindings should support a familiar shape:

```js
import PPTXCompose from "pptx-compose";

const composer = new PPTXCompose(options);
const json = await composer.toJSON("input.pptx", options);
const bytes = await composer.toPPTX(json, options);
```

Legacy JSZip option names may be accepted but should map to explicit new concepts such as binary encoding and output type.

## CLI

This section is a **non-normative orientation only**. The normative, agent-facing CLI contract — command set, flags, JSON discipline, and exit codes — is [CLI agent contract](071-cli-agent-contract.md). Where this summary and 071 differ, 071 wins. Do not duplicate or re-specify exit codes here.

The core agent commands (`inspect`, `validate`, `apply`/`apply --dry-run`, `media`, `schema`) are defined in 071.

```bash
pptx-compose inspect input.pptx --format agent-json --output out.json
pptx-compose apply input.pptx patch.json --output output.pptx
pptx-compose apply input.pptx patch.json --dry-run --report report.json
pptx-compose validate input.pptx
pptx-compose schema patch-v1
```

### Legacy conversion commands (separate compat scope)

`to-json`, `to-pptx`, and the `convert` alias reproduce the legacy TypeScript package's whole-deck JSON round-trip. They are **not** part of the normative V1 agent CLI in 071; they are a distinct compatibility unit specified by observable current behavior ([002](002-observable-current-behavior.md)) and gated by the legacy-parity fixtures in [080](080-testing-and-fixtures.md). They are tracked as their own epic and may ship later than the core agent CLI.

```bash
pptx-compose to-json input.pptx output.json --compat-json
pptx-compose to-pptx input.json output.pptx --compat-json
pptx-compose convert input.pptx output.json   # alias
```

Exit codes for all commands follow the single normative table in [CLI agent contract](071-cli-agent-contract.md#exit-codes). The MCP server is a separate public agent interface specified in [MCP server contract](072-mcp-server-contract.md); it must not be inferred from CLI commands alone.
