//! `mergesmith retry <id>` — re-queue a Failed or NeedsHelp entry.

use std::process::ExitCode;

use crate::cli::context::CliContext;
use crate::cli::parser::IdArg;
use crate::core::conflict::ConflictOutcome;
use crate::core::queue::{QueueStatus, StepOutcome};
use crate::error::Result;

pub fn run(args: IdArg) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let id = super::cancel::resolve_id(&*ctx.store, &args.id)?;
    let Some(mut entry) = ctx.store.get_entry(id)? else {
        eprintln!("no such entry: {}", args.id);
        return Ok(ExitCode::from(3));
    };
    match entry.status {
        QueueStatus::Failed | QueueStatus::NeedsHelp | QueueStatus::Cancelled => {}
        other => {
            eprintln!("cannot retry an entry in state {other}");
            return Ok(ExitCode::from(1));
        }
    }
    // NeedsHelp re-queues with last_outcome=AgentResolvedConflict so the
    // FSM routes us back to Rebase. Failed/Cancelled re-queue from
    // scratch (no last_outcome).
    let resumed_from_conflict = entry.status == QueueStatus::NeedsHelp;

    // Close the conflict session (if any) as Resolved before re-queueing.
    // The human running `retry` is asserting "the agent's done." If the
    // session was already closed (e.g. by the recovery sweep marking it
    // Abandoned) we leave its outcome alone.
    if resumed_from_conflict {
        if let Some(sid) = entry.conflict_session_id {
            if let Some(session) = ctx.store.get_conflict_session(sid)? {
                if session.ended_at.is_none() {
                    ctx.store.close_conflict_session(
                        sid,
                        ConflictOutcome::Resolved,
                        ctx.clock.now(),
                    )?;
                }
            }
        }
    }

    entry.status = QueueStatus::Queued;
    entry.failure_reason = None;
    entry.finished_at = None;
    entry.last_outcome = if resumed_from_conflict {
        Some(StepOutcome::AgentResolvedConflict)
    } else {
        None
    };
    entry.claimed_by_pid = None;
    entry.claimed_at = None;
    ctx.store.update_entry(&entry)?;
    println!("re-queued {}", entry.id.short());
    Ok(ExitCode::SUCCESS)
}
