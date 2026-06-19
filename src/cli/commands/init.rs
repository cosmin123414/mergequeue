//! `mergequeue init` — register the current repo.

use std::process::ExitCode;
use std::time::Duration;

use crate::cli::context::CliContext;
use crate::cli::parser::InitArgs;
use crate::core::agent_backend::AgentBackend;
use crate::core::ids::RepoId;
use crate::core::repo::{RegisteredRepo, RepoCiConfig};
use crate::error::Result;

pub fn run(args: InitArgs) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let cwd = std::env::current_dir()?;
    let root = ctx.git.discover_worktree_root(&cwd)?;

    // Refuse to register the same root twice.
    if let Some(existing) = ctx.store.get_repo_by_root(&root)? {
        eprintln!(
            "repo already registered: {} ({})",
            existing.root_path.display(),
            existing.id.short()
        );
        return Ok(ExitCode::from(1));
    }

    let default_branch = match args.default_branch {
        Some(b) => b,
        None => ctx.git.current_branch(&root)?,
    };

    let repo = RegisteredRepo {
        id: RepoId::new(),
        root_path: root.clone(),
        default_branch: default_branch.clone(),
        ci: RepoCiConfig {
            lint_command: args.lint,
            test_command: args.test,
            build_command: args.build,
            dirty_retry: Duration::from_secs(30),
        },
        agent_backend: AgentBackend::Opencode,
        created_at: ctx.clock.now(),
        updated_at: ctx.clock.now(),
    };
    ctx.store.insert_repo(&repo)?;

    println!(
        "registered {} (default branch: {}, id: {})",
        root.display(),
        default_branch,
        repo.id.short()
    );
    Ok(ExitCode::SUCCESS)
}
