//! `mergequeue cancel <id>` — mark an entry Cancelled.

use std::process::ExitCode;
use std::str::FromStr;

use crate::cli::context::CliContext;
use crate::cli::parser::IdArg;
use crate::core::ids::QueueEntryId;
use crate::core::queue::{MergeFailureReason, QueueStatus};
use crate::error::Result;

pub fn run(args: IdArg) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let id = resolve_id(&*ctx.store, &args.id)?;
    let Some(mut entry) = ctx.store.get_entry(id)? else {
        eprintln!("no such entry: {}", args.id);
        return Ok(ExitCode::from(3));
    };
    if entry.status.is_terminal() {
        eprintln!("entry is already terminal ({}).", entry.status);
        return Ok(ExitCode::from(1));
    }
    entry.status = QueueStatus::Cancelled;
    entry.failure_reason = Some(MergeFailureReason::UserCancelled);
    entry.finished_at = Some(ctx.clock.now());
    entry.claimed_by_pid = None;
    entry.claimed_at = None;
    ctx.store.update_entry(&entry)?;
    println!("cancelled {}", entry.id.short());
    Ok(ExitCode::SUCCESS)
}

/// Accept either a full UUID or a 6-char short form.
pub(crate) fn resolve_id(
    store: &dyn crate::core::ports::QueueStore,
    s: &str,
) -> Result<QueueEntryId> {
    if let Ok(id) = QueueEntryId::from_str(s) {
        return Ok(id);
    }
    if s.len() < 4 {
        return Err(crate::error::Error::invalid(format!(
            "id `{s}` is too short (need ≥4 chars or full UUID)"
        )));
    }
    let entries = store.list_entries(crate::core::ports::EntryFilter::all())?;
    let mut matching: Vec<&crate::core::queue::QueueEntry> = entries
        .iter()
        .filter(|e| e.id.to_string().replace('-', "").starts_with(s))
        .collect();
    if matching.is_empty() {
        return Err(crate::error::Error::NotFound(format!("entry {s}")));
    }
    if matching.len() > 1 {
        return Err(crate::error::Error::invalid(format!(
            "id `{s}` is ambiguous ({} matches)",
            matching.len()
        )));
    }
    Ok(matching.remove(0).id)
}
