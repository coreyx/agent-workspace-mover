//! End-to-end checks of the binary's command-line surface.

use std::process::Command;

/// Runs the built binary with `args`, returning its exit code and standard output.
fn run(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_agent-workspace-mover"))
        .args(args)
        .output()
        .expect("the binary runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn version_prints_the_package_version() {
    let (code, stdout) = run(&["--version"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout.trim(),
        format!("agent-workspace-mover {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn an_unknown_command_is_a_usage_error() {
    let (code, _) = run(&["no-such-command"]);
    assert_eq!(code, Some(2));
}
