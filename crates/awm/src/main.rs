//! The `agent-workspace-mover` command.
//!
//! This binary parses commands, builds the adapter registry, runs prompts, renders
//! human-readable and JSON output and maps outcomes to exit codes (design.md,
//! "Crates"). Until those land (tasks.md, TASK-9 and TASK-11), it answers only
//! `--version` and `--help`.

use std::process::ExitCode;

/// Exit code for a usage error, matching the table in design.md ("Exit codes").
const USAGE_ERROR: u8 = 2;

/// Runs the command with the process's arguments.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("agent-workspace-mover {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") | None => {
            println!("{}", help());
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!(
                "agent-workspace-mover: unknown command '{other}'\n\n{}",
                help()
            );
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// The help text shown for `--help` and after a usage error.
fn help() -> String {
    format!(
        "agent-workspace-mover {}\n\
         Moves or renames a workspace folder and carries everything keyed on its path.\n\n\
         This build has no commands yet. Options:\n  \
         -h, --help     Show this help\n  \
         -V, --version  Show the version",
        env!("CARGO_PKG_VERSION")
    )
}
