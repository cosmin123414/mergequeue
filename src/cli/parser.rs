//! Clap definitions for the CLI surface.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "mergequeue",
    version,
    about = "Local merge queue with an animated Penrose-tiling visualizer",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the animated TUI (also the default when no subcommand
    /// is given).
    Tui(TuiArgs),
    /// Register the current repo with MergeQueue.
    Init(InitArgs),
    /// Queue the current worktree for merge.
    Enqueue(EnqueueArgs),
    /// Print the live queue.
    Status(StatusArgs),
    /// Cancel a queued or in-flight entry.
    Cancel(IdArg),
    /// Re-enqueue a Failed or NeedsHelp entry.
    Retry(IdArg),
    /// Attach to (or open) the agent session for a NeedsHelp entry.
    Resolve(IdArg),
    /// Print CI + merge logs for an entry.
    Logs(IdArg),
    /// Manage registered repos.
    Repos(ReposArgs),
    /// Environment sanity check.
    Doctor,
}

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Default branch to merge into. Defaults to whatever HEAD points at.
    #[arg(long)]
    pub default_branch: Option<String>,

    /// Optional lint command (e.g. `cargo clippy -- -D warnings`).
    #[arg(long)]
    pub lint: Option<String>,

    /// Optional test command.
    #[arg(long)]
    pub test: Option<String>,

    /// Optional build command.
    #[arg(long)]
    pub build: Option<String>,
}

#[derive(Debug, Default, Parser)]
pub struct TuiArgs {
    /// Run the TUI with in-memory showcase data and no workers.
    #[arg(long)]
    pub demo: bool,
}

#[derive(Debug, Parser)]
pub struct EnqueueArgs {
    /// Override the target branch (defaults to the repo's default).
    #[arg(long)]
    pub target: Option<String>,

    /// Optional human-readable message attached to the queue entry.
    #[arg(long)]
    pub message: Option<String>,
}

#[derive(Debug, Parser)]
pub struct StatusArgs {
    /// Filter to one registered repo (root path).
    #[arg(long)]
    pub repo: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Parser)]
pub struct IdArg {
    /// Queue entry ID.
    pub id: String,
}

#[derive(Debug, Parser)]
pub struct ReposArgs {
    #[command(subcommand)]
    pub command: ReposCommand,
}

#[derive(Debug, Subcommand)]
pub enum ReposCommand {
    /// List registered repos.
    List,
    /// Deregister a repo (cascades to its queue entries).
    Remove(IdArg),
}

pub fn parse() -> Cli {
    Cli::parse()
}
