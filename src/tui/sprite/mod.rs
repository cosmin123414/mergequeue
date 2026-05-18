//! Sprite rendering subsystem.
//!
//! Three responsibilities:
//!
//! 1. **Asset lifecycle** ([`SpriteRenderer`]): upload the sprite sheet
//!    on construction, place the active frame on each render tick,
//!    delete on drop.
//! 2. **State derivation** ([`state`]): map a `QueueSnapshot` to a
//!    [`SpriteState`] (Idle / Working / NeedsHelp for M4).
//! 3. **Wire protocol** ([`kitty`]): pure byte-buffer construction.
//!
//! The renderer is decoupled from the rest of ratatui: it writes to a
//! `Box<dyn Write>` (typically `io::stdout()`), so tests can capture
//! its output to a `Vec<u8>`.

pub mod kitty;
pub mod placeholder;
pub mod state;

use std::io::Write;

use crate::error::{Error, Result};

pub use state::{compute_sprite_state, QueueSnapshot, SpriteState};

/// Image id under which we upload the sprite sheet. Kitty allows any
/// non-zero id; we pick a small constant so two `mergesmith tui`
/// instances on the same tmux server (if such a thing happened) would
/// at least be consistent.
pub const SPRITE_IMAGE_ID: u32 = 7;

/// Renders the blacksmith sprite into a reserved rectangle in the TUI.
///
/// Construction uploads the sprite sheet to the terminal. [`place`]
/// emits one place command for the current `(state, frame)`. [`drop`]
/// deletes the image so we don't leak terminal-cache slots.
///
/// `tmux_passthrough` controls whether every emitted sequence is
/// wrapped in tmux's `ESC P tmux ; … ESC \` envelope. Set this
/// according to `$TMUX` at TUI startup.
pub struct SpriteRenderer {
    sheet: placeholder::SpriteSheet,
    out: Box<dyn Write + Send>,
    tmux_passthrough: bool,
    uploaded: bool,
}

impl SpriteRenderer {
    /// Construct and upload the sprite sheet.
    ///
    /// `out` is typically `Box::new(io::stdout())` in production and
    /// `Box::new(Vec::new())` in tests.
    pub fn new(out: Box<dyn Write + Send>, tmux_passthrough: bool) -> Result<Self> {
        let sheet = placeholder::SpriteSheet::generate();
        let mut me = Self {
            sheet,
            out,
            tmux_passthrough,
            uploaded: false,
        };
        me.upload()?;
        Ok(me)
    }

    fn upload(&mut self) -> Result<()> {
        let png = self.sheet.png_bytes();
        let bytes =
            kitty::serialize_transmit(SPRITE_IMAGE_ID, kitty::ImageFormat::Png, None, None, png);
        self.write_kitty(&bytes)?;
        self.uploaded = true;
        Ok(())
    }

    /// Draw `state` at `(cell_x, cell_y)` in the terminal grid,
    /// occupying `cols` × `rows` cells. The frame chosen depends on
    /// `tick`: it picks an index within the state's animation loop.
    ///
    /// `place` must be called AFTER the regular ratatui draw and AFTER
    /// the terminal cursor has been positioned at the sprite's cell
    /// origin. We let the caller handle cursor placement because
    /// crossterm and ratatui interact with it; we just emit pixels.
    pub fn place(&mut self, state: SpriteState, tick: u64, cols: u32, rows: u32) -> Result<()> {
        if !self.uploaded {
            return Err(Error::other("sprite sheet not uploaded"));
        }
        let frame_index = state.frame_for_tick(tick);
        let (src_x, src_y, src_w, src_h) = self.sheet.frame_rect(state, frame_index);
        let bytes = kitty::serialize_place(
            SPRITE_IMAGE_ID,
            0,
            src_x,
            src_y,
            src_w,
            src_h,
            cols,
            rows,
            0,
        );
        self.write_kitty(&bytes)?;
        self.out
            .flush()
            .map_err(|e| Error::other(format!("flush sprite: {e}")))?;
        Ok(())
    }

    /// Delete the uploaded sprite from the terminal cache. Safe to
    /// call from `Drop`.
    pub fn cleanup(&mut self) -> Result<()> {
        if !self.uploaded {
            return Ok(());
        }
        let bytes = kitty::serialize_delete(SPRITE_IMAGE_ID);
        self.write_kitty(&bytes)?;
        self.out
            .flush()
            .map_err(|e| Error::other(format!("flush cleanup: {e}")))?;
        self.uploaded = false;
        Ok(())
    }

    fn write_kitty(&mut self, bytes: &[u8]) -> Result<()> {
        if self.tmux_passthrough {
            let wrapped = kitty::wrap_for_tmux(bytes);
            self.out
                .write_all(&wrapped)
                .map_err(|e| Error::other(format!("write sprite (tmux-wrapped): {e}")))
        } else {
            self.out
                .write_all(bytes)
                .map_err(|e| Error::other(format!("write sprite: {e}")))
        }
    }
}

impl Drop for SpriteRenderer {
    fn drop(&mut self) {
        // Best-effort cleanup; if it fails the terminal will GC the
        // image on its own LRU policy.
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// `Write` impl that shares a `Vec<u8>` with the test harness so
    /// we can inspect the bytes the renderer emitted.
    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl SharedBuf {
        fn snapshot(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Write for SharedBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn new_uploads_a_png_envelope() {
        let buf = SharedBuf::default();
        let r = SpriteRenderer::new(Box::new(buf.clone()), false).unwrap();
        // Drop the renderer so the cleanup envelope also lands.
        drop(r);
        let bytes = buf.snapshot();
        let txt = String::from_utf8_lossy(&bytes);
        // Upload envelope.
        assert!(txt.contains("a=t"));
        assert!(txt.contains("f=100"));
        assert!(txt.contains(&format!("i={SPRITE_IMAGE_ID}")));
        // Cleanup envelope.
        assert!(txt.contains("a=d"));
        assert!(txt.contains("d=I"));
    }

    #[test]
    fn place_emits_position_command() {
        let buf = SharedBuf::default();
        let mut r = SpriteRenderer::new(Box::new(buf.clone()), false).unwrap();
        r.place(SpriteState::Working, 0, 12, 6).unwrap();
        let bytes = buf.snapshot();
        let txt = String::from_utf8_lossy(&bytes);
        assert!(txt.contains("a=p"));
        // Working row starts at y=64; frame 0 starts at x=0.
        assert!(txt.contains("y=64"));
        // Source dimensions are 64×64.
        assert!(txt.contains("w=64"));
        assert!(txt.contains("h=64"));
        // Cell footprint:
        assert!(txt.contains("c=12"));
        assert!(txt.contains("r=6"));
    }

    #[test]
    fn tmux_passthrough_wraps_envelopes() {
        let buf = SharedBuf::default();
        let _r = SpriteRenderer::new(Box::new(buf.clone()), true).unwrap();
        let bytes = buf.snapshot();
        // Every Kitty envelope is wrapped: the outer envelope starts
        // with `ESC P tmux ;`.
        assert!(bytes.windows(7).any(|w| w == b"\x1bPtmux;"));
    }

    #[test]
    fn place_frame_advances_with_tick() {
        let buf = SharedBuf::default();
        let mut r = SpriteRenderer::new(Box::new(buf.clone()), false).unwrap();
        r.place(SpriteState::Idle, 0, 12, 6).unwrap();
        r.place(SpriteState::Idle, 2, 12, 6).unwrap();
        let bytes = buf.snapshot();
        let txt = String::from_utf8_lossy(&bytes);
        // Idle frame 0 starts at x=0; frame 1 starts at x=64.
        // (Idle ticks_per_frame=2 → tick=0 picks frame 0, tick=2 picks frame 1.)
        // Both `a=p` envelopes appear.
        assert_eq!(txt.matches("a=p").count(), 2);
        assert!(txt.contains("x=64"));
    }
}
