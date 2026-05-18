//! `mergesmith doctor` — sanity check.

use std::process::ExitCode;

use crate::cli::context::CliContext;
use crate::error::Result;
use crate::paths;

pub fn run() -> Result<ExitCode> {
    let mut all_ok = true;

    let state_root = paths::state_root()?;
    println!("[✓] state root: {}", state_root.display());

    // Open the store so we surface migration / schema errors early.
    match CliContext::open() {
        Ok(_) => println!("[✓] state.sqlite open + migrations applied"),
        Err(e) => {
            println!("[✗] state.sqlite open failed: {e}");
            all_ok = false;
        }
    }

    for tool in ["git", "tmux"] {
        if let Ok(p) = which::which(tool) {
            println!("[✓] {tool} found at {}", p.display());
        } else {
            println!("[⚠] {tool} not on PATH");
            if tool == "git" {
                all_ok = false;
            }
        }
    }

    if all_ok {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(2))
    }
}
