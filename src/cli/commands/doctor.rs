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

    // git is required.
    if let Ok(p) = which::which("git") {
        println!("[✓] git found at {}", p.display());
    } else {
        println!("[✗] git not on PATH (required)");
        all_ok = false;
    }
    // tmux is required from M2 onward — without it `NeedsHelp` handoff
    // can't open an agent window.
    if let Ok(p) = which::which("tmux") {
        println!("[✓] tmux found at {}", p.display());
    } else {
        println!("[✗] tmux not on PATH (required for agent handoff)");
        all_ok = false;
    }

    // Agent backends. `opencode` is the M2 default; the others are
    // optional and only emit a warning.
    let agent_tools: &[(&str, bool)] = &[
        ("opencode", false),
        ("claude", true),
        ("cursor-agent", true),
        ("codex", true),
    ];
    let mut any_agent = false;
    for (tool, optional) in agent_tools {
        if let Ok(p) = which::which(tool) {
            println!("[✓] {tool} found at {}", p.display());
            any_agent = true;
        } else if *optional {
            println!("[•] {tool} not on PATH (optional)");
        } else {
            println!("[⚠] {tool} not on PATH (default agent backend)");
        }
    }
    if !any_agent {
        println!("[⚠] no agent CLIs detected; NeedsHelp entries will be unattended");
    }

    if all_ok {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(2))
    }
}
