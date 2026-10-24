//! End-to-end checks of `plan` and `move`, running the real binary against a fixture
//! home (tasks.md, TASK-11). Each test names the acceptance criterion it proves.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use awm_testkit::{FakeProcess, FixtureHome};

awm_testkit::fake_process_entry!();

/// A fixture home with a workspace at `work/repo` and an empty `new/` beside it.
struct Fixture {
    home: FixtureHome,
    root: PathBuf,
}

impl Fixture {
    /// The fixture, with a small workspace in it.
    fn new() -> Self {
        let home = FixtureHome::new().unwrap();
        let root = strip_verbatim(fs::canonicalize(home.root()).unwrap());
        let repo = root.join("work").join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(root.join("new")).unwrap();
        fs::write(repo.join("README.md"), b"# repo\n").unwrap();
        fs::write(repo.join("src").join("main.rs"), b"fn main() {}\n").unwrap();
        Self { home, root }
    }

    /// The workspace.
    fn source(&self) -> PathBuf {
        self.root.join("work").join("repo")
    }

    /// Where it moves to.
    fn target(&self) -> PathBuf {
        self.root.join("new").join("repo")
    }

    /// The binary, pointed at the fixture home, run from the fixture's root.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::cargo_bin("agent-workspace-mover").unwrap();
        let mut std = std::process::Command::new(command.get_program());
        self.home.apply(&mut std);
        for (name, value) in std.get_envs() {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        command.current_dir(&self.root).args(args);
        command
    }

    /// Every file and folder under the fixture's root, with each file's content hash:
    /// the "hashed listing" the criteria compare before and after.
    fn listing(&self) -> BTreeMap<PathBuf, String> {
        let mut listing = BTreeMap::new();
        let mut folders = vec![self.root.clone()];
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(&folder).unwrap() {
                let entry = entry.unwrap();
                let relative = entry.path().strip_prefix(&self.root).unwrap().to_path_buf();
                if entry.file_type().unwrap().is_dir() {
                    listing.insert(relative, "folder".into());
                    folders.push(entry.path());
                } else {
                    listing.insert(relative, format!("{:?}", fs::read(entry.path()).unwrap()));
                }
            }
        }
        listing
    }
}

/// `path` without Windows' verbatim prefix.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    text.strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(path)
}

/// `path` as an argument.
fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Standard output as text.
fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// 1.2: a folder with no version control moves, and the plan says no adapter applied.
// 3.5: `move --yes` with no terminal completes the default move.
#[test]
fn a_plain_folder_moves_with_yes_and_no_terminal() {
    let f = Fixture::new();
    let before: BTreeMap<_, _> = f
        .listing()
        .into_iter()
        .filter(|(p, _)| p.starts_with("work"))
        .collect();
    let output = f
        .command(&["move", arg(&f.source()), arg(&f.target()), "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("No version-control system was found"));
    assert!(!f.source().exists());
    let after: BTreeMap<_, _> = f
        .listing()
        .into_iter()
        .filter_map(|(p, h)| {
            p.strip_prefix("new")
                .ok()
                .map(|rest| (Path::new("work").join(rest), h))
        })
        .collect();
    assert_eq!(after, before, "every file arrived unchanged");
}

// 1.6: the report has its "done", "left unchanged" and "couldn't do" sections, each
// present even when empty, as text and as JSON.
#[test]
fn the_report_always_has_its_three_sections() {
    let f = Fixture::new();
    let output = f
        .command(&["move", arg(&f.source()), arg(&f.target()), "--yes"])
        .output()
        .unwrap();
    let text = stdout(&output);
    for section in ["Done:", "Left unchanged:\n  none", "Couldn't do:\n  none"] {
        assert!(text.contains(section), "missing {section:?} in:\n{text}");
    }
    let g = Fixture::new();
    let output = g
        .command(&[
            "move",
            arg(&g.source()),
            arg(&g.target()),
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for section in ["done", "left_unchanged", "couldnt_do"] {
        assert!(report[section].is_array(), "{section} is present");
    }
}

// 2.1: with the target already present, move is blocked and nothing changes.
#[test]
fn an_existing_target_blocks_the_move_and_changes_nothing() {
    let f = Fixture::new();
    fs::create_dir(f.target()).unwrap();
    let before = f.listing();
    let output = f
        .command(&["move", arg(&f.source()), arg(&f.target()), "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(stdout(&output).contains("[target-exists]"));
    assert_eq!(f.listing(), before);
}

// 2.2: the home folder, a drive root and Claude Code's data folder are each refused.
#[test]
fn home_drive_roots_and_agent_data_are_refused() {
    let f = Fixture::new();
    let claude = f.home.home().join(".claude");
    fs::create_dir_all(&claude).unwrap();
    let root = if cfg!(windows) {
        f.root.to_string_lossy()[..3].to_string()
    } else {
        "/".to_string()
    };
    for source in [
        arg(&f.home.home()).to_string(),
        root,
        arg(&claude).to_string(),
    ] {
        let output = f
            .command(&["move", &source, arg(&f.target()), "--yes"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{source}");
        let text = stdout(&output);
        assert!(
            text.contains("[refused-location]"),
            "{source} refused:\n{text}"
        );
    }
}

// 2.3: with a terminal open in a subfolder of the source, the blockers name it.
#[test]
fn a_process_in_the_folder_is_named_and_nothing_changes() {
    let f = Fixture::new();
    let holder = FakeProcess::start(&f.source().join("src"), None).unwrap();
    let before = f.listing();
    let output = f
        .command(&["move", arg(&f.source()), arg(&f.target()), "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let text = stdout(&output);
    assert!(text.contains("[folder-held]"));
    assert!(
        text.contains(&format!("process {}", holder.pid())),
        "the holder is named with its id:\n{text}"
    );
    assert_eq!(f.listing(), before);
}

// 2.6: a source with three different blockers reports all three in one run.
#[test]
fn three_blockers_are_reported_together() {
    let f = Fixture::new();
    let inside = f.source().join("src");
    let _holder = FakeProcess::start(&inside, None).unwrap();
    let output = f
        .command(&["move", arg(&f.source()), arg(&inside), "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let text = stdout(&output);
    for code in ["[target-exists]", "[target-inside-source]", "[folder-held]"] {
        assert!(text.contains(code), "missing {code} in:\n{text}");
    }
}

// 3.6: with no terminal and no --yes, move prints the plan, changes nothing and exits 3.
#[test]
fn without_a_terminal_or_yes_move_only_shows_the_plan() {
    let f = Fixture::new();
    let before = f.listing();
    let output = f
        .command(&["move", arg(&f.source()), arg(&f.target())])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(stdout(&output).contains("Will do:"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--yes"));
    assert_eq!(f.listing(), before);
}

// 4.2: `plan` changes nothing at all in the fixture, the tool's own state included.
#[test]
fn plan_changes_nothing() {
    let f = Fixture::new();
    let before = f.listing();
    for args in [
        vec!["plan", arg(&f.source()), arg(&f.target())],
        vec!["move", "--dry-run", arg(&f.source()), arg(&f.target())],
    ] {
        let output = f.command(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(3), "{args:?}");
        assert_eq!(f.listing(), before, "{args:?} changed nothing");
    }
}

// 22.1 and 22.3, on real output: `plan --json` is one document that validates against
// the committed schema.
#[test]
fn plan_json_validates_against_the_committed_schema() {
    let f = Fixture::new();
    let output = f
        .command(&["plan", arg(&f.source()), arg(&f.target()), "--json"])
        .output()
        .unwrap();
    let documents: Vec<serde_json::Value> = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(documents.len(), 1);
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema")
        .join("awm.plan.v1.json");
    let schema: serde_json::Value =
        serde_json::from_slice(&fs::read(schema_path).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&documents[0])
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_bad_command_line_is_a_usage_error() {
    let f = Fixture::new();
    assert_eq!(
        f.command(&["move", "only-one-path"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        f.command(&[]).output().unwrap().status.code(),
        Some(0),
        "no command shows help"
    );
}
