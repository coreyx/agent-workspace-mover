//! Open files, volumes and creation times on Windows.

// Unsafe code is denied workspace-wide; this module calls the Restart Manager and
// file-information functions.
#![allow(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{FileTimesExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, GetDiskFreeSpaceExW, GetFileInformationByHandle,
};
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmStartSession,
};
use windows::core::{PCWSTR, PWSTR};

use super::VolumeId;
use super::holders::OpenFiles;

/// The most files registered in one Restart Manager session. The Restart Manager
/// silently reports no holders at 65,536 files and crashes the caller above that
/// (facts.md, F-8), so this stays well under.
pub const MAX_SESSION_FILES: usize = 50_000;

/// The Restart Manager's largest safe session (facts.md, F-8).
const RESTART_MANAGER_LIMIT: usize = 65_535;

// A session limit at or over the Restart Manager's own doesn't compile.
const _: () = assert!(MAX_SESSION_FILES < RESTART_MANAGER_LIMIT);

/// Lets a directory handle be opened, which Windows otherwise refuses.
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// The access right needed to change a file's times.
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;

/// Every file under `root`, without following links or junctions, as wide strings.
/// Returns false as the second value when `deadline` passed before the walk finished.
fn files_under(root: &Path, deadline: Option<Instant>) -> (Vec<Vec<u16>>, bool) {
    let mut files = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return (files, false);
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path: PathBuf = entry.path();
            if kind.is_dir() {
                folders.push(path);
            } else {
                files.push(path.as_os_str().encode_wide().chain(Some(0)).collect());
            }
        }
    }
    (files, true)
}

/// Asks the Restart Manager which processes hold any of `files`, in one session.
fn holders_of(files: &[Vec<u16>]) -> Result<Vec<u32>, ()> {
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        if RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
            return Err(());
        }
        let result = (|| {
            let names: Vec<PCWSTR> = files.iter().map(|w| PCWSTR(w.as_ptr())).collect();
            if RmRegisterResources(session, Some(&names), None, None) != ERROR_SUCCESS {
                return Err(());
            }
            let (mut needed, mut returned, mut reasons) = (0u32, 0u32, 0u32);
            let first = RmGetList(session, &mut needed, &mut returned, None, &mut reasons);
            if first == ERROR_SUCCESS {
                return Ok(vec![]);
            }
            if first != ERROR_MORE_DATA {
                return Err(());
            }
            // Processes can open files between the two calls, so leave room for more.
            let mut infos = vec![RM_PROCESS_INFO::default(); needed as usize + 8];
            returned = infos.len() as u32;
            if RmGetList(
                session,
                &mut needed,
                &mut returned,
                Some(infos.as_mut_ptr()),
                &mut reasons,
            ) != ERROR_SUCCESS
            {
                return Err(());
            }
            Ok(infos[..returned as usize]
                .iter()
                .map(|info| info.Process.dwProcessId)
                .collect())
        })();
        let _ = RmEndSession(session);
        result
    }
}

/// Processes with a file under `root` open, from the Restart Manager, within `budget`.
pub fn open_files_under(root: &Path, budget: Option<Duration>) -> OpenFiles {
    let deadline = budget.map(|b| Instant::now() + b);
    let (files, walked) = files_under(root, deadline);
    let mut found = OpenFiles {
        complete: walked,
        ..OpenFiles::default()
    };
    for chunk in files.chunks(MAX_SESSION_FILES) {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            found.complete = false;
            break;
        }
        match holders_of(chunk) {
            Ok(pids) => {
                for pid in pids {
                    if !found.pids.contains(&pid) {
                        found.pids.push(pid);
                    }
                }
            }
            Err(()) => found.complete = false,
        }
    }
    found
}

/// Opens `path` (a file or a folder) with `access`, for reading its information.
fn open_handle(path: &Path, access: u32) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

/// The serial number of the volume `path` is on.
pub fn volume_id(path: &Path) -> io::Result<VolumeId> {
    let file = open_handle(path, 0)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(io::Error::other)?;
    Ok(VolumeId(u64::from(info.dwVolumeSerialNumber)))
}

/// The bytes available to the current user on `path`'s volume, allowing for quotas.
pub fn available_space(path: &Path) -> io::Result<u64> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(name.as_ptr()), Some(&mut available), None, None) }
        .map_err(io::Error::other)?;
    Ok(available)
}

/// Sets `path`'s creation time, for a file or a folder.
pub fn set_creation_time(path: &Path, time: SystemTime) -> io::Result<()> {
    open_handle(path, FILE_WRITE_ATTRIBUTES)?.set_times(fs::FileTimes::new().set_created(time))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_used_up_budget_is_incomplete_never_empty_handed() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::write(dir.path().join(format!("{i}.txt")), b"x").unwrap();
        }
        let found = open_files_under(dir.path(), Some(Duration::ZERO));
        assert!(!found.complete);
    }

    #[test]
    fn walks_nested_folders() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a").join("b")).unwrap();
        fs::write(dir.path().join("a").join("b").join("f.txt"), b"x").unwrap();
        fs::write(dir.path().join("top.txt"), b"x").unwrap();
        let (files, complete) = files_under(dir.path(), None);
        assert_eq!(files.len(), 2);
        assert!(complete);
    }

    #[test]
    fn a_missing_path_has_no_volume() {
        assert!(volume_id(Path::new(r"C:\definitely\missing\awm")).is_err());
    }
}
