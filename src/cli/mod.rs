//! CLI entry point and subcommand router.

mod commands;
mod context;
mod parser;

use std::process::ExitCode;

pub use context::CliContext;

/// Library entry point used by `main.rs`. Returns a process exit code.
pub fn run() -> ExitCode {
    init_logging();
    let cli = parser::parse();
    match commands::dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(10)
        }
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("MERGESMITH_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
