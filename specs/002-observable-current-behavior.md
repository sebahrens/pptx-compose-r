# 002. Observable Current Behavior

This spec documents the current TypeScript package only as externally observable behavior. It does not prescribe Rust implementation details.

## Package Surface

The current package exposes a default `Composer` class with two main methods:

- `toJSON(file, options?)`
- `toPPTX(json, options?)`

It also has parser helpers in the source package:

- `jszip2json(jszip, options?)`
- `json2jszip(json, options?)`

## `toJSON` Behavior

Observable behavior:

- Reads a PPTX/ZIP file.
- Iterates package entries.
- Skips directory entries.
- Parses entries ending in `.xml` or `.rels` into JavaScript objects.
- Reads every other entry as binary data.
- Returns an object keyed by ZIP-relative paths such as `ppt/presentation.xml`.
- If an output path is passed, writes pretty JSON and resolves to the file-write result rather than returning the JSON object.

## `toPPTX` Behavior

Observable behavior:

- Accepts a path-keyed JSON object.
- Serializes values for `.xml` and `.rels` paths back to XML.
- Writes every other value as a raw ZIP file payload.
- Generates a PPTX/ZIP byte output.
- If an output path is passed, writes the output file and resolves to the file-write result.

## Current Options

- `jszipBinary`: controls binary read output type in the current JS implementation.
- `jszipGenerateType`: controls generated output type when set at construction time.
- `output`: writes method output to a file.

The Rust rewrite should not carry JSZip-specific option names into the Rust-native API. Node compatibility bindings may map legacy options to safer new options.

## Test-Covered Behavior

Current tests assert only smoke-level behavior:

- Parsing a sample PPTX produces a key named `ppt/presentation.xml`.
- `toJSON` followed by `toPPTX` returns a generated value.
- Adding a binary JPEG entry under `ppt/media/...` does not crash writing.
- A simple synthetic ZIP with one XML file can be created from JSON.

## Important Current Limitations

- No typed presentation model.
- No content-type management.
- No relationship graph management.
- No validation that output opens in PowerPoint.
- No guarantee that XML formatting, comments, namespace details, or raw bytes survive XML parse/write.
- No durable file-based binary JSON format.
- Adding an image file to `ppt/media` alone does not make the image appear on any slide.
- CLI behavior and README examples contain inconsistencies.

## Compatibility Guidance

The Rust rewrite should distinguish these tiers:

- `must-match`: path-keyed package inventory, XML-vs-binary classification, basic parse/write API shape for compatibility wrappers.
- `should-match`: ability to export a legacy JSON-like view.
- `intentionally-different`: safer path resolution, useful return values for file writes, real CLI flags, validation, explicit binary encoding.
- `unsupported`: exact JSZip and `xml2js` internal quirks unless a separate compatibility mode is required.

The V1 compatibility checklist is maintained in the
[compatibility decision register](_planning/compatibility-decisions.md). Before
closing compatibility implementation beads, every behavior relied on by this spec
and the compatibility tests in [080](080-testing-and-fixtures.md#compatibility-tests)
must be represented there with a tier, Rust decision, links, and owner bead.
