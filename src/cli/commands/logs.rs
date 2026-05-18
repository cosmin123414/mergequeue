//! `mergesmith logs <id>` — print CI + merge logs for an entry.

use std::process::ExitCode;

use crate::cli::context::CliContext;
use crate::cli::parser::IdArg;
use crate::error::Result;
use crate::paths;

pub fn run(args: IdArg) -> Result<ExitCode> {
    let ctx = CliContext::open()?;
    let id = super::cancel::resolve_id(&*ctx.store, &args.id)?;
    let Some(entry) = ctx.store.get_entry(id)? else {
        eprintln!("no such entry: {}", args.id);
        return Ok(ExitCode::from(3));
    };
    let dir = entry
        .ci_log_dir
        .clone()
        .unwrap_or_else(|| paths::runs_dir(&ctx.state_root).join(entry.id.to_string()));
    if !dir.exists() {
        println!("(no log files yet for {})", entry.id.short());
        return Ok(ExitCode::SUCCESS);
    }
    for step in ["ci-lint", "ci-test", "ci-build"] {
        let path = dir.join(format!("{step}.log"));
        if path.exists() {
            println!("===== {} =====", path.display());
            match std::fs::read_to_string(&path) {
                Ok(s) => print!("{s}"),
                Err(e) => eprintln!("(cannot read {}: {e})", path.display()),
            }
            println!();
        }
    }
    Ok(ExitCode::SUCCESS)
}
