//! `mergesmith retry <id>` — re-queue a Failed or NeedsHelp entry.

use std::process::ExitCode;

use crate::cli::context::CliContext;
use crate::cli::parser::IdArg;
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
