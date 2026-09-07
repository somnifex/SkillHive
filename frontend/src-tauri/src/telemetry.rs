//! Structured desktop telemetry (M4, handoff §16 observability).
//!
//! One JSON line per event, appended to `<app data>/logs/skillhive.log`.
//! The emitter is deliberately dependency-free and failure-tolerant: a log
//! write never propagates an error into sync correctness paths — telemetry
//! is diagnostics, and its failure mode is "line dropped".
//!
//! Redaction contract (handoff §16): events carry identifiers and counts
//! only — never tokens, passwords, skill bodies, or workspace file
//! contents. Callers pass field slices, so the schema is enumerable at
//! every call site.
//!
//! Rotation is size-based and conservative: past `MAX_LOG_BYTES` the file
//! is truncated to a fresh file (one-generation rollover via `.old`).

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use chrono::Utc;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Cap before rollover; generous for text lines, small on disk.
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Points the emitter at the app's log file. Call once during setup;
/// events before initialization are dropped (pre-log-path startup is
/// exactly the window where structured logging has no consumer yet).
pub fn init(log_path: PathBuf) {
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = LOG_PATH.set(log_path);
}

/// Emits one structured event. Never panics and never blocks sync
/// correctness: all filesystem failures are swallowed by contract.
pub fn event(kind: &str, fields: &[(&str, &str)]) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let mut line = format!(
        "{{\"ts\":\"{}\",\"event\":\"{}\"",
        Utc::now().to_rfc3339(),
        json_escape(kind)
    );
    for (key, value) in fields {
        line.push_str(&format!(
            ",\"{}\":\"{}\"",
            json_escape(key),
            json_escape(value)
        ));
    }
    line.push('}');

    let _guard = WRITE_LOCK.lock();
    let Ok(mut file) = OpenOptions::new().append(true).create(true).open(path) else {
        return;
    };
    if let Ok(metadata) = file.metadata() {
        if metadata.len() > MAX_LOG_BYTES {
            drop(file);
            let old = path.with_extension("old");
            let _ = fs::rename(path, &old);
            let Ok(reopened) = OpenOptions::new().append(true).create(true).open(path) else {
                return;
            };
            file = reopened;
        }
    }
    let _ = writeln!(file, "{line}");
}

/// Convenience for a plain message event with no fields.
pub fn message(kind: &str) {
    event(kind, &[]);
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if (c as u32) < 0x20 => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_path(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        // Leak the tempdir for the process lifetime of the test; LOG_PATH is
        // a process-global so the directory must outlive it.
        std::mem::forget(dir);
        path
    }

    #[test]
    fn events_append_json_lines() {
        let path = fresh_path("skillhive-test-append.log");
        init(path.clone());
        event("cycle_end", &[("cycle", "abc"), ("pushed", "2")]);
        event("message_only", &[]);
        let text = fs::read_to_string(&path).expect("read log");
        let mut lines = text.lines();
        let first = lines.next().expect("first line");
        let second = lines.next().expect("second line");
        let parsed: serde_json::Value = serde_json::from_str(first).expect("line 1 parses");
        assert_eq!(parsed["event"], "cycle_end");
        assert_eq!(parsed["cycle"], "abc");
        assert_eq!(parsed["pushed"], "2");
        assert!(parsed.get("ts").is_some());
        let parsed2: serde_json::Value = serde_json::from_str(second).expect("line 2 parses");
        assert_eq!(parsed2["event"], "message_only");
    }

    #[test]
    fn values_are_json_escaped() {
        assert_eq!(json_escape("a\"b\\c\nd\te"), "a\\\"b\\\\c\\nd\\te");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
        assert_eq!(json_escape("plain"), "plain");
    }

    #[test]
    fn events_before_init_are_dropped() {
        // Do not init; this must not panic. (Events before a prior init in
        // the same process reuse that path — acceptable for tests.)
        event("pre_init_probe", &[]);
    }
}
