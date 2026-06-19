//! M4 e2e: confirm the TUI exits cleanly (with the right code) on
//! environments that can't host it.
//!
//! These are the only "TUI tests" that work in CI / under a subagent:
//! actually driving the render loop requires a real TTY and a
//! Kitty-aware terminal emulator. The unit tests in
//! `tui::sprite::tests` cover the byte-level protocol; here we just
//! pin down the gates.

use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

fn ms(state_root: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("mergequeue").unwrap();
    c.env("MERGEQUEUE_HOME", state_root);
    c
}

#[test]
fn tui_subcommand_refuses_without_a_tty() {
    let state_root = TempDir::new().unwrap();
    // assert_cmd inherits a pipe for stdin/stdout in tests, so this
    // exercises the NotATty path. Exit code 2 per docs/07-cli.md.
    ms(state_root.path())
        .arg("tui")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not a TTY"));
}

#[test]
fn bare_mergequeue_also_refuses_without_a_tty() {
    // Bare `mergequeue` aliases `mergequeue tui` (per docs/07-cli.md).
    let state_root = TempDir::new().unwrap();
    ms(state_root.path()).assert().code(2);
}
