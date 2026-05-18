//! MergeSmith — a local merge queue with a pixel-art blacksmith.
//!
//! See `docs/` at the repository root for the architectural plan.
//!
//! The crate is organized feature-first: `core` holds pure domain types
//! plus the four port traits, and adapter modules (`store`, `git`,
//! `agents`) implement those ports. The `engine` consumes ports
//! exclusively; the `tui` and `cli` are the two presentation surfaces.

pub mod cli;
pub mod core;
pub mod engine;
pub mod error;
pub mod git;
pub mod paths;
pub mod store;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
