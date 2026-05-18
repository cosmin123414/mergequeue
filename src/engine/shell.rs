//! Action executors. Each `run_*` function executes one `NextAction`
//! and returns the resulting `StepOutcome`. None of them touch the FSM
//! directly; they only touch ports.

use crate::core::ports::{FastForwardOutcome, GitOps, RebaseOutcome};
use crate::core::queue::{QueueEntry, StepOutcome};
use crate::core::repo::RegisteredRepo;
use crate::engine::ci::CiRunner;
use crate::error::Result;

pub fn run_rebase(git: &dyn GitOps, entry: &QueueEntry) -> Result<StepOutcome> {
    match git.rebase_onto(&entry.source_worktree, &entry.target_branch)? {
        RebaseOutcome::Ok => Ok(StepOutcome::RebaseOk),
        RebaseOutcome::Conflict => Ok(StepOutcome::RebaseConflict),
        RebaseOutcome::OtherError(_msg) => {
            // We map all other errors to RebaseConflict so the user gets
            // a `NeedsHelp` handoff rather than a hard `Failed`. The
            // agent prompt will surface the original message.
            Ok(StepOutcome::RebaseConflict)
        }
    }
}

pub fn run_lint(
    ci: &CiRunner<'_>,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<StepOutcome> {
    let cmd = repo
        .ci
        .lint_command
        .as_deref()
        .expect("FSM routed to RunLint without lint_command configured");
    let ok = ci.run("lint", cmd, &entry.source_worktree, entry)?;
    Ok(if ok {
        StepOutcome::LintPassed
    } else {
        StepOutcome::LintFailed
    })
}

pub fn run_test(
    ci: &CiRunner<'_>,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<StepOutcome> {
    let cmd = repo
        .ci
        .test_command
        .as_deref()
        .expect("FSM routed to RunTest without test_command configured");
    let ok = ci.run("test", cmd, &entry.source_worktree, entry)?;
    Ok(if ok {
        StepOutcome::TestPassed
    } else {
        StepOutcome::TestFailed
    })
}

pub fn run_build(
    ci: &CiRunner<'_>,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<StepOutcome> {
    let cmd = repo
        .ci
        .build_command
        .as_deref()
        .expect("FSM routed to RunBuild without build_command configured");
    let ok = ci.run("build", cmd, &entry.source_worktree, entry)?;
    Ok(if ok {
        StepOutcome::BuildPassed
    } else {
        StepOutcome::BuildFailed
    })
}

pub fn run_fast_forward(
    git: &dyn GitOps,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<StepOutcome> {
    match git.fast_forward(&repo.root_path, &entry.source_branch)? {
        FastForwardOutcome::Ok => Ok(StepOutcome::FastForwardOk),
        FastForwardOutcome::NonFastForward | FastForwardOutcome::OtherError(_) => {
            Ok(StepOutcome::FastForwardRejected)
        }
    }
}
