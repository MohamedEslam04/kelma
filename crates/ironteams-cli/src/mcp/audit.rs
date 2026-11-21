//! A local record of every write the MCP server was asked to do: one JSON line per send attempt
//! in `<data dir>/mcp-audit.jsonl`. It names the time, account, kind of write, the chat or
//! meeting id and the outcome; never the text.

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE: &str = "mcp-audit.jsonl";

#[derive(Serialize)]
struct Line<'a> {
    time: String,
    account: &'a str,
    write: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<&'a str>,
    draft: &'a str,
    outcome: &'a str,
}

pub struct Audit {
    path: PathBuf,
    account: String,
}

impl Audit {
    pub fn new(data_dir: &Path, account: &str) -> Audit {
        Audit {
            path: data_dir.join(FILE),
            account: account.to_string(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One line, without the newline.
    pub fn line(
        &self,
        time_ms: i64,
        write: &str,
        target: Option<&str>,
        draft: &str,
        outcome: &str,
    ) -> String {
        serde_json::to_string(&Line {
            time: calendar::format_iso_ms(time_ms),
            account: &self.account,
            write,
            target,
            draft,
            outcome,
        })
        .unwrap_or_default()
    }

    /// Appends a line. A log that cannot be written is reported on stderr (the MCP client shows
    /// it in its server log); it does not undo or block the write.
    pub fn record(
        &self,
        time_ms: i64,
        write: &str,
        target: Option<&str>,
        draft: &str,
        outcome: &str,
    ) {
        let line = self.line(time_ms, write, target, draft, outcome);
        let r = std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
            })
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = r {
            eprintln!(
                "ironteams mcp: could not write {}: {e}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_names_the_write_but_holds_no_text() {
        let a = Audit::new(Path::new("/nowhere"), "personal");
        let line = a.line(0, "message", Some("48:notes"), "draft-1-1", "sent");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["time"], "1970-01-01T00:00:00.000Z");
        assert_eq!(v["account"], "personal");
        assert_eq!(v["write"], "message");
        assert_eq!(v["target"], "48:notes");
        assert_eq!(v["draft"], "draft-1-1");
        assert_eq!(v["outcome"], "sent");
        assert_eq!(
            v.as_object().unwrap().len(),
            6,
            "no other field, no content"
        );
        let no_target = a.line(0, "presence", None, "d", "declined");
        assert!(!no_target.contains("target"));
    }

    #[test]
    fn records_append() {
        let dir = std::env::temp_dir().join(format!(
            "ironteams-mcp-audit-{}-{}",
            std::process::id(),
            teams_proto::time::now_ms()
        ));
        let a = Audit::new(&dir, "personal");
        a.record(0, "message", Some("48:notes"), "d1", "sent");
        a.record(1, "reaction", Some("48:notes"), "d2", "failed");
        let text = std::fs::read_to_string(a.path()).unwrap();
        assert_eq!(text.lines().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
