//! End-to-end queue lifecycle against a real git repo, real SQLite,
//! and the real `ProcessGit` adapter. This is the harness M2/M3
//! will extend.

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use mergequeue::agents::tmux::{ProcessTmux, TmuxOps};
use mergequeue::agents::DefaultAgentRegistry;
use mergequeue::core::agent_backend::AgentBackend;
use mergequeue::core::ids::{QueueEntryId, RepoId};
use mergequeue::core::ports::{AgentRegistry, Clock, GitOps, QueueStore};
use mergequeue::core::queue::{QueueEntry, QueueStatus};
use mergequeue::core::repo::{RegisteredRepo, RepoCiConfig};
use mergequeue::engine::events::EventBroadcaster;
use mergequeue::engine::shutdown::ShutdownToken;
use mergequeue::engine::worker::{Worker, WorkerDeps};
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

#[test]
fn worker_merges_a_clean_fast_forward() {
    // Set up a real git repo: main has commit C1; feat/x has C1+C2.
    let repo_dir = TempDir::new().unwrap();
    let state_root = TempDir::new().unwrap();

    sh(repo_dir.path(), &["git", "init", "-q", "-b", "main"]);
    sh(repo_dir.path(), &["git", "config", "user.email", "t@t"]);
    sh(repo_dir.path(), &["git", "config", "user.name", "T"]);
    std::fs::write(repo_dir.path().join("a.txt"), "1").unwrap();
    sh(repo_dir.path(), &["git", "add", "."]);
    sh(repo_dir.path(), &["git", "commit", "-q", "-m", "c1"]);

    sh(repo_dir.path(), &["git", "checkout", "-q", "-b", "feat/x"]);
    std::fs::write(repo_dir.path().join("b.txt"), "2").unwrap();
    sh(repo_dir.path(), &["git", "add", "."]);
    sh(repo_dir.path(), &["git", "commit", "-q", "-m", "c2"]);

    // Set HEAD back to main so the FF target is correct.
    sh(repo_dir.path(), &["git", "checkout", "-q", "main"]);

    // Build the engine.
    let store_path = state_root.path().join("state.sqlite");
    let store: Arc<dyn QueueStore> = Arc::new(SqliteStore::open(&store_path).unwrap());
    let git: Arc<dyn GitOps> = Arc::new(ProcessGit::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let events = Arc::new(EventBroadcaster::new());

    let repo = RegisteredRepo {
        id: RepoId::new(),
        root_path: repo_dir.path().to_path_buf(),
        default_branch: "main".into(),
        ci: RepoCiConfig::default(),
        agent_backend: AgentBackend::Opencode,
        created_at: OffsetDateTime::now_utc(),
        updated_at: OffsetDateTime::now_utc(),
    };
    store.insert_repo(&repo).unwrap();

    let entry = QueueEntry {
        id: QueueEntryId::new(),
        repo_id: repo.id,
        source_worktree: repo_dir.path().to_path_buf(),
        source_branch: "feat/x".into(),
        target_branch: "main".into(),
        status: QueueStatus::Queued,
        last_outcome: None,
        enqueued_at: clock.now(),
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

    // Tmux + agent registry are constructed but never called: this test
    // exercises a clean fast-forward path that doesn't hit `handoff`.
    let tmux_ops: Arc<dyn TmuxOps> = Arc::new(ProcessTmux::new());
    let agents: Arc<dyn AgentRegistry> = Arc::new(DefaultAgentRegistry::new(tmux_ops.clone()));

    let deps = WorkerDeps {
        store: store.clone(),
        git,
        clock,
        events,
        shutdown: ShutdownToken::new(),
        runs_dir: state_root.path().join("runs"),
        tmux: tmux_ops,
        agents,
        tmux_session_pid: std::process::id(),
    };
    let worker = Worker::new(repo.clone(), deps, Duration::from_millis(10));

    let claimed = store
        .claim_next(repo.id, std::process::id(), OffsetDateTime::now_utc())
        .unwrap()
        .expect("worker should claim the queued entry");
    worker.process(claimed).unwrap();

    let after = store.get_entry(entry.id).unwrap().unwrap();
    assert_eq!(after.status, QueueStatus::Merged, "{after:?}");

    // Verify the actual git state: main should now have b.txt.
    assert!(repo_dir.path().join("b.txt").exists());
}
