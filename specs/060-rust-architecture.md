# 060. Rust Architecture

## Workspace Layout

```text
crates/
  pptx-compose/          # public facade
  pptx-compose-core/     # OPC, ZIP, XML, PPTX model
  pptx-compose-json/     # legacy JSON and agent JSON serialization
  pptx-compose-edit/     # patches, selectors, edit journal
  pptx-compose-cli/      # command line tools
  pptx-compose-mcp/      # optional MCP server exposing safe agent tools
  pptx-compose-node/     # optional napi-rs bindings
  pptx-compose-wasm/     # optional browser/WASM bindings
```

## Module Boundaries

```text
pptx-compose-core
  opc/
    package
    part
    part_name
    content_types
    relationships
  zip/
    reader
    writer
    limits
  xml/
    document
    namespaces
    parser
    writer
  pptx/
    presentation
    slide
    shape
    text
    picture
    media
    ids
  validation/
  error/

pptx-compose-json
  legacy_path_map
  agent_view
  binary_encoding
  schemas
  schema_versions

pptx-compose-edit
  selectors
  operations
  patch
  media_inputs
  journal
  diffs
  reports

pptx-compose-mcp
  tools
  resources
  prompts
  sessions
  permissions
```

## Boundary Rules

- `opc` knows nothing about PowerPoint semantics.
- `zip` knows nothing about OPC semantics beyond safe entry handling.
- `xml` knows nothing about slides or media.
- `pptx` uses `opc` and `xml` to expose PowerPoint concepts.
- `edit` applies typed operations and returns reports.
- `json` serializes compatibility and agent-facing views.
- `mcp` depends on public facade/json/edit APIs and must not bypass validation or mutate internal OPC/XML structures directly.
- Public APIs should avoid exposing internal parser types unless behind advanced features.

## Error Handling

Use structured errors with context:

- I/O path.
- ZIP entry.
- Part name.
- Relationship ID.
- Slide/element ID.
- Operation name.
- Stable error code.
- Suggested next action for agents.

No malformed user input should panic.

Internal error kinds map 1:1 onto the canonical stable error codes defined in [results, validation, and errors](044-results-validation-errors.md#error-envelope) — that spec is the single source for the code vocabulary. Do not introduce error-code names here that are absent from 044; add them to 044 first. The structured error type carries the 044 `code` plus the context fields above.
