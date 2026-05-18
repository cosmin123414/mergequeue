//! The four seam traits. Adapters live in `crate::{store, git, agents}`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;

use crate::core::agent_backend::AgentBackend;
use crate::core::conflict::{ConflictOutcome, ConflictSession};
// `ConflictSession` is used by the QueueStore trait; agents return an
// `AgentSessionAck` and the engine wraps it into a `ConflictSession`.
use crate::core::ids::{ConflictSessionId, QueueEntryId, RepoId};
use crate::core::queue::{QueueEntry, QueueStatus};
use crate::core::repo::RegisteredRepo;
use crate::error::Result;

// ---------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------

pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
    fn sleep(&self, d: Duration);
}

// ---------------------------------------------------------------------
// QueueStore
// ---------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct EntryFilter {
    pub repo_id: Option<RepoId>,
    pub statuses: Option<Vec<QueueStatus>>,
}

impl EntryFilter {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn by_repo(repo: RepoId) -> Self {
        Self {
            repo_id: Some(repo),
            statuses: None,
        }
    }

    pub fn with_status(mut self, statuses: Vec<QueueStatus>) -> Self {
        self.statuses = Some(statuses);
        self
    }
}

pub trait QueueStore: Send + Sync {
    // Repos
    fn list_repos(&self) -> Result<Vec<RegisteredRepo>>;
    fn get_repo(&self, id: RepoId) -> Result<Option<RegisteredRepo>>;
    fn get_repo_by_root(&self, root: &Path) -> Result<Option<RegisteredRepo>>;
    fn insert_repo(&self, repo: &RegisteredRepo) -> Result<()>;
    fn update_repo(&self, repo: &RegisteredRepo) -> Result<()>;
    fn delete_repo(&self, id: RepoId) -> Result<()>;

    // Queue entries
    fn enqueue(&self, entry: &QueueEntry) -> Result<()>;
    fn list_entries(&self, filter: EntryFilter) -> Result<Vec<QueueEntry>>;
    fn get_entry(&self, id: QueueEntryId) -> Result<Option<QueueEntry>>;
    fn claim_next(&self, repo: RepoId, pid: u32, now: OffsetDateTime)
        -> Result<Option<QueueEntry>>;
    fn update_entry(&self, entry: &QueueEntry) -> Result<()>;
    fn release(&self, id: QueueEntryId) -> Result<()>;
    fn delete_queued(&self, id: QueueEntryId) -> Result<bool>;
    fn sweep_dead_pid_claims(&self, live_pids: &[u32]) -> Result<usize>;

    // Conflict sessions
    fn open_conflict_session(&self, s: &ConflictSession) -> Result<()>;
    fn get_conflict_session(&self, id: ConflictSessionId) -> Result<Option<ConflictSession>>;
    /// Sessions with `ended_at IS NULL`. Used by the recovery sweep to
    /// detect orphans when MergeSmith restarts.
    fn list_open_conflict_sessions(&self) -> Result<Vec<ConflictSession>>;
    fn close_conflict_session(
        &self,
        id: ConflictSessionId,
        outcome: ConflictOutcome,
        when: OffsetDateTime,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------
// GitOps
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseOutcome {
    Ok,
    Conflict,
    OtherError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastForwardOutcome {
    Ok,
    NonFastForward,
    OtherError(String),
}

pub trait GitOps: Send + Sync {
    fn worktree_is_dirty(&self, path: &Path) -> Result<bool>;
    fn current_branch(&self, worktree: &Path) -> Result<String>;
    fn head_sha(&self, worktree: &Path) -> Result<String>;
    fn worktree_exists(&self, path: &Path) -> Result<bool>;
    fn rebase_onto(&self, worktree: &Path, target_ref: &str) -> Result<RebaseOutcome>;
    fn abort_rebase(&self, worktree: &Path) -> Result<()>;
    fn fast_forward(&self, target_worktree: &Path, source_ref: &str) -> Result<FastForwardOutcome>;
    /// Locate the worktree root (`git rev-parse --show-toplevel`).
    fn discover_worktree_root(&self, start: &Path) -> Result<PathBuf>;
    /// Paths (relative to the worktree root) currently flagged as
    /// unmerged (`git diff --name-only --diff-filter=U`). Empty when
    /// no rebase is in progress.
    fn conflicted_files(&self, worktree: &Path) -> Result<Vec<String>>;
}

// ---------------------------------------------------------------------
// MergeAgent
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ConflictPrompt {
    pub repo_name: String,
    pub source_branch: String,
    pub target_branch: String,
    pub conflicted_files: Vec<String>,
    pub ci_command_hint: Option<String>,
}

/// Opaque handle returned by the tmux layer; agents place themselves
/// inside the session/window described here.
#[derive(Debug, Clone)]
pub struct TmuxHandle {
    pub session: String,
    pub window: String,
}

/// Acknowledgement that the agent placed itself inside the given tmux
/// handle. The engine wraps this in a `ConflictSession` with the right
/// `QueueEntryId`, `Clock`-stamped `started_at`, and a fresh
/// `ConflictSessionId`. Returned as a struct so we can grow it
/// (e.g. an agent-side session identifier) without churning the
/// engine.
#[derive(Debug, Clone, Default)]
pub struct AgentSessionAck {
    /// Optional opaque token the backend can return (e.g. an opencode
    /// session id). Persisted for diagnostics; not used by the engine.
    pub agent_session_token: Option<String>,
}

pub trait MergeAgent: Send + Sync {
    fn backend(&self) -> AgentBackend;
    fn check_available(&self) -> Result<()>;
    /// Spawn the agent inside `tmux` with `prompt`, working from
    /// `worktree`. Implementations must return promptly once the agent
    /// has been started; they must NOT block until the agent finishes.
    fn open_conflict_session(
        &self,
        worktree: &Path,
        prompt: &ConflictPrompt,
        tmux: &TmuxHandle,
    ) -> Result<AgentSessionAck>;
}

pub trait AgentRegistry: Send + Sync {
    fn get(&self, b: AgentBackend) -> Result<Arc<dyn MergeAgent>>;
}
