//! State-root resolution. See docs/11-config-and-paths.md.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// Resolve the MergeQueue state-root directory.
///
/// Order:
/// 1. `$MERGEQUEUE_HOME` if set.
/// 2. macOS: `$HOME/Library/Application Support/MergeQueue`.
/// 3. Linux: `$XDG_DATA_HOME/mergequeue` (fallback `$HOME/.local/share/mergequeue`).
pub fn state_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("MERGEQUEUE_HOME") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }

    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or(Error::StateRootMissing)?;

    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library/Application Support/MergeQueue"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            if !xdg.is_empty() {
                return Ok(PathBuf::from(xdg).join("mergequeue"));
            }
        }
        Ok(home.join(".local/share/mergequeue"))
    }
}

pub fn sqlite_path(root: &std::path::Path) -> PathBuf {
    root.join("state.sqlite")
}

pub fn tui_pid_path(root: &std::path::Path) -> PathBuf {
    root.join("tui.pid")
}

pub fn runs_dir(root: &std::path::Path) -> PathBuf {
    root.join("runs")
}

/// Ensure the state-root exists.
pub fn ensure_state_root(root: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    std::fs::create_dir_all(runs_dir(root))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        // Use a unique env var since std::env is process-global.
        let prev = std::env::var("MERGEQUEUE_HOME").ok();
        std::env::set_var("MERGEQUEUE_HOME", "/tmp/ms-override-xyz");
        assert_eq!(state_root().unwrap(), PathBuf::from("/tmp/ms-override-xyz"));
        match prev {
            Some(v) => std::env::set_var("MERGEQUEUE_HOME", v),
            None => std::env::remove_var("MERGEQUEUE_HOME"),
        }
    }
}
