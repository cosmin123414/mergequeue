//! Helpers for reopening a conflict-resolution agent session.

use std::io::Read;
use std::process::{Command, Stdio};

use crate::agents::tmux::{self, TmuxOps};
use crate::core::conflict::{ConflictOutcome, ConflictSession};
use crate::core::ids::ConflictSessionId;
use crate::core::ports::{AgentRegistry, ConflictPrompt, QueueStore, TmuxHandle};
use crate::core::queue::QueueEntry;
use crate::core::repo::RegisteredRepo;
use crate::error::{Error, Result};

pub fn conflict_session_handle(
    store: &dyn QueueStore,
    tmux: &dyn TmuxOps,
    agents: &dyn AgentRegistry,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<TmuxHandle> {
    let session_id = entry.conflict_session_id.ok_or_else(|| {
        Error::invalid(format!(
            "entry {} has no conflict session recorded",
            entry.id.short()
        ))
    })?;
    let session = store
        .get_conflict_session(session_id)?
        .ok_or_else(|| Error::NotFound(format!("conflict session {session_id}")))?;

    let handle = TmuxHandle {
        session: session.tmux_session.clone(),
        window: session.tmux_window.clone(),
    };
    if tmux.window_alive(&handle).unwrap_or(false) {
        return Ok(handle);
    }

    respawn(store, tmux, agents, repo, entry, session_id, &session)
}

pub fn attach_tmux(handle: &TmuxHandle) -> Result<()> {
    let target = format!("{}:{}", handle.session, handle.window);
    let args = if std::env::var_os("TMUX").is_some() {
        vec!["switch-client", "-t", target.as_str()]
    } else {
        vec!["attach-session", "-t", target.as_str()]
    };

    let mut child = Command::new("tmux")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::tmux(format!("spawn attach: {e}")))?;
    let stderr_handle = child.stderr.take();
    let stderr_buf = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut stderr) = stderr_handle {
            let _ = stderr.read_to_end(&mut buf);
        }
        buf
    });
    let status = child
        .wait()
        .map_err(|e| Error::tmux(format!("wait attach: {e}")))?;
    let stderr = stderr_buf.join().unwrap_or_default();
    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr).trim().to_string();
        if msg.is_empty() {
            return Err(Error::tmux("attach failed"));
        }
        return Err(Error::tmux(format!("attach failed: {msg}")));
    }
    Ok(())
}

fn respawn(
    store: &dyn QueueStore,
    tmux: &dyn TmuxOps,
    agents: &dyn AgentRegistry,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
    session_id: ConflictSessionId,
    existing: &ConflictSession,
) -> Result<TmuxHandle> {
    let agent = agents.get(repo.agent_backend)?;
    let window = tmux::window_name(&entry.id.short());
    let new_handle = tmux.new_window(&existing.tmux_session, &window, &entry.source_worktree)?;

    let prompt = ConflictPrompt {
        repo_name: repo
            .root_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo")
            .to_string(),
        source_branch: entry.source_branch.clone(),
        target_branch: entry.target_branch.clone(),
        // Respawn lets the agent rediscover the current conflict set.
        conflicted_files: Vec::new(),
        ci_command_hint: ci_hint(repo),
    };
    agent
        .open_conflict_session(&entry.source_worktree, &prompt, &new_handle)
        .map_err(|e| Error::agent(format!("respawn agent: {e}")))?;

    let now = time::OffsetDateTime::now_utc();
    store.close_conflict_session(session_id, ConflictOutcome::Abandoned, now)?;
    let fresh = ConflictSession {
        id: ConflictSessionId::new(),
        queue_entry_id: entry.id,
        agent_backend: repo.agent_backend,
        tmux_session: new_handle.session.clone(),
        tmux_window: new_handle.window.clone(),
        started_at: now,
        ended_at: None,
        outcome: None,
    };
    store.open_conflict_session(&fresh)?;
    let mut updated = entry.clone();
    updated.conflict_session_id = Some(fresh.id);
    store.update_entry(&updated)?;

    Ok(new_handle)
}

fn ci_hint(repo: &RegisteredRepo) -> Option<String> {
    let parts: Vec<&str> = [
        repo.ci.lint_command.as_deref(),
        repo.ci.test_command.as_deref(),
        repo.ci.build_command.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" && "))
    }
}
