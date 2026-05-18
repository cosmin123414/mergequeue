//! The merge-queue FSM, encoded as data.
//!
//! `transition` is a pure `fn(QueueStatus, StepOutcome, &RepoCiConfig) ->
//! NextAction`. It performs no I/O, takes no `&mut self`, returns no
//! `Result`. The imperative shell in `crate::engine::worker` is what
//! executes the action and feeds the next `StepOutcome` back.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::queue::{MergeFailureReason, QueueStatus, StepOutcome};
use crate::core::repo::RepoCiConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NextAction {
    Precheck,
    Rebase,
    RunLint,
    RunTest,
    RunBuild,
    FastForward,
    HandoffToAgent,
    Sleep(#[serde(with = "duration_ms")] Duration),
    Finalize(TerminalStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalStatus {
    Merged,
    Failed(MergeFailureReason),
    NeedsHelp(MergeFailureReason),
    Cancelled,
}

impl From<TerminalStatus> for QueueStatus {
    fn from(t: TerminalStatus) -> Self {
        match t {
            TerminalStatus::Merged => Self::Merged,
            TerminalStatus::Failed(_) => Self::Failed,
            TerminalStatus::NeedsHelp(_) => Self::NeedsHelp,
            TerminalStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl TerminalStatus {
    pub fn failure_reason(&self) -> Option<MergeFailureReason> {
        match self {
            Self::Failed(r) | Self::NeedsHelp(r) => Some(*r),
            Self::Merged | Self::Cancelled => None,
        }
    }
}

/// The FSM. Pure. No I/O, no time, no async.
///
/// # Panics
///
/// Panics on logically-impossible `(status, outcome)` pairs — these
/// indicate a shell-side bug (the shell produced an outcome for a
/// state that cannot have produced it).
pub fn transition(current: QueueStatus, outcome: StepOutcome, cfg: &RepoCiConfig) -> NextAction {
    use NextAction::{FastForward, Finalize, HandoffToAgent, Rebase, Sleep};
    use QueueStatus::{CIRunning, Merging, NeedsHelp, Queued, Rebasing};
    use StepOutcome::{
        AgentResolvedConflict, BuildFailed, BuildPassed, FastForwardOk, FastForwardRejected,
        LintFailed, LintPassed, PrecheckDirtyTarget, PrecheckOk, RebaseConflict, RebaseOk,
        TestFailed, TestPassed, UserCancelled,
    };
    use TerminalStatus::{Cancelled, Failed, Merged};

    // UserCancelled from any non-terminal state terminates immediately.
    if matches!(outcome, UserCancelled) {
        return Finalize(Cancelled);
    }

    // The `match_same_arms` lint wants us to merge `(Queued, PrecheckOk)` and
    // `(NeedsHelp, AgentResolvedConflict)` because both route to `Rebase`.
    // We keep them separate so the FSM reads top-to-bottom by source state.
    #[allow(clippy::match_same_arms)]
    match (current, outcome) {
        (Queued, PrecheckOk) => Rebase,
        (Queued, PrecheckDirtyTarget) => Sleep(cfg.dirty_retry),

        (Rebasing, RebaseOk) => first_ci_step(cfg),
        (Rebasing, RebaseConflict) => HandoffToAgent,

        (CIRunning, LintPassed) => after_lint(cfg),
        (CIRunning, LintFailed) => Finalize(Failed(MergeFailureReason::CILintFailed)),
        (CIRunning, TestPassed) => after_test(cfg),
        (CIRunning, TestFailed) => Finalize(Failed(MergeFailureReason::CITestsFailed)),
        (CIRunning, BuildPassed) => FastForward,
        (CIRunning, BuildFailed) => Finalize(Failed(MergeFailureReason::CIBuildFailed)),

        (Merging, FastForwardOk) => Finalize(Merged),
        (Merging, FastForwardRejected) => Finalize(Failed(MergeFailureReason::FastForwardFailed)),

        (NeedsHelp, AgentResolvedConflict) => Rebase,

        (s, o) => panic!("invalid transition from {s:?} on {o:?}"),
    }
}

/// Map a `NextAction` to the persisted `QueueStatus` *after* the action
/// has been executed by the shell.
pub fn advance_status(current: QueueStatus, action: NextAction) -> QueueStatus {
    match action {
        NextAction::Precheck | NextAction::Sleep(_) => current,
        NextAction::Rebase => QueueStatus::Rebasing,
        NextAction::RunLint | NextAction::RunTest | NextAction::RunBuild => QueueStatus::CIRunning,
        NextAction::FastForward => QueueStatus::Merging,
        NextAction::HandoffToAgent => QueueStatus::NeedsHelp,
        NextAction::Finalize(t) => t.into(),
    }
}

/// First CI step that's configured, or `FastForward` if none are.
fn first_ci_step(cfg: &RepoCiConfig) -> NextAction {
    if cfg.lint_command.is_some() {
        NextAction::RunLint
    } else if cfg.test_command.is_some() {
        NextAction::RunTest
    } else if cfg.build_command.is_some() {
        NextAction::RunBuild
    } else {
        NextAction::FastForward
    }
}

fn after_lint(cfg: &RepoCiConfig) -> NextAction {
    if cfg.test_command.is_some() {
        NextAction::RunTest
    } else if cfg.build_command.is_some() {
        NextAction::RunBuild
    } else {
        NextAction::FastForward
    }
}

fn after_test(cfg: &RepoCiConfig) -> NextAction {
    if cfg.build_command.is_some() {
        NextAction::RunBuild
    } else {
        NextAction::FastForward
    }
}

mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_all() -> RepoCiConfig {
        RepoCiConfig {
            lint_command: Some("lint".into()),
            test_command: Some("test".into()),
            build_command: Some("build".into()),
            dirty_retry: Duration::from_secs(30),
        }
    }

    fn cfg_none() -> RepoCiConfig {
        RepoCiConfig::default()
    }

    fn cfg_only_test() -> RepoCiConfig {
        RepoCiConfig {
            test_command: Some("test".into()),
            ..Default::default()
        }
    }

    fn cfg_no_lint() -> RepoCiConfig {
        RepoCiConfig {
            test_command: Some("test".into()),
            build_command: Some("build".into()),
            ..Default::default()
        }
    }

    #[test]
    fn queued_precheck_ok_routes_to_rebase() {
        assert_eq!(
            transition(QueueStatus::Queued, StepOutcome::PrecheckOk, &cfg_all()),
            NextAction::Rebase,
        );
    }

    #[test]
    fn queued_dirty_target_sleeps_for_configured_retry() {
        let cfg = RepoCiConfig {
            dirty_retry: Duration::from_secs(7),
            ..Default::default()
        };
        assert_eq!(
            transition(QueueStatus::Queued, StepOutcome::PrecheckDirtyTarget, &cfg),
            NextAction::Sleep(Duration::from_secs(7)),
        );
    }

    #[test]
    fn rebase_ok_with_all_ci_runs_lint_first() {
        assert_eq!(
            transition(QueueStatus::Rebasing, StepOutcome::RebaseOk, &cfg_all()),
            NextAction::RunLint,
        );
    }

    #[test]
    fn rebase_ok_with_no_lint_skips_to_test() {
        assert_eq!(
            transition(QueueStatus::Rebasing, StepOutcome::RebaseOk, &cfg_no_lint()),
            NextAction::RunTest,
        );
    }

    #[test]
    fn rebase_ok_with_only_test_runs_test() {
        assert_eq!(
            transition(
                QueueStatus::Rebasing,
                StepOutcome::RebaseOk,
                &cfg_only_test()
            ),
            NextAction::RunTest,
        );
    }

    #[test]
    fn rebase_ok_with_no_ci_jumps_to_fast_forward() {
        assert_eq!(
            transition(QueueStatus::Rebasing, StepOutcome::RebaseOk, &cfg_none()),
            NextAction::FastForward,
        );
    }

    #[test]
    fn rebase_conflict_handoff() {
        assert_eq!(
            transition(
                QueueStatus::Rebasing,
                StepOutcome::RebaseConflict,
                &cfg_all()
            ),
            NextAction::HandoffToAgent,
        );
    }

    #[test]
    fn lint_passed_routes_to_test_when_configured() {
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::LintPassed, &cfg_all()),
            NextAction::RunTest,
        );
    }

    #[test]
    fn lint_passed_with_no_test_routes_to_build() {
        let cfg = RepoCiConfig {
            lint_command: Some("l".into()),
            build_command: Some("b".into()),
            ..Default::default()
        };
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::LintPassed, &cfg),
            NextAction::RunBuild,
        );
    }

    #[test]
    fn lint_passed_with_only_lint_jumps_to_fast_forward() {
        let cfg = RepoCiConfig {
            lint_command: Some("l".into()),
            ..Default::default()
        };
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::LintPassed, &cfg),
            NextAction::FastForward,
        );
    }

    #[test]
    fn lint_failed_terminates_with_correct_reason() {
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::LintFailed, &cfg_all()),
            NextAction::Finalize(TerminalStatus::Failed(MergeFailureReason::CILintFailed)),
        );
    }

    #[test]
    fn test_failed_terminates_with_correct_reason() {
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::TestFailed, &cfg_all()),
            NextAction::Finalize(TerminalStatus::Failed(MergeFailureReason::CITestsFailed)),
        );
    }

    #[test]
    fn build_failed_terminates_with_correct_reason() {
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::BuildFailed, &cfg_all()),
            NextAction::Finalize(TerminalStatus::Failed(MergeFailureReason::CIBuildFailed)),
        );
    }

    #[test]
    fn build_passed_goes_to_fast_forward() {
        assert_eq!(
            transition(QueueStatus::CIRunning, StepOutcome::BuildPassed, &cfg_all()),
            NextAction::FastForward,
        );
    }

    #[test]
    fn ff_ok_finalizes_merged() {
        assert_eq!(
            transition(QueueStatus::Merging, StepOutcome::FastForwardOk, &cfg_all()),
            NextAction::Finalize(TerminalStatus::Merged),
        );
    }

    #[test]
    fn ff_rejected_finalizes_failed() {
        assert_eq!(
            transition(
                QueueStatus::Merging,
                StepOutcome::FastForwardRejected,
                &cfg_all()
            ),
            NextAction::Finalize(TerminalStatus::Failed(
                MergeFailureReason::FastForwardFailed
            )),
        );
    }

    #[test]
    fn agent_resolved_from_needs_help_retries_rebase() {
        assert_eq!(
            transition(
                QueueStatus::NeedsHelp,
                StepOutcome::AgentResolvedConflict,
                &cfg_all()
            ),
            NextAction::Rebase,
        );
    }

    #[test]
    fn user_cancel_from_any_non_terminal_state_terminates() {
        for s in [
            QueueStatus::Queued,
            QueueStatus::Rebasing,
            QueueStatus::CIRunning,
            QueueStatus::Merging,
            QueueStatus::NeedsHelp,
        ] {
            assert_eq!(
                transition(s, StepOutcome::UserCancelled, &cfg_all()),
                NextAction::Finalize(TerminalStatus::Cancelled),
            );
        }
    }

    #[test]
    fn advance_status_table() {
        assert_eq!(
            advance_status(QueueStatus::Queued, NextAction::Rebase),
            QueueStatus::Rebasing,
        );
        assert_eq!(
            advance_status(QueueStatus::Rebasing, NextAction::RunLint),
            QueueStatus::CIRunning,
        );
        assert_eq!(
            advance_status(QueueStatus::CIRunning, NextAction::FastForward),
            QueueStatus::Merging,
        );
        assert_eq!(
            advance_status(
                QueueStatus::Merging,
                NextAction::Finalize(TerminalStatus::Merged)
            ),
            QueueStatus::Merged,
        );
        assert_eq!(
            advance_status(QueueStatus::Rebasing, NextAction::HandoffToAgent),
            QueueStatus::NeedsHelp,
        );
        // Sleep / Precheck do not change persisted status.
        assert_eq!(
            advance_status(
                QueueStatus::Queued,
                NextAction::Sleep(Duration::from_secs(1))
            ),
            QueueStatus::Queued,
        );
    }

    #[test]
    #[should_panic(expected = "invalid transition")]
    fn invalid_transition_panics() {
        // `LintPassed` from `Queued` is not a possible shell output.
        let _ = transition(QueueStatus::Queued, StepOutcome::LintPassed, &cfg_all());
    }

    #[test]
    fn terminal_status_failure_reason_matches() {
        assert_eq!(
            TerminalStatus::Failed(MergeFailureReason::CITestsFailed).failure_reason(),
            Some(MergeFailureReason::CITestsFailed),
        );
        assert_eq!(TerminalStatus::Merged.failure_reason(), None);
        assert_eq!(TerminalStatus::Cancelled.failure_reason(), None);
    }
}
