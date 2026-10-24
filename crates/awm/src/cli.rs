//! The command line (design.md, "Commands").

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Moves or renames a workspace folder, and carries everything keyed on its path with
/// it: agent sessions, memory and settings, version control's links, and editor state.
#[derive(Debug, Parser)]
#[command(name = "agent-workspace-mover", version)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Move or rename a workspace folder, after checking everything and showing the plan.
    Move(MoveArgs),
    /// Show what moving a workspace folder would do, and change nothing.
    Plan(PlanArgs),
}

/// `move`'s arguments.
#[derive(Debug, Args)]
pub struct MoveArgs {
    /// The workspace folder to move.
    pub source: PathBuf,
    /// Where it goes. It mustn't exist yet.
    pub target: PathBuf,
    /// Show the plan and change nothing, like `plan`.
    #[arg(long)]
    pub dry_run: bool,
    /// Go ahead without asking. Never turns on an opt-in.
    #[arg(long)]
    pub yes: bool,
    /// Write the plan or report as JSON on standard output.
    #[arg(long)]
    pub json: bool,
}

/// `plan`'s arguments.
#[derive(Debug, Args)]
pub struct PlanArgs {
    /// The workspace folder that would move.
    pub source: PathBuf,
    /// Where it would go.
    pub target: PathBuf,
    /// Write the plan as JSON on standard output.
    #[arg(long)]
    pub json: bool,
}
