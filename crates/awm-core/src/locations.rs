//! Where things live on this machine: the home folder, Claude Code's data, VS Code's
//! user folder, and the tool's own state folder.
//!
//! Every location comes from the standard environment variables, read in one place
//! (design.md, "Key design decisions", 14). Tests point the tool at a fixture home by
//! setting the same variables, with no test-only override. The OS is a parameter, so the
//! rules for each OS can be tested on any of them.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// The name of the tool's own folder inside the OS's application-data location.
const STATE_FOLDER: &str = "agent-workspace-mover";

/// Which OS's conventions to follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Windows: `%USERPROFILE%`, `%APPDATA%`, `%LOCALAPPDATA%`.
    Windows,
    /// macOS: `~/Library/Application Support`.
    MacOs,
    /// Linux and other Unix systems: the XDG base directories.
    Linux,
}

impl Os {
    /// The OS this program runs on.
    pub fn host() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// The folders and files the tool reads and writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locations {
    /// The user's home folder.
    pub home: PathBuf,
    /// Claude Code's data folder: `$CLAUDE_CONFIG_DIR` if set, otherwise `~/.claude`.
    pub claude_dir: PathBuf,
    /// Claude Code's global config. It sits inside `$CLAUDE_CONFIG_DIR` when that's set,
    /// and at `~/.claude.json` otherwise (facts.md, F-3).
    pub claude_json: PathBuf,
    /// VS Code's user folder (`.../Code/User`).
    pub vscode_user: PathBuf,
    /// The tool's own state folder: move records, backups, the lock and the log.
    pub state: PathBuf,
}

/// Why the locations couldn't be worked out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationError {
    /// A variable the OS always sets is missing or empty.
    Missing(&'static str),
    /// A variable holds a relative path, which can't be a location.
    NotAbsolute(&'static str),
}

impl fmt::Display for LocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocationError::Missing(name) => write!(f, "the environment variable {name} isn't set"),
            LocationError::NotAbsolute(name) => {
                write!(
                    f,
                    "the environment variable {name} doesn't hold an absolute path"
                )
            }
        }
    }
}

impl std::error::Error for LocationError {}

impl Locations {
    /// The locations for this process's environment and OS.
    pub fn from_env() -> Result<Self, LocationError> {
        Self::from_vars(|name| std::env::var_os(name), Os::host())
    }

    /// The locations given a way to read environment variables, and an OS.
    pub fn from_vars(
        var: impl Fn(&str) -> Option<OsString>,
        os: Os,
    ) -> Result<Self, LocationError> {
        // An empty variable counts as unset, as shells and the OS treat it.
        let get = |name: &'static str| -> Result<Option<PathBuf>, LocationError> {
            match var(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
            {
                Some(path) if is_absolute(&path, os) => Ok(Some(path)),
                Some(_) => Err(LocationError::NotAbsolute(name)),
                None => Ok(None),
            }
        };
        let need = |name: &'static str| get(name)?.ok_or(LocationError::Missing(name));

        let home = match os {
            Os::Windows => need("USERPROFILE")?,
            Os::MacOs | Os::Linux => need("HOME")?,
        };
        let (claude_dir, claude_json) = match get("CLAUDE_CONFIG_DIR")? {
            Some(dir) => (dir.clone(), dir.join(".claude.json")),
            None => (home.join(".claude"), home.join(".claude.json")),
        };
        let (vscode_parent, state_parent) = match os {
            Os::Windows => (need("APPDATA")?, need("LOCALAPPDATA")?),
            Os::MacOs => {
                let support = home.join("Library").join("Application Support");
                (support.clone(), support)
            }
            Os::Linux => (
                get("XDG_CONFIG_HOME")?.unwrap_or_else(|| home.join(".config")),
                get("XDG_STATE_HOME")?.unwrap_or_else(|| home.join(".local").join("state")),
            ),
        };
        Ok(Self {
            home,
            claude_dir,
            claude_json,
            vscode_user: vscode_parent.join("Code").join("User"),
            state: state_parent.join(STATE_FOLDER),
        })
    }
}

/// Whether `path` is absolute under `os`'s rules, whatever OS this runs on.
fn is_absolute(path: &std::path::Path, os: Os) -> bool {
    let text = path.to_string_lossy();
    match os {
        Os::Windows => {
            let bytes = text.as_bytes();
            (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/'))
                || text.starts_with("\\\\")
        }
        Os::MacOs | Os::Linux => text.starts_with('/'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Locations for `os` with exactly the variables in `vars` set.
    fn locations(os: Os, vars: &[(&str, &str)]) -> Result<Locations, LocationError> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        Locations::from_vars(|name| vars.get(name).cloned(), os)
    }

    #[test]
    fn windows_uses_the_profile_and_app_data_folders() {
        let l = locations(
            Os::Windows,
            &[
                ("USERPROFILE", r"C:\Users\me"),
                ("APPDATA", r"C:\Users\me\AppData\Roaming"),
                ("LOCALAPPDATA", r"C:\Users\me\AppData\Local"),
            ],
        )
        .unwrap();
        assert_eq!(l.home, PathBuf::from(r"C:\Users\me"));
        assert_eq!(l.claude_dir, PathBuf::from(r"C:\Users\me").join(".claude"));
        assert_eq!(
            l.claude_json,
            PathBuf::from(r"C:\Users\me").join(".claude.json")
        );
        assert_eq!(
            l.vscode_user,
            PathBuf::from(r"C:\Users\me\AppData\Roaming")
                .join("Code")
                .join("User")
        );
        assert_eq!(
            l.state,
            PathBuf::from(r"C:\Users\me\AppData\Local").join("agent-workspace-mover")
        );
    }

    #[test]
    fn macos_uses_application_support() {
        let l = locations(Os::MacOs, &[("HOME", "/Users/me")]).unwrap();
        let support = PathBuf::from("/Users/me")
            .join("Library")
            .join("Application Support");
        assert_eq!(l.vscode_user, support.join("Code").join("User"));
        assert_eq!(l.state, support.join("agent-workspace-mover"));
    }

    #[test]
    fn linux_follows_xdg_with_its_defaults() {
        let defaults = locations(Os::Linux, &[("HOME", "/home/me")]).unwrap();
        assert_eq!(
            defaults.vscode_user,
            PathBuf::from("/home/me")
                .join(".config")
                .join("Code")
                .join("User")
        );
        assert_eq!(
            defaults.state,
            PathBuf::from("/home/me")
                .join(".local")
                .join("state")
                .join("agent-workspace-mover")
        );
        let set = locations(
            Os::Linux,
            &[
                ("HOME", "/home/me"),
                ("XDG_CONFIG_HOME", "/cfg"),
                ("XDG_STATE_HOME", "/st"),
            ],
        )
        .unwrap();
        assert_eq!(
            set.vscode_user,
            PathBuf::from("/cfg").join("Code").join("User")
        );
        assert_eq!(
            set.state,
            PathBuf::from("/st").join("agent-workspace-mover")
        );
    }

    // F-3: with CLAUDE_CONFIG_DIR set, the global config moves inside it too.
    #[test]
    fn claude_config_dir_holds_the_global_config_when_set() {
        let l = locations(
            Os::Linux,
            &[
                ("HOME", "/home/me"),
                ("CLAUDE_CONFIG_DIR", "/elsewhere/claude"),
            ],
        )
        .unwrap();
        assert_eq!(l.claude_dir, PathBuf::from("/elsewhere/claude"));
        assert_eq!(
            l.claude_json,
            PathBuf::from("/elsewhere/claude").join(".claude.json")
        );
        let empty = locations(
            Os::Linux,
            &[("HOME", "/home/me"), ("CLAUDE_CONFIG_DIR", "")],
        )
        .unwrap();
        assert_eq!(
            empty.claude_dir,
            PathBuf::from("/home/me").join(".claude"),
            "an empty variable counts as unset"
        );
    }

    #[test]
    fn missing_or_relative_variables_are_errors() {
        assert_eq!(
            locations(Os::Linux, &[]),
            Err(LocationError::Missing("HOME"))
        );
        assert_eq!(
            locations(Os::Windows, &[("USERPROFILE", r"C:\me")]),
            Err(LocationError::Missing("APPDATA"))
        );
        assert_eq!(
            locations(Os::Linux, &[("HOME", "relative")]),
            Err(LocationError::NotAbsolute("HOME"))
        );
        assert_eq!(
            locations(Os::Linux, &[("HOME", "/h"), ("XDG_STATE_HOME", "st")]),
            Err(LocationError::NotAbsolute("XDG_STATE_HOME"))
        );
        assert!(LocationError::Missing("HOME").to_string().contains("HOME"));
        assert!(
            LocationError::NotAbsolute("HOME")
                .to_string()
                .contains("absolute")
        );
    }

    #[test]
    fn the_host_environment_resolves() {
        assert!(Locations::from_env().is_ok());
        let _ = Os::host();
    }
}
