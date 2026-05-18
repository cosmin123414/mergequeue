//! `mergesmith status` — print the queue.

use std::process::ExitCode;

use crate::cli::context::CliContext;
use crate::cli::parser::{OutputFormat, StatusArgs};
use crate::core::ports::EntryFilter;
use crate::error::Result;

pub fn run(args: StatusArgs) -> Result<ExitCode> {
    let ctx = CliContext::open()?;

    let mut filter = EntryFilter::all();
    if let Some(repo_root) = args.repo.as_deref() {
        let Some(repo) = ctx
            .store
            .get_repo_by_root(std::path::Path::new(repo_root))?
        else {
            eprintln!("no registered repo at {repo_root}");
            return Ok(ExitCode::from(3));
        };
        filter.repo_id = Some(repo.id);
    }

    let entries = ctx.store.list_entries(filter)?;
    match args.format {
        OutputFormat::Human => print_human(&entries),
        OutputFormat::Json => {
            let s = serde_json::to_string_pretty(&entries)
                .map_err(|e| crate::error::Error::other(e.to_string()))?;
            println!("{s}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_human(entries: &[crate::core::queue::QueueEntry]) {
    if entries.is_empty() {
        println!("(queue is empty)");
        return;
    }
    println!(
        "{:<8} {:<11} {:<24} {:<24}",
        "id", "status", "source", "target"
    );
    for e in entries {
        println!(
            "{:<8} {:<11} {:<24} {:<24}",
            e.id.short(),
            e.status.as_str(),
            truncate(&e.source_branch, 24),
            truncate(&e.target_branch, 24),
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
