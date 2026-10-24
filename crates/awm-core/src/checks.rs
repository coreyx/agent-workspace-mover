//! The core's own checks before a move (requirements.md, REQ-2), and the folder move
//! they lead to.
//!
//! Every check runs, and every blocker is collected, so one run reports everything in
//! the way (2.0.9). Nothing here changes anything: the checks read the file system and
//! the process snapshot only, so a dry run can make them while agents run (4.0.3).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adapter::{CORE_ADAPTER_ID, CoreFindings};
use crate::locations::Locations;
use crate::paths::{CaseRule, PathMapping};
use crate::plan::{
    Action, Blocker, HolderEntry, Notice, Phase, PlanHolders, PlannedAction, Precondition,
    VolumeRelation,
};
use crate::platform::{HoldKind, Platform, ProcessSnapshot};

/// How long planning spends asking which processes have files open in the source
/// (design.md, "Key design decisions", 9). The rename itself is the final test.
pub const PLANNING_HOLDER_BUDGET: Duration = Duration::from_secs(3);

/// A move's paths as they'll be used, and what the core's checks found.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// The real folder that moves: absolute, with links in its path resolved.
    pub source: PathBuf,
    /// Where it goes: absolute, with links in its parent's path resolved.
    pub target: PathBuf,
    /// Whether names on the source's volume are case-sensitive.
    pub case: CaseRule,
    /// Blockers, notices, holders and the folder move itself.
    pub findings: CoreFindings,
}

/// A blocker with `code`, `message` and `remedy`.
fn blocker(code: &str, message: String, remedy: &str) -> Blocker {
    Blocker {
        code: code.into(),
        message,
        remedy: Some(remedy.into()),
    }
}

/// `path` made absolute against `cwd`, without touching the file system.
fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// `path` with Windows' verbatim prefix removed (`\\?\C:\x` becomes `C:\x`), which is
/// how `fs::canonicalize` returns paths there. Elsewhere `path` is returned unchanged.
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

/// `path` with links resolved, if it exists.
fn resolved(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().map(strip_verbatim)
}

/// Whether `inner` is `outer` or under it, under `case`.
fn is_within(outer: &Path, inner: &Path, case: CaseRule) -> bool {
    match (PathMapping::for_host(outer, outer, case), inner.to_str()) {
        (Ok(mapping), Some(text)) => mapping.covers(text).is_some(),
        _ => false,
    }
}

/// The total size of the files under `folder`, not following links.
fn tree_size(folder: &Path) -> io::Result<u64> {
    let mut total = 0;
    let mut folders = vec![folder.to_path_buf()];
    while let Some(current) = folders.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}

/// Bytes as people read them, such as `1.5 GB`.
fn human_bytes(bytes: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// Runs the core's checks for moving `given_source` to `given_target`, relative paths
/// being relative to `cwd`, and plans the folder move itself.
pub fn prepare_move(
    given_source: &Path,
    given_target: &Path,
    cwd: &Path,
    locations: &Locations,
    platform: &dyn Platform,
    processes: &ProcessSnapshot,
    holder_budget: Option<Duration>,
) -> Prepared {
    let mut blockers = Vec::new();
    let mut notices = Vec::new();

    // The real folder moves, so a source reached through a link is resolved (5.0.5).
    let source_given = absolute(cwd, given_source);
    let source_exists = fs::symlink_metadata(&source_given).is_ok();
    let source = if source_exists {
        resolved(&source_given).unwrap_or_else(|| source_given.clone())
    } else {
        source_given.clone()
    };
    if !source_exists {
        blockers.push(blocker(
            "source-missing",
            format!("{} doesn't exist.", source_given.display()),
            "Check the path, or use rescue for a folder that was already moved by hand.",
        ));
    } else if source != source_given {
        notices.push(Notice {
            code: "source-through-link".into(),
            message: format!(
                "{} is reached through a link. The real folder, {}, is the one that moves.",
                source_given.display(),
                source.display()
            ),
        });
    }

    let target_given = absolute(cwd, given_target);
    let target_parent = target_given.parent().map(Path::to_path_buf);
    let target = match (&target_parent, target_given.file_name()) {
        (Some(parent), Some(name)) => {
            resolved(parent).map_or_else(|| target_given.clone(), |p| p.join(name))
        }
        _ => target_given.clone(),
    };
    let case = platform.case_rule(if source_exists { &source } else { cwd });

    if fs::symlink_metadata(&target).is_ok() {
        blockers.push(blocker(
            "target-exists",
            format!("{} already exists.", target.display()),
            "Choose a target that doesn't exist yet, or move what's there out of the way.",
        ));
    }
    match target.parent() {
        Some(parent) if parent.is_dir() => {}
        _ => blockers.push(blocker(
            "target-parent-missing",
            format!(
                "The folder the target goes in, {}, doesn't exist.",
                target_parent.as_deref().unwrap_or(&target).display()
            ),
            "Create it first, then plan the move again.",
        )),
    }
    if is_within(&source, &target, case) {
        blockers.push(blocker(
            "target-inside-source",
            format!("{} is inside the folder being moved.", target.display()),
            "Choose a target outside the source folder.",
        ));
    }
    if is_within(&target, &source, case) && source != target {
        blockers.push(blocker(
            "source-inside-target",
            format!("The folder being moved is inside {}.", target.display()),
            "Choose a target that doesn't contain the source.",
        ));
    }

    // Folders whose move would strand the user's whole setup are refused (2.0.3).
    let refuse = |why: String| {
        blocker(
            "refused-location",
            why,
            "Move a workspace folder rather than this one.",
        )
    };
    if source.parent().is_none() {
        blockers.push(refuse(format!(
            "{} is the root of a drive.",
            source.display()
        )));
    }
    if is_within(&source, &locations.home, case) {
        blockers.push(refuse(format!(
            "{} is or contains the home folder.",
            source.display()
        )));
    }
    for managed in [
        &locations.claude_dir,
        &locations.claude_json,
        &locations.vscode_user,
        &locations.state,
    ] {
        if is_within(&source, managed, case) || is_within(managed, &source, case) {
            blockers.push(refuse(format!(
                "{} holds agent or editor data the tool manages.",
                source.display()
            )));
            break;
        }
    }

    let mut volume_relation = VolumeRelation::Same;
    if let (true, Some(parent)) = (source_exists, target.parent().filter(|p| p.is_dir())) {
        match (platform.volume_id(&source), platform.volume_id(parent)) {
            (Ok(a), Ok(b)) if a != b => {
                volume_relation = VolumeRelation::Different;
                if let (Ok(needed), Ok(available)) =
                    (tree_size(&source), platform.available_space(parent))
                    && needed > available
                {
                    blockers.push(blocker(
                        "not-enough-space",
                        format!(
                            "Copying the folder needs {} on the target's volume, which has {} free.",
                            human_bytes(needed),
                            human_bytes(available)
                        ),
                        "Free some space there, or choose a target on another volume.",
                    ));
                }
            }
            _ => {}
        }
    }

    for folder in [source.parent(), target.parent()].into_iter().flatten() {
        if folder.is_dir() && crate::platform::writable(folder) == Some(false) {
            blockers.push(blocker(
                "permission-denied",
                format!("You can't change the contents of {}.", folder.display()),
                "Choose folders you can write to, or fix their permissions.",
            ));
        }
    }

    let mut holders = PlanHolders {
        complete: true,
        ..PlanHolders::default()
    };
    if source_exists {
        let found = platform.holders(processes, &source, holder_budget);
        holders = PlanHolders {
            holders: found
                .holders
                .iter()
                .map(|h| HolderEntry {
                    pid: h.pid,
                    name: h.name.clone(),
                    exe: h.exe.clone(),
                    kind: h.kind,
                    this_session: h.in_own_chain,
                })
                .collect(),
            uninspectable: found.uninspectable as u64,
            complete: found.complete,
        };
        if !holders.holders.is_empty() {
            let named: Vec<String> = holders
                .holders
                .iter()
                .map(|h| {
                    let how = if h.kind == HoldKind::WorkingFolder {
                        "working in it"
                    } else {
                        "with a file in it open"
                    };
                    format!("{} (process {}), {how}", h.name, h.pid)
                })
                .collect();
            let remedy = if holders.holders.iter().any(|h| h.this_session) {
                "The session you're running this from is one of them. Run the move from a terminal outside the folder after that session ends, and close the others."
            } else {
                "Close them, then try again."
            };
            blockers.push(blocker(
                "folder-held",
                format!("Something is holding the folder: {}.", named.join("; ")),
                remedy,
            ));
        }
        if !holders.complete || holders.uninspectable > 0 {
            notices.push(Notice {
                code: "holders-incomplete".into(),
                message: "Some processes couldn't be inspected, so something else may be holding the folder. If so, the move stops before changing anything.".into(),
            });
        }
    }

    let actions = if source_exists {
        vec![PlannedAction {
            phase: Phase::MoveWorkspace,
            adapter: CORE_ADAPTER_ID.into(),
            summary: match volume_relation {
                VolumeRelation::Different => format!(
                    "copy the folder to {}, verify it, then remove the original",
                    target.display()
                ),
                _ => format!("move the folder to {}", target.display()),
            },
            action: Action::MoveTree {
                from: source.clone(),
                to: target.clone(),
            },
            preconditions: vec![
                Precondition::Present {
                    path: source.clone(),
                },
                Precondition::Absent {
                    path: target.clone(),
                },
            ],
            opt_in: None,
        }]
    } else {
        Vec::new()
    };

    Prepared {
        source,
        target,
        case,
        findings: CoreFindings {
            volume_relation,
            holders,
            blockers,
            notices,
            actions,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Host;
    use std::collections::HashMap;
    use std::ffi::OsString;

    /// A fixture: locations inside a temporary folder, a workspace, and a place to move it.
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        locations: Locations,
        source: PathBuf,
    }

    impl Fixture {
        /// A workspace at `<root>/work/repo`, with home and data folders under `<root>`.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = strip_verbatim(fs::canonicalize(dir.path()).unwrap());
            let home = root.join("home");
            let vars: HashMap<&str, OsString> = [
                ("HOME", home.clone().into_os_string()),
                ("USERPROFILE", home.clone().into_os_string()),
                ("APPDATA", root.join("roaming").into_os_string()),
                ("LOCALAPPDATA", root.join("local").into_os_string()),
            ]
            .into_iter()
            .collect();
            let locations =
                Locations::from_vars(|name| vars.get(name).cloned(), crate::locations::Os::host())
                    .unwrap();
            let source = root.join("work").join("repo");
            fs::create_dir_all(source.join("src")).unwrap();
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(root.join("new")).unwrap();
            fs::write(source.join("README.md"), b"# repo\n").unwrap();
            Self {
                _dir: dir,
                root,
                locations,
                source,
            }
        }

        /// The core's checks for moving `source` to `target`.
        fn check(&self, source: &Path, target: &Path) -> Prepared {
            prepare_move(
                source,
                target,
                &self.root,
                &self.locations,
                &Host,
                &ProcessSnapshot::take(),
                Some(PLANNING_HOLDER_BUDGET),
            )
        }
    }

    /// The codes of `prepared`'s blockers, in order.
    fn codes(prepared: &Prepared) -> Vec<String> {
        prepared
            .findings
            .blockers
            .iter()
            .map(|b| b.code.clone())
            .collect()
    }

    #[test]
    fn a_clean_move_has_no_blockers_and_one_folder_move() {
        let f = Fixture::new();
        let target = f.root.join("new").join("repo");
        let prepared = f.check(&f.source, &target);
        assert!(
            codes(&prepared).is_empty(),
            "{:?}",
            prepared.findings.blockers
        );
        assert_eq!(prepared.findings.volume_relation, VolumeRelation::Same);
        assert_eq!(prepared.findings.actions.len(), 1);
        assert_eq!(
            prepared.findings.actions[0].action,
            Action::MoveTree {
                from: f.source.clone(),
                to: target.clone()
            }
        );
        assert!(
            prepared.findings.holders.complete
                || prepared
                    .findings
                    .notices
                    .iter()
                    .any(|n| n.code == "holders-incomplete")
        );
    }

    #[test]
    fn relative_paths_are_taken_from_the_working_folder() {
        let f = Fixture::new();
        let prepared = f.check(Path::new("work/repo"), Path::new("new/repo"));
        assert_eq!(prepared.source, f.source);
        assert_eq!(prepared.target, f.root.join("new").join("repo"));
    }

    // 2.0.2: target exists, source missing, one inside the other.
    #[test]
    fn reports_every_problem_with_the_paths() {
        let f = Fixture::new();
        fs::create_dir(f.root.join("new").join("taken")).unwrap();
        assert_eq!(
            codes(&f.check(&f.source, &f.root.join("new").join("taken"))),
            ["target-exists"]
        );
        let missing = f.check(&f.root.join("nowhere"), &f.root.join("new").join("x"));
        assert_eq!(codes(&missing), ["source-missing"]);
        assert!(missing.findings.actions.is_empty());
        assert_eq!(
            codes(&f.check(&f.source, &f.source.join("sub"))),
            ["target-inside-source"]
        );
        assert_eq!(
            codes(&f.check(&f.source, &f.root.join("no-such-folder").join("repo"))),
            ["target-parent-missing"]
        );
        let inner = f.source.join("src");
        assert_eq!(
            codes(&f.check(&inner, &f.root.join("new").join("src"))),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_source_inside_the_target_is_a_blocker() {
        let f = Fixture::new();
        let prepared = f.check(&f.source.join("src"), &f.source);
        assert!(codes(&prepared).contains(&"source-inside-target".to_string()));
        assert!(codes(&prepared).contains(&"target-exists".to_string()));
    }

    // 2.0.3: the home folder, a drive root, and the data the tool manages are refused.
    #[test]
    fn refuses_home_roots_and_managed_data() {
        let f = Fixture::new();
        let target = f.root.join("new").join("x");
        assert!(
            codes(&f.check(&f.locations.home, &target)).contains(&"refused-location".to_string())
        );
        assert!(
            codes(&f.check(&f.root, &target)).contains(&"refused-location".to_string()),
            "contains the home folder"
        );
        fs::create_dir_all(&f.locations.claude_dir).unwrap();
        assert!(
            codes(&f.check(&f.locations.claude_dir, &target))
                .contains(&"refused-location".to_string())
        );
        let root = if cfg!(windows) {
            PathBuf::from(&f.root.to_string_lossy()[..3])
        } else {
            PathBuf::from("/")
        };
        let prepared = f.check(&root, &target);
        assert!(
            prepared
                .findings
                .blockers
                .iter()
                .any(|b| b.code == "refused-location" && b.message.contains("root of a drive"))
        );
    }

    // 2.0.4: a process working in the source is named as holding it.
    #[test]
    fn names_a_process_holding_the_folder() {
        let f = Fixture::new();
        let _holder = awm_testkit::FakeProcess::start(&f.source.join("src"), None).unwrap();
        let prepared = f.check(&f.source, &f.root.join("new").join("repo"));
        let held = prepared
            .findings
            .blockers
            .iter()
            .find(|b| b.code == "folder-held")
            .expect("a folder-held blocker");
        assert!(held.message.contains("working in it"));
        assert_eq!(held.remedy.as_deref(), Some("Close them, then try again."));
        assert!(!prepared.findings.holders.holders.is_empty());
    }

    #[test]
    fn a_source_reached_through_a_link_resolves_to_the_real_folder() {
        let f = Fixture::new();
        let link = f.root.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(f.source.parent().unwrap(), &link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(f.source.parent().unwrap())
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(made.success());
        }
        let prepared = f.check(&link.join("repo"), &f.root.join("new").join("repo"));
        assert_eq!(prepared.source, f.source);
        assert!(
            prepared
                .findings
                .notices
                .iter()
                .any(|n| n.code == "source-through-link")
        );
    }

    #[test]
    fn sizes_and_byte_counts() {
        let f = Fixture::new();
        assert_eq!(tree_size(&f.source).unwrap(), 7);
        assert_eq!(human_bytes(7), "7 bytes");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(strip_verbatim(PathBuf::from("/a/b")), PathBuf::from("/a/b"));
        if cfg!(windows) {
            assert_eq!(
                strip_verbatim(PathBuf::from(r"\\?\C:\a")),
                PathBuf::from(r"C:\a")
            );
            assert_eq!(
                strip_verbatim(PathBuf::from(r"\\?\UNC\srv\share")),
                PathBuf::from(r"\\srv\share")
            );
        }
    }

    // 2.0.8: a cross-volume move that needs more space than the target has is blocked.
    #[test]
    fn a_copy_needing_more_space_than_there_is_is_blocked() {
        struct Tiny;
        impl Platform for Tiny {
            fn processes(&self) -> ProcessSnapshot {
                ProcessSnapshot::from_processes(vec![], 1)
            }
            fn holders(
                &self,
                s: &ProcessSnapshot,
                r: &Path,
                b: Option<Duration>,
            ) -> crate::platform::HolderReport {
                Host.holders(s, r, b)
            }
            fn volume_id(&self, path: &Path) -> io::Result<crate::platform::VolumeId> {
                Ok(crate::platform::VolumeId(if path.ends_with("new") {
                    2
                } else {
                    1
                }))
            }
            fn case_rule(&self, path: &Path) -> CaseRule {
                Host.case_rule(path)
            }
            fn creation_time(&self, path: &Path) -> io::Result<Option<std::time::SystemTime>> {
                Host.creation_time(path)
            }
            fn set_creation_time(
                &self,
                path: &Path,
                time: std::time::SystemTime,
            ) -> io::Result<()> {
                Host.set_creation_time(path, time)
            }
            fn restrict_to_user(&self, path: &Path) -> io::Result<()> {
                Host.restrict_to_user(path)
            }
            fn available_space(&self, _: &Path) -> io::Result<u64> {
                Ok(3)
            }
        }
        let f = Fixture::new();
        let prepared = prepare_move(
            &f.source,
            &f.root.join("new").join("repo"),
            &f.root,
            &f.locations,
            &Tiny,
            &Tiny.processes(),
            None,
        );
        assert_eq!(prepared.findings.volume_relation, VolumeRelation::Different);
        assert!(prepared.findings.actions[0].summary.contains("verify"));
        let space = prepared
            .findings
            .blockers
            .iter()
            .find(|b| b.code == "not-enough-space")
            .unwrap();
        assert!(space.message.contains("7 bytes") && space.message.contains("3 bytes"));
    }
}
