# 02 — Domain model

All types live in `src/core/`. No I/O dependencies. `serde::{Serialize,
Deserialize}` everywhere. Newtype IDs over `Uuid`.

## IDs

```rust
pub struct RepoId(Uuid);
pub struct QueueEntryId(Uuid);
pub struct ConflictSessionId(Uuid);
```

## RegisteredRepo

```rust
pub struct RegisteredRepo {
    pub id: RepoId,
    pub root_path: PathBuf,
    pub default_branch: String,
    pub ci: RepoCiConfig,
    pub agent_backend: AgentBackend,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub struct RepoCiConfig {
    pub lint_command: Option<String>,
    pub test_command: Option<String>,
    pub build_command: Option<String>,
    pub dirty_retry: Duration,   // per-repo override of global default
}
```

## QueueEntry

```rust
pub struct QueueEntry {
    pub id: QueueEntryId,
    pub repo_id: RepoId,
    pub source_worktree: PathBuf,
    pub source_branch: String,
    pub target_branch: String,
    pub status: QueueStatus,
    pub last_outcome: Option<StepOutcome>,
    pub enqueued_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub failure_reason: Option<MergeFailureReason>,
    pub ci_log_dir: Option<PathBuf>,
    pub merge_log_path: Option<PathBuf>,
    pub conflict_session_id: Option<ConflictSessionId>,
    pub message: Option<String>,
    pub claimed_by_pid: Option<u32>,
    pub claimed_at: Option<OffsetDateTime>,
}
```

## QueueStatus (FSM states)

```rust
pub enum QueueStatus {
    Queued,
    Rebasing,
    CIRunning,
    Merging,
    NeedsHelp,
    Merged,        // terminal
    Failed,        // terminal
    Cancelled,     // terminal
}
```

(`PreCheck` is not a persisted state — it always runs at the start of
`Queued` claim and either transitions to `Rebasing` or stays `Queued` with a
sleep hint.)

## StepOutcome (FSM input alphabet)

```rust
pub enum StepOutcome {
    PrecheckOk,
    PrecheckDirtyTarget,
    RebaseOk,
    RebaseConflict,
    LintPassed,    LintFailed,
    TestPassed,    TestFailed,
    BuildPassed,   BuildFailed,
    FastForwardOk, FastForwardRejected,
    AgentResolvedConflict,
    UserCancelled,
}
```

## MergeFailureReason

```rust
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
```

## AgentBackend

```rust
pub enum AgentBackend { Opencode }
```

Kept as an enum (rather than collapsed away) so the per-repo
`agent_backend` column and the `MergeAgent` seam stay stable if more
backends are added later.

## ConflictSession

```rust
pub struct ConflictSession {
    pub id: ConflictSessionId,
    pub queue_entry_id: QueueEntryId,
    pub agent_backend: AgentBackend,
    pub tmux_session: String,
    pub tmux_window: String,
    pub started_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
    pub outcome: Option<ConflictOutcome>,
}

pub enum ConflictOutcome { Resolved, Abandoned, AgentCrashed }
```

## Events

```rust
pub enum QueueEvent {
    StatusChanged { id: QueueEntryId, from: QueueStatus, to: QueueStatus },
    StepStarted { id: QueueEntryId, action: NextAction },
    StepFinished { id: QueueEntryId, outcome: StepOutcome },
}
```

The TUI subscribes to a broadcast channel of these.
