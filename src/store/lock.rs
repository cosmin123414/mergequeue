//! TUI singleton lock via exclusive flock on `tui.pid`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use fs2::FileExt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuiLockError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("another MergeSmith TUI is already running (pid {pid})")]
    AlreadyHeld { pid: u32 },
}

pub struct TuiLock {
    file: File,
}

impl TuiLock {
    /// Try to acquire the singleton lock. If a stale PID file is held by
    /// a dead process, `try_lock_exclusive` succeeds and we overwrite the
    /// file with our own PID. If a live process holds it, return
    /// `AlreadyHeld` with the existing PID.
    pub fn acquire(path: &Path) -> Result<Self, TuiLockError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        if file.try_lock_exclusive().is_ok() {
            // We hold it; rewrite our PID.
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            writeln!(file, "{}", std::process::id())?;
            file.flush()?;
            Ok(Self { file })
        } else {
            // Read the existing PID for the error message.
            let mut s = String::new();
            let _ = file.read_to_string(&mut s);
            let pid = s.trim().parse::<u32>().unwrap_or(0);
            Err(TuiLockError::AlreadyHeld { pid })
        }
    }
}

impl Drop for TuiLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
