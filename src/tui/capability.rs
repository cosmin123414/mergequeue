//! Kitty graphics protocol capability probe.
//!
//! We refuse to open the TUI on terminals that don't speak the
//! protocol — there's no graceful text-art fallback in v1 (per
//! `docs/08-tui-and-sprite.md`), and a half-working UI is worse than
//! a "go install Ghostty/Kitty/WezTerm" exit.
//!
//! The probe:
//!
//! 1. Switch the terminal into raw mode so we can read individual
//!    bytes back without line-buffering.
//! 2. Emit a 1×1 transparent PNG transmit with `q=1` (force ACK) and
//!    a known image id.
//! 3. Read stdin for up to `PROBE_TIMEOUT` looking for an APC reply
//!    that mentions our id.
//! 4. **Always** restore the terminal state via the
//!    [`RawModeGuard`]. RAII gives us the cleanup-on-panic property
//!    we need.
//!
//! tmux-passthrough is honored: if `$TMUX` is set we wrap the probe
//! envelope before writing. If passthrough isn't enabled in the user's
//! tmux config we'll time out and report unsupported — the doctor
//! command surfaces the hint.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::tui::sprite::kitty;

/// Total wall-clock time we wait for the terminal's reply. A
/// well-behaved terminal answers within a few ms; we keep the budget
/// at 300 ms to be friendly to slow ssh hops without making the
/// "unsupported terminal" path feel sluggish.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// Image id used by the probe. Distinct from `SPRITE_IMAGE_ID` so the
/// terminal doesn't accidentally try to reuse a probe slot for the
/// real sprite.
const PROBE_IMAGE_ID: u32 = 31;

/// Result of the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittySupport {
    Supported,
    Unsupported,
}

/// Sentinel guard: enables raw mode on construction, disables it on
/// drop (including on panic). Wraps `crossterm::terminal::enable_raw_mode`.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enable() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()
            .map_err(|e| Error::other(format!("enable raw mode: {e}")))?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Run the probe against an arbitrary I/O pair. Production callers
/// use [`probe_terminal`] which wires this up to stdin/stdout; tests
/// can pass in any `Read`/`Write` pair.
pub fn probe_io<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    tmux_passthrough: bool,
    timeout: Duration,
) -> Result<KittySupport> {
    let envelope = kitty::serialize_probe(PROBE_IMAGE_ID);
    let envelope = if tmux_passthrough {
        kitty::wrap_for_tmux(&envelope)
    } else {
        envelope
    };
    output
        .write_all(&envelope)
        .map_err(|e| Error::other(format!("write probe: {e}")))?;
    output
        .flush()
        .map_err(|e| Error::other(format!("flush probe: {e}")))?;

    let deadline = Instant::now() + timeout;
    let mut buf = Vec::with_capacity(128);
    let mut chunk = [0u8; 64];
    while Instant::now() < deadline {
        // Non-blocking read with a tiny sleep between attempts.
        // crossterm provides a `poll` for events but we want raw bytes
        // here. We can't easily set non-blocking on stdin in stable
        // Rust without unsafe; instead, the caller wires `input` to a
        // stream that polls/timeouts on its own — see `probe_stdin`.
        match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if kitty::parse_probe_ack(&buf, PROBE_IMAGE_ID) {
                    return Ok(KittySupport::Supported);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    Ok(KittySupport::Unsupported)
}

/// Probe stdin/stdout with appropriate terminal-mode handling.
/// Caller is responsible for not invoking this twice in a row from
/// the same process (the second call's raw-mode toggle is harmless
/// but wasteful).
pub fn probe_stdin(timeout: Duration) -> Result<KittySupport> {
    let _guard = RawModeGuard::enable()?;
    let tmux = std::env::var_os("TMUX").is_some();

    // crossterm's `event::poll` is the right way to wait on stdin
    // without going non-blocking. We poll for input, then drain it
    // synchronously when ready. Within a budget, we accumulate bytes
    // and check for a match.
    let envelope = kitty::serialize_probe(PROBE_IMAGE_ID);
    let envelope = if tmux {
        kitty::wrap_for_tmux(&envelope)
    } else {
        envelope
    };
    {
        let mut stdout = std::io::stdout();
        stdout
            .write_all(&envelope)
            .map_err(|e| Error::other(format!("write probe: {e}")))?;
        stdout
            .flush()
            .map_err(|e| Error::other(format!("flush probe: {e}")))?;
    }

    let deadline = Instant::now() + timeout;
    let mut buf = Vec::with_capacity(128);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        // Poll for *any* event up to the remaining budget. We don't
        // use crossterm's event abstraction (which would parse the
        // bytes into key events) because we want raw APC bytes.
        // Instead we let crossterm just signal availability and then
        // read from stdin directly.
        match crossterm::event::poll(remaining) {
            Ok(true) => {
                // Drain whatever events crossterm parsed — they're
                // junk for our purposes, but they correspond to bytes
                // already consumed from stdin. To avoid losing the
                // APC reply we read from stdin ourselves before
                // crossterm can swallow it. In practice crossterm's
                // poll returns true precisely because raw bytes are
                // available; we read them ourselves.
                use std::io::{stdin, Read};
                let mut chunk = [0u8; 128];
                match stdin().read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if kitty::parse_probe_ack(&buf, PROBE_IMAGE_ID) {
                            return Ok(KittySupport::Supported);
                        }
                    }
                }
            }
            Ok(false) | Err(_) => return Ok(KittySupport::Unsupported),
        }
    }
    Ok(KittySupport::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn ack_bytes(id: u32) -> Vec<u8> {
        format!("\x1b_Gi={id};OK\x1b\\").into_bytes()
    }

    #[test]
    fn probe_supported_when_terminal_acks() {
        let input = Cursor::new(ack_bytes(PROBE_IMAGE_ID));
        let mut output = Vec::new();
        let r = probe_io(input, &mut output, false, PROBE_TIMEOUT).unwrap();
        assert_eq!(r, KittySupport::Supported);
        // The probe envelope was written.
        let s = String::from_utf8_lossy(&output);
        assert!(s.contains("a=q"));
        assert!(s.contains(&format!("i={PROBE_IMAGE_ID}")));
    }

    #[test]
    fn probe_unsupported_on_silence() {
        let input: &[u8] = b"";
        let mut output = Vec::new();
        let r = probe_io(input, &mut output, false, Duration::from_millis(20)).unwrap();
        assert_eq!(r, KittySupport::Unsupported);
    }

    #[test]
    fn probe_unsupported_on_unrelated_ack() {
        let input = Cursor::new(ack_bytes(99));
        let mut output = Vec::new();
        let r = probe_io(input, &mut output, false, Duration::from_millis(50)).unwrap();
        assert_eq!(r, KittySupport::Unsupported);
    }

    #[test]
    fn probe_wraps_for_tmux() {
        let input = Cursor::new(ack_bytes(PROBE_IMAGE_ID));
        let mut output = Vec::new();
        let _ = probe_io(input, &mut output, true, PROBE_TIMEOUT).unwrap();
        let s = String::from_utf8_lossy(&output);
        // Outer envelope present.
        assert!(s.starts_with("\x1bPtmux;"));
    }
}
