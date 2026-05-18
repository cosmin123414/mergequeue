# 06 — The engine

`src/engine/` is the heart of MergeSmith. It contains:

```
engine/
├── mod.rs           pub fn spawn_pool(...) -> EnginePool
├── pool.rs          EnginePool, one Worker thread per RegisteredRepo
├── worker.rs        single-repo loop (the imperative shell)
├── shell.rs         the action executors (run_precheck, run_rebase, ...)
├── ci.rs            CI subprocess runner (Lint/Test/Build)
├── handoff.rs       NeedsHelp transition + agent spawning
├── shutdown.rs      ShutdownToken + soft/hard quit semantics
├── recovery.rs      startup sweeps (dead claims, abandoned tmux)
└── events.rs        EventBroadcaster (crossbeam-channel fan-out)
```

The FSM (`core::state_machine::transition`) is what drives the loop. The
shell is what executes the FSM's chosen `NextAction`.

## Worker loop (sketch)

```rust
impl Worker {
    fn run(&self) -> Result<()> {
        while !self.shutdown.is_set() {
            let Some(entry) = self.store
                .claim_next(self.repo.id, self.pid, self.clock.now())? else
            {
                self.clock.sleep(Duration::from_secs(1));
                continue;
            };
            self.process(entry)?;
        }
        Ok(())
    }

    fn process(&self, mut entry: QueueEntry) -> Result<()> {
        // Precheck is always step zero on a freshly-claimed entry.
        let outcome = run_precheck(&self.git, &entry, &self.repo)?;
        entry.last_outcome = Some(outcome);
        loop {
            let action = transition(entry.status, outcome, &self.repo.ci);
            self.events.emit(QueueEvent::StepStarted { id: entry.id, action });
            match action {
                Sleep(d)          => { self.clock.sleep(d); break; }
                Finalize(t)       => { self.finalize(&mut entry, t)?; break; }
                HandoffToAgent    => { self.handoff(&mut entry)?; break; }
                other             => {
                    let outcome = self.execute(other, &mut entry)?;
                    entry.last_outcome = Some(outcome);
                    entry.status = advance_status(entry.status, other);
                    self.store.update_entry(&entry)?;
                    self.events.emit(QueueEvent::StepFinished {
                        id: entry.id, outcome,
                    });
                }
            }
        }
        Ok(())
    }
}
```

`self.execute` dispatches to `run_rebase`, `run_lint`, `run_test`,
`run_build`, `run_ff` — small functions in `shell.rs` that each touch one
seam trait and return a `StepOutcome`.

## CI runner

`ci.rs` runs `RepoCiConfig::{lint,test,build}_command` in order, but the
FSM controls *which* one runs next; the runner only ever runs the one step
it's asked to. Each command runs in the source worktree, captures stdout
+ stderr to `runs/<entry-id>/ci-{lint,test,build}.log`, and reports the
matching `StepOutcome` based on exit code (`0 → Passed`, non-zero →
`Failed`).

The runner respects `ShutdownToken`: on soft shutdown, after the current
subprocess exits, it stops; on hard shutdown, it sends SIGTERM to the
process group, waits 5s, then SIGKILL.

## Conflict handoff

When the FSM returns `HandoffToAgent`:

1. Look up the configured `AgentBackend` (per-repo, falling back to global).
2. Acquire a `MergeAgent` from the registry.
3. Open a tmux session named `mergesmith-<entry-shortid>` with a window
   named `<repo>-<branch>`.
4. Render the conflict prompt with minijinja (see `09-agents.md`).
5. Spawn the agent inside the tmux window.
6. Record a `ConflictSession` row.
7. Set `entry.status = NeedsHelp` (already implied by `advance_status`),
   persist, broadcast event.

The worker does **not** wait for the agent to finish. The user is expected
to resolve and re-enqueue via `mergesmith retry <id>`.

## Per-repo workers

`EnginePool::spawn` looks at the current `registered_repos` table and
spawns one `Worker` thread per row. `on_repo_registered(repo_id)` and
`on_repo_deregistered(repo_id)` adjust the pool live when the CLI mutates
the repo set. The pool keeps a `JoinHandle` per worker; shutdown joins
them all with timeout.

## Shutdown

- **Soft (`q` in TUI):** broadcast `ShutdownToken::set_soft()`. Workers
  finish their current `execute(action)` call (one git op or one CI
  subprocess), then exit. On exit, any entry still in `Rebasing`/
  `CIRunning`/`Merging` rolls back to `Queued` via `release`. Timeout:
  60s (configurable). After timeout, prompt "still running, force quit?
  (y/N)".
- **Hard (`Ctrl-C ×2` or `Q`):** `ShutdownToken::set_hard()`. Workers
  SIGTERM their children, mark current entry `Cancelled{UncleanShutdown}`,
  exit immediately.

In either case, the next startup's crash recovery handles whatever's left.
