//! Engine worker tests using the four fakes. These prove that the FSM
//! and the shell wire up correctly across the full happy / sad paths.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use time::OffsetDateTime;

use crate::core::agent_backend::AgentBackend;
use crate::core::ids::{QueueEntryId, RepoId};
use crate::core::ports::{FastForwardOutcome, GitOps, QueueStore, RebaseOutcome};
use crate::core::queue::{MergeFailureReason, QueueEntry, QueueStatus};
use crate::core::repo::{RegisteredRepo, RepoCiConfig};
use crate::engine::events::EventBroadcaster;
use crate::engine::shutdown::ShutdownToken;
use crate::engine::worker::{Worker, WorkerDeps};
use crate::test_support::{fake_store::make_fake_store, FakeClock, FakeGit, GitScript};

fn fixture_repo(root: std::path::PathBuf, ci: RepoCiConfig) -> RegisteredRepo {
    let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    RegisteredRepo {
        id: RepoId::new(),
        root_path: root,
        default_branch: "main".into(),
        ci,
        agent_backend: AgentBackend::Opencode,
        created_at: now,
        updated_at: now,
    }
}

fn fixture_entry(repo: &RegisteredRepo, source_branch: &str) -> QueueEntry {
    QueueEntry {
        id: QueueEntryId::new(),
        repo_id: repo.id,
        source_worktree: repo.root_path.join(source_branch.replace('/', "-")),
        source_branch: source_branch.into(),
        target_branch: repo.default_branch.clone(),
        status: QueueStatus::Queued,
        last_outcome: None,
        enqueued_at: OffsetDateTime::from_unix_timestamp(1_700_000_001).unwrap(),
        started_at: None,
        finished_at: None,
        failure_reason: None,
        ci_log_dir: None,
        merge_log_path: None,
        conflict_session_id: None,
        message: None,
        claimed_by_pid: None,
        claimed_at: None,
    }
}

fn build_deps(
    git: Arc<dyn GitOps>,
    store: Arc<dyn QueueStore>,
    runs: std::path::PathBuf,
) -> (WorkerDeps, Arc<EventBroadcaster>, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock::epoch());
    let events = Arc::new(EventBroadcaster::with_history());
    let deps = WorkerDeps {
        store,
        git,
        clock: clock.clone(),
        events: events.clone(),
        shutdown: ShutdownToken::new(),
        runs_dir: runs,
    };
    (deps, events, clock)
}

/// Happy path: precheck OK, rebase OK, no CI configured, fast-forward
/// OK → Merged.
#[test]
fn happy_path_no_ci_merges_cleanly() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(make_fake_store());
    let repo = fixture_repo(tmp.path().to_path_buf(), RepoCiConfig::default());
    store.insert_repo(&repo).unwrap();
    let entry = fixture_entry(&repo, "feat/a");
    store.enqueue(&entry).unwrap();

    let script = GitScript {
        rebase: vec![RebaseOutcome::Ok].into(),
        fast_forward: vec![FastForwardOutcome::Ok].into(),
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, _events, _clock) = build_deps(git, store.clone(), tmp.path().to_path_buf());
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    // Claim the entry the way the run loop would, then process it.
    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .unwrap();
    worker.process(claimed).unwrap();

    let final_entry = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(final_entry.status, QueueStatus::Merged);
    assert!(final_entry.finished_at.is_some());
    assert_eq!(final_entry.failure_reason, None);
}

/// Rebase conflict should hand off → entry ends in NeedsHelp with
/// RebaseUnresolvable.
#[test]
fn rebase_conflict_becomes_needs_help() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(make_fake_store());
    let repo = fixture_repo(tmp.path().to_path_buf(), RepoCiConfig::default());
    store.insert_repo(&repo).unwrap();
    let entry = fixture_entry(&repo, "feat/conflict");
    store.enqueue(&entry).unwrap();

    let script = GitScript {
        rebase: vec![RebaseOutcome::Conflict].into(),
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, _events, _clock) = build_deps(git, store.clone(), tmp.path().to_path_buf());
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .unwrap();
    worker.process(claimed).unwrap();

    let final_entry = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(final_entry.status, QueueStatus::NeedsHelp);
    assert_eq!(
        final_entry.failure_reason,
        Some(MergeFailureReason::RebaseUnresolvable)
    );
    assert!(final_entry.conflict_session_id.is_some());
}

/// Dirty target should not consume a rebase script entry — worker must
/// hop back to Queued and sleep for the configured retry.
#[test]
fn dirty_target_returns_to_queued_and_sleeps() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(make_fake_store());
    let ci = RepoCiConfig {
        dirty_retry: Duration::from_secs(7),
        ..Default::default()
    };
    let repo = fixture_repo(tmp.path().to_path_buf(), ci);
    store.insert_repo(&repo).unwrap();
    let entry = fixture_entry(&repo, "feat/dirty");
    store.enqueue(&entry).unwrap();

    let script = GitScript {
        dirty: vec![true].into(),
        // We deliberately leave rebase empty: if we reach it, FakeGit
        // returns RebaseOutcome::Ok by default, which would route to a
        // fast-forward. The assertion on `status == Queued` proves we
        // never got that far.
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, _events, clock) = build_deps(git, store.clone(), tmp.path().to_path_buf());
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .unwrap();
    worker.process(claimed).unwrap();

    let after = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(after.status, QueueStatus::Queued);
    assert!(after.claimed_by_pid.is_none());
    // FakeClock recorded a sleep of exactly the configured retry.
    assert_eq!(clock.sleeps(), vec![Duration::from_secs(7)]);
}

/// FF rejected should finalize as Failed with FastForwardFailed.
#[test]
fn fast_forward_rejected_finalizes_failed() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(make_fake_store());
    let repo = fixture_repo(tmp.path().to_path_buf(), RepoCiConfig::default());
    store.insert_repo(&repo).unwrap();
    let entry = fixture_entry(&repo, "feat/ff-fail");
    store.enqueue(&entry).unwrap();

    let script = GitScript {
        rebase: vec![RebaseOutcome::Ok].into(),
        fast_forward: vec![FastForwardOutcome::NonFastForward].into(),
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, _events, _clock) = build_deps(git, store.clone(), tmp.path().to_path_buf());
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .unwrap();
    worker.process(claimed).unwrap();

    let after = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(after.status, QueueStatus::Failed);
    assert_eq!(
        after.failure_reason,
        Some(MergeFailureReason::FastForwardFailed)
    );
}

/// CI gate: lint fails → entry ends Failed with CILintFailed and
/// neither test nor build ran.
#[test]
fn ci_lint_failure_short_circuits() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(make_fake_store());
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();

    // Use shell `false` to force a non-zero exit, simulating CI failure.
    let ci = RepoCiConfig {
        lint_command: Some("false".into()),
        test_command: Some("true".into()),
        build_command: Some("true".into()),
        ..Default::default()
    };
    let repo = fixture_repo(tmp.path().to_path_buf(), ci);
    store.insert_repo(&repo).unwrap();
    let entry = fixture_entry(&repo, "feat/ci");
    std::fs::create_dir_all(&entry.source_worktree).unwrap();
    store.enqueue(&entry).unwrap();

    let script = GitScript {
        rebase: vec![RebaseOutcome::Ok].into(),
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, _events, _clock) = build_deps(git, store.clone(), runs.clone());
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .unwrap();
    worker.process(claimed).unwrap();

    let after = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(after.status, QueueStatus::Failed);
    assert_eq!(after.failure_reason, Some(MergeFailureReason::CILintFailed));
    // CI lint log file should exist; test/build should not.
    let entry_dir = runs.join(entry.id.to_string());
    assert!(entry_dir.join("ci-lint.log").exists());
    assert!(!entry_dir.join("ci-test.log").exists());
    assert!(!entry_dir.join("ci-build.log").exists());
}

/// Full CI happy path: lint → test → build → FF.
#[test]
fn ci_all_steps_pass_then_merges() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(make_fake_store());
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();

    let ci = RepoCiConfig {
        lint_command: Some("true".into()),
        test_command: Some("true".into()),
        build_command: Some("true".into()),
        ..Default::default()
    };
    let repo = fixture_repo(tmp.path().to_path_buf(), ci);
    store.insert_repo(&repo).unwrap();
    let entry = fixture_entry(&repo, "feat/full-ci");
    std::fs::create_dir_all(&entry.source_worktree).unwrap();
    store.enqueue(&entry).unwrap();

    let script = GitScript {
        rebase: vec![RebaseOutcome::Ok].into(),
        fast_forward: vec![FastForwardOutcome::Ok].into(),
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, _events, _clock) = build_deps(git, store.clone(), runs.clone());
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .unwrap();
    worker.process(claimed).unwrap();

    let after = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(after.status, QueueStatus::Merged);
    let entry_dir = runs.join(entry.id.to_string());
    assert!(entry_dir.join("ci-lint.log").exists());
    assert!(entry_dir.join("ci-test.log").exists());
    assert!(entry_dir.join("ci-build.log").exists());
}
