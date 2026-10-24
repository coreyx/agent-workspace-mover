//! The `agent-workspace-mover` command.
//!
//! This binary parses commands, builds the adapter registry, runs prompts, renders
//! human-readable and JSON output and maps outcomes to exit codes (design.md,
//! "Crates"). Until the commands land (tasks.md, TASK-11), it answers only `--version`
//! and `--help`.

use std::process::ExitCode;

use exit::Exit;

mod exit;
// The plan and move commands that render and print these arrive in TASK-11.
#[cfg_attr(not(test), allow(dead_code))]
mod output;
#[cfg_attr(not(test), allow(dead_code))]
mod render;
#[cfg(test)]
mod samples;
#[cfg_attr(not(test), allow(dead_code))]
mod schema;

/// Runs the command with the process's arguments.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("agent-workspace-mover {}", env!("CARGO_PKG_VERSION"));
            Exit::Success.into()
        }
        Some("--help" | "-h") | None => {
            println!("{}", help());
            Exit::Success.into()
        }
        Some(other) => {
            eprintln!(
                "agent-workspace-mover: unknown command '{other}'\n\n{}",
                help()
            );
            Exit::Usage.into()
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
