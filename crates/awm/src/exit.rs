//! Exit codes, mapped in one place (requirements.md, 22.0.2; design.md, "Exit codes").

use std::process::ExitCode;

use awm_core::plan::Outcome;

/// How the process exits. Each variant's number is part of the tool's interface:
/// scripts and the agent skill rely on it, so none may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Everything planned was done and verified.
    Success,
    /// Done, with warnings, things that couldn't be done, or unexpected leftovers.
    Warnings,
    /// The command line was wrong.
    Usage,
    /// Nothing changed, by choice: plan shown, dry run, declined, or no terminal and
    /// no `--yes`.
    NothingChanged,
    /// Checks found blockers, and nothing changed.
    Blocked,
    /// It failed, and was rolled back automatically.
    RolledBack,
    /// It failed or was interrupted, and needs `resume` or `rollback`.
    NeedsAttention,
    /// A bug in the tool. The state is as the record says.
    Internal,
}

impl Exit {
    /// The number the process exits with.
    pub fn code(self) -> u8 {
        match self {
            Exit::Success => 0,
            Exit::Warnings => 1,
            Exit::Usage => 2,
            Exit::NothingChanged => 3,
            Exit::Blocked => 4,
            Exit::RolledBack => 5,
            Exit::NeedsAttention => 6,
            Exit::Internal => 70,
        }
    }
}

impl From<Outcome> for Exit {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Succeeded => Exit::Success,
            Outcome::SucceededWithWarnings => Exit::Warnings,
            Outcome::NothingChanged => Exit::NothingChanged,
            Outcome::Blocked => Exit::Blocked,
            Outcome::RolledBack => Exit::RolledBack,
            Outcome::NeedsAttention => Exit::NeedsAttention,
        }
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_the_documented_table() {
        let table = [
            (Exit::Success, 0),
            (Exit::Warnings, 1),
            (Exit::Usage, 2),
            (Exit::NothingChanged, 3),
            (Exit::Blocked, 4),
            (Exit::RolledBack, 5),
            (Exit::NeedsAttention, 6),
            (Exit::Internal, 70),
        ];
        for (exit, code) in table {
            assert_eq!(exit.code(), code, "{exit:?}");
        }
    }

    #[test]
    fn every_outcome_has_its_exit() {
        assert_eq!(Exit::from(Outcome::Succeeded), Exit::Success);
        assert_eq!(Exit::from(Outcome::SucceededWithWarnings), Exit::Warnings);
        assert_eq!(Exit::from(Outcome::NothingChanged), Exit::NothingChanged);
        assert_eq!(Exit::from(Outcome::Blocked), Exit::Blocked);
        assert_eq!(Exit::from(Outcome::RolledBack), Exit::RolledBack);
        assert_eq!(Exit::from(Outcome::NeedsAttention), Exit::NeedsAttention);
        let _: ExitCode = Exit::Blocked.into();
    }
}
