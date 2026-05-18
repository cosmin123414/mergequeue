//! `QueueEntry`, `QueueStatus`, `StepOutcome`, `MergeFailureReason`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::core::ids::{ConflictSessionId, QueueEntryId, RepoId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueueStatus {
    Queued,
    Rebasing,
    CIRunning,
    Merging,
    NeedsHelp,
    Merged,
    Failed,
    Cancelled,
}

impl QueueStatus {
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Rebasing,
        Self::CIRunning,
        Self::Merging,
        Self::NeedsHelp,
        Self::Merged,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Merged | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Rebasing => "Rebasing",
            Self::CIRunning => "CIRunning",
            Self::Merging => "Merging",
            Self::NeedsHelp => "NeedsHelp",
            Self::Merged => "Merged",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl std::fmt::Display for QueueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepOutcome {
    PrecheckOk,
    PrecheckDirtyTarget,
    RebaseOk,
    RebaseConflict,
    LintPassed,
    LintFailed,
    TestPassed,
    TestFailed,
    BuildPassed,
    BuildFailed,
    FastForwardOk,
    FastForwardRejected,
    AgentResolvedConflict,
    UserCancelled,
}

impl StepOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrecheckOk => "PrecheckOk",
            Self::PrecheckDirtyTarget => "PrecheckDirtyTarget",
            Self::RebaseOk => "RebaseOk",
            Self::RebaseConflict => "RebaseConflict",
            Self::LintPassed => "LintPassed",
            Self::LintFailed => "LintFailed",
            Self::TestPassed => "TestPassed",
            Self::TestFailed => "TestFailed",
            Self::BuildPassed => "BuildPassed",
            Self::BuildFailed => "BuildFailed",
            Self::FastForwardOk => "FastForwardOk",
            Self::FastForwardRejected => "FastForwardRejected",
            Self::AgentResolvedConflict => "AgentResolvedConflict",
            Self::UserCancelled => "UserCancelled",
        }
    }
}

impl std::fmt::Display for StepOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeFailureReason {
    TargetWorktreeDirty,
    RebaseUnresolvable,
    CILintFailed,
    CITestsFailed,
    CIBuildFailed,
    FastForwardFailed,
    WorktreeGone,
    UserCancelled,
    UncleanShutdown,
}

impl MergeFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TargetWorktreeDirty => "TargetWorktreeDirty",
            Self::RebaseUnresolvable => "RebaseUnresolvable",
            Self::CILintFailed => "CILintFailed",
            Self::CITestsFailed => "CITestsFailed",
            Self::CIBuildFailed => "CIBuildFailed",
            Self::FastForwardFailed => "FastForwardFailed",
            Self::WorktreeGone => "WorktreeGone",
            Self::UserCancelled => "UserCancelled",
            Self::UncleanShutdown => "UncleanShutdown",
        }
    }
}

impl std::fmt::Display for MergeFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: QueueEntryId,
    pub repo_id: RepoId,
    pub source_worktree: PathBuf,
    pub source_branch: String,
    pub target_branch: String,
    pub status: QueueStatus,
    pub last_outcome: Option<StepOutcome>,
    #[serde(with = "time::serde::iso8601")]
    pub enqueued_at: OffsetDateTime,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub finished_at: Option<OffsetDateTime>,
    pub failure_reason: Option<MergeFailureReason>,
    pub ci_log_dir: Option<PathBuf>,
    pub merge_log_path: Option<PathBuf>,
    pub conflict_session_id: Option<ConflictSessionId>,
    pub message: Option<String>,
    pub claimed_by_pid: Option<u32>,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub claimed_at: Option<OffsetDateTime>,
}
