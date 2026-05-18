//! `mergesmith repos {list|remove}`.

use std::process::ExitCode;
use std::str::FromStr;

use crate::cli::context::CliContext;
use crate::cli::parser::{IdArg, ReposArgs, ReposCommand};
use crate::core::ids::RepoId;
use crate::error::Result;

pub fn run(args: ReposArgs) -> Result<ExitCode> {
    match args.command {
        ReposCommand::List => list(),
        ReposCommand::Remove(a) => remove(a),
    }
}

fn list() -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let repos = ctx.store.list_repos()?;
    if repos.is_empty() {
        println!("(no repos registered — run `mergesmith init` inside a repo)");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "{:<8} {:<32} {:<16} {:<10}",
        "id", "root_path", "default_branch", "agent"
    );
    for r in repos {
        println!(
            "{:<8} {:<32} {:<16} {:<10}",
            r.id.short(),
            r.root_path.display(),
            r.default_branch,
            r.agent_backend.as_str(),
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn remove(args: IdArg) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let id = resolve_repo_id(&*ctx.store, &args.id)?;
    ctx.store.delete_repo(id)?;
    println!("removed repo {}", id.short());
    Ok(ExitCode::SUCCESS)
}

fn resolve_repo_id(store: &dyn crate::core::ports::QueueStore, s: &str) -> Result<RepoId> {
    if let Ok(id) = RepoId::from_str(s) {
        return Ok(id);
    }
    // Match by short prefix or by root_path.
    let repos = store.list_repos()?;
    let candidates: Vec<&crate::core::repo::RegisteredRepo> = repos
        .iter()
        .filter(|r| {
            r.id.to_string().replace('-', "").starts_with(s) || r.root_path.to_string_lossy() == s
        })
        .collect();
    match candidates.len() {
        0 => Err(crate::error::Error::NotFound(format!("repo {s}"))),
        1 => Ok(candidates[0].id),
        _ => Err(crate::error::Error::invalid(format!(
            "ambiguous repo identifier `{s}` ({} matches)",
            candidates.len()
        ))),
    }
}
