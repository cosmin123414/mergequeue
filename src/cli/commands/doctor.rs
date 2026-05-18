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

    // Kitty graphics: best-effort probe. We don't fail doctor on a
    // missing graphics terminal — `mergesmith doctor` should be usable
    // from non-Kitty shells. Just report.
    probe_kitty_for_doctor();

    // tmux passthrough warning: if we're inside tmux, remind the user
    // to enable allow-passthrough. We can't programmatically read tmux
    // config without shelling out; we just print the hint.
    if std::env::var_os("TMUX").is_some() {
        println!("[•] inside tmux: ensure `set -g allow-passthrough on` is in your tmux config");
    }

    if all_ok {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(2))
    }
}

fn probe_kitty_for_doctor() {
    // The probe needs raw mode + stdin reads. If we're not attached
    // to a TTY (e.g. piping `mergesmith doctor` into a file), skip.
    if !is_tty() {
        println!("[•] kitty graphics probe skipped (stdin not a TTY)");
        return;
    }
    match crate::tui::capability::probe_stdin(crate::tui::capability::PROBE_TIMEOUT) {
        Ok(crate::tui::capability::KittySupport::Supported) => {
            println!("[✓] terminal speaks the Kitty graphics protocol");
        }
        Ok(crate::tui::capability::KittySupport::Unsupported) => {
            println!("[⚠] terminal does NOT speak the Kitty graphics protocol; the TUI will refuse to open");
        }
        Err(e) => {
            println!("[•] kitty graphics probe failed: {e}");
        }
    }
}

fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}
