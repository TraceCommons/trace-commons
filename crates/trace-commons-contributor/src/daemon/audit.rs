//! Local audit log of consequential, otherwise-invisible changes.
//!
//! Arming a project for automatic upload (every future session in it
//! uploads with no further prompt) and approving the whole pending queue at
//! once both used to require a terminal, which was itself a kind of
//! visibility -- a person had to be sitting at a shell to do either. That
//! restriction was removed, and this local log is what replaces it.
//!
//! Two more actions are logged for the same reason, both newly reachable
//! over the socket: widening consent scopes (a socket caller can silently
//! add e.g. `model_training`) and acknowledging the NEAR AI first-use
//! notice (a socket caller asserts, on its own unverified word, that a
//! third-party disclosure was actually shown to someone -- defeating that
//! gate is exactly as consequential as the other three, and less
//! recoverable once traces have already gone out under the false
//! acknowledgment).
//!
//! This is **user-facing visibility, not a security control**. It does not
//! gate, authorize, or prevent anything; it is written after the fact so a
//! contributor auditing their own machine has something to look at.
//!
//! Entries are label-only, matching the rest of this crate's audit and
//! logging conventions: no filesystem path, no `project_key` (a full local
//! path), no token, no URL, no trace content. `project_label` is the
//! disambiguated display label, never the key.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, DAEMON_AUDIT_FILE};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    /// A fixed label, e.g. "armed-auto-upload" or "bulk-approved". Never a
    /// message body or free text.
    pub action: String,
    pub project_label: Option<String>,
    /// A fixed label only, e.g. a count rendered as a string. Never free
    /// text.
    pub detail: Option<String>,
}

/// Append one entry to the log via a whole-file read-modify-write through
/// `write_daemon_file`, matching the shape used by `queue` and `history`.
pub fn append(store: &ConfigStore, entry: &AuditEntry) -> Result<()> {
    let mut entries = load(store)?;
    entries.push(entry.clone());
    save(store, &entries)
}

pub fn load(store: &ConfigStore) -> Result<Vec<AuditEntry>> {
    let Some(body) = store.read_daemon_file(DAEMON_AUDIT_FILE)? else {
        return Ok(Vec::new());
    };
    let text = String::from_utf8(body).context("audit log is not utf-8")?;
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<AuditEntry>(line) {
            Ok(e) => out.push(e),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        tracing::warn!(skipped, "skipped unparseable audit lines");
    }
    Ok(out)
}

fn save(store: &ConfigStore, entries: &[AuditEntry]) -> Result<()> {
    let mut body = String::new();
    for e in entries {
        body.push_str(&serde_json::to_string(e).context("serializing audit entry")?);
        body.push('\n');
    }
    store.write_daemon_file(DAEMON_AUDIT_FILE, body.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &str, project_label: Option<&str>) -> AuditEntry {
        AuditEntry {
            at: "2026-08-08T12:00:00Z".parse().unwrap(),
            action: action.to_string(),
            project_label: project_label.map(str::to_string),
            detail: None,
        }
    }

    #[test]
    fn entries_round_trip_in_order() {
        let (_d, store) = crate::config::tests_support::temp_store();
        append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
        append(&store, &entry("bulk-approved", None)).unwrap();
        let all = load(&store).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].action, "armed-auto-upload");
    }

    #[test]
    fn an_audit_entry_never_carries_a_path() {
        let (_d, store) = crate::config::tests_support::temp_store();
        append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
        let raw = store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(!text.contains('/'), "audit must be label-only: {text}");
    }

    #[test]
    fn a_corrupt_line_is_skipped_rather_than_losing_the_log() {
        let (_d, store) = crate::config::tests_support::temp_store();
        append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
        let mut raw = store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().unwrap();
        raw.extend_from_slice(b"not json\n");
        store.write_daemon_file(DAEMON_AUDIT_FILE, &raw).unwrap();
        assert_eq!(load(&store).unwrap().len(), 1);
    }

    #[test]
    fn the_audit_log_is_removed_on_logout() {
        let (_d, store) = crate::config::tests_support::temp_store();
        append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
        store.wipe().unwrap();
        assert!(store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().is_none());
    }
}
