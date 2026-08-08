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
//! gate, authorize, or prevent anything; a contributor auditing their own
//! machine has something to look at, and that is all.
//!
//! It is nonetheless written **fail-closed**. Every call site treats an
//! append failure as fatal to the action it was recording and rolls that
//! action back (or, where there is nothing to roll back to, records first
//! and acts second). Best-effort appending reduced a disk-full or
//! permissions failure to a log warning while the call still returned
//! success -- which silently defeats the one mechanism a deliberately
//! removed terminal-only restriction was replaced with. Being "not a
//! security control" is a statement about what the log can prove, not a
//! licence to skip writing it.
//!
//! The log is capped at `MAX_AUDIT_ENTRIES` and rotates oldest-first.
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

/// The durable log's hard ceiling, in entries.
///
/// `append` is a whole-file read-modify-write (the shape `queue` and
/// `history` use), so an unbounded file makes every subsequent append
/// slower and likelier to fail -- and an append failure now *refuses* the
/// action it was recording, so an unbounded file eventually starts breaking
/// the very calls it audits. Capping `list_audit`'s output does nothing
/// about that; only capping the file does.
///
/// Rotation drops the oldest entries. This is a local visibility record,
/// not evidence: a contributor asking "what was armed, and when" is asking
/// about recent history, and losing a year-old entry is a far better
/// outcome than an append that fails and blocks a legitimate change.
pub const MAX_AUDIT_ENTRIES: usize = 5_000;

/// Append one entry to the log via a whole-file read-modify-write through
/// `write_daemon_file`, matching the shape used by `queue` and `history`.
///
/// Rotates to the newest `MAX_AUDIT_ENTRIES` on the way out. Callers must
/// treat an `Err` as fatal to whatever they were doing: every call site
/// rolls its own change back rather than letting a change stand with no
/// record of it. The audit log is not a security control, but it is the
/// stated replacement for a removed terminal-only restriction, and a
/// silently-skipped append defeats exactly that.
pub fn append(store: &ConfigStore, entry: &AuditEntry) -> Result<()> {
    let mut entries = load(store)?;
    entries.push(entry.clone());
    if entries.len() > MAX_AUDIT_ENTRIES {
        entries.drain(..entries.len() - MAX_AUDIT_ENTRIES);
    }
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
        // The fixture used to be the hand-written literal "proj", which
        // could not have caught a `project_label` that had degenerated into
        // a full local path -- the test passed because its own input was
        // already clean. Every label written here now comes from
        // `policy::project_label_for`, including the degenerate working
        // directories whose basename `Path::file_name` cannot produce (`/`,
        // anything ending in `..`, the empty string), which used to fall
        // back to the raw key.
        let (_d, store) = crate::config::tests_support::temp_store();
        for cwd in [
            "/Users/z/code/secret-client-project",
            "/",
            "/Users/z/code/..",
            "..",
            "",
        ] {
            let key = crate::daemon::policy::project_key_for(Some(cwd));
            let label = crate::daemon::policy::project_label_for(&key);
            append(&store, &entry("armed-auto-upload", Some(&label))).unwrap();
        }
        let raw = store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(!text.contains('/'), "audit must be label-only: {text}");
        assert!(
            !text.contains("secret-client-project/") && !text.contains("Users"),
            "audit must be label-only: {text}"
        );
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
    fn the_log_is_capped_and_keeps_the_newest_entries() {
        // Without a cap the file grows forever, and since an append failure
        // now refuses the action it was recording, an unbounded file
        // eventually breaks the calls it audits.
        let (_d, store) = crate::config::tests_support::temp_store();
        let mut entries = Vec::new();
        for i in 0..MAX_AUDIT_ENTRIES + 10 {
            let mut e = entry("armed-auto-upload", Some("proj"));
            e.detail = Some(i.to_string());
            entries.push(e);
        }
        save(&store, &entries).unwrap();
        let mut last = entry("bulk-approved", None);
        last.detail = Some("last".to_string());
        append(&store, &last).unwrap();

        let all = load(&store).unwrap();
        assert_eq!(all.len(), MAX_AUDIT_ENTRIES);
        assert_eq!(all.last().unwrap().detail.as_deref(), Some("last"));
        // The oldest entries are the ones dropped.
        assert_eq!(all.first().unwrap().detail.as_deref(), Some("11"));
    }

    #[test]
    fn the_audit_log_is_removed_on_logout() {
        let (_d, store) = crate::config::tests_support::temp_store();
        append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
        store.wipe().unwrap();
        assert!(store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().is_none());
    }
}
