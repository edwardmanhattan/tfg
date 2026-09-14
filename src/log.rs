//! Per-minute action journal (Log grill, #20): append-only JSONL.
//!
//! One line per entry, flushed per append; the file IS the export
//! (close = flush, no transform). Entries cite fixes by ingest sequence
//! in their payload — never duplicating positions — via seqs the UI
//! reports back after ingest (`SimCommand::FixAck`): arrival/blocked
//! entries cite the ship's latest acked seq, minute markers carry a full
//! ship-to-seq snapshot.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use serde::Serialize;

/// Entry kinds. Join/Telegram have no producers yet (session grill, #23):
/// the variants reserve their place in the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LogKind {
    Command,
    CommandRefused,
    CommandOverridden,
    OrderRefused,
    Arrival,
    ShipBlocked,
    /// Game-minute boundary crossed, derived from the ADR-0004 clock.
    Marker,
    Join,
    Telegram,
}

/// `{seq, game_ts, actor, kind, payload}` with seat actors (precedence
/// grill, #19). Until seats land, command actors read `authority:<rank>`
/// and sim-observed entries read `sim`. Fix citations ride in `payload`:
/// single `fix_seq` on ship events, `fix_seqs` snapshot on markers.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub seq: u64,
    pub game_ts: Option<String>,
    pub actor: String,
    pub kind: LogKind,
    pub payload: serde_json::Value,
}

/// Single-writer journal owned by the sim. `disabled()` is a no-op sink
/// for tests and log-less runs.
pub struct Journal {
    next_seq: u64,
    writer: Option<BufWriter<File>>,
}

impl Journal {
    pub fn disabled() -> Self {
        Self { next_seq: 0, writer: None }
    }

    /// Fixed prototype path until session setup (#23) supplies one.
    /// Truncates: opening starts a new session journal.
    pub fn prototype_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/tfg-session-log.jsonl")
    }

    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        Ok(Self { next_seq: 0, writer: Some(BufWriter::new(File::create(path)?)) })
    }

    pub fn append(
        &mut self,
        game_ts: Option<String>,
        actor: impl Into<String>,
        kind: LogKind,
        payload: serde_json::Value,
    ) {
        let entry = LogEntry {
            seq: self.next_seq,
            game_ts,
            actor: actor.into(),
            kind,
            payload,
        };
        self.next_seq += 1;
        if let Some(w) = self.writer.as_mut() {
            if let Ok(mut line) = serde_json::to_string(&entry) {
                line.push('\n');
                let _ = w.write_all(line.as_bytes());
                let _ = w.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tfg-log-test-{name}.jsonl"))
    }

    #[test]
    fn appends_numbered_json_lines() {
        let path = scratch("numbered");
        let mut j = Journal::open(path.clone()).unwrap();
        j.append(None, "sim", LogKind::Marker, json!({"minute": 3}));
        j.append(None, "authority:1", LogKind::Command, json!({"legs": 2}));
        drop(j);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["seq"], 0);
        assert_eq!(first["kind"], "Marker");
        assert_eq!(first["payload"]["minute"], 3);
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["seq"], 1);
        assert_eq!(second["actor"], "authority:1");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn disabled_journal_is_a_quiet_noop() {
        let mut j = Journal::disabled();
        j.append(None, "sim", LogKind::Marker, json!({}));
    }
}
