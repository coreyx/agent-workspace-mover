//! A streaming scanner for JSON Lines files, such as Claude Code's transcripts and
//! prompt history (design.md, "Transcripts and prompt history" in tech.md, and "Key
//! design decisions", 7 and 8).
//!
//! It reads a file one line at a time and, for each line, reports the byte span and
//! decoded value of named fields **at the top level of the line's object only**. The
//! same name inside a nested object, or inside a string that happens to hold JSON, is
//! ignored. That's what lets an opt-in rewrite (requirements.md, REQ-8) change exactly
//! one field's value: [`splice_string`] replaces the reported span and copies every
//! other byte unchanged, so conversation text is never re-encoded or touched.
//!
//! Every line is fully validated, with the same rules as `serde_json`, including its
//! nesting limit and its refusal of lone surrogate escapes. A line that `serde_json`
//! would reject is reported as [`LineKind::Malformed`], with its line number, and a
//! final line with no newline after it is marked unterminated, since that's what a
//! session killed mid-write leaves (design.md, "Key design decisions", 8). Values read
//! are wrapped in [`AgentBytes`], so they can't reach output by accident.

use std::io::{self, BufRead};
use std::ops::Range;

use crate::sensitive::AgentBytes;

/// The deepest nesting of arrays and objects a line may have. `serde_json` rejects
/// anything deeper, so the scanner does too.
pub const MAX_DEPTH: usize = 127;

/// One line of a JSON Lines file, as the scanner found it.
#[derive(Debug)]
pub struct ScannedLine {
    /// The line's number, counting from 1.
    pub number: usize,
    /// Where the line starts in the file, in bytes.
    pub start: u64,
    /// The line's length in bytes, without its newline.
    pub len: usize,
    /// Whether a newline ends the line. Only a file's last line can lack one.
    pub terminated: bool,
    /// What the line holds.
    pub kind: LineKind,
}

/// What a line holds.
#[derive(Debug)]
pub enum LineKind {
    /// Nothing but whitespace.
    Blank,
    /// A JSON object. Holds one entry per field asked for, in the order asked, each
    /// `None` when the object has no such top-level field.
    Object(Vec<Option<Field>>),
    /// Valid JSON, but not an object.
    NotAnObject,
    /// Not valid JSON.
    Malformed,
}

/// A top-level field found in a line's object.
#[derive(Debug)]
pub struct Field {
    /// The bytes of the field's value within the line, including a string's quotes.
    pub span: Range<usize>,
    /// The value, decoded when it's a string.
    pub value: FieldValue,
    /// How many times the field appears at the top level. More than one means the
    /// object has duplicate keys; `span` and `value` are the last, as `serde_json` keeps.
    pub occurrences: usize,
}

/// A field's value.
#[derive(Debug)]
pub enum FieldValue {
    /// A string, decoded from its JSON escapes.
    String(AgentBytes),
    /// Any other JSON value. Its content isn't decoded.
    Other,
}

/// Reads `reader` line by line, calling `visit` with each line's scan and its bytes
/// (without the newline). Returns how many lines there were.
pub fn scan<R: BufRead>(
    mut reader: R,
    fields: &[&str],
    mut visit: impl FnMut(&ScannedLine, &[u8]),
) -> io::Result<usize> {
    let mut buffer = Vec::new();
    let mut offset = 0u64;
    let mut number = 0;
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            return Ok(number);
        }
        number += 1;
        let terminated = buffer.last() == Some(&b'\n');
        let content = if terminated {
            &buffer[..read - 1]
        } else {
            &buffer[..]
        };
        let line = ScannedLine {
            number,
            start: offset,
            len: content.len(),
            terminated,
            kind: scan_line(content, fields),
        };
        visit(&line, content);
        offset += read as u64;
    }
}

/// Scans one line (without its newline) for the top-level `fields`.
pub fn scan_line(line: &[u8], fields: &[&str]) -> LineKind {
    if line.iter().all(|byte| is_whitespace(*byte)) {
        return LineKind::Blank;
    }
    if std::str::from_utf8(line).is_err() {
        return LineKind::Malformed;
    }
    let mut parser = Parser {
        bytes: line,
        pos: 0,
        depth: 0,
    };
    parser.skip_whitespace();
    let found = if parser.peek() == Some(b'{') {
        parser.top_object(fields).map(Some)
    } else {
        parser.value().map(|()| None)
    };
    parser.skip_whitespace();
    match found {
        Ok(_) if parser.pos != line.len() => LineKind::Malformed,
        Ok(Some(fields)) => LineKind::Object(fields),
        Ok(None) => LineKind::NotAnObject,
        Err(Invalid) => LineKind::Malformed,
    }
}

/// `line` with the bytes in `span` replaced by `value` written as a JSON string. Every
/// other byte is copied unchanged.
pub fn splice_string(line: &[u8], span: Range<usize>, value: &str) -> Vec<u8> {
    let encoded = encode_string(value);
    let mut out = Vec::with_capacity(line.len() - span.len() + encoded.len());
    out.extend_from_slice(&line[..span.start]);
    out.extend_from_slice(encoded.as_bytes());
    out.extend_from_slice(&line[span.end..]);
    out
}

/// `value` as a JSON string, quotes included, escaped the way `serde_json` escapes it.
pub fn encode_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// JSON's whitespace characters.
fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// The line isn't valid JSON. Where and why don't matter to callers, which report the
/// line number only, so as not to echo what the line holds.
#[derive(Debug)]
struct Invalid;

/// A recursive-descent JSON validator over one line's bytes, which the caller has
/// already checked are UTF-8.
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    /// The next byte, without consuming it.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Consumes `byte`, or fails if the next byte is something else.
    fn expect(&mut self, byte: u8) -> Result<(), Invalid> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Invalid)
        }
    }

    /// Skips JSON whitespace.
    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(is_whitespace) {
            self.pos += 1;
        }
    }

    /// Enters an array or object, failing past `MAX_DEPTH`.
    fn enter(&mut self) -> Result<(), Invalid> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(Invalid)
        } else {
            Ok(())
        }
    }

    /// Validates any JSON value.
    fn value(&mut self) -> Result<(), Invalid> {
        match self.peek().ok_or(Invalid)? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string(false).map(|_| ()),
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(Invalid),
        }
    }

    /// Validates a literal such as `true`.
    fn literal(&mut self, word: &[u8]) -> Result<(), Invalid> {
        if self.bytes[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(())
        } else {
            Err(Invalid)
        }
    }

    /// Validates a number: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
    fn number(&mut self) -> Result<(), Invalid> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(Invalid),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.required_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.required_digits()?;
        }
        Ok(())
    }

    /// Skips any digits.
    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    /// Skips one or more digits, failing if there are none.
    fn required_digits(&mut self) -> Result<(), Invalid> {
        let start = self.pos;
        self.digits();
        if self.pos == start {
            Err(Invalid)
        } else {
            Ok(())
        }
    }

    /// Validates an array.
    fn array(&mut self) -> Result<(), Invalid> {
        self.enter()?;
        self.expect(b'[')?;
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
        } else {
            loop {
                self.skip_whitespace();
                self.value()?;
                self.skip_whitespace();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b']') => {
                        self.pos += 1;
                        break;
                    }
                    _ => return Err(Invalid),
                }
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// Validates a nested object, whose fields don't matter.
    fn object(&mut self) -> Result<(), Invalid> {
        self.members(|parser, _, _| parser.value())
    }

    /// Validates the line's top-level object and records the values of `fields`.
    fn top_object(&mut self, fields: &[&str]) -> Result<Vec<Option<Field>>, Invalid> {
        let mut found: Vec<Option<Field>> = fields.iter().map(|_| None).collect();
        self.members(|parser, key, value_start| {
            let wanted = fields.iter().position(|field| field.as_bytes() == key);
            let value = if parser.peek() == Some(b'"') && wanted.is_some() {
                FieldValue::String(AgentBytes::new(parser.string(true)?.unwrap_or_default()))
            } else {
                parser.value()?;
                FieldValue::Other
            };
            if let Some(index) = wanted {
                let occurrences = found[index].as_ref().map_or(0, |f| f.occurrences) + 1;
                found[index] = Some(Field {
                    span: value_start..parser.pos,
                    value,
                    occurrences,
                });
            }
            Ok(())
        })?;
        Ok(found)
    }

    /// Validates an object, calling `member` for each field with the decoded key and the
    /// value's start. `member` must consume the value.
    fn members(
        &mut self,
        mut member: impl FnMut(&mut Self, &[u8], usize) -> Result<(), Invalid>,
    ) -> Result<(), Invalid> {
        self.enter()?;
        self.expect(b'{')?;
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
        } else {
            loop {
                self.skip_whitespace();
                let key = self.string(true)?.unwrap_or_default();
                self.skip_whitespace();
                self.expect(b':')?;
                self.skip_whitespace();
                let start = self.pos;
                member(self, &key, start)?;
                self.skip_whitespace();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => {
                        self.pos += 1;
                        break;
                    }
                    _ => return Err(Invalid),
                }
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// Validates a string and, when `decode` is set, returns its decoded bytes.
    fn string(&mut self, decode: bool) -> Result<Option<Vec<u8>>, Invalid> {
        self.expect(b'"')?;
        let mut out = decode.then(Vec::new);
        loop {
            let byte = self.peek().ok_or(Invalid)?;
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self.peek().ok_or(Invalid)?;
                    self.pos += 1;
                    let decoded = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(Invalid),
                    };
                    if let Some(out) = out.as_mut() {
                        let mut utf8 = [0u8; 4];
                        out.extend_from_slice(decoded.encode_utf8(&mut utf8).as_bytes());
                    }
                }
                0x00..=0x1f => return Err(Invalid),
                byte => {
                    if let Some(out) = out.as_mut() {
                        out.push(byte);
                    }
                }
            }
        }
    }

    /// Decodes the rest of a `\u` escape, including the second half of a surrogate
    /// pair. A lone surrogate is invalid, as `serde_json` treats it.
    fn unicode_escape(&mut self) -> Result<char, Invalid> {
        let first = self.hex4()?;
        match first {
            0xD800..=0xDBFF => {
                self.expect(b'\\')?;
                self.expect(b'u')?;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(Invalid);
                }
                let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                char::from_u32(code).ok_or(Invalid)
            }
            0xDC00..=0xDFFF => Err(Invalid),
            code => char::from_u32(code).ok_or(Invalid),
        }
    }

    /// Reads four hex digits.
    fn hex4(&mut self) -> Result<u32, Invalid> {
        let digits = self.bytes.get(self.pos..self.pos + 4).ok_or(Invalid)?;
        let text = std::str::from_utf8(digits).map_err(|_| Invalid)?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Invalid);
        }
        self.pos += 4;
        u32::from_str_radix(text, 16).map_err(|_| Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The string value of `field` in `line`, if it's a top-level string.
    fn string_field(line: &str, field: &str) -> Option<String> {
        match scan_line(line.as_bytes(), &[field]) {
            LineKind::Object(mut found) => match found.remove(0)?.value {
                FieldValue::String(bytes) => Some(bytes.as_str().unwrap().to_owned()),
                FieldValue::Other => None,
            },
            _ => None,
        }
    }

    /// The scanned field `field` of `line`, which must be an object.
    fn field(line: &str, field: &str) -> Option<Field> {
        match scan_line(line.as_bytes(), &[field]) {
            LineKind::Object(mut found) => found.remove(0),
            other => panic!("not an object: {other:?}"),
        }
    }

    #[test]
    fn finds_only_top_level_fields() {
        let line = r#"{"message":{"cwd":"nested"},"text":"{\"cwd\":\"in a string\"}","cwd":"C:\\Users\\u\\repo"}"#;
        assert_eq!(
            string_field(line, "cwd").as_deref(),
            Some(r"C:\Users\u\repo")
        );
        assert_eq!(string_field(r#"{"message":{"cwd":"nested"}}"#, "cwd"), None);
    }

    #[test]
    fn reports_spans_that_splice_cleanly() {
        let line = r#"{"a":1, "cwd" : "C:\\old" ,"z":[true]}"#;
        let found = field(line, "cwd").unwrap();
        assert_eq!(&line[found.span.clone()], r#""C:\\old""#);
        let spliced = splice_string(line.as_bytes(), found.span, r"D:\new");
        assert_eq!(
            String::from_utf8(spliced).unwrap(),
            r#"{"a":1, "cwd" : "D:\\new" ,"z":[true]}"#
        );
    }

    #[test]
    fn tells_non_string_values_and_duplicates_apart() {
        assert!(matches!(
            field(r#"{"cwd":42}"#, "cwd").unwrap().value,
            FieldValue::Other
        ));
        let twice = field(r#"{"cwd":"a","cwd":"b"}"#, "cwd").unwrap();
        assert_eq!(twice.occurrences, 2);
        assert!(matches!(&twice.value, FieldValue::String(b) if b.as_bytes() == b"b"));
        assert!(field(r#"{"other":"x"}"#, "cwd").is_none());
    }

    #[test]
    fn decodes_every_escape() {
        let line = r#"{"cwd":"q\" s\\ f\/ \b\f\n\r\t \u00e9 \ud83d\ude00"}"#;
        assert_eq!(
            string_field(line, "cwd").unwrap(),
            "q\" s\\ f/ \u{8}\u{c}\n\r\t é 😀"
        );
    }

    #[test]
    fn classifies_lines() {
        assert!(matches!(scan_line(b"", &[]), LineKind::Blank));
        assert!(matches!(scan_line(b" \t\r", &[]), LineKind::Blank));
        assert!(matches!(scan_line(b"[1, 2]", &[]), LineKind::NotAnObject));
        assert!(matches!(
            scan_line(b" \"text\" ", &[]),
            LineKind::NotAnObject
        ));
        for bad in [
            r#"{"a":}"#,
            r#"{"a":1,}"#,
            r#"{"a" 1}"#,
            r#"{"a":1} x"#,
            "[1,]",
            "[1 2]",
            "{a:1}",
            r#"{"a":tru}"#,
            r#"{"a":01}"#,
            r#"{"a":1.}"#,
            r#"{"a":1e}"#,
            r#"{"a":-}"#,
            r#"{"a":"\x"}"#,
            r#"{"a":"\ud800"}"#,
            r#"{"a":"\ude00"}"#,
            r#"{"a":"\ud800\u0041"}"#,
            r#"{"a":"\u12"}"#,
            r#"{"a":"\u12g4"}"#,
            "{\"a\":\"tab\there\"}",
            r#"{"a":"unterminated"#,
            "{",
            "[",
            "x",
            "{1:2}",
        ] {
            assert!(
                matches!(scan_line(bad.as_bytes(), &["a"]), LineKind::Malformed),
                "malformed: {bad}"
            );
        }
        assert!(matches!(
            scan_line(&[b'{', b'"', 0xff, b'"', b':', b'1', b'}'], &[]),
            LineKind::Malformed
        ));
    }

    #[test]
    fn accepts_every_number_and_literal_form() {
        let line = r#"{"n":[0,-0,12,-3.5,1e9,2E-3,4e+2,true,false,null,{},[]]}"#;
        assert!(matches!(
            scan_line(line.as_bytes(), &[]),
            LineKind::Object(_)
        ));
    }

    #[test]
    fn limits_nesting_like_serde_json() {
        for depth in [MAX_DEPTH, MAX_DEPTH + 1] {
            let line = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
            let ours = !matches!(scan_line(line.as_bytes(), &[]), LineKind::Malformed);
            let theirs = serde_json::from_str::<serde_json::Value>(&line).is_ok();
            assert_eq!(ours, theirs, "depth {depth}");
        }
        let deep_object = format!(
            "{{\"a\":{}1{}}}",
            "[".repeat(MAX_DEPTH - 1),
            "]".repeat(MAX_DEPTH - 1)
        );
        assert!(matches!(
            scan_line(deep_object.as_bytes(), &[]),
            LineKind::Object(_)
        ));
    }

    #[test]
    fn streams_lines_with_their_positions() {
        let text = b"{\"cwd\":\"a\"}\n\n[1]\r\n{\"cwd\":\"b\"}";
        let mut seen = Vec::new();
        let count = scan(&text[..], &["cwd"], |line, bytes| {
            seen.push((
                line.number,
                line.start,
                line.len,
                line.terminated,
                bytes.to_vec(),
                matches!(line.kind, LineKind::Object(_)),
            ));
        })
        .unwrap();
        assert_eq!(count, 4);
        assert_eq!(seen[0], (1, 0, 11, true, b"{\"cwd\":\"a\"}".to_vec(), true));
        assert_eq!(seen[1].2, 0);
        assert_eq!(
            (seen[2].1, seen[2].2),
            (13, 4),
            "a carriage return stays in the line, as JSON whitespace"
        );
        assert_eq!(
            (seen[3].1, seen[3].3, seen[3].5),
            (18, false, true),
            "the last line has no newline"
        );
    }

    #[test]
    fn encodes_like_serde_json() {
        for value in [
            "plain",
            "q\"b\\",
            "\u{0}\u{1f}\u{8}\u{c}\n\r\t",
            "é😀",
            "C:\\Users\\u",
        ] {
            assert_eq!(encode_string(value), serde_json::to_string(value).unwrap());
        }
    }

    #[test]
    fn handles_real_transcript_shapes() {
        let lines = [
            r#"{"parentUuid":null,"isSidechain":false,"type":"user","message":{"role":"user","content":"cd C:\\Users\\u\\repo and set \"cwd\": \"x\""},"cwd":"C:\\Users\\u\\repo","sessionId":"s","version":"2.1.309","gitBranch":"main","entrypoint":"cli"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"ls","cwd":"/elsewhere"}}]},"cwd":"/c/Users/u/repo","entrypoint":"cli"}"#,
            r#"{"type":"user","serverClassifierContext":{"context":{"live_cwd":"/c/Users/u/repo","git_state":{"root":"/c/Users/u/repo"}}},"cwd":"C:\\Users\\u\\repo"}"#,
            r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-22T00:00:00Z","sessionId":"s"}"#,
        ];
        let expected = [
            Some(r"C:\Users\u\repo"),
            Some("/c/Users/u/repo"),
            Some(r"C:\Users\u\repo"),
            None,
        ];
        for (line, cwd) in lines.iter().zip(expected) {
            assert_eq!(string_field(line, "cwd").as_deref(), cwd, "{line}");
        }
        let long = format!(r#"{{"blob":"{}","cwd":"/a"}}"#, "x".repeat(2_000_000));
        assert_eq!(string_field(&long, "cwd").as_deref(), Some("/a"));
    }

    mod properties {
        use super::super::*;
        use proptest::prelude::*;
        use serde_json::Value;

        /// Any JSON value, a few levels deep, with strings that include quotes,
        /// backslashes, control characters and non-ASCII.
        fn json() -> impl Strategy<Value = Value> {
            let leaf = prop_oneof![
                Just(Value::Null),
                any::<bool>().prop_map(Value::Bool),
                any::<i64>().prop_map(Value::from),
                (-1e12f64..1e12).prop_map(Value::from),
                any::<String>().prop_map(Value::String),
            ];
            leaf.prop_recursive(4, 32, 6, |inner| {
                prop_oneof![
                    prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
                    prop::collection::btree_map("[a-z]{0,4}|cwd", inner, 0..6)
                        .prop_map(|map| Value::Object(map.into_iter().collect())),
                ]
            })
        }

        /// A top-level object that may hold `cwd` as a string, another value, or not at all.
        fn line_object() -> impl Strategy<Value = Value> {
            (
                prop::collection::btree_map("[a-z]{1,4}", json(), 0..5),
                prop::option::of(json()),
            )
                .prop_map(|(map, cwd)| {
                    let mut object: serde_json::Map<String, Value> = map.into_iter().collect();
                    if let Some(cwd) = cwd {
                        object.insert("cwd".to_owned(), cwd);
                    }
                    Value::Object(object)
                })
        }

        proptest! {
            // The scanner is critical code (tasks.md), so it gets more cases than the default.
            #![proptest_config(ProptestConfig::with_cases(2048))]

            // Every span the scanner reports parses to the value serde_json sees, compact
            // or spaced out, and splicing a new value changes nothing else.
            #[test]
            fn agrees_with_serde_json(object in line_object(), spaced in any::<bool>(), new in any::<String>()) {
                let line = if spaced {
                    serde_json::to_string_pretty(&object).unwrap().replace('\n', " ")
                } else {
                    serde_json::to_string(&object).unwrap()
                };
                let LineKind::Object(mut found) = scan_line(line.as_bytes(), &["cwd"]) else {
                    return Err(TestCaseError::fail("an object wasn't recognised"));
                };
                match (object.get("cwd"), found.remove(0)) {
                    (None, None) => {}
                    (Some(Value::String(expected)), Some(field)) => {
                        let FieldValue::String(bytes) = &field.value else {
                            return Err(TestCaseError::fail("a string field wasn't decoded"));
                        };
                        prop_assert_eq!(bytes.as_str(), Some(expected.as_str()));
                        let at_span: String = serde_json::from_slice(&line.as_bytes()[field.span.clone()]).unwrap();
                        prop_assert_eq!(&at_span, expected);
                        let spliced = splice_string(line.as_bytes(), field.span.clone(), &new);
                        let mut reparsed: Value = serde_json::from_slice(&spliced).unwrap();
                        prop_assert_eq!(reparsed["cwd"].take(), Value::String(new.clone()));
                        let mut original = object.clone();
                        original["cwd"] = Value::Null;
                        reparsed["cwd"] = Value::Null;
                        prop_assert_eq!(reparsed, original, "only cwd changed");
                    }
                    (Some(value), Some(field)) => {
                        prop_assert!(matches!(field.value, FieldValue::Other));
                        let at_span: Value = serde_json::from_slice(&line.as_bytes()[field.span]).unwrap();
                        prop_assert_eq!(&at_span, value);
                    }
                    (expected, got) => return Err(TestCaseError::fail(format!("expected {expected:?}, got {got:?}"))),
                }
            }

            // A damaged line is malformed exactly when serde_json rejects it.
            #[test]
            fn rejects_what_serde_json_rejects(
                object in line_object(),
                at in any::<prop::sample::Index>(),
                edit in 0u8..3,
                byte in prop::sample::select(b"{}[]\",:\\ae01.-u \x01".to_vec()),
            ) {
                let mut line = serde_json::to_vec(&object).unwrap();
                let i = at.index(line.len() + 1);
                match edit {
                    0 if i < line.len() => { line.remove(i); }
                    1 if i < line.len() => line[i] = byte,
                    _ => line.insert(i, byte),
                }
                let ours = scan_line(&line, &["cwd"]);
                let theirs = serde_json::from_slice::<Value>(&line);
                match ours {
                    LineKind::Blank => {}
                    LineKind::Malformed => prop_assert!(theirs.is_err(), "we rejected what serde_json accepts: {:?}", String::from_utf8_lossy(&line)),
                    LineKind::Object(_) | LineKind::NotAnObject => prop_assert!(theirs.is_ok(), "we accepted what serde_json rejects: {:?}", String::from_utf8_lossy(&line)),
                }
            }
        }
    }
}
