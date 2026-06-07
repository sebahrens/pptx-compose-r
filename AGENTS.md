# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git
> remote — separate from `refs/heads/*` where your code lives.
> `.beads/issues.jsonl` is a passive export, not the wire protocol.
>
> See [SYNC_CONCEPTS.md](https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the
> source of truth; don't `bd import` during normal operation; don't
> reach for third-party Dolt hosting before trying the default).

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Using the pptx-compose binaries

The Rust agent-facing binaries are `pptx-compose` for CLI workflows and
`pptx-compose-mcp` for the MCP server. The normative contracts are
[`specs/071-cli-agent-contract.md`](specs/071-cli-agent-contract.md) and
[`specs/072-mcp-server-contract.md`](specs/072-mcp-server-contract.md); check
those specs before inventing command shapes or MCP tool behavior.

For CLI edits, use the bounded V1 agent flow:

```bash
pptx-compose --version
pptx-compose inspect INPUT.pptx --output deck.view.json --report inspect.report.json --json-errors
pptx-compose find-text INPUT.pptx "QUERY" --slides slide-N --limit N --json-errors
pptx-compose apply INPUT.pptx PATCH.json --dry-run --media-manifest media.json --report dry-run.report.json --diff diff.json --json-errors
pptx-compose apply INPUT.pptx PATCH.json --media-manifest media.json --output OUTPUT.pptx --report apply.report.json --json-errors
pptx-compose validate OUTPUT.pptx --report validation.json --json-errors
```

`inspect`, `validate`, and `apply --dry-run` are read-only. `apply --output`
is the CLI export/write step; it must validate edited output before the final
write by default and must not overwrite existing files unless `--overwrite` is
explicit. `find-text` may be scoped with `--slides slide-N`; its matches carry
selector guards (`slide_id`, `kind`, `part`, `text_hash`, `fingerprint`) that
paste directly into a `replace_text` operation.

CLI discipline: primary machine JSON goes to stdout or the explicit output path;
human progress and logs go to stderr; commands must not mix prose into JSON
streams. Use `--json-errors` for automation: failed commands then emit exactly
one JSON error envelope to stderr. Exit codes are coarse buckets, so branch on
the stable `error.code` field from the JSON envelope, not on the exit code alone.

Launch the MCP server over stdio with:

```bash
pptx-compose-mcp --workspace DIR --temp-dir DIR
```

The MCP flow is `pptx_open` -> scoped inspection tools such as
`pptx_get_document_summary`, `pptx_list_slides`, `pptx_get_slide`,
`pptx_find_text`, `pptx_list_elements`, and `pptx_get_element` ->
`pptx_validate_patch` -> `pptx_apply_patch` -> `pptx_export` -> `pptx_close`.
Mutating MCP tools require `session_id` plus `expected_revision` or a patch
`base_revision`; stale revisions return the `stale_patch` error code. Stage
media through `pptx_import_media` handles. Unsupported raw package/XML mutation
is outside the V1 agent surface; refuse unsupported edits rather than modifying
package internals directly.

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
