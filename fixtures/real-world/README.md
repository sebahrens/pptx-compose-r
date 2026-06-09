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
decks — generated **through the V1 agent edit surface** as a real-world exercise of
`inspect` → run-scoped `replace_text` → `apply` → `validate`. They are wired into
`fixtures/manifest.toml` with the `localized` feature and round-trip byte-exact clean
(same `expected_warnings` as their source).

How they were produced: every editable `shape`/`text_box` run was translated and
replaced with `mode: run_scoped`, preserving paragraph auto-numbering, bullets,
and per-run bold/colour. Regeneration should use selector guards copied from the
agent view, including element `text_hash`/`fingerprint` and selected-run
`text_hash` when present; related SmartArt text additionally requires a safe
cache mapping proof rather than bare numeric paragraph/run selectors.

Known untranslated content (current V1 engine limitations, tracked in beads — do not
treat as translation bugs):

- **SmartArt diagrams and charts** keep their original-language text: that text lives in
  `ppt/diagrams/data*.xml` and chart XML / embedded workbooks and is **not text-editable
  in V1** (release blocker `pptx-compose-9x09`).
- **Table cells** are not translated (cell-level editing not yet wired into this pipeline).
- **In-run line breaks are flattened to a space** (e.g. the agenda block on
  `worldbank-cpf-*` slide 4): `run_scoped` cannot emit `<a:br/>` and `U+000B` is illegal
  XML, so multi-line runs render as one wrapped line (`pptx-compose-t6pa`).

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
e097250aba583a730ba343bbf63545dba25b7591df06dab5e94f537a74d7257b  worldbank-cpf-concept-note-de.pptx
eb52f1dfd84195aefbe195b20e8c317e4f272a34959de1308be6b1eb369ffc11  worldbank-cpf-concept-note-fr.pptx
5e2fd3a9530ab61e01f1daaa8494c5b42f0b8553b98f983dd42975032e6a4ab4  worldbank-macro-economic-update-de.pptx
c24f5fbf9eca56fdddba51ff52fc11c6c790a9a2ed31db07d1601499bd14c904  worldbank-macro-economic-update-fr.pptx
4833dfa6c75522d0d709a952588782520ac6faf231dd0d6b25a056fe494a36a1  worldbank-smart-rwanda-roadshow-de.pptx
83296d386fe7017bff2cdaf768c1ec6d935d16a8efc6082daa961a69d9f0681e  worldbank-smart-rwanda-roadshow-fr.pptx
4f2bda5a5dfc24848e3951d88e7a7a79fb32d1a6103b877ee0da09d88b191db4  oecd-economic-outlook-2017-de.pptx
46c71db9f21d1d3c904ba06f186db3267c4ed1452e87aad7630b68ac2af1b043  oecd-economic-outlook-2017-fr.pptx
6eb1e29ccf23a08becdbf6cef37dc9a005ace19dbe735cb2908b94c42cb1423a  rsm-technology-strategy-de.pptx
d0e5d5037b1abf38a42a96e4cf2c7a6197666b6fdace0a1da870de70143a626f  rsm-technology-strategy-fr.pptx
```
