export const meta = {
  name: 'editability-v1-expansion',
  description: 'Catalogue all editable PPTX features, debate Rust implementation strategy, extend specs, add E2E+visual tests, emit atomic beads',
  phases: [
    { title: 'Discover' },
    { title: 'Catalogue' },
    { title: 'Debate' },
    { title: 'Author' },
  ],
}

const REPO = '/Users/seb/projects/pptx-compose'

const ORIENT = `
Repo: ${REPO} (cleanroom Rust rewrite of a PPTX engine for AI-agent round-tripping).
Authoritative requirements live in specs/ (read SPEC.md index). Key specs:
  - specs/001-goals-and-scope.md   (current V1 scope + preserve-only + non-goals)
  - specs/030-pptx-presentation-model.md, 031-slides-shapes-and-text.md, 032-media-images.md, 033-layouts-masters-themes.md
  - specs/040-agent-json-format.md, 041-agent-edit-operations.md, 042-agent-protocol-schemas.md, 047-drawingml-construction.md
  - specs/060-rust-architecture.md, 050-roundtrip-invariants.md, 046-provenance-and-hashing.md, 080-testing-and-fixtures.md
Rust crates: crates/pptx-compose-core (OPC/XML/pptx model, provenance), -json (agent_view), -edit (operations), -cli, -mcp.
Today V1 edit ops (capabilities): replace_text, add_text_box, move_resize_element, set_alt_text, add_image, replace_image.
replace_text ONLY accepts ElementKind::TextBox|Shape (crates/pptx-compose-edit/src/operations/replace_text.rs:135). graphicFrame (tables/charts/SmartArt) and grpSp are rejected with unsupported_edit.
"Preserve-only" in V1 today: charts+workbooks, tables beyond detection, SmartArt, animations/transitions, comments, notes, masters/layouts/themes, custom XML, OLE.
Core invariant: preserve unmodified XML/binary bytes; never corrupt; prefer unsupported_edit over corrupt output.
Use the coderlm CLI (.claude/coderlm_state/coderlm_cli.py) and Read/Grep to ground claims in real files; cite file:line. Bring real OOXML/ECMA-376 knowledge (a:tbl, dsp/dgm diagrams, c:chart, a:rPr/a:pPr, a:hlinkClick, p:notes, etc.).
`

const FINDINGS_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['domain', 'features', 'cross_cutting_notes'],
  properties: {
    domain: { type: 'string' },
    features: {
      type: 'array',
      items: {
        type: 'object', additionalProperties: false,
        required: ['name', 'ooxml_location', 'description', 'current_support', 'edit_complexity', 'v1_recommendation', 'rationale', 'preservation_risk'],
        properties: {
          name: { type: 'string' },
          ooxml_location: { type: 'string', description: 'part + element path, e.g. ppt/slides/slideN.xml a:tbl/a:tr/a:tc/a:txBody' },
          description: { type: 'string' },
          current_support: { enum: ['none', 'preserve_only', 'partial', 'full'] },
          edit_complexity: { enum: ['low', 'medium', 'high', 'very_high'] },
          v1_recommendation: { enum: ['v1_core', 'v1_stretch', 'defer_post_v1', 'preserve_only'] },
          rationale: { type: 'string' },
          preservation_risk: { type: 'string', description: 'what could corrupt or break byte-preservation if edited naively' },
        },
      },
    },
    cross_cutting_notes: { type: 'string' },
  },
}

const CATALOGUE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['features', 'proposed_v1_line', 'open_questions', 'gaps_found'],
  properties: {
    features: {
      type: 'array',
      items: {
        type: 'object', additionalProperties: false,
        required: ['name', 'domain', 'ooxml_location', 'current_support', 'edit_complexity', 'recommendation'],
        properties: {
          name: { type: 'string' }, domain: { type: 'string' }, ooxml_location: { type: 'string' },
          current_support: { enum: ['none', 'preserve_only', 'partial', 'full'] },
          edit_complexity: { enum: ['low', 'medium', 'high', 'very_high'] },
          recommendation: { enum: ['v1_core', 'v1_stretch', 'defer_post_v1', 'preserve_only'] },
          notes: { type: 'string' },
        },
      },
    },
    proposed_v1_line: { type: 'string', description: 'where to draw V1 vs deferred, and why' },
    open_questions: { type: 'array', items: { type: 'string' } },
    gaps_found: { type: 'array', items: { type: 'string' }, description: 'editable things no discovery agent covered, or contradictions' },
  },
}

const DEBATE_TURN_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['position', 'key_arguments', 'concessions'],
  properties: {
    position: { type: 'string' },
    key_arguments: { type: 'array', items: { type: 'string' } },
    concessions: { type: 'array', items: { type: 'string' } },
  },
}

const VERDICT_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['recommended_strategy', 'v1_operation_set', 'deferred_set', 'op_model_decision', 'crate_layout_notes', 'phasing', 'key_risks', 'test_strategy'],
  properties: {
    recommended_strategy: { type: 'string' },
    v1_operation_set: { type: 'array', items: { type: 'object', additionalProperties: false, required: ['op', 'targets', 'rationale'], properties: { op: { type: 'string' }, targets: { type: 'string' }, rationale: { type: 'string' } } } },
    deferred_set: { type: 'array', items: { type: 'string' } },
    op_model_decision: { type: 'string', description: 'generic vs per-type ops; selector/guard model; formatting-preservation approach; raw-bytes preservation strategy' },
    crate_layout_notes: { type: 'string' },
    phasing: { type: 'array', items: { type: 'object', additionalProperties: false, required: ['phase', 'items'], properties: { phase: { type: 'string' }, items: { type: 'array', items: { type: 'string' } } } } },
    key_risks: { type: 'array', items: { type: 'string' } },
    test_strategy: { type: 'string' },
  },
}

const BEADS_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['beads'],
  properties: {
    beads: {
      type: 'array',
      items: {
        type: 'object', additionalProperties: false,
        required: ['key', 'title', 'type', 'priority', 'description', 'acceptance', 'depends_on'],
        properties: {
          key: { type: 'string', description: 'local handle for wiring deps, e.g. t1' },
          title: { type: 'string' },
          type: { enum: ['feature', 'task', 'bug'] },
          priority: { type: 'integer', minimum: 0, maximum: 4 },
          description: { type: 'string', description: 'why + what + concrete code/spec refs' },
          acceptance: { type: 'string' },
          depends_on: { type: 'array', items: { type: 'string' }, description: 'keys of beads that block this one (task->task only, no epics)' },
        },
      },
    },
  },
}

// ---------------- Phase 1: Discover ----------------
phase('Discover')
const DOMAINS = [
  { key: 'surface-map', label: 'current-impl-surface', prompt: `Produce the AUTHORITATIVE map of what pptx-compose supports TODAY. Read crates/pptx-compose-edit/src/operations/*, crates/pptx-compose-json/src/agent_view/*, crates/pptx-compose-core/src/pptx/* and the capabilities output. Enumerate: every edit operation and the exact element kinds it accepts/rejects; every field the agent view exposes per element; which OOXML constructs are parsed vs copied-through. This is the baseline other findings must agree with. Treat each supported/unsupported capability as a "feature" with accurate current_support.` },
  { key: 'text-format', label: 'text-and-rich-formatting', prompt: `Domain: text content & rich formatting inside sp/txBody. Cover run properties (a:rPr: bold/italic/underline/strike/color/font/size/caps/spacing/baseline), paragraph properties (a:pPr: align/indent/marL/lvl/line+space-before/after), bullets & numbering (a:buChar/a:buAutoNum/a:buNone), hyperlinks (a:hlinkClick/hlinkMouseOver + rels), fields (a:fld slide number/date), language/dirty flags, line breaks (a:br) vs paragraphs. For each: is it editable today, complexity, V1 recommendation. Note the known whole_element/formatting_simplified limitation.` },
  { key: 'tables', label: 'tables', prompt: `Domain: PowerPoint tables (graphicFrame > a:tbl). Cover cell text edit, add/delete row/column, merge/split cells (gridSpan/rowSpan/hMerge/vMerge), column width / row height, table style (a:tblPr + tableStyleId), cell fill/borders/margins, header row. Note tables are not exposed in the agent view today and replace_text rejects graphicFrame.` },
  { key: 'smartart', label: 'smartart-diagrams', prompt: `Domain: SmartArt / DrawingML diagrams. graphicFrame referencing ppt/diagrams/data#.xml (dgm:* point text in a:txBody), plus layout#, colors#, quickStyle#, and the dsp drawing part (ppt/drawings/drawing#.xml or diagram drawing). Cover: edit node text, add/remove/reorder nodes, the data-model<->drawing consistency problem. Realistically assess complexity and whether data-only text edit is feasible while keeping the cached drawing valid.` },
  { key: 'charts', label: 'charts', prompt: `Domain: charts (graphicFrame > c:chart in ppt/charts/chart#.xml + embedded workbook in ppt/embeddings/*.xlsx). Cover: chart title, axis titles, data labels, series names (c:tx), category/series text (c:cat/c:val numCache/strCache), changing data values, legend text. Critical: the cached values in chart XML vs the embedded xlsx source of truth. Assess what text edits are safe without rewriting the workbook.` },
  { key: 'shape-style', label: 'shape-geometry-and-style', prompt: `Domain: shape geometry & visual style. Cover fill (solidFill/gradFill/blipFill/noFill), line (a:ln color/width/dash), effects (shadow/glow), preset geometry + adjustments (prstGeom/avLst), rotation/flip (already partly via move_resize?), z-order reordering within spTree, grouping/ungrouping, connectors (cxnSp) and their endpoints. For each: editable today? complexity? V1 line?` },
  { key: 'media', label: 'media-and-embeds', prompt: `Domain: media & embeds. Cover image add/replace (exist today) plus crop (a:srcRect)/recolor/duotone, picture fill, audio/video (p:pic with media + a:videoFile/audioFile rels), OLE objects (p:oleObj), slide background image, alt text (exists). Assess what is editable vs preserve-only for V1.` },
  { key: 'structure', label: 'deck-and-slide-structure', prompt: `Domain: deck & slide structure. Cover add/duplicate/delete/reorder slides (sldIdLst + rels + cleanup), sections (p:sectionLst), slide size (sldSz), presentation-level defaults, layouts/masters/themes edits, background, headers/footers, slide-number/date/footer placeholders. These are largely post-V1 today (non-goal) — assess which are low-risk enough to pull into an expanded V1 vs keep deferred.` },
  { key: 'aux', label: 'notes-comments-metadata', prompt: `Domain: auxiliary editable content. Cover speaker notes (ppt/notesSlides/notesSlide#.xml text), comments (modern p188 + legacy), document metadata (docProps/core.xml dc:title/creator/subject/keywords, app.xml, custom.xml), deck-level hyperlinks/actions, animations/transitions (almost certainly preserve-only), custom XML parts. Assess V1 line for each.` },
]
const findings = await parallel(DOMAINS.map(d => () =>
  agent(`${ORIENT}\n\nYou are a senior OOXML+Rust engineer doing a discovery pass.\n${d.prompt}\n\nReturn structured findings. Be exhaustive within your domain; every distinct editable thing is its own feature entry with an accurate current_support grounded in the actual code (cite file:line where you verified). Do NOT modify any files.`,
    { label: `discover:${d.label}`, phase: 'Discover', schema: FINDINGS_SCHEMA, agentType: 'Explore' })
))
const findingsClean = findings.filter(Boolean)
log(`Discovery complete: ${findingsClean.length}/${DOMAINS.length} domains, ${findingsClean.reduce((n, f) => n + (f.features?.length || 0), 0)} feature entries`)

// ---------------- Phase 2: Catalogue ----------------
phase('Catalogue')
const catalogue = await agent(
  `${ORIENT}\n\nYou are the lead architect. Below are discovery findings from ${findingsClean.length} domain experts as JSON. Merge them into ONE authoritative editability catalogue: dedupe overlapping features, reconcile any disagreements about current_support against the surface-map findings, and flag gaps (editable things nobody covered) and contradictions. Then propose where the V1 line should sit (what becomes editable in an expanded V1 vs explicitly deferred), honoring the core invariant (preserve unmodified bytes; prefer unsupported_edit over corruption). Findings JSON:\n\n${JSON.stringify(findingsClean)}`,
  { label: 'catalogue:merge', phase: 'Catalogue', schema: CATALOGUE_SCHEMA })

// ---------------- Phase 3: Debate ----------------
phase('Debate')
const ctx = `${ORIENT}\n\nEDITABILITY CATALOGUE (authoritative):\n${JSON.stringify(catalogue)}`
const advocate1 = await agent(
  `${ctx}\n\nYou are Senior Rust Dev A, the AMBITIOUS ADVOCATE. Argue for the most capable expanded-V1 editing strategy that is still safe: which operations to add, a unifying op/selector model, how to preserve unmodified bytes while editing typed sub-trees (tables/diagrams/charts/formatting), crate boundaries, and how to keep the agent view honest. Push for breadth where the risk is manageable. Be concrete about Rust design (where ops live, how the typed-edit-over-preserved-XML works, provenance/guards).`,
  { label: 'debate:advocate-r1', phase: 'Debate', schema: DEBATE_TURN_SCHEMA })
const devil1 = await agent(
  `${ctx}\n\nYou are Senior Rust Dev B, the DEVIL'S ADVOCATE. Dev A argued:\n${JSON.stringify(advocate1)}\n\nRebut hard. Attack scope creep, byte-preservation hazards (chart cache vs xlsx, SmartArt data-vs-drawing, table style inheritance), maintenance cost, validation gaps, and the risk of corrupt output. Argue for the smallest defensible expansion and a strict phasing. Identify which of A's proposals are genuinely unsafe for V1 and why.`,
  { label: 'debate:devil-r1', phase: 'Debate', schema: DEBATE_TURN_SCHEMA })
const advocate2 = await agent(
  `${ctx}\n\nDebate round 2. Your round-1 position (Dev A):\n${JSON.stringify(advocate1)}\nDev B rebutted:\n${JSON.stringify(devil1)}\n\nRespond: concede what's genuinely unsafe, defend what's worth keeping, and refine toward a strategy you'd both sign off on. Be specific about the safe subset and the sequencing.`,
  { label: 'debate:advocate-r2', phase: 'Debate', schema: DEBATE_TURN_SCHEMA })
const verdict = await agent(
  `${ctx}\n\nYou are the PRINCIPAL ENGINEER acting as judge. The debate transcript:\nA-r1: ${JSON.stringify(advocate1)}\nB-r1: ${JSON.stringify(devil1)}\nA-r2: ${JSON.stringify(advocate2)}\n\nSynthesize the FINAL implementation strategy. Decide the concrete expanded-V1 operation set (with target element kinds), what stays deferred, the op/selector/guard model, formatting & byte preservation approach, crate layout, an ordered phasing plan, the key risks, and the test strategy (incl. edit round-trip + visual QA). This verdict drives the spec edits, beads, and tests.`,
  { label: 'debate:verdict', phase: 'Debate', schema: VERDICT_SCHEMA })
log(`Debate verdict: V1 op set of ${verdict.v1_operation_set.length}, ${verdict.phasing.length} phases, ${verdict.deferred_set.length} deferred`)

// ---------------- Phase 4: Author ----------------
phase('Author')
// 4a: spec writer (single agent, writes all spec files sequentially to avoid races)
const specSummary = await agent(
  `${ORIENT}\n\nCATALOGUE:\n${JSON.stringify(catalogue)}\n\nVERDICT (authoritative implementation strategy):\n${JSON.stringify(verdict)}\n\nYou are the spec editor. Extend the specs so they EXPLICITLY cover everything editable. Concretely:\n1) Rewrite the V1 scope section of specs/001-goals-and-scope.md so every editable feature class is explicitly listed as either in-V1 (per the verdict) or deferred-with-reason; update the Preserve-Only and Non-Goals sections to match the expanded scope.\n2) Create a NEW spec file specs/048-editability-catalogue.md: the exhaustive editability catalogue (table of feature, OOXML location, V1 status, rationale, preservation risk).\n3) Extend specs/041-agent-edit-operations.md with the new operations from the verdict (required/optional fields, guards, newline/formatting policy, unsupported_edit conditions), consistent with the existing patch envelope and selector/guard model.\n4) Update the SPEC.md index and any reading-order list to include 048.\nKeep the existing house style and cleanroom rules. Use the Edit/Write tools. Do NOT touch code or run bd. When done, return a concise manifest of files written and the key scope decisions encoded.`,
  { label: 'author:specs', phase: 'Author' })

// 4b + 4c run in parallel: tests author writes files; bead author returns JSON only.
const EXISTING_BEADS = `Already-filed beads (reference/avoid duplicating; new beads may depend on or refine these):
pptx-compose-3vug P1 view over-reports text-editability for graphicFrame/group; pptx-compose-egij P2 edit table cells; pptx-compose-83gr P2 edit SmartArt; pptx-compose-nr9w P2 edit grouped-shape text; pptx-compose-73ac P1 inspect guard hashes rejected by apply; pptx-compose-5iyh P2 inspect --detail full empty without --slides; pptx-compose-md5g P2 dry-run aborts no report; pptx-compose-uk97 P3 --slides rejects slide-N ids; pptx-compose-z5hj P3 run-preserving text replace; pptx-compose-m51j P4 output-path error message.`

const [testSummary, beadsObj] = await Promise.all([
  agent(
    `${ORIENT}\n\nVERDICT:\n${JSON.stringify(verdict)}\n\nYou are the test engineer. Add E2E tests WITH visual QA that exercise the NEW edit functionality (not just no-edit round-trip). Study ralph-scripts/pptx_roundtrip_e2e.py (the existing harness: input->compat JSON->pptx, compares xml/media/visual/validation, optional LibreOffice+pdftoppm visual stage) and ralph-scripts/tests/. Add an EDIT round-trip path: apply a representative patch (per new V1 op) to a fixture, then validate the output, assert the targeted text/structure changed while unrelated parts stay byte-identical, and run the visual stage (render to images via soffice+pdftoppm) so layout regressions are caught. Wire it so it degrades gracefully when LibreOffice is absent (mark visual inconclusive, per the known synthetic-fixture gotcha). Prefer extending the existing harness + adding a pytest in ralph-scripts/tests/. Use real fixtures in fixtures/real-world and fixtures/. Use Edit/Write; you MAY run the new test to sanity-check it parses/imports, but do NOT run bd or git. Return a manifest of files added/changed and how to run the new tests.`,
    { label: 'author:tests', phase: 'Author' }),
  agent(
    `${ORIENT}\n\nCATALOGUE:\n${JSON.stringify(catalogue)}\n\nVERDICT:\n${JSON.stringify(verdict)}\n\n${EXISTING_BEADS}\n\nYou are the planner. Produce a set of ATOMIC beads implementing the verdict's expanded-V1 editing scope and its phasing. Rules: each bead is independently shippable and testable; small (a few files); has a clear title, why+what description with concrete code/spec refs (crate paths, replace_text.rs:135, agent_view, specs/041/048), and acceptance criteria. Order with depends_on using bead keys (task->task blocking only — NO epics, NO parent-child). Cover: agent-view exposure changes, each new edit op, validation rules, schema/capabilities updates, and the new tests. Where a new bead supersedes/extends an already-filed one, say so in the description (do not duplicate). Do NOT run bd — return the beads as data only.`,
    { label: 'author:beads', phase: 'Author', schema: BEADS_SCHEMA }),
])

return {
  discovery_domains: findingsClean.length,
  catalogue_features: catalogue.features.length,
  catalogue_gaps: catalogue.gaps_found,
  open_questions: catalogue.open_questions,
  verdict,
  spec_changes: specSummary,
  test_changes: testSummary,
  beads: beadsObj?.beads || [],
}
