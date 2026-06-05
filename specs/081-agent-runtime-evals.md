# 081. Agent Runtime Evals

Correct crate tests are not enough. The CLI and MCP contracts must be evaluated with realistic agent workflows.

## Eval Corpus Layout

```text
evals/
  cli/
    replace-title/
    add-text-box/
    add-image/
    replace-image/
    stale-patch/
    missing-media/
    unsupported-chart-edit/
  mcp/
    inspect-large-deck/
    patch-after-pagination/
    media-import-add-image/
    validation-error-explain/
```

Each eval includes input PPTX, user instruction, expected patch or refusal, expected transcript shape, expected reports, and output invariants.

## Capability Evals

Agents must complete these using only CLI commands or MCP tools:

1. Inspect a deck and answer slide count/title questions.
2. Find slides containing exact text.
3. Replace a title and write a valid output.
4. Add a text box to a specified slide.
5. Add an image using staged media bytes.
6. Replace an image while preserving the picture element and alt text.
7. Validate a stale patch and explain why it failed.
8. Handle unsupported chart editing without corrupting the deck.
9. Page through a large deck without requesting full-deck JSON.
10. Detect and report malformed input validation failure.

## Negative Evals

- Hallucinated `element_id`.
- Wrong `document_id`.
- Stale `base_revision`.
- Ambiguous selector.
- Missing `media_ref`.
- Media checksum mismatch.
- Declared media type mismatches sniffed bytes.
- Huge image exceeds limit.
- Unsupported chart edit.
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

Maintain fixture-backed transcripts for representative workflows:

```text
replace-title.transcript.json
add-image.transcript.json
stale-revision.transcript.json
unsupported-chart-edit.transcript.json
```

Transcripts test the runtime protocol, not just Rust internals.
