# 11 — Config and paths

## State root resolution

```
1. $MERGEQUEUE_HOME (if set)
2. macOS:  $HOME/Library/Application Support/MergeQueue
   Linux:  $XDG_DATA_HOME/mergequeue
              (fallback $HOME/.local/share/mergequeue)
3. error
```

## Layout

```
$MERGEQUEUE_HOME/
  state.sqlite[-wal][-shm]
  tui.pid
  config.toml
  runs/<entry-id>/
    ci-lint.log
    ci-test.log
    ci-build.log
    rebase.log
    merge.log
```

## Global config (`config.toml`)

```toml
[worker]
poll_interval_secs        = 1
dirty_target_retry_secs   = 30
soft_shutdown_timeout_secs = 60

[agent]
default = "opencode"     # opencode (the only shipped backend)

[tui]
sprite_fps              = 12
sprite_scale            = 1
celebrate_on_merge      = true
```

## Per-repo config

Lives in `registered_repos` row. Edit via `mergequeue repos edit <id|name>`,
which round-trips through `$EDITOR` on a TOML stub:

```toml
# Editing repo: mergequeue (a82e1c…)
root_path       = "/Users/cosmin/Projects/mergequeue"
default_branch  = "main"
lint_command    = "cargo fmt --check && cargo clippy -- -D warnings"
test_command    = "cargo test"
build_command   = "cargo build --release"
dirty_retry_ms  = 30000
agent_backend   = "opencode"
```
