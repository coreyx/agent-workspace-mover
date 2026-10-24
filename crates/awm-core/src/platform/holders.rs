//! Finding what's holding a folder (requirements.md, REQ-20; design.md, "Key design
//! decisions", 9).
//!
//! A process holds a folder when its working folder is inside it, or when it has a file
//! inside it open. On Windows either one stops the folder being renamed. On macOS and
//! Linux neither does, but an editor or agent left pointing at the old path loses track,
//! so the plan names them all the same.
//!
//! What can't be checked is counted rather than ignored: a process whose working folder
//! or open files can't be read is `uninspectable`, and a scan that ran out of time or hit
//! an error is not `complete` (20.0.4). Neither is ever reported as "nothing holds it".

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::process::ProcessSnapshot;
use crate::paths::{CaseRule, PathMapping};

/// How a process holds a folder.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum HoldKind {
    /// Its working folder is the folder or inside it.
    WorkingFolder,
    /// It has a file inside the folder open.
    OpenFile,
}

/// One process holding a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    /// Its process id.
    pub pid: u32,
    /// Its program's name.
    pub name: String,
    /// Its executable's path, when readable.
    pub exe: Option<PathBuf>,
    /// How it holds the folder.
    pub kind: HoldKind,
    /// Whether it's this tool's own process or one of the processes that started it,
    /// such as the agent session that asked for the move (20.0.3).
    pub in_own_chain: bool,
}

/// What a search for holders found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HolderReport {
    /// The processes found holding the folder, at most once per process and kind.
    pub holders: Vec<Holder>,
    /// How many processes couldn't be inspected, usually other users' or elevated ones.
    pub uninspectable: usize,
    /// Whether the open-file search covered everything. It's false when it ran out of
    /// time or the OS reported an error.
    pub complete: bool,
}

/// What an OS's open-file search found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenFiles {
    /// Processes with a file under the folder open.
    pub pids: Vec<u32>,
    /// Processes whose open files couldn't be read.
    pub uninspectable: usize,
    /// Whether the search covered every file and process it meant to.
    pub complete: bool,
}

/// A mapping whose only use is asking whether a path is `root` or under it.
fn under(root: &Path, case: CaseRule) -> Option<PathMapping> {
    PathMapping::for_host(root, root, case).ok()
}

/// Whether `path` is `root`, or under it, per `mapping`.
pub(crate) fn is_under(mapping: &PathMapping, path: &Path) -> bool {
    path.to_str()
        .is_some_and(|text| mapping.covers(text.trim_end_matches(['\\', '/'])).is_some())
}

/// Processes whose working folder is `root` or under it.
pub fn working_folder_holders(
    snapshot: &ProcessSnapshot,
    root: &Path,
    case: CaseRule,
) -> HolderReport {
    let Some(mapping) = under(root, case) else {
        return HolderReport {
            holders: vec![],
            uninspectable: 0,
            complete: false,
        };
    };
    let chain = snapshot.own_chain();
    let mut report = HolderReport {
        complete: true,
        ..HolderReport::default()
    };
    for process in snapshot.iter() {
        match &process.cwd {
            Some(cwd) if is_under(&mapping, cwd) => report.holders.push(Holder {
                pid: process.pid,
                name: process.name.clone(),
                exe: process.exe.clone(),
                kind: HoldKind::WorkingFolder,
                in_own_chain: chain.contains(&process.pid),
            }),
            Some(_) => {}
            None => report.uninspectable += 1,
        }
    }
    report
}

/// Every process holding `root`: by working folder, and by open file within `budget`
/// (no limit when `None`).
pub fn find_holders(
    snapshot: &ProcessSnapshot,
    root: &Path,
    case: CaseRule,
    budget: Option<Duration>,
) -> HolderReport {
    let mut report = working_folder_holders(snapshot, root, case);
    let Some(mapping) = under(root, case) else {
        return report;
    };
    let open = super::open_files_under(snapshot, root, &mapping, budget);
    let chain = snapshot.own_chain();
    for pid in open.pids {
        let listed = report
            .holders
            .iter()
            .any(|h| h.pid == pid && h.kind == HoldKind::OpenFile);
        if !listed {
            let process = snapshot.get(pid);
            report.holders.push(Holder {
                pid,
                name: process.map_or_else(|| format!("process {pid}"), |p| p.name.clone()),
                exe: process.and_then(|p| p.exe.clone()),
                kind: HoldKind::OpenFile,
                in_own_chain: chain.contains(&pid),
            });
        }
    }
    report.uninspectable += open.uninspectable;
    report.complete &= open.complete;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::process::ProcessInfo;
    use awm_testkit::FakeProcess;

    /// A process with a working folder, started by `parent`.
    fn process(pid: u32, parent: Option<u32>, cwd: Option<&Path>) -> ProcessInfo {
        ProcessInfo {
            pid,
            start_time: 1,
            name: format!("p{pid}"),
            exe: None,
            args: vec![],
            parent,
            cwd: cwd.map(Path::to_path_buf),
        }
    }

    /// The host's case rule for temporary folders.
    fn case() -> CaseRule {
        if cfg!(windows) || cfg!(target_os = "macos") {
            CaseRule::Insensitive
        } else {
            CaseRule::Sensitive
        }
    }

    #[test]
    fn working_folders_under_the_root_are_holders_and_unreadable_ones_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let sibling = dir.path().join("repo-old");
        let snapshot = ProcessSnapshot::from_processes(
            vec![
                process(10, Some(5), Some(&root.join("src"))),
                process(5, None, Some(&root)),
                process(20, None, Some(&sibling)),
                process(30, None, None),
            ],
            10,
        );
        let report = working_folder_holders(&snapshot, &root, case());
        let pids: Vec<(u32, bool)> = report
            .holders
            .iter()
            .map(|h| (h.pid, h.in_own_chain))
            .collect();
        assert_eq!(
            pids,
            [(10, true), (5, true)],
            "the tool's own chain is flagged; the sibling isn't a holder"
        );
        assert_eq!(report.uninspectable, 1);
        assert!(report.complete);
    }

    // 20.1: a process whose working folder is under the source is named with its id.
    #[test]
    fn finds_a_process_working_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let fake = FakeProcess::start(&root.join("src"), None).unwrap();
        let report = find_holders(&ProcessSnapshot::take(), &root, case(), None);
        let holder = report
            .holders
            .iter()
            .find(|h| h.pid == fake.pid())
            .expect("the fake process is found");
        assert_eq!(holder.kind, HoldKind::WorkingFolder);
        assert!(!holder.in_own_chain);
    }

    // 20.1: a process holding a file open under the source is named with its id.
    #[test]
    fn finds_a_process_holding_a_file_open() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("held.txt");
        std::fs::write(&file, b"x").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let fake = FakeProcess::start(elsewhere.path(), Some(&file)).unwrap();
        let report = find_holders(&ProcessSnapshot::take(), &root, case(), None);
        assert!(report.complete);
        let kinds: Vec<HoldKind> = report
            .holders
            .iter()
            .filter(|h| h.pid == fake.pid())
            .map(|h| h.kind)
            .collect();
        assert_eq!(kinds, [HoldKind::OpenFile]);
    }

    // 20.1, with a real shell on Windows rather than a fake process.
    #[cfg(windows)]
    #[test]
    fn finds_a_real_cmd_working_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let mut cmd = std::process::Command::new("cmd.exe")
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .current_dir(&root)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        // Give cmd.exe a moment to start before reading its working folder.
        std::thread::sleep(Duration::from_millis(500));
        let report = find_holders(&ProcessSnapshot::take(), &root, case(), None);
        let _ = cmd.kill();
        let _ = cmd.wait();
        let holder = report
            .holders
            .iter()
            .find(|h| h.pid == cmd.id())
            .expect("cmd.exe is found");
        assert_eq!(
            (holder.name.as_str(), holder.kind),
            ("cmd.exe", HoldKind::WorkingFolder)
        );
    }

    #[test]
    fn a_folder_nothing_holds_has_no_holders() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("file.txt"), b"x").unwrap();
        let report = find_holders(&ProcessSnapshot::take(), &root, case(), None);
        assert!(
            report.holders.is_empty(),
            "nothing holds a fresh temporary folder"
        );
        assert!(report.complete);
    }

    #[test]
    fn a_root_that_is_not_unicode_is_never_reported_complete() {
        #[cfg(unix)]
        let root = {
            use std::os::unix::ffi::OsStrExt;
            PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff"))
        };
        #[cfg(windows)]
        let root = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_wide(&[
                b'C' as u16,
                b':' as u16,
                b'\\' as u16,
                0xD800,
            ]))
        };
        let snapshot = ProcessSnapshot::from_processes(vec![], 1);
        assert!(!working_folder_holders(&snapshot, &root, case()).complete);
        assert!(!find_holders(&snapshot, &root, case(), None).complete);
    }
}
