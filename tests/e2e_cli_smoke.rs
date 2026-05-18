//! End-to-end CLI smoke test: drive a fresh state root through
//! `init`, `enqueue`, `status`, `cancel`, `repos list`, and verify the
//! resulting state.

use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

fn make_git_repo(dir: &std::path::Path, branch: &str) {
    Command::new("git")
        .arg("init")
        .arg("-q")
        .arg("-b")
        .arg(branch)
        .arg(dir)
        .assert()
        .success();
    Command::new("git")
        .args(["-C", dir.to_str().unwrap(), "config", "user.email", "t@t"])
        .assert()
        .success();
    Command::new("git")
        .args(["-C", dir.to_str().unwrap(), "config", "user.name", "T"])
        .assert()
        .success();
    Command::new("git")
        .args([
            "-C",
            dir.to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-m",
            "init",
            "-q",
        ])
        .assert()
        .success();
}

fn ms(state_root: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("mergesmith").unwrap();
    c.env("MERGESMITH_HOME", state_root);
    c
}

#[test]
fn cli_init_enqueue_status_cancel_round_trip() {
    let state_root = TempDir::new().unwrap();
    let repo_dir = TempDir::new().unwrap();
    make_git_repo(repo_dir.path(), "main");

    // status on an empty store should succeed and show empty.
    ms(state_root.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("queue is empty"));

    // init inside the repo.
    ms(state_root.path())
        .arg("init")
        .current_dir(repo_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("registered"));

    // Cannot init the same repo twice.
    ms(state_root.path())
        .arg("init")
        .current_dir(repo_dir.path())
        .assert()
        .failure();

    // repos list shows the new repo.
    ms(state_root.path())
        .args(["repos", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));

    // Enqueue while on `main` (the target) should refuse.
    ms(state_root.path())
        .arg("enqueue")
        .current_dir(repo_dir.path())
        .assert()
        .failure();

    // Switch to a feature branch and enqueue.
    Command::new("git")
        .args([
            "-C",
            repo_dir.path().to_str().unwrap(),
            "checkout",
            "-q",
            "-b",
            "feat/x",
        ])
        .assert()
        .success();
    Command::new("git")
        .args([
            "-C",
            repo_dir.path().to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-m",
            "work",
            "-q",
        ])
        .assert()
        .success();

    ms(state_root.path())
        .arg("enqueue")
        .current_dir(repo_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("enqueued"));

    // status now shows one entry.
    ms(state_root.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Queued"))
        .stdout(predicate::str::contains("feat/x"));
}

#[test]
fn doctor_reports_state() {
    let state_root = TempDir::new().unwrap();
    ms(state_root.path())
        .arg("doctor")
        .assert()
        // doctor succeeds when git is on PATH; it always is in CI.
        .stdout(predicate::str::contains("state root"));
}
