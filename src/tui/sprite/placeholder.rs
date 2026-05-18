// The procedural drawing code does a lot of float-math → pixel
// coordinate conversions. The casts are deliberate and bounded
// (coords clamp to the frame); silencing the cast lints
// crate-wide keeps the math readable.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

//! Procedurally-generated stippled blacksmith sprite sheet.
//!
//! Aesthetic target: Dark Souls "Andre of Astora" — a hooded smith
//! hunched at an anvil — rendered entirely as white dots on a black
//! background. Stippling rather than filled shapes, so the figure
//! reads as illustration rather than as cartoon pixel-art.
//!
//! The sheet contains three rows of 3 frames each (320×448 per frame):
//!
//! - Row 0 (Idle): smith stands at the anvil, sparse forge embers float.
//! - Row 1 (Working): same pose, dense bright sparks puff upward.
//! - Row 2 (NeedsHelp): silhouette dimmed, a stipple question mark
//!   pulses above the head.
//!
//! Implementation strategy:
//!
//! 1. Define a **silhouette field** as a sum of soft-edged "blobs"
//!    (filled ellipses with falloff). Each body part — hood, torso,
//!    arms, hammer, anvil — is one or more blobs.
//! 2. For every pixel, compute the silhouette density and a
//!    deterministic per-pixel hash. Paint white iff `hash < density`.
//!    The hash gives the dot field an organic, non-gridded look.
//! 3. Per-frame variations (sparks, question mark, dim factor) are
//!    drawn on top of the base silhouette using the same stipple
//!    rule with different fields.
//!
//! Everything is deterministic so test assertions on dimensions and
//! frame layout stay stable.

use std::io::Cursor;

use crate::tui::sprite::state::SpriteState;

pub const FRAME_W: u32 = 320;
pub const FRAME_H: u32 = 448;
pub const FRAMES_PER_STATE: u32 = 3;
pub const SHEET_W: u32 = FRAME_W * FRAMES_PER_STATE;
pub const SHEET_H: u32 = FRAME_H * 3;

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

    /// Pixel rectangle `(src_x, src_y, src_w, src_h)` for the given
    /// `(state, frame)`. Frames out of range clamp to the last
    /// frame in the row.
    #[allow(clippy::unused_self)]
    pub fn frame_rect(&self, state: SpriteState, frame: u32) -> (u32, u32, u32, u32) {
        let row = match state {
            SpriteState::Idle => 0,
            SpriteState::Working => 1,
            SpriteState::NeedsHelp => 2,
        };
        let frames = state.frame_count().min(FRAMES_PER_STATE);
        let frame = frame.min(frames.saturating_sub(1));
        (frame * FRAME_W, row * FRAME_H, FRAME_W, FRAME_H)
    }
}

// ---------------------------------------------------------------------
// Render pipeline. Pure pixel math.
// ---------------------------------------------------------------------

type Rgba = [u8; 4];
const WHITE: Rgba = [240, 240, 235, 255]; // very-slightly-warm white
const DIM: Rgba = [110, 110, 105, 255];
const EMBER: Rgba = [255, 180, 90, 255]; // forge ember orange — used sparingly

fn render_rgba() -> Vec<u8> {
    // Fully transparent background (alpha = 0). Pixels we paint
    // become opaque where the stipple lands; everything else lets
    // the terminal background show through, so the smith doesn't
    // render as a hard black rectangle on non-black terminals.
    let mut buf = vec![0u8; (SHEET_W * SHEET_H * 4) as usize];

    for row_idx in 0..3 {
        let state = match row_idx {
            0 => SpriteState::Idle,
            1 => SpriteState::Working,
            _ => SpriteState::NeedsHelp,
        };
        for frame in 0..FRAMES_PER_STATE {
            let ox = frame * FRAME_W;
            let oy = row_idx * FRAME_H;
            paint_frame(&mut buf, ox, oy, state, frame);
        }
    }

    buf
}

/// Paint one 320×448 frame. `(ox, oy)` is its top-left within the
/// sheet. `state` selects the variant; `frame` selects the
/// animation step.
fn paint_frame(buf: &mut [u8], ox: u32, oy: u32, state: SpriteState, frame: u32) {
    // NeedsHelp dims the silhouette so the question mark pops.
    let body_color = if matches!(state, SpriteState::NeedsHelp) {
        DIM
    } else {
        WHITE
    };

    // 1. Base silhouette.
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let density = silhouette_density(x, y);
            if density <= 0.0 {
                continue;
            }
            if hash01(x, y, 0) < density {
                put_pixel(buf, ox, oy, x, y, body_color);
            }
        }
    }

    // 2. Rim highlights along the hood / shoulder / hammer.
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let rim = rim_density(x, y);
            if rim <= 0.0 {
                continue;
            }
            if hash01(x, y, 17) < rim {
                put_pixel(buf, ox, oy, x, y, WHITE);
            }
        }
    }

    // 3. Per-state overlay.
    match state {
        SpriteState::Idle => paint_embers(buf, ox, oy, frame, 0.35),
        SpriteState::Working => paint_embers(buf, ox, oy, frame, 1.0),
        SpriteState::NeedsHelp => paint_question_mark(buf, ox, oy, frame),
    }
}

#[inline]
fn put_pixel(buf: &mut [u8], ox: u32, oy: u32, x: u32, y: u32, c: Rgba) {
    if x >= FRAME_W || y >= FRAME_H {
        return;
    }
    let xx = ox + x;
    let yy = oy + y;
    let idx = ((yy * SHEET_W + xx) * 4) as usize;
    buf[idx..idx + 4].copy_from_slice(&c);
}

// ---------------------------------------------------------------------
// Density fields. Each returns a value in [0, ~0.6] that we compare
// to a per-pixel hash. Soft falloff at the edges produces the
// stippled appearance.
// ---------------------------------------------------------------------

/// Total silhouette density: union of all body-part blobs.
fn silhouette_density(x: u32, y: u32) -> f32 {
    let fx = x as f32;
    let fy = y as f32;

    let mut d: f32 = 0.0;

    // The composition:
    //   center-x = 160, the smith stands centered.
    //   anvil sits at y ≈ 340..400
    //   torso  y ≈ 180..340
    //   head   y ≈ 100..200
    //   hammer arm raises right of head
    //   left hand grips anvil
    //
    // Each blob is `ellipse(cx, cy, rx, ry, edge_softness)` adding
    // to the density. Densities clip at ~0.6 to leave room for
    // texture variation.

    // Anvil — wide flat block with a horn on the right.
    d = d.max(ellipse(fx, fy, 160.0, 360.0, 110.0, 26.0, 6.0));
    d = d.max(ellipse(fx, fy, 245.0, 358.0, 30.0, 14.0, 4.0)); // horn
    d = d.max(ellipse(fx, fy, 160.0, 395.0, 95.0, 18.0, 6.0)); // base
    d = d.max(ellipse(fx, fy, 160.0, 415.0, 70.0, 12.0, 5.0)); // foot

    // Torso — broad shoulders, slightly hunched.
    d = d.max(ellipse(fx, fy, 158.0, 250.0, 78.0, 70.0, 12.0));
    // Belly bulge.
    d = d.max(ellipse(fx, fy, 162.0, 295.0, 70.0, 38.0, 10.0));
    // Apron front — a slightly higher-density region at the waist
    // gives the impression of a leather apron.
    d = d.max(ellipse(fx, fy, 160.0, 320.0, 60.0, 25.0, 8.0) * 0.95);

    // Left arm bracing the anvil.
    d = d.max(ellipse(fx, fy, 95.0, 290.0, 22.0, 50.0, 8.0));
    d = d.max(ellipse(fx, fy, 92.0, 340.0, 26.0, 18.0, 6.0)); // hand on anvil

    // Right arm raised behind the head — broad upper arm, forearm
    // angles back toward the upper-right corner.
    d = d.max(ellipse(fx, fy, 220.0, 220.0, 28.0, 36.0, 8.0));
    d = d.max(ellipse_rotated(fx, fy, 250.0, 165.0, 16.0, 60.0, 0.5, 8.0));
    // Hammer head — small bright block at the end of the forearm.
    d = d.max(ellipse(fx, fy, 282.0, 110.0, 22.0, 14.0, 5.0));
    d = d.max(ellipse(fx, fy, 282.0, 110.0, 14.0, 8.0, 3.0) * 1.05);

    // Hood — a large droplet shape covering the head. The bottom is
    // wider (covers shoulders); the top tapers to a point slightly
    // to the upper-left to suggest cloth.
    d = d.max(ellipse(fx, fy, 158.0, 150.0, 56.0, 62.0, 14.0));
    // The deep shadow inside the hood — a dark zone is achieved
    // not by reducing density, but by cutting it back where the
    // face would be (we subtract).
    let face_cutout = ellipse(fx, fy, 158.0, 158.0, 28.0, 36.0, 10.0);
    d -= face_cutout * 0.95;
    if d < 0.0 {
        d = 0.0;
    }

    // Beard suggestion: a slightly denser zone at the lower jaw.
    d = d.max(ellipse(fx, fy, 158.0, 195.0, 26.0, 18.0, 6.0));

    // Clip to a comfortable max so the figure has texture rather
    // than appearing as solid white.
    d.clamp(0.0, 0.55)
}

/// Bright rim accents — sparse high-density bands along the hood top
/// and the hammer head, to suggest a light source from above-left.
fn rim_density(x: u32, y: u32) -> f32 {
    let fx = x as f32;
    let fy = y as f32;
    let mut d: f32 = 0.0;
    // Hood top-left rim.
    d = d.max(arc_band(fx, fy, 158.0, 150.0, 56.0, 62.0, 5.0, -1.7, -0.4));
    // Shoulder highlight.
    d = d.max(arc_band(fx, fy, 158.0, 220.0, 84.0, 70.0, 4.0, -2.4, -1.6));
    // Hammer head top edge.
    d = d.max(ellipse(fx, fy, 282.0, 104.0, 18.0, 4.0, 3.0));
    d.clamp(0.0, 0.7)
}

/// Forge embers / sparks floating above the anvil. Density and
/// brightness vary by frame so the animation reads as flicker. We
/// keep them small and few so they don't overpower the figure.
fn paint_embers(buf: &mut [u8], ox: u32, oy: u32, frame: u32, intensity: f32) {
    let n_embers: u32 = (35.0 * intensity) as u32;
    for i in 0..n_embers {
        let h1 = hash01(i, frame, 101);
        let h2 = hash01(i, frame, 211);
        let x_signed = 60.0_f32 + h1 * 200.0;
        let y_signed = 260.0_f32 - h2 * 200.0;
        if x_signed < 0.0 || y_signed < 0.0 {
            continue;
        }
        let x = x_signed as u32;
        let y = y_signed as u32;

        // 3-frame flicker.
        if hash01(i, frame, 311) <= 0.35 {
            continue;
        }
        let brightness = hash01(i, frame, 401);
        let color = if brightness > 0.65 {
            EMBER
        } else if brightness > 0.4 {
            WHITE
        } else {
            DIM
        };
        put_pixel(buf, ox, oy, x, y, color);
        if brightness > 0.85 {
            put_pixel(buf, ox, oy, x.saturating_add(1), y, color);
        }
    }

    // Working: upward streaks above the hammer-strike zone.
    if intensity > 0.7 {
        for i in 0..18 {
            if hash01(i, frame, 503) < 0.3 {
                continue;
            }
            let x_signed = 220.0_f32 + hash01(i, frame, 509) * 60.0;
            let base_y: i32 = 340;
            let drift = (frame as i32) * 14 - (hash01(i, frame, 601) * 30.0) as i32;
            let y_signed = base_y - drift - (hash01(i, frame, 701) * 60.0) as i32;
            if x_signed < 0.0 || y_signed < 0 {
                continue;
            }
            put_pixel(buf, ox, oy, x_signed as u32, y_signed as u32, WHITE);
        }
    }
}

/// A small stipple-question-mark above the hood. Pulses on frame 1.
fn paint_question_mark(buf: &mut [u8], ox: u32, oy: u32, frame: u32) {
    let stroke = if matches!(frame, 1) { WHITE } else { DIM };
    let cx = 158.0_f32;
    let cy = 60.0_f32;

    // Top arc.
    for t in 0..40 {
        let theta = std::f32::consts::PI * (1.2 - (t as f32) / 40.0 * 1.6);
        let r = 18.0;
        let x_signed = cx + theta.cos() * r;
        let y_signed = cy - theta.sin() * r;
        if x_signed < 0.0 || y_signed < 0.0 {
            continue;
        }
        if hash01(t, frame, 13) < 0.85 {
            put_pixel(buf, ox, oy, x_signed as u32, y_signed as u32, stroke);
        }
    }
    // Tail.
    for t in 0..16 {
        let x_signed = cx + 6.0;
        let y_signed = cy + 14.0 + t as f32;
        if x_signed < 0.0 || y_signed < 0.0 {
            continue;
        }
        if hash01(t, frame, 19) < 0.9 {
            put_pixel(buf, ox, oy, x_signed as u32, y_signed as u32, stroke);
        }
    }
    // Dot.
    for dy in 0..4u32 {
        for dx in 0..4u32 {
            let x = (cx + 4.0 + dx as f32) as u32;
            let y = (cy + 36.0 + dy as f32) as u32;
            if hash01(dx + dy * 10, frame, 23) < 0.7 {
                put_pixel(buf, ox, oy, x, y, stroke);
            }
        }
    }
}

// ---------------------------------------------------------------------
// Math helpers.
// ---------------------------------------------------------------------

/// Density contribution of an axis-aligned ellipse with a soft edge.
/// Returns 0 outside the ellipse-plus-edge, ramps from `edge` to 1
/// across the soft band, and `0.45` inside. (Capping inside-density
/// at ~0.45 is what gives the stipple texture; pure 1.0 would paint
/// solid white.)
fn ellipse(px: f32, py: f32, cx: f32, cy: f32, rx: f32, ry: f32, edge: f32) -> f32 {
    let dx = (px - cx) / rx;
    let dy = (py - cy) / ry;
    let r = (dx * dx + dy * dy).sqrt();
    // r = 0 at center, 1 at the ellipse boundary.
    if r >= 1.0 {
        // Soft falloff outside.
        let outside = (r - 1.0) * rx.min(ry) / edge.max(0.5);
        if outside >= 1.0 {
            0.0
        } else {
            0.4 * (1.0 - outside)
        }
    } else {
        // Inside: density increases toward the center.
        0.30 + 0.25 * (1.0 - r)
    }
}

/// Same as `ellipse` but rotated by `theta` radians. Used for the
/// forearm which angles up and back.
#[allow(clippy::too_many_arguments)]
fn ellipse_rotated(
    px: f32,
    py: f32,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    theta: f32,
    edge: f32,
) -> f32 {
    let dx = px - cx;
    let dy = py - cy;
    let (s, c) = theta.sin_cos();
    let rxp = (dx * c + dy * s) / rx;
    let ryp = (-dx * s + dy * c) / ry;
    let r = (rxp * rxp + ryp * ryp).sqrt();
    if r >= 1.0 {
        let outside = (r - 1.0) * rx.min(ry) / edge.max(0.5);
        if outside >= 1.0 {
            0.0
        } else {
            0.4 * (1.0 - outside)
        }
    } else {
        0.30 + 0.25 * (1.0 - r)
    }
}

/// A thin arc band on the boundary of an ellipse, used for rim
/// highlights. `theta_lo`/`theta_hi` clip the band to a specific
/// angular range so we only light one side of the hood.
#[allow(clippy::too_many_arguments)]
fn arc_band(
    px: f32,
    py: f32,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    width: f32,
    theta_lo: f32,
    theta_hi: f32,
) -> f32 {
    let dx = (px - cx) / rx;
    let dy = (py - cy) / ry;
    let r = (dx * dx + dy * dy).sqrt();
    if (r - 1.0).abs() > (width / rx.min(ry)) {
        return 0.0;
    }
    let theta = dy.atan2(dx);
    if theta < theta_lo || theta > theta_hi {
        return 0.0;
    }
    // Density tapers off at the ends of the angular range so the
    // highlight has soft endpoints.
    let mid = (theta_lo + theta_hi) * 0.5;
    let half = (theta_hi - theta_lo) * 0.5;
    let t = ((theta - mid).abs() / half.max(0.01)).clamp(0.0, 1.0);
    0.55 * (1.0 - t)
}

/// Deterministic per-pixel hash in [0, 1). `salt` lets different
/// stipple layers (body / rim / overlays) draw uncorrelated patterns
/// at the same `(x, y)`.
fn hash01(x: u32, y: u32, salt: u32) -> f32 {
    // FNV-1a 32-bit, then map to [0, 1) — fast and good-enough.
    let mut h: u32 = 0x811c_9dc5;
    for byte in x.to_le_bytes() {
        h ^= u32::from(byte);
        h = h.wrapping_mul(16_777_619);
    }
    for byte in y.to_le_bytes() {
        h ^= u32::from(byte);
        h = h.wrapping_mul(16_777_619);
    }
    for byte in salt.to_le_bytes() {
        h ^= u32::from(byte);
        h = h.wrapping_mul(16_777_619);
    }
    (h as f32) / (u32::MAX as f32)
}

// ---------------------------------------------------------------------
// PNG encoding.
// ---------------------------------------------------------------------

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4);
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // The black/white stipple compresses well; bump the
        // compression level so the embedded asset stays small.
        encoder.set_compression(png::Compression::Best);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(rgba).expect("png write");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_has_correct_dimensions() {
        let s = SpriteSheet::generate();
        let decoder = png::Decoder::new(Cursor::new(s.png_bytes()));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!(info.width, SHEET_W);
        assert_eq!(info.height, SHEET_H);
    }

    #[test]
    fn frame_rect_indexes_correctly() {
        let s = SpriteSheet::generate();
        // Row 0 (Idle), first frame: top-left of sheet.
        assert_eq!(s.frame_rect(SpriteState::Idle, 0), (0, 0, FRAME_W, FRAME_H));
        // Last Idle frame: (2 * FRAME_W, 0).
        assert_eq!(
            s.frame_rect(SpriteState::Idle, 2),
            (2 * FRAME_W, 0, FRAME_W, FRAME_H)
        );
        // Working row: y starts at FRAME_H.
        assert_eq!(
            s.frame_rect(SpriteState::Working, 0),
            (0, FRAME_H, FRAME_W, FRAME_H)
        );
        // NeedsHelp row: y starts at 2*FRAME_H.
        assert_eq!(
            s.frame_rect(SpriteState::NeedsHelp, 0),
            (0, 2 * FRAME_H, FRAME_W, FRAME_H)
        );
    }

    #[test]
    fn frame_clamps_to_last_in_state() {
        let s = SpriteSheet::generate();
        // NeedsHelp has 3 frames; asking for frame 99 returns the last.
        assert_eq!(
            s.frame_rect(SpriteState::NeedsHelp, 99),
            (2 * FRAME_W, 2 * FRAME_H, FRAME_W, FRAME_H)
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let a = SpriteSheet::generate();
        let b = SpriteSheet::generate();
        assert_eq!(a.png, b.png);
    }

    #[test]
    fn hash01_is_in_unit_interval() {
        for x in 0..50 {
            for y in 0..50 {
                let h = hash01(x, y, 7);
                assert!((0.0..1.0).contains(&h), "hash01({x},{y})={h}");
            }
        }
    }

    #[test]
    fn ellipse_falls_off_to_zero_far_away() {
        // Far outside the ellipse the density must be exactly 0 (we
        // rely on the `<= 0.0` early-return in the render loop).
        let d = ellipse(1000.0, 1000.0, 0.0, 0.0, 10.0, 10.0, 4.0);
        assert!(d <= f32::EPSILON, "expected ~0, got {d}");
    }

    #[test]
    fn ellipse_peaks_at_center() {
        let center = ellipse(50.0, 50.0, 50.0, 50.0, 30.0, 30.0, 6.0);
        let edge = ellipse(80.0, 50.0, 50.0, 50.0, 30.0, 30.0, 6.0);
        assert!(center > edge);
    }
}
