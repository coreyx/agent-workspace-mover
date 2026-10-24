//! The `agent-workspace-mover` command.
//!
//! This binary parses commands, builds the adapter registry, runs prompts, renders
//! human-readable and JSON output and maps outcomes to exit codes (design.md,
//! "Crates").

use std::io;
use std::process::ExitCode;

use awm_core::locations::Locations;
use awm_core::platform::Host;
use clap::Parser;

use cli::{Cli, Command};
use commands::Env;
use exit::Exit;
use prompt::{Prompter, Terminal};

mod cli;
mod commands;
mod exit;
mod output;
mod prompt;
mod render;
#[cfg(test)]
mod samples;
// The schemas are generated and checked by tests; nothing at run time needs them.
#[cfg_attr(not(test), allow(dead_code))]
mod schema;

/// Runs the command with the process's arguments.
fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // clap prints help and version itself, exiting 0, and usage errors, exiting 2.
            let _ = error.print();
            return ExitCode::from(error.exit_code() as u8);
        }
    };
    let Some(command) = cli.command else {
        use clap::CommandFactory;
        let _ = Cli::command().print_help();
        return Exit::Success.into();
    };
    let locations = match Locations::from_env() {
        Ok(locations) => locations,
        Err(error) => {
            eprintln!("agent-workspace-mover: {error}");
            return Exit::Usage.into();
        }
    };
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("agent-workspace-mover: the current folder can't be read: {error}");
            return Exit::Internal.into();
        }
    };
    let (stdout, stderr) = (io::stdout(), io::stderr());
    let (mut out, mut err) = (stdout.lock(), stderr.lock());
    let mut terminal = Terminal::open();
    let mut env = Env {
        locations,
        platform: &Host,
        adapters: vec![],
        cwd,
        out: &mut out,
        err: &mut err,
        prompter: terminal.as_mut().map(|t| t as &mut dyn Prompter),
    };
    let exit = match &command {
        Command::Move(args) => commands::move_command(&mut env, args),
        Command::Plan(args) => commands::plan_command(&mut env, args),
    };
    exit.into()
}
