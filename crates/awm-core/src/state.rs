//! The tool's own state folder, and the lock that keeps two runs from changing things
//! at once (design.md, "On disk").
//!
//! The folder holds one subfolder per move (`moves/<move id>/`), finish-later state
//! (`pending/`), the log (`log/`) and the lock file. Everything in it is readable by the
//! current user only (requirements.md, 12.0.4). Mutating commands hold the lock for
//! their whole run; read-only ones never take it, so a dry run works while a move runs.

use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use crate::platform;

/// The tool's state folder.
#[derive(Clone, Debug)]
pub struct StateDir {
    root: PathBuf,
}

/// Holds the machine-wide lock until dropped.
#[derive(Debug)]
pub struct StateLock {
    _file: File,
}

/// Why the lock couldn't be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another run of the tool holds it.
    Busy,
    /// The lock file couldn't be opened or locked.
    Io(io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Busy => write!(
                f,
                "another run of agent-workspace-mover is changing things; try again when it finishes"
            ),
            LockError::Io(error) => write!(f, "the state folder's lock couldn't be taken: {error}"),
        }
    }
}

impl std::error::Error for LockError {}

impl StateDir {
    /// Opens the state folder at `root`, creating it and its subfolders, all restricted
    /// to the current user, if they don't exist yet.
    pub fn open(root: &Path) -> io::Result<Self> {
        let state = Self {
            root: root.to_path_buf(),
        };
        for folder in [
            state.root.clone(),
            state.moves_dir(),
            state.pending_dir(),
            state.log_dir(),
        ] {
            fs::create_dir_all(&folder)?;
            platform::restrict_to_user(&folder)?;
        }
        Ok(state)
    }

    /// The state folder itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where move records live, one folder per move.
    pub fn moves_dir(&self) -> PathBuf {
        self.root.join("moves")
    }

    /// Where finish-later state lives while a move waits.
    pub fn pending_dir(&self) -> PathBuf {
        self.root.join("pending")
    }

    /// Where the daily log files live.
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("log")
    }

    /// Takes the machine-wide lock, or reports that another run holds it. It isn't
    /// waited for: a second run is refused, not queued.
    pub fn try_lock(&self) -> Result<StateLock, LockError> {
        let path = self.root.join("lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(LockError::Io)?;
        platform::restrict_to_user(&path).map_err(LockError::Io)?;
        match file.try_lock() {
            Ok(()) => Ok(StateLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::Busy),
            Err(TryLockError::Error(error)) => Err(LockError::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// The environment variable that tells `lock_probe` which state folder to try.
    const PROBE_VAR: &str = "AWM_LOCK_PROBE";

    #[test]
    fn creates_its_folders_restricted_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(&dir.path().join("state")).unwrap();
        for folder in [
            state.root().to_path_buf(),
            state.moves_dir(),
            state.pending_dir(),
            state.log_dir(),
        ] {
            assert!(folder.is_dir());
            assert!(
                platform::is_user_only(&folder).unwrap(),
                "{} is user-only",
                folder.display()
            );
        }
        StateDir::open(state.root()).expect("opening it again is fine");
    }

    #[test]
    fn the_lock_is_exclusive_within_a_process() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(dir.path()).unwrap();
        let held = state.try_lock().unwrap();
        assert!(matches!(state.try_lock(), Err(LockError::Busy)));
        assert!(LockError::Busy.to_string().contains("another run"));
        drop(held);
        state.try_lock().expect("free again once released");
    }

    /// Runs `lock_probe` in a separate process against `root`, returning what it saw.
    fn probe_from_another_process(root: &Path) -> String {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::tests::lock_probe",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PROBE_VAR, root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn the_lock_is_exclusive_between_processes() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(dir.path()).unwrap();
        let held = state.try_lock().unwrap();
        assert!(probe_from_another_process(dir.path()).contains("probe: busy"));
        drop(held);
        assert!(probe_from_another_process(dir.path()).contains("probe: acquired"));
    }

    /// Not a test on its own: `the_lock_is_exclusive_between_processes` runs it in a
    /// child process to try the lock from outside.
    #[test]
    #[ignore = "run by the_lock_is_exclusive_between_processes"]
    fn lock_probe() {
        let Some(root) = std::env::var_os(PROBE_VAR) else {
            return;
        };
        match StateDir::open(Path::new(&root)).unwrap().try_lock() {
            Ok(_) => println!("probe: acquired"),
            Err(LockError::Busy) => println!("probe: busy"),
            Err(LockError::Io(error)) => println!("probe: error {error}"),
        }
    }

    #[test]
    fn an_unusable_lock_file_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(dir.path()).unwrap();
        fs::create_dir(dir.path().join("lock")).unwrap();
        let error = state.try_lock().unwrap_err();
        assert!(matches!(error, LockError::Io(_)));
        assert!(error.to_string().contains("couldn't be taken"));
    }
}
