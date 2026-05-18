//! Subcommand dispatch.

mod cancel;
mod doctor;
mod enqueue;
mod init;
mod logs;
mod repos;
mod retry;
mod status;

use std::process::ExitCode;

use crate::cli::parser::{Cli, Command};
use crate::error::Result;

pub fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        None => {
            // Bare `mergesmith` would launch the TUI; in M1 we print
            // help with a friendly note that the TUI lands in M4.
            eprintln!("mergesmith: TUI not implemented yet (planned for M4).");
            eprintln!("Run `mergesmith --help` for available subcommands.");
            Ok(ExitCode::from(2))
        }
        Some(Command::Init(a)) => init::run(a),
        Some(Command::Enqueue(a)) => enqueue::run(a),
        Some(Command::Status(a)) => status::run(a),
        Some(Command::Cancel(a)) => cancel::run(a),
        Some(Command::Retry(a)) => retry::run(a),
        Some(Command::Logs(a)) => logs::run(a),
        Some(Command::Repos(a)) => repos::run(a),
        Some(Command::Doctor) => doctor::run(),
    }
}
