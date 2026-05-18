//! `mergesmith resolve <id>` — attach to (or re-spawn) the agent
//! session for a `NeedsHelp` entry.
//!
//! Behaviour:
//!
//! 1. Look up the queue entry. If it doesn't exist → exit 3.
//! 2. If status isn't `NeedsHelp` → exit 1 with a hint.
//! 3. Look up the conflict session.
//! 4. If the tmux window is alive → print the `tmux attach-session`
//!    command for the user to run. We deliberately do NOT exec
//!    tmux ourselves: a CLI tool stealing the terminal is surprising,
//!    and the TUI (M4) will offer an integrated "attach to agent"
//!    action instead.
//! 5. If the window is dead → re-spawn the agent into a fresh window
//!    using the configured backend, then print the attach command.

use std::process::ExitCode;
use std::sync::Arc;

use crate::agents::tmux::{self, ProcessTmux, TmuxOps};
use crate::agents::DefaultAgentRegistry;
use crate::cli::context::CliContext;
use crate::cli::parser::IdArg;
use crate::core::conflict::ConflictSession;
use crate::core::ids::ConflictSessionId;
use crate::core::ports::{AgentRegistry, ConflictPrompt, QueueStore, TmuxHandle};
use crate::core::queue::{QueueEntry, QueueStatus};
use crate::core::repo::RegisteredRepo;
use crate::error::{Error, Result};

pub fn run(args: IdArg) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let id = super::cancel::resolve_id(&*ctx.store, &args.id)?;
    let Some(entry) = ctx.store.get_entry(id)? else {
        eprintln!("no such entry: {}", args.id);
        return Ok(ExitCode::from(3));
    };
    if entry.status != QueueStatus::NeedsHelp {
        eprintln!(
            "entry is in state {}; `resolve` only applies to NeedsHelp entries",
            entry.status
        );
        return Ok(ExitCode::from(1));
    }
    let Some(session_id) = entry.conflict_session_id else {
        eprintln!(
            "entry is NeedsHelp but has no conflict session recorded; \
             run `mergesmith retry {}` to re-enqueue it",
            entry.id.short()
        );
        return Ok(ExitCode::from(3));
    };
    let Some(session) = ctx.store.get_conflict_session(session_id)? else {
        eprintln!("conflict session {session_id} missing from store");
        return Ok(ExitCode::from(3));
    };

    let tmux: Arc<dyn TmuxOps> = Arc::new(ProcessTmux::new());
    let handle = TmuxHandle {
        session: session.tmux_session.clone(),
        window: session.tmux_window.clone(),
    };
    let alive = tmux.window_alive(&handle).unwrap_or(false);

    if alive {
        print_attach_hint(&handle);
        return Ok(ExitCode::SUCCESS);
    }

    // Window is gone — respawn the agent inside a new window and link
    // the existing ConflictSession to it. We keep the same
    // ConflictSessionId so logs/diagnostics correlate.
    let Some(repo) = ctx.store.get_repo(entry.repo_id)? else {
        eprintln!("repo for entry has been deregistered");
        return Ok(ExitCode::from(3));
    };

    let new_handle = respawn(&*ctx.store, &*tmux, &repo, &entry, session_id, &session)?;
    print_attach_hint(&new_handle);
    println!(
        "(previous tmux window {}:{} was gone; spawned a fresh one)",
        session.tmux_session, session.tmux_window
    );
    Ok(ExitCode::SUCCESS)
}

fn print_attach_hint(handle: &TmuxHandle) {
    println!(
        "agent waiting in tmux. Attach with:\n  tmux attach-session -t {}:{}",
        handle.session, handle.window
    );
}

fn respawn(
    store: &dyn QueueStore,
    tmux: &dyn TmuxOps,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
    session_id: ConflictSessionId,
    existing: &ConflictSession,
) -> Result<TmuxHandle> {
    let agents = DefaultAgentRegistry::new(Arc::new(ProcessTmux::new()));
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
        // Respawn passes an empty list; the agent re-discovers conflicts
        // from `git status` inside the worktree.
        conflicted_files: Vec::new(),
        ci_command_hint: ci_hint(repo),
    };
    agent
        .open_conflict_session(&entry.source_worktree, &prompt, &new_handle)
        .map_err(|e| Error::agent(format!("respawn agent: {e}")))?;

    // Close the (now-dead) old session as Abandoned and open a fresh
    // one with the new tmux window. We keep `entry.conflict_session_id`
    // pointing at the live session for diagnostics.
    let now = time::OffsetDateTime::now_utc();
    store.close_conflict_session(
        session_id,
        crate::core::conflict::ConflictOutcome::Abandoned,
        now,
    )?;
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

/// Build a single "lint && test && build" hint from the repo's CI
/// config, or `None` if no commands are configured. Shared with
/// `engine::handoff`.
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
