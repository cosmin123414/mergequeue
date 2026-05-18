//! Startup sweeps: dead PID claims, abandoned conflict sessions.

use crate::core::ports::QueueStore;
use crate::error::Result;

/// Sweep stale `claimed_by_pid` rows whose PIDs are no longer alive
/// back to `Queued`. `live_pids` should include the current process so
/// that an in-progress recovery doesn't reset its own claims (this is
/// only relevant if MergeSmith is restarted in-place; on first start
/// our PID has not yet claimed anything, so passing `[]` is safe).
pub fn sweep_dead_claims(store: &dyn QueueStore, live_pids: &[u32]) -> Result<usize> {
    store.sweep_dead_pid_claims(live_pids)
}
