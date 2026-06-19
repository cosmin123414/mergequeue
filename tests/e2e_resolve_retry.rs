//! M2 e2e: verifies that `mergequeue retry <id>` on a `NeedsHelp` entry
//! closes the open `ConflictSession` as `Resolved` and re-queues the
//! entry with `last_outcome = AgentResolvedConflict`.
//!
//! We seed the store directly (via the public `SqliteStore` API) rather
//! than driving an actual conflict through the worker, because the
//! worker exit on NeedsHelp is already covered by engine unit tests.

use std::process::Command;
use std::sync::Arc;

use assert_cmd::prelude::*;
use mergequeue::core::agent_backend::AgentBackend;
use mergequeue::core::conflict::{ConflictOutcome, ConflictSession};
use mergequeue::core::ids::{ConflictSessionId, QueueEntryId, RepoId};
use mergequeue::core::ports::{EntryFilter, QueueStore};
use mergequeue::core::queue::{MergeFailureReason, QueueEntry, QueueStatus, StepOutcome};
use mergequeue::core::repo::{RegisteredRepo, RepoCiConfig};
use mergequeue::store::SqliteStore;
use tempfile::TempDir;
use time::OffsetDateTime;

fn now() -> OffsetDateTime {
    // Truncate to seconds so round-trips compare equal.
    OffsetDateTime::from_unix_timestamp(OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

fn ms(state_root: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("mergequeue").unwrap();
    c.env("MERGEQUEUE_HOME", state_root);
    c
}

fn seed_needs_help(
    state_root: &std::path::Path,
    repo_dir: &std::path::Path,
) -> (QueueEntryId, ConflictSessionId) {
    let store_path = state_root.join("state.sqlite");
    std::fs::create_dir_all(state_root).unwrap();
    let store: Arc<dyn QueueStore> = Arc::new(SqliteStore::open(&store_path).unwrap());

    let repo = RegisteredRepo {
        id: RepoId::new(),
        root_path: repo_dir.to_path_buf(),
        default_branch: "main".into(),
        ci: RepoCiConfig::default(),
        agent_backend: AgentBackend::Opencode,
        created_at: now(),
        updated_at: now(),
    };
    store.insert_repo(&repo).unwrap();

    let session = ConflictSession {
        id: ConflictSessionId::new(),
        queue_entry_id: QueueEntryId::new(), // overwritten below for entry linkage
        agent_backend: AgentBackend::Opencode,
        tmux_session: "mergequeue-test".into(),
        tmux_window: "conflict-test".into(),
        started_at: now(),
        ended_at: None,
        outcome: None,
    };
    store.open_conflict_session(&session).unwrap();

    let entry_id = QueueEntryId::new();
    let entry = QueueEntry {
        id: entry_id,
        repo_id: repo.id,
        source_worktree: repo_dir.to_path_buf(),
        source_branch: "feat/conflict".into(),
        target_branch: "main".into(),
        status: QueueStatus::NeedsHelp,
        last_outcome: Some(StepOutcome::RebaseConflict),
        enqueued_at: now(),
        started_at: None,
        finished_at: None,
        failure_reason: Some(MergeFailureReason::RebaseUnresolvable),
        ci_log_dir: None,
        merge_log_path: None,
        conflict_session_id: Some(session.id),
        message: None,
        details: None,
        claimed_by_pid: None,
        claimed_at: None,
    };
    store.enqueue(&entry).unwrap();

    (entry_id, session.id)
}

#[test]
fn retry_on_needs_help_closes_conflict_session_as_resolved() {
    let state_root = TempDir::new().unwrap();
    let repo_dir = TempDir::new().unwrap();
    let (entry_id, session_id) = seed_needs_help(state_root.path(), repo_dir.path());

    ms(state_root.path())
        .args(["retry", &entry_id.to_string()])
        .assert()
        .success();

    // Re-open the store and inspect both records.
    let store_path = state_root.path().join("state.sqlite");
    let store: Arc<dyn QueueStore> = Arc::new(SqliteStore::open(&store_path).unwrap());

    let entry = store.get_entry(entry_id).unwrap().unwrap();
    assert_eq!(entry.status, QueueStatus::Queued);
    assert_eq!(entry.last_outcome, Some(StepOutcome::AgentResolvedConflict));
    assert_eq!(entry.failure_reason, None);

    let session = store.get_conflict_session(session_id).unwrap().unwrap();
    assert!(
        session.ended_at.is_some(),
        "expected conflict session to be closed"
    );
    assert_eq!(session.outcome, Some(ConflictOutcome::Resolved));
}

#[test]
fn resolve_on_non_needs_help_entry_refuses() {
    let state_root = TempDir::new().unwrap();
    let repo_dir = TempDir::new().unwrap();

    // Seed a plain Queued entry — resolve should refuse.
    let store_path = state_root.path().join("state.sqlite");
    std::fs::create_dir_all(state_root.path()).unwrap();
    let store: Arc<dyn QueueStore> = Arc::new(SqliteStore::open(&store_path).unwrap());
    let repo = RegisteredRepo {
        id: RepoId::new(),
        root_path: repo_dir.path().to_path_buf(),
        default_branch: "main".into(),
        ci: RepoCiConfig::default(),
        agent_backend: AgentBackend::Opencode,
        created_at: now(),
        updated_at: now(),
    };
    store.insert_repo(&repo).unwrap();
    let entry = QueueEntry {
        id: QueueEntryId::new(),
        repo_id: repo.id,
        source_worktree: repo_dir.path().to_path_buf(),
        source_branch: "feat/a".into(),
        target_branch: "main".into(),
        status: QueueStatus::Queued,
        last_outcome: None,
        enqueued_at: now(),
        started_at: None,
        finished_at: None,
        failure_reason: None,
        ci_log_dir: None,
        merge_log_path: None,
        conflict_session_id: None,
        message: None,
        details: None,
        claimed_by_pid: None,
        claimed_at: None,
    };
    store.enqueue(&entry).unwrap();

    let _ = store
        .list_entries(EntryFilter::all())
        .expect("list_entries works");

    ms(state_root.path())
        .args(["resolve", &entry.id.to_string()])
        .assert()
        .failure();
}
