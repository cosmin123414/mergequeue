//! `ConflictSession`: a record of a `NeedsHelp` handoff to an agent.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::core::agent_backend::AgentBackend;
use crate::core::ids::{ConflictSessionId, QueueEntryId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSession {
    pub id: ConflictSessionId,
    pub queue_entry_id: QueueEntryId,
    pub agent_backend: AgentBackend,
    pub tmux_session: String,
    pub tmux_window: String,
    #[serde(with = "time::serde::iso8601")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::iso8601::option")]
    pub ended_at: Option<OffsetDateTime>,
    pub outcome: Option<ConflictOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictOutcome {
    Resolved,
    Abandoned,
    AgentCrashed,
}

impl ConflictOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Resolved => "Resolved",
            Self::Abandoned => "Abandoned",
            Self::AgentCrashed => "AgentCrashed",
        }
    }
}
