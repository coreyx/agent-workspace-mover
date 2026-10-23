//! Move records and backups (requirements.md, REQ-12; design.md, "Data model").
//!
//! Every move, rescue and undo gets a folder under the state folder's `moves/`:
//!
//! ```text
//! moves/<move id>/
//! ├── move.json          id, kind, source, target, state, what it undoes
//! ├── plan.json          the approved plan
//! ├── steps/0001.json …  write-ahead step records
//! ├── backups/0001-<name> …  content backups
//! └── report.json        the final report
//! ```
//!
//! Each file is written atomically and readable by the current user only. A step's
//! record is written before the step changes anything, and again after (design.md,
//! "Key design decisions", 4), so an interruption at any point leaves enough to resume
//! or roll back. Records are plain JSON, so they can be read by hand when the tool
//! itself is what failed.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::state::StateDir;
use crate::timestamp::Utc;
use crate::{atomic, platform};

/// A move's id: when it was created, and a random suffix, as `20261023-095701-3fa9`.
/// Ids sort in creation order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MoveId(String);

impl MoveId {
    /// A new id for a move created at `now`. `salt` varies the suffix when an id is
    /// taken and another is needed.
    fn generate(now: Utc, salt: u32) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&now.unix().to_le_bytes());
        hasher.update(&std::process::id().to_le_bytes());
        hasher.update(&salt.to_le_bytes());
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        hasher.update(&nanos.to_le_bytes());
        let suffix = &hasher.finalize().to_hex()[..4];
        Self(format!("{}-{suffix}", now.compact()))
    }

    /// Parses an id written as `yyyymmdd-hhmmss-xxxx`, as a user might type it.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let well_formed = bytes.len() == 20
            && bytes[8] == b'-'
            && bytes[15] == b'-'
            && bytes[..8]
                .iter()
                .chain(&bytes[9..15])
                .all(u8::is_ascii_digit)
            && bytes[16..]
                .iter()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        well_formed.then(|| Self(text.to_owned()))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a record describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoveKind {
    /// A folder and everything keyed on its path moved (REQ-1).
    Move,
    /// Orphaned data reattached to a folder already moved by hand (REQ-15).
    Rescue,
    /// A past move reversed (REQ-14).
    Undo,
}

/// Where a move stands (design.md, "A move's states").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoveState {
    /// Planned, not yet approved.
    Planned,
    /// Approved, waiting for what holds the folder to exit (REQ-19).
    Waiting,
    /// Its steps are being carried out.
    Applying,
    /// Every step done and verified.
    Completed,
    /// A step failed or was interrupted, or verification failed: resume or roll back.
    NeedsAttention,
    /// Reversed after being interrupted.
    RolledBack,
    /// Reversed by a later undo move.
    Undone,
    /// A wait ended because the re-plan wasn't acceptable.
    Stopped,
    /// A wait ended because its time limit ran out.
    GaveUp,
    /// A wait ended because the user cancelled it.
    Cancelled,
}

/// The contents of `move.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveMeta {
    /// The move's id, which is also its folder's name.
    pub id: MoveId,
    /// What the record describes.
    pub kind: MoveKind,
    /// The workspace's path before the move.
    pub source: String,
    /// The workspace's path after the move.
    pub target: String,
    /// Where the move stands.
    pub state: MoveState,
    /// For an undo, the move it reverses.
    pub undoes: Option<MoveId>,
    /// When the record was created, in RFC 3339.
    pub created: String,
    /// When `state` last changed, in RFC 3339.
    pub updated: String,
}

/// Where one step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepStatus {
    /// Recorded before the step's effect; the effect may or may not have happened.
    Started,
    /// The step's effect happened, and `effects` records what undo needs.
    Done,
    /// The step failed. Its effect didn't happen, or happened only in part.
    Failed,
    /// The step was reversed by a rollback.
    Reversed,
}

/// One step's write-ahead record, `steps/NNNN.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    /// The step's position in the plan, from 1.
    pub index: u32,
    /// Where it stands.
    pub status: StepStatus,
    /// The planned action, as the plan recorded it.
    pub action: serde_json::Value,
    /// What the step did that undo and rollback need, such as entries that already
    /// existed at a merge's destination. Null until the step is done.
    pub effects: serde_json::Value,
    /// The backups made before this step, by index.
    pub backups: Vec<u32>,
    /// When the step started, in RFC 3339.
    pub started: String,
    /// When it finished or failed, in RFC 3339.
    pub finished: Option<String>,
}

/// One backed-up file, recorded so a restore can check it got the right bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    /// The backup's number within its move, from 1.
    pub index: u32,
    /// The file that was backed up.
    pub original: PathBuf,
    /// The backup's file name inside the move's `backups/` folder.
    pub stored_as: String,
    /// The content's `blake3` hash, in hex.
    pub blake3: String,
    /// The content's length.
    pub bytes: u64,
}

/// One move's folder of records.
#[derive(Clone, Debug)]
pub struct MoveRecord {
    dir: PathBuf,
    id: MoveId,
}

/// How many fresh ids to try if the first ones are taken, which needs two moves
/// created in the same second with colliding suffixes.
const ID_ATTEMPTS: u32 = 16;

impl MoveRecord {
    /// Creates a new record, in state `Planned`.
    pub fn create(
        state: &StateDir,
        kind: MoveKind,
        source: &str,
        target: &str,
    ) -> io::Result<Self> {
        let now = Utc::now();
        for salt in 0..ID_ATTEMPTS {
            let id = MoveId::generate(now, salt);
            let dir = state.moves_dir().join(id.as_str());
            match fs::create_dir(&dir) {
                Ok(()) => {
                    let record = Self { dir, id };
                    for folder in [record.dir.clone(), record.steps_dir(), record.backups_dir()] {
                        fs::create_dir_all(&folder)?;
                        platform::restrict_to_user(&folder)?;
                    }
                    let meta = MoveMeta {
                        id: record.id.clone(),
                        kind,
                        source: source.to_owned(),
                        target: target.to_owned(),
                        state: MoveState::Planned,
                        undoes: None,
                        created: now.rfc3339(),
                        updated: now.rfc3339(),
                    };
                    record.write_json("move.json", &meta)?;
                    return Ok(record);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free move id after several attempts",
        ))
    }

    /// Opens an existing record.
    pub fn open(state: &StateDir, id: &MoveId) -> io::Result<Self> {
        let dir = state.moves_dir().join(id.as_str());
        if !dir.join("move.json").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("there's no move {}", id.as_str()),
            ));
        }
        Ok(Self {
            dir,
            id: id.clone(),
        })
    }

    /// Every record in the state folder, oldest first.
    pub fn list(state: &StateDir) -> io::Result<Vec<MoveId>> {
        let mut ids: Vec<MoveId> = fs::read_dir(state.moves_dir())?
            .filter_map(|entry| entry.ok()?.file_name().to_str().and_then(MoveId::parse))
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// The move's id.
    pub fn id(&self) -> &MoveId {
        &self.id
    }

    /// The record's folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The folder of write-ahead step records.
    fn steps_dir(&self) -> PathBuf {
        self.dir.join("steps")
    }

    /// The folder of content backups.
    fn backups_dir(&self) -> PathBuf {
        self.dir.join("backups")
    }

    /// Writes `value` as pretty JSON to `name` in the record's folder, atomically and
    /// restricted to the user.
    fn write_json<T: Serialize>(&self, name: &str, value: &T) -> io::Result<()> {
        let path = self.dir.join(name);
        let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
        atomic::write(&path, &bytes)?;
        platform::restrict_to_user(&path)
    }

    /// Reads `name` in the record's folder as JSON.
    fn read_json<T: DeserializeOwned>(&self, name: &str) -> io::Result<T> {
        let bytes = fs::read(self.dir.join(name))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// The contents of `move.json`.
    pub fn meta(&self) -> io::Result<MoveMeta> {
        self.read_json("move.json")
    }

    /// Records that the move now stands at `state`, returning the updated contents.
    pub fn set_state(&self, state: MoveState) -> io::Result<MoveMeta> {
        let mut meta = self.meta()?;
        meta.state = state;
        meta.updated = Utc::now().rfc3339();
        self.write_json("move.json", &meta)?;
        Ok(meta)
    }

    /// Records which move this one reverses, for an undo.
    pub fn set_undoes(&self, original: &MoveId) -> io::Result<()> {
        let mut meta = self.meta()?;
        meta.undoes = Some(original.clone());
        self.write_json("move.json", &meta)
    }

    /// Writes the approved plan.
    pub fn write_plan<T: Serialize>(&self, plan: &T) -> io::Result<()> {
        self.write_json("plan.json", plan)
    }

    /// Reads the approved plan.
    pub fn read_plan<T: DeserializeOwned>(&self) -> io::Result<T> {
        self.read_json("plan.json")
    }

    /// Writes the final report.
    pub fn write_report<T: Serialize>(&self, report: &T) -> io::Result<()> {
        self.write_json("report.json", report)
    }

    /// Reads the final report.
    pub fn read_report<T: DeserializeOwned>(&self) -> io::Result<T> {
        self.read_json("report.json")
    }

    /// Writes (or rewrites) one step's record. It's on disk when this returns.
    pub fn write_step(&self, step: &StepRecord) -> io::Result<()> {
        self.write_json(&format!("steps/{:04}.json", step.index), step)
    }

    /// Every step record, in plan order.
    pub fn steps(&self) -> io::Result<Vec<StepRecord>> {
        let mut names: Vec<PathBuf> = fs::read_dir(self.steps_dir())?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .collect();
        names.sort();
        names
            .iter()
            .map(|path| {
                let name = path
                    .strip_prefix(&self.dir)
                    .expect("listed inside the record");
                self.read_json(&name.to_string_lossy())
            })
            .collect()
    }

    /// Copies `original` into the record's backups as number `index`, and checks the
    /// copy byte for byte against what it copied (12.0.1). Returns the entry to record.
    pub fn back_up(&self, index: u32, original: &Path) -> io::Result<BackupEntry> {
        let base = original
            .file_name()
            .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
        let stored_as = format!("{index:04}-{base}");
        let stored = self.backups_dir().join(&stored_as);
        let before = hash_file(original)?;
        fs::copy(original, &stored)?;
        // Windows only flushes a file opened for writing.
        fs::OpenOptions::new()
            .write(true)
            .open(&stored)?
            .sync_all()?;
        platform::restrict_to_user(&stored)?;
        let (copy, bytes) = hash_file(&stored)?;
        if copy != before.0 {
            fs::remove_file(&stored)?;
            return Err(io::Error::other(format!(
                "{} changed while it was being backed up",
                original.display()
            )));
        }
        Ok(BackupEntry {
            index,
            original: original.to_path_buf(),
            stored_as,
            blake3: copy,
            bytes,
        })
    }

    /// The path of a recorded backup's bytes.
    pub fn backup_path(&self, entry: &BackupEntry) -> PathBuf {
        self.backups_dir().join(&entry.stored_as)
    }

    /// Whether a recorded backup is still present and intact.
    pub fn backup_is_intact(&self, entry: &BackupEntry) -> io::Result<bool> {
        match hash_file(&self.backup_path(entry)) {
            Ok((hash, bytes)) => Ok(hash == entry.blake3 && bytes == entry.bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The total size of the record's backups, for `history` (12.0.6).
    pub fn backup_size(&self) -> io::Result<u64> {
        fs::read_dir(self.backups_dir())?
            .try_fold(0, |total, entry| Ok(total + entry?.metadata()?.len()))
    }
}

/// A file's `blake3` hash in hex, and its length, read in a stream.
pub fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok((hasher.finalize().to_hex().to_string(), total));
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh state folder in a temporary folder.
    fn state() -> (tempfile::TempDir, StateDir) {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::open(&dir.path().join("state")).unwrap();
        (dir, state)
    }

    #[test]
    fn ids_are_well_formed_and_parse_back() {
        let id = MoveId::generate(Utc::from_unix(1_792_749_421), 0);
        assert!(id.as_str().starts_with("20261023-095701-"));
        assert_eq!(MoveId::parse(id.as_str()), Some(id));
        for bad in [
            "",
            "20261023-095701",
            "20261023-095701-ABCD",
            "2026102x-095701-abcd",
            "20261023_095701-abcd",
        ] {
            assert_eq!(MoveId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn creates_a_record_with_its_folders_and_meta() {
        let (_dir, state) = state();
        let record = MoveRecord::create(&state, MoveKind::Move, "/a/repo", "/b/repo").unwrap();
        let meta = record.meta().unwrap();
        assert_eq!(
            (meta.kind, meta.state),
            (MoveKind::Move, MoveState::Planned)
        );
        assert_eq!(
            (meta.source.as_str(), meta.target.as_str(), &meta.undoes),
            ("/a/repo", "/b/repo", &None)
        );
        for path in [
            record.dir().to_path_buf(),
            record.dir().join("move.json"),
            record.steps_dir(),
            record.backups_dir(),
        ] {
            assert!(
                platform::is_user_only(&path).unwrap(),
                "{} is user-only",
                path.display()
            );
        }
        let reopened = MoveRecord::open(&state, record.id()).unwrap();
        assert_eq!(reopened.meta().unwrap(), meta);
        assert_eq!(MoveRecord::list(&state).unwrap(), vec![record.id().clone()]);
    }

    #[test]
    fn many_records_in_one_second_get_distinct_ids() {
        let (_dir, state) = state();
        let ids: Vec<MoveId> = (0..12)
            .map(|_| {
                MoveRecord::create(&state, MoveKind::Move, "/a", "/b")
                    .unwrap()
                    .id()
                    .clone()
            })
            .collect();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
        assert_eq!(MoveRecord::list(&state).unwrap().len(), 12);
    }

    #[test]
    fn opening_a_missing_record_is_an_error() {
        let (_dir, state) = state();
        let id = MoveId::parse("20261023-095701-abcd").unwrap();
        assert_eq!(
            MoveRecord::open(&state, &id).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn records_state_plans_reports_and_undo_links() {
        let (_dir, state) = state();
        let record = MoveRecord::create(&state, MoveKind::Undo, "/b", "/a").unwrap();
        assert_eq!(
            record.set_state(MoveState::Applying).unwrap().state,
            MoveState::Applying
        );
        let original = MoveId::parse("20261023-095701-abcd").unwrap();
        record.set_undoes(&original).unwrap();
        assert_eq!(record.meta().unwrap().undoes, Some(original));
        record
            .write_plan(&serde_json::json!({"schema": "awm.plan/1", "actions": []}))
            .unwrap();
        let plan: serde_json::Value = record.read_plan().unwrap();
        assert_eq!(plan["schema"], "awm.plan/1");
        record
            .write_report(&serde_json::json!({"done": []}))
            .unwrap();
        let report: serde_json::Value = record.read_report().unwrap();
        assert!(report["done"].is_array());
    }

    #[test]
    fn steps_are_written_ahead_and_read_in_order() {
        let (_dir, state) = state();
        let record = MoveRecord::create(&state, MoveKind::Move, "/a", "/b").unwrap();
        let step = |index, status| StepRecord {
            index,
            status,
            action: serde_json::json!({"kind": "move-tree"}),
            effects: serde_json::Value::Null,
            backups: vec![],
            started: Utc::now().rfc3339(),
            finished: None,
        };
        record.write_step(&step(2, StepStatus::Started)).unwrap();
        record.write_step(&step(1, StepStatus::Started)).unwrap();
        record.write_step(&step(1, StepStatus::Done)).unwrap();
        let steps = record.steps().unwrap();
        assert_eq!(
            steps
                .iter()
                .map(|s| (s.index, s.status))
                .collect::<Vec<_>>(),
            [(1, StepStatus::Done), (2, StepStatus::Started)]
        );
        assert!(platform::is_user_only(&record.steps_dir().join("0001.json")).unwrap());
    }

    #[test]
    fn a_damaged_record_is_invalid_data() {
        let (_dir, state) = state();
        let record = MoveRecord::create(&state, MoveKind::Move, "/a", "/b").unwrap();
        fs::write(record.dir().join("move.json"), b"{ not json").unwrap();
        assert_eq!(
            record.meta().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn backups_copy_and_verify_the_original() {
        let (dir, state) = state();
        let record = MoveRecord::create(&state, MoveKind::Move, "/a", "/b").unwrap();
        let original = dir.path().join(".claude.json");
        fs::write(&original, b"{\"projects\":{}}").unwrap();
        let entry = record.back_up(1, &original).unwrap();
        assert_eq!(entry.stored_as, "0001-.claude.json");
        assert_eq!(entry.bytes, 15);
        assert_eq!(entry.blake3, hash_file(&original).unwrap().0);
        assert_eq!(
            fs::read(record.backup_path(&entry)).unwrap(),
            fs::read(&original).unwrap()
        );
        assert!(platform::is_user_only(&record.backup_path(&entry)).unwrap());
        assert!(record.backup_is_intact(&entry).unwrap());
        assert_eq!(record.backup_size().unwrap(), 15);

        fs::write(record.backup_path(&entry), b"tampered").unwrap();
        assert!(!record.backup_is_intact(&entry).unwrap());
        fs::remove_file(record.backup_path(&entry)).unwrap();
        assert!(
            !record.backup_is_intact(&entry).unwrap(),
            "a pruned backup isn't intact"
        );
    }

    #[test]
    fn backing_up_a_missing_file_is_an_error() {
        let (dir, state) = state();
        let record = MoveRecord::create(&state, MoveKind::Move, "/a", "/b").unwrap();
        assert!(record.back_up(1, &dir.path().join("missing")).is_err());
    }

    #[test]
    fn hashes_large_files_in_a_stream() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("big");
        let content = vec![7u8; 200_000];
        fs::write(&file, &content).unwrap();
        assert_eq!(
            hash_file(&file).unwrap(),
            (blake3::hash(&content).to_hex().to_string(), 200_000)
        );
    }
}
