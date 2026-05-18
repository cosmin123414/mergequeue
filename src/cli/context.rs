//! `CliContext`: bundles the bits every subcommand needs (paths, store,
//! git, clock).

use std::path::PathBuf;
use std::sync::Arc;

use crate::core::ports::{Clock, GitOps, QueueStore};
use crate::error::Result;
use crate::git::ProcessGit;
use crate::paths;
use crate::store::{SqliteStore, SystemClock};

pub struct CliContext {
    pub state_root: PathBuf,
    pub store: Arc<dyn QueueStore>,
    pub git: Arc<dyn GitOps>,
    pub clock: Arc<dyn Clock>,
}

impl CliContext {
    pub fn open() -> Result<Self> {
        let state_root = paths::state_root()?;
        paths::ensure_state_root(&state_root)?;
        let store: Arc<dyn QueueStore> =
            Arc::new(SqliteStore::open(&paths::sqlite_path(&state_root))?);
        let git: Arc<dyn GitOps> = Arc::new(ProcessGit::new());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        Ok(Self {
            state_root,
            store,
            git,
            clock,
        })
    }
}
