//! `mergequeue resolve <id>` — attach to (or re-spawn) the agent
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

use crate::agents::tmux::ProcessTmux;
use crate::agents::{resolve, DefaultAgentRegistry};
use crate::cli::context::CliContext;
use crate::cli::parser::IdArg;
use crate::core::ports::TmuxHandle;
use crate::core::queue::QueueStatus;
use crate::error::Result;

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
    if entry.conflict_session_id.is_none() {
        eprintln!(
            "entry is NeedsHelp but has no conflict session recorded; \
             run `mergequeue retry {}` to re-enqueue it",
            entry.id.short()
        );
        return Ok(ExitCode::from(3));
    }
    let Some(repo) = ctx.store.get_repo(entry.repo_id)? else {
        eprintln!("repo for entry has been deregistered");
        return Ok(ExitCode::from(3));
    };

    let tmux = ProcessTmux::new();
    let agents = DefaultAgentRegistry::new(std::sync::Arc::new(ProcessTmux::new()));
    let handle = resolve::conflict_session_handle(&*ctx.store, &tmux, &agents, &repo, &entry)?;
    print_attach_hint(&handle);
    Ok(ExitCode::SUCCESS)
}

fn print_attach_hint(handle: &TmuxHandle) {
    println!(
        "agent waiting in tmux. Attach with:\n  tmux attach-session -t {}:{}",
        handle.session, handle.window
    );
}
