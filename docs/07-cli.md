# 07 — CLI

`src/cli/` contains the clap definitions and one file per subcommand under
`src/cli/commands/`. Every subcommand is a thin function that:

1. Opens `MERGESMITH_HOME` and the SQLite store.
2. Does its thing.
3. Exits with an appropriate exit code.

## Subcommands

```
mergesmith [tui]                  Open the animated TUI (default)
mergesmith init                   Register the current repo
mergesmith enqueue [--target <branch>] [--message "..."]
                                  Queue the current worktree
mergesmith status [--repo <id|name>] [--format human|json]
                                  Print queue contents
mergesmith cancel <id>            Cancel a queued OR in-flight entry
mergesmith retry <id>             Re-enqueue Failed/NeedsHelp entry
mergesmith resolve <id>           Open agent session for NeedsHelp entry
mergesmith logs <id>              Print CI + merge logs for an entry
mergesmith repos list             List registered repos
mergesmith repos edit <id|name>   $EDITOR a TOML stub, write back
mergesmith repos remove <id|name> Deregister (entries cascade-delete)
mergesmith doctor                 Sanity checks
```

## Exit codes

```
0   success
1   user error (invalid args, bad input)
2   environment error (Kitty unsupported, missing tool, etc.)
3   state error (no repo registered here, entry doesn't exist, etc.)
4   busy (another TUI is running, claim contention)
10  internal error (panic-equivalent at boundary)
```

## Hard preconditions worth refusing early

- `mergesmith init` inside a path that's not a git repo → exit 1 with a
  helpful message.
- `mergesmith enqueue` outside any registered repo → exit 3 with a hint
  to run `init`.
- `mergesmith enqueue` while on the target branch (i.e. `HEAD` == default
  branch) → exit 1. Can't merge `main` into `main`.
- `mergesmith enqueue` with a dirty source worktree → exit 1 with a hint
  to commit or stash.
- `mergesmith tui` in a non-Kitty terminal → exit 2 with the supported-
  terminals list.
- `mergesmith tui` when the singleton lock is held → exit 4.

## `mergesmith tui` startup

1. Acquire PID-file flock; refuse if held.
2. Open store; run migrations; run crash-recovery sweep.
3. Detect Kitty graphics support; refuse if absent.
4. Build `AgentRegistry`.
5. `EnginePool::spawn` (one worker thread per registered repo).
6. Run TUI render loop.
7. On exit: `EnginePool::shutdown_graceful(timeout)`, drop flock, exit.

## `mergesmith repos edit`

Generates a TOML stub from the current repo row, writes to a tempfile,
opens `$EDITOR` (fallback `$VISUAL`, fallback `vi`). On editor exit
status 0, re-parses the TOML and updates the row. On parse error,
prompts to re-edit.

## `mergesmith doctor`

```
[✓] state.sqlite reachable at /Users/cosmin/Library/.../state.sqlite
[✓] schema version 1 matches binary version 1
[✓] git found at /opt/homebrew/bin/git (version 2.45.0)
[✓] tmux found at /opt/homebrew/bin/tmux (version 3.4)
[✓] terminal supports Kitty graphics protocol (TERM_PROGRAM=ghostty)
[⚠] no repos registered — run `mergesmith init` inside a repo
[✓] opencode found at /opt/homebrew/bin/opencode
```

Exit 0 if no `[✗]`, else exit 2.
