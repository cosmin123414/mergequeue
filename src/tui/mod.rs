//! Animated TUI with a pixel-art blacksmith rendered via the Kitty
//! graphics protocol.
//!
//! Module shape:
//!
//! ```text
//! tui/
//!   mod.rs           run() entry point + render-loop scaffolding
//!   app.rs           AppState: queue snapshot + sprite ticker + selection
//!   ui.rs            ratatui draw fn (layout + widgets)
//!   events.rs        crossterm event poll + key mapping
//!   capability.rs    Kitty-graphics probe with timeout + terminal-mode guard
//!   sprite/
//!     mod.rs         SpriteRenderer: lifecycle (upload / place / delete)
//!     state.rs       SpriteState + compute_sprite_state()
//!     kitty.rs       pure protocol-bytes functions
//!     placeholder.rs procedurally-generated 64x64 sprite (M4 only;
//!                    M5 swaps in commissioned art).
//! ```
//!
//! The TUI is intentionally read-only with respect to the queue. The
//! only mutation it supports is "delete a Queued entry" (`d` key). All
//! other operations — enqueue, cancel-in-flight, retry, resolve — are
//! CLI subcommands, per `docs/07-cli.md`.

pub mod app;
pub mod capability;
pub mod events;
pub mod sprite;
pub mod ui;

mod run;

pub use run::{run, RunError};
