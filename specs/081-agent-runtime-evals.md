# 081. Agent Runtime Evals

Correct crate tests are not enough. The CLI and MCP contracts must be evaluated
with realistic agent workflows and fixture-backed transcripts.

## Eval Corpus Layout

```text
evals/
  cli/
    replace-title/
    add-text-box/
    add-image/
    replace-image/
    find-text-selector/
    stale-revision/
    missing-media/
    unsupported-chart-edit/
  mcp/
    inspect-large-deck/
    patch-after-pagination/
    stale-revision/
    validation-error-explain/
```

Each eval directory contains an `instruction.txt`, an `input-ref.txt` pointing to
a repository fixture, and one or more expected artifacts such as
`expected.transcript.json`, `expected.patch.json`, `expected.refusal.json`,
`patch.json`, and media manifests. The corpus should reference checked-in
fixtures by path instead of copying PPTX binaries into every eval case.

## Capability Evals

Agents must complete these using only CLI commands or MCP tools:

1. Inspect a deck and answer slide count/title questions.
2. Find slides containing exact text and copy guarded selectors into a patch.
3. Replace a title and write a valid output.
4. Replace a rich-text run without simplifying sibling formatting.
5. Add a text box to a specified slide.
6. Add an image using staged media bytes.
7. Replace an image while preserving the picture element and alt text.
8. Validate a stale patch/revision and explain why it failed.
9. Handle unsupported chart data/authoring edits without corrupting the deck.
10. Page through a large deck without requesting full-deck JSON.
11. Detect and report malformed input validation failure.

Existing visible chart and SmartArt text is V1 inspection/editing scope. Negative
evals should say “unsupported chart data/authoring edit” or “unsupported
SmartArt structure/layout edit” instead of implying all visible chart/SmartArt
text must remain unsupported.

## Negative Evals

- Hallucinated `element_id`.
- Wrong `document_id`.
- Stale `base_revision` / stale MCP revision.
- Ambiguous or guard-failed selector.
- `find-text` cursor reused with a different query or scope.
- Missing `media_ref`.
- Media checksum mismatch.
- Declared media type mismatches sniffed bytes.
- Huge image exceeds limit.
- Multiple JSON outputs would resolve to stdout.
- Unsupported chart data/authoring edit.
- Unsupported SmartArt structure/layout authoring edit.
- Signed deck edit warning.
- Macro-enabled deck preserve/no-execute behavior.

## Metrics

Track:

- schema-valid patch rate,
- successful dry-run rate,
- successful apply rate,
- output validates,
- output opens in LibreOffice/PowerPoint-compatible validator where available,
- unchanged parts preserved,
- unsupported content preserved,
- stale patch rejected,
- missing media produces actionable error,
- token/context size of agent views,
- no raw XML mutation required for supported V1 edits.

## Golden Transcripts

Maintain fixture-backed transcripts for representative workflows. The CLI replay
tests accept the standard `expected.transcript.json` name and, where present,
case-specific transcript filenames such as `replace-title.transcript.json`.

```text
replace-title/expected.transcript.json
add-image/expected.transcript.json
find-text-selector/expected.transcript.json
stale-revision/expected.transcript.json
unsupported-chart-edit/expected.transcript.json
```

CLI eval tests replay the transcript against the built binary and validate JSON
outputs, reports, and output invariants. MCP eval tests currently validate corpus
shape and tool argument schemas; runtime session replay is expected follow-up
work, especially for placeholder path substitution and stale-revision behavior.

Transcripts test the runtime protocol, not just Rust internals.
