//! Sprite rendering subsystem.
//!
//! Two responsibilities:
//!
//! 1. **State derivation** ([`state`]): map a `QueueSnapshot` to a
//!    [`SpriteState`] (Idle / Working / NeedsHelp), which tunes the
//!    animation's zoom speed and accent color.
//! 2. **Frame geometry** ([`penrose`]): advance the self-similar Penrose
//!    tiling and rasterize it into a [`DotBitmap`], which [`glyph`]
//!    renders as Braille text.

pub mod glyph;
pub mod penrose;
pub mod state;

pub use penrose::{DotBitmap, PenroseAnimator, FRAME_H, FRAME_W};
pub use state::{compute_sprite_state, QueueSnapshot, SpriteState};
