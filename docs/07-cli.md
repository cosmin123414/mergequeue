# 07 — CLI

`src/cli/` contains the clap definitions and one file per subcommand under
`src/cli/commands/`. Every subcommand is a thin function that:

1. Opens `MERGEQUEUE_HOME` and the SQLite store.
2. Does its thing.
3. Exits with an appropriate exit code.

## Subcommands

```
mergequeue [tui]                  Open the animated TUI (default)
mergequeue init                   Register the current repo
mergequeue enqueue [--target <branch>] [--message "..."]
                                  Queue the current worktree
mergequeue status [--repo <id|name>] [--format human|json]
                                  Print queue contents
mergequeue cancel <id>            Cancel a queued OR in-flight entry
mergequeue retry <id>             Re-enqueue Failed/NeedsHelp entry
mergequeue resolve <id>           Open agent session for NeedsHelp entry
mergequeue logs <id>              Print CI + merge logs for an entry
mergequeue repos list             List registered repos
mergequeue repos edit <id|name>   $EDITOR a TOML stub, write back
mergequeue repos remove <id|name> Deregister (entries cascade-delete)
mergequeue doctor                 Sanity checks
```

## Exit codes

```
0   success
1   user error (invalid args, bad input)
2   environment error (not a TTY, missing tool, etc.)
3   state error (no repo registered here, entry doesn't exist, etc.)
4   busy (another TUI is running, claim contention)
10  internal error (panic-equivalent at boundary)
```

## Hard preconditions worth refusing early

- `mergequeue init` inside a path that's not a git repo → exit 1 with a
  helpful message.
- `mergequeue enqueue` outside any registered repo → exit 3 with a hint
  to run `init`.
- `mergequeue enqueue` while on the target branch (i.e. `HEAD` == default
  branch) → exit 1. Can't merge `main` into `main`.
- `mergequeue enqueue` with a dirty source worktree → exit 1 with a hint
  to commit or stash.
- `mergequeue tui` with stdin/stdout not a TTY → exit 2.
- `mergequeue tui` when the singleton lock is held → exit 4.

## `mergequeue tui` startup

1. Acquire PID-file flock; refuse if held.
2. Open store; run migrations; run crash-recovery sweep.
3. Build `AgentRegistry`.
4. `EnginePool::spawn` (one worker thread per registered repo).
5. Run TUI render loop (Braille-glyph visualizer).
6. On exit: `EnginePool::shutdown_graceful(timeout)`, drop flock, exit.

## `mergequeue repos edit`

Generates a TOML stub from the current repo row, writes to a tempfile,
opens `$EDITOR` (fallback `$VISUAL`, fallback `vi`). On editor exit
status 0, re-parses the TOML and updates the row. On parse error,
prompts to re-edit.

## `mergequeue doctor`

```
[✓] state.sqlite reachable at /Users/cosmin/Library/.../state.sqlite
[✓] schema version 1 matches binary version 1
[✓] git found at /opt/homebrew/bin/git (version 2.45.0)
[✓] tmux found at /opt/homebrew/bin/tmux (version 3.4)
[⚠] no repos registered — run `mergequeue init` inside a repo
[✓] opencode found at /opt/homebrew/bin/opencode
```

Exit 0 if no `[✗]`, else exit 2.
