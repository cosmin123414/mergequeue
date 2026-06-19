//! Animated TUI with a Penrose-tiling visualizer rendered as Braille
//! text glyphs (works in any terminal).
//!
//! Module shape:
//!
//! ```text
//! tui/
//!   mod.rs           run() entry point + render-loop scaffolding
//!   app.rs           AppState: queue snapshot + sprite ticker + selection
//!   ui.rs            ratatui draw fn (layout + widgets)
//!   events.rs        crossterm event poll + key mapping
//!   sprite/
//!     mod.rs         re-exports for the animator + state derivation
//!     state.rs       SpriteState + compute_sprite_state()
//!     penrose.rs     PenroseAnimator: rasterizes the tiling per frame.
//!     glyph.rs       Braille text rendering of a rasterized frame.
//! ```
//!
//! The TUI is the primary queue surface. It can delete a queued entry
//! (`d`) and attach directly to a selected `NeedsHelp` agent session
//! (`Enter` / `r`); enqueue/retry remain CLI commands for now.

pub mod app;
pub mod events;
pub mod sprite;
pub mod ui;

mod run;

pub use run::{run, RunError};
