//! Procedurally-generated placeholder sprite sheet for M4.
//!
//! Builds a 384×192 PNG at startup containing three rows of 64×64
//! frames:
//!
//! - Row 0 (y=0..64):    `Idle`     — 4 frames, fingers tapping
//! - Row 1 (y=64..128):  `Working`  — 6 frames, hammer swing
//! - Row 2 (y=128..192): `NeedsHelp`— 3 frames, scratch-head twitch
//!
//! The art is chunky-rectangles — not pretty but recognisable enough
//! to dogfood the rendering pipeline. M5 replaces the bytes here with
//! a commissioned sheet without changing any of the surrounding code.
//!
//! All "art" is computed deterministically so the PNG is byte-stable
//! across runs (useful for diffing test output if needed).

use std::io::Cursor;

use crate::tui::sprite::state::SpriteState;

pub const FRAME_W: u32 = 64;
pub const FRAME_H: u32 = 64;
pub const SHEET_W: u32 = 6 * FRAME_W; // max frame count (working = 6)
pub const SHEET_H: u32 = 3 * FRAME_H;

/// A decoded sprite sheet + a function that maps `(state, frame)` to
/// pixel coordinates within the sheet.
pub struct SpriteSheet {
    png: Vec<u8>,
}

impl SpriteSheet {
    pub fn generate() -> Self {
        let rgba = render_rgba();
        let png = encode_png(SHEET_W, SHEET_H, &rgba);
        Self { png }
    }

    pub fn png_bytes(&self) -> &[u8] {
        &self.png
    }

    /// Pixel rectangle for `(state, frame)` in the sheet:
    /// `(src_x, src_y, src_w, src_h)`.
    ///
    /// `&self` is intentional: the rect depends only on `(state,
    /// frame)` today, but M5's commissioned sheet may carry a JSON
    /// frame table on the sheet itself, in which case the lookup
    /// becomes self-relative.
    #[allow(clippy::unused_self)]
    pub fn frame_rect(&self, state: SpriteState, frame: u32) -> (u32, u32, u32, u32) {
        let row = match state {
            SpriteState::Idle => 0,
            SpriteState::Working => 1,
            SpriteState::NeedsHelp => 2,
        };
        let frame = frame.min(state.frame_count().saturating_sub(1));
        (frame * FRAME_W, row * FRAME_H, FRAME_W, FRAME_H)
    }
}

// ---------------------------------------------------------------------
// Procedural art. Pure pixel math; no I/O.
// ---------------------------------------------------------------------

type Rgba = [u8; 4];

const BG: Rgba = [10, 10, 14, 255]; // near-black panel background
const SKIN: Rgba = [220, 170, 130, 255];
const APRON: Rgba = [60, 40, 30, 255];
const HAIR: Rgba = [40, 25, 15, 255];
const ANVIL: Rgba = [70, 70, 75, 255];
const HAMMER: Rgba = [50, 50, 55, 255];
const HANDLE: Rgba = [120, 80, 40, 255];
const SPARK: Rgba = [255, 200, 60, 255];
const ALERT: Rgba = [220, 90, 60, 255];

fn render_rgba() -> Vec<u8> {
    let mut buf = vec![0u8; (SHEET_W * SHEET_H * 4) as usize];
    // Background fill.
    for chunk in buf.chunks_exact_mut(4) {
        chunk.copy_from_slice(&BG);
    }

    // Idle row.
    for f in 0..SpriteState::Idle.frame_count() {
        draw_idle_frame(&mut buf, f * FRAME_W, 0, f);
    }
    // Working row.
    for f in 0..SpriteState::Working.frame_count() {
        draw_working_frame(&mut buf, f * FRAME_W, FRAME_H, f);
    }
    // NeedsHelp row.
    for f in 0..SpriteState::NeedsHelp.frame_count() {
        draw_needs_help_frame(&mut buf, f * FRAME_W, 2 * FRAME_H, f);
    }

    buf
}

fn put(buf: &mut [u8], x: u32, y: u32, c: Rgba) {
    if x >= SHEET_W || y >= SHEET_H {
        return;
    }
    let idx = ((y * SHEET_W + x) * 4) as usize;
    buf[idx..idx + 4].copy_from_slice(&c);
}

#[allow(clippy::too_many_arguments, clippy::many_single_char_names)]
fn fill_rect(buf: &mut [u8], ox: u32, oy: u32, x: u32, y: u32, w: u32, h: u32, c: Rgba) {
    for dy in 0..h {
        for dx in 0..w {
            put(buf, ox + x + dx, oy + y + dy, c);
        }
    }
}

/// Common silhouette: head + torso + apron + anvil.
fn draw_base_smith(buf: &mut [u8], ox: u32, oy: u32) {
    // Anvil (bottom): 30 wide, 6 tall, centred.
    fill_rect(buf, ox, oy, 17, 50, 30, 6, ANVIL);
    fill_rect(buf, ox, oy, 22, 56, 20, 4, ANVIL);
    // Apron (lower body).
    fill_rect(buf, ox, oy, 24, 36, 16, 14, APRON);
    // Torso (skin under apron's bib).
    fill_rect(buf, ox, oy, 26, 28, 12, 12, SKIN);
    fill_rect(buf, ox, oy, 26, 30, 12, 4, APRON); // bib strap
                                                  // Arms — stubs hanging from shoulders.
    fill_rect(buf, ox, oy, 22, 30, 4, 10, SKIN);
    fill_rect(buf, ox, oy, 38, 30, 4, 10, SKIN);
    // Head.
    fill_rect(buf, ox, oy, 27, 18, 10, 10, SKIN);
    // Hair / beanie.
    fill_rect(buf, ox, oy, 27, 18, 10, 3, HAIR);
    // Eyes — two dark pixels.
    put(buf, ox + 29, oy + 23, BG);
    put(buf, ox + 34, oy + 23, BG);
}

fn draw_idle_frame(buf: &mut [u8], ox: u32, oy: u32, frame: u32) {
    draw_base_smith(buf, ox, oy);
    // Subtle tapping: shift right hand 1px up/down based on parity.
    let dy = i32::try_from(frame % 2).unwrap();
    let arm_y = u32::try_from(40 + dy).unwrap();
    fill_rect(buf, ox, oy, 38, arm_y, 4, 4, SKIN);
    // Tiny lean: every other frame, head shifts 1px down.
    if frame == 2 {
        fill_rect(buf, ox, oy, 27, 19, 10, 10, SKIN);
        fill_rect(buf, ox, oy, 27, 19, 10, 3, HAIR);
    }
}

fn draw_working_frame(buf: &mut [u8], ox: u32, oy: u32, frame: u32) {
    draw_base_smith(buf, ox, oy);
    // Hammer arm swings through six positions: high, descending,
    // strike, sparks-flying, raised, recovering.
    let positions = [
        (40, 16, 0u32), // 0 high
        (40, 22, 0),    // 1
        (40, 32, 0),    // 2
        (40, 40, 1),    // 3 strike (sparks)
        (40, 28, 0),    // 4 recovering
        (40, 20, 0),    // 5 raised
    ];
    let (hx, hy, sparks) = positions[frame as usize % positions.len()];
    // Hammer handle.
    fill_rect(buf, ox, oy, 38, hy, 4, 14, HANDLE);
    // Hammer head.
    fill_rect(buf, ox, oy, 34, hy.saturating_sub(4), 12, 6, HAMMER);
    // Sparks on strike.
    if sparks == 1 {
        for (dx, dy) in [(20, 48), (15, 50), (47, 48), (50, 51), (32, 45)] {
            put(buf, ox + dx, oy + dy, SPARK);
        }
    }
    // Hammer-side hand follows.
    fill_rect(buf, ox, oy, 38, hy + 14, 4, 4, SKIN);
    // Left arm braces the work.
    fill_rect(buf, ox, oy, 22, 38, 4, 4, SKIN);
    let _ = hx; // (kept for clarity; the x is already 40)
}

fn draw_needs_help_frame(buf: &mut [u8], ox: u32, oy: u32, frame: u32) {
    draw_base_smith(buf, ox, oy);
    // Scratch-head animation: right arm rises towards the head.
    let arm_y = match frame % 3 {
        0 => 26,
        1 => 22,
        _ => 18,
    };
    fill_rect(buf, ox, oy, 38, arm_y, 4, 10, SKIN);
    // Alert "!" above the head.
    fill_rect(buf, ox, oy, 32, 6, 2, 5, ALERT);
    put(buf, ox + 32, oy + 13, ALERT);
    put(buf, ox + 33, oy + 13, ALERT);
}

// ---------------------------------------------------------------------
// PNG encoding.
// ---------------------------------------------------------------------

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 2);
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(rgba).expect("png write");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_has_three_rows() {
        let s = SpriteSheet::generate();
        let png = s.png_bytes();
        // PNG signature.
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // The header IHDR chunk encodes the dimensions; we'll just
        // re-decode and assert.
        let decoder = png::Decoder::new(Cursor::new(png));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!(info.width, SHEET_W);
        assert_eq!(info.height, SHEET_H);
    }

    #[test]
    fn frame_rect_indexes_correctly() {
        let s = SpriteSheet::generate();

        // Idle row 0:
        assert_eq!(s.frame_rect(SpriteState::Idle, 0), (0, 0, 64, 64));
        assert_eq!(s.frame_rect(SpriteState::Idle, 3), (192, 0, 64, 64));

        // Working row 1:
        assert_eq!(s.frame_rect(SpriteState::Working, 0), (0, 64, 64, 64));
        assert_eq!(s.frame_rect(SpriteState::Working, 5), (320, 64, 64, 64));

        // NeedsHelp row 2:
        assert_eq!(s.frame_rect(SpriteState::NeedsHelp, 0), (0, 128, 64, 64));
        assert_eq!(s.frame_rect(SpriteState::NeedsHelp, 2), (128, 128, 64, 64));
    }

    #[test]
    fn frame_clamped_to_count() {
        let s = SpriteSheet::generate();
        // Asking for frame 99 on a 4-frame state returns the last frame.
        assert_eq!(s.frame_rect(SpriteState::Idle, 99), (192, 0, 64, 64));
    }

    #[test]
    fn determinism() {
        let a = SpriteSheet::generate();
        let b = SpriteSheet::generate();
        assert_eq!(a.png, b.png, "sprite generation must be deterministic");
    }
}
