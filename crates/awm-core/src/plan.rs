//! The plan and the report: what a move will do, and what it did (requirements.md,
//! REQ-3 and REQ-22; design.md, "Data model" and "Interfaces").
//!
//! A [`Plan`] is built once, from every adapter's contribution, and is the same object a
//! dry run prints, the user approves, the record stores and the executor carries out
//! (design.md, "Key design decisions", 2). Its actions come from the closed [`Action`]
//! set the executor knows how to perform, record, probe and reverse (decision 1).
//!
//! Both types serialise to the JSON that `--json` prints, and their JSON Schemas are
//! generated from these same definitions, so the two can't drift (22.0.1).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::paths::MappingSpec;
use crate::platform::HoldKind;

/// The plan's schema name and version, written into every plan.
pub const PLAN_SCHEMA: &str = "awm.plan/1";

/// The report's schema name and version, written into every report.
pub const REPORT_SCHEMA: &str = "awm.report/1";

/// What a plan is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PlanCommand {
    /// Move a folder and everything keyed on its path (REQ-1).
    Move,
    /// Reattach data orphaned by a move done by hand (REQ-15).
    Rescue,
    /// Reverse a past move (REQ-14).
    Undo,
}

/// Where the move's target is relative to its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeRelation {
    /// The same volume: the folder moves with one rename (1.0.3).
    Same,
    /// Different volumes: the folder is copied, verified, then removed (1.0.4).
    Different,
    /// Nothing moves on disk, as in a rescue.
    NoFolderMove,
}

/// The phases a plan's actions run in, in this order (design.md, "Order of steps").
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// Moving the workspace folder itself.
    MoveWorkspace,
    /// Repairing version control's links.
    RepairVcs,
    /// Carrying agent data: session folders and memory.
    CarryAgentData,
    /// Carrying per-project settings.
    CarrySettings,
    /// Carrying editor state.
    CarryEditorState,
    /// Updating the paths inside the workspace the user selected.
    UpdateWorkspacePaths,
    /// Edits to the record, each only with its typed confirmation.
    OptIn,
}

/// An edit to the record that happens only when the user asks for it and types its
/// phrase (REQ-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OptInId {
    /// Rewrite the `cwd` recorded on transcript lines.
    RewriteSessions,
    /// Repoint the `project` field in prompt history.
    RewriteHistory,
}

impl OptInId {
    /// The phrase the user types to confirm this opt-in, exactly.
    pub fn phrase(self) -> &'static str {
        match self {
            OptInId::RewriteSessions => "rewrite sessions",
            OptInId::RewriteHistory => "rewrite history",
        }
    }

    /// The command-line flag that requests this opt-in.
    pub fn flag(self) -> &'static str {
        match self {
            OptInId::RewriteSessions => "--rewrite-sessions",
            OptInId::RewriteHistory => "--rewrite-history",
        }
    }
}

/// Where a merge's entries go: a path known now, or one an adapter works out after the
/// folder has moved, such as VS Code's storage id (design.md, "Key design decisions", 13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Destination {
    /// A path known when planning.
    Path {
        /// The destination folder.
        path: PathBuf,
    },
    /// A path the named adapter resolves at apply time from `key`.
    Deferred {
        /// The adapter that resolves it.
        adapter: String,
        /// What the adapter needs to resolve it.
        key: String,
    },
}

/// One structural edit to a JSON document. Pointers are JSON Pointers (RFC 6901).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum JsonEdit {
    /// Renames key `from` to `to` in the object at `object`, keeping its value and place.
    RenameKey {
        /// The object holding the key.
        object: String,
        /// The key's current name.
        from: String,
        /// Its new name.
        to: String,
    },
    /// Replaces the string at `at`, which must currently equal `from`.
    ReplaceString {
        /// The string's location.
        at: String,
        /// Its current value.
        from: String,
        /// Its new value.
        to: String,
    },
    /// Rewrites every string under `at` that is a path or URI under the mapping's
    /// source, recursing into JSON held in strings.
    RewritePathStrings {
        /// Where to start.
        at: String,
        /// The source and target.
        mapping: MappingSpec,
    },
}

/// A byte range in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextSpan {
    /// The first byte.
    pub start: u64,
    /// The byte after the last.
    pub end: u64,
}

/// The closed set of changes the executor makes (design.md, "Key design decisions", 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    /// Moves a folder: a rename on one volume, a verified copy across volumes.
    MoveTree {
        /// Where it is.
        from: PathBuf,
        /// Where it goes.
        to: PathBuf,
    },
    /// Merges a folder's entries into another, renaming it when the destination is free.
    MergeDir {
        /// The folder to merge.
        from: PathBuf,
        /// Where its entries go.
        to: Destination,
    },
    /// Edits a JSON file as JSON.
    EditJsonFile {
        /// The file.
        file: PathBuf,
        /// The edits, in order.
        edits: Vec<JsonEdit>,
    },
    /// Edits the JSON value of one row in a SQLite table.
    EditSqliteJsonRow {
        /// The database file.
        db: PathBuf,
        /// The table.
        table: String,
        /// The row's key.
        key: String,
        /// The edits, in order.
        edits: Vec<JsonEdit>,
    },
    /// Replaces a top-level string field's path on every line of a JSON Lines file.
    SpliceJsonlField {
        /// The file.
        file: PathBuf,
        /// The field, such as `cwd`.
        field: String,
        /// The source and target.
        mapping: MappingSpec,
    },
    /// Replaces the source path at chosen places in a text file.
    ReplaceInText {
        /// The file.
        file: PathBuf,
        /// The source and target.
        mapping: MappingSpec,
        /// Where the matches are.
        matches: Vec<TextSpan>,
    },
    /// Runs an external program, such as `git worktree repair`.
    RunTool {
        /// The program.
        program: String,
        /// Its arguments.
        args: Vec<String>,
        /// The folder to run it in.
        cwd: PathBuf,
        /// What reverses it, run after the reverse move.
        inverse: Option<Box<Action>>,
    },
}

/// Something that must still be true for an action to be safe (design.md, "Key design
/// decisions", 6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Precondition {
    /// The file is unchanged since planning.
    FileUnchanged {
        /// The file.
        path: PathBuf,
        /// Its length.
        size: u64,
        /// Its `blake3` hash, in hex.
        blake3: String,
    },
    /// The folder's listing is unchanged since planning.
    DirUnchanged {
        /// The folder.
        path: PathBuf,
        /// A hash of its entries' names, sizes and modification times.
        listing: String,
    },
    /// Nothing exists at the path.
    Absent {
        /// The path.
        path: PathBuf,
    },
    /// Something exists at the path.
    Present {
        /// The path.
        path: PathBuf,
    },
}

/// One action in a plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlannedAction {
    /// The phase it runs in.
    pub phase: Phase,
    /// The adapter that planned it.
    pub adapter: String,
    /// What it does, in a sentence the plan shows.
    pub summary: String,
    /// The change itself.
    pub action: Action,
    /// What must still be true when it runs.
    pub preconditions: Vec<Precondition>,
    /// The opt-in it belongs to, if any. Such an action runs only with that opt-in's
    /// typed confirmation.
    pub opt_in: Option<OptInId>,
}

/// Something that stops the real run, with what the user can do about it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Blocker {
    /// A stable code scripts and the agent skill can rely on, such as `agent-running`.
    pub code: String,
    /// What's wrong.
    pub message: String,
    /// What to do about it.
    pub remedy: Option<String>,
}

/// Something worth knowing that doesn't stop the run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Notice {
    /// A stable code.
    pub code: String,
    /// What to know.
    pub message: String,
}

/// Where a leftover of the source path is expected after a move (design.md, "Key
/// design decisions", 17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LeftoverPattern {
    /// The files it's expected in, as a pattern relative to a root, such as
    /// `projects/*/*.jsonl`.
    pub files: String,
    /// The JSON field it's expected in, if any, such as `cwd`.
    pub field: Option<String>,
}

/// Something deliberately left as it is, with how many and what would change it
/// (3.0.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LeftUnchanged {
    /// What's left, such as "recorded paths in sessions".
    pub what: String,
    /// How many.
    pub count: u64,
    /// The opt-in or flag that would change it, such as `--rewrite-sessions`.
    pub changed_by: Option<String>,
    /// Where the leftover search should expect it.
    pub expected_leftover: Option<LeftoverPattern>,
}

/// How many matches an opt-in would change in one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileCount {
    /// The file.
    pub file: PathBuf,
    /// How many.
    pub count: u64,
}

/// An opt-in an adapter offers, with what it would change (8.0.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OptInOffer {
    /// Which opt-in.
    pub id: OptInId,
    /// The phrase to type.
    pub phrase: String,
    /// What it would change, per file.
    pub counts: Vec<FileCount>,
    /// Whether the user asked for it this run.
    pub requested: bool,
    /// Whether it was applied. Only a report sets this.
    pub applied: bool,
}

/// A labelled count, such as ("sessions the picker lists", 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Count {
    /// What's counted.
    pub label: String,
    /// How many.
    pub value: u64,
}

/// What kind of tool an adapter covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    /// An AI agent, such as Claude Code.
    Agent,
    /// A version-control system, such as git.
    VersionControl,
    /// An editor, such as VS Code.
    Editor,
    /// A report-only detector (REQ-11).
    Detector,
}

/// A version range an adapter has been checked against, on one OS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Verification {
    /// The versions, such as `2.1.309`.
    pub versions: String,
    /// The OS, such as `Windows 11`.
    pub os: String,
}

/// Who an adapter is and what it's been checked against (24.0.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdapterInfo {
    /// A stable id, such as `claude-code`.
    pub id: String,
    /// A name for people, such as `Claude Code`.
    pub name: String,
    /// What kind of tool it covers.
    pub kind: AdapterKind,
    /// The versions and OSes it's verified against.
    pub verified: Vec<Verification>,
    /// The versions it recognises, beyond those verified, such as `>=2.1.0, <2.2.0`.
    pub recognised: Option<String>,
}

/// One adapter's part of a plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdapterSection {
    /// The adapter.
    pub adapter: AdapterInfo,
    /// The tool's version found on this machine, if it has one.
    pub found_version: Option<String>,
    /// What it will do.
    pub actions: Vec<PlannedAction>,
    /// What it leaves unchanged by default.
    pub left_unchanged: Vec<LeftUnchanged>,
    /// The opt-ins it offers.
    pub opt_ins: Vec<OptInOffer>,
    /// What stops its changes.
    pub blockers: Vec<Blocker>,
    /// What's worth knowing.
    pub notices: Vec<Notice>,
    /// Path-keyed state it found but version 1 doesn't fix (REQ-11).
    pub detected: Vec<Notice>,
    /// Counts for the plan to show.
    pub counts: Vec<Count>,
}

/// One process holding the source, as the plan shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HolderEntry {
    /// Its process id.
    pub pid: u32,
    /// Its program's name.
    pub name: String,
    /// Its executable, when readable.
    pub exe: Option<PathBuf>,
    /// How it holds the folder.
    pub kind: HoldKind,
    /// Whether it's the session the tool is running in (20.0.3).
    pub this_session: bool,
}

/// What holds the source, as the plan shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanHolders {
    /// The processes found.
    pub holders: Vec<HolderEntry>,
    /// How many processes couldn't be inspected.
    pub uninspectable: u64,
    /// Whether the search covered everything (20.0.4).
    pub complete: bool,
}

/// A whole plan (REQ-3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// The schema name and version: [`PLAN_SCHEMA`].
    pub schema: String,
    /// What the plan is for.
    pub command: PlanCommand,
    /// The workspace's path now.
    pub source: PathBuf,
    /// Its path after the move.
    pub target: PathBuf,
    /// Whether the folder moves by rename, by copy, or not at all.
    pub volume_relation: VolumeRelation,
    /// What holds the source.
    pub holders: PlanHolders,
    /// Blockers found by the core's own checks, such as a target that already exists.
    pub blockers: Vec<Blocker>,
    /// Notices from the core's own checks.
    pub notices: Vec<Notice>,
    /// Each adapter's part.
    pub adapters: Vec<AdapterSection>,
}

impl Plan {
    /// Every blocker, the core's and the adapters', in that order.
    pub fn all_blockers(&self) -> impl Iterator<Item = &Blocker> {
        self.blockers
            .iter()
            .chain(self.adapters.iter().flat_map(|a| a.blockers.iter()))
    }

    /// Whether anything stops the real run.
    pub fn is_blocked(&self) -> bool {
        self.all_blockers().next().is_some()
    }

    /// Every action, in the order the executor runs them: by phase, then in the order
    /// adapters listed them.
    pub fn ordered_actions(&self) -> Vec<&PlannedAction> {
        let mut actions: Vec<&PlannedAction> = self
            .adapters
            .iter()
            .flat_map(|a| a.actions.iter())
            .collect();
        // A stable sort keeps each phase's actions in the adapters' order.
        actions.sort_by_key(|a| a.phase);
        actions
    }
}

/// How a run ended. Each maps to one exit code (design.md, "Exit codes").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Everything planned was done and verified.
    Succeeded,
    /// Done, with warnings or things it couldn't do.
    SucceededWithWarnings,
    /// Nothing changed, by choice: a dry run, or a declined go-ahead.
    NothingChanged,
    /// Checks found blockers, and nothing changed.
    Blocked,
    /// It failed, and was rolled back automatically.
    RolledBack,
    /// It failed or was interrupted, and needs `resume` or `rollback`.
    NeedsAttention,
}

/// One line of a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReportItem {
    /// The adapter it concerns, or `core`.
    pub adapter: String,
    /// What was done or not done.
    pub summary: String,
    /// A stable code for things not done, such as `git-missing`.
    pub code: Option<String>,
}

/// What a run did (1.0.6). Its three sections are always present, even when empty (1.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// The schema name and version: [`REPORT_SCHEMA`].
    pub schema: String,
    /// The move's id, when one was recorded.
    pub move_id: Option<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// What was done.
    pub done: Vec<ReportItem>,
    /// What was deliberately left unchanged.
    pub left_unchanged: Vec<LeftUnchanged>,
    /// What couldn't be done, and why.
    pub couldnt_do: Vec<ReportItem>,
    /// Warnings, such as unexpected leftovers.
    pub warnings: Vec<Notice>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{CaseRule, PathStyle};

    /// A section with one action in `phase`, labelled `label`.
    fn section(id: &str, actions: &[(Phase, &str)]) -> AdapterSection {
        AdapterSection {
            adapter: AdapterInfo {
                id: id.into(),
                name: id.into(),
                kind: AdapterKind::Agent,
                verified: vec![],
                recognised: None,
            },
            found_version: None,
            actions: actions
                .iter()
                .map(|(phase, label)| PlannedAction {
                    phase: *phase,
                    adapter: id.into(),
                    summary: (*label).into(),
                    action: Action::MoveTree {
                        from: "/a".into(),
                        to: "/b".into(),
                    },
                    preconditions: vec![],
                    opt_in: None,
                })
                .collect(),
            left_unchanged: vec![],
            opt_ins: vec![],
            blockers: vec![],
            notices: vec![],
            detected: vec![],
            counts: vec![],
        }
    }

    /// A plan with `adapters` and nothing else.
    fn plan(adapters: Vec<AdapterSection>) -> Plan {
        Plan {
            schema: PLAN_SCHEMA.into(),
            command: PlanCommand::Move,
            source: "/a".into(),
            target: "/b".into(),
            volume_relation: VolumeRelation::Same,
            holders: PlanHolders::default(),
            blockers: vec![],
            notices: vec![],
            adapters,
        }
    }

    #[test]
    fn orders_actions_by_phase_then_adapter_order() {
        let p = plan(vec![
            section(
                "agent",
                &[
                    (Phase::CarrySettings, "settings"),
                    (Phase::CarryAgentData, "sessions"),
                ],
            ),
            section("git", &[(Phase::RepairVcs, "repair")]),
            section(
                "core",
                &[
                    (Phase::MoveWorkspace, "folder"),
                    (Phase::CarryAgentData, "memory"),
                ],
            ),
        ]);
        let order: Vec<&str> = p
            .ordered_actions()
            .iter()
            .map(|a| a.summary.as_str())
            .collect();
        assert_eq!(
            order,
            ["folder", "repair", "sessions", "memory", "settings"]
        );
    }

    #[test]
    fn a_plan_is_blocked_by_its_own_or_any_adapters_blockers() {
        let blocker = Blocker {
            code: "target-exists".into(),
            message: "x".into(),
            remedy: None,
        };
        let mut p = plan(vec![section("agent", &[])]);
        assert!(!p.is_blocked());
        p.adapters[0].blockers.push(blocker.clone());
        assert!(p.is_blocked());
        p.adapters[0].blockers.clear();
        p.blockers.push(blocker);
        assert_eq!(p.all_blockers().count(), 1);
    }

    #[test]
    fn opt_ins_have_their_phrases_and_flags() {
        assert_eq!(OptInId::RewriteSessions.phrase(), "rewrite sessions");
        assert_eq!(OptInId::RewriteHistory.phrase(), "rewrite history");
        assert_eq!(OptInId::RewriteSessions.flag(), "--rewrite-sessions");
        assert_eq!(OptInId::RewriteHistory.flag(), "--rewrite-history");
    }

    #[test]
    fn round_trips_through_json_with_kebab_case_tags() {
        let mapping = MappingSpec {
            source: "/a".into(),
            target: "/b".into(),
            style: PathStyle::Posix,
            case: CaseRule::Sensitive,
        };
        let action = Action::RunTool {
            program: "git".into(),
            args: vec!["worktree".into(), "repair".into()],
            cwd: "/b".into(),
            inverse: Some(Box::new(Action::SpliceJsonlField {
                file: "/f.jsonl".into(),
                field: "cwd".into(),
                mapping,
            })),
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["kind"], "run-tool");
        assert_eq!(json["inverse"]["kind"], "splice-jsonl-field");
        assert_eq!(serde_json::from_value::<Action>(json).unwrap(), action);
        assert_eq!(
            serde_json::to_value(Phase::CarryEditorState).unwrap(),
            "carry-editor-state"
        );
    }

    #[test]
    fn mapping_specs_rebuild_the_mapping() {
        let mapping = crate::paths::PathMapping::new(
            r"C:\a",
            r"D:\b",
            PathStyle::Windows,
            CaseRule::Insensitive,
        )
        .unwrap();
        let rebuilt = crate::paths::PathMapping::from_spec(&mapping.spec()).unwrap();
        assert_eq!(
            (rebuilt.source(), rebuilt.target()),
            (mapping.source(), mapping.target())
        );
    }
}
