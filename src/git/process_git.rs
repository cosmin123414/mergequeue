//! `ProcessGit`: shells out to the user's `git` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::core::ports::{FastForwardOutcome, GitOps, RebaseOutcome};
use crate::error::{Error, Result};

#[derive(Debug, Default, Clone)]
pub struct ProcessGit {
    /// Path to `git`. If `None`, we let `PATH` resolution happen.
    git_path: Option<PathBuf>,
}

impl ProcessGit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_git_path(mut self, p: PathBuf) -> Self {
        self.git_path = Some(p);
        self
    }

    fn cmd(&self, workdir: &Path) -> Command {
        let mut c = match &self.git_path {
            Some(p) => Command::new(p),
            None => Command::new("git"),
        };
        c.current_dir(workdir);
        // Make output deterministic and locale-independent.
        c.env("LC_ALL", "C");
        c.env("GIT_TERMINAL_PROMPT", "0");
        c
    }

    fn run(&self, workdir: &Path, args: &[&str]) -> Result<Output> {
        let out = self
            .cmd(workdir)
            .args(args)
            .output()
            .map_err(|e| Error::Git(format!("spawn git: {e}")))?;
        Ok(out)
    }

    fn stdout_string(out: &Output) -> String {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn stderr_string(out: &Output) -> String {
        String::from_utf8_lossy(&out.stderr).trim().to_string()
    }
}

impl GitOps for ProcessGit {
    fn worktree_is_dirty(&self, path: &Path) -> Result<bool> {
        let out = self.run(path, &["status", "--porcelain"])?;
        if !out.status.success() {
            return Err(Error::Git(format!(
                "git status failed: {}",
                Self::stderr_string(&out)
            )));
        }
        Ok(!Self::stdout_string(&out).is_empty())
    }

    fn current_branch(&self, worktree: &Path) -> Result<String> {
        let out = self.run(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        if !out.status.success() {
            return Err(Error::Git(format!(
                "git rev-parse failed: {}",
                Self::stderr_string(&out)
            )));
        }
        Ok(Self::stdout_string(&out))
    }

    fn head_sha(&self, worktree: &Path) -> Result<String> {
        let out = self.run(worktree, &["rev-parse", "HEAD"])?;
        if !out.status.success() {
            return Err(Error::Git(format!(
                "git rev-parse HEAD failed: {}",
                Self::stderr_string(&out)
            )));
        }
        Ok(Self::stdout_string(&out))
    }

    fn worktree_exists(&self, path: &Path) -> Result<bool> {
        // A worktree exists if it has a .git entry (file or dir) and `git`
        // recognizes it as a working tree.
        if !path.exists() {
            return Ok(false);
        }
        let out = self.run(path, &["rev-parse", "--is-inside-work-tree"])?;
        Ok(out.status.success() && Self::stdout_string(&out) == "true")
    }

    fn rebase_onto(&self, worktree: &Path, target_ref: &str) -> Result<RebaseOutcome> {
        let out = self.run(worktree, &["rebase", target_ref])?;
        if out.status.success() {
            return Ok(RebaseOutcome::Ok);
        }
        let stderr = Self::stderr_string(&out);
        let stdout = Self::stdout_string(&out);
        let combined = format!("{stdout}\n{stderr}");
        // Heuristic: any of these substrings indicates conflicts. Git's
        // exit codes don't differentiate cleanly between conflict and
        // genuine error, so we string-match on the human messages.
        if combined.contains("CONFLICT")
            || combined.contains("could not apply")
            || combined.contains("merge conflict")
        {
            return Ok(RebaseOutcome::Conflict);
        }
        Ok(RebaseOutcome::OtherError(combined.trim().to_string()))
    }

    fn abort_rebase(&self, worktree: &Path) -> Result<()> {
        let out = self.run(worktree, &["rebase", "--abort"])?;
        if !out.status.success() {
            return Err(Error::Git(format!(
                "git rebase --abort failed: {}",
                Self::stderr_string(&out)
            )));
        }
        Ok(())
    }

    fn fast_forward(&self, target_worktree: &Path, source_ref: &str) -> Result<FastForwardOutcome> {
        let out = self.run(target_worktree, &["merge", "--ff-only", source_ref])?;
        if out.status.success() {
            return Ok(FastForwardOutcome::Ok);
        }
        let combined = format!(
            "{}\n{}",
            Self::stdout_string(&out),
            Self::stderr_string(&out)
        );
        if combined.contains("Not possible to fast-forward")
            || combined.contains("non-fast-forward")
        {
            return Ok(FastForwardOutcome::NonFastForward);
        }
        Ok(FastForwardOutcome::OtherError(combined.trim().to_string()))
    }

    fn discover_worktree_root(&self, start: &Path) -> Result<PathBuf> {
        let out = self.run(start, &["rev-parse", "--show-toplevel"])?;
        if !out.status.success() {
            return Err(Error::NotAGitRepo(start.display().to_string()));
        }
        let s = Self::stdout_string(&out);
        if s.is_empty() {
            return Err(Error::NotAGitRepo(start.display().to_string()));
        }
        Ok(PathBuf::from(s))
    }
}
