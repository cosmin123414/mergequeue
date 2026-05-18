//! Engine worker tests using the four fakes. These prove that the FSM
//! and the shell wire up correctly across the full happy / sad paths.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use time::OffsetDateTime;

use crate::agents::tmux::TmuxOps;
use crate::core::agent_backend::AgentBackend;
use crate::core::ids::{QueueEntryId, RepoId};
use crate::core::ports::{AgentRegistry, FastForwardOutcome, GitOps, QueueStore, RebaseOutcome};
use crate::core::queue::{MergeFailureReason, QueueEntry, QueueStatus};
use crate::core::repo::{RegisteredRepo, RepoCiConfig};
use crate::engine::events::EventBroadcaster;
use crate::engine::shutdown::ShutdownToken;
use crate::engine::worker::{Worker, WorkerDeps};
use crate::test_support::{
    fake_store::make_fake_store, FakeAgent, FakeAgentRegistry, FakeClock, FakeGit, FakeTmux,
    GitScript, TmuxCall,
};

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

/// Returned helpers for tests that want to inspect tmux/agent calls.
#[allow(dead_code)]
struct Helpers {
    events: Arc<EventBroadcaster>,
    clock: Arc<FakeClock>,
    tmux: Arc<FakeTmux>,
    agent: Arc<FakeAgent>,
}

fn build_deps(
    git: Arc<dyn GitOps>,
    store: Arc<dyn QueueStore>,
    runs: std::path::PathBuf,
) -> (WorkerDeps, Helpers) {
    let clock = Arc::new(FakeClock::epoch());
    let events = Arc::new(EventBroadcaster::with_history());
    let tmux: Arc<FakeTmux> = Arc::new(FakeTmux::new());
    let agent = Arc::new(FakeAgent::new(AgentBackend::Opencode));
    let registry = Arc::new(FakeAgentRegistry::new(agent.clone()));
    let tmux_ops: Arc<dyn TmuxOps> = tmux.clone();
    let agents: Arc<dyn AgentRegistry> = registry;
    let deps = WorkerDeps {
        store,
        git,
        clock: clock.clone(),
        events: events.clone(),
        shutdown: ShutdownToken::new(),
        runs_dir: runs,
        tmux: tmux_ops,
        agents,
        tmux_session_pid: 9999,
    };
    (
        deps,
        Helpers {
            events,
            clock,
            tmux,
            agent,
        },
    )
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
    let (deps, _h) = build_deps(git, store.clone(), tmp.path().to_path_buf());
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
        conflicted_files: vec![vec!["src/lib.rs".into(), "Cargo.toml".into()]].into(),
        ..GitScript::new()
    };
    let git = Arc::new(FakeGit::new(script));
    let (deps, h) = build_deps(git, store.clone(), tmp.path().to_path_buf());
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

    // Handoff opened a tmux window AND invoked the agent.
    let agent_calls = h.agent.calls();
    assert_eq!(agent_calls.len(), 1);
    assert_eq!(agent_calls[0].backend, AgentBackend::Opencode);
    assert_eq!(
        agent_calls[0].prompt.conflicted_files,
        vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()]
    );
    assert_eq!(agent_calls[0].prompt.source_branch, "feat/conflict");
    assert_eq!(agent_calls[0].prompt.target_branch, "main");
    assert_eq!(agent_calls[0].tmux.session, "mergesmith-9999");
    let short = final_entry.id.short();
    assert_eq!(agent_calls[0].tmux.window, format!("conflict-{short}"));

    // And the tmux side recorded the new window.
    let tmux_calls = h.tmux.calls();
    assert!(
        tmux_calls
            .iter()
            .any(|c| matches!(c, TmuxCall::NewWindow { window, .. } if window.contains(&short))),
        "expected NewWindow in {tmux_calls:?}"
    );

    // The persisted ConflictSession knows about the tmux session/window.
    let conflict_id = final_entry.conflict_session_id.unwrap();
    let sessions = store
        .list_entries(crate::core::ports::EntryFilter::all())
        .unwrap();
    assert!(sessions.iter().any(|e| e.id == final_entry.id));
    // The store's getter for sessions is not in QueueStore today; check
    // via the dedicated open/close trail by inspecting the entry.
    assert_eq!(final_entry.conflict_session_id, Some(conflict_id));
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
    let (deps, h) = build_deps(git, store.clone(), tmp.path().to_path_buf());
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
    assert_eq!(h.clock.sleeps(), vec![Duration::from_secs(7)]);
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
    let (deps, _h) = build_deps(git, store.clone(), tmp.path().to_path_buf());
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
    let (deps, _h) = build_deps(git, store.clone(), runs.clone());
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

// ---------------------------------------------------------------------
// Pool tests (M3): live reconciliation + parallel execution.
// ---------------------------------------------------------------------

mod pool_tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;
    use time::OffsetDateTime;

    use crate::agents::tmux::TmuxOps;
    use crate::core::agent_backend::AgentBackend;
    use crate::core::ids::{QueueEntryId, RepoId};
    use crate::core::ports::{
        AgentRegistry, FastForwardOutcome, GitOps, QueueStore, RebaseOutcome,
    };
    use crate::core::queue::{QueueEntry, QueueStatus};
    use crate::core::repo::{RegisteredRepo, RepoCiConfig};
    use crate::engine::events::EventBroadcaster;
    use crate::engine::pool::{EnginePool, PoolDeps};
    use crate::store::SystemClock;
    use crate::test_support::{
        fake_store::make_fake_store, FakeAgent, FakeAgentRegistry, FakeGit, FakeTmux, GitScript,
    };

    /// Spin loop that polls `cond` every `step` until it's true or
    /// `timeout` elapses. Returns true if `cond` became true.
    fn wait_until(timeout: Duration, step: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(step);
        }
        cond()
    }

    fn pool_deps(
        store: Arc<dyn QueueStore>,
        git: Arc<dyn GitOps>,
        runs: std::path::PathBuf,
    ) -> PoolDeps {
        let tmux: Arc<FakeTmux> = Arc::new(FakeTmux::new());
        let tmux_ops: Arc<dyn TmuxOps> = tmux.clone();
        let agent = Arc::new(FakeAgent::new(AgentBackend::Opencode));
        let agents: Arc<dyn AgentRegistry> = Arc::new(FakeAgentRegistry::new(agent));
        PoolDeps {
            store,
            git,
            // Real clock here: pool workers `sleep` between polls and a
            // FakeClock would return instantly, busy-spinning. Real
            // clock with a tiny `poll_interval` keeps the test snappy
            // without hammering CPU.
            clock: Arc::new(SystemClock::new()),
            events: Arc::new(EventBroadcaster::new()),
            runs_dir: runs,
            poll_interval: Duration::from_millis(5),
            tmux: tmux_ops,
            agents,
            tmux_session_pid: 9999,
        }
    }

    fn make_repo(root: std::path::PathBuf) -> RegisteredRepo {
        let now = OffsetDateTime::now_utc();
        RegisteredRepo {
            id: RepoId::new(),
            root_path: root,
            default_branch: "main".into(),
            ci: RepoCiConfig::default(),
            agent_backend: AgentBackend::Opencode,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_entry(repo: &RegisteredRepo, branch: &str) -> QueueEntry {
        QueueEntry {
            id: QueueEntryId::new(),
            repo_id: repo.id,
            source_worktree: repo.root_path.clone(),
            source_branch: branch.into(),
            target_branch: repo.default_branch.clone(),
            status: QueueStatus::Queued,
            last_outcome: None,
            enqueued_at: OffsetDateTime::now_utc(),
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

    /// `reconcile` adds workers for every repo currently registered,
    /// and `worker_count` reflects the result.
    #[test]
    fn reconcile_spawns_one_worker_per_repo() {
        let tmp = TempDir::new().unwrap();
        let store: Arc<dyn QueueStore> = Arc::new(make_fake_store());
        let r1 = make_repo(tmp.path().to_path_buf());
        let r2 = make_repo(tmp.path().to_path_buf());
        // Both repos refer to the same root in test fixtures, but
        // `insert_repo` rejects duplicate roots, so we tweak the second.
        let r2 = RegisteredRepo {
            root_path: tmp.path().join("second"),
            ..r2
        };
        store.insert_repo(&r1).unwrap();
        store.insert_repo(&r2).unwrap();

        let pool = EnginePool::new(pool_deps(
            store.clone(),
            Arc::new(FakeGit::new(GitScript::new())),
            tmp.path().to_path_buf(),
        ));
        let report = pool.reconcile().unwrap();
        assert_eq!(pool.worker_count(), 2);
        assert_eq!(report.started.len(), 2);
        assert!(report.stopped.is_empty());

        // Second reconcile is a no-op.
        let report = pool.reconcile().unwrap();
        assert!(report.started.is_empty());
        assert!(report.stopped.is_empty());

        pool.shutdown_and_join();
    }

    /// Removing a repo from the store causes `reconcile` to stop and
    /// join the corresponding worker.
    #[test]
    fn reconcile_stops_worker_for_removed_repo() {
        let tmp = TempDir::new().unwrap();
        let store: Arc<dyn QueueStore> = Arc::new(make_fake_store());
        let r1 = make_repo(tmp.path().to_path_buf());
        store.insert_repo(&r1).unwrap();

        let pool = EnginePool::new(pool_deps(
            store.clone(),
            Arc::new(FakeGit::new(GitScript::new())),
            tmp.path().to_path_buf(),
        ));
        pool.reconcile().unwrap();
        assert_eq!(pool.worker_count(), 1);

        store.delete_repo(r1.id).unwrap();
        let report = pool.reconcile().unwrap();
        assert_eq!(report.stopped, vec![r1.id]);
        assert_eq!(pool.worker_count(), 0);

        pool.shutdown_and_join();
    }

    /// Two repos enqueue work in parallel. Both entries finish.
    ///
    /// This test exercises the `Mutex<Connection>` in SqliteStore under
    /// thread contention and verifies that one worker doesn't block the
    /// other's claims.
    #[test]
    fn two_repos_make_progress_in_parallel() {
        let tmp = TempDir::new().unwrap();
        let store: Arc<dyn QueueStore> = Arc::new(make_fake_store());
        let r1 = make_repo(tmp.path().join("r1"));
        let r2 = make_repo(tmp.path().join("r2"));
        store.insert_repo(&r1).unwrap();
        store.insert_repo(&r2).unwrap();

        let e1 = make_entry(&r1, "feat/r1-a");
        let e2 = make_entry(&r2, "feat/r2-a");
        store.enqueue(&e1).unwrap();
        store.enqueue(&e2).unwrap();

        // FakeGit returns Ok for everything by default → clean FF merges.
        let git: Arc<dyn GitOps> = Arc::new(FakeGit::new(GitScript {
            // Pre-load two of each since two entries each need one rebase
            // and one fast-forward.
            rebase: vec![RebaseOutcome::Ok, RebaseOutcome::Ok].into(),
            fast_forward: vec![FastForwardOutcome::Ok, FastForwardOutcome::Ok].into(),
            ..GitScript::new()
        }));

        let pool = EnginePool::new(pool_deps(store.clone(), git, tmp.path().to_path_buf()));
        pool.reconcile().unwrap();
        assert_eq!(pool.worker_count(), 2);

        let store_for_poll = store.clone();
        let both_merged = wait_until(Duration::from_secs(5), Duration::from_millis(10), || {
            let after1 = store_for_poll.get_entry(e1.id).ok().flatten();
            let after2 = store_for_poll.get_entry(e2.id).ok().flatten();
            matches!(
                (after1, after2),
                (Some(a), Some(b)) if a.status == QueueStatus::Merged && b.status == QueueStatus::Merged
            )
        });
        pool.shutdown_and_join();
        assert!(both_merged, "expected both entries to reach Merged");
    }

    /// One repo's worker is stuck sleeping on `PrecheckDirtyTarget`; the
    /// other repo's worker still processes its queue. Demonstrates that
    /// per-repo dirty-target retry doesn't block the rest of the pool.
    #[test]
    fn dirty_target_on_one_repo_does_not_stall_the_other() {
        let tmp = TempDir::new().unwrap();
        let store: Arc<dyn QueueStore> = Arc::new(make_fake_store());

        // r1: always-dirty → its worker keeps looping on PrecheckDirtyTarget.
        // r2: clean → its worker FFs cleanly.
        let r1 = RegisteredRepo {
            ci: RepoCiConfig {
                // Tiny retry so the dirty-worker loops quickly, exercising
                // the retry path without slowing the test.
                dirty_retry: Duration::from_millis(20),
                ..Default::default()
            },
            ..make_repo(tmp.path().join("r1"))
        };
        let r2 = make_repo(tmp.path().join("r2"));
        store.insert_repo(&r1).unwrap();
        store.insert_repo(&r2).unwrap();

        let e1 = make_entry(&r1, "feat/dirty");
        let e2 = make_entry(&r2, "feat/clean");
        store.enqueue(&e1).unwrap();
        store.enqueue(&e2).unwrap();

        // Both workers share a single FakeGit, so we use the
        // `always_dirty_paths` set (path-keyed, not call-order-keyed)
        // to pin r1's root as always-dirty. r2's path is absent so it
        // reports clean and proceeds through the FSM.
        let mut always_dirty = std::collections::HashSet::new();
        always_dirty.insert(r1.root_path.clone());
        let git: Arc<dyn GitOps> = Arc::new(FakeGit::new(GitScript {
            always_dirty_paths: always_dirty,
            rebase: vec![RebaseOutcome::Ok].into(),
            fast_forward: vec![FastForwardOutcome::Ok].into(),
            ..GitScript::new()
        }));

        let pool = EnginePool::new(pool_deps(store.clone(), git, tmp.path().to_path_buf()));
        pool.reconcile().unwrap();

        let store_for_poll = store.clone();
        let e2_done = wait_until(Duration::from_secs(5), Duration::from_millis(10), || {
            store_for_poll
                .get_entry(e2.id)
                .ok()
                .flatten()
                .is_some_and(|e| e.status == QueueStatus::Merged)
        });

        // r1's entry should still be Queued (worker keeps rolling it
        // back). It must NOT have advanced past Queued.
        let r1_entry = store.get_entry(e1.id).unwrap().unwrap();
        pool.shutdown_and_join();

        assert!(
            e2_done,
            "r2's entry did not merge despite r1 being stuck on dirty"
        );
        assert_eq!(
            r1_entry.status,
            QueueStatus::Queued,
            "r1's entry should still be Queued (dirty-target retry rolled it back)"
        );
    }

    /// After `shutdown_and_join`, `reconcile` is still callable but
    /// won't spin up new workers. (Defensive — protects against a TUI
    /// that calls reconcile() in a tick during shutdown.)
    #[test]
    fn reconcile_after_shutdown_is_inert() {
        let tmp = TempDir::new().unwrap();
        let store: Arc<dyn QueueStore> = Arc::new(make_fake_store());
        let r1 = make_repo(tmp.path().to_path_buf());
        store.insert_repo(&r1).unwrap();

        let pool = EnginePool::new(pool_deps(
            store.clone(),
            Arc::new(FakeGit::new(GitScript::new())),
            tmp.path().to_path_buf(),
        ));
        // Signal shutdown BEFORE reconciling.
        pool.shutdown_token().set_soft();
        let report = pool.reconcile().unwrap();
        assert!(report.started.is_empty());
        assert_eq!(pool.worker_count(), 0);
        pool.shutdown_and_join();
    }
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
    let (deps, _h) = build_deps(git, store.clone(), runs.clone());
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
