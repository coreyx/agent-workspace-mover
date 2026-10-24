//! Human-readable plans and reports (requirements.md, REQ-3 and 1.0.6).
//!
//! The plan shows what will happen, grouped by adapter with counts; what's deliberately
//! left unchanged, with the flag or opt-in that would change it; what's detected but not
//! handled; and what stops the real run, with the remedy for each. A report always shows
//! its "done", "left unchanged" and "couldn't do" sections, even when empty (1.6).

use std::fmt::Write;

use awm_core::plan::{
    AdapterSection, HolderEntry, LeftUnchanged, Plan, PlanCommand, Report, VolumeRelation,
};
use awm_core::platform::HoldKind;

/// The text written for an empty section.
const NONE: &str = "  none";

/// A plan as the user reads it before going ahead.
pub fn plan(plan: &Plan) -> String {
    let mut out = String::new();
    let verb = match plan.command {
        PlanCommand::Move => "Move",
        PlanCommand::Rescue => "Rescue",
        PlanCommand::Undo => "Undo",
    };
    let how = match plan.volume_relation {
        VolumeRelation::Same => "on the same volume, so the folder moves with one rename",
        VolumeRelation::Different => {
            "across volumes, so the folder is copied, verified, then removed"
        }
        VolumeRelation::NoFolderMove => "with no folder to move",
    };
    let _ = writeln!(
        out,
        "{verb} {} -> {}",
        plan.source.display(),
        plan.target.display()
    );
    let _ = writeln!(out, "  ({how})");

    if !plan.holders.holders.is_empty() || !plan.holders.complete || plan.holders.uninspectable > 0
    {
        let _ = writeln!(out, "\nHolding the folder:");
        for holder in &plan.holders.holders {
            let _ = writeln!(out, "  - {}", holder_line(holder));
        }
        if !plan.holders.complete {
            let _ = writeln!(
                out,
                "  The search didn't cover everything, so something else may be holding it too."
            );
        }
        if plan.holders.uninspectable > 0 {
            let _ = writeln!(
                out,
                "  {} processes couldn't be inspected.",
                plan.holders.uninspectable
            );
        }
    }

    let _ = writeln!(out, "\nWill do:");
    let doing: Vec<&AdapterSection> = plan
        .adapters
        .iter()
        .filter(|s| !s.actions.is_empty() || !s.counts.is_empty())
        .collect();
    if doing.is_empty() {
        let _ = writeln!(out, "{NONE}");
    }
    for section in doing {
        let _ = writeln!(out, "  {}", section_title(section));
        for action in &section.actions {
            let suffix = action
                .opt_in
                .map(|o| format!(" (only once you type '{}')", o.phrase()))
                .unwrap_or_default();
            let _ = writeln!(out, "    - {}{suffix}", action.summary);
        }
        for count in &section.counts {
            let _ = writeln!(out, "    {}: {}", count.label, count.value);
        }
    }

    let left: Vec<&LeftUnchanged> = plan
        .adapters
        .iter()
        .flat_map(|s| &s.left_unchanged)
        .collect();
    let _ = writeln!(out, "\nLeft unchanged:");
    write_left_unchanged(&mut out, &left);

    let detected: Vec<String> = plan
        .adapters
        .iter()
        .flat_map(|s| {
            s.detected
                .iter()
                .map(move |n| format!("{}: {}", s.adapter.name, n.message))
        })
        .collect();
    if !detected.is_empty() {
        let _ = writeln!(out, "\nDetected, not handled:");
        for line in detected {
            let _ = writeln!(out, "  - {line}");
        }
    }

    let notices: Vec<String> = plan
        .notices
        .iter()
        .map(|n| n.message.clone())
        .chain(plan.adapters.iter().flat_map(|s| {
            s.notices
                .iter()
                .map(move |n| format!("{}: {}", s.adapter.name, n.message))
        }))
        .collect();
    if !notices.is_empty() {
        let _ = writeln!(out, "\nNotices:");
        for line in notices {
            let _ = writeln!(out, "  - {line}");
        }
    }

    let _ = writeln!(out, "\nBlockers:");
    let mut any = false;
    for blocker in plan.all_blockers() {
        any = true;
        let _ = writeln!(out, "  - [{}] {}", blocker.code, blocker.message);
        if let Some(remedy) = &blocker.remedy {
            let _ = writeln!(out, "    What to do: {remedy}");
        }
    }
    if !any {
        let _ = writeln!(out, "{NONE}");
    }
    out
}

/// A report as the user reads it after a run.
pub fn report(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Done:");
    if report.done.is_empty() {
        let _ = writeln!(out, "{NONE}");
    }
    for item in &report.done {
        let _ = writeln!(out, "  - {}", item.summary);
    }
    let _ = writeln!(out, "\nLeft unchanged:");
    write_left_unchanged(&mut out, &report.left_unchanged.iter().collect::<Vec<_>>());
    let _ = writeln!(out, "\nCouldn't do:");
    if report.couldnt_do.is_empty() {
        let _ = writeln!(out, "{NONE}");
    }
    for item in &report.couldnt_do {
        let code = item
            .code
            .as_deref()
            .map(|c| format!("[{c}] "))
            .unwrap_or_default();
        let _ = writeln!(out, "  - {code}{}", item.summary);
    }
    if !report.warnings.is_empty() {
        let _ = writeln!(out, "\nWarnings:");
        for warning in &report.warnings {
            let _ = writeln!(out, "  - [{}] {}", warning.code, warning.message);
        }
    }
    if let Some(id) = &report.move_id {
        let _ = writeln!(
            out,
            "\nRecorded as move {id}. To reverse it: agent-workspace-mover undo {id}"
        );
    }
    out
}

/// An adapter's heading: its name, and the version found.
fn section_title(section: &AdapterSection) -> String {
    match &section.found_version {
        Some(version) => format!("{} {version}", section.adapter.name),
        None => section.adapter.name.clone(),
    }
}

/// One holder: what it is, how it holds the folder, and whether it's this session.
fn holder_line(holder: &HolderEntry) -> String {
    let how = match holder.kind {
        HoldKind::WorkingFolder => "working in the folder",
        HoldKind::OpenFile => "has a file in the folder open",
    };
    let session = if holder.this_session {
        " (this session)"
    } else {
        ""
    };
    format!("{} (process {}), {how}{session}", holder.name, holder.pid)
}

/// The "left unchanged" lines, with what would change each.
fn write_left_unchanged(out: &mut String, items: &[&LeftUnchanged]) {
    if items.is_empty() {
        let _ = writeln!(out, "{NONE}");
    }
    for item in items {
        let by = item
            .changed_by
            .as_deref()
            .map(|f| format!(" ({f} would change them)"))
            .unwrap_or_default();
        let _ = writeln!(out, "  - {} {}{by}", item.count, item.what);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples;

    #[test]
    fn a_plan_shows_every_section_it_has() {
        let text = plan(&samples::plan());
        for expected in [
            "Move /a/repo -> /b/repo",
            "one rename",
            "Holding the folder:",
            "claude.exe (process 42), working in the folder (this session)",
            "3 processes couldn't be inspected.",
            "Will do:",
            "Claude Code 2.1.309",
            "- carry 3 session folders",
            "- rewrite recorded paths (only once you type 'rewrite sessions')",
            "sessions the picker lists: 2",
            "Left unchanged:",
            "- 15 recorded paths in sessions (--rewrite-sessions would change them)",
            "Detected, not handled:",
            "Python virtual environment",
            "Notices:",
            "Blockers:",
            "[agent-running] Claude Code is running",
            "What to do: Close every Claude Code session",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }

    #[test]
    fn an_empty_plan_says_none_rather_than_leaving_sections_out() {
        let mut empty = samples::plan();
        for section in &mut empty.adapters {
            section.actions.clear();
            section.counts.clear();
            section.left_unchanged.clear();
            section.blockers.clear();
            section.detected.clear();
            section.notices.clear();
        }
        empty.holders = Default::default();
        empty.holders.complete = true;
        empty.notices.clear();
        empty.volume_relation = VolumeRelation::Different;
        let text = plan(&empty);
        assert!(text.contains("Will do:\n  none"));
        assert!(text.contains("Left unchanged:\n  none"));
        assert!(text.contains("Blockers:\n  none"));
        assert!(text.contains("copied, verified, then removed"));
        assert!(!text.contains("Holding the folder"));
    }

    #[test]
    fn an_incomplete_search_says_so() {
        let mut p = samples::plan();
        p.holders.complete = false;
        p.command = PlanCommand::Rescue;
        p.volume_relation = VolumeRelation::NoFolderMove;
        let text = plan(&p);
        assert!(text.contains("didn't cover everything"));
        assert!(text.starts_with("Rescue "));
        p.command = PlanCommand::Undo;
        assert!(plan(&p).starts_with("Undo "));
    }

    // 1.6: a report's three sections are always present, even when empty.
    #[test]
    fn a_report_always_has_its_three_sections() {
        let mut r = samples::report();
        let text = report(&r);
        for expected in [
            "Done:",
            "- moved the folder",
            "Left unchanged:",
            "Couldn't do:",
            "[git-missing]",
            "Warnings:",
            "undo 20261023-095701-abcd",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        r.done.clear();
        r.left_unchanged.clear();
        r.couldnt_do.clear();
        r.warnings.clear();
        r.move_id = None;
        let text = report(&r);
        assert_eq!(
            text,
            "Done:\n  none\n\nLeft unchanged:\n  none\n\nCouldn't do:\n  none\n"
        );
    }

    #[test]
    fn open_file_holders_say_so() {
        let holder = HolderEntry {
            pid: 7,
            name: "code".into(),
            exe: None,
            kind: HoldKind::OpenFile,
            this_session: false,
        };
        assert_eq!(
            holder_line(&holder),
            "code (process 7), has a file in the folder open"
        );
    }
}
