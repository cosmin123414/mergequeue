//! `EnginePool`: one worker thread per registered repo, with live
//! reconciliation against the `registered_repos` table.
//!
//! Why this shape:
//!
//! - MergeSmith has no daemon. The pool only runs inside `mergesmith
//!   tui` (M4) and inside engine tests.
//! - The user can `mergesmith init` / `mergesmith repos remove` from
//!   another terminal while the TUI is up. The TUI's render loop calls
//!   `pool.reconcile()` periodically so the worker set tracks the store.
//! - We give each worker its own `ShutdownToken` so removing one repo
//!   stops only that worker. The pool also holds a "global" token; the
//!   TUI's quit handler sets it to bring down everything at once.
//!
//! Concurrency:
//!
//! - `SqliteStore` is `Send + Sync` and serializes its internal
//!   `Mutex<Connection>` per call. Workers do not hold the mutex during
//!   long ops (rebase / CI subprocess), so the per-call serialization
//!   is irrelevant in practice.
//! - `claim_next` is scoped by `repo_id`; two workers for two different
//!   repos never compete for the same row.
//! - One worker per repo => no concurrent claims within a repo, so
//!   FIFO ordering is preserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::agents::tmux::TmuxOps;
use crate::core::ids::RepoId;
use crate::core::ports::{AgentRegistry, Clock, GitOps, QueueStore};
use crate::core::repo::RegisteredRepo;
use crate::engine::events::EventBroadcaster;
use crate::engine::shutdown::ShutdownToken;
use crate::engine::worker::{Worker, WorkerDeps};
use crate::error::Result;

/// Shared deps a pool hands to every worker it spawns.
#[derive(Clone)]
pub struct PoolDeps {
    pub store: Arc<dyn QueueStore>,
    pub git: Arc<dyn GitOps>,
    pub clock: Arc<dyn Clock>,
    pub events: Arc<EventBroadcaster>,
    pub runs_dir: std::path::PathBuf,
    pub poll_interval: Duration,
    pub tmux: Arc<dyn TmuxOps>,
    pub agents: Arc<dyn AgentRegistry>,
    pub tmux_session_pid: u32,
}

/// Outcome of a single `reconcile()` call. Useful for the TUI: log
/// when workers spin up or wind down.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub started: Vec<RepoId>,
    pub stopped: Vec<RepoId>,
}

struct WorkerHandle {
    join: JoinHandle<()>,
    shutdown: ShutdownToken,
}

pub struct EnginePool {
    /// Per-repo worker handles. Wrapped in a `Mutex` so the TUI thread
    /// and a reconcile-on-tick caller don't race.
    workers: Mutex<HashMap<RepoId, WorkerHandle>>,
    deps: PoolDeps,
    /// Fires when the whole pool is being torn down. Workers OR-test
    /// this with their own per-repo token.
    global: ShutdownToken,
}

impl EnginePool {
    pub fn new(deps: PoolDeps) -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            deps,
            global: ShutdownToken::new(),
        }
    }

    /// Returns the global shutdown token. Setting it brings the whole
    /// pool down; the TUI's quit handler calls `.set_soft()` on this.
    pub fn shutdown_token(&self) -> ShutdownToken {
        self.global.clone()
    }

    /// Reconcile the running worker set to the current `registered_repos`
    /// table. Adds workers for new repos; drops workers for repos no
    /// longer present.
    ///
    /// Idempotent: calling on an already-converged pool is a no-op.
    pub fn reconcile(&self) -> Result<Reconciliation> {
        let repos = self.deps.store.list_repos()?;
        let desired: HashMap<RepoId, RegisteredRepo> =
            repos.into_iter().map(|r| (r.id, r)).collect();

        let mut report = Reconciliation::default();
        let mut workers = self.workers.lock().expect("pool mutex poisoned");

        // Stop workers whose repo is gone.
        let stale: Vec<RepoId> = workers
            .keys()
            .filter(|id| !desired.contains_key(id))
            .copied()
            .collect();
        for id in stale {
            if let Some(handle) = workers.remove(&id) {
                handle.shutdown.set_soft();
                // Join inline. If a single worker blocks shutdown we'd
                // rather notice now than leak threads.
                let _ = handle.join.join();
                report.stopped.push(id);
            }
        }

        // Start workers for repos that don't yet have one. We honour
        // the global shutdown token: if the pool itself is winding down,
        // we don't spin up anything new.
        if !self.global.is_set() {
            for (id, repo) in &desired {
                if workers.contains_key(id) {
                    continue;
                }
                let handle = spawn_worker(repo.clone(), &self.deps, self.global.clone())?;
                workers.insert(*id, handle);
                report.started.push(*id);
            }
        }

        Ok(report)
    }

    /// Total number of live worker threads in the pool.
    pub fn worker_count(&self) -> usize {
        self.workers.lock().expect("pool mutex poisoned").len()
    }

    /// Signal global shutdown and join every worker. Returns once
    /// every worker has exited.
    pub fn shutdown_and_join(self) {
        self.global.set_soft();
        // Per-worker tokens chain off the global one inside the worker
        // loop, so setting `global` is enough; setting per-worker
        // tokens here is also fine and a touch faster.
        let mut workers = self.workers.into_inner().expect("pool mutex poisoned");
        for (_, handle) in workers.drain() {
            handle.shutdown.set_soft();
            let _ = handle.join.join();
        }
    }
}

fn spawn_worker(
    repo: RegisteredRepo,
    deps: &PoolDeps,
    global: ShutdownToken,
) -> Result<WorkerHandle> {
    // The worker checks one token that observes BOTH a per-repo source
    // and the pool-global source. Setting the per-repo token stops
    // only this worker; setting the global stops every worker.
    let per_repo = ShutdownToken::new();
    let combined = ShutdownToken::merged([per_repo.clone(), global]);

    let deps_for_worker = WorkerDeps {
        store: deps.store.clone(),
        git: deps.git.clone(),
        clock: deps.clock.clone(),
        events: deps.events.clone(),
        shutdown: combined,
        runs_dir: deps.runs_dir.clone(),
        tmux: deps.tmux.clone(),
        agents: deps.agents.clone(),
        tmux_session_pid: deps.tmux_session_pid,
    };
    let poll_interval = deps.poll_interval;
    let worker = Worker::new(repo, deps_for_worker, poll_interval);
    let repo_short = worker.repo.id.short();

    let join = std::thread::Builder::new()
        .name(format!("mergesmith-worker-{repo_short}"))
        .spawn(move || {
            if let Err(e) = worker.run() {
                tracing::error!("worker for repo {} exited with error: {e}", worker.repo.id);
            }
        })
        .map_err(crate::error::Error::Io)?;

    Ok(WorkerHandle {
        join,
        shutdown: per_repo,
    })
}
