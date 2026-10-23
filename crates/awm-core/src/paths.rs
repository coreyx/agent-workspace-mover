//! The path model: one workspace path, every form tools record it in, and how to find
//! and replace it safely (requirements.md, REQ-5; design.md, "Interfaces", path model).
//!
//! A [`PathMapping`] holds a move's source and target. It finds the source wherever it
//! appears in text, in any recorded [`PathForm`], but only as a whole path: a match must
//! start and end at a segment boundary, so `repo` never matches inside `repo-old` or
//! `/mnt/repo` (5.0.3). Each match is replaced by the target in the same form and with
//! the same drive-letter case (5.0.4).
//!
//! The style of path (Windows or POSIX) is explicit rather than taken from the host, so
//! the same rules can be tested on every OS. Case-insensitive matching folds ASCII
//! letters only: two paths that differ only in the case of a non-ASCII letter don't
//! match. That errs towards a missed match, which the leftover search reports, never
//! towards a wrong edit.

use std::fmt;
use std::ops::Range;
use std::path::Path;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

/// Which operating system's rules a path follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    /// Drive-letter paths such as `C:\Users\me\repo`.
    Windows,
    /// Rooted paths such as `/home/me/repo`.
    Posix,
}

impl PathStyle {
    /// The style of the OS this program runs on.
    pub fn host() -> Self {
        if cfg!(windows) {
            PathStyle::Windows
        } else {
            PathStyle::Posix
        }
    }
}

/// Whether two paths that differ only in letter case name the same folder (5.0.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseRule {
    /// Case matters, as on Linux and case-sensitive macOS volumes.
    Sensitive,
    /// Case doesn't matter, as on Windows and most macOS volumes.
    Insensitive,
}

impl CaseRule {
    /// Whether two path segments are the same name under this rule.
    fn same(self, a: &str, b: &str) -> bool {
        match self {
            CaseRule::Sensitive => a == b,
            CaseRule::Insensitive => a.eq_ignore_ascii_case(b),
        }
    }
}

/// A form in which tools are known to record a path (5.0.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathForm {
    /// The OS's own form: `C:\Users\me\repo` or `/home/me/repo`.
    Native,
    /// A Windows path with forward slashes, as `~/.claude.json` keys use: `C:/Users/me/repo`.
    ForwardSlash,
    /// The native form inside a JSON string, so each `\` is written `\\`.
    JsonEscaped,
    /// A Windows path as POSIX-style shells write it: `/c/Users/me/repo`.
    PosixDrive,
    /// A `file://` URI with the path as written: `file:///C:/Users/me/repo`.
    FileUri,
    /// A `file://` URI encoded the way VS Code encodes it: `file:///c%3A/Users/me/repo`.
    FileUriPercentEncoded,
}

impl PathForm {
    /// The forms a path of `style` can be recorded in, in the order patterns are built.
    fn for_style(style: PathStyle) -> &'static [PathForm] {
        match style {
            PathStyle::Windows => &[
                PathForm::Native,
                PathForm::ForwardSlash,
                PathForm::JsonEscaped,
                PathForm::PosixDrive,
                PathForm::FileUri,
                PathForm::FileUriPercentEncoded,
            ],
            PathStyle::Posix => &[
                PathForm::Native,
                PathForm::JsonEscaped,
                PathForm::FileUri,
                PathForm::FileUriPercentEncoded,
            ],
        }
    }

    /// The byte offset of the drive letter in this form's text, for Windows paths.
    fn drive_offset(self) -> usize {
        match self {
            PathForm::Native | PathForm::ForwardSlash | PathForm::JsonEscaped => 0,
            PathForm::PosixDrive => 1,
            PathForm::FileUri | PathForm::FileUriPercentEncoded => "file:///".len(),
        }
    }

    /// Whether this form is a URI, where `?` and `#` also end a path.
    fn is_uri(self) -> bool {
        matches!(self, PathForm::FileUri | PathForm::FileUriPercentEncoded)
    }
}

/// Why a path couldn't be used as a move's source or target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    /// The path isn't absolute in its style.
    NotAbsolute(String),
    /// The path is absolute, but of a kind version 1 doesn't handle, such as a UNC path,
    /// a path containing `.` or `..`, or one that isn't valid Unicode.
    Unsupported(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::NotAbsolute(path) => write!(f, "'{path}' isn't an absolute path"),
            PathError::Unsupported(path) => {
                write!(f, "'{path}' is a kind of path this version can't move")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// How to write a Windows drive letter when rendering a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DriveCase {
    /// As the path was given.
    AsGiven,
    /// Lowercase, as in the text being replaced.
    Lower,
    /// Uppercase, as in the text being replaced.
    Upper,
}

/// An absolute path split into its parts, independent of how it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedPath {
    style: PathStyle,
    /// The drive letter, for Windows paths.
    drive: Option<char>,
    segments: Vec<String>,
}

impl ParsedPath {
    /// Parses an absolute path of `style`, accepting either separator on Windows.
    fn parse(text: &str, style: PathStyle) -> Result<Self, PathError> {
        let (drive, rest) = match style {
            PathStyle::Windows => {
                if text.starts_with("\\\\") || text.starts_with("//") {
                    return Err(PathError::Unsupported(text.to_owned()));
                }
                let mut chars = text.chars();
                let (letter, colon) = (chars.next(), chars.next());
                let rest = chars.as_str();
                match (letter, colon) {
                    (Some(letter), Some(':'))
                        if letter.is_ascii_alphabetic()
                            && (rest.is_empty() || rest.starts_with(['\\', '/'])) =>
                    {
                        (Some(letter), rest)
                    }
                    _ => return Err(PathError::NotAbsolute(text.to_owned())),
                }
            }
            PathStyle::Posix => match text.strip_prefix('/') {
                Some(rest) => (None, rest),
                None => return Err(PathError::NotAbsolute(text.to_owned())),
            },
        };
        let separators: &[char] = match style {
            PathStyle::Windows => &['\\', '/'],
            PathStyle::Posix => &['/'],
        };
        let segments: Vec<String> = rest
            .split(separators)
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        if segments.iter().any(|s| s == "." || s == "..") {
            return Err(PathError::Unsupported(text.to_owned()));
        }
        Ok(Self {
            style,
            drive,
            segments,
        })
    }

    /// The drive letter written in `case`, for Windows paths.
    fn drive_letter(&self, case: DriveCase) -> String {
        let letter = self.drive.unwrap_or('C');
        match case {
            DriveCase::AsGiven => letter.to_string(),
            DriveCase::Lower => letter.to_ascii_lowercase().to_string(),
            DriveCase::Upper => letter.to_ascii_uppercase().to_string(),
        }
    }

    /// This path written in `form`, with its drive letter in `case`.
    fn render(&self, form: PathForm, case: DriveCase) -> String {
        match self.style {
            PathStyle::Windows => {
                let drive = self.drive_letter(case);
                let joined = |separator: &str| {
                    if self.segments.is_empty() {
                        separator.to_owned()
                    } else {
                        self.segments
                            .iter()
                            .map(|s| format!("{separator}{s}"))
                            .collect()
                    }
                };
                match form {
                    PathForm::Native => format!("{drive}:{}", joined("\\")),
                    PathForm::ForwardSlash => format!("{drive}:{}", joined("/")),
                    PathForm::JsonEscaped => json_escape(&self.render(PathForm::Native, case)),
                    PathForm::PosixDrive => {
                        // Shells that write this form use a lowercase drive unless the
                        // text being replaced says otherwise.
                        let drive = if case == DriveCase::AsGiven {
                            drive.to_ascii_lowercase()
                        } else {
                            drive
                        };
                        format!("/{drive}{}", joined("/"))
                    }
                    PathForm::FileUri => format!("file:///{drive}:{}", joined("/")),
                    PathForm::FileUriPercentEncoded => {
                        // VS Code always lowercases the drive and encodes the colon.
                        let drive = if case == DriveCase::Upper {
                            drive
                        } else {
                            drive.to_ascii_lowercase()
                        };
                        let encoded: String = self
                            .segments
                            .iter()
                            .map(|s| format!("/{}", percent_encode(s)))
                            .collect();
                        format!(
                            "file:///{drive}%3A{}",
                            if encoded.is_empty() { "/" } else { &encoded }
                        )
                    }
                }
            }
            PathStyle::Posix => {
                let native: String = if self.segments.is_empty() {
                    "/".to_owned()
                } else {
                    self.segments.iter().map(|s| format!("/{s}")).collect()
                };
                match form {
                    PathForm::Native | PathForm::ForwardSlash | PathForm::PosixDrive => native,
                    PathForm::JsonEscaped => json_escape(&native),
                    PathForm::FileUri => format!("file://{native}"),
                    PathForm::FileUriPercentEncoded => {
                        let encoded: String = self
                            .segments
                            .iter()
                            .map(|s| format!("/{}", percent_encode(s)))
                            .collect();
                        format!("file://{}", if encoded.is_empty() { "/" } else { &encoded })
                    }
                }
            }
        }
    }
}

/// `text` as it appears inside a JSON string: `\` and `"` escaped, and control
/// characters written as escapes.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// One path segment encoded as VS Code's URI code encodes it: unreserved ASCII kept,
/// every other byte of its UTF-8 written as `%XX` with uppercase hex.
fn percent_encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Whether `byte` can continue a path segment, so a match next to it isn't a whole path.
/// Non-ASCII bytes count, since they're part of a UTF-8 character in a name.
fn continues_path(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_-.~%$+/\\@".contains(&byte) || byte >= 0x80
}

/// One occurrence of a mapping's source path in some text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathMatch {
    /// Where the source path's text sits. Anything after it, such as the rest of a
    /// longer path inside the source, is left as it is.
    pub range: Range<usize>,
    /// The form the path was recorded in.
    pub form: PathForm,
    /// For Windows paths, whether the matched drive letter was lowercase.
    drive_lowercase: Option<bool>,
}

/// The part of a path below a mapping's source, as returned by [`PathMapping::covers`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelativePath {
    segments: Vec<String>,
}

impl RelativePath {
    /// The segments below the source, outermost first. Empty for the source itself.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Whether this is the source itself rather than something under it.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

/// A move's source and target, with everything needed to find the source in recorded
/// text and replace it with the target.
#[derive(Clone, Debug)]
pub struct PathMapping {
    case: CaseRule,
    source: ParsedPath,
    target: ParsedPath,
    /// The source rendered in each form, deduplicated, paired with its form. A pattern's
    /// index is its index in `automaton`.
    patterns: Vec<(PathForm, String)>,
    automaton: AhoCorasick,
}

impl PathMapping {
    /// A mapping from `source` to `target`, both absolute paths of `style`.
    pub fn new(
        source: &str,
        target: &str,
        style: PathStyle,
        case: CaseRule,
    ) -> Result<Self, PathError> {
        let source = ParsedPath::parse(source, style)?;
        let target = ParsedPath::parse(target, style)?;
        let mut patterns: Vec<(PathForm, String)> = Vec::new();
        for &form in PathForm::for_style(style) {
            let text = source.render(form, DriveCase::AsGiven);
            let duplicate = patterns.iter().any(|(_, existing)| match case {
                CaseRule::Sensitive => *existing == text,
                CaseRule::Insensitive => existing.eq_ignore_ascii_case(&text),
            });
            if !duplicate {
                patterns.push((form, text));
            }
        }
        let automaton = AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .ascii_case_insensitive(case == CaseRule::Insensitive)
            .build(patterns.iter().map(|(_, text)| text))
            .expect("a handful of short literal patterns always builds");
        Ok(Self {
            case,
            source,
            target,
            patterns,
            automaton,
        })
    }

    /// A mapping between two host paths, in the host's style.
    pub fn for_host(source: &Path, target: &Path, case: CaseRule) -> Result<Self, PathError> {
        /// The path as text, or an error if it isn't valid Unicode.
        fn as_str(path: &Path) -> Result<&str, PathError> {
            path.to_str()
                .ok_or_else(|| PathError::Unsupported(path.to_string_lossy().into_owned()))
        }
        Self::new(as_str(source)?, as_str(target)?, PathStyle::host(), case)
    }

    /// The source in its native form.
    pub fn source(&self) -> String {
        self.source.render(PathForm::Native, DriveCase::AsGiven)
    }

    /// The target in its native form.
    pub fn target(&self) -> String {
        self.target.render(PathForm::Native, DriveCase::AsGiven)
    }

    /// Every occurrence of the source path in `text` that is a whole path, in reading
    /// order. Occurrences that start or end inside a longer name are skipped (5.0.3).
    pub fn find(&self, text: &[u8]) -> Vec<PathMatch> {
        self.automaton
            .find_iter(text)
            .filter_map(|found| {
                let form = self.patterns[found.pattern().as_usize()].0;
                let starts_clean = found.start() == 0 || !continues_path(text[found.start() - 1]);
                let ends_clean = self.ends_path(form, text, found.end());
                (starts_clean && ends_clean).then(|| PathMatch {
                    range: found.start()..found.end(),
                    form,
                    drive_lowercase: self
                        .source
                        .drive
                        .map(|_| text[found.start() + form.drive_offset()].is_ascii_lowercase()),
                })
            })
            .collect()
    }

    /// Whether the byte at `at` ends a path written in `form`: the end of the text, a
    /// separator of that form, or a character that can't be part of a path.
    fn ends_path(&self, form: PathForm, text: &[u8], at: usize) -> bool {
        let Some(&next) = text.get(at) else {
            return true;
        };
        let windows = self.source.style == PathStyle::Windows;
        match next {
            b'/' => true,
            // In a JSON string a backslash starts an escape: `\\` is a separator only in a
            // Windows path, and `\"` or a whitespace escape ends the path.
            b'\\' if form == PathForm::JsonEscaped => match text.get(at + 1) {
                Some(b'\\') => windows,
                Some(b'"' | b'n' | b'r' | b't') => true,
                _ => false,
            },
            b'\\' => windows,
            b'?' | b'#' => form.is_uri(),
            b'"' | b'\'' | b'`' | b'<' | b'>' | b'|' | b':' | b';' | b',' | b')' | b']' | b'}' => {
                true
            }
            byte => byte.is_ascii_whitespace(),
        }
    }

    /// The text that replaces `found`: the target in the same form, with the drive
    /// letter in the same case as the matched text (5.0.4).
    pub fn rewrite(&self, found: &PathMatch) -> Vec<u8> {
        let case = match found.drive_lowercase {
            Some(true) => DriveCase::Lower,
            Some(false) => DriveCase::Upper,
            None => DriveCase::AsGiven,
        };
        self.target.render(found.form, case).into_bytes()
    }

    /// `text` with every whole-path occurrence of the source replaced by the target.
    pub fn rewrite_all(&self, text: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        let mut copied = 0;
        for found in self.find(text) {
            out.extend_from_slice(&text[copied..found.range.start]);
            out.extend_from_slice(&self.rewrite(&found));
            copied = found.range.end;
        }
        out.extend_from_slice(&text[copied..]);
        out
    }

    /// If `path` is the source or under it, the part below the source.
    pub fn covers(&self, path: &str) -> Option<RelativePath> {
        let path = ParsedPath::parse(path, self.source.style).ok()?;
        // Drive letters name the same drive in either case, whatever the file system.
        let same_drive = match (path.drive, self.source.drive) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(&b),
            (None, None) => true,
            _ => false,
        };
        if !same_drive || path.segments.len() < self.source.segments.len() {
            return None;
        }
        let (head, rest) = path.segments.split_at(self.source.segments.len());
        head.iter()
            .zip(&self.source.segments)
            .all(|(a, b)| self.case.same(a, b))
            .then(|| RelativePath {
                segments: rest.to_vec(),
            })
    }

    /// If `path` is the source or under it, where it ends up after the move, in native form.
    pub fn map(&self, path: &str) -> Option<String> {
        let below = self.covers(path)?;
        let mut moved = self.target.clone();
        moved.segments.extend(below.segments);
        Some(moved.render(PathForm::Native, DriveCase::AsGiven))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windows mapping most tests use.
    fn windows() -> PathMapping {
        PathMapping::new(
            r"C:\Users\u\repo",
            r"D:\work\new",
            PathStyle::Windows,
            CaseRule::Insensitive,
        )
        .unwrap()
    }

    /// The forms found in `text` by `mapping`, in order.
    fn forms(mapping: &PathMapping, text: &str) -> Vec<PathForm> {
        mapping
            .find(text.as_bytes())
            .into_iter()
            .map(|m| m.form)
            .collect()
    }

    /// `text` rewritten by `mapping`, as a string.
    fn rewritten(mapping: &PathMapping, text: &str) -> String {
        String::from_utf8(mapping.rewrite_all(text.as_bytes())).unwrap()
    }

    // 5.1: every recorded form is recognised as the source.
    #[test]
    fn recognises_every_windows_form() {
        let m = windows();
        assert_eq!(
            forms(&m, r#""cwd":"C:\\Users\\u\\repo""#),
            [PathForm::JsonEscaped]
        );
        assert_eq!(
            forms(&m, "key c:/Users/u/repo here"),
            [PathForm::ForwardSlash]
        );
        assert_eq!(
            forms(&m, r#""cwd":"/c/Users/u/repo""#),
            [PathForm::PosixDrive]
        );
        assert_eq!(forms(&m, "file:///c:/Users/u/repo"), [PathForm::FileUri]);
        assert_eq!(
            forms(&m, "file:///c%3A/Users/u/repo"),
            [PathForm::FileUriPercentEncoded]
        );
        assert_eq!(forms(&m, r"cd C:\Users\u\repo"), [PathForm::Native]);
    }

    #[test]
    fn recognises_posix_forms() {
        let m = PathMapping::new(
            "/home/u/my repo",
            "/srv/new",
            PathStyle::Posix,
            CaseRule::Sensitive,
        )
        .unwrap();
        assert_eq!(
            forms(&m, r#""cwd":"/home/u/my repo/src""#),
            [PathForm::Native]
        );
        assert_eq!(
            forms(&m, "file:///home/u/my%20repo/a"),
            [PathForm::FileUriPercentEncoded]
        );
        assert_eq!(forms(&m, "file:///home/u/my repo"), [PathForm::FileUri]);
        assert!(
            forms(&m, "/home/u/My repo").is_empty(),
            "case matters on a case-sensitive volume"
        );
    }

    // 5.2: siblings and longer names never match.
    #[test]
    fn never_matches_inside_a_longer_name() {
        let m = windows();
        for text in [
            r"C:\Users\u\repo-old",
            r"C:\Users\u\repo.bak",
            r"C:\Users\u\repository",
            r"X:\backup\C:\Users\u\repo",
            "/mnt/c/Users/u/repo",
            r#""C:\\Users\\u\\repo\u00e9""#,
        ] {
            assert!(forms(&m, text).is_empty(), "no match in {text}");
        }
    }

    #[test]
    fn matches_at_every_kind_of_boundary() {
        let m = windows();
        for text in [
            r"C:\Users\u\repo",
            r"C:\Users\u\repo\src\main.rs",
            r"C:\Users\u\repo/mixed",
            r"(C:\Users\u\repo)",
            r#"'C:\Users\u\repo', "x""#,
            "file:///c%3A/Users/u/repo?query",
            "file:///c%3A/Users/u/repo#anchor",
            r#""C:\\Users\\u\\repo\" said""#,
        ] {
            assert_eq!(forms(&m, text).len(), 1, "one match in {text}");
        }
    }

    // 5.3: a replacement keeps the form and drive-letter case of what it replaces.
    #[test]
    fn rewrites_in_the_same_form_and_drive_case() {
        let m = windows();
        assert_eq!(rewritten(&m, "/c/Users/u/repo/sub"), "/d/work/new/sub");
        assert_eq!(
            rewritten(&m, r#""C:\\Users\\u\\repo\\sub""#),
            r#""D:\\work\\new\\sub""#
        );
        assert_eq!(rewritten(&m, "c:/Users/u/repo"), "d:/work/new");
        assert_eq!(
            rewritten(&m, "file:///c%3A/Users/u/repo/a"),
            "file:///d%3A/work/new/a"
        );
        assert_eq!(
            rewritten(&m, r"C:\Users\u\repo-old and C:\Users\u\repo"),
            r"C:\Users\u\repo-old and D:\work\new"
        );
    }

    #[test]
    fn percent_encodes_like_vs_code() {
        let m = PathMapping::new(
            r"C:\a b\é",
            r"C:\x",
            PathStyle::Windows,
            CaseRule::Insensitive,
        )
        .unwrap();
        assert_eq!(
            forms(&m, "file:///c%3A/a%20b/%C3%A9"),
            [PathForm::FileUriPercentEncoded]
        );
    }

    #[test]
    fn covers_the_source_and_what_is_under_it() {
        let m = windows();
        assert!(m.covers(r"C:\Users\u\repo").unwrap().is_empty());
        assert_eq!(
            m.covers(r"c:/users/U/Repo/src").unwrap().segments(),
            ["src"]
        );
        assert!(m.covers(r"C:\Users\u\repo-old").is_none());
        assert!(m.covers(r"C:\Users\u").is_none());
        assert!(m.covers(r"D:\Users\u\repo").is_none());
        assert_eq!(
            m.map(r"C:\Users\u\repo\.claude\worktrees\wt1").unwrap(),
            r"D:\work\new\.claude\worktrees\wt1"
        );
        assert_eq!(m.source(), r"C:\Users\u\repo");
        assert_eq!(m.target(), r"D:\work\new");
    }

    #[test]
    fn refuses_paths_it_cannot_handle() {
        let new = |path: &str, style| {
            PathMapping::new(path, "/x", style, CaseRule::Sensitive).map(|_| ())
        };
        assert!(matches!(
            new("relative/path", PathStyle::Posix),
            Err(PathError::NotAbsolute(_))
        ));
        assert!(matches!(
            new("/a/../b", PathStyle::Posix),
            Err(PathError::Unsupported(_))
        ));
        let windows = |path: &str| {
            PathMapping::new(path, r"C:\x", PathStyle::Windows, CaseRule::Insensitive).map(|_| ())
        };
        assert!(matches!(
            windows(r"\\server\share\repo"),
            Err(PathError::Unsupported(_))
        ));
        assert!(matches!(
            windows(r"C:relative"),
            Err(PathError::NotAbsolute(_))
        ));
        assert!(matches!(windows("repo"), Err(PathError::NotAbsolute(_))));
        assert!(
            PathError::NotAbsolute("x".into())
                .to_string()
                .contains("absolute")
        );
        assert!(
            PathError::Unsupported("x".into())
                .to_string()
                .contains("can't move")
        );
    }

    #[test]
    fn for_host_uses_the_host_style() {
        let (source, target) = if cfg!(windows) {
            (r"C:\a\repo", r"C:\b\repo")
        } else {
            ("/a/repo", "/b/repo")
        };
        let m = PathMapping::for_host(Path::new(source), Path::new(target), CaseRule::Sensitive)
            .unwrap();
        assert_eq!(m.source(), source);
        assert_eq!(
            m.map(&format!("{source}{}x", std::path::MAIN_SEPARATOR))
                .unwrap(),
            format!("{target}{}x", std::path::MAIN_SEPARATOR)
        );
    }

    mod properties {
        use super::super::*;
        use proptest::prelude::*;

        /// A path segment: letters, digits, spaces and a little punctuation, never `.` or `..`.
        fn segment() -> impl Strategy<Value = String> {
            "[A-Za-z0-9 _.é-]{1,8}".prop_filter("not . or ..", |s| s != "." && s != "..")
        }

        /// A Windows path's parts: a drive letter and one or more segments.
        fn windows_parts() -> impl Strategy<Value = (char, Vec<String>)> {
            (
                prop::sample::select(vec!['C', 'D', 'e']),
                prop::collection::vec(segment(), 1..4),
            )
        }

        /// The Windows path made from `drive` and `segments`, in native form.
        fn native(drive: char, segments: &[String]) -> String {
            format!("{drive}:\\{}", segments.join("\\"))
        }

        proptest! {
            // Rewriting a match and mapping it back gives the original text, in every form.
            #[test]
            fn rewriting_round_trips(
                (sd, source) in windows_parts(),
                (td, target) in windows_parts(),
                below in prop::collection::vec(segment(), 0..3),
            ) {
                let forward = PathMapping::new(&native(sd, &source), &native(td, &target), PathStyle::Windows, CaseRule::Insensitive).unwrap();
                let back = PathMapping::new(&native(td, &target), &native(sd, &source), PathStyle::Windows, CaseRule::Insensitive).unwrap();
                let full = ParsedPath { style: PathStyle::Windows, drive: Some(sd), segments: [source.clone(), below].concat() };
                for &form in PathForm::for_style(PathStyle::Windows) {
                    let text = format!("\"{}\"", full.render(form, DriveCase::AsGiven));
                    let moved = forward.rewrite_all(text.as_bytes());
                    prop_assert_ne!(&moved, text.as_bytes(), "{:?} was found in {}", form, text);
                    prop_assert_eq!(back.rewrite_all(&moved), text.as_bytes().to_vec(), "{:?} round-trips", form);
                }
            }

            // No match ever ends inside a segment.
            #[test]
            fn no_match_ends_inside_a_segment(
                (sd, source) in windows_parts(),
                tail in "[A-Za-z0-9_.~-]",
            ) {
                let mapping = PathMapping::new(&native(sd, &source), r"Z:\t", PathStyle::Windows, CaseRule::Insensitive).unwrap();
                let parsed = ParsedPath { style: PathStyle::Windows, drive: Some(sd), segments: source };
                for &form in PathForm::for_style(PathStyle::Windows) {
                    let text = format!("{}{tail}", parsed.render(form, DriveCase::AsGiven));
                    prop_assert!(mapping.find(text.as_bytes()).is_empty(), "no match in {}", text);
                }
            }
        }
    }
}
