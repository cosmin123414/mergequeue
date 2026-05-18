//! CI subprocess runner. Runs one configured command per call,
//! captures stdout+stderr to `runs/<entry-id>/ci-<step>.log`.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::core::queue::QueueEntry;
use crate::error::{Error, Result};

pub struct CiRunner<'a> {
    pub runs_dir: &'a Path,
}

impl<'a> CiRunner<'a> {
    pub fn new(runs_dir: &'a Path) -> Self {
        Self { runs_dir }
    }

    fn entry_dir(&self, entry: &QueueEntry) -> PathBuf {
        self.runs_dir.join(entry.id.to_string())
    }

    /// Run one CI step in `workdir`. Returns `true` on exit-zero, `false`
    /// on any non-zero exit. Captures stdout+stderr to the run-log file.
    /// The shell is `/bin/sh -c <cmd>` on Unix.
    pub fn run(&self, step: &str, cmd: &str, workdir: &Path, entry: &QueueEntry) -> Result<bool> {
        let dir = self.entry_dir(entry);
        fs::create_dir_all(&dir)?;
        let log_path = dir.join(format!("ci-{step}.log"));
        let log = File::create(&log_path)?;
        let log_err = log.try_clone()?;

        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(workdir)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .status()
            .map_err(|e| Error::other(format!("spawn ci `{cmd}`: {e}")))?;

        Ok(status.success())
    }
}
