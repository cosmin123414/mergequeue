// The tiling math does a lot of float → pixel-coordinate conversions.
// The casts are deliberate and bounded (coords clamp to the frame);
// silencing the cast lints module-wide keeps the geometry readable.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

//! Penrose P3 (thick + thin rhombs) inflation loader, sampled into a
//! high-resolution dot bitmap for the Braille wireframe renderer.
//!
//! This is a Rust port of the `penrose-loader` canvas animation. Unlike
//! the old sprite sheet (a single static PNG placed by sub-rectangle),
//! the Penrose tiling is a *continuous, ever-evolving zoom*: each frame
//! is freshly sampled from the tiling geometry for the current time.
//! [`PenroseAnimator`] owns the tiling state and produces one
//! [`DotBitmap`] of rhombus edges per call to
//! [`PenroseAnimator::dot_bitmap`].
//!
//! ## Geometry
//!
//! We carry "Robinson triangles" (half-rhombs) because they have a
//! single clean subdivision rule. Two kinds:
//!
//! - `0` = thin half (acute angle 36° at A) — part of a thin rhomb.
//! - `1` = thick half (acute angle 72° at A) — part of a thick rhomb.
//!
//! Each triangle is `{ kind, a, b, c }` with complex points. Subdivision
//! uses the golden ratio φ.
//!
//! ## Emergence cross-fade zoom
//!
//! Penrose tilings are only *statistically* self-similar, so a
//! pixel-perfect loop by pure zoom is impossible. We use the trick the
//! eye accepts as an infinite zoom: keep a COARSE tiling and its FINE
//! (and next-finer) subdivisions; per cycle the camera zooms in by φ
//! while finer detail fades in center-out. At the wrap we promote
//! `fine → coarse` (an exact identity) and rebuild.

use std::f32::consts::PI;

use crate::tui::sprite::state::SpriteState;

/// Output frame dimensions. Square — the Penrose disk is centered.
pub const FRAME_W: u32 = 384;
pub const FRAME_H: u32 = 384;

const PHI: f32 = 1.618_034; // (1 + sqrt 5) / 2
const IPHI: f32 = 0.618_034; // 1 / PHI = PHI - 1

/// Clip-circle radius as a fraction of `min(w, h)`. Near 0.5 so the disk
/// fills the frame edge-to-edge.
const CLIP_FRAC: f32 = 0.5;
/// Generations used to build the starting coarse tiling. Fewer
/// generations → larger base rhombi (the tiling is less subdivided), so
/// each tile takes up more of the canvas.
const COARSE_GEN: usize = 2;
/// Working-envelope cull radius (model units).
///
/// Must be generous enough that the φ-scale at the wrap never culls a
/// tile that is still inside the visible disk. The visible disk reaches
/// model-radius ≈1.05; after a wrap the geometry scales by φ, so a tile
/// at radius `1.05·φ ≈ 1.70` must survive in the pre-wrap envelope to
/// reappear seamlessly. 2.2 covers that with margin. (1.35 was too tight
/// and culled visible outer tiles at the wrap → a visible "pop".)
const CULL_R: f32 = 2.2;
/// Fraction of each *fading* edge's own length trimmed from EACH end when
/// the layer is fully faded out (`w = 0`); it shrinks to 0 as the layer
/// fades in (`w → 1`). Trimming proportionally (not by a fixed pixel gap)
/// pulls back short edges near a convergence hub just as much as long
/// ones, so emerging edges grow outward from their middles and their
/// shared subdivision vertices close LAST — killing the starburst hub
/// that otherwise forms at convergence points before the rays fill in.
/// 0.5 would collapse an edge to a point at w=0; this leaves a short but
/// *visible* stub at w=0 (kept middle = `1 - 2·END_GAP_FRAC` of the edge),
/// so a newly-emerging edge reads as a tiny line segment rather than a
/// single flickering dot. It must stay below 0.5 so the convergence-hub
/// vertices (the shared endpoints) still close last and don't starburst.
const END_GAP_FRAC: f32 = 0.38;
/// Fraction of the cycle over which the reveal happens. The fade-in, endpoint
/// closing, and brightness ramp all complete by `tau = REVEAL_DONE` and then
/// hold full for the rest of the cycle. Past this point the geometry is done;
/// the only motion is the continuing zoom (lines magnify outward smoothly).
/// This stops the slow accretion/"pulse" where detail keeps growing in areas
/// that have already settled — the reveal finishes, then the field is stable
/// until the wrap.
const REVEAL_DONE: f32 = 0.85;
/// When the coarsest (leaving) layer starts fading out within a cycle. It
/// holds full until here, then eases to 0 by the wrap so the discarded
/// structure dissolves gradually rather than popping at the promotion.
const COARSE_FADE_START: f32 = 0.6;
/// Start closing fading-edge endpoint gaps faster once the line bodies are
/// already mostly visible. Keeping the original trim until this point avoids
/// early starburst hubs; closing to zero before promotion avoids the end-of-
/// fade thickness pulse where trimmed fade edges suddenly become untrimmed
/// settled edges.
const END_GAP_CLOSE_START: f32 = 0.85;
/// By this fade weight, fading edges are already full-length. The remaining
/// fade to 1.0 is color/alpha-only, so promotion to settled geometry no
/// longer changes which dots are occupied.
const END_GAP_CLOSE_DONE: f32 = 0.97;
/// Do not render the fading-in detail until its trimmed edge body is at least
/// one-third revealed. Before this point edges are only a few dots long and
/// tend to read as isolated particles as they slide across the Braille grid.
/// Alpha is re-normalized from this gate so the layer still fades in smoothly
/// instead of popping on at partial brightness.
const REVEAL_GATE: f32 = 1.0 / 3.0;
/// Fade weight at which the next layer is treated as fully opaque. Set to
/// the point where the edge bodies are essentially connected — NOT all the
/// way to promotion. Brightness reaches full when the geometry reads as
/// complete and then holds flat; without this the alpha keeps climbing for
/// the back third of the cycle after the lines have already connected, which
/// is the slow "pulse" (everything keeps getting brighter/denser-looking
/// past the point it should have settled).
const FADE_ALPHA_FULL: f32 = 0.62;
/// Width of the outer feather band as a fraction of the clip radius. Dots in
/// this band are progressively thinned/dimmed so the visible field dissolves
/// instead of ending at a strict circular cut-line.
const EDGE_FEATHER_FRAC: f32 = 0.34;
/// Dot-space stroke coverage for the Braille renderer. The core is fully lit;
/// the feather is partial alpha, which lets lines slide smoothly between dot
/// centers instead of snapping one integer dot at a time.
const DOT_STROKE_CORE_R: f32 = 0.38;
const DOT_STROKE_FEATHER_R: f32 = 0.76;

/// A high-resolution monochrome dot bitmap (row-major booleans), used by
/// the Braille renderer. A dot is `true` where the tiling has an edge;
/// `alpha` carries that dot's fade level (0..255) for the opacity
/// transition.
#[derive(Debug, Clone)]
pub struct DotBitmap {
    pub width: u16,
    pub height: u16,
    pub dots: Vec<bool>,
    pub alpha: Vec<u8>,
}

impl DotBitmap {
    /// Dot at `(x, y)`, or `false` if out of range.
    #[must_use]
    pub fn at(&self, x: u16, y: u16) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.dots[usize::from(y) * usize::from(self.width) + usize::from(x)]
    }

    /// Fade level (0.0..=1.0) of the dot at `(x, y)`; 0 if off/out of range.
    #[must_use]
    pub fn alpha_at(&self, x: u16, y: u16) -> f32 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        f32::from(self.alpha[usize::from(y) * usize::from(self.width) + usize::from(x)]) / 255.0
    }
}

/// A complex point `x + iy`.
#[derive(Clone, Copy)]
struct P {
    x: f32,
    y: f32,
}

#[inline]
fn p(x: f32, y: f32) -> P {
    P { x, y }
}

/// `a` toward `b` by fraction `t` (t=0 → a, t=1 → b).
#[inline]
fn lerp(a: P, b: P, t: f32) -> P {
    P {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    }
}

#[derive(Clone, Copy)]
struct Tri {
    kind: u8,
    a: P,
    b: P,
    c: P,
    /// Squared min-vertex radius — cached for cull + reveal frontier.
    r2: f32,
}

/// Canonical Robinson-triangle P3 subdivision (standard references).
///
/// - red  (0): `Pp = A + (B-A)/φ` → `(0, C,Pp,B)`, `(1, Pp,C,A)`
/// - blue (1): `Q = B + (A-B)/φ`, `R = B + (C-B)/φ`
///   → `(1, R,C,A)`, `(1, Q,R,B)`, `(0, R,Q,A)`
fn subdivide(tris: &[Tri]) -> Vec<Tri> {
    let mut out = Vec::with_capacity(tris.len() * 2);
    for t in tris {
        if t.kind == 0 {
            let pp = lerp(t.a, t.b, IPHI);
            out.push(mk(0, t.c, pp, t.b));
            out.push(mk(1, pp, t.c, t.a));
        } else {
            let q = lerp(t.b, t.a, IPHI);
            let r = lerp(t.b, t.c, IPHI);
            out.push(mk(1, r, t.c, t.a));
            out.push(mk(1, q, r, t.b));
            out.push(mk(0, r, q, t.a));
        }
    }
    out
}

#[inline]
fn mk(kind: u8, a: P, b: P, c: P) -> Tri {
    let ra = a.x * a.x + a.y * a.y;
    let rb = b.x * b.x + b.y * b.y;
    let rc = c.x * c.x + c.y * c.y;
    Tri {
        kind,
        a,
        b,
        c,
        r2: ra.min(rb).min(rc),
    }
}

/// Small rotation applied to the seed wheel so no rhomb-edge family is
/// axis-aligned. The 10-fold seed places vertices at odd multiples of
/// 18°, which includes exactly-vertical (90°/270°) directions — and a
/// vertical edge rasterizes into a dense single-column dot run while a
/// diagonal staircases, so verticals visually over-pop in the wireframe.
/// Rotating by half a wedge (9°) puts the five edge families as far from the
/// terminal grid axes as possible. Other rotations can move the artifact to a
/// different family, but 9° maximizes the worst-case distance from horizontal
/// or vertical.
const SEED_ROT: f32 = PI / 20.0;

/// Seed: 10 red (half-thin) triangles forming a "sun" wheel; adjacent
/// wedges mirror and glue into whole rhombi. Apex A at the origin.
fn seed() -> Vec<Tri> {
    let mut tris = Vec::with_capacity(10);
    for i in 0..10 {
        let mut b = p(
            ((2 * i - 1) as f32 * PI / 10.0 + SEED_ROT).cos(),
            ((2 * i - 1) as f32 * PI / 10.0 + SEED_ROT).sin(),
        );
        let mut c = p(
            ((2 * i + 1) as f32 * PI / 10.0 + SEED_ROT).cos(),
            ((2 * i + 1) as f32 * PI / 10.0 + SEED_ROT).sin(),
        );
        if i % 2 == 0 {
            std::mem::swap(&mut b, &mut c);
        }
        tris.push(mk(0, p(0.0, 0.0), b, c));
    }
    tris
}

fn cull(tris: Vec<Tri>, r: f32) -> Vec<Tri> {
    let r2 = r * r;
    tris.into_iter().filter(|t| t.r2 <= r2).collect()
}

fn scale_tiles(tris: &[Tri], s: f32) -> Vec<Tri> {
    let s2 = s * s;
    tris.iter()
        .map(|t| Tri {
            kind: t.kind,
            a: P {
                x: t.a.x * s,
                y: t.a.y * s,
            },
            b: P {
                x: t.b.x * s,
                y: t.b.y * s,
            },
            c: P {
                x: t.c.x * s,
                y: t.c.y * s,
            },
            r2: t.r2 * s2,
        })
        .collect()
}

/// Owns the three-layer tiling state and the cycle phase `tau`.
///
/// `dot_bitmap()` is the only public entry: feed it the elapsed wall-clock
/// delta (seconds) and the current sprite state; get back a [`DotBitmap`] of
/// edge dots for the Braille renderer.
pub struct PenroseAnimator {
    coarse: Vec<Tri>,
    fine: Vec<Tri>,
    next_fine: Vec<Tri>,
    /// Cycle phase in `[0, 1)`. One full unit zooms in by a factor of φ.
    tau: f32,
}

impl Default for PenroseAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl PenroseAnimator {
    pub fn new() -> Self {
        let coarse = {
            let mut tris = seed();
            for _ in 0..COARSE_GEN {
                tris = subdivide(&tris);
            }
            cull(tris, CULL_R)
        };
        let fine = cull(subdivide(&coarse), CULL_R);
        let next_fine = cull(subdivide(&fine), CULL_R);
        Self {
            coarse,
            fine,
            next_fine,
            tau: 0.0,
        }
    }

    /// Advance the cycle phase by `dt` seconds, promoting a generation
    /// at the wrap. Returns `true` if a wrap (promotion) occurred this
    /// call. Shared by both the pixel and glyph render paths.
    fn advance(&mut self, dt: f32, state: SpriteState) -> bool {
        // Time to zoom in by one factor of φ. Tuned per state so the
        // queue activity reads in the motion: working zooms faster.
        let step_secs = match state {
            SpriteState::Idle => 6.5,
            SpriteState::Working => 4.0,
            SpriteState::NeedsHelp => 9.0,
        };
        self.tau += (dt / step_secs).clamp(0.0, 0.5);
        let mut wrapped = false;
        if self.tau >= 1.0 {
            self.tau -= 1.0;
            self.promote_generation();
            wrapped = true;
        }
        wrapped
    }

    fn promote_generation(&mut self) {
        // Promote up one generation. In screen space, `tau = 1` before this
        // relabel matches `tau = 0` after it because rendering multiplies all
        // model coordinates by `PHI.powf(tau)`. The old `fine` is scaled by
        // PHI to become the new `coarse`; subdividing that scaled geometry
        // gives the old `next_fine` scaled by PHI as the new `fine`.
        self.coarse = cull(scale_tiles(&self.fine, PHI), CULL_R);
        self.fine = cull(subdivide(&self.coarse), CULL_R);
        self.next_fine = cull(subdivide(&self.fine), CULL_R);
    }

    /// Advance the animation by `dt` seconds and sample its rhombus
    /// *edges* into a high-resolution dot bitmap of `dot_w × dot_h`
    /// booleans (row-major). Used by the Braille renderer: a terminal of
    /// `cols × rows` cells maps to `cols*2 × rows*4` dots, so this gives
    /// 8× the resolution of one glyph per cell.
    ///
    /// The glyph path rasterizes edges directly in dot-space. Earlier we drew
    /// 1px high-res lines and block-OR downsampled them; that made apparent
    /// edge thickness/run length depend on angle, zoom, and sub-dot phase, so
    /// some diagonals became very long skinny staircases while others were
    /// short 2-dot steps. Dot-space Bresenham gives every edge the same
    /// terminal-grid stroke rule.
    pub fn dot_bitmap(&mut self, dt: f32, state: SpriteState, dot_w: u16, dot_h: u16) -> DotBitmap {
        self.advance(dt, state);
        self.render_edges_dot(dot_w, dot_h)
    }

    fn render_edges_dot(&mut self, dot_w: u16, dot_h: u16) -> DotBitmap {
        let dot_w = dot_w.max(1);
        let dot_h = dot_h.max(1);
        let len = usize::from(dot_w) * usize::from(dot_h);
        let mut settled = vec![0u8; len];
        let mut fade = vec![0u8; len];
        let mut coarse_mask = vec![0u8; len];

        let side = u32::from(dot_w.min(dot_h));
        let off_x = (u32::from(dot_w) - side) / 2;
        let off_y = (u32::from(dot_h) - side) / 2;
        let clip_r = side as f32 * CLIP_FRAC;
        let cx = off_x as f32 + side as f32 * 0.5;
        let cy = off_y as f32 + side as f32 * 0.5;
        let px_per_model = clip_r * PHI.powf(self.tau);
        let vis_model_r = clip_r / px_per_model;
        let vis_r2 = (vis_model_r * 1.05).powi(2);
        let xf = |p: P| -> (f32, f32) { (cx + p.x * px_per_model, cy + p.y * px_per_model) };

        let coarse = self.coarse.clone();
        let fine = self.fine.clone();
        let next_fine = self.next_fine.clone();

        let coarse_a = (coarse_fade_out(self.tau) * 255.0).round() as u8;
        if coarse_a > 0 {
            for t in &coarse {
                if t.r2 <= vis_r2 {
                    draw_dot_tri_edges(
                        &mut coarse_mask,
                        dot_w,
                        dot_h,
                        t,
                        coarse_a,
                        clip_r,
                        cx,
                        cy,
                        &xf,
                    );
                }
            }
        }

        for t in &fine {
            if t.r2 <= vis_r2 {
                draw_dot_tri_edges(&mut settled, dot_w, dot_h, t, 255, clip_r, cx, cy, &xf);
            }
        }

        let reveal = reveal_w(self.tau);
        if reveal >= REVEAL_GATE {
            let frac = endpoint_trim_frac(reveal);
            for t in &next_fine {
                if t.r2 > vis_r2 {
                    continue;
                }
                let pa = xf(t.a);
                let pb = xf(t.b);
                let pc = xf(t.c);
                let (c2, a2) = trim_frac(pc, pa, frac);
                draw_dot_mask_line(&mut fade, dot_w, dot_h, c2, a2, 255, clip_r, cx, cy);
                let (a3, b3) = trim_frac(pa, pb, frac);
                draw_dot_mask_line(&mut fade, dot_w, dot_h, a3, b3, 255, clip_r, cx, cy);
            }
        }

        self.coarse = coarse;
        self.fine = fine;
        self.next_fine = next_fine;

        let fade_byte = (fading_alpha(reveal) * 255.0).round() as u8;
        let feather = (clip_r * EDGE_FEATHER_FRAC).max(1.0);
        let mut dots = vec![false; len];
        let mut alpha = vec![0u8; len];
        for y in 0..usize::from(dot_h) {
            for x in 0..usize::from(dot_w) {
                let i = y * usize::from(dot_w) + x;
                let out_a = if settled[i] > 0 {
                    settled[i]
                } else if fade[i] > 0 {
                    ((u16::from(fade_byte) * u16::from(fade[i])) / 255) as u8
                } else {
                    coarse_mask[i]
                };
                if out_a == 0 {
                    continue;
                }
                let r = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let edge_t = ((clip_r - r) / feather).clamp(0.0, 1.0);
                if edge_t <= 0.0 {
                    continue;
                }
                let falloff = edge_t * edge_t * (3.0 - 2.0 * edge_t);
                let out_a = (f32::from(out_a) * falloff).round().max(1.0) as u8;
                dots[i] = true;
                alpha[i] = out_a;
            }
        }

        despeckle(dot_w, dot_h, &mut dots, &mut alpha);

        DotBitmap {
            width: dot_w,
            height: dot_h,
            dots,
            alpha,
        }
    }
}

// ---------------------------------------------------------------------
// Rasterization helpers — all clip to the disk of radius `clip_r`.
// ---------------------------------------------------------------------

#[inline]
fn in_disk(x: f32, y: f32, cx: f32, cy: f32, clip_r: f32) -> bool {
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= clip_r * clip_r
}

#[allow(clippy::too_many_arguments)]
fn draw_dot_tri_edges(
    mask: &mut [u8],
    width: u16,
    height: u16,
    t: &Tri,
    alpha: u8,
    clip_r: f32,
    cx: f32,
    cy: f32,
    xf: &impl Fn(P) -> (f32, f32),
) {
    let pa = xf(t.a);
    let pb = xf(t.b);
    let pc = xf(t.c);
    draw_dot_mask_line(mask, width, height, pc, pa, alpha, clip_r, cx, cy);
    draw_dot_mask_line(mask, width, height, pa, pb, alpha, clip_r, cx, cy);
}

#[allow(clippy::too_many_arguments)]
fn draw_dot_mask_line(
    mask: &mut [u8],
    width: u16,
    height: u16,
    start: (f32, f32),
    end: (f32, f32),
    alpha: u8,
    clip_r: f32,
    cx: f32,
    cy: f32,
) {
    if alpha == 0 {
        return;
    }

    // Continuous dot-space coverage: keep stroke width/runs consistent in the
    // Braille grid, but don't round the segment endpoints to integer dots.
    // Integer Bresenham made the whole animation quantize to horizontal/
    // vertical dot steps; distance coverage lets the line slide sub-dot and
    // changes brightness smoothly as it crosses dot centers.
    let min_x = (start.0.min(end.0) - DOT_STROKE_FEATHER_R).floor().max(0.0) as i32;
    let max_x = (start.0.max(end.0) + DOT_STROKE_FEATHER_R)
        .ceil()
        .min(f32::from(width.saturating_sub(1))) as i32;
    let min_y = (start.1.min(end.1) - DOT_STROKE_FEATHER_R).floor().max(0.0) as i32;
    let max_y = (start.1.max(end.1) + DOT_STROKE_FEATHER_R)
        .ceil()
        .min(f32::from(height.saturating_sub(1))) as i32;
    let width_usize = usize::from(width);
    let vx = end.0 - start.0;
    let vy = end.1 - start.1;
    let len = (vx * vx + vy * vy).sqrt();

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = (x as f32 + 0.5, y as f32 + 0.5);
            if !in_disk(point.0, point.1, cx, cy, clip_r) {
                continue;
            }
            let (distance, _) = point_segment_distance_and_along(point, start, end, len);
            if distance > DOT_STROKE_FEATHER_R {
                continue;
            }
            let coverage = if distance <= DOT_STROKE_CORE_R {
                1.0
            } else {
                let coverage_t = ((DOT_STROKE_FEATHER_R - distance)
                    / (DOT_STROKE_FEATHER_R - DOT_STROKE_CORE_R))
                    .clamp(0.0, 1.0);
                coverage_t * coverage_t * (3.0 - 2.0 * coverage_t)
            };
            let covered_alpha = (f32::from(alpha) * coverage).round() as u8;
            if covered_alpha > 0 {
                let idx = y as usize * width_usize + x as usize;
                mask[idx] = mask[idx].max(covered_alpha);
            }
        }
    }
}

fn point_segment_distance_and_along(
    p: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
    len: f32,
) -> (f32, f32) {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= f32::EPSILON {
        return (((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt(), 0.0);
    }
    let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0);
    let qx = a.0 + vx * t;
    let qy = a.1 + vy * t;
    (((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt(), t * len)
}

fn despeckle(dot_w: u16, dot_h: u16, dots: &mut [bool], alpha: &mut [u8]) {
    let dw = usize::from(dot_w);
    let dh = usize::from(dot_h);
    let snapshot = dots.to_vec();
    let has_neighbour = |x: usize, y: usize| -> bool {
        for (ox, oy) in [
            (-1i32, 0i32),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, 1),
            (-1, 1),
            (1, -1),
        ] {
            let nx = x as i32 + ox;
            let ny = y as i32 + oy;
            if nx < 0 || ny < 0 || nx >= i32::from(dot_w) || ny >= i32::from(dot_h) {
                continue;
            }
            if snapshot[(ny as usize) * dw + nx as usize] {
                return true;
            }
        }
        false
    };
    for y in 0..dh {
        for x in 0..dw {
            let i = y * dw + x;
            if snapshot[i] && !has_neighbour(x, y) {
                dots[i] = false;
                alpha[i] = 0;
            }
        }
    }
}

/// Shorten a segment `a→b` by `frac` of its length at EACH end (so the
/// kept middle is `1 - 2·frac` of the original), returning the trimmed
/// `(a', b')`. `frac` is clamped below 0.5 so the segment never inverts;
/// at `frac → 0.5` it collapses toward the midpoint. Trimming by a
/// fraction (rather than a fixed distance) keeps the "grow from the middle
/// outward" behavior consistent across edge lengths and zoom levels.
fn trim_frac(a: (f32, f32), b: (f32, f32), frac: f32) -> ((f32, f32), (f32, f32)) {
    let f = frac.clamp(0.0, 0.49);
    if f <= 0.0 {
        return (a, b);
    }
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    ((a.0 + dx * f, a.1 + dy * f), (b.0 - dx * f, b.1 - dy * f))
}

fn endpoint_trim_frac(w: f32) -> f32 {
    let w = w.clamp(0.0, 1.0);
    // `w⁴` keeps the trim near-max through the middle of the reveal, so the
    // shared convergence vertices (the edge endpoints) close LAST and never
    // starburst. The "edge starts as a flickering single dot" problem is
    // addressed separately by `END_GAP_FRAC` < ~0.45, which guarantees the
    // w=0 stub already spans several dots (a short line), not one — so there
    // is no single-dot phase even at the very start of the reveal.
    let base = END_GAP_FRAC * (1.0 - w.powi(4));
    if w <= END_GAP_CLOSE_START {
        return base;
    }
    if w >= END_GAP_CLOSE_DONE {
        return 0.0;
    }
    let t =
        ((w - END_GAP_CLOSE_START) / (END_GAP_CLOSE_DONE - END_GAP_CLOSE_START)).clamp(0.0, 1.0);
    let ease = t * t * (3.0 - 2.0 * t);
    base * (1.0 - ease)
}

/// Reveal weight `w ∈ [0,1]` from cycle phase `tau`. The reveal is
/// compressed into `tau ∈ [0, REVEAL_DONE]` (smoothstep), then HELD at 1 for
/// the rest of the cycle. So the next layer fades in, connects, and brightens
/// in the first part of the cycle and is then completely stable until the
/// wrap — no continued growth in already-settled areas.
fn reveal_w(tau: f32) -> f32 {
    let t = (tau.clamp(0.0, 1.0) / REVEAL_DONE).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fading_alpha(w: f32) -> f32 {
    let w = w.clamp(0.0, 1.0);
    ((w - REVEAL_GATE) / (FADE_ALPHA_FULL - REVEAL_GATE)).clamp(0.0, 1.0)
}

/// Fade-out alpha for the coarsest (leaving) layer over a cycle. Held full
/// until `COARSE_FADE_START`, then smoothly eased to 0 by the wrap so the
/// coarse-exclusive edges — the only geometry actually discarded at the
/// promotion — dissolve gracefully instead of being hard-culled. Continuous
/// across the wrap: this layer reaches 0 exactly as it is discarded, and the
/// newly-promoted coarse (last cycle's full `fine`) starts at 1.0 (tau=0).
fn coarse_fade_out(tau: f32) -> f32 {
    let tau = tau.clamp(0.0, 1.0);
    if tau <= COARSE_FADE_START {
        return 1.0;
    }
    let t = ((tau - COARSE_FADE_START) / (1.0 - COARSE_FADE_START)).clamp(0.0, 1.0);
    let ease = t * t * (3.0 - 2.0 * t);
    1.0 - ease
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_ten_triangles() {
        assert_eq!(seed().len(), 10);
    }

    #[test]
    fn subdivision_grows_tile_count() {
        let s = seed();
        let once = subdivide(&s);
        // Each red → 2, so 10 → 20.
        assert_eq!(once.len(), 20);
    }

    /// Total ink (sum of per-dot alpha) of one sampled frame.
    fn frame_ink(anim: &mut PenroseAnimator, dt: f32, dot_w: u16, dot_h: u16) -> u64 {
        let bmp = anim.dot_bitmap(dt, SpriteState::Working, dot_w, dot_h);
        bmp.alpha.iter().map(|&a| u64::from(a)).sum()
    }

    /// The wrap must not *add* ink: a generation promotion that overlays the
    /// fading-out old layer on top of the fading-in new layer would spike
    /// total ink for the duration of the cross-fade — that spike is the
    /// visible "pulse". We assert no frame during/after a wrap exceeds the
    /// pre-wrap steady ink by more than a small margin.
    #[test]
    fn wrap_does_not_spike_total_ink() {
        const WINDOW: usize = 16;
        let dot_w: u16 = 120;
        let dot_h: u16 = 120;
        let dt = 1.0 / 30.0;
        let mut anim = PenroseAnimator::new();

        // Warm up past the first wrap so layers are in steady rotation.
        for _ in 0..200 {
            let _ = anim.dot_bitmap(dt, SpriteState::Working, dot_w, dot_h);
        }

        // Record the ink series across one wrap and a fixed window after it.
        // The geometry zooms continuously, so ink naturally drifts; a wrap
        // pulse would be a *local* bump confined to a few frames around the
        // promotion, not the slow drift.
        let mut series: Vec<u64> = Vec::new();
        let mut wrap_idx: Option<usize> = None;
        for k in 0..240 {
            let before = anim.tau;
            let ink = frame_ink(&mut anim, dt, dot_w, dot_h);
            series.push(ink);
            if anim.tau < before && wrap_idx.is_none() {
                wrap_idx = Some(k);
            }
            if let Some(wi) = wrap_idx {
                if k >= wi + WINDOW {
                    break;
                }
            }
        }

        let wrap_idx = wrap_idx.expect("test never drove through a wrap");
        let fade_done = (wrap_idx + WINDOW).min(series.len() - 1);

        // Envelope = linear interpolation between the frame just before the
        // wrap and the frame when the fade finished. Any frame in the fade
        // window that rises substantially above this envelope is a pulse.
        let pre = series[wrap_idx.saturating_sub(1)];
        let post = series[fade_done];
        let span = fade_done - wrap_idx + 1;
        let mut worst_excess = 0i64;
        for (j, &ink) in series[wrap_idx..=fade_done].iter().enumerate() {
            let t = j as f64 / span as f64;
            let env = pre as f64 * (1.0 - t) + post as f64 * t;
            let excess = ink as i64 - env as i64;
            worst_excess = worst_excess.max(excess);
        }
        let envelope_ref = pre.max(post) as i64;
        eprintln!(
            "pre={pre} post={post} worst_excess={worst_excess} ({}%)",
            worst_excess * 100 / envelope_ref.max(1)
        );
        // The fade window may exceed the straight-line envelope by at most a
        // few percent of the surrounding ink. An overlay pulse adds ~20-100%.
        assert!(
            worst_excess <= envelope_ref / 12,
            "ink pulses across wrap: pre={pre} post={post} worst_excess={worst_excess}"
        );
    }

    /// The visible "pulse" is a cycle-wide density sawtooth: `next_fine`
    /// fades in over the cycle, adding ~φ² more edges, but the zoom only
    /// densifies by φ per cycle — so the frame grows steadily denser, peaks
    /// just before the wrap, then resets. Measure peak/trough ink over a full
    /// cycle; a small ratio means the field "breathes" little.
    #[test]
    fn density_stays_roughly_flat_over_a_cycle() {
        let dot_w: u16 = 120;
        let dot_h: u16 = 120;
        let dt = 1.0 / 30.0;
        let mut anim = PenroseAnimator::new();
        for _ in 0..300 {
            let _ = anim.dot_bitmap(dt, SpriteState::Working, dot_w, dot_h);
        }

        // Record ink across enough frames to cover at least one full cycle
        // (Working: ~4s per φ step → ~120 frames at 30fps).
        let mut min_ink = u64::MAX;
        let mut max_ink = 0u64;
        for _ in 0..130 {
            let bmp = anim.dot_bitmap(dt, SpriteState::Working, dot_w, dot_h);
            let ink: u64 = bmp.alpha.iter().map(|&a| u64::from(a)).sum();
            min_ink = min_ink.min(ink);
            max_ink = max_ink.max(ink);
        }
        let ratio = max_ink as f64 / min_ink.max(1) as f64;
        eprintln!("cycle ink min={min_ink} max={max_ink} ratio={ratio:.3}");
        // The ideal is ~1.0 (no breathing). The current design breathes ~1.45×
        // because `next_fine` adds ~φ² edges over a cycle while the zoom only
        // densifies by φ — that sawtooth IS the residual pulse. This guard
        // pins the *current* behavior so it can't regress further while the
        // density-flattening work continues; tighten the bound as it improves.
        assert!(
            ratio <= 1.5,
            "density breathes {ratio:.2}× over a cycle (pulse): min={min_ink} max={max_ink}"
        );
    }

    /// A pulse reads as a sudden frame-to-frame jump in on-screen ink. The
    /// animation zooms continuously, so consecutive frames should differ only
    /// gradually; a large single-frame delta (especially right at the wrap)
    /// is the visible pulse.
    #[test]
    fn no_large_frame_to_frame_ink_jump() {
        let dot_w: u16 = 120;
        let dot_h: u16 = 120;
        let dt = 1.0 / 30.0;
        let mut anim = PenroseAnimator::new();
        for _ in 0..200 {
            let _ = anim.dot_bitmap(dt, SpriteState::Working, dot_w, dot_h);
        }

        let mut prev = frame_ink(&mut anim, 0.0, dot_w, dot_h);
        let mut worst: i64 = 0;
        let mut worst_at_wrap = false;
        for _ in 0..400 {
            let before = anim.tau;
            let ink = frame_ink(&mut anim, dt, dot_w, dot_h);
            let delta = (ink as i64 - prev as i64).abs();
            if delta > worst {
                worst = delta;
                worst_at_wrap = anim.tau < before;
            }
            prev = ink;
        }

        // Typical per-frame drift from the zoom is small. Express the bound as
        // a fraction of steady-state ink (~5e5 for this grid).
        let steady = 500_000i64;
        eprintln!(
            "worst_frame_delta={worst} ({}% of steady, at_wrap={worst_at_wrap})",
            worst * 100 / steady
        );
        assert!(
            worst <= steady / 8,
            "large frame-to-frame ink jump: {worst} (at_wrap={worst_at_wrap})"
        );
    }

    #[test]
    fn endpoint_trim_closes_before_promotion() {
        // At w=0 the edge is heavily trimmed so shared convergence vertices
        // close last (no starburst), but it must leave a *visible* middle
        // stub (trim < 0.5) so an emerging edge reads as a short line, not a
        // single flickering dot.
        assert!(endpoint_trim_frac(0.0) > 0.30);
        assert!(endpoint_trim_frac(0.0) < 0.5);
        // The w=0 stub must keep a non-trivial middle (≥ ~20% of the edge)
        // so an emerging edge spans several connected dots and survives the
        // despeckle pass — otherwise it starts as a 1-dot speck that flickers
        // in and out before the edge forms.
        let kept_body_at_w0 = 1.0 - 2.0 * endpoint_trim_frac(0.0);
        assert!(
            kept_body_at_w0 >= 0.20,
            "w=0 stub keeps only {kept_body_at_w0:.3} of edge length (too short, will flicker)"
        );
        assert!(endpoint_trim_frac(END_GAP_CLOSE_START) > 0.0);
        assert!(endpoint_trim_frac(END_GAP_CLOSE_DONE).abs() < f32::EPSILON);
        assert!(endpoint_trim_frac(1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn brightness_completes_at_connect_then_holds_flat() {
        // Brightness must reach full when the next shape reads as CONNECTED
        // (edge bodies in) and then stop increasing — otherwise the field
        // keeps brightening for the rest of the cycle (the slow pulse). The
        // geometric endpoint tips keep closing later (END_GAP_CLOSE_DONE),
        // but at already-full brightness, so the cap is reached well before
        // promotion and held flat afterwards.
        const _: () = assert!(
            FADE_ALPHA_FULL < END_GAP_CLOSE_DONE,
            "brightness should complete before the geometry's final tip-close"
        );
        const _: () = assert!(
            REVEAL_GATE < FADE_ALPHA_FULL,
            "fade gate must precede the full-brightness point"
        );
        // Full and flat from the connect point all the way to the wrap.
        assert!((fading_alpha(FADE_ALPHA_FULL) - 1.0).abs() < f32::EPSILON);
        assert!((fading_alpha(END_GAP_CLOSE_DONE) - 1.0).abs() < f32::EPSILON);
        assert!((fading_alpha(1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fade_alpha_reaches_full_continuously() {
        assert!((fading_alpha(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((fading_alpha(REVEAL_GATE - 0.001) - 0.0).abs() < f32::EPSILON);
        assert!((fading_alpha(REVEAL_GATE) - 0.0).abs() < f32::EPSILON);
        let after_gate = fading_alpha(REVEAL_GATE + 0.001);
        assert!(
            after_gate > 0.0,
            "fade should start immediately after reveal gate"
        );
        assert!((fading_alpha(FADE_ALPHA_FULL) - 1.0).abs() < f32::EPSILON);
        assert!((fading_alpha(1.0) - 1.0).abs() < f32::EPSILON);
        let before = fading_alpha(FADE_ALPHA_FULL - 0.001);
        assert!(
            before > 0.99,
            "fade should approach 1.0 smoothly, got {before}"
        );
    }

    #[test]
    fn tau_wraps_and_stays_in_range() {
        let dot_w: u16 = 64;
        let dot_h: u16 = 64;
        let mut anim = PenroseAnimator::new();
        // Drive well past one full cycle.
        for _ in 0..200 {
            anim.dot_bitmap(0.08, SpriteState::Working, dot_w, dot_h);
            assert!(
                (0.0..1.0).contains(&anim.tau),
                "tau out of range: {}",
                anim.tau
            );
        }
    }
}
