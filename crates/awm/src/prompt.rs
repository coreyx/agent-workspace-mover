//! Asking the user, at a terminal (requirements.md, 3.0.5).
//!
//! A [`Terminal`] can only be opened when both standard input and standard output are
//! terminals, so a script, a pipe or an agent driving the tool never gets one, and so
//! can never be asked or answer (3.0.7). Commands take any [`Prompter`], so tests can
//! answer for the user.

use std::io::{self, BufRead, IsTerminal, Write};

/// Something that can ask the user a question and read their answer.
pub trait Prompter {
    /// Asks `question` and returns whether the user answered yes. Anything other than
    /// "y" or "yes" is a no.
    fn confirm(&mut self, question: &str) -> io::Result<bool>;
}

/// The user's terminal.
#[derive(Debug)]
pub struct Terminal {
    _private: (),
}

impl Terminal {
    /// The terminal, when both standard input and standard output are one.
    pub fn open() -> Option<Self> {
        (io::stdin().is_terminal() && io::stdout().is_terminal()).then_some(Self { _private: () })
    }
}

impl Prompter for Terminal {
    fn confirm(&mut self, question: &str) -> io::Result<bool> {
        ask(question, &mut io::stdin().lock(), &mut io::stdout().lock())
    }
}

/// Writes `question` with a `[y/N]` hint to `out`, reads one line from `input`, and
/// says whether it was a yes.
fn ask(question: &str, input: &mut impl BufRead, out: &mut impl Write) -> io::Result<bool> {
    write!(out, "{question} [y/N] ")?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(is_yes(&answer))
}

/// Whether `answer` means yes: "y" or "yes", in any case, with surrounding space ignored.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_with_a_hint_and_reads_one_line() {
        let mut out = Vec::new();
        assert!(
            ask(
                "Go ahead?",
                &mut io::Cursor::new(b"yes\nignored\n".to_vec()),
                &mut out
            )
            .unwrap()
        );
        assert_eq!(out, b"Go ahead? [y/N] ");
        assert!(
            !ask(
                "Go ahead?",
                &mut io::Cursor::new(b"\n".to_vec()),
                &mut Vec::new()
            )
            .unwrap()
        );
        assert!(
            !ask(
                "Go ahead?",
                &mut io::Cursor::new(Vec::new()),
                &mut Vec::new()
            )
            .unwrap(),
            "no input is a no"
        );
    }

    #[test]
    fn only_y_or_yes_means_yes() {
        for yes in ["y", "Y", "yes", "YES", " yes\n", "Yes\r\n"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "n", "no", "yep", "sure", "ok", "y es", "\n"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    #[test]
    fn a_test_run_has_no_terminal() {
        // Tests run with their output captured, so this is never a terminal.
        assert!(Terminal::open().is_none() || io::stdout().is_terminal());
    }
}
