# 033. Layouts, Masters, and Themes

Layouts, masters, and themes are required for deck fidelity, but most of their semantics can remain preserve-only in V1.

## Requirements

- Preserve all slide layout parts.
- Preserve all slide master parts.
- Preserve theme parts.
- Resolve each slide's layout relationship.
- Expose layout/master/theme part names for context.
- Do not attempt full inherited style computation in V1.

## Post-V1: Adding Slides

Slide creation is not a V1 requirement. When slide creation is added post-V1, the implementation must link it to a valid layout. Acceptable strategies include:

- Clone a template slide and edit the clone.
- Create a minimal slide linked to an existing blank/title layout.
- Require the caller to specify a layout selector.

## Guardrails

- Do not delete unused layouts/masters/themes in V1.
- Do not rewrite layout/master/theme XML unless explicitly edited.
- Do not break placeholder references.
