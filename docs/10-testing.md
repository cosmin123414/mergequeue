# 10 — Testing strategy

Three layers, each covering what it's good at.

## Layer 1 — FSM tests (microseconds)

`src/core/state_machine.rs` `#[cfg(test)] mod tests` exhaustively covers
`(QueueStatus × StepOutcome × RepoCiConfig)`. No fakes, no setup. ~30-50
tests, all running in under 1ms total.

## Layer 2 — Shell tests with seam-trait fakes

`src/engine/` tests use `FakeClock`, `FakeStore`, `FakeGit`, `FakeAgent`
from `test_support/`. Cover:

- Worker claim/release semantics
- Per-repo isolation (two repos, two workers, no cross-contamination)
- Dirty-target retry with virtual time
- CI step ordering
- Conflict handoff lifecycle
- Soft/hard shutdown
- Crash recovery on startup

Each test sets up fakes, instantiates the worker, calls `process`, asserts
on store state + recorded events. ~50-100 tests, all under 100ms total.

## Layer 3 — End-to-end integration tests

`tests/` at workspace root (one binary per file):

```
tests/
├── e2e_queue_lifecycle.rs      real git, fake agent, real sqlite
├── e2e_multi_repo_parallel.rs  proves per-repo workers don't block each other
├── e2e_conflict_resolution.rs  real git conflict, fake agent resolves it
├── e2e_cli_smoke.rs            assert_cmd against each subcommand
└── e2e_recovery.rs             kill -9 mid-merge, restart, verify sweep
```

Use `tempfile::tempdir()` for state root and for real git repos. Use a
`FakeAgent` even in e2e tests (we don't want CI to call real `claude`).

## test_support module

Feature-gated:

```toml
[features]
test-support = []
```

```rust
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
```

Each fake records calls (so tests can assert "rebase was called twice
with these paths") and accepts scripted responses (so tests can say "the
next rebase returns Conflict").

## Snapshot tests for the TUI

`insta` for ratatui-rendered output (using `ratatui::backend::TestBackend`).
Sprite cells are excluded from snapshots — they're tested separately by
capturing the emitted Kitty protocol bytes and asserting they parse as
well-formed escape sequences.
