//! `mergesmith enqueue` — queue the current worktree for merge.

use std::process::ExitCode;

use crate::cli::context::CliContext;
use crate::cli::parser::EnqueueArgs;
use crate::core::ids::QueueEntryId;
use crate::core::queue::{QueueEntry, QueueStatus};
use crate::error::Result;

pub fn run(args: EnqueueArgs) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let cwd = std::env::current_dir()?;
    let worktree_root = ctx.git.discover_worktree_root(&cwd)?;
    let source_branch = ctx.git.current_branch(&worktree_root)?;

    // Find the registered repo whose root_path is an ancestor of (or
    // equal to) the discovered worktree root. The user's worktree may
    // be a `git worktree add` sibling, so we walk up until we hit a
    // registered root.
    let repos = ctx.store.list_repos()?;
    let Some(repo) = pick_repo_for_worktree(&repos, &worktree_root) else {
        eprintln!(
            "no registered repo for {}.\nrun `mergesmith init` inside the repo first.",
            worktree_root.display()
        );
        return Ok(ExitCode::from(3));
    };

    let target_branch = args.target.unwrap_or_else(|| repo.default_branch.clone());

    if source_branch == target_branch {
        eprintln!("cannot enqueue: source branch `{source_branch}` is the target branch.");
        return Ok(ExitCode::from(1));
    }

    if ctx.git.worktree_is_dirty(&worktree_root)? {
        eprintln!("cannot enqueue: source worktree is dirty. Commit or stash first.");
        return Ok(ExitCode::from(1));
    }

    let entry = QueueEntry {
        id: QueueEntryId::new(),
        repo_id: repo.id,
        source_worktree: worktree_root,
        source_branch,
        target_branch,
        status: QueueStatus::Queued,
        last_outcome: None,
        enqueued_at: ctx.clock.now(),
        started_at: None,
        finished_at: None,
        failure_reason: None,
        ci_log_dir: None,
        merge_log_path: None,
        conflict_session_id: None,
        message: args.message,
        claimed_by_pid: None,
        claimed_at: None,
    };
    ctx.store.enqueue(&entry)?;
    println!(
        "enqueued {} ({} -> {})",
        entry.id.short(),
        entry.source_branch,
        entry.target_branch
    );
    Ok(ExitCode::SUCCESS)
}

fn pick_repo_for_worktree<'a>(
    repos: &'a [crate::core::repo::RegisteredRepo],
    worktree_root: &std::path::Path,
) -> Option<&'a crate::core::repo::RegisteredRepo> {
    // Prefer exact match; fall back to ancestor-of-source match.
    if let Some(r) = repos.iter().find(|r| r.root_path == worktree_root) {
        return Some(r);
    }
    repos
        .iter()
        .find(|r| worktree_root.starts_with(&r.root_path))
}
