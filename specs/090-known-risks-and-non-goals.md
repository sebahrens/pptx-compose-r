# 090. Known Risks and Non-Goals

## Major Risks

### Lossy XML Serialization

Rewriting every XML part can corrupt or degrade PPTX fidelity. Mitigation: preserve raw bytes for unmodified XML and only serialize dirty parts.

### Relationship Management

Adding content requires updating relationships and content types. Mitigation: make relationships and content types first-class APIs and validate before write.

### OpenXML Complexity

Charts, SmartArt, animations, layouts, masters, notes, comments, and embedded objects are large domains. Mitigation: preserve unsupported content and implement narrow V1 edits.

### Agent Over-Editing

LLMs can accidentally delete required XML if asked to mutate raw JSON. Mitigation: expose compact agent JSON plus explicit patch operations.

### Fixture Gaps

One sample PPTX is not enough. Mitigation: build a corpus from PowerPoint, LibreOffice, Google Slides, media-heavy decks, chart-heavy decks, and malformed packages.

### Cleanroom Drift

The old TypeScript implementation is small enough to accidentally clone. Mitigation: document behavior, not old implementation structure.

## Hard Non-Goals for V1

- Full PowerPoint rendering.
- Pixel-perfect layout computation.
- Full chart editing.
- Full SmartArt editing.
- Full theme/style inheritance.
- Complete ECMA-376 object model coverage.
- Slide add/duplicate/delete/reorder in V1.
- Network fetching of external relationships.
- Raw JSON mutation as the main editing API.

## Unsupported or Hazardous Package Classes

- Encrypted or password-protected presentations are unsupported and must return a clear error.
- Digitally signed packages may be preserved on no-edit reads, but edits invalidate signatures and must warn or remove/mark invalid signature parts according to a later signing policy.
- Macro-enabled/VBA-containing packages may preserve opaque macro parts but must never execute them.
- External relationships must be preserved but never fetched during parse or validation.

## Guardrails

- Return `unsupported edit` rather than guessing.
- Preserve unknown content.
- Validate every edited package before writing.
- Make all agent-visible operations explicit and reviewable.
