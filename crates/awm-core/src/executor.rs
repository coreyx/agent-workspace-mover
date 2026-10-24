//! The executor: the only code that changes the machine (design.md, "Key design
//! decisions", 1, 3, 4, 6 and 12).
//!
//! It carries out an approved [`Plan`] in this order:
//!
//! 1. **Recheck** every action's preconditions. Anything changed since planning stops the
//!    run before any change.
//! 2. **Back up** every file a later step will edit, and verify each copy (12.0.1).
//! 3. **Run the actions** in phase order. Each step is recorded as started before it
//!    changes anything and as done after, with what undo will need (decision 4). Its
//!    preconditions are checked again just before it runs.
//!
//! The workspace folder moves first, because on Windows it's the step most likely to
//! fail, and failing there leaves nothing changed (decision 3). On one volume it moves
//! with a single rename. Across volumes it's copied to a hidden sibling, verified file by
//! file, renamed into place and only then removed (decision 12).
//!
//! Every action type can tell from the machine whether it has happened ([`probe`]),
//! which `resume` and `rollback` rely on (REQ-13).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::plan::{Action, Outcome, Plan, PlannedAction, Precondition};
use crate::platform::Platform;
use crate::record::{BackupEntry, MoveRecord, MoveState, StepRecord, StepStatus, hash_file};
use crate::timestamp::Utc;

/// Why a step didn't complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// Something holds the folder, so it can't be renamed (Windows).
    Held,
    /// A precondition no longer holds: something changed since planning.
    PreconditionChanged,
    /// A cross-volume copy didn't match its source.
    VerifyFailed,
    /// The action isn't one this version can carry out.
    Unsupported,
    /// Any other I/O failure.
    Io,
}

/// A step that didn't complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepFailure {
    /// The step's number, from 1.
    pub index: u32,
    /// Why.
    pub kind: FailureKind,
    /// What happened, in the tool's own words.
    pub message: String,
}

/// How an apply ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// How it ended, which decides the exit code.
    pub outcome: Outcome,
    /// How many steps completed.
    pub steps_done: u32,
    /// The step that failed, if one did.
    pub failure: Option<StepFailure>,
}

/// What [`probe`] finds about an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeState {
    /// Nothing of it has happened.
    NotStarted,
    /// It has happened.
    Done,
    /// Some of it has happened.
    Partial,
}

/// A step error, before it's turned into a [`StepFailure`].
#[derive(Debug)]
struct StepError {
    kind: FailureKind,
    message: String,
}

impl StepError {
    /// An error of `kind` described by `message`.
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<io::Error> for StepError {
    fn from(error: io::Error) -> Self {
        let kind = if is_held(&error) {
            FailureKind::Held
        } else {
            FailureKind::Io
        };
        StepError::new(kind, error.to_string())
    }
}

/// Whether `error` means something holds the file or folder. On Windows a rename of a
/// folder that a process works in, or has a file open in, fails with a sharing
/// violation or access denied.
fn is_held(error: &io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32))
}

/// Whether `error` means a rename crossed a volume boundary, so it must be a copy.
fn crosses_volumes(error: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    matches!(error.raw_os_error(), Some(code) if (cfg!(unix) && code == 18) || (cfg!(windows) && code == 17))
}

/// Carries out plans against one move's record.
pub struct Executor<'a> {
    platform: &'a dyn Platform,
    record: &'a MoveRecord,
}

impl<'a> Executor<'a> {
    /// An executor that records into `record` and reaches the OS through `platform`.
    pub fn new(platform: &'a dyn Platform, record: &'a MoveRecord) -> Self {
        Self { platform, record }
    }

    /// Carries out `plan`, which must be the approved one. Opt-in actions don't run:
    /// they need a typed confirmation, which arrives with the opt-ins themselves (REQ-8).
    pub fn apply(&self, plan: &Plan) -> io::Result<ApplyOutcome> {
        let actions: Vec<&PlannedAction> = plan
            .ordered_actions()
            .into_iter()
            .filter(|a| a.opt_in.is_none())
            .collect();
        self.record.write_plan(plan)?;

        // 1. Recheck: a change since planning stops everything, before any change.
        for (index, action) in actions.iter().enumerate() {
            if let Err(message) = check_preconditions(action) {
                return Ok(ApplyOutcome {
                    outcome: Outcome::Blocked,
                    steps_done: 0,
                    failure: Some(StepFailure {
                        index: step_number(index),
                        kind: FailureKind::PreconditionChanged,
                        message,
                    }),
                });
            }
        }
        self.record.set_state(MoveState::Applying)?;

        // 2. Back up every file a later step edits, before any step runs.
        let mut backups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut entries: Vec<BackupEntry> = Vec::new();
        for (index, action) in actions.iter().enumerate() {
            for file in files_edited(&action.action) {
                let entry = self.record.back_up(entries.len() as u32 + 1, &file)?;
                backups
                    .entry(step_number(index))
                    .or_default()
                    .push(entry.index);
                entries.push(entry);
            }
        }
        self.record.write_backups(&entries)?;
        fail::fail_point!("executor::after-backups", |_| Err(io::Error::other(
            "fail point: after backups"
        )));

        // 3. Run each action, recording it before and after.
        for (index, action) in actions.iter().enumerate() {
            let number = step_number(index);
            let mut step = StepRecord {
                index: number,
                status: StepStatus::Started,
                action: serde_json::to_value(&action.action).map_err(io::Error::other)?,
                effects: serde_json::Value::Null,
                backups: backups.get(&number).cloned().unwrap_or_default(),
                started: Utc::now().rfc3339(),
                finished: None,
            };
            if let Err(message) = check_preconditions(action) {
                return self.fail(
                    step,
                    number,
                    StepError::new(FailureKind::PreconditionChanged, message),
                );
            }
            self.record.write_step(&step)?;
            fail::fail_point!("executor::before-step", |_| Err(io::Error::other(
                "fail point: before a step"
            )));

            match self.run(&action.action) {
                Ok(effects) => {
                    step.status = StepStatus::Done;
                    step.effects = effects;
                    step.finished = Some(Utc::now().rfc3339());
                    self.record.write_step(&step)?;
                }
                Err(error) => {
                    // A copy that failed verification has already been removed, so the
                    // move rolled itself back (1.4). Nothing before it had run.
                    if error.kind == FailureKind::VerifyFailed && index == 0 {
                        step.status = StepStatus::Reversed;
                        step.finished = Some(Utc::now().rfc3339());
                        self.record.write_step(&step)?;
                        self.record.set_state(MoveState::RolledBack)?;
                        return Ok(ApplyOutcome {
                            outcome: Outcome::RolledBack,
                            steps_done: 0,
                            failure: Some(StepFailure {
                                index: number,
                                kind: error.kind,
                                message: error.message,
                            }),
                        });
                    }
                    return self.fail(step, number, error);
                }
            }
            fail::fail_point!("executor::after-step", |_| Err(io::Error::other(
                "fail point: after a step"
            )));
        }
        self.record.set_state(MoveState::Completed)?;
        Ok(ApplyOutcome {
            outcome: Outcome::Succeeded,
            steps_done: actions.len() as u32,
            failure: None,
        })
    }

    /// Records `step` as failed and the move as needing attention, or, when nothing has
    /// changed yet, as blocked.
    fn fail(
        &self,
        mut step: StepRecord,
        number: u32,
        error: StepError,
    ) -> io::Result<ApplyOutcome> {
        step.status = StepStatus::Failed;
        step.finished = Some(Utc::now().rfc3339());
        self.record.write_step(&step)?;
        let nothing_changed = number == 1
            && matches!(
                error.kind,
                FailureKind::Held | FailureKind::PreconditionChanged
            );
        let (outcome, state) = if nothing_changed {
            (Outcome::Blocked, MoveState::Planned)
        } else {
            (Outcome::NeedsAttention, MoveState::NeedsAttention)
        };
        self.record.set_state(state)?;
        Ok(ApplyOutcome {
            outcome,
            steps_done: number - 1,
            failure: Some(StepFailure {
                index: number,
                kind: error.kind,
                message: error.message,
            }),
        })
    }

    /// Carries out one action, returning what undo and rollback will need.
    fn run(&self, action: &Action) -> Result<serde_json::Value, StepError> {
        match action {
            Action::MoveTree { from, to } => self.move_tree(from, to),
            other => Err(StepError::new(
                FailureKind::Unsupported,
                format!(
                    "this version can't carry out a {} step yet",
                    action_kind(other)
                ),
            )),
        }
    }

    /// Moves the folder `from` to `to`: one rename on one volume, a verified copy across
    /// volumes.
    fn move_tree(&self, from: &Path, to: &Path) -> Result<serde_json::Value, StepError> {
        let meta = fs::symlink_metadata(from)?;
        if meta.file_type().is_symlink() {
            return Err(StepError::new(
                FailureKind::Unsupported,
                "the source is a link; renaming it would move only the link, so the plan must name the real folder",
            ));
        }
        let parent = to
            .parent()
            .ok_or_else(|| StepError::new(FailureKind::Io, "the target has no parent folder"))?;
        let same_volume = self.platform.volume_id(from)? == self.platform.volume_id(parent)?;
        if same_volume {
            match fs::rename(from, to) {
                Ok(()) => return Ok(json!({ "mode": "rename" })),
                Err(error) if crosses_volumes(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let partial = parent.join(format!(".awm-partial-{}", self.record.id().as_str()));
        let result = self.copy_verified(from, &partial);
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&partial);
            return Err(error);
        }
        fs::rename(&partial, to)?;
        fail::fail_point!("executor::before-source-removed", |_| Err(StepError::new(
            FailureKind::Io,
            "fail point: before the source is removed"
        )));
        fs::remove_dir_all(from)?;
        Ok(json!({ "mode": "copy", "partial": partial }))
    }

    /// Copies `from` to `to`, keeping modification times and the top folder's creation
    /// time, then checks every file's size and hash against the source.
    fn copy_verified(&self, from: &Path, to: &Path) -> Result<(), StepError> {
        copy_tree(from, to)?;
        if let Some(created) = self.platform.creation_time(from)? {
            match self.platform.set_creation_time(to, created) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::Unsupported => {}
                Err(error) => return Err(error.into()),
            }
        }
        fail::fail_point!("executor::copy-verify", |_| Err(StepError::new(
            FailureKind::VerifyFailed,
            "fail point: the copy doesn't match"
        )));
        verify_tree(from, to)
    }
}

/// Copies the folder `from` to `to`, which mustn't exist, keeping files' modification
/// times. Links are recreated as links where the OS allows it without extra rights.
fn copy_tree(from: &Path, to: &Path) -> Result<(), StepError> {
    fs::create_dir(to)?;
    let mut folders = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, dest)) = folders.pop() {
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let target = dest.join(entry.file_name());
            if kind.is_symlink() {
                copy_link(&entry.path(), &target)?;
            } else if kind.is_dir() {
                fs::create_dir(&target)?;
                folders.push((entry.path(), target));
            } else {
                fs::copy(entry.path(), &target)?;
                let modified = entry.metadata()?.modified()?;
                fs::OpenOptions::new()
                    .write(true)
                    .open(&target)?
                    .set_modified(modified)?;
            }
        }
    }
    Ok(())
}

/// Recreates the link at `source` at `target`.
#[cfg(unix)]
fn copy_link(source: &Path, target: &Path) -> Result<(), StepError> {
    std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
    Ok(())
}

/// Windows needs extra rights to create links, so a workspace with links inside can't
/// be copied across volumes. The copy stops before the source is touched.
#[cfg(windows)]
fn copy_link(source: &Path, _target: &Path) -> Result<(), StepError> {
    Err(StepError::new(
        FailureKind::Unsupported,
        format!(
            "{} is a link, which can't be copied to another volume without administrator rights",
            source.display()
        ),
    ))
}

/// Checks every file under `from` has a copy under `to` with the same size and hash.
fn verify_tree(from: &Path, to: &Path) -> Result<(), StepError> {
    let mut folders = vec![from.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let relative = entry
                .path()
                .strip_prefix(from)
                .expect("walked under from")
                .to_path_buf();
            let copy = to.join(&relative);
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file() {
                let matches = copy.is_file() && hash_file(&entry.path())? == hash_file(&copy)?;
                if !matches {
                    return Err(StepError::new(
                        FailureKind::VerifyFailed,
                        format!("the copy of {} doesn't match it", relative.display()),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The files an action edits, which are backed up before any step runs.
fn files_edited(action: &Action) -> Vec<PathBuf> {
    match action {
        Action::EditJsonFile { file, .. }
        | Action::SpliceJsonlField { file, .. }
        | Action::ReplaceInText { file, .. } => {
            vec![file.clone()]
        }
        Action::EditSqliteJsonRow { db, .. } => vec![db.clone()],
        Action::MoveTree { .. } | Action::MergeDir { .. } | Action::RunTool { .. } => vec![],
    }
}

/// An action's kind, as the plan's JSON names it.
fn action_kind(action: &Action) -> String {
    serde_json::to_value(action)
        .ok()
        .and_then(|v| v["kind"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "?".into())
}

/// A step's number, from 1, for the action at `index` in run order.
fn step_number(index: usize) -> u32 {
    index as u32 + 1
}

/// Whether every precondition of `action` still holds, or which one doesn't.
fn check_preconditions(action: &PlannedAction) -> Result<(), String> {
    for condition in &action.preconditions {
        let holds = match condition {
            Precondition::Absent { path } => fs::symlink_metadata(path).is_err(),
            Precondition::Present { path } => fs::symlink_metadata(path).is_ok(),
            Precondition::FileUnchanged { path, size, blake3 } => {
                matches!(hash_file(path), Ok((hash, bytes)) if bytes == *size && hash == *blake3)
            }
            Precondition::DirUnchanged { path, listing } => {
                listing_hash(path).is_ok_and(|now| now == *listing)
            }
        };
        if !holds {
            return Err(format!(
                "{} changed since the plan was made; plan again",
                describe(condition)
            ));
        }
    }
    Ok(())
}

/// What a precondition is about, for a message.
fn describe(condition: &Precondition) -> String {
    match condition {
        Precondition::Absent { path } | Precondition::Present { path } => {
            path.display().to_string()
        }
        Precondition::FileUnchanged { path, .. } | Precondition::DirUnchanged { path, .. } => {
            path.display().to_string()
        }
    }
}

/// A hash of a folder's direct entries: their names, sizes and modification times. A
/// [`Precondition::DirUnchanged`] compares it before and after planning.
pub fn listing_hash(folder: &Path) -> io::Result<String> {
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        let modified = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        entries.push((
            entry.file_name().to_string_lossy().into_owned(),
            meta.len(),
            modified,
        ));
    }
    entries.sort();
    let mut hasher = blake3::Hasher::new();
    for (name, len, modified) in entries {
        hasher.update(name.as_bytes());
        hasher.update(&[0]);
        hasher.update(&len.to_le_bytes());
        hasher.update(&modified.to_le_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Whether `action` has happened, from the machine's state. `partial` is where a
/// cross-volume copy of the move with this id would be.
pub fn probe(action: &Action, move_id: &str) -> io::Result<ProbeState> {
    match action {
        Action::MoveTree { from, to } => {
            let partial = to
                .parent()
                .map(|p| p.join(format!(".awm-partial-{move_id}")));
            let (source, target) = (
                fs::symlink_metadata(from).is_ok(),
                fs::symlink_metadata(to).is_ok(),
            );
            Ok(
                match (source, target, partial.is_some_and(|p| p.exists())) {
                    (_, _, true) => ProbeState::Partial,
                    (true, false, false) => ProbeState::NotStarted,
                    (false, true, false) => ProbeState::Done,
                    // Both present: the copy is in place and the source not yet fully removed.
                    (true, true, false) => ProbeState::Partial,
                    (false, false, false) => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "neither the source nor the target exists",
                        ));
                    }
                },
            )
        }
        other => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("can't probe a {} step yet", action_kind(other)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{
        AdapterInfo, AdapterKind, AdapterSection, JsonEdit, PLAN_SCHEMA, Phase, PlanCommand,
        PlanHolders, VolumeRelation,
    };
    use crate::platform::{Host, ProcessSnapshot, VolumeId};
    use crate::record::MoveKind;
    use crate::state::StateDir;
    use std::time::{Duration, SystemTime};

    /// The host, except that `different` is reported on its own volume, to drive the
    /// cross-volume path without a second disk.
    struct SplitVolumes {
        different: PathBuf,
    }

    impl Platform for SplitVolumes {
        fn processes(&self) -> ProcessSnapshot {
            Host.processes()
        }
        fn holders(
            &self,
            s: &ProcessSnapshot,
            root: &Path,
            budget: Option<Duration>,
        ) -> crate::platform::HolderReport {
            Host.holders(s, root, budget)
        }
        fn volume_id(&self, path: &Path) -> io::Result<VolumeId> {
            if path.starts_with(&self.different) {
                Ok(VolumeId(u64::MAX))
            } else {
                Host.volume_id(path)
            }
        }
        fn case_rule(&self, path: &Path) -> crate::paths::CaseRule {
            Host.case_rule(path)
        }
        fn creation_time(&self, path: &Path) -> io::Result<Option<SystemTime>> {
            Host.creation_time(path)
        }
        fn set_creation_time(&self, path: &Path, time: SystemTime) -> io::Result<()> {
            Host.set_creation_time(path, time)
        }
        fn restrict_to_user(&self, path: &Path) -> io::Result<()> {
            Host.restrict_to_user(path)
        }
    }

    /// A temporary state folder, a record in it, and a workspace with a few files.
    ///
    /// Fail points are global to the process, so a fixture holds the fail-point lock for
    /// its whole life: executor tests run one at a time, and a test that sets a fail
    /// point can't trip another.
    struct Fixture {
        dir: tempfile::TempDir,
        record: MoveRecord,
        source: PathBuf,
        target: PathBuf,
        _fail: fail::FailScenario<'static>,
    }

    impl Fixture {
        /// A workspace at `<tmp>/from/repo`, moving to `<tmp>/to/repo`.
        fn new() -> Self {
            let fail = fail::FailScenario::setup();
            let dir = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(dir.path()).unwrap();
            let state = StateDir::open(&root.join("state")).unwrap();
            let source = root.join("from").join("repo");
            let target = root.join("to").join("repo");
            fs::create_dir_all(source.join("src").join("deep")).unwrap();
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(source.join("README.md"), b"# repo\n").unwrap();
            fs::write(source.join("src").join("main.rs"), b"fn main() {}\n").unwrap();
            fs::write(
                source.join("src").join("deep").join("big.bin"),
                vec![9u8; 300_000],
            )
            .unwrap();
            let record = MoveRecord::create(
                &state,
                MoveKind::Move,
                &source.to_string_lossy(),
                &target.to_string_lossy(),
            )
            .unwrap();
            Self {
                dir,
                record,
                source,
                target,
                _fail: fail,
            }
        }

        /// A plan whose only action is `actions`, in the core's section.
        fn plan(&self, actions: Vec<PlannedAction>) -> Plan {
            Plan {
                schema: PLAN_SCHEMA.into(),
                command: PlanCommand::Move,
                source: self.source.clone(),
                target: self.target.clone(),
                volume_relation: VolumeRelation::Same,
                holders: PlanHolders::default(),
                blockers: vec![],
                notices: vec![],
                adapters: vec![AdapterSection {
                    adapter: AdapterInfo {
                        id: "core".into(),
                        name: "core".into(),
                        kind: AdapterKind::Detector,
                        verified: vec![],
                        recognised: None,
                    },
                    found_version: None,
                    actions,
                    left_unchanged: vec![],
                    opt_ins: vec![],
                    blockers: vec![],
                    notices: vec![],
                    detected: vec![],
                    counts: vec![],
                }],
            }
        }

        /// The move-the-folder action, with its usual preconditions.
        fn move_folder(&self) -> PlannedAction {
            PlannedAction {
                phase: Phase::MoveWorkspace,
                adapter: "core".into(),
                summary: "move the folder".into(),
                action: Action::MoveTree {
                    from: self.source.clone(),
                    to: self.target.clone(),
                },
                preconditions: vec![
                    Precondition::Present {
                        path: self.source.clone(),
                    },
                    Precondition::Absent {
                        path: self.target.clone(),
                    },
                ],
                opt_in: None,
            }
        }

        /// Every file under `root`, relative, with its content.
        fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
            let mut files = BTreeMap::new();
            let mut folders = vec![root.to_path_buf()];
            while let Some(folder) = folders.pop() {
                for entry in fs::read_dir(&folder).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_type().unwrap().is_dir() {
                        folders.push(entry.path());
                    } else {
                        files.insert(
                            entry.path().strip_prefix(root).unwrap().to_path_buf(),
                            fs::read(entry.path()).unwrap(),
                        );
                    }
                }
            }
            files
        }
    }

    // 1.3: on one volume the folder moves with a single rename, and no copy appears.
    #[test]
    fn moves_a_folder_on_one_volume_with_a_rename() {
        let f = Fixture::new();
        let before = Fixture::files(&f.source);
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&f.source).unwrap());
        let outcome = Executor::new(&Host, &f.record)
            .apply(&f.plan(vec![f.move_folder()]))
            .unwrap();
        assert_eq!(
            (outcome.outcome, outcome.steps_done, outcome.failure),
            (Outcome::Succeeded, 1, None)
        );
        assert!(!f.source.exists());
        assert_eq!(Fixture::files(&f.target), before);
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::MetadataExt::ino(&fs::metadata(&f.target).unwrap()),
            inode,
            "the same folder, renamed"
        );
        let steps = f.record.steps().unwrap();
        assert_eq!(
            (steps[0].status, steps[0].effects["mode"].as_str()),
            (StepStatus::Done, Some("rename"))
        );
        let leftovers: Vec<_> = fs::read_dir(f.target.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no partial copy was made");
        assert_eq!(f.record.meta().unwrap().state, MoveState::Completed);
        let _ = &f.dir;
    }

    // 1.4: across volumes the folder is copied, verified, renamed into place, then removed.
    #[test]
    fn moves_a_folder_across_volumes_with_a_verified_copy() {
        let f = Fixture::new();
        let before = Fixture::files(&f.source);
        let platform = SplitVolumes {
            different: f.target.parent().unwrap().to_path_buf(),
        };
        let earlier = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let _ = Host.set_creation_time(&f.source, earlier);
        let outcome = Executor::new(&platform, &f.record)
            .apply(&f.plan(vec![f.move_folder()]))
            .unwrap();
        assert_eq!(outcome.outcome, Outcome::Succeeded);
        assert!(!f.source.exists());
        assert_eq!(Fixture::files(&f.target), before);
        assert_eq!(f.record.steps().unwrap()[0].effects["mode"], "copy");
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(
                Host.creation_time(&f.target).unwrap(),
                Some(earlier),
                "the folder's creation time came along"
            );
        }
    }

    // 1.4: a copy that doesn't verify is removed, and the source is left intact.
    #[test]
    fn a_copy_that_fails_verification_rolls_back() {
        let f = Fixture::new();
        fail::cfg("executor::copy-verify", "return").unwrap();
        let before = Fixture::files(&f.source);
        let platform = SplitVolumes {
            different: f.target.parent().unwrap().to_path_buf(),
        };
        let outcome = Executor::new(&platform, &f.record)
            .apply(&f.plan(vec![f.move_folder()]))
            .unwrap();
        assert_eq!(outcome.outcome, Outcome::RolledBack);
        assert_eq!(outcome.failure.unwrap().kind, FailureKind::VerifyFailed);
        assert_eq!(Fixture::files(&f.source), before, "the source is untouched");
        assert!(!f.target.exists());
        assert_eq!(
            fs::read_dir(f.target.parent().unwrap()).unwrap().count(),
            0,
            "the partial copy was removed"
        );
        assert_eq!(f.record.meta().unwrap().state, MoveState::RolledBack);
    }

    #[test]
    fn verification_catches_a_changed_or_missing_copy() {
        let f = Fixture::new();
        let copy = f.dir.path().join("copy");
        copy_tree(&f.source, &copy).unwrap();
        assert!(verify_tree(&f.source, &copy).is_ok());
        fs::write(copy.join("README.md"), b"changed").unwrap();
        assert_eq!(
            verify_tree(&f.source, &copy).err().unwrap().kind,
            FailureKind::VerifyFailed
        );
        fs::remove_file(copy.join("README.md")).unwrap();
        assert_eq!(
            verify_tree(&f.source, &copy).err().unwrap().kind,
            FailureKind::VerifyFailed
        );
    }

    // 5.4: a source reached through a link in its path moves the real folder; a source
    // that is itself a link is refused, since renaming it would move only the link.
    #[test]
    fn links_in_and_at_the_source() {
        let f = Fixture::new();
        let root = f.source.parent().unwrap().parent().unwrap().to_path_buf();
        let link = root.join("link");
        if make_dir_link(f.source.parent().unwrap(), &link).is_err() {
            return; // Links can't be made here; nothing to test.
        }
        let through_link = link.join("repo");
        let mut action = f.move_folder();
        action.action = Action::MoveTree {
            from: through_link.clone(),
            to: f.target.clone(),
        };
        action.preconditions.clear();
        let outcome = Executor::new(&Host, &f.record)
            .apply(&f.plan(vec![action]))
            .unwrap();
        assert_eq!(outcome.outcome, Outcome::Succeeded);
        assert!(f.target.join("README.md").is_file());
        assert!(!f.source.exists(), "the real folder moved");

        let second = MoveRecord::create(
            &crate::state::StateDir::open(&root.join("state")).unwrap(),
            MoveKind::Move,
            "x",
            "y",
        )
        .unwrap();
        let link_to_repo = root.join("repo-link");
        make_dir_link(&f.target, &link_to_repo).unwrap();
        let at_link = PlannedAction {
            action: Action::MoveTree {
                from: link_to_repo,
                to: root.join("elsewhere"),
            },
            preconditions: vec![],
            ..f.move_folder()
        };
        let outcome = Executor::new(&Host, &second)
            .apply(&f.plan(vec![at_link]))
            .unwrap();
        assert_eq!(outcome.failure.unwrap().kind, FailureKind::Unsupported);
        assert!(f.target.is_dir(), "the real folder stayed put");
    }

    /// Makes a link to the folder `target` at `link`: a symlink on Unix, a junction
    /// (which needs no extra rights) on Windows.
    fn make_dir_link(target: &Path, link: &Path) -> io::Result<()> {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("mklink failed"))
            }
        }
    }

    // 5.5: a workspace whose deepest path exceeds 260 characters moves completely.
    #[test]
    fn moves_paths_longer_than_260_characters() {
        let f = Fixture::new();
        let mut deep = f.source.clone();
        while deep.as_os_str().len() < 300 {
            deep.push("a-fairly-long-folder-name");
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("file.txt"), b"deep").unwrap();
        let before = Fixture::files(&f.source);
        let platform = SplitVolumes {
            different: f.target.parent().unwrap().to_path_buf(),
        };
        let outcome = Executor::new(&platform, &f.record)
            .apply(&f.plan(vec![f.move_folder()]))
            .unwrap();
        assert_eq!(outcome.outcome, Outcome::Succeeded);
        assert_eq!(Fixture::files(&f.target), before);
    }

    // 12.1: every file a step will change is backed up, with a matching hash, before
    // any step runs.
    #[test]
    fn files_are_backed_up_before_any_step_runs() {
        let f = Fixture::new();
        fail::cfg("executor::after-backups", "return").unwrap();
        let config = f.source.join("config.json");
        fs::write(&config, b"{\"a\":1}").unwrap();
        let edit = PlannedAction {
            phase: Phase::CarrySettings,
            adapter: "core".into(),
            summary: "edit".into(),
            action: Action::EditJsonFile {
                file: config.clone(),
                edits: vec![JsonEdit::RenameKey {
                    object: "".into(),
                    from: "a".into(),
                    to: "b".into(),
                }],
            },
            preconditions: vec![],
            opt_in: None,
        };
        let result = Executor::new(&Host, &f.record).apply(&f.plan(vec![f.move_folder(), edit]));
        assert!(result.is_err(), "stopped by the fail point");
        let backups = f.record.backups().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].blake3, hash_file(&config).unwrap().0);
        assert!(f.record.backup_is_intact(&backups[0]).unwrap());
        assert!(f.source.exists() && !f.target.exists(), "nothing moved");
        assert!(f.record.steps().unwrap().is_empty(), "no step started");
    }

    #[test]
    fn a_precondition_that_changed_stops_everything_before_any_change() {
        let f = Fixture::new();
        fs::create_dir(&f.target).unwrap();
        let outcome = Executor::new(&Host, &f.record)
            .apply(&f.plan(vec![f.move_folder()]))
            .unwrap();
        assert_eq!(outcome.outcome, Outcome::Blocked);
        assert_eq!(
            outcome.failure.unwrap().kind,
            FailureKind::PreconditionChanged
        );
        assert!(f.source.join("README.md").is_file());
        assert!(f.record.steps().unwrap().is_empty());
    }

    #[test]
    fn an_unsupported_action_fails_its_step_and_needs_attention() {
        let f = Fixture::new();
        let merge = PlannedAction {
            phase: Phase::CarryAgentData,
            adapter: "core".into(),
            summary: "merge".into(),
            action: Action::MergeDir {
                from: f.dir.path().join("x"),
                to: crate::plan::Destination::Path {
                    path: f.dir.path().join("y"),
                },
            },
            preconditions: vec![],
            opt_in: None,
        };
        let outcome = Executor::new(&Host, &f.record)
            .apply(&f.plan(vec![f.move_folder(), merge]))
            .unwrap();
        assert_eq!(
            (outcome.outcome, outcome.steps_done),
            (Outcome::NeedsAttention, 1)
        );
        let failure = outcome.failure.unwrap();
        assert_eq!((failure.index, failure.kind), (2, FailureKind::Unsupported));
        assert!(failure.message.contains("merge-dir"));
        let steps = f.record.steps().unwrap();
        assert_eq!(
            (steps[0].status, steps[1].status),
            (StepStatus::Done, StepStatus::Failed)
        );
        assert_eq!(f.record.meta().unwrap().state, MoveState::NeedsAttention);
    }

    #[test]
    fn opt_in_actions_never_run_without_a_confirmation() {
        let f = Fixture::new();
        let mut opt_in = f.move_folder();
        opt_in.opt_in = Some(crate::plan::OptInId::RewriteSessions);
        let outcome = Executor::new(&Host, &f.record)
            .apply(&f.plan(vec![opt_in]))
            .unwrap();
        assert_eq!(
            (outcome.outcome, outcome.steps_done),
            (Outcome::Succeeded, 0)
        );
        assert!(f.source.exists(), "the opt-in action didn't run");
    }

    // On Windows, a folder something works in can't be renamed: the step fails as held,
    // and since it's the first, nothing changed.
    #[cfg(windows)]
    #[test]
    fn a_held_folder_blocks_the_move_and_changes_nothing() {
        let f = Fixture::new();
        let _holder = awm_testkit::FakeProcess::start(&f.source.join("src"), None).unwrap();
        let outcome = Executor::new(&Host, &f.record)
            .apply(&f.plan(vec![f.move_folder()]))
            .unwrap();
        assert_eq!(outcome.outcome, Outcome::Blocked);
        assert_eq!(outcome.failure.unwrap().kind, FailureKind::Held);
        assert!(f.source.join("README.md").is_file());
        assert_eq!(f.record.meta().unwrap().state, MoveState::Planned);
    }

    #[test]
    fn preconditions_check_files_and_folders() {
        let f = Fixture::new();
        let readme = f.source.join("README.md");
        let (hash, size) = hash_file(&readme).unwrap();
        let listing = listing_hash(&f.source).unwrap();
        let action = |preconditions| PlannedAction {
            preconditions,
            ..f.move_folder()
        };
        assert!(
            check_preconditions(&action(vec![
                Precondition::FileUnchanged {
                    path: readme.clone(),
                    size,
                    blake3: hash.clone()
                },
                Precondition::DirUnchanged {
                    path: f.source.clone(),
                    listing: listing.clone()
                },
            ]))
            .is_ok()
        );
        fs::write(&readme, b"changed").unwrap();
        let message = check_preconditions(&action(vec![Precondition::FileUnchanged {
            path: readme.clone(),
            size,
            blake3: hash,
        }]))
        .unwrap_err();
        assert!(message.contains("README.md") && message.contains("plan again"));
        fs::write(f.source.join("new.txt"), b"x").unwrap();
        assert!(
            check_preconditions(&action(vec![Precondition::DirUnchanged {
                path: f.source.clone(),
                listing
            }]))
            .is_err()
        );
    }

    #[test]
    fn probes_tell_where_a_folder_move_stands() {
        let f = Fixture::new();
        let action = Action::MoveTree {
            from: f.source.clone(),
            to: f.target.clone(),
        };
        let id = f.record.id().as_str();
        assert_eq!(probe(&action, id).unwrap(), ProbeState::NotStarted);
        let partial = f
            .target
            .parent()
            .unwrap()
            .join(format!(".awm-partial-{id}"));
        fs::create_dir(&partial).unwrap();
        assert_eq!(probe(&action, id).unwrap(), ProbeState::Partial);
        fs::remove_dir(&partial).unwrap();
        fs::create_dir(&f.target).unwrap();
        assert_eq!(
            probe(&action, id).unwrap(),
            ProbeState::Partial,
            "both present: the source isn't fully removed"
        );
        fs::remove_dir_all(&f.source).unwrap();
        assert_eq!(probe(&action, id).unwrap(), ProbeState::Done);
        fs::remove_dir(&f.target).unwrap();
        assert!(probe(&action, id).is_err());
        let merge = Action::MergeDir {
            from: "/x".into(),
            to: crate::plan::Destination::Path { path: "/y".into() },
        };
        assert_eq!(
            probe(&merge, id).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn edited_files_are_named_for_every_action_that_edits() {
        let mapping = crate::paths::MappingSpec {
            source: "/a".into(),
            target: "/b".into(),
            style: crate::paths::PathStyle::Posix,
            case: crate::paths::CaseRule::Sensitive,
        };
        let editing = [
            Action::EditJsonFile {
                file: "/j".into(),
                edits: vec![],
            },
            Action::EditSqliteJsonRow {
                db: "/d".into(),
                table: "t".into(),
                key: "k".into(),
                edits: vec![],
            },
            Action::SpliceJsonlField {
                file: "/l".into(),
                field: "cwd".into(),
                mapping: mapping.clone(),
            },
            Action::ReplaceInText {
                file: "/t".into(),
                mapping,
                matches: vec![],
            },
        ];
        for action in &editing {
            assert_eq!(files_edited(action).len(), 1);
        }
        assert!(
            files_edited(&Action::RunTool {
                program: "git".into(),
                args: vec![],
                cwd: "/".into(),
                inverse: None
            })
            .is_empty()
        );
    }
}
