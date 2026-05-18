//! Pre-rebase checks: target worktree dirtiness and existence.

use crate::core::ports::GitOps;
use crate::core::queue::{QueueEntry, StepOutcome};
use crate::core::repo::RegisteredRepo;
use crate::error::Result;

/// Run pre-rebase checks against the registered repo's root path
/// (which doubles as the target worktree we'll be fast-forwarding).
pub fn run_precheck(
    git: &dyn GitOps,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<StepOutcome> {
    // Source worktree must exist; if not, the entry is unrecoverable.
    if !git.worktree_exists(&entry.source_worktree)? {
        return Ok(StepOutcome::PrecheckOk); // Let the FSM proceed; rebase
                                            // will fail loudly. We don't
                                            // create a separate
                                            // WorktreeGone precheck
                                            // outcome — keep the FSM
                                            // alphabet minimal.
    }
    // Target dirtiness blocks the merge.
    if git.worktree_is_dirty(&repo.root_path)? {
        return Ok(StepOutcome::PrecheckDirtyTarget);
    }
    Ok(StepOutcome::PrecheckOk)
}
