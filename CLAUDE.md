# MergeQueue — Architectural Intent

**One feature** (a local merge queue) with **two presentation surfaces** (CLI
one-shots and a long-running TUI). Both surfaces share a single SQLite file as
the only coordination mechanism.

## Mental model

```
┌────────────────────────────┐        ┌────────────────────────────────┐
│ CLI one-shots              │        │ TUI (single long-running proc) │
│   init, enqueue, cancel,   │        │   ratatui Braille-glyph        │
│   retry, resolve, status,  │        │   visualizer + delete-queued   │
│   logs, repos, doctor      │        │   worker pool runs in-process  │
└────────────────────────────┘        └────────────────────────────────┘
              │                                       │
              └────────────────┬──────────────────────┘
                               ▼
                   state.sqlite (WAL)
```

## Code organization (feature-first, single crate)

```
src/
├── main.rs               clap dispatch
├── lib.rs                crate root, re-exports for tests/
├── error.rs              crate Error + Result
├── config.rs             config.toml loader
├── paths.rs              state-root resolution
│
├── core/                 domain primitives + ports (no I/O)
├── store/                SQLite adapter for QueueStore
├── git/                  git-subprocess adapter for GitOps
├── engine/               THE MERGE-QUEUE ENGINE (state machine + worker pool)
├── agents/               MergeAgent impl (opencode) + tmux + prompts
├── tui/                  ratatui + Braille-glyph Penrose visualizer
├── cli/                  clap defs + subcommand handlers
└── test_support/         feature-gated fakes (Clock, Git, Store, Agent)
```

## Key invariants

1. **The FSM never touches I/O.** `core::state_machine::transition` is a pure
   `fn(QueueStatus, StepOutcome, &RepoCiConfig) -> NextAction`. If you find
   yourself wanting to make it async or pass a store handle, you're doing it
   wrong.
2. **The engine talks to the world only through four traits**: `Clock`,
   `QueueStore`, `GitOps`, `MergeAgent`. Adding a fifth means a real new
   abstraction boundary; don't add one casually.
3. **One worker per registered repo.** Workers are isolated; a slow CI on
   repo A cannot block repo B. FIFO is enforced *within* a repo via
   `repo_id`-scoped row claiming.
4. **The TUI is a visualizer.** Its only mutation is `d` to delete a queued
   (not in-flight) entry. Every other mutation is a CLI subcommand.
5. **The visualizer renders as Braille glyphs** via ratatui, so the TUI
   works in any terminal — no Kitty graphics protocol required.
6. **Crash recovery on every startup.** Entries with claimed_by_pid pointing
   at a dead PID get swept back to Queued.
