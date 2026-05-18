# 01 — Architecture

## Processes and storage

One process while running. Two processes during enqueue overlap (a one-shot
CLI run alongside a long-running TUI). They communicate through one SQLite
file with WAL journaling and FK enforcement.

```
$MERGESMITH_HOME/
  state.sqlite[+ wal + shm]
  tui.pid                   (singleton lock for `mergesmith tui`)
  runs/<entry-id>/
    ci-lint.log
    ci-test.log
    ci-build.log
    rebase.log
    merge.log
  config.toml
```

State-root resolution:

1. `$MERGESMITH_HOME` if set
2. macOS: `$HOME/Library/Application Support/MergeSmith`
3. Linux: `$XDG_DATA_HOME/mergesmith` (default `$HOME/.local/share/mergesmith`)

## Modules and dependency direction

```
core   (pure domain + ports, no I/O)
  ▲
  ├──── store    (SQLite adapter implementing QueueStore)
  ├──── git      (subprocess adapter implementing GitOps)
  └──── agents   (impls of MergeAgent: opencode/claude/cursor/codex)
                      ▲
                      │
                   engine     (state_machine + worker pool, uses ports)
                      ▲
                      ├──── tui    (visualizer + delete-queued)
                      └──── cli    (subcommands; runs the engine for `tui`)
```

Strictly acyclic. `core` depends on nothing inside the crate. Adapters depend
only on `core`. The engine depends on `core` plus port traits (whose impls
live in adapters; the engine doesn't import the adapters directly — it takes
`Arc<dyn QueueStore>`, etc.). The TUI and CLI integrate everything.

## Why a single crate (not a workspace)

There is one feature, one binary, no externally-published library, no
cross-crate compilation isolation needed. A workspace would add manifest
overhead and refactor friction with no compile-time or testability gain.
Module visibility (`pub(crate)`) is sufficient to keep boundaries honest.

## Why state-machine-as-data + four seam traits

Together they let the engine be tested with **no subprocesses, no real
files, no real time**. See `03-state-machine.md` and `04-seam-traits.md`.
