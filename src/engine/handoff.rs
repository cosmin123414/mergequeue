//! Conflict handoff. Opens a tmux window, asks the configured agent to
//! place itself inside that window, and records a `ConflictSession`.
//!
//! Failure modes worth knowing about:
//!
//! - Tmux command fails (e.g. `tmux` not on PATH) → we still write a
//!   `ConflictSession` with empty `tmux_session/window`, log the error,
//!   and let the entry land in `NeedsHelp` so the human notices. The
//!   alternative (refusing to flip status) would leave the worker
//!   spinning on a permanent failure.
//! - Agent unavailable → same: record the session with the (already-
//!   created or empty) tmux info; the user can attach to the window
//!   manually and run their preferred CLI.
//! - Discovering `conflicted_files` fails → we proceed with an empty
//!   list. The prompt is still useful (it tells the agent which branches
//!   are involved).
//!
//! Net: the engine never gets stuck in handoff. The user always sees
//! `NeedsHelp` and can decide what to do.

use std::sync::Arc;

use crate::agents::tmux::{self, TmuxOps};
use crate::core::conflict::ConflictSession;
use crate::core::ids::ConflictSessionId;
use crate::core::ports::{AgentRegistry, Clock, ConflictPrompt, GitOps, QueueStore, TmuxHandle};
use crate::core::queue::QueueEntry;
use crate::core::repo::RegisteredRepo;
use crate::error::Result;

pub struct ConflictHandoff {
    pub session: Arc<ConflictSession>,
    pub conflicted_files: Vec<String>,
    pub agent_started: bool,
    pub error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn open_conflict_session(
    store: &dyn QueueStore,
    clock: &dyn Clock,
    tmux_ops: &dyn TmuxOps,
    agents: &dyn AgentRegistry,
    git: &dyn GitOps,
    tmux_pid: u32,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<ConflictHandoff> {
    let session_name = tmux::session_name(tmux_pid);
    let window_name = tmux::window_name(&entry.id.short());
    let conflicted_files = git
        .conflicted_files(&entry.source_worktree)
        .unwrap_or_default();

    let (tmux_session, tmux_window, agent_token, agent_started, error) =
        match try_open_tmux_and_agent(
            tmux_ops,
            agents,
            repo,
            entry,
            &conflicted_files,
            &session_name,
            &window_name,
        ) {
            Ok((handle, token)) => (handle.session, handle.window, token, true, None),
            Err(err) => {
                tracing::warn!(
                    entry = %entry.id,
                    "conflict handoff failed: {err}; recording session anyway so the user can take over"
                );
                (
                    session_name,
                    window_name,
                    None,
                    false,
                    Some(err.to_string()),
                )
            }
        };

    let session = ConflictSession {
        id: ConflictSessionId::new(),
        queue_entry_id: entry.id,
        agent_backend: repo.agent_backend,
        tmux_session,
        tmux_window,
        started_at: clock.now(),
        ended_at: None,
        outcome: None,
    };
    // For diagnostics only — not yet persisted; M3+ can wire a column.
    if let Some(t) = agent_token {
        tracing::info!(entry = %entry.id, agent_token = %t, "agent acknowledged");
    }
    store.open_conflict_session(&session)?;
    Ok(ConflictHandoff {
        session: Arc::new(session),
        conflicted_files,
        agent_started,
        error,
    })
}

fn try_open_tmux_and_agent(
    tmux_ops: &dyn TmuxOps,
    agents: &dyn AgentRegistry,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
    conflicted_files: &[String],
    session: &str,
    window: &str,
) -> Result<(TmuxHandle, Option<String>)> {
    // 1. Compose the prompt.
    let prompt = ConflictPrompt {
        repo_name: repo
            .root_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo")
            .to_string(),
        source_branch: entry.source_branch.clone(),
        target_branch: entry.target_branch.clone(),
        conflicted_files: conflicted_files.to_vec(),
        ci_command_hint: ci_command_hint(repo),
    };

    // 2. Open the tmux window. `new_window` ensures the session exists.
    let handle = tmux_ops.new_window(session, window, &entry.source_worktree)?;

    // 3. Resolve and spawn the agent.
    let agent = agents.get(repo.agent_backend)?;
    let ack = agent.open_conflict_session(&entry.source_worktree, &prompt, &handle)?;
    Ok((handle, ack.agent_session_token))
}

fn ci_command_hint(repo: &RegisteredRepo) -> Option<String> {
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
