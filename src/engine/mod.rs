//! The merge-queue engine.
//!
//! The FSM (`crate::core::state_machine::transition`) chooses the next
//! action; the shell (`worker`) executes it through the four ports.
//! `Pool` spawns one worker thread per registered repo.

pub mod ci;
pub mod events;
pub mod handoff;
pub mod pool;
pub mod precheck;
pub mod recovery;
pub mod shell;
pub mod shutdown;
pub mod worker;

#[cfg(test)]
mod tests;

pub use events::EventBroadcaster;
pub use pool::{EnginePool, PoolDeps, Reconciliation};
pub use shutdown::{ShutdownLevel, ShutdownToken};
pub use worker::{Worker, WorkerDeps};
