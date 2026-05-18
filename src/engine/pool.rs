//! `EnginePool`: one worker thread per registered repo.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::core::ports::{Clock, GitOps, QueueStore};
use crate::engine::events::EventBroadcaster;
use crate::engine::shutdown::ShutdownToken;
use crate::engine::worker::{Worker, WorkerDeps};
use crate::error::Result;

pub struct EnginePool {
    handles: Vec<JoinHandle<()>>,
    pub shutdown: ShutdownToken,
}

#[allow(clippy::too_many_arguments)]
impl EnginePool {
    /// Spawn one worker thread per repo currently in the store.
    ///
    /// `poll_interval` is the time a worker sleeps after finding no
    /// claimable entry.
    pub fn spawn(
        store: Arc<dyn QueueStore>,
        git: Arc<dyn GitOps>,
        clock: Arc<dyn Clock>,
        events: Arc<EventBroadcaster>,
        shutdown: ShutdownToken,
        runs_dir: std::path::PathBuf,
        poll_interval: Duration,
    ) -> Result<Self> {
        let repos = store.list_repos()?;
        let mut handles = Vec::with_capacity(repos.len());
        for repo in repos {
            let deps = WorkerDeps {
                store: store.clone(),
                git: git.clone(),
                clock: clock.clone(),
                events: events.clone(),
                shutdown: shutdown.clone(),
                runs_dir: runs_dir.clone(),
            };
            let worker = Worker::new(repo, deps, poll_interval);
            let h = std::thread::Builder::new()
                .name(format!("mergesmith-worker-{}", worker.repo.id.short()))
                .spawn(move || {
                    if let Err(e) = worker.run() {
                        tracing::error!(
                            "worker for repo {} exited with error: {e}",
                            worker.repo.id
                        );
                    }
                })
                .map_err(crate::error::Error::Io)?;
            handles.push(h);
        }
        Ok(Self { handles, shutdown })
    }

    /// Signal shutdown and join all worker threads. Returns once every
    /// worker has exited.
    pub fn shutdown_and_join(self) {
        self.shutdown.set_soft();
        for h in self.handles {
            let _ = h.join();
        }
    }
}
