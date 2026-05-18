//! `mergesmith tui` — open the animated TUI.
//!
//! Also the default action when `mergesmith` is invoked with no
//! subcommand. The actual logic lives in `crate::tui::run`; this is
//! the thin CLI shim that maps the `RunError` variants to documented
//! exit codes.

use std::process::ExitCode;

use crate::error::Result;
use crate::tui;

// `Result<ExitCode>` is the consistent return shape across every
// other CLI subcommand; keep it here even though this fn can't
// actually produce a `Result::Err` in practice.
#[allow(clippy::unnecessary_wraps)]
pub fn run() -> Result<ExitCode> {
    match tui::run() {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("{e}");
            Ok(ExitCode::from(e.exit_code()))
        }
    }
}
