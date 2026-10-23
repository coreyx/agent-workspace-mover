//! A snapshot of running processes: who they are, where they run, and who started them.
//!
//! Taken once per plan through `sysinfo`, which on Windows reads another process's
//! working folder from its process environment block (tech.md). Command-line arguments
//! can carry secrets, so they're held as [`AgentBytes`]: adapters can match them
//! (facts.md, F-5), and nothing can print them (requirements.md, 20.0.5).

use std::collections::HashSet;
use std::path::PathBuf;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::sensitive::AgentBytes;

/// One process, as the snapshot saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Its process id.
    pub pid: u32,
    /// When it started, in seconds since the Unix epoch. With `pid`, this identifies a
    /// process even after its id is reused.
    pub start_time: u64,
    /// Its program's name, such as `claude.exe`.
    pub name: String,
    /// Its executable's path, when readable.
    pub exe: Option<PathBuf>,
    /// Its command-line arguments, which may hold secrets.
    pub args: Vec<AgentBytes>,
    /// The process that started it, when known.
    pub parent: Option<u32>,
    /// Its working folder, or `None` when it can't be read (an elevated or protected
    /// process, or another user's).
    pub cwd: Option<PathBuf>,
}

impl ProcessInfo {
    /// The pair that identifies this process even after its id is reused.
    pub fn reference(&self) -> ProcessRef {
        ProcessRef {
            pid: self.pid,
            start_time: self.start_time,
        }
    }
}

/// A process id with its start time, which together identify one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ProcessRef {
    /// The process id.
    pub pid: u32,
    /// When it started, in seconds since the Unix epoch.
    pub start_time: u64,
}

/// The processes running at one moment.
#[derive(Clone, Debug, Default)]
pub struct ProcessSnapshot {
    processes: Vec<ProcessInfo>,
    own_pid: u32,
}

impl ProcessSnapshot {
    /// Takes a snapshot of every process on the machine.
    pub fn take() -> Self {
        let mut system = System::new();
        let kind = ProcessRefreshKind::nothing()
            .with_cwd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always);
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let processes = system
            .processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                start_time: p.start_time(),
                name: p.name().to_string_lossy().into_owned(),
                exe: p.exe().map(PathBuf::from),
                args: p
                    .cmd()
                    .iter()
                    .map(|arg| AgentBytes::new(arg.to_string_lossy().into_owned().into_bytes()))
                    .collect(),
                parent: p.parent().map(|pid| pid.as_u32()),
                // sysinfo reports an empty path when the working folder can't be read.
                cwd: p
                    .cwd()
                    .filter(|cwd| !cwd.as_os_str().is_empty())
                    .map(PathBuf::from),
            })
            .collect();
        Self {
            processes,
            own_pid: std::process::id(),
        }
    }

    /// A snapshot made of `processes`, as seen by a process whose id is `own_pid`. For
    /// tests that need exact control over what's running.
    pub fn from_processes(processes: Vec<ProcessInfo>, own_pid: u32) -> Self {
        Self { processes, own_pid }
    }

    /// Every process in the snapshot.
    pub fn iter(&self) -> impl Iterator<Item = &ProcessInfo> {
        self.processes.iter()
    }

    /// The process with id `pid`, if it was running.
    pub fn get(&self, pid: u32) -> Option<&ProcessInfo> {
        self.processes.iter().find(|p| p.pid == pid)
    }

    /// Whether the process `reference` names is still the one running under its id.
    pub fn is_running(&self, reference: ProcessRef) -> bool {
        self.get(reference.pid)
            .is_some_and(|p| p.start_time == reference.start_time)
    }

    /// This process and every process that started it, nearest first. Used to tell
    /// when the tool is running inside the session that's holding a folder (20.0.3).
    pub fn own_chain(&self) -> Vec<u32> {
        let mut chain = vec![self.own_pid];
        let mut seen: HashSet<u32> = chain.iter().copied().collect();
        let mut current = self.own_pid;
        while let Some(parent) = self.get(current).and_then(|p| p.parent) {
            // A parent can appear twice when ids are reused; stop rather than loop.
            if !seen.insert(parent) || self.get(parent).is_none() {
                break;
            }
            chain.push(parent);
            current = parent;
        }
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process with `pid` and `parent`, and nothing else of interest.
    fn process(pid: u32, parent: Option<u32>) -> ProcessInfo {
        ProcessInfo {
            pid,
            start_time: 100 + u64::from(pid),
            name: format!("p{pid}"),
            exe: None,
            args: vec![],
            parent,
            cwd: None,
        }
    }

    #[test]
    fn the_host_snapshot_includes_this_process_and_its_parent() {
        let snapshot = ProcessSnapshot::take();
        let me = snapshot
            .get(std::process::id())
            .expect("this process is listed");
        assert!(
            me.cwd.is_some(),
            "this process's working folder is readable"
        );
        assert!(me.exe.is_some());
        let chain = snapshot.own_chain();
        assert_eq!(chain[0], std::process::id());
        assert!(chain.len() >= 2, "the test runner's parent is listed too");
        assert!(snapshot.is_running(me.reference()));
        assert!(!snapshot.is_running(ProcessRef {
            pid: me.pid,
            start_time: me.start_time + 1
        }));
    }

    #[test]
    fn the_own_chain_follows_parents_and_stops_at_loops_or_gaps() {
        let snapshot = ProcessSnapshot::from_processes(
            vec![
                process(10, Some(5)),
                process(5, Some(1)),
                process(1, None),
                process(20, Some(21)),
                process(21, Some(20)),
            ],
            10,
        );
        assert_eq!(snapshot.own_chain(), [10, 5, 1]);
        assert_eq!(
            ProcessSnapshot::from_processes(vec![process(20, Some(21)), process(21, Some(20))], 20)
                .own_chain(),
            [20, 21]
        );
        assert_eq!(
            ProcessSnapshot::from_processes(vec![process(7, Some(99))], 7).own_chain(),
            [7]
        );
        assert_eq!(snapshot.iter().count(), 5);
    }

    // 20.0.5: nothing a process was started with reaches output.
    #[test]
    fn arguments_never_appear_in_debug_output() {
        let dir = tempfile::tempdir().unwrap();
        let fake = awm_testkit::FakeProcess::start(dir.path(), None).unwrap();
        let snapshot = ProcessSnapshot::take();
        let info = snapshot
            .get(fake.pid())
            .expect("the fake process is listed");
        // The fake process is started with "awm_fake_process" as an argument.
        assert!(
            info.args
                .iter()
                .any(|arg| arg.as_bytes() == b"awm_fake_process")
        );
        assert!(
            !format!("{info:?}").contains("awm_fake_process"),
            "an argument appeared in debug output"
        );
    }
}
