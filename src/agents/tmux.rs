//! `TmuxOps`: a minimal abstraction over the bits of `tmux(1)` MergeQueue
//! needs to manage detached agent sessions.
//!
//! This is intentionally **not** one of the four core seam traits — only
//! the agents subsystem talks tmux. It lives here so we can write a
//! `FakeTmux` for unit tests without dragging tmux into `core::ports`.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::core::ports::TmuxHandle;
use crate::error::{Error, Result};

/// Operations on a tmux server.
///
/// All methods are best-effort: they shell out to `tmux(1)` and surface
/// non-zero exits as `Error::tmux(...)`. Implementations must be
/// `Send + Sync` so the engine can hold one in an `Arc`.
pub trait TmuxOps: Send + Sync {
    /// Create a detached session if it doesn't already exist. No-op if it
    /// does. Returns the session name actually created.
    fn ensure_session(&self, session: &str) -> Result<()>;

    /// `tmux has-session -t <session>`. `true` iff the session exists.
    fn has_session(&self, session: &str) -> Result<bool>;

    /// Open a new named window inside `session`, with `cwd` as its working
    /// directory. Does NOT spawn a program — the window opens a default
    /// shell. Returns a `TmuxHandle` you can later target with `send_keys`.
    fn new_window(&self, session: &str, window: &str, cwd: &Path) -> Result<TmuxHandle>;

    /// `tmux send-keys -t <session>:<window> <keys> [Enter]`. If `enter`
    /// is `true` we append `Enter` so the line is submitted.
    fn send_keys(&self, handle: &TmuxHandle, keys: &str, enter: bool) -> Result<()>;

    /// `true` iff the window referenced by `handle` is still alive on the
    /// tmux server. Used by the recovery sweep to detect abandoned
    /// `ConflictSession`s.
    fn window_alive(&self, handle: &TmuxHandle) -> Result<bool>;
}

// ---------------------------------------------------------------------
// ProcessTmux — the real adapter.
// ---------------------------------------------------------------------

/// Shells out to the user's `tmux` binary.
#[derive(Debug, Default, Clone)]
pub struct ProcessTmux;

impl ProcessTmux {
    pub fn new() -> Self {
        Self
    }

    fn tmux() -> Command {
        let mut c = Command::new("tmux");
        c.stdin(Stdio::null());
        c.stdout(Stdio::piped());
        c.stderr(Stdio::piped());
        c
    }
}

impl TmuxOps for ProcessTmux {
    fn ensure_session(&self, session: &str) -> Result<()> {
        if self.has_session(session)? {
            return Ok(());
        }
        let mut c = Self::tmux();
        c.args([
            "new-session",
            "-d",
            "-s",
            session,
            // First window stays as a holding shell. We never use it; all
            // real work happens in named per-conflict windows.
            "-n",
            "mergequeue",
        ]);
        let out = c.output().map_err(|e| Error::tmux(format!("spawn: {e}")))?;
        if !out.status.success() {
            return Err(Error::tmux(format!(
                "new-session -s {session} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn has_session(&self, session: &str) -> Result<bool> {
        let mut c = Self::tmux();
        c.args(["has-session", "-t", session]);
        let out = c.output().map_err(|e| Error::tmux(format!("spawn: {e}")))?;
        Ok(out.status.success())
    }

    fn new_window(&self, session: &str, window: &str, cwd: &Path) -> Result<TmuxHandle> {
        self.ensure_session(session)?;
        let mut c = Self::tmux();
        let cwd_str = cwd
            .to_str()
            .ok_or_else(|| Error::tmux(format!("non-utf8 cwd: {}", cwd.display())))?;
        c.args([
            "new-window",
            "-d",
            "-t",
            session,
            "-n",
            window,
            "-c",
            cwd_str,
        ]);
        let out = c.output().map_err(|e| Error::tmux(format!("spawn: {e}")))?;
        if !out.status.success() {
            return Err(Error::tmux(format!(
                "new-window {session}:{window} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(TmuxHandle {
            session: session.to_string(),
            window: window.to_string(),
        })
    }

    fn send_keys(&self, handle: &TmuxHandle, keys: &str, enter: bool) -> Result<()> {
        let target = format!("{}:{}", handle.session, handle.window);
        let mut c = Self::tmux();
        c.args(["send-keys", "-t", &target, keys]);
        if enter {
            c.arg("Enter");
        }
        let out = c.output().map_err(|e| Error::tmux(format!("spawn: {e}")))?;
        if !out.status.success() {
            return Err(Error::tmux(format!(
                "send-keys -t {target} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn window_alive(&self, handle: &TmuxHandle) -> Result<bool> {
        if !self.has_session(&handle.session)? {
            return Ok(false);
        }
        // `tmux list-windows -t <session> -F '#W'` prints one window name
        // per line. We just check for membership.
        let mut c = Self::tmux();
        c.args(["list-windows", "-t", &handle.session, "-F", "#W"]);
        let out = c.output().map_err(|e| Error::tmux(format!("spawn: {e}")))?;
        if !out.status.success() {
            return Ok(false);
        }
        let names = String::from_utf8_lossy(&out.stdout);
        Ok(names.lines().any(|n| n == handle.window))
    }
}

// ---------------------------------------------------------------------
// Naming helpers (shared with the engine).
// ---------------------------------------------------------------------

/// Per-MergeQueue-process session name. We scope by PID so that two
/// MergeQueue instances on the same host (even though only one holds the
/// TUI lock) don't share tmux state.
pub fn session_name(pid: u32) -> String {
    format!("mergequeue-{pid}")
}

/// Per-queue-entry window name. Stable for the lifetime of a
/// `ConflictSession`.
pub fn window_name(short_entry_id: &str) -> String {
    format!("conflict-{short_entry_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_includes_pid() {
        assert_eq!(session_name(4242), "mergequeue-4242");
    }

    #[test]
    fn window_name_prefixed() {
        assert_eq!(window_name("abcd1234"), "conflict-abcd1234");
    }
}
