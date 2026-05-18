//! Pure domain types and the four port traits.
//!
//! Nothing in this module performs I/O, holds a connection, spawns a
//! subprocess, or reads wall-clock time directly. Everything that
//! needs to do those things goes through a port (`Clock`, `QueueStore`,
//! `GitOps`, `MergeAgent`).

pub mod agent_backend;
pub mod conflict;
pub mod events;
pub mod ids;
pub mod ports;
pub mod queue;
pub mod repo;
pub mod state_machine;

pub use agent_backend::AgentBackend;
pub use conflict::{ConflictOutcome, ConflictSession};
pub use events::QueueEvent;
pub use ids::{ConflictSessionId, QueueEntryId, RepoId};
pub use ports::{
    AgentRegistry, Clock, ConflictPrompt, EntryFilter, FastForwardOutcome, GitOps, MergeAgent,
    QueueStore, RebaseOutcome, TmuxHandle,
};
pub use queue::{MergeFailureReason, QueueEntry, QueueStatus, StepOutcome};
pub use repo::{RegisteredRepo, RepoCiConfig};
pub use state_machine::{advance_status, transition, NextAction, TerminalStatus};
