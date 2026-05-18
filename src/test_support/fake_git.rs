//! `FakeGit` — scripted `GitOps` for unit tests. Each method either
//! consumes one entry from a pre-programmed queue or returns the
//! default for that method.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::core::ports::{FastForwardOutcome, GitOps, RebaseOutcome};
use crate::error::Result;

#[derive(Default)]
pub struct GitScript {
    pub dirty: VecDeque<bool>,
    pub rebase: VecDeque<RebaseOutcome>,
    pub fast_forward: VecDeque<FastForwardOutcome>,
    pub worktree_exists: VecDeque<bool>,
    pub current_branch: VecDeque<String>,
    pub head_sha: VecDeque<String>,
    pub discover: VecDeque<PathBuf>,
    pub conflicted_files: VecDeque<Vec<String>>,
}

impl GitScript {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct FakeGit {
    script: Mutex<GitScript>,
    calls: Mutex<Vec<String>>,
}

impl FakeGit {
    pub fn new(script: GitScript) -> Self {
        Self {
            script: Mutex::new(script),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, s: impl Into<String>) {
        self.calls.lock().unwrap().push(s.into());
    }
}

impl GitOps for FakeGit {
    fn worktree_is_dirty(&self, path: &Path) -> Result<bool> {
        self.record(format!("worktree_is_dirty({})", path.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .dirty
            .pop_front()
            .unwrap_or(false))
    }

    fn current_branch(&self, worktree: &Path) -> Result<String> {
        self.record(format!("current_branch({})", worktree.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .current_branch
            .pop_front()
            .unwrap_or_else(|| "feat/x".into()))
    }

    fn head_sha(&self, worktree: &Path) -> Result<String> {
        self.record(format!("head_sha({})", worktree.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .head_sha
            .pop_front()
            .unwrap_or_else(|| "deadbeef".into()))
    }

    fn worktree_exists(&self, path: &Path) -> Result<bool> {
        self.record(format!("worktree_exists({})", path.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .worktree_exists
            .pop_front()
            .unwrap_or(true))
    }

    fn rebase_onto(&self, worktree: &Path, target_ref: &str) -> Result<RebaseOutcome> {
        self.record(format!("rebase_onto({}, {target_ref})", worktree.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .rebase
            .pop_front()
            .unwrap_or(RebaseOutcome::Ok))
    }

    fn abort_rebase(&self, worktree: &Path) -> Result<()> {
        self.record(format!("abort_rebase({})", worktree.display()));
        Ok(())
    }

    fn fast_forward(&self, target_worktree: &Path, source_ref: &str) -> Result<FastForwardOutcome> {
        self.record(format!(
            "fast_forward({}, {source_ref})",
            target_worktree.display()
        ));
        Ok(self
            .script
            .lock()
            .unwrap()
            .fast_forward
            .pop_front()
            .unwrap_or(FastForwardOutcome::Ok))
    }

    fn discover_worktree_root(&self, start: &Path) -> Result<PathBuf> {
        self.record(format!("discover_worktree_root({})", start.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .discover
            .pop_front()
            .unwrap_or_else(|| start.to_path_buf()))
    }

    fn conflicted_files(&self, worktree: &Path) -> Result<Vec<String>> {
        self.record(format!("conflicted_files({})", worktree.display()));
        Ok(self
            .script
            .lock()
            .unwrap()
            .conflicted_files
            .pop_front()
            .unwrap_or_default())
    }
}
