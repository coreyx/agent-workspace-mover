//! The `plan` and `move` commands (requirements.md, REQ-1, REQ-3 and REQ-4).
//!
//! Both build the same plan: the core's checks, then every adapter's contribution
//! (design.md, "Key design decisions", 2). `plan` and `move --dry-run` print it and stop.
//! `move` stops too when there's a blocker, or when no one has said to go ahead: the user
//! at a terminal, or `--yes`. Otherwise it records the move, carries it out, and prints
//! the report.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use awm_core::adapter::{Adapter, InspectContext, MoveRequest, assemble};
use awm_core::checks::{PLANNING_HOLDER_BUDGET, prepare_move};
use awm_core::executor::{ApplyOutcome, Executor, FailureKind};
use awm_core::locations::Locations;
use awm_core::paths::PathMapping;
use awm_core::plan::{
    AdapterKind, Notice, Outcome, Plan, PlanCommand, REPORT_SCHEMA, Report, ReportItem,
};
use awm_core::platform::Platform;
use awm_core::record::{MoveKind, MoveRecord};
use awm_core::state::{LockError, StateDir};

use crate::cli::{MoveArgs, PlanArgs};
use crate::exit::Exit;
use crate::output::{note, write_json};
use crate::prompt::Prompter;
use crate::render;

/// Everything a command reaches outside itself, so tests can stand in for any of it.
pub struct Env<'a> {
    /// Where things live.
    pub locations: Locations,
    /// The OS.
    pub platform: &'a dyn Platform,
    /// The adapters that contribute to plans.
    pub adapters: Vec<&'a dyn Adapter>,
    /// The folder relative paths are relative to.
    pub cwd: PathBuf,
    /// Standard output.
    pub out: &'a mut dyn Write,
    /// Standard error, for every human-readable message in `--json` mode.
    pub err: &'a mut dyn Write,
    /// The user's terminal, when there is one.
    pub prompter: Option<&'a mut dyn Prompter>,
}

/// Runs `plan`: prints the plan, and changes nothing.
pub fn plan_command(env: &mut Env<'_>, args: &PlanArgs) -> Exit {
    match make_plan(env, &args.source, &args.target) {
        Ok((plan, _)) => {
            if let Err(error) = print_plan(env, &plan, args.json) {
                return internal(env, &error);
            }
            Exit::NothingChanged
        }
        Err(exit) => exit,
    }
}

/// Runs `move`: plans, and unless that's all that was asked for, gets the go-ahead and
/// carries the move out.
pub fn move_command(env: &mut Env<'_>, args: &MoveArgs) -> Exit {
    let (plan, _request) = match make_plan(env, &args.source, &args.target) {
        Ok(planned) => planned,
        Err(exit) => return exit,
    };
    match go_ahead(env, &plan, args) {
        Ok(Some(exit)) => exit,
        Ok(None) => match carry_out(env, &plan, args.json) {
            Ok(exit) => exit,
            Err(error) => internal(env, &error),
        },
        Err(error) => internal(env, &error),
    }
}

/// Decides whether the move goes ahead. Returns the exit when it doesn't: a dry run, a
/// blocker, no go-ahead from a terminal, or no terminal and no `--yes` (3.0.5–3.0.7).
fn go_ahead(env: &mut Env<'_>, plan: &Plan, args: &MoveArgs) -> io::Result<Option<Exit>> {
    if args.dry_run {
        print_plan(env, plan, args.json)?;
        return Ok(Some(Exit::NothingChanged));
    }
    if plan.is_blocked() {
        if args.json {
            write_json(&blocked_report(plan), env.out)?;
        } else {
            write!(env.out, "{}", render::plan(plan))?;
            writeln!(env.out, "\nNothing was changed.")?;
        }
        return Ok(Some(Exit::Blocked));
    }
    if args.yes {
        if !args.json {
            write!(env.out, "{}", render::plan(plan))?;
        }
        return Ok(None);
    }
    match env.prompter.as_deref_mut() {
        None => {
            print_plan(env, plan, args.json)?;
            note(
                env.err,
                "Nothing was changed. To go ahead, run this again in a terminal, or with --yes.",
            )?;
            Ok(Some(Exit::NothingChanged))
        }
        Some(prompter) => {
            write!(env.out, "{}", render::plan(plan))?;
            if prompter.confirm("\nGo ahead with the move?")? {
                Ok(None)
            } else {
                writeln!(env.out, "Nothing was changed.")?;
                Ok(Some(Exit::NothingChanged))
            }
        }
    }
}

/// Records the move, carries it out, and prints the report.
fn carry_out(env: &mut Env<'_>, plan: &Plan, json: bool) -> io::Result<Exit> {
    let state = StateDir::open(&env.locations.state)?;
    let _lock = match state.try_lock() {
        Ok(lock) => lock,
        Err(error @ LockError::Busy) => {
            note(env.err, &error.to_string())?;
            return Ok(Exit::Blocked);
        }
        Err(LockError::Io(error)) => return Err(error),
    };
    let record = MoveRecord::create(
        &state,
        MoveKind::Move,
        &plan.source.to_string_lossy(),
        &plan.target.to_string_lossy(),
    )?;
    let applied = Executor::new(env.platform, &record).apply(plan)?;
    if applied
        .failure
        .as_ref()
        .is_some_and(|f| f.kind == FailureKind::Held)
    {
        // The rename found a holder the planning search missed, so look properly now.
        let found = env
            .platform
            .holders(&env.platform.processes(), &plan.source, None);
        for holder in found.holders {
            note(
                env.err,
                &format!(
                    "Holding the folder: {} (process {})",
                    holder.name, holder.pid
                ),
            )?;
        }
    }
    let report = build_report(plan, &record, &applied);
    record.write_report(&report)?;
    if json {
        write_json(&report, env.out)?;
    } else {
        write!(env.out, "\n{}", render::report(&report))?;
    }
    Ok(Exit::from(report.outcome))
}

/// The plan for moving `source` to `target`: the core's checks, then the adapters'.
fn make_plan(env: &mut Env<'_>, source: &Path, target: &Path) -> Result<(Plan, MoveRequest), Exit> {
    let processes = env.platform.processes();
    let prepared = prepare_move(
        source,
        target,
        &env.cwd,
        &env.locations,
        env.platform,
        &processes,
        Some(PLANNING_HOLDER_BUDGET),
    );
    let mapping = match PathMapping::for_host(&prepared.source, &prepared.target, prepared.case) {
        Ok(mapping) => mapping,
        Err(error) => {
            let _ = note(env.err, &format!("agent-workspace-mover: {error}"));
            return Err(Exit::Usage);
        }
    };
    let request = MoveRequest {
        command: PlanCommand::Move,
        source: prepared.source.clone(),
        target: prepared.target.clone(),
        mapping,
        fresh_settings: false,
        requested_opt_ins: vec![],
        allow_unverified: vec![],
    };
    let cx = InspectContext {
        locations: &env.locations,
        platform: env.platform,
        processes: &processes,
    };
    let mut plan = assemble(&env.adapters, &cx, &request, prepared.findings);
    let has_vcs = plan.adapters.iter().any(|s| {
        s.adapter.kind == AdapterKind::VersionControl
            && (!s.actions.is_empty() || !s.counts.is_empty())
    });
    if !has_vcs {
        plan.notices.push(Notice {
            code: "no-version-control".into(),
            message:
                "No version-control system was found for this folder, so there's nothing to repair."
                    .into(),
        });
    }
    Ok((plan, request))
}

/// Prints `plan` as JSON or for people.
fn print_plan(env: &mut Env<'_>, plan: &Plan, json: bool) -> io::Result<()> {
    if json {
        write_json(plan, env.out)
    } else {
        write!(env.out, "{}", render::plan(plan))
    }
}

/// The report for a move that checks stopped: every blocker, as something not done.
fn blocked_report(plan: &Plan) -> Report {
    Report {
        schema: REPORT_SCHEMA.into(),
        move_id: None,
        outcome: Outcome::Blocked,
        done: vec![],
        left_unchanged: vec![],
        couldnt_do: plan
            .all_blockers()
            .map(|b| ReportItem {
                adapter: "core".into(),
                summary: b.message.clone(),
                code: Some(b.code.clone()),
            })
            .collect(),
        warnings: vec![],
    }
}

/// The report for a move the executor ran (1.0.6).
fn build_report(plan: &Plan, record: &MoveRecord, applied: &ApplyOutcome) -> Report {
    let actions: Vec<_> = plan
        .ordered_actions()
        .into_iter()
        .filter(|a| a.opt_in.is_none())
        .collect();
    let done = actions[..applied.steps_done as usize]
        .iter()
        .map(|a| ReportItem {
            adapter: a.adapter.clone(),
            summary: a.summary.clone(),
            code: None,
        })
        .collect();
    let couldnt_do: Vec<ReportItem> = applied
        .failure
        .iter()
        .map(|f| ReportItem {
            adapter: actions
                .get(f.index as usize - 1)
                .map_or_else(|| "core".into(), |a| a.adapter.clone()),
            summary: f.message.clone(),
            code: Some(failure_code(f.kind).into()),
        })
        .collect();
    let outcome = match applied.outcome {
        Outcome::Succeeded if !couldnt_do.is_empty() => Outcome::SucceededWithWarnings,
        other => other,
    };
    Report {
        schema: REPORT_SCHEMA.into(),
        move_id: (outcome != Outcome::Blocked).then(|| record.id().as_str().to_owned()),
        outcome,
        done,
        left_unchanged: plan
            .adapters
            .iter()
            .flat_map(|s| s.left_unchanged.iter().cloned())
            .collect(),
        couldnt_do,
        warnings: vec![],
    }
}

/// The stable code for a step failure, for reports and scripts.
fn failure_code(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Held => "folder-held",
        FailureKind::PreconditionChanged => "changed-since-plan",
        FailureKind::VerifyFailed => "copy-verify-failed",
        FailureKind::Unsupported => "unsupported-step",
        FailureKind::Io => "io-error",
    }
}

/// Reports an unexpected I/O failure and returns the internal-error exit.
fn internal(env: &mut Env<'_>, error: &io::Error) -> Exit {
    let _ = note(env.err, &format!("agent-workspace-mover: {error}"));
    Exit::Internal
}

#[cfg(test)]
mod tests {
    use super::*;
    use awm_core::platform::Host;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::fs;

    /// A prompter that answers every question with `answer`.
    struct Answers(bool);

    impl Prompter for Answers {
        fn confirm(&mut self, _: &str) -> io::Result<bool> {
            Ok(self.0)
        }
    }

    /// A workspace with home and data folders, all in a temporary folder.
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        locations: Locations,
    }

    impl Fixture {
        /// A workspace at `<root>/work/repo`, moving to `<root>/new/repo`.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = awm_core::checks::strip_verbatim(fs::canonicalize(dir.path()).unwrap());
            let vars: HashMap<&str, OsString> = [
                ("HOME", root.join("home").into_os_string()),
                ("USERPROFILE", root.join("home").into_os_string()),
                ("APPDATA", root.join("roaming").into_os_string()),
                ("LOCALAPPDATA", root.join("local").into_os_string()),
                ("XDG_STATE_HOME", root.join("state").into_os_string()),
            ]
            .into_iter()
            .collect();
            let locations =
                Locations::from_vars(|n| vars.get(n).cloned(), awm_core::locations::Os::host())
                    .unwrap();
            fs::create_dir_all(root.join("work").join("repo")).unwrap();
            fs::create_dir_all(root.join("new")).unwrap();
            fs::write(
                root.join("work").join("repo").join("README.md"),
                b"# repo\n",
            )
            .unwrap();
            Self {
                _dir: dir,
                root,
                locations,
            }
        }

        /// Runs `move` with `args`, answering any go-ahead with `answer` (or with no
        /// terminal at all when `None`), and returns the exit, standard output and error.
        fn run_move(&self, args: &MoveArgs, answer: Option<bool>) -> (Exit, String, String) {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let mut answers = answer.map(Answers);
            let exit = {
                let mut env = Env {
                    locations: self.locations.clone(),
                    platform: &Host,
                    adapters: vec![],
                    cwd: self.root.clone(),
                    out: &mut out,
                    err: &mut err,
                    prompter: answers.as_mut().map(|a| a as &mut dyn Prompter),
                };
                move_command(&mut env, args)
            };
            (
                exit,
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        }

        /// `move work/repo new/repo` with the given switches.
        fn args(&self, yes: bool, json: bool, dry_run: bool) -> MoveArgs {
            MoveArgs {
                source: "work/repo".into(),
                target: "new/repo".into(),
                dry_run,
                yes,
                json,
            }
        }
    }

    // 3.4: anything but yes at the go-ahead changes nothing.
    #[test]
    fn declining_at_the_terminal_changes_nothing() {
        let f = Fixture::new();
        let (exit, out, _) = f.run_move(&f.args(false, false, false), Some(false));
        assert_eq!(exit, Exit::NothingChanged);
        assert!(out.contains("Nothing was changed."));
        assert!(f.root.join("work").join("repo").is_dir());
        assert!(!f.root.join("new").join("repo").exists());
    }

    #[test]
    fn saying_yes_at_the_terminal_moves_the_folder_and_reports() {
        let f = Fixture::new();
        let (exit, out, _) = f.run_move(&f.args(false, false, false), Some(true));
        assert_eq!(exit, Exit::Success, "{out}");
        assert!(f.root.join("new").join("repo").join("README.md").is_file());
        assert!(out.contains("Done:") && out.contains("Recorded as move"));
        let ids = MoveRecord::list(&StateDir::open(&f.locations.state).unwrap()).unwrap();
        assert_eq!(ids.len(), 1, "the move was recorded");
    }

    #[test]
    fn json_with_yes_prints_only_the_report() {
        let f = Fixture::new();
        let (exit, out, _) = f.run_move(&f.args(true, true, false), None);
        assert_eq!(exit, Exit::Success);
        let report: Report = serde_json::from_str(&out).unwrap();
        assert_eq!(
            (report.schema.as_str(), report.outcome),
            (REPORT_SCHEMA, Outcome::Succeeded)
        );
        assert_eq!(report.done.len(), 1);
    }

    #[test]
    fn json_without_yes_or_a_terminal_prints_the_plan_and_stops() {
        let f = Fixture::new();
        let (exit, out, err) = f.run_move(&f.args(false, true, false), None);
        assert_eq!(exit, Exit::NothingChanged);
        let plan: Plan = serde_json::from_str(&out).unwrap();
        assert!(plan.notices.iter().any(|n| n.code == "no-version-control"));
        assert!(err.contains("--yes"));
        assert!(!f.root.join("new").join("repo").exists());
    }

    #[test]
    fn a_blocked_move_reports_its_blockers_in_json() {
        let f = Fixture::new();
        fs::create_dir(f.root.join("new").join("repo")).unwrap();
        let (exit, out, _) = f.run_move(&f.args(true, true, false), None);
        assert_eq!(exit, Exit::Blocked);
        let report: Report = serde_json::from_str(&out).unwrap();
        assert_eq!(report.outcome, Outcome::Blocked);
        assert_eq!(report.couldnt_do[0].code.as_deref(), Some("target-exists"));
    }

    #[test]
    fn a_dry_run_prints_the_plan_and_changes_nothing() {
        let f = Fixture::new();
        let (exit, out, _) = f.run_move(&f.args(true, false, true), None);
        assert_eq!(exit, Exit::NothingChanged);
        assert!(out.contains("Will do:") && out.contains("move the folder"));
        assert!(f.root.join("work").join("repo").is_dir());
        assert!(
            !f.locations.state.exists(),
            "a dry run doesn't even create the state folder"
        );
    }

    #[test]
    fn a_second_run_while_one_holds_the_lock_is_refused() {
        let f = Fixture::new();
        let state = StateDir::open(&f.locations.state).unwrap();
        let _held = state.try_lock().unwrap();
        let (exit, _, err) = f.run_move(&f.args(true, false, false), None);
        assert_eq!(exit, Exit::Blocked);
        assert!(err.contains("another run"));
        assert!(f.root.join("work").join("repo").is_dir());
    }

    #[test]
    fn failures_have_stable_codes() {
        assert_eq!(failure_code(FailureKind::Held), "folder-held");
        assert_eq!(
            failure_code(FailureKind::PreconditionChanged),
            "changed-since-plan"
        );
        assert_eq!(
            failure_code(FailureKind::VerifyFailed),
            "copy-verify-failed"
        );
        assert_eq!(failure_code(FailureKind::Unsupported), "unsupported-step");
        assert_eq!(failure_code(FailureKind::Io), "io-error");
    }

    #[test]
    fn plan_prints_and_exits_without_changing_anything() {
        let f = Fixture::new();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let exit = {
            let mut env = Env {
                locations: f.locations.clone(),
                platform: &Host,
                adapters: vec![],
                cwd: f.root.clone(),
                out: &mut out,
                err: &mut err,
                prompter: None,
            };
            plan_command(
                &mut env,
                &PlanArgs {
                    source: "work/repo".into(),
                    target: "new/repo".into(),
                    json: false,
                },
            )
        };
        assert_eq!(exit, Exit::NothingChanged);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("No version-control system was found")
        );
        assert!(f.root.join("work").join("repo").is_dir());
    }
}
