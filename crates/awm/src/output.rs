//! Writing results: one JSON document on standard output with `--json`, and every
//! human-readable message on standard error (requirements.md, 22.0.1 and 22.3).

use std::io::{self, Write};

use serde::Serialize;

/// Writes `value` to `out` as one JSON document, followed by a newline. Nothing else
/// may be written to standard output in `--json` mode.
pub fn write_json<T: Serialize>(value: &T, out: &mut impl Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, value).map_err(io::Error::other)?;
    out.write_all(b"\n")?;
    out.flush()
}

/// Writes a human-readable message for the user, which always goes to standard error
/// so it can't mix with JSON on standard output.
pub fn note(err: &mut impl Write, message: &str) -> io::Result<()> {
    writeln!(err, "{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 22.3: standard output holds exactly one JSON document; messages go elsewhere.
    #[test]
    fn json_output_is_one_document_and_messages_go_to_stderr() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        note(&mut stderr, "Planning the move…").unwrap();
        write_json(
            &serde_json::json!({"schema": "awm.plan/1", "adapters": []}),
            &mut stdout,
        )
        .unwrap();
        note(&mut stderr, "Done.").unwrap();
        let documents: Vec<serde_json::Value> = serde_json::Deserializer::from_slice(&stdout)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0]["schema"], "awm.plan/1");
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "Planning the move…\nDone.\n"
        );
    }
}
