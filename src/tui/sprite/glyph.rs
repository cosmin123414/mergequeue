//! Text/ratatui rendering of the Penrose tiling — the fallback used on
//! terminals that don't speak the Kitty graphics protocol.
//!
//! The renderer:
//!
//! - [`render_braille`] (preferred) — high-resolution wireframe. A
//!   [`DotBitmap`] of `cols*2 × rows*4` edge dots is packed into Unicode
//!   Braille cells (each cell is a 2×4 dot matrix), giving 8× the
//!   resolution of one glyph per cell. This is what the TUI uses.
//!
//! The per-state color mirrors the pixel renderer: muted green when
//! idle, livelier green when working, amber when an entry needs help.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::tui::sprite::penrose::DotBitmap;
use crate::tui::sprite::state::SpriteState;

/// Cells with effectively zero coverage aren't drawn at all (they show
/// the terminal background). This is a near-zero epsilon, not a visible
/// threshold: the fade is continuous from 0, so an emerging edge ramps
/// smoothly up from the background instead of popping in at a floor.
const REVEAL_FLOOR: f32 = 1.0 / 255.0;

/// Faded-out floor as a fraction of the full wire color. An emerging dot
/// starts here (a dim version of the *same* hue) and ramps to full — so
/// the low end of the fade is a faded green, never black.
const FADE_FLOOR: f32 = 0.30;

/// Fade a channel from the faded-out floor (`FADE_FLOOR * channel`) up to
/// the full `channel`, by opacity `a∈[0,1]`. At `a=0` the dot is the dim
/// floor color (same hue, lower brightness); at `a=1` it's the full wire
/// color. The result stays in `[0, 255]`, so the cast is always in range.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fade(channel: u8, a: f32) -> u8 {
    let c = f32::from(channel);
    let floor = c * FADE_FLOOR;
    (floor + (c - floor) * a).round() as u8
}

/// Base foreground RGB for the Braille wireframe, per animation mood.
/// `render_braille` fades each cell from a dim floor of this hue up to
/// full by its opacity.
fn wire_rgb(state: SpriteState) -> (u8, u8, u8) {
    match state {
        SpriteState::Idle => (0x8f, 0xa8, 0x86),
        SpriteState::Working | SpriteState::NeedsHelp => (0x9d, 0xc1, 0x8f),
    }
}

fn gradient_wire_rgb(x: u16, y: u16, width: u16, height: u16, base: (u8, u8, u8)) -> (u8, u8, u8) {
    const ORANGE: (u8, u8, u8) = (233, 132, 95);

    let cx = f32::from(width) * 0.5;
    let cy = f32::from(height) * 0.5;
    let rx = (f32::from(x) + 0.5 - cx) / cx.max(1.0);
    let ry = (f32::from(y) + 0.5 - cy) / cy.max(1.0);
    let radial = (rx.mul_add(rx, ry * ry).sqrt()).clamp(0.0, 1.0);
    let t = ((radial - 0.45) / 0.55).clamp(0.0, 1.0).powf(1.35);

    (
        blend_channel(base.0, ORANGE.0, t),
        blend_channel(base.1, ORANGE.1, t),
        blend_channel(base.2, ORANGE.2, t),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn blend_channel(from: u8, to: u8, t: f32) -> u8 {
    (f32::from(from) + (f32::from(to) - f32::from(from)) * t.clamp(0.0, 1.0)).round() as u8
}

/// Pack a [`DotBitmap`] into Braille `Line`s. The bitmap is `cols*2`
/// wide and `rows*4` tall; each `2×4` block becomes one Braille glyph
/// (`U+2800` + dot bits). Empty cells render as a space so the widget
/// doesn't paint a background block over the terminal.
///
/// Braille dot → bit layout (within each 2×4 cell):
///
/// ```text
///   (0,0)=0x01  (1,0)=0x08
///   (0,1)=0x02  (1,1)=0x10
///   (0,2)=0x04  (1,2)=0x20
///   (0,3)=0x40  (1,3)=0x80
/// ```
#[must_use]
pub fn render_braille(bitmap: &DotBitmap, state: SpriteState) -> Vec<Line<'static>> {
    // Bit weight for the dot at in-cell position (col, row), col∈0..2,
    // row∈0..4.
    const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

    let cols = bitmap.width / 2;
    let rows = bitmap.height / 4;
    let (fr, fg_, fb) = wire_rgb(state);

    let mut lines = Vec::with_capacity(usize::from(rows));
    for cy in 0..rows {
        // Build the row cell-by-cell. Each Braille glyph's color is the
        // full wire color faded toward black by the cell's opacity, so
        // emerging/receding detail (and the loop wrap) fades smoothly
        // instead of snapping. Same-color runs coalesce into one span.
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut run = String::new();
        let mut run_color: Option<Color> = None;

        for cx in 0..cols {
            let mut pattern: u8 = 0;
            let mut cell_alpha: f32 = 0.0;
            for (ry, row_bits) in (0u16..).zip(BITS.iter()) {
                for (rx, bit) in (0u16..).zip(row_bits.iter()) {
                    let dx = cx * 2 + rx;
                    let dy = cy * 4 + ry;
                    if bitmap.at(dx, dy) {
                        pattern |= bit;
                        cell_alpha = cell_alpha.max(bitmap.alpha_at(dx, dy));
                    }
                }
            }

            // Continuous fade from a dim floor: an emerging edge ramps
            // smoothly from a faded-out version of the wire hue (NOT black)
            // up to the full wire color, scaled by the cell's opacity (see
            // `fade`/`FADE_FLOOR`). Only truly-zero cells are skipped (they
            // show the bg); every visible cell stays recognizably green.
            let a = cell_alpha.clamp(0.0, 1.0);
            let (glyph_ch, color) = if pattern == 0 || a < REVEAL_FLOOR {
                (' ', Color::Reset)
            } else {
                let ch = char::from_u32(0x2800 + u32::from(pattern)).unwrap_or(' ');
                let (red, green, blue) = gradient_wire_rgb(cx, cy, cols, rows, (fr, fg_, fb));
                let color = Color::Rgb(fade(red, a), fade(green, a), fade(blue, a));
                (ch, color)
            };

            if run_color == Some(color) {
                run.push(glyph_ch);
            } else {
                if let Some(prev) = run_color.take() {
                    spans.push(Span::styled(
                        std::mem::take(&mut run),
                        Style::default().fg(prev),
                    ));
                }
                run.push(glyph_ch);
                run_color = Some(color);
            }
        }
        if let Some(prev) = run_color {
            spans.push(Span::styled(run, Style::default().fg(prev)));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Render the edge bitmap as a terminal-friendly approximation of the Kitty
/// hex-dot visual: sample the original edge mask onto a staggered lattice, then
/// pack those lattice dots into Braille cells. The terminal still constrains us
/// to a rectangular 2×4 subcell grid, but the staggered resampling avoids the
/// fully-occupied rectangular Braille rails that make some edge families pop.
#[must_use]
pub fn render_hex_braille(bitmap: &DotBitmap, state: SpriteState) -> Vec<Line<'static>> {
    let len = usize::from(bitmap.width) * usize::from(bitmap.height);
    let mut dots = vec![false; len];
    let mut alpha = vec![0u8; len];

    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            if !bitmap.at(x, y) {
                continue;
            }
            let idx = usize::from(y) * usize::from(bitmap.width) + usize::from(x);
            let a = bitmap.alpha[idx];
            if a == 0 || should_thin_dot(x, y, a) {
                continue;
            }
            dots[idx] = true;
            alpha[idx] = a;
        }
    }

    render_braille(
        &DotBitmap {
            width: bitmap.width,
            height: bitmap.height,
            dots,
            alpha,
        },
        state,
    )
}

fn should_thin_dot(x: u16, y: u16, alpha: u8) -> bool {
    // Static blue-noise-ish thinning with a soft staggered bias. A plain
    // rectangular Braille field makes long diagonal staircases too obvious;
    // a hard hex lattice makes visible dot clusters. This keeps the stroke
    // continuous, but favors alternating row phases so rails don't align for
    // long runs on the terminal's square subgrid.
    let preferred_stagger = (x + (y & 1)) & 1 == 0;
    let keep = if preferred_stagger {
        168 + u32::from(alpha) * 72 / 255
    } else {
        104 + u32::from(alpha) * 72 / 255
    };
    (hash_u16(x, y) & 0xff) >= keep
}

fn hash_u16(x: u16, y: u16) -> u32 {
    let mut v = u32::from(x).wrapping_mul(0x7feb_352d) ^ u32::from(y).wrapping_mul(0x846c_a68b);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2c1b_3c6d);
    v ^= v >> 12;
    v = v.wrapping_mul(0x297a_2d39);
    v ^ (v >> 15)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::sprite::penrose::PenroseAnimator;

    #[test]
    fn braille_lines_match_cell_dimensions() {
        let mut anim = PenroseAnimator::new();
        let (cols, rows) = (40u16, 20u16);
        let bmp = anim.dot_bitmap(0.08, SpriteState::Working, cols * 2, rows * 4);
        let lines = render_braille(&bmp, SpriteState::Working);
        assert_eq!(lines.len(), usize::from(rows));
        for line in &lines {
            let width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(width, usize::from(cols));
        }
    }

    #[test]
    fn hex_braille_lines_match_cell_dimensions_and_draw() {
        let mut anim = PenroseAnimator::new();
        let (cols, rows) = (40u16, 20u16);
        let bmp = anim.dot_bitmap(0.08, SpriteState::Working, cols * 2, rows * 4);
        let lines = render_hex_braille(&bmp, SpriteState::Working);
        assert_eq!(lines.len(), usize::from(rows));
        let mut visible = 0usize;
        for line in &lines {
            let width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(width, usize::from(cols));
            visible += line
                .spans
                .iter()
                .flat_map(|s| s.content.chars())
                .filter(|&ch| ch != ' ')
                .count();
        }
        assert!(
            visible > 0,
            "expected hex Braille renderer to draw visible cells"
        );
    }

    #[test]
    fn braille_uses_block_code_points_for_dots() {
        let mut anim = PenroseAnimator::new();
        let bmp = anim.dot_bitmap(0.08, SpriteState::Idle, 96, 96);
        let lines = render_braille(&bmp, SpriteState::Idle);
        // At least some cells should be non-space Braille glyphs in the
        // U+2800..=U+28FF block (the wireframe is visible).
        let braille = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .flat_map(|s| s.content.chars())
            .filter(|c| ('\u{2801}'..='\u{28FF}').contains(c))
            .count();
        assert!(
            braille > 20,
            "expected a visible wireframe, got {braille} dots"
        );
    }
}
