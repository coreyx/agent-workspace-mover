//! The adapter interface, and assembling a plan from what adapters contribute
//! (requirements.md, REQ-24; design.md, "Key design decisions", 1).
//!
//! An adapter holds everything one agent, version-control system, editor or detector
//! knows about the machine. It reads, and describes what should change, as data; it
//! never changes anything itself. The core gathers every adapter's [`Contribution`]
//! into one [`Plan`], and only the executor acts on it. Adding a tool means adding an
//! adapter, with no change to the core (24.0.1).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::locations::Locations;
use crate::paths::PathMapping;
use crate::plan::{
    AdapterInfo, AdapterSection, Blocker, Count, LeftUnchanged, LeftoverPattern, Notice, OptInId,
    OptInOffer, PLAN_SCHEMA, Plan, PlanCommand, PlanHolders, PlannedAction, VolumeRelation,
};
use crate::platform::{Platform, ProcessRef, ProcessSnapshot};

/// What a plan is being made for.
#[derive(Clone, Debug)]
pub struct MoveRequest {
    /// Move, rescue or undo.
    pub command: PlanCommand,
    /// The workspace's path now (for a rescue, its old path).
    pub source: PathBuf,
    /// Its path after the move (for a rescue, where it is now).
    pub target: PathBuf,
    /// The source and target, ready to find and replace in recorded text.
    pub mapping: PathMapping,
    /// Leave per-project settings behind (`--fresh-settings`, 7.0.3).
    pub fresh_settings: bool,
    /// The opt-ins the user asked for by flag. Asking isn't confirming (8.0.3).
    pub requested_opt_ins: Vec<OptInId>,
    /// Adapters allowed to change data from versions they don't recognise (24.0.3).
    pub allow_unverified: Vec<String>,
}

impl MoveRequest {
    /// Whether the user asked for `opt_in` by its flag.
    pub fn requested(&self, opt_in: OptInId) -> bool {
        self.requested_opt_ins.contains(&opt_in)
    }
}

/// What an adapter can see while it inspects the machine.
#[derive(Clone, Copy)]
pub struct InspectContext<'a> {
    /// Where things live.
    pub locations: &'a Locations,
    /// The OS.
    pub platform: &'a dyn Platform,
    /// The processes running when planning began.
    pub processes: &'a ProcessSnapshot,
}

/// What one adapter adds to a plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contribution {
    /// The tool's version found on this machine.
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
    /// Path-keyed state found but not handled in version 1.
    pub detected: Vec<Notice>,
    /// Counts for the plan to show.
    pub counts: Vec<Count>,
}

/// Why an adapter couldn't do something. The message is the tool's own words, never
/// text read from agent data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterError {
    /// What went wrong.
    pub message: String,
}

impl AdapterError {
    /// An error with `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AdapterError {}

/// One check an adapter makes after a move (16.0.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyCheck {
    /// What was checked.
    pub what: String,
    /// Whether it passed.
    pub passed: bool,
}

/// Where the leftover search should look, and what it should expect there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeftoverSpec {
    /// The folders and files to search.
    pub roots: Vec<PathBuf>,
    /// Leftovers the plan expects, which aren't warnings.
    pub expected: Vec<LeftoverPattern>,
}

/// Agent data whose recorded folder no longer exists (REQ-18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Orphan {
    /// The adapter that owns it.
    pub adapter: String,
    /// The folder it was recorded for.
    pub recorded_path: String,
    /// Where the data is.
    pub location: PathBuf,
}

/// What identifies a workspace wherever it is (REQ-17).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Its root commits, for git.
    pub root_commits: Vec<String>,
    /// Its remotes, normalised, without credentials.
    pub remotes: Vec<String>,
}

/// One agent, version-control system, editor or detector's knowledge of the machine.
/// Implementations never change anything; they describe what should change.
pub trait Adapter: Send + Sync {
    /// Who the adapter is, and what it's verified against.
    fn info(&self) -> AdapterInfo;

    /// Reads the machine, read only, and contributes to the plan for `request`.
    fn inspect(
        &self,
        cx: &InspectContext<'_>,
        request: &MoveRequest,
    ) -> Result<Contribution, AdapterError>;

    /// Resolves a destination this adapter deferred until after the folder moved.
    fn resolve(&self, cx: &InspectContext<'_>, key: &str) -> Result<PathBuf, AdapterError> {
        let _ = (cx, key);
        Err(AdapterError::new(format!(
            "{} defers no destinations",
            self.info().id
        )))
    }

    /// Checks the result after a move.
    fn verify(&self, cx: &InspectContext<'_>, request: &MoveRequest) -> Vec<VerifyCheck> {
        let _ = (cx, request);
        Vec::new()
    }

    /// Where the leftover search should look, and what it should expect.
    fn leftovers(&self, cx: &InspectContext<'_>, section: &AdapterSection) -> LeftoverSpec {
        let _ = (cx, section);
        LeftoverSpec::default()
    }

    /// This adapter's program's running processes, which block changes to its data.
    fn running(&self, processes: &ProcessSnapshot) -> Vec<ProcessRef> {
        let _ = processes;
        Vec::new()
    }

    /// Data whose recorded folder no longer exists.
    fn orphans(&self, cx: &InspectContext<'_>) -> Result<Vec<Orphan>, AdapterError> {
        let _ = cx;
        Ok(Vec::new())
    }

    /// The identity of the workspace at `folder`, for version-control adapters.
    fn identity(&self, cx: &InspectContext<'_>, folder: &Path) -> Option<Identity> {
        let _ = (cx, folder);
        None
    }
}

/// What the core's own checks found, before adapters add theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreFindings {
    /// Whether the folder moves by rename, copy, or not at all.
    pub volume_relation: VolumeRelation,
    /// What holds the source.
    pub holders: PlanHolders,
    /// The core's blockers.
    pub blockers: Vec<Blocker>,
    /// The core's notices.
    pub notices: Vec<Notice>,
    /// The core's own actions, such as moving the folder.
    pub actions: Vec<PlannedAction>,
}

/// The id the core's own section has in a plan.
pub const CORE_ADAPTER_ID: &str = "core";

/// Builds the plan: the core's findings, then every adapter's contribution. An adapter
/// that fails to inspect gets a blocker for its changes, rather than stopping the plan,
/// so the user still sees everything else (2.0.9).
pub fn assemble(
    adapters: &[&dyn Adapter],
    cx: &InspectContext<'_>,
    request: &MoveRequest,
    core: CoreFindings,
) -> Plan {
    let mut sections = Vec::with_capacity(adapters.len() + 1);
    if !core.actions.is_empty() {
        sections.push(AdapterSection {
            adapter: AdapterInfo {
                id: CORE_ADAPTER_ID.into(),
                name: "agent-workspace-mover".into(),
                kind: crate::plan::AdapterKind::Detector,
                verified: vec![],
                recognised: None,
            },
            found_version: None,
            actions: core.actions,
            left_unchanged: vec![],
            opt_ins: vec![],
            blockers: vec![],
            notices: vec![],
            detected: vec![],
            counts: vec![],
        });
    }
    for adapter in adapters {
        let info = adapter.info();
        let contribution = adapter
            .inspect(cx, request)
            .unwrap_or_else(|error| Contribution {
                blockers: vec![Blocker {
                    code: "adapter-failed".into(),
                    message: format!("{} couldn't be checked: {error}", info.name),
                    remedy: Some(
                        "Nothing it covers will change. Fix the cause, then plan again.".into(),
                    ),
                }],
                ..Contribution::default()
            });
        sections.push(AdapterSection {
            adapter: info,
            found_version: contribution.found_version,
            actions: contribution.actions,
            left_unchanged: contribution.left_unchanged,
            opt_ins: contribution.opt_ins,
            blockers: contribution.blockers,
            notices: contribution.notices,
            detected: contribution.detected,
            counts: contribution.counts,
        });
    }
    Plan {
        schema: PLAN_SCHEMA.into(),
        command: request.command,
        source: request.source.clone(),
        target: request.target.clone(),
        volume_relation: core.volume_relation,
        holders: core.holders,
        blockers: core.blockers,
        notices: core.notices,
        adapters: sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{CaseRule, PathStyle};
    use crate::plan::{Action, AdapterKind, Phase};
    use crate::platform::Host;

    /// An adapter whose inspection returns `result`.
    struct Fixed {
        id: &'static str,
        result: Result<Contribution, AdapterError>,
    }

    impl Adapter for Fixed {
        fn info(&self) -> AdapterInfo {
            AdapterInfo {
                id: self.id.into(),
                name: self.id.into(),
                kind: AdapterKind::Agent,
                verified: vec![],
                recognised: None,
            }
        }

        fn inspect(
            &self,
            _: &InspectContext<'_>,
            _: &MoveRequest,
        ) -> Result<Contribution, AdapterError> {
            self.result.clone()
        }
    }

    /// A request to move `/a/repo` to `/b/repo`.
    fn request() -> MoveRequest {
        MoveRequest {
            command: PlanCommand::Move,
            source: "/a/repo".into(),
            target: "/b/repo".into(),
            mapping: PathMapping::new("/a/repo", "/b/repo", PathStyle::Posix, CaseRule::Sensitive)
                .unwrap(),
            fresh_settings: false,
            requested_opt_ins: vec![OptInId::RewriteHistory],
            allow_unverified: vec![],
        }
    }

    /// A core finding with one folder move.
    fn core() -> CoreFindings {
        CoreFindings {
            volume_relation: VolumeRelation::Same,
            holders: PlanHolders {
                complete: true,
                ..PlanHolders::default()
            },
            blockers: vec![],
            notices: vec![],
            actions: vec![PlannedAction {
                phase: Phase::MoveWorkspace,
                adapter: CORE_ADAPTER_ID.into(),
                summary: "move the folder".into(),
                action: Action::MoveTree {
                    from: "/a/repo".into(),
                    to: "/b/repo".into(),
                },
                preconditions: vec![],
                opt_in: None,
            }],
        }
    }

    /// Assembles a plan from `adapters` on the host, with fixture-free locations.
    fn assemble_with(adapters: &[&dyn Adapter]) -> Plan {
        let locations = Locations::from_env().unwrap();
        let processes = ProcessSnapshot::from_processes(vec![], 1);
        let cx = InspectContext {
            locations: &locations,
            platform: &Host,
            processes: &processes,
        };
        assemble(adapters, &cx, &request(), core())
    }

    #[test]
    fn gathers_the_core_and_each_adapter_into_one_plan() {
        let ok = Fixed {
            id: "agent",
            result: Ok(Contribution {
                found_version: Some("2.1.309".into()),
                ..Contribution::default()
            }),
        };
        let plan = assemble_with(&[&ok]);
        assert_eq!(plan.schema, PLAN_SCHEMA);
        let ids: Vec<&str> = plan
            .adapters
            .iter()
            .map(|a| a.adapter.id.as_str())
            .collect();
        assert_eq!(ids, [CORE_ADAPTER_ID, "agent"]);
        assert_eq!(plan.adapters[1].found_version.as_deref(), Some("2.1.309"));
        assert_eq!(plan.ordered_actions().len(), 1);
        assert!(!plan.is_blocked());
    }

    #[test]
    fn an_adapter_that_fails_blocks_only_its_own_changes() {
        let failing = Fixed {
            id: "editor",
            result: Err(AdapterError::new("its data folder is unreadable")),
        };
        let ok = Fixed {
            id: "agent",
            result: Ok(Contribution::default()),
        };
        let plan = assemble_with(&[&failing, &ok]);
        let editor = plan
            .adapters
            .iter()
            .find(|a| a.adapter.id == "editor")
            .unwrap();
        assert_eq!(editor.blockers[0].code, "adapter-failed");
        assert!(
            editor.blockers[0]
                .message
                .contains("its data folder is unreadable")
        );
        assert!(
            plan.adapters.iter().any(|a| a.adapter.id == "agent"),
            "the others still appear"
        );
        assert!(plan.is_blocked());
    }

    #[test]
    fn a_plan_with_no_core_actions_has_no_core_section() {
        let mut findings = core();
        findings.actions.clear();
        let locations = Locations::from_env().unwrap();
        let processes = ProcessSnapshot::from_processes(vec![], 1);
        let cx = InspectContext {
            locations: &locations,
            platform: &Host,
            processes: &processes,
        };
        assert!(assemble(&[], &cx, &request(), findings).adapters.is_empty());
    }

    #[test]
    fn default_methods_describe_nothing() {
        let adapter = Fixed {
            id: "plain",
            result: Ok(Contribution::default()),
        };
        let locations = Locations::from_env().unwrap();
        let processes = ProcessSnapshot::from_processes(vec![], 1);
        let cx = InspectContext {
            locations: &locations,
            platform: &Host,
            processes: &processes,
        };
        assert!(
            adapter
                .resolve(&cx, "key")
                .unwrap_err()
                .to_string()
                .contains("plain")
        );
        assert!(adapter.verify(&cx, &request()).is_empty());
        assert_eq!(adapter.running(&processes), vec![]);
        assert!(adapter.orphans(&cx).unwrap().is_empty());
        assert!(adapter.identity(&cx, Path::new("/a")).is_none());
        let plan = assemble_with(&[&adapter]);
        assert_eq!(
            adapter.leftovers(&cx, &plan.adapters[1]),
            LeftoverSpec::default()
        );
        assert!(request().requested(OptInId::RewriteHistory));
        assert!(!request().requested(OptInId::RewriteSessions));
    }
}
