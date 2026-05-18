//! Events broadcast by the engine and consumed by the TUI.

use serde::{Deserialize, Serialize};

use crate::core::ids::QueueEntryId;
use crate::core::queue::{QueueStatus, StepOutcome};
use crate::core::state_machine::NextAction;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueEvent {
    Enqueued(QueueEntryId),
    StatusChanged {
        id: QueueEntryId,
        from: QueueStatus,
        to: QueueStatus,
    },
    StepStarted {
        id: QueueEntryId,
        action: NextAction,
    },
    StepFinished {
        id: QueueEntryId,
        outcome: StepOutcome,
    },
    Removed(QueueEntryId),
}
