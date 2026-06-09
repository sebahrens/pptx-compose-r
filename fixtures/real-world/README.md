# Real-World Consulting Fixtures

Publicly-published consulting / strategy decks, downloaded verbatim for round-trip and
stress testing. Unlike the synthetic and `source-family` fixtures, these are **real decks
authored in Microsoft PowerPoint by third parties**: dozens of slides, embedded chart
workbooks, heavy media, external hyperlinks, and non-ASCII metadata — the kind of
complexity the spec's round-trip and preservation invariants must survive.

All five parse and validate cleanly with the project CLI
(`pptx-compose validate --report -`): `status = valid`, `errors = 0`. None contain VBA
macros or encryption.

## Corpus

| File | Source | Authored in | Slides | Charts | Media | Embedded workbooks | Validate warnings |
|------|--------|-------------|-------:|-------:|------:|-------------------:|-------------------|
| `worldbank-cpf-concept-note.pptx` | World Bank | PowerPoint 16 | 26 | 44 | 14 | 10 | 1 × `external_relationship_not_checked` |
| `worldbank-macro-economic-update.pptx` | World Bank | PowerPoint 16 | 22 | 36 | 15 | 0 | 9 × `external_relationship_not_checked` |
| `worldbank-smart-rwanda-roadshow.pptx` | World Bank | PowerPoint 14 | 24 | 0 | 63 | 0 | none |
| `oecd-economic-outlook-2017.pptx` | OECD | PowerPoint 14 | 23 | 2 | 31 | 1 | none |
| `rsm-technology-strategy.pptx` | RSM (via E3M) | PowerPoint 14 | 12 | 0 | 14 | 0 | 1 × `external_relationship_not_checked` |

### Why each was kept

- **`worldbank-cpf-concept-note.pptx`** — richest chart fixture: 44 charts backed by 10
  embedded `.xlsx` workbooks under `ppt/embeddings/`. Exercises chart-part + embedded-OLE
  preservation at scale.
- **`worldbank-macro-economic-update.pptx`** — 36 charts and 9 external relationships;
  good for the `external_relationship_not_checked` validation path.
- **`worldbank-smart-rwanda-roadshow.pptx`** — image-heavy (63 media parts, no charts) and
  carries a non-ASCII (`PowerPoint 프레젠테이션`, Korean) `dc:title`, exercising Unicode
  metadata round-tripping.
- **`oecd-economic-outlook-2017.pptx`** — dense economic-outlook deck mixing charts, media,
  and one embedding; representative consulting/economics layout.
- **`rsm-technology-strategy.pptx`** — smaller (12-slide) strategy deck from a professional
  services firm; a lighter-weight real-world baseline.

## Localized (machine-translated) variants

Ten derived decks — German (`-de`) and French (`-fr`) for each of the five source
decks — are retained as stale real-world translation evidence. They are wired into
`fixtures/manifest.toml` with `localized` and `localized-stale-evidence` features
and still round-trip byte-exact clean (same `expected_warnings` as their source),
but they must not be treated as complete V1 translation-fidelity proof.

What the current files prove: older V1 run-scoped `replace_text` generations can
survive clean no-edit persistence on large real-world decks. They do not prove that
the full V1-supported visible text surface is translated.

Regeneration requirements before removing `localized-stale-evidence`:

- Use selector-ready guarded edits from `inspect --detail full` and/or
  `find-text`, including `match` evidence and element/run `text_hash` guards
  where the current CLI can validate them. Do not use bare `element_id` run
  selectors as translation evidence.
- Translate all V1-supported visible text classes: `shape`/`text_box` runs,
  supported table cells, supported notes text, and chart/SmartArt visible text
  only when V1 exposes selectors that keep chart XML/workbook or SmartArt
  data/cache consistency intact.
- Run `ralph-scripts/translation_fidelity.py` against every source/locale pair
  and document any remaining supported-but-untranslated findings separately from
  truly unsupported authoring surfaces.

Current supported-but-untranslated failures:

- **Supported chart text** remains unchanged in chart-heavy decks where V1 could
  not yet prove safe chart XML/workbook label synchronization for the old
  generation.
- **Supported SmartArt text** must be checked as data plus rendered drawing mirror;
  stale mirror findings are release-blocking regeneration failures.
- **Supported table-cell text** was not included by the old translation pipeline.
- **In-run line breaks are flattened to a space** (e.g. the agenda block on
  `worldbank-cpf-*` slide 4): old `run_scoped` generations could not emit
  `<a:br/>`, so multi-line runs render as one wrapped line (`pptx-compose-t6pa`).

Unsupported/preserve-only classes are tracked separately: chart data/value
authoring, SmartArt node/layout/color/structure authoring, embedded workbook data
authoring, media, OLE objects, animations, masters, themes, comments, custom XML,
and other preserve-only package content are not translation targets.

| Files | Lang | replace_text ops |
|-------|------|-----------------:|
| `worldbank-cpf-concept-note-{de,fr}.pptx` | DE / FR | 266 / 266 |
| `worldbank-macro-economic-update-{de,fr}.pptx` | DE / FR | 97 / 99 |
| `worldbank-smart-rwanda-roadshow-{de,fr}.pptx` | DE / FR | 370 / 392 |
| `oecd-economic-outlook-2017-{de,fr}.pptx` | DE / FR | 282 / 283 |
| `rsm-technology-strategy-{de,fr}.pptx` | DE / FR | 72 / 75 |

## Provenance & licensing

These are retained as **black-box test inputs**, not redistributed as original works.
Each is attributed to its publisher below with its original public URL. Verify the
applicable license before any redistribution beyond local testing.

| File | Original URL | Notes |
|------|-------------|-------|
| `worldbank-cpf-concept-note.pptx` | https://consultations.worldbank.org/content/dam/sites/consultations/docs/CPF-Concept-Note-for-consultations-English-Public-Version.pptx | World Bank public consultation document. World Bank content is generally CC BY 4.0. |
| `worldbank-macro-economic-update.pptx` | https://thedocs.worldbank.org/en/doc/4e60fba90606cc94357a5fe7e07641b9-0280032021/original/April-15-Rachel-and-Ha-MEU-launch-slides.pptx | World Bank Macro Economic Update launch slides. |
| `worldbank-smart-rwanda-roadshow.pptx` | https://thedocs.worldbank.org/en/doc/741861434649630055-0190022013/original/HELPRoadshowSmartRwanda3Choi.pptx | World Bank HELP Roadshow (Smart Rwanda). |
| `oecd-economic-outlook-2017.pptx` | https://formatresearch.com/img/file/OCSE/2016/Better-but-not-good-enough-oecd-economic-outlook-presentation-june-2017.pptx | OECD Economic Outlook, June 2017 ("Better, but not good enough"), via formatresearch.com mirror. © OECD. |
| `rsm-technology-strategy.pptx` | https://e3m.org.uk/wp-content/uploads/2016/05/RSM_Embedding-your-technology-strategy.pptx | RSM "Embedding your technology strategy", hosted by E3M. |

### SHA-256

```
1cc9a1130916b147b56cce438cd3a8952f68440696057c1da84e68f6f05e5481  worldbank-cpf-concept-note.pptx
b379de7dce5a5b623597dcb41dac51a4c0ed3415d65d0128aa12b5451804b4d6  worldbank-macro-economic-update.pptx
c76979a1ec16ea693bba6bdef048445f124e79459b4393e44cb7ebafbf575668  worldbank-smart-rwanda-roadshow.pptx
8b7d9767076a17abb26826732d0f8fede168c46b94cf8c3dc9d59a3ae2adde70  oecd-economic-outlook-2017.pptx
df538264185cfa715ec832a17290c9c0bed630f0492fd60b7993213454406aae  rsm-technology-strategy.pptx
```

Localized variants (derived; regenerate if the translation pipeline changes):

```
c62f0050271f8a70d8718760ad4a47c9822565afe4320e3a6edadecab42b17cb  worldbank-cpf-concept-note-de.pptx
078fa1505284af000fe83dccd3f99024d4a74e436afb1f66ad29ffad5c5c5c51  worldbank-cpf-concept-note-fr.pptx
a6306faf6d848e26f51536ef1c4a1ef5468533f2a7fbd5ffd2868b85777b37b2  worldbank-macro-economic-update-de.pptx
0dc341102ff04992359ec2c18cfa17566540f94ffa4d054faabd7740df9666e3  worldbank-macro-economic-update-fr.pptx
7dbf35914ab9072a9ef63c5a07ece6f227f5ce6def31401ae91335149526d7c0  worldbank-smart-rwanda-roadshow-de.pptx
7d749310c7d21f6b111fc10361ead9cda071658391736c402cc5dc2587b75db4  worldbank-smart-rwanda-roadshow-fr.pptx
96fe96a188fa39539023212a6d2ca48fc5810f95eba2f3ff602de982e6c31bcc  oecd-economic-outlook-2017-de.pptx
184435daff6e9809c298ccbfc7aeb8af7b0f3ea8770bdef3906626748b03ad60  oecd-economic-outlook-2017-fr.pptx
0e2669fbf92ddcfef46012ff701e544ac9e3c5dfa73fddde4ed3f1c90a7c394a  rsm-technology-strategy-de.pptx
a727b11e6116cd514b0a14c351af9c0c6263d713661eef031513c783cad3e74e  rsm-technology-strategy-fr.pptx
```
