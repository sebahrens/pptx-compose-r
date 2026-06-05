# 003. Cleanroom Process

## Allowed Inputs

The Rust rewrite may use:

- Public README and package metadata.
- Existing public tests and fixtures.
- Black-box behavior observed by running the current package.
- ECMA-376 / ISO OpenXML and Open Packaging Convention documentation.
- Public PPTX examples created by PowerPoint-compatible tools.
- This spec suite.

## Disallowed Practices

- Translating TypeScript source line by line into Rust.
- Copying internal implementation structure just because it exists today.
- Preserving bugs without explicitly labeling them as compatibility requirements.
- Designing around JSZip or `xml2js` internals in Rust-native APIs.

## Behavior Matrix

Every compatibility decision should be labeled:

| Tier | Meaning |
| --- | --- |
| `must-match` | Required for compatibility tests or documented public API. |
| `should-match` | Preferred unless it conflicts with safety or correctness. |
| `intentionally-different` | Changed deliberately; document migration impact. |
| `unsupported` | Not implemented; return clear diagnostics. |

The V1 compatibility decision log lives at
[_planning/compatibility-decisions.md](_planning/compatibility-decisions.md).
Update it before closing any compatibility bead so legacy behavior remains
explicitly classified and separate from core correctness requirements.

## Review Checklist

- Does the spec describe behavior rather than old implementation details?
- Does the Rust API use domain concepts rather than JS library concepts?
- Are compatibility quirks separated from core correctness requirements?
- Are unsafe operations replaced with explicit validation?
- Is unsupported content preserved by default?
