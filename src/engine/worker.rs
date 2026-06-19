//! Single-repo worker — the imperative shell driving the FSM.

use std::sync::Arc;
use std::time::Duration;

use crate::agents::tmux::TmuxOps;
use crate::core::events::QueueEvent;
use crate::core::ports::{AgentRegistry, Clock, GitOps, QueueStore};
use crate::core::queue::{
    MergeFailureReason, QueueEntry, QueueEntryDetailStatus, QueueEntryDetails, QueueStatus,
    StepOutcome,
};
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
    /// MergeQueue PID). Lets workers running in the same process share
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
            set_detail_headline(&mut entry, "Waiting for target worktree to be clean");
            push_detail(
                &mut entry,
                QueueEntryDetailStatus::Blocked,
                "Target worktree is dirty",
                Some(format!(
                    "{} has uncommitted changes; will retry after {:?}",
                    self.repo.root_path.display(),
                    self.repo.ci.dirty_retry
                )),
            );
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
                    if matches!(
                        other,
                        NextAction::RunLint | NextAction::RunTest | NextAction::RunBuild
                    ) {
                        entry.ci_log_dir = Some(self.deps.runs_dir.join(entry.id.to_string()));
                    }
                    let new_outcome = self.execute(other, &entry)?;
                    record_step_detail(&mut entry, other, new_outcome);
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
        record_terminal_detail(entry, terminal);
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
        let handoff = handoff::open_conflict_session(
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
        entry.conflict_session_id = Some(handoff.session.id);
        entry.claimed_by_pid = None;
        entry.claimed_at = None;
        set_detail_headline(entry, "Needs help resolving merge conflicts");
        let conflict_detail = if handoff.conflicted_files.is_empty() {
            None
        } else {
            Some(handoff.conflicted_files.join(", "))
        };
        push_detail(
            entry,
            QueueEntryDetailStatus::Blocked,
            "Rebase has conflicts",
            conflict_detail,
        );
        let agent_detail = if handoff.agent_started {
            Some(format!(
                "Attach with `t` to tmux {}:{}",
                handoff.session.tmux_session, handoff.session.tmux_window
            ))
        } else {
            Some(format!(
                "Agent did not launch automatically: {}",
                handoff.error.unwrap_or_else(|| "unknown error".to_string())
            ))
        };
        push_detail(
            entry,
            QueueEntryDetailStatus::Info,
            format!("{} handoff", self.repo.agent_backend),
            agent_detail,
        );
        self.deps.store.update_entry(entry)?;
        self.deps.events.emit(QueueEvent::StatusChanged {
            id: entry.id,
            from,
            to: entry.status,
        });
        Ok(())
    }
}

fn details_mut(entry: &mut QueueEntry) -> &mut QueueEntryDetails {
    if entry.details.is_none() {
        let headline = format!(
            "Merging {} into {}",
            entry.source_branch, entry.target_branch
        );
        entry.details = Some(QueueEntryDetails::new(headline));
    }
    entry.details.as_mut().expect("details initialized above")
}

fn set_detail_headline(entry: &mut QueueEntry, headline: impl Into<String>) {
    details_mut(entry).headline = Some(headline.into());
}

fn push_detail(
    entry: &mut QueueEntry,
    status: QueueEntryDetailStatus,
    title: impl Into<String>,
    detail: Option<String>,
) {
    details_mut(entry).push(status, title, detail);
}

fn record_step_detail(entry: &mut QueueEntry, action: NextAction, outcome: StepOutcome) {
    match (action, outcome) {
        (NextAction::Rebase, StepOutcome::RebaseOk) => push_detail(
            entry,
            QueueEntryDetailStatus::Success,
            "Rebase completed",
            Some(format!(
                "{} onto {}",
                entry.source_branch, entry.target_branch
            )),
        ),
        (NextAction::Rebase, StepOutcome::RebaseConflict) => push_detail(
            entry,
            QueueEntryDetailStatus::Blocked,
            "Rebase blocked",
            Some("Conflicts need an agent or human resolution".to_string()),
        ),
        (NextAction::RunLint, StepOutcome::LintPassed) => push_detail(
            entry,
            QueueEntryDetailStatus::Success,
            "Lint passed",
            log_detail(entry, "lint"),
        ),
        (NextAction::RunLint, StepOutcome::LintFailed) => push_detail(
            entry,
            QueueEntryDetailStatus::Blocked,
            "Lint failed",
            log_detail(entry, "lint"),
        ),
        (NextAction::RunTest, StepOutcome::TestPassed) => push_detail(
            entry,
            QueueEntryDetailStatus::Success,
            "Tests passed",
            log_detail(entry, "test"),
        ),
        (NextAction::RunTest, StepOutcome::TestFailed) => push_detail(
            entry,
            QueueEntryDetailStatus::Blocked,
            "Tests failed",
            log_detail(entry, "test"),
        ),
        (NextAction::RunBuild, StepOutcome::BuildPassed) => push_detail(
            entry,
            QueueEntryDetailStatus::Success,
            "Build passed",
            log_detail(entry, "build"),
        ),
        (NextAction::RunBuild, StepOutcome::BuildFailed) => push_detail(
            entry,
            QueueEntryDetailStatus::Blocked,
            "Build failed",
            log_detail(entry, "build"),
        ),
        (NextAction::FastForward, StepOutcome::FastForwardOk) => push_detail(
            entry,
            QueueEntryDetailStatus::Success,
            "Fast-forward completed",
            Some(format!("{} advanced", entry.target_branch)),
        ),
        (NextAction::FastForward, StepOutcome::FastForwardRejected) => push_detail(
            entry,
            QueueEntryDetailStatus::Blocked,
            "Fast-forward rejected",
            Some("Target moved while this entry was running".to_string()),
        ),
        _ => {}
    }
}

fn record_terminal_detail(entry: &mut QueueEntry, terminal: TerminalStatus) {
    match terminal {
        TerminalStatus::Merged => {
            set_detail_headline(entry, "Merged successfully");
            push_detail(
                entry,
                QueueEntryDetailStatus::Success,
                "Entry merged",
                Some(format!(
                    "{} -> {}",
                    entry.source_branch, entry.target_branch
                )),
            );
        }
        TerminalStatus::Failed(reason) => {
            set_detail_headline(entry, format!("Blocked: {reason}"));
        }
        TerminalStatus::NeedsHelp(reason) => {
            set_detail_headline(entry, format!("Needs help: {reason}"));
        }
        TerminalStatus::Cancelled => {
            set_detail_headline(entry, "Cancelled");
            push_detail(entry, QueueEntryDetailStatus::Info, "Entry cancelled", None);
        }
    }
}

fn log_detail(entry: &QueueEntry, step: &str) -> Option<String> {
    entry
        .ci_log_dir
        .as_ref()
        .map(|dir| format!("log: {}", dir.join(format!("ci-{step}.log")).display()))
}
