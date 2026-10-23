//! Writing files so they're never left half-written (requirements.md, 12.0.3).
//!
//! A file is written to a temporary file in the same folder, flushed to disk, and only
//! then renamed over the original. A rename within one folder is atomic, so a reader,
//! or the tool after a crash, sees either the old content or the new, never a mix.

use std::io::{self, Write};
use std::path::Path;

/// Replaces `path` with `bytes` atomically, creating it if it doesn't exist.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, bytes, || Ok(()))
}

/// Like [`write`], but runs `before_replace` after the new content is on disk and
/// before it replaces the original. An error from it abandons the write, leaving the
/// original untouched and no temporary file behind. Tests use it to simulate a crash
/// at the last moment.
pub fn write_with(
    path: &Path,
    bytes: &[u8],
    before_replace: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".awm-")
        .suffix(".tmp")
        .tempfile_in(folder)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    before_replace()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_folder(folder)
}

/// Flushes a folder's entries to disk, so a completed rename survives a power cut.
/// Windows has no equivalent call for folders; its file system journals the rename.
fn sync_folder(folder: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(folder)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("record.json");
        write(&file, b"first").unwrap();
        write(&file, b"second").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"second");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temporary file is left"
        );
    }

    // A crash just before the replace leaves the original as it was.
    #[test]
    fn an_abandoned_write_leaves_the_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("record.json");
        write(&file, b"original").unwrap();
        let crashed = write_with(&file, b"half", || Err(io::Error::other("simulated crash")));
        assert!(crashed.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "the temporary file is cleaned up"
        );
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write(&dir.path().join("missing").join("x.json"), b"x").is_err());
    }

    #[test]
    fn a_bare_file_name_writes_to_the_current_folder() {
        // Only checks the folder is worked out; writing in the test's own folder would
        // leave a file behind, so the write is abandoned.
        let abandoned = write_with(Path::new("awm-atomic-test.tmp"), b"x", || {
            Err(io::Error::other("stop"))
        });
        assert!(abandoned.is_err());
        assert!(!Path::new("awm-atomic-test.tmp").exists());
    }
}
