//! Conflict handoff. M1 stub: marks the entry `NeedsHelp` and records
//! a placeholder `ConflictSession` without actually spawning an agent.
//! M2 will plug in tmux + `AgentRegistry`.

use std::sync::Arc;

use crate::core::conflict::ConflictSession;
use crate::core::ids::ConflictSessionId;
use crate::core::ports::{Clock, QueueStore};
use crate::core::queue::QueueEntry;
use crate::core::repo::RegisteredRepo;
use crate::error::Result;

pub fn open_placeholder_session(
    store: &dyn QueueStore,
    clock: &dyn Clock,
    repo: &RegisteredRepo,
    entry: &QueueEntry,
) -> Result<Arc<ConflictSession>> {
    let session = ConflictSession {
        id: ConflictSessionId::new(),
        queue_entry_id: entry.id,
        agent_backend: repo.agent_backend,
        // M2 fills these in for real.
        tmux_session: format!("mergesmith-{}", entry.id.short()),
        tmux_window: format!("conflict-{}", entry.id.short()),
        started_at: clock.now(),
        ended_at: None,
        outcome: None,
    };
    store.open_conflict_session(&session)?;
    Ok(Arc::new(session))
}
