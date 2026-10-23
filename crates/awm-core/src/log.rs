//! The tool's log, which can only be given things that are safe to keep.
//!
//! Agent data holds conversations and secrets (requirements.md, REQ-26), so the log
//! doesn't accept arbitrary text. Each entry is an event name and a list of fields,
//! and a field's value is a path, a count, a stable code or a move id. The event name
//! and codes are `&'static str`, written into the tool's own source, so nothing read at
//! run time can reach the log as text (design.md, "Errors and logging").

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use crate::platform;
use crate::record::MoveId;
use crate::state::StateDir;
use crate::timestamp::Utc;

/// A value a log entry may hold.
#[derive(Clone, Copy, Debug)]
pub enum LogValue<'a> {
    /// A file or folder.
    Path(&'a Path),
    /// A count, such as the number of sessions moved.
    Count(u64),
    /// A stable code from the tool's own source, such as a blocker's code.
    Code(&'static str),
    /// A move's id.
    Move(&'a MoveId),
}

/// Appends entries to the day's log file, `log/awm-YYYYMMDD.log`.
#[derive(Debug)]
pub struct Logger {
    file: Mutex<File>,
}

impl Logger {
    /// Opens today's log file in `state`, restricted to the current user.
    pub fn open(state: &StateDir) -> io::Result<Self> {
        let path = state
            .log_dir()
            .join(format!("awm-{}.log", Utc::now().date()));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        platform::restrict_to_user(&path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    /// Writes one entry: the time, `event`, and each field as `name=value`.
    pub fn event(
        &self,
        event: &'static str,
        fields: &[(&'static str, LogValue<'_>)],
    ) -> io::Result<()> {
        let mut line = format!("{} {event}", Utc::now().rfc3339());
        for (name, value) in fields {
            line.push(' ');
            line.push_str(name);
            line.push('=');
            match value {
                LogValue::Path(path) => line.push_str(&format!("{:?}", path.display().to_string())),
                LogValue::Count(count) => line.push_str(&count.to_string()),
                LogValue::Code(code) => line.push_str(code),
                LogValue::Move(id) => line.push_str(id.as_str()),
            }
        }
        line.push('\n');
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        file.write_all(line.as_bytes())?;
        file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_entries_with_only_safe_values() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(dir.path()).unwrap();
        let logger = Logger::open(&state).unwrap();
        let id = MoveId::parse("20261023-095701-abcd").unwrap();
        logger
            .event(
                "move.planned",
                &[
                    ("source", LogValue::Path(Path::new("/a/my repo"))),
                    ("sessions", LogValue::Count(3)),
                    ("blocker", LogValue::Code("agent-running")),
                    ("move", LogValue::Move(&id)),
                ],
            )
            .unwrap();
        logger.event("move.done", &[]).unwrap();
        let name = format!("awm-{}.log", Utc::now().date());
        let text = std::fs::read_to_string(state.log_dir().join(&name)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with(r#" move.planned source="/a/my repo" sessions=3 blocker=agent-running move=20261023-095701-abcd"#));
        assert!(lines[1].ends_with(" move.done"));
        assert!(platform::is_user_only(&state.log_dir().join(name)).unwrap());
    }
}
