# 03 — State machine as data

Lives in `src/core/state_machine.rs`. Pure, no I/O, no async, no `&mut self`,
no `Arc<Mutex<...>>`. Inputs: current `QueueStatus`, last `StepOutcome`,
the repo's `RepoCiConfig` (so the FSM knows which CI steps to skip).
Output: `NextAction` for the imperative shell to execute.

## NextAction

```rust
pub enum NextAction {
    Precheck,
    Rebase,
    RunLint,
    RunTest,
    RunBuild,
    FastForward,
    HandoffToAgent,
    Sleep(Duration),
    Finalize(TerminalStatus),
}

pub enum TerminalStatus {
    Merged,
    Failed(MergeFailureReason),
    NeedsHelp(MergeFailureReason),
    Cancelled,
}
```

## `transition`

```rust
pub fn transition(
    current: QueueStatus,
    outcome: StepOutcome,
    cfg: &RepoCiConfig,
) -> NextAction { … }
```

## Routing rules (informal)

- `Queued` is the only entry point. The shell calls the FSM with
  `(Queued, PrecheckOk)` or `(Queued, PrecheckDirtyTarget)` after running
  the precheck.
- CI step ordering is `Lint → Test → Build`, **skipping any step whose
  command is `None`**. If all three are `None`, `RebaseOk` routes straight
  to `FastForward`.
- `UserCancelled` from any non-terminal state routes to
  `Finalize(Cancelled)`.
- `AgentResolvedConflict` from `NeedsHelp` routes back to `Rebase`.
- `Failed` / `Merged` / `Cancelled` are terminal; the FSM is never called
  on them.

## Sketch

```rust
match (current, outcome) {
    (Queued,    PrecheckOk)              => Rebase,
    (Queued,    PrecheckDirtyTarget)     => Sleep(cfg.dirty_retry),

    (Rebasing,  RebaseOk)                => first_ci_step(cfg),
    (Rebasing,  RebaseConflict)          => HandoffToAgent,

    (CIRunning, LintPassed)              => after_lint(cfg),
    (CIRunning, LintFailed)              => Finalize(Failed(CILintFailed)),
    (CIRunning, TestPassed)              => after_test(cfg),
    (CIRunning, TestFailed)              => Finalize(Failed(CITestsFailed)),
    (CIRunning, BuildPassed)             => FastForward,
    (CIRunning, BuildFailed)             => Finalize(Failed(CIBuildFailed)),

    (Merging,   FastForwardOk)           => Finalize(Merged),
    (Merging,   FastForwardRejected)     => Finalize(Failed(FastForwardFailed)),

    (NeedsHelp, AgentResolvedConflict)   => Rebase,

    (_,         UserCancelled)           => Finalize(Cancelled),

    (s, o) => panic!("invalid transition from {s:?} on {o:?}"),
}
```

`first_ci_step` / `after_lint` / `after_test` are 3-line helpers that
implement the "skip unconfigured CI steps" rule.

## Status mapping

`advance_status(current, action) -> QueueStatus` is also pure and lives
beside `transition`:

```
Precheck         -> Queued (the precheck happens inside Queued)
Rebase           -> Rebasing
RunLint/Test/Build -> CIRunning
FastForward      -> Merging
HandoffToAgent   -> NeedsHelp
Sleep            -> (no change)
Finalize(t)      -> t.into()
```

The shell calls `transition` to get the action, executes it, gets a new
`StepOutcome`, persists `(status, last_outcome)`, then asks the FSM again.

## Tests

`src/core/state_machine.rs` has its own `#[cfg(test)] mod tests` with
exhaustive coverage. Each `(QueueStatus, StepOutcome, RepoCiConfig)` tuple
that the shell can produce has an assertion. These tests run in
microseconds and need no fakes.
