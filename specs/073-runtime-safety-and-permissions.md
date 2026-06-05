# 073. Runtime Safety and Permissions

Agent-facing CLI and MCP surfaces introduce host filesystem and session risks beyond PPTX parsing. This spec defines default safety policies.

## Filesystem Permission Model

- Runtime wrappers must default to an allowlisted workspace/root.
- Input paths, output paths, media paths, and temp paths are normalized before permission checks.
- Symlinks must be resolved or rejected according to the configured policy before access.
- `~`, relative paths, and platform-specific path aliases must not bypass allowlists.
- Commands/tools must reject reads and writes outside allowed roots by default.

## Declared Write Set

A command or tool may write only:

- explicit output paths,
- files under `--workspace`,
- temporary files under `--temp-dir`,
- stdout/stderr or MCP tool responses/resources.

No wrapper may create extraction directories, cache files, reports, or sibling files unless explicitly requested.

## Overwrite and Atomicity

- Existing output files are not overwritten unless `overwrite: true` or `--overwrite` is explicit.
- Input files are never modified unless an explicit in-place mode is used.
- In-place mode must create a backup unless `--no-backup` is explicit.
- PPTX writes use temporary files and atomic rename where supported.
- Partial temp files are cleaned up on failure unless debug `--keep-temp` is set.

## Destructive Operation Policy

- `apply_patch` is destructive only to in-memory/session state until export.
- `export`/`write` is destructive to filesystem only if output path exists and overwrite is permitted.
- Raw XML replacement is disabled by default for MCP and omitted from default CLI agent docs.
- Unsafe validation bypass is not exposed in default agent workflows.

## Sensitive Content Policy

- Do not fetch external relationships.
- Do not execute macros, OLE objects, embedded scripts, or embedded packages.
- Do not expose embedded files' bytes unless explicitly requested and permitted.
- Digitally signed packages must warn that edits invalidate signatures.
- Macro-enabled packages may preserve opaque macro parts but must not execute them.

## Audit Information

Patch/export reports must include:

- operation count,
- changed parts,
- media added/replaced,
- validation status,
- warnings,
- output path or resource,
- whether state changed,
- transaction/request ID where available.
