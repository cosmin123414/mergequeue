//! Startup sweeps: dead PID claims, abandoned conflict sessions.

use crate::agents::tmux::TmuxOps;
use crate::core::conflict::ConflictOutcome;
use crate::core::ports::{Clock, QueueStore, TmuxHandle};
use crate::error::Result;

/// Sweep stale `claimed_by_pid` rows whose PIDs are no longer alive
/// back to `Queued`. `live_pids` should include the current process so
/// that an in-progress recovery doesn't reset its own claims (this is
/// only relevant if MergeQueue is restarted in-place; on first start
/// our PID has not yet claimed anything, so passing `[]` is safe).
pub fn sweep_dead_claims(store: &dyn QueueStore, live_pids: &[u32]) -> Result<usize> {
    store.sweep_dead_pid_claims(live_pids)
}

/// For every open `ConflictSession`, check whether its tmux window is
/// still alive. If not, mark it `ConflictOutcome::Abandoned`. Returns
/// the number of sessions closed.
///
/// Note: this does **not** touch the corresponding `QueueEntry`. The
/// entry remains in `NeedsHelp`; the user can still `mergequeue resolve
/// <id>` (which will re-spawn an agent) or `mergequeue retry <id>`.
pub fn sweep_abandoned_conflict_sessions(
    store: &dyn QueueStore,
    tmux: &dyn TmuxOps,
    clock: &dyn Clock,
) -> Result<usize> {
    let open = store.list_open_conflict_sessions()?;
    let mut closed = 0;
    for s in open {
        let handle = TmuxHandle {
            session: s.tmux_session.clone(),
            window: s.tmux_window.clone(),
        };
        // If the tmux query itself fails (e.g. tmux not on PATH), treat
        // the session as alive so we don't close real sessions on a
        // false negative. Better to leak a Resolved/Abandoned outcome
        // than to drop a session the user is actively using.
        let alive = tmux.window_alive(&handle).unwrap_or(true);
        if !alive {
            store.close_conflict_session(s.id, ConflictOutcome::Abandoned, clock.now())?;
            closed += 1;
        }
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use time::OffsetDateTime;

    use crate::core::agent_backend::AgentBackend;
    use crate::core::conflict::{ConflictOutcome, ConflictSession};
    use crate::core::ids::{ConflictSessionId, QueueEntryId};
    use crate::core::ports::TmuxHandle;
    use crate::test_support::{fake_store::make_fake_store, FakeClock, FakeTmux};

    use super::*;

    fn epoch_dt() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    #[test]
    fn closes_abandoned_sessions_and_leaves_live_ones() {
        let store = Arc::new(make_fake_store());
        let tmux = Arc::new(FakeTmux::new());
        let clock = Arc::new(FakeClock::epoch());

        // Two sessions: one with a live tmux window, one without.
        let alive_handle = tmux
            .new_window(
                "mergequeue-1",
                "conflict-alive",
                std::path::Path::new("/tmp"),
            )
            .unwrap();
        let dead_handle = TmuxHandle {
            session: "mergequeue-1".into(),
            window: "conflict-dead".into(),
        };

        let alive = ConflictSession {
            id: ConflictSessionId::new(),
            queue_entry_id: QueueEntryId::new(),
            agent_backend: AgentBackend::Opencode,
            tmux_session: alive_handle.session.clone(),
            tmux_window: alive_handle.window.clone(),
            started_at: epoch_dt(),
            ended_at: None,
            outcome: None,
        };
        let dead = ConflictSession {
            id: ConflictSessionId::new(),
            queue_entry_id: QueueEntryId::new(),
            agent_backend: AgentBackend::Opencode,
            tmux_session: dead_handle.session.clone(),
            tmux_window: dead_handle.window.clone(),
            started_at: epoch_dt(),
            ended_at: None,
            outcome: None,
        };
        store.open_conflict_session(&alive).unwrap();
        store.open_conflict_session(&dead).unwrap();

        let closed = sweep_abandoned_conflict_sessions(&*store, &*tmux, &*clock).unwrap();
        assert_eq!(closed, 1);

        let alive_after = store.get_conflict_session(alive.id).unwrap().unwrap();
        assert!(alive_after.ended_at.is_none());
        assert_eq!(alive_after.outcome, None);

        let dead_after = store.get_conflict_session(dead.id).unwrap().unwrap();
        assert!(dead_after.ended_at.is_some());
        assert_eq!(dead_after.outcome, Some(ConflictOutcome::Abandoned));
    }
}
