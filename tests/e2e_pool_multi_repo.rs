//! M3 e2e: spin an `EnginePool` over two real git repos and verify
//! both entries reach `Merged` in parallel.
//!
//! This is the integration-level guarantee for "multiple registered
//! repos; per-repo workers running in parallel" from
//! `docs/13-milestones.md`. Unit tests (`engine::tests::pool_tests`)
//! cover reconciliation and dirty-target isolation with fakes; this
//! test pins down the contract end-to-end against real `git` +
//! `SqliteStore` + `ProcessGit`.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mergequeue::agents::tmux::{ProcessTmux, TmuxOps};
use mergequeue::agents::DefaultAgentRegistry;
use mergequeue::core::agent_backend::AgentBackend;
use mergequeue::core::ids::{QueueEntryId, RepoId};
use mergequeue::core::ports::{AgentRegistry, Clock, GitOps, QueueStore};
use mergequeue::core::queue::{QueueEntry, QueueStatus};
use mergequeue::core::repo::{RegisteredRepo, RepoCiConfig};
use mergequeue::engine::events::EventBroadcaster;
use mergequeue::engine::pool::{EnginePool, PoolDeps};
use mergequeue::git::ProcessGit;
use mergequeue::store::{SqliteStore, SystemClock};
use tempfile::TempDir;
use time::OffsetDateTime;

fn sh(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new(args[0])
        .args(&args[1..])
        .current_dir(dir)
        .status()
        .expect("spawn");
    assert!(
        status.success(),
        "command {args:?} failed in {}",
        dir.display()
    );
}

/// Build a git repo with `main` (commit C1) and a feature branch
/// (`feat/x`) holding C1+C2. Leaves HEAD pointing at `main`.
fn make_ff_ready_repo(root: &std::path::Path, branch: &str) {
    sh(root, &["git", "init", "-q", "-b", "main"]);
    sh(root, &["git", "config", "user.email", "t@t"]);
    sh(root, &["git", "config", "user.name", "T"]);
    std::fs::write(root.join("a.txt"), "1").unwrap();
    sh(root, &["git", "add", "."]);
    sh(root, &["git", "commit", "-q", "-m", "c1"]);

    sh(root, &["git", "checkout", "-q", "-b", branch]);
    std::fs::write(root.join("b.txt"), "2").unwrap();
    sh(root, &["git", "add", "."]);
    sh(root, &["git", "commit", "-q", "-m", "c2"]);

    sh(root, &["git", "checkout", "-q", "main"]);
}

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

#[test]
fn pool_merges_two_independent_repos_in_parallel() {
    let r1_dir = TempDir::new().unwrap();
    let r2_dir = TempDir::new().unwrap();
    let state_root = TempDir::new().unwrap();

    make_ff_ready_repo(r1_dir.path(), "feat/x");
    make_ff_ready_repo(r2_dir.path(), "feat/y");

    let store_path = state_root.path().join("state.sqlite");
    let store: Arc<dyn QueueStore> = Arc::new(SqliteStore::open(&store_path).unwrap());
    let git: Arc<dyn GitOps> = Arc::new(ProcessGit::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let tmux_ops: Arc<dyn TmuxOps> = Arc::new(ProcessTmux::new());
    let agents: Arc<dyn AgentRegistry> = Arc::new(DefaultAgentRegistry::new(tmux_ops.clone()));

    let now = OffsetDateTime::now_utc();
    let r1 = RegisteredRepo {
        id: RepoId::new(),
        root_path: r1_dir.path().to_path_buf(),
        default_branch: "main".into(),
        ci: RepoCiConfig::default(),
        agent_backend: AgentBackend::Opencode,
        created_at: now,
        updated_at: now,
    };
    let r2 = RegisteredRepo {
        id: RepoId::new(),
        root_path: r2_dir.path().to_path_buf(),
        default_branch: "main".into(),
        ci: RepoCiConfig::default(),
        agent_backend: AgentBackend::Opencode,
        created_at: now,
        updated_at: now,
    };
    store.insert_repo(&r1).unwrap();
    store.insert_repo(&r2).unwrap();

    let mk_entry = |repo: &RegisteredRepo, branch: &str| QueueEntry {
        id: QueueEntryId::new(),
        repo_id: repo.id,
        source_worktree: repo.root_path.clone(),
        source_branch: branch.into(),
        target_branch: "main".into(),
        status: QueueStatus::Queued,
        last_outcome: None,
        enqueued_at: now,
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
    let e1 = mk_entry(&r1, "feat/x");
    let e2 = mk_entry(&r2, "feat/y");
    store.enqueue(&e1).unwrap();
    store.enqueue(&e2).unwrap();

    let pool = EnginePool::new(PoolDeps {
        store: store.clone(),
        git,
        clock,
        events: Arc::new(EventBroadcaster::new()),
        runs_dir: state_root.path().join("runs"),
        poll_interval: Duration::from_millis(10),
        tmux: tmux_ops,
        agents,
        tmux_session_pid: std::process::id(),
    });
    pool.reconcile().unwrap();
    assert_eq!(pool.worker_count(), 2);

    let store_for_poll = store.clone();
    let both = wait_until(Duration::from_secs(10), Duration::from_millis(25), || {
        let a = store_for_poll
            .get_entry(e1.id)
            .ok()
            .flatten()
            .map(|e| e.status);
        let b = store_for_poll
            .get_entry(e2.id)
            .ok()
            .flatten()
            .map(|e| e.status);
        matches!(
            (a, b),
            (Some(QueueStatus::Merged), Some(QueueStatus::Merged))
        )
    });
    pool.shutdown_and_join();
    assert!(both, "expected both entries to reach Merged");

    // Verify the real git state: b.txt landed in r1/main, b.txt in r2/main.
    sh(r1_dir.path(), &["git", "checkout", "-q", "main"]);
    sh(r2_dir.path(), &["git", "checkout", "-q", "main"]);
    assert!(
        r1_dir.path().join("b.txt").exists(),
        "r1 main missing b.txt"
    );
    assert!(
        r2_dir.path().join("b.txt").exists(),
        "r2 main missing b.txt"
    );
}
