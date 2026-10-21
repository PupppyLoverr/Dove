//! A record of each call: when it ran, which threads it talked to, and its
//! spoken turns in order. The turns themselves also land in each thread's
//! normal transcript (they are sent like typed messages); this log adds what
//! the transcript format has no place for: that a turn was spoken, and the
//! call it belonged to. No audio is ever stored.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Started {
        call: String,
        at_ms: i64,
        engines: String,
    },
    /// The user spoke; `thread` is where it went (if anywhere).
    User {
        call: String,
        at_ms: i64,
        text: String,
        thread: Option<String>,
    },
    /// Dove spoke (a status, an acknowledgement, a read-out of a reply).
    Dove {
        call: String,
        at_ms: i64,
        text: String,
        thread: Option<String>,
    },
    Ended {
        call: String,
        at_ms: i64,
        duration_ms: i64,
        threads: Vec<String>,
    },
}

pub fn path(root: &Path) -> PathBuf {
    root.join("voice").join("calls.jsonl")
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Append one entry. Failures are logged, never fatal to the call.
pub fn append(root: &Path, entry: &Entry) {
    let p = path(root);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = match serde_json::to_string(entry) {
        Ok(l) => l,
        Err(_) => return,
    };
    match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("voice call log: {e}"),
    }
}

/// Every entry of one call, in the order written.
pub fn read_call(root: &Path, call: &str) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path(root)) else { return Vec::new() };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
        .filter(|e| match e {
            Entry::Started { call: c, .. }
            | Entry::User { call: c, .. }
            | Entry::Dove { call: c, .. }
            | Entry::Ended { call: c, .. } => c == call,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        append(r, &Entry::Started { call: "c1".into(), at_ms: 1, engines: "parakeet/system".into() });
        append(r, &Entry::User { call: "c1".into(), at_ms: 2, text: "status".into(), thread: None });
        append(r, &Entry::User { call: "c2".into(), at_ms: 2, text: "other call".into(), thread: None });
        append(
            r,
            &Entry::Dove { call: "c1".into(), at_ms: 3, text: "Codex is working.".into(), thread: Some("t2".into()) },
        );
        append(r, &Entry::Ended { call: "c1".into(), at_ms: 4, duration_ms: 3, threads: vec!["t2".into()] });
        let got = read_call(r, "c1");
        assert_eq!(got.len(), 4);
        assert!(matches!(got[1], Entry::User { ref text, .. } if text == "status"));
        assert!(matches!(got[3], Entry::Ended { duration_ms: 3, .. }));
    }
}
