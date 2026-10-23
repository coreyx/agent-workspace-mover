//! Test helpers for agent-workspace-mover. Never shipped.
//!
//! Every test that runs the tool does so against a [`FixtureHome`]: a throwaway folder
//! standing in for the user's home, application-data and XDG folders. The tool finds
//! every location it reads from these environment variables (design.md, "Key design
//! decisions", 14), so pointing them at a fixture is enough to keep tests away from
//! real agent data.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// The environment variables that tell the tool where the user's folders are. A fixture
/// sets each of them for the child process.
pub const LOCATION_VARS: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
];

/// Variables a fixture removes from the child process, because each could lead it back
/// to the real home folder. `HOMEDRIVE` and `HOMEPATH` rebuild the Windows home folder;
/// `CLAUDE_CONFIG_DIR` relocates Claude Code's data, and is set again only when a test
/// asks for it with [`FixtureHome::with_claude_config_dir`].
pub const REMOVED_VARS: &[&str] = &["HOMEDRIVE", "HOMEPATH", "CLAUDE_CONFIG_DIR"];

/// A throwaway home for one test, deleted when dropped.
///
/// Its layout mirrors the real thing closely enough for every adapter: a home folder,
/// Windows' roaming and local application-data folders, and the four XDG folders.
pub struct FixtureHome {
    root: TempDir,
    claude_config_dir: Option<PathBuf>,
}

impl FixtureHome {
    /// Creates an empty fixture home with every location folder present.
    pub fn new() -> io::Result<Self> {
        let root = tempfile::Builder::new().prefix("awm-fixture-").tempdir()?;
        let fixture = Self {
            root,
            claude_config_dir: None,
        };
        for (_, path) in fixture.locations() {
            std::fs::create_dir_all(path)?;
        }
        Ok(fixture)
    }

    /// Sets `CLAUDE_CONFIG_DIR` for the child process to a folder inside the fixture,
    /// for tests of a relocated Claude Code data folder (requirements.md, 6.6).
    pub fn with_claude_config_dir(mut self) -> io::Result<Self> {
        let dir = self.root.path().join("claude-config");
        std::fs::create_dir_all(&dir)?;
        self.claude_config_dir = Some(dir);
        Ok(self)
    }

    /// The fixture's root folder. Every location it sets is inside it.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// The folder standing in for the user's home.
    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    /// Each location variable paired with the folder this fixture sets it to.
    pub fn locations(&self) -> Vec<(&'static str, PathBuf)> {
        let root = self.root.path();
        let home = self.home();
        let mut locations = vec![
            ("HOME", home.clone()),
            ("USERPROFILE", home),
            ("APPDATA", root.join("appdata").join("roaming")),
            ("LOCALAPPDATA", root.join("appdata").join("local")),
            ("XDG_CONFIG_HOME", root.join("xdg").join("config")),
            ("XDG_DATA_HOME", root.join("xdg").join("data")),
            ("XDG_STATE_HOME", root.join("xdg").join("state")),
            ("XDG_CACHE_HOME", root.join("xdg").join("cache")),
        ];
        if let Some(dir) = &self.claude_config_dir {
            locations.push(("CLAUDE_CONFIG_DIR", dir.clone()));
        }
        locations
    }

    /// Points `command` at this fixture: sets every location variable, and removes the
    /// variables in [`REMOVED_VARS`] that this fixture doesn't set itself.
    pub fn apply<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        for name in REMOVED_VARS {
            command.env_remove(name);
        }
        for (name, path) in self.locations() {
            command.env(name, path);
        }
        command
    }
}

/// Fake processes that hold a folder, for tests of finding what's holding it.
pub mod fake_process;
pub use fake_process::FakeProcess;
