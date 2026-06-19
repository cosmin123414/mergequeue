//! Subcommand dispatch.

mod cancel;
mod doctor;
mod enqueue;
mod init;
mod logs;
mod repos;
mod resolve;
mod retry;
mod status;
mod tui;

use std::process::ExitCode;

use crate::cli::parser::{Cli, Command, TuiArgs};
use crate::error::Result;

pub fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        // Bare `mergequeue` aliases `mergequeue tui` (per `docs/07-cli.md`).
        None => tui::run(TuiArgs::default()),
        Some(Command::Tui(a)) => tui::run(a),
        Some(Command::Init(a)) => init::run(a),
        Some(Command::Enqueue(a)) => enqueue::run(a),
        Some(Command::Status(a)) => status::run(a),
        Some(Command::Cancel(a)) => cancel::run(a),
        Some(Command::Retry(a)) => retry::run(a),
        Some(Command::Resolve(a)) => resolve::run(a),
        Some(Command::Logs(a)) => logs::run(a),
        Some(Command::Repos(a)) => repos::run(a),
        Some(Command::Doctor) => doctor::run(),
    }
}
