//! A plan and a report with something in every field, for tests of rendering and of
//! the published schemas.

use awm_core::paths::{CaseRule, MappingSpec, PathStyle};
use awm_core::plan::*;
use awm_core::platform::HoldKind;

/// A mapping from `/a/repo` to `/b/repo`.
fn mapping() -> MappingSpec {
    MappingSpec {
        source: "/a/repo".into(),
        target: "/b/repo".into(),
        style: PathStyle::Posix,
        case: CaseRule::Sensitive,
    }
}

/// A planned action in `phase`.
fn action(phase: Phase, summary: &str, action: Action, opt_in: Option<OptInId>) -> PlannedAction {
    PlannedAction {
        phase,
        adapter: "claude-code".into(),
        summary: summary.into(),
        action,
        preconditions: vec![],
        opt_in,
    }
}

/// A plan that uses every kind of action, precondition, edit and destination.
pub fn plan() -> Plan {
    let edits = vec![
        JsonEdit::RenameKey {
            object: "/projects".into(),
            from: "/a/repo".into(),
            to: "/b/repo".into(),
        },
        JsonEdit::ReplaceString {
            at: "/githubRepoPaths/o~1r/0".into(),
            from: "/a/repo".into(),
            to: "/b/repo".into(),
        },
        JsonEdit::RewritePathStrings {
            at: "".into(),
            mapping: mapping(),
        },
    ];
    let mut carry = action(
        Phase::CarryAgentData,
        "carry 3 session folders",
        Action::MergeDir {
            from: "/h/.claude/projects/-a-repo".into(),
            to: Destination::Path {
                path: "/h/.claude/projects/-b-repo".into(),
            },
        },
        None,
    );
    carry.preconditions = vec![
        Precondition::DirUnchanged {
            path: "/h/.claude/projects/-a-repo".into(),
            listing: "ab12".into(),
        },
        Precondition::Absent {
            path: "/b/repo".into(),
        },
        Precondition::Present {
            path: "/a/repo".into(),
        },
        Precondition::FileUnchanged {
            path: "/h/.claude.json".into(),
            size: 10,
            blake3: "cd34".into(),
        },
    ];
    let actions = vec![
        action(
            Phase::MoveWorkspace,
            "move the folder",
            Action::MoveTree {
                from: "/a/repo".into(),
                to: "/b/repo".into(),
            },
            None,
        ),
        carry,
        action(
            Phase::CarryEditorState,
            "re-key workspace storage",
            Action::MergeDir {
                from: "/h/ws/old".into(),
                to: Destination::Deferred {
                    adapter: "vscode".into(),
                    key: "/b/repo".into(),
                },
            },
            None,
        ),
        action(
            Phase::CarrySettings,
            "carry settings",
            Action::EditJsonFile {
                file: "/h/.claude.json".into(),
                edits: edits.clone(),
            },
            None,
        ),
        action(
            Phase::CarryEditorState,
            "update the recent list",
            Action::EditSqliteJsonRow {
                db: "/h/state.vscdb".into(),
                table: "ItemTable".into(),
                key: "history.recentlyOpenedPathsList".into(),
                edits,
            },
            None,
        ),
        action(
            Phase::OptIn,
            "rewrite recorded paths",
            Action::SpliceJsonlField {
                file: "/h/s.jsonl".into(),
                field: "cwd".into(),
                mapping: mapping(),
            },
            Some(OptInId::RewriteSessions),
        ),
        action(
            Phase::UpdateWorkspacePaths,
            "update config.json",
            Action::ReplaceInText {
                file: "/b/repo/config.json".into(),
                mapping: mapping(),
                matches: vec![TextSpan { start: 4, end: 11 }],
            },
            None,
        ),
        action(
            Phase::RepairVcs,
            "repair 1 worktree",
            Action::RunTool {
                program: "git".into(),
                args: vec!["worktree".into(), "repair".into()],
                cwd: "/b/repo".into(),
                inverse: Some(Box::new(Action::RunTool {
                    program: "git".into(),
                    args: vec![],
                    cwd: "/a/repo".into(),
                    inverse: None,
                })),
            },
            None,
        ),
    ];
    let section = AdapterSection {
        adapter: AdapterInfo {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            kind: AdapterKind::Agent,
            verified: vec![Verification {
                versions: "2.1.309".into(),
                os: "Windows 11".into(),
            }],
            recognised: Some(">=2.1.0, <2.2.0".into()),
        },
        found_version: Some("2.1.309".into()),
        actions,
        left_unchanged: vec![LeftUnchanged {
            what: "recorded paths in sessions".into(),
            count: 15,
            changed_by: Some("--rewrite-sessions".into()),
            expected_leftover: Some(LeftoverPattern {
                files: "projects/*/*.jsonl".into(),
                field: Some("cwd".into()),
            }),
        }],
        opt_ins: vec![OptInOffer {
            id: OptInId::RewriteSessions,
            phrase: "rewrite sessions".into(),
            counts: vec![FileCount {
                file: "/h/s.jsonl".into(),
                count: 15,
            }],
            requested: true,
            applied: false,
        }],
        blockers: vec![Blocker {
            code: "agent-running".into(),
            message: "Claude Code is running (2 processes)".into(),
            remedy: Some(
                "Close every Claude Code session, or finish the move later with --when-free."
                    .into(),
            ),
        }],
        notices: vec![Notice {
            code: "version-unverified".into(),
            message: "2.1.310 is recognised but not verified".into(),
        }],
        detected: vec![Notice {
            code: "python-venv".into(),
            message: "Python virtual environment in .venv".into(),
        }],
        counts: vec![Count {
            label: "sessions the picker lists".into(),
            value: 2,
        }],
    };
    let detector = AdapterSection {
        adapter: AdapterInfo {
            id: "detectors".into(),
            name: "Detectors".into(),
            kind: AdapterKind::Detector,
            verified: vec![],
            recognised: None,
        },
        found_version: None,
        actions: vec![],
        left_unchanged: vec![],
        opt_ins: vec![],
        blockers: vec![],
        notices: vec![],
        detected: vec![],
        counts: vec![],
    };
    let kinds = [
        (AdapterKind::VersionControl, "git"),
        (AdapterKind::Editor, "vscode"),
    ];
    let mut adapters = vec![section, detector];
    for (kind, id) in kinds {
        adapters.push(AdapterSection {
            adapter: AdapterInfo {
                id: id.into(),
                name: id.into(),
                kind,
                verified: vec![],
                recognised: None,
            },
            ..adapters[1].clone()
        });
    }
    Plan {
        schema: PLAN_SCHEMA.into(),
        command: PlanCommand::Move,
        source: "/a/repo".into(),
        target: "/b/repo".into(),
        volume_relation: VolumeRelation::Same,
        holders: PlanHolders {
            holders: vec![HolderEntry {
                pid: 42,
                name: "claude.exe".into(),
                exe: Some("/bin/claude".into()),
                kind: HoldKind::WorkingFolder,
                this_session: true,
            }],
            uninspectable: 3,
            complete: true,
        },
        blockers: vec![],
        notices: vec![Notice {
            code: "large-workspace".into(),
            message: "The workspace has 120,000 files.".into(),
        }],
        adapters,
    }
}

/// A report with something in every section.
pub fn report() -> Report {
    Report {
        schema: REPORT_SCHEMA.into(),
        move_id: Some("20261023-095701-abcd".into()),
        outcome: Outcome::SucceededWithWarnings,
        done: vec![ReportItem {
            adapter: "core".into(),
            summary: "moved the folder".into(),
            code: None,
        }],
        left_unchanged: vec![LeftUnchanged {
            what: "prompts in history".into(),
            count: 4,
            changed_by: Some("--rewrite-history".into()),
            expected_leftover: None,
        }],
        couldnt_do: vec![ReportItem {
            adapter: "git".into(),
            summary: "git isn't installed; run git worktree repair".into(),
            code: Some("git-missing".into()),
        }],
        warnings: vec![Notice {
            code: "unexpected-leftover".into(),
            message: "1 file still names the old path".into(),
        }],
    }
}
