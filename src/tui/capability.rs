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
///
/// Reads raw bytes from stdin's fd via `poll(2)` rather than going
/// through `crossterm::event::poll`. The crossterm event loop parses
/// stdin into its own `Event` taxonomy (keys, mouse, resize, …) and
/// **silently discards** anything it doesn't recognize. A Kitty
/// graphics ACK is an APC envelope crossterm has no opinion on, so
/// going through `event::poll` loses the reply and we always time
/// out. The `libc::poll` syscall lets us wait on the raw fd and then
/// read the bytes ourselves before any cooked-event parsing happens.
///
/// Caller is responsible for not invoking this twice in a row from
/// the same process.
pub fn probe_stdin(timeout: Duration) -> Result<KittySupport> {
    let _guard = RawModeGuard::enable()?;
    let tmux = std::env::var_os("TMUX").is_some();

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

    read_until_ack_or_deadline(timeout)
}

/// Unix path: wait on fd 0 with `poll(2)` (via the safe `nix`
/// wrapper), then `read(2)` whatever's pending. Loops until we see
/// an ACK or the deadline passes.
///
/// `Result` is kept for symmetry with `probe_stdin`'s signature and
/// the future possibility of returning `Err` (e.g. if we add a
/// "probe failed unexpectedly" branch distinct from "terminal said
/// no").
#[allow(clippy::unnecessary_wraps)]
#[cfg(unix)]
fn read_until_ack_or_deadline(timeout: Duration) -> Result<KittySupport> {
    use std::io::{stdin, Read};
    use std::os::fd::{AsFd, BorrowedFd};

    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

    let deadline = Instant::now() + timeout;
    let stdin_handle = stdin();
    let borrowed: BorrowedFd<'_> = stdin_handle.as_fd();
    let mut buf = Vec::with_capacity(128);

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let timeout_ms =
            u16::try_from(remaining.as_millis().min(u128::from(u16::MAX))).unwrap_or(u16::MAX);
        let mut fds = [PollFd::new(borrowed, PollFlags::POLLIN)];
        match poll(&mut fds, PollTimeout::from(timeout_ms)) {
            Ok(0) => break, // budget elapsed
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Ok(KittySupport::Unsupported),
        }
        let revents = fds[0].revents().unwrap_or(PollFlags::empty());
        if !revents.contains(PollFlags::POLLIN) {
            // POLLHUP / POLLERR / POLLNVAL — give up.
            return Ok(KittySupport::Unsupported);
        }
        let mut chunk = [0u8; 256];
        match stdin().read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if kitty::parse_probe_ack(&buf, PROBE_IMAGE_ID) {
                    return Ok(KittySupport::Supported);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    Ok(KittySupport::Unsupported)
}

/// Windows fallback: there's no `poll(2)` on the stdin handle in a
/// portable way without going through the Console API. For M4 we
/// take the simple path — read in a thread with a deadline — which
/// is good enough since Windows Terminal doesn't speak the Kitty
/// protocol anyway and the probe will time out either way.
#[allow(clippy::unnecessary_wraps)]
#[cfg(not(unix))]
fn read_until_ack_or_deadline(timeout: Duration) -> Result<KittySupport> {
    use std::io::{stdin, Read};
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 256];
        if let Ok(n) = stdin().read(&mut buf) {
            let _ = tx.send(buf[..n].to_vec());
        }
    });
    let mut acc = Vec::new();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(bytes) => {
                acc.extend_from_slice(&bytes);
                if kitty::parse_probe_ack(&acc, PROBE_IMAGE_ID) {
                    return Ok(KittySupport::Supported);
                }
            }
            Err(_) => break,
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
