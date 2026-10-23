//! User-only permissions on macOS and Linux, through file modes.

use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Sets mode `0700` on a folder, so only its owner can list or enter it, and `0600` on
/// a file.
pub fn restrict_to_user(path: &Path) -> io::Result<()> {
    let mode = if fs::metadata(path)?.is_dir() {
        0o700
    } else {
        0o600
    };
    fs::set_permissions(path, Permissions::from_mode(mode))
}

/// Whether no one but the owner has any access to `path`.
pub fn is_user_only(path: &Path) -> io::Result<bool> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o077 == 0)
}
