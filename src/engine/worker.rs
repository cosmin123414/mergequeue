//! Single-repo worker — the imperative shell driving the FSM.

use std::sync::Arc;
use std::time::Duration;

use crate::agents::tmux::TmuxOps;
use crate::core::events::QueueEvent;
use crate::core::ports::{AgentRegistry, Clock, GitOps, QueueStore};
use crate::core::queue::{MergeFailureReason, QueueEntry, QueueStatus, StepOutcome};
use crate::core::repo::RegisteredRepo;
use crate::core::state_machine::{advance_status, transition, NextAction, TerminalStatus};
use crate::engine::ci::CiRunner;
use crate::engine::events::EventBroadcaster;
use crate::engine::handoff;
use crate::engine::precheck::run_precheck;
use crate::engine::shell;
use crate::engine::shutdown::ShutdownToken;
use crate::error::Result;

/// Bag of dependencies a worker needs. All come in through trait objects
/// so tests can supply fakes.
pub struct WorkerDeps {
    pub store: Arc<dyn QueueStore>,
    pub git: Arc<dyn GitOps>,
    pub clock: Arc<dyn Clock>,
    pub events: Arc<EventBroadcaster>,
    pub shutdown: ShutdownToken,
    pub runs_dir: std::path::PathBuf,
    pub tmux: Arc<dyn TmuxOps>,
    pub agents: Arc<dyn AgentRegistry>,
    /// Identifier the tmux session is scoped to (typically the
    /// MergeSmith PID). Lets workers running in the same process share
    /// one tmux session.
    pub tmux_session_pid: u32,
}

pub struct Worker {
    pub repo: RegisteredRepo,
    pub deps: WorkerDeps,
    pub poll_interval: Duration,
}

impl Worker {
    pub fn new(repo: RegisteredRepo, deps: WorkerDeps, poll_interval: Duration) -> Self {
        Self {
            repo,
            deps,
            poll_interval,
        }
    }

    /// Drive the worker until the shutdown token is set. Each iteration
    /// either processes one claimed entry to a terminal/sleep state, or
    /// sleeps `poll_interval`.
    pub fn run(&self) -> Result<()> {
        let pid = std::process::id();
        while !self.deps.shutdown.is_set() {
            let claimed = self
                .deps
                .store
                .claim_next(self.repo.id, pid, self.deps.clock.now())?;
            match claimed {
                Some(entry) => self.process(entry)?,
                None => self.deps.clock.sleep(self.poll_interval),
            }
        }
        Ok(())
    }

    /// Process exactly one claimed entry. Returns when the FSM
    /// finalizes, hands off, or hits a sleep instruction.
    pub fn process(&self, mut entry: QueueEntry) -> Result<()> {
        // Step 0 — precheck. `claim_next` already moved status to
        // `Rebasing` and set `last_outcome = PrecheckOk` as a default.
        // We re-run the precheck here so that a dirty target sends us
        // back to Queued without performing a rebase.
        let pre = run_precheck(&*self.deps.git, &self.repo, &entry)?;
        entry.last_outcome = Some(pre);
        if matches!(pre, StepOutcome::PrecheckDirtyTarget) {
            // Hop status back to Queued and bail with a Sleep so the
            // outer loop reclaims after the retry interval.
            entry.status = QueueStatus::Queued;
            entry.claimed_by_pid = None;
            entry.claimed_at = None;
            self.deps.store.update_entry(&entry)?;
            self.deps.events.emit(QueueEvent::StatusChanged {
                id: entry.id,
                from: QueueStatus::Rebasing,
                to: QueueStatus::Queued,
            });
            self.deps.clock.sleep(self.repo.ci.dirty_retry);
            return Ok(());
        }

        // The precheck already set us to Rebasing (via claim_next) and
        // last_outcome=PrecheckOk; the FSM will route us to Rebase.
        // We loop until the FSM says Finalize / HandoffToAgent / Sleep.
        // We need to feed the FSM `(current_status, last_outcome)` and
        // execute its `NextAction`.
        //
        // The first loop iteration uses (Rebasing, PrecheckOk) — but
        // strictly speaking the FSM expects (Queued, PrecheckOk) -> Rebase.
        // Reconcile this by treating the post-claim state as "we owe
        // a Rebase action," i.e. we drive from status=Queued in memory
        // even though the store has us at Rebasing. The store gets
        // overwritten at the end of each step.
        entry.status = QueueStatus::Queued;

        loop {
            if self.deps.shutdown.is_set() {
                // Roll back to Queued for next startup.
                self.deps.store.release(entry.id)?;
                return Ok(());
            }
            let outcome = entry
                .last_outcome
                .expect("worker invariant: last_outcome set before transition");
            let action = transition(entry.status, outcome, &self.repo.ci);
            self.deps.events.emit(QueueEvent::StepStarted {
                id: entry.id,
                action,
            });

            match action {
                NextAction::Sleep(d) => {
                    self.deps.clock.sleep(d);
                    return Ok(());
                }
                NextAction::Finalize(t) => {
                    self.finalize(&mut entry, t)?;
                    return Ok(());
                }
                NextAction::HandoffToAgent => {
                    self.handoff(&mut entry)?;
                    return Ok(());
                }
                NextAction::Precheck => {
                    // Shouldn't normally be returned by the FSM in M1's
                    // loop shape — precheck happens before the loop.
                    return Ok(());
                }
                other => {
                    let new_outcome = self.execute(other, &entry)?;
                    let from = entry.status;
                    entry.status = advance_status(entry.status, other);
                    entry.last_outcome = Some(new_outcome);
                    self.deps.store.update_entry(&entry)?;
                    self.deps.events.emit(QueueEvent::StatusChanged {
                        id: entry.id,
                        from,
                        to: entry.status,
                    });
                    self.deps.events.emit(QueueEvent::StepFinished {
                        id: entry.id,
                        outcome: new_outcome,
                    });
                }
            }
        }
    }

    fn execute(&self, action: NextAction, entry: &QueueEntry) -> Result<StepOutcome> {
        match action {
            NextAction::Rebase => shell::run_rebase(&*self.deps.git, entry),
            NextAction::RunLint => {
                let ci = CiRunner::new(&self.deps.runs_dir);
                shell::run_lint(&ci, &self.repo, entry)
            }
            NextAction::RunTest => {
                let ci = CiRunner::new(&self.deps.runs_dir);
                shell::run_test(&ci, &self.repo, entry)
            }
            NextAction::RunBuild => {
                let ci = CiRunner::new(&self.deps.runs_dir);
                shell::run_build(&ci, &self.repo, entry)
            }
            NextAction::FastForward => shell::run_fast_forward(&*self.deps.git, &self.repo, entry),
            NextAction::Precheck
            | NextAction::HandoffToAgent
            | NextAction::Sleep(_)
            | NextAction::Finalize(_) => {
                // Filtered out at the call-site.
                unreachable!("Worker::execute should never see {action:?}")
            }
        }
    }

    fn finalize(&self, entry: &mut QueueEntry, terminal: TerminalStatus) -> Result<()> {
        let from = entry.status;
        entry.status = terminal.into();
        entry.failure_reason = terminal.failure_reason();
        entry.finished_at = Some(self.deps.clock.now());
        entry.claimed_by_pid = None;
        entry.claimed_at = None;
        self.deps.store.update_entry(entry)?;
        self.deps.events.emit(QueueEvent::StatusChanged {
            id: entry.id,
            from,
            to: entry.status,
        });
        Ok(())
    }

    fn handoff(&self, entry: &mut QueueEntry) -> Result<()> {
        let from = entry.status;
        let session = handoff::open_conflict_session(
            &*self.deps.store,
            &*self.deps.clock,
            &*self.deps.tmux,
            &*self.deps.agents,
            &*self.deps.git,
            self.deps.tmux_session_pid,
            &self.repo,
            entry,
        )?;
        entry.status = QueueStatus::NeedsHelp;
        entry.failure_reason = Some(MergeFailureReason::RebaseUnresolvable);
        entry.conflict_session_id = Some(session.id);
        entry.claimed_by_pid = None;
        entry.claimed_at = None;
        self.deps.store.update_entry(entry)?;
        self.deps.events.emit(QueueEvent::StatusChanged {
            id: entry.id,
            from,
            to: entry.status,
        });
        Ok(())
    }
}
