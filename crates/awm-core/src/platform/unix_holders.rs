//! Open files, volumes and case sensitivity on macOS and Linux.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::VolumeId;
use super::holders::{OpenFiles, is_under};
use super::process::ProcessSnapshot;
use crate::paths::{CaseRule, PathMapping};

/// The paths of the files `pid` has open. Fails with `PermissionDenied` for a process
/// the current user can't inspect.
#[cfg(target_os = "linux")]
fn open_paths(pid: u32) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(format!("/proc/{pid}/fd"))? {
        // A descriptor can close between listing and reading it.
        if let Ok(target) = fs::read_link(entry?.path()) {
            paths.push(target);
        }
    }
    Ok(paths)
}

/// The paths of the files `pid` has open, through `libproc`. Fails with
/// `PermissionDenied` for a process the current user can't inspect.
#[cfg(target_os = "macos")]
fn open_paths(pid: u32) -> io::Result<Vec<PathBuf>> {
    super::macos::open_paths(pid)
}

/// The paths of the files `pid` has open, on Unix systems with no known way to ask.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_paths(_pid: u32) -> io::Result<Vec<PathBuf>> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Processes with a file under `mapping`'s root open, within `budget`.
pub fn open_files_under(
    snapshot: &ProcessSnapshot,
    mapping: &PathMapping,
    budget: Option<Duration>,
) -> OpenFiles {
    let deadline = budget.map(|b| Instant::now() + b);
    let mut found = OpenFiles {
        complete: true,
        ..OpenFiles::default()
    };
    for process in snapshot.iter() {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            found.complete = false;
            break;
        }
        match open_paths(process.pid) {
            Ok(paths) => {
                if paths.iter().any(|path| is_under(mapping, path)) {
                    found.pids.push(process.pid);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {} // It exited.
            Err(_) => found.uninspectable += 1,
        }
    }
    found
}

/// The device `path` is on.
pub fn volume_id(path: &Path) -> io::Result<VolumeId> {
    Ok(VolumeId(fs::metadata(path)?.dev()))
}

/// Whether names on `path`'s volume are case-sensitive.
pub fn case_rule(path: &Path) -> CaseRule {
    #[cfg(target_os = "macos")]
    return super::macos::case_rule(path);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        CaseRule::Sensitive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_path_has_no_volume() {
        assert!(volume_id(Path::new("/definitely/missing/awm")).is_err());
    }

    #[test]
    fn a_used_up_budget_is_incomplete() {
        let snapshot = ProcessSnapshot::take();
        let mapping =
            PathMapping::for_host(Path::new("/tmp"), Path::new("/tmp"), CaseRule::Sensitive)
                .unwrap();
        assert!(!open_files_under(&snapshot, &mapping, Some(Duration::ZERO)).complete);
    }

    #[test]
    fn this_process_open_files_are_readable() {
        let dir = tempfile::tempdir().unwrap();
        let file = fs::canonicalize(dir.path()).unwrap().join("open.txt");
        fs::write(&file, b"x").unwrap();
        let _open = fs::File::open(&file).unwrap();
        assert!(open_paths(std::process::id()).unwrap().contains(&file));
    }
}
