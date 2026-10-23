//! The only code that calls OS-specific interfaces (design.md, "Platform layer").
//!
//! The rest of the tool reaches the OS through the [`Platform`] trait, so tests can
//! stand in for it. [`Host`] is the real one. This layer covers processes and what's
//! holding a folder, volumes, creation times, and user-only permissions. TASK-28 adds
//! starting and waiting for the finish-later waiter.

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub use holders::{HoldKind, Holder, HolderReport, OpenFiles};
pub use process::{ProcessInfo, ProcessRef, ProcessSnapshot};

use crate::paths::{CaseRule, PathMapping};

pub mod holders;
pub mod process;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
mod unix_holders;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_holders;

/// Identifies a volume, so the tool can tell whether a move is a rename or a copy
/// (requirements.md, 1.0.3 and 1.0.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VolumeId(pub u64);

/// Everything the tool asks of the OS. [`Host`] implements it for real.
pub trait Platform: Send + Sync {
    /// A snapshot of every running process.
    fn processes(&self) -> ProcessSnapshot;

    /// What holds `root`, by working folder or open file, with the open-file search
    /// limited to `budget` (none when `None`).
    fn holders(
        &self,
        snapshot: &ProcessSnapshot,
        root: &Path,
        budget: Option<Duration>,
    ) -> HolderReport;

    /// The volume `path` is on. `path` must exist.
    fn volume_id(&self, path: &Path) -> io::Result<VolumeId>;

    /// Whether names on `path`'s volume are case-sensitive.
    fn case_rule(&self, path: &Path) -> CaseRule;

    /// When `path` was created, where the file system records it.
    fn creation_time(&self, path: &Path) -> io::Result<Option<SystemTime>>;

    /// Sets when `path` was created (requirements.md, 21.0.7). Unsupported on Linux.
    fn set_creation_time(&self, path: &Path, time: SystemTime) -> io::Result<()>;

    /// Makes `path` readable by the current user only.
    fn restrict_to_user(&self, path: &Path) -> io::Result<()>;
}

/// The machine this program runs on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Host;

impl Platform for Host {
    fn processes(&self) -> ProcessSnapshot {
        ProcessSnapshot::take()
    }

    fn holders(
        &self,
        snapshot: &ProcessSnapshot,
        root: &Path,
        budget: Option<Duration>,
    ) -> HolderReport {
        holders::find_holders(snapshot, root, self.case_rule(root), budget)
    }

    fn volume_id(&self, path: &Path) -> io::Result<VolumeId> {
        #[cfg(windows)]
        return windows_holders::volume_id(path);
        #[cfg(unix)]
        return unix_holders::volume_id(path);
    }

    fn case_rule(&self, path: &Path) -> CaseRule {
        #[cfg(windows)]
        {
            let _ = path;
            CaseRule::Insensitive
        }
        #[cfg(unix)]
        return unix_holders::case_rule(path);
    }

    fn creation_time(&self, path: &Path) -> io::Result<Option<SystemTime>> {
        Ok(std::fs::metadata(path)?.created().ok())
    }

    fn set_creation_time(&self, path: &Path, time: SystemTime) -> io::Result<()> {
        set_created(path, time)
    }

    fn restrict_to_user(&self, path: &Path) -> io::Result<()> {
        restrict_to_user(path)
    }
}

/// Sets `path`'s creation time on Windows.
#[cfg(windows)]
fn set_created(path: &Path, time: SystemTime) -> io::Result<()> {
    windows_holders::set_creation_time(path, time)
}

/// Sets `path`'s creation time on macOS, for a file or a folder.
#[cfg(target_os = "macos")]
fn set_created(path: &Path, time: SystemTime) -> io::Result<()> {
    use std::os::macos::fs::FileTimesExt;
    std::fs::File::open(path)?.set_times(std::fs::FileTimes::new().set_created(time))
}

/// Linux and other Unix systems keep no settable creation time.
#[cfg(not(any(windows, target_os = "macos")))]
fn set_created(_path: &Path, _time: SystemTime) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this OS doesn't let a creation time be set",
    ))
}

/// Processes with a file under `root` open, from whichever mechanism this OS has.
pub(crate) fn open_files_under(
    snapshot: &ProcessSnapshot,
    root: &Path,
    mapping: &PathMapping,
    budget: Option<Duration>,
) -> OpenFiles {
    #[cfg(windows)]
    {
        let _ = (snapshot, mapping);
        windows_holders::open_files_under(root, budget)
    }
    #[cfg(unix)]
    {
        let _ = root;
        unix_holders::open_files_under(snapshot, mapping, budget)
    }
}

/// Makes `path` readable and writable by the current user only. A folder's setting is
/// inherited by what's created in it later.
///
/// On Windows this sets a protected access-control list granting full control to the
/// current user's account and no one else. On macOS and Linux it sets mode `0700` for a
/// folder and `0600` for a file.
pub fn restrict_to_user(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    return windows::restrict_to_user(path);
    #[cfg(unix)]
    return unix::restrict_to_user(path);
}

/// Whether `path` is readable by the current user only, as [`restrict_to_user`] leaves it.
pub fn is_user_only(path: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    return windows::is_user_only(path);
    #[cfg(unix)]
    return unix::is_user_only(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    // 12.3: backups and records grant access only to the user who ran the tool.
    #[test]
    fn restricts_folders_and_files_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("state");
        std::fs::create_dir(&folder).unwrap();
        Host.restrict_to_user(&folder).unwrap();
        assert!(is_user_only(&folder).unwrap());

        let file = folder.join("record.json");
        std::fs::write(&file, b"{}").unwrap();
        restrict_to_user(&file).unwrap();
        assert!(is_user_only(&file).unwrap());
    }

    #[test]
    fn a_fresh_temporary_file_is_not_user_only_until_restricted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("open.txt");
        std::fs::write(&file, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        // A new file inherits its folder's access-control list on Windows, which grants
        // more than the user alone (SYSTEM and Administrators, at least).
        assert!(!is_user_only(&file).unwrap());
        restrict_to_user(&file).unwrap();
        assert!(is_user_only(&file).unwrap());
    }

    #[test]
    fn a_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(restrict_to_user(&missing).is_err());
        assert!(is_user_only(&missing).is_err());
        assert!(Host.volume_id(&missing).is_err());
        assert!(Host.creation_time(&missing).is_err());
    }

    #[test]
    fn paths_on_one_volume_share_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(&b, b"x").unwrap();
        assert_eq!(Host.volume_id(&a).unwrap(), Host.volume_id(&b).unwrap());
    }

    #[test]
    fn the_case_rule_matches_the_platform_default() {
        let dir = tempfile::tempdir().unwrap();
        let rule = Host.case_rule(dir.path());
        if cfg!(windows) || cfg!(target_os = "macos") {
            assert_eq!(
                rule,
                CaseRule::Insensitive,
                "the default volume on this OS is case-insensitive"
            );
        } else {
            assert_eq!(rule, CaseRule::Sensitive);
        }
    }

    // 21.0.7: a copy across volumes carries its folder's creation time where the OS allows.
    #[test]
    fn creation_times_can_be_set_where_the_os_allows() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let earlier = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let result = Host.set_creation_time(&folder, earlier);
        if cfg!(any(windows, target_os = "macos")) {
            result.unwrap();
            assert_eq!(Host.creation_time(&folder).unwrap(), Some(earlier));
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
    }

    #[test]
    fn the_host_lists_processes_and_finds_no_holders_in_an_empty_folder() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = Host.processes();
        assert!(snapshot.get(std::process::id()).is_some());
        let report = Host.holders(&snapshot, dir.path(), Some(Duration::from_secs(10)));
        assert!(report.holders.is_empty());
    }
}
