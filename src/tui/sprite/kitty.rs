//! Kitty graphics protocol — pure wire-format functions.
//!
//! Every Kitty graphics command is an APC envelope:
//!
//! ```text
//! ESC _ G <key>=<value>,... ; <payload> ESC \
//! ```
//!
//! - `ESC _` opens an Application Program Command.
//! - `G` identifies the kitty-graphics dialect.
//! - Key/value pairs are single-letter keys (`a`, `i`, `f`, `s`, `v`,
//!   `m`, …) — see the protocol spec at
//!   <https://sw.kovidgoyal.net/kitty/graphics-protocol/>.
//! - `;<payload>` is the base64-encoded image data.
//! - `ESC \` (a String Terminator) ends the envelope.
//!
//! Large payloads must be chunked (`m=1` on every chunk except the
//! last, which uses `m=0`) because terminals, ssh, and tmux all have
//! finite line-buffer sizes. We chunk at 4 KiB of base64 (≈ 3 KiB raw)
//! which is the spec's recommended ceiling.
//!
//! `serialize_*` functions return `Vec<u8>` rather than writing
//! directly to stdout so that:
//!
//! 1. They're easy to unit-test (assert on substrings).
//! 2. The renderer can buffer a frame's worth of commands and flush
//!    once per tick, avoiding partial frames mid-redraw.
//! 3. tmux-passthrough wrapping is a separate concern applied at the
//!    very end (see [`wrap_for_tmux`]).

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// Base64-encoded chunk size. 4096 chars of base64 ≈ 3072 raw bytes,
/// well under any terminal's line-buffer ceiling.
pub const CHUNK_B64_BYTES: usize = 4096;

const ESC: u8 = 0x1b;
const APC_OPEN: &[u8] = b"\x1b_G";
const APC_CLOSE: &[u8] = b"\x1b\\";

/// Image format hint for the `f` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// `f=24` — raw RGB; size taken from `s`/`v`.
    Rgb,
    /// `f=32` — raw RGBA; size taken from `s`/`v`.
    Rgba,
    /// `f=100` — PNG; size read from the file header.
    Png,
}

impl ImageFormat {
    fn key(self) -> u32 {
        match self {
            Self::Rgb => 24,
            Self::Rgba => 32,
            Self::Png => 100,
        }
    }
}

/// Build the byte stream that uploads `data` under `image_id`. The
/// returned `Vec<u8>` may contain many APC envelopes if `data` was
/// large enough to require chunking.
///
/// For PNG payloads `width` and `height` may be `None` (the terminal
/// reads them from the PNG header). For raw formats they're required.
pub fn serialize_transmit(
    image_id: u32,
    format: ImageFormat,
    width: Option<u32>,
    height: Option<u32>,
    data: &[u8],
) -> Vec<u8> {
    let encoded = B64.encode(data);
    let mut out = Vec::with_capacity(encoded.len() + 256);

    let chunks: Vec<&str> = if encoded.len() <= CHUNK_B64_BYTES {
        vec![encoded.as_str()]
    } else {
        encoded
            .as_bytes()
            .chunks(CHUNK_B64_BYTES)
            // SAFETY: base64 output is ASCII, so chunk boundaries are
            // always valid UTF-8 boundaries.
            .map(|c| std::str::from_utf8(c).expect("base64 output is ASCII"))
            .collect()
    };
    let last_idx = chunks.len() - 1;

    for (i, chunk) in chunks.iter().enumerate() {
        out.extend_from_slice(APC_OPEN);
        // First chunk carries the full key set; subsequent chunks
        // carry only the chunk-control keys (`m`). This is what the
        // spec calls "control-data is sticky": the terminal remembers
        // the first chunk's keys for the whole logical command.
        if i == 0 {
            out.extend_from_slice(b"a=t,t=d,q=2"); // transmit, direct, quiet-2
            out.extend_from_slice(format!(",i={image_id}").as_bytes());
            out.extend_from_slice(format!(",f={}", format.key()).as_bytes());
            if let Some(w) = width {
                out.extend_from_slice(format!(",s={w}").as_bytes());
            }
            if let Some(h) = height {
                out.extend_from_slice(format!(",v={h}").as_bytes());
            }
        }
        // `m=1` on every chunk except the last.
        let m = u32::from(i != last_idx);
        if chunks.len() > 1 || i == last_idx {
            if i == 0 {
                out.push(b',');
            }
            out.extend_from_slice(format!("m={m}").as_bytes());
        }
        out.push(b';');
        out.extend_from_slice(chunk.as_bytes());
        out.extend_from_slice(APC_CLOSE);
    }
    out
}

/// Build the command that places (draws) an already-uploaded image at
/// the current cursor cell.
///
/// `placement_id` lets one image be drawn many times in distinct
/// stacking positions; use `0` if you don't need to address the
/// placement later.
///
/// `src_x`/`src_y`/`src_w`/`src_h` crop the source image (in pixels).
/// `cols`/`rows` constrain the on-screen footprint (in cells); the
/// terminal scales to fit. Pass `0` to disable a dimension.
///
/// `z` is the z-index; default text is at `z=0`.
#[allow(clippy::too_many_arguments)]
pub fn serialize_place(
    image_id: u32,
    placement_id: u32,
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
    cols: u32,
    rows: u32,
    z: i32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(APC_OPEN);
    out.extend_from_slice(b"a=p,q=2");
    out.extend_from_slice(format!(",i={image_id}").as_bytes());
    if placement_id != 0 {
        out.extend_from_slice(format!(",p={placement_id}").as_bytes());
    }
    if src_x != 0 {
        out.extend_from_slice(format!(",x={src_x}").as_bytes());
    }
    if src_y != 0 {
        out.extend_from_slice(format!(",y={src_y}").as_bytes());
    }
    if src_w != 0 {
        out.extend_from_slice(format!(",w={src_w}").as_bytes());
    }
    if src_h != 0 {
        out.extend_from_slice(format!(",h={src_h}").as_bytes());
    }
    if cols != 0 {
        out.extend_from_slice(format!(",c={cols}").as_bytes());
    }
    if rows != 0 {
        out.extend_from_slice(format!(",r={rows}").as_bytes());
    }
    if z != 0 {
        out.extend_from_slice(format!(",z={z}").as_bytes());
    }
    out.push(b';');
    out.extend_from_slice(APC_CLOSE);
    out
}

/// Delete a single image by id from the terminal's cache.
pub fn serialize_delete(image_id: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(APC_OPEN);
    out.extend_from_slice(b"a=d,d=I,q=2");
    out.extend_from_slice(format!(",i={image_id}").as_bytes());
    out.push(b';');
    out.extend_from_slice(APC_CLOSE);
    out
}

/// Delete all images we uploaded. Useful on shutdown.
pub fn serialize_delete_all() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(APC_OPEN);
    out.extend_from_slice(b"a=d,d=A,q=2;");
    out.extend_from_slice(APC_CLOSE);
    out
}

/// Build the capability-probe envelope. We upload a 1x1 transparent
/// RGBA pixel with `q=1` (which forces the terminal to ACK regardless
/// of success) and a known image id. A Kitty-protocol terminal
/// replies with another APC envelope echoing the id and a status.
/// Non-supporting terminals stay silent.
pub fn serialize_probe(image_id: u32) -> Vec<u8> {
    // A single transparent RGBA pixel: four zero bytes.
    let payload = B64.encode([0u8, 0, 0, 0]);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(APC_OPEN);
    out.extend_from_slice(b"a=q,t=d,f=32,s=1,v=1");
    out.extend_from_slice(format!(",i={image_id}").as_bytes());
    out.push(b';');
    out.extend_from_slice(payload.as_bytes());
    out.extend_from_slice(APC_CLOSE);
    out
}

/// Returns `true` iff `buf` contains a Kitty-graphics ACK envelope for
/// the given probe id. Tolerant of garbage before/after — we only
/// require the APC envelope to appear somewhere in the stream.
///
/// A successful ACK looks like:
///
/// ```text
/// ESC _ G i=<id>;OK ESC \
/// ```
///
/// Older Kitty versions sometimes emit `i=<id>,...;OK` with extra
/// keys; we accept any envelope that mentions our id and contains
/// `OK` in the body. An `ENOENT` or similar error reply also counts
/// as protocol-aware (the terminal speaks the dialect), so we treat
/// "any APC reply containing our id" as positive.
pub fn parse_probe_ack(buf: &[u8], expected_id: u32) -> bool {
    // Scan for APC openings; check each candidate envelope.
    let mut i = 0;
    let needle_id = format!("i={expected_id}");
    while i + APC_OPEN.len() <= buf.len() {
        if &buf[i..i + APC_OPEN.len()] != APC_OPEN {
            i += 1;
            continue;
        }
        // Find the closing ESC \ from here.
        let start = i + APC_OPEN.len();
        if let Some(end_rel) = find_subslice(&buf[start..], APC_CLOSE) {
            let body = &buf[start..start + end_rel];
            if let Ok(s) = std::str::from_utf8(body) {
                if s.contains(&needle_id) {
                    return true;
                }
            }
            i = start + end_rel + APC_CLOSE.len();
        } else {
            // Truncated envelope at end of buf — give up.
            return false;
        }
    }
    false
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Wrap a Kitty escape sequence for tmux passthrough.
///
/// When the program is running inside tmux, raw APC sequences are
/// eaten by tmux unless we wrap them in tmux's own passthrough
/// envelope:
///
/// ```text
/// ESC P tmux ; <doubled-ESC> ESC \
/// ```
///
/// tmux strips the outer envelope and forwards the inner content to
/// the host terminal. Inner `ESC` bytes must be doubled so tmux's
/// parser doesn't terminate the passthrough early.
///
/// Requires `set -g allow-passthrough on` in `tmux.conf`. Without
/// that, tmux 3.3+ swallows the sequence and the user sees nothing.
pub fn wrap_for_tmux(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + payload.len() / 8 + 8);
    out.extend_from_slice(b"\x1bPtmux;");
    for &b in payload {
        out.push(b);
        if b == ESC {
            // Double every ESC byte inside the passthrough envelope.
            out.push(ESC);
        }
    }
    out.extend_from_slice(APC_CLOSE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[u8]) -> String {
        String::from_utf8_lossy(v).into_owned()
    }

    #[test]
    fn transmit_png_small_is_one_envelope() {
        // A tiny PNG (8-byte signature only — invalid but fine for
        // the byte-shape test). Should not chunk.
        let data = b"\x89PNG\r\n\x1a\n";
        let bytes = serialize_transmit(7, ImageFormat::Png, None, None, data);
        let txt = s(&bytes);
        assert!(txt.starts_with("\x1b_G"));
        assert!(txt.ends_with("\x1b\\"));
        assert!(txt.contains("a=t"));
        assert!(txt.contains("i=7"));
        assert!(txt.contains("f=100"));
        assert!(txt.contains("m=0"));
        // No `s=` / `v=` because we passed None.
        assert!(!txt.contains(",s="));
        // Exactly one envelope (two ESC bytes: opener + closer).
        #[allow(clippy::naive_bytecount)]
        let esc_count = bytes.iter().filter(|&&b| b == 0x1b).count();
        assert_eq!(esc_count, 2);
    }

    #[test]
    fn transmit_rgba_emits_dimensions() {
        let data = vec![0xff; 64 * 64 * 4];
        let bytes = serialize_transmit(1, ImageFormat::Rgba, Some(64), Some(64), &data);
        let txt = s(&bytes);
        assert!(txt.contains("s=64"));
        assert!(txt.contains("v=64"));
        assert!(txt.contains("f=32"));
    }

    #[test]
    fn transmit_chunks_when_above_threshold() {
        // 16 KiB of raw → ~22 KiB base64 → 6 chunks at 4096-byte boundaries.
        let data = vec![0xab; 16 * 1024];
        let bytes = serialize_transmit(1, ImageFormat::Rgba, Some(64), Some(64), &data);
        let txt = s(&bytes);
        // First chunk gets `m=1`; last chunk `m=0`.
        let m1 = txt.matches("m=1").count();
        let m0 = txt.matches("m=0").count();
        assert!(m1 >= 1, "expected several m=1 chunks, got {m1}");
        assert_eq!(m0, 1, "expected exactly one m=0 (terminal chunk)");
        // The full keyset (a=t, f=32, …) only appears in the first envelope.
        assert_eq!(txt.matches("a=t").count(), 1);
        assert_eq!(txt.matches("f=32").count(), 1);
    }

    #[test]
    fn place_emits_required_keys() {
        let bytes = serialize_place(1, 0, 64, 0, 64, 64, 8, 4, 0);
        let txt = s(&bytes);
        assert!(txt.contains("a=p"));
        assert!(txt.contains("i=1"));
        // src offsets:
        assert!(txt.contains("x=64"));
        assert!(!txt.contains("y=0"));
        assert!(txt.contains("w=64"));
        assert!(txt.contains("h=64"));
        // Cell footprint:
        assert!(txt.contains("c=8"));
        assert!(txt.contains("r=4"));
        // z not present when default:
        assert!(!txt.contains(",z="));
    }

    #[test]
    fn place_with_z_index() {
        let bytes = serialize_place(1, 0, 0, 0, 0, 0, 0, 0, -1);
        let txt = s(&bytes);
        assert!(txt.contains("z=-1"));
    }

    #[test]
    fn delete_by_id_envelope() {
        let bytes = serialize_delete(42);
        let txt = s(&bytes);
        assert!(txt.starts_with("\x1b_G"));
        assert!(txt.contains("a=d"));
        assert!(txt.contains("d=I"));
        assert!(txt.contains("i=42"));
        assert!(txt.ends_with("\x1b\\"));
    }

    #[test]
    fn delete_all_envelope() {
        let bytes = serialize_delete_all();
        let txt = s(&bytes);
        assert!(txt.contains("d=A"));
        assert!(!txt.contains("i="));
    }

    #[test]
    fn probe_is_one_pixel_rgba() {
        let bytes = serialize_probe(31);
        let txt = s(&bytes);
        assert!(txt.contains("a=q"));
        assert!(txt.contains("f=32"));
        assert!(txt.contains("s=1"));
        assert!(txt.contains("v=1"));
        assert!(txt.contains("i=31"));
        // Base64 of 4 zero bytes ("\0\0\0\0") is "AAAAAA==" (4 bytes
        // → 32 bits → 6 base64 chars + padding to multiple of 4).
        assert!(
            txt.ends_with("AAAAAA==\x1b\\"),
            "unexpected probe payload tail: {txt:?}"
        );
    }

    #[test]
    fn probe_ack_recognized() {
        // A typical kitty ACK.
        let reply = b"\x1b_Gi=31;OK\x1b\\";
        assert!(parse_probe_ack(reply, 31));
    }

    #[test]
    fn probe_ack_with_extra_keys() {
        // Newer kitty versions include more keys.
        let reply = b"\x1b_Gi=31,I=0;OK\x1b\\";
        assert!(parse_probe_ack(reply, 31));
    }

    #[test]
    fn probe_ack_rejects_wrong_id() {
        let reply = b"\x1b_Gi=99;OK\x1b\\";
        assert!(!parse_probe_ack(reply, 31));
    }

    #[test]
    fn probe_ack_rejects_garbage() {
        assert!(!parse_probe_ack(b"", 31));
        assert!(!parse_probe_ack(b"random terminal noise", 31));
        // Unterminated APC — terminal cut us off.
        assert!(!parse_probe_ack(b"\x1b_Gi=31;OK", 31));
    }

    #[test]
    fn probe_ack_ignores_leading_garbage() {
        // Some terminals interleave cursor-position responses.
        let mut reply = b"\x1b[24;80R".to_vec(); // CPR response
        reply.extend_from_slice(b"\x1b_Gi=31;OK\x1b\\");
        assert!(parse_probe_ack(&reply, 31));
    }

    #[test]
    fn tmux_wrap_doubles_escapes() {
        let inner = b"\x1b_Gi=1;A\x1b\\";
        let wrapped = wrap_for_tmux(inner);
        // Outer envelope: ESC P tmux ; ... ESC \
        assert!(wrapped.starts_with(b"\x1bPtmux;"));
        assert!(wrapped.ends_with(b"\x1b\\"));
        // Inner ESC bytes are doubled.
        // Count ESC inside the body (between "tmux;" and the final close).
        let body = &wrapped[7..wrapped.len() - 2];
        #[allow(clippy::naive_bytecount)]
        let esc_count = body.iter().filter(|&&b| b == 0x1b).count();
        // The original had 2 ESCs; doubling yields 4.
        assert_eq!(esc_count, 4);
    }
}
