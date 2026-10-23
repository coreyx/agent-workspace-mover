//! The only code that calls OS-specific interfaces (design.md, "Platform layer").
//!
//! For now this holds the user-only permissions that backups and records need
//! (requirements.md, 12.0.4). TASK-8 adds processes, holders and volumes, behind the
//! `Platform` trait design.md describes.

use std::io;
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

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
        restrict_to_user(&folder).unwrap();
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
    }
}
