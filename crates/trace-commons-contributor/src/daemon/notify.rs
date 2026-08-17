//! Telling the contributor there is something to look at, without becoming
//! noise.
//!
//! Notifications batch. A busy day across several repositories should be one
//! interruption, not a dozen, and the queue is durable so nothing is lost by
//! not interrupting: an ignored digest costs nothing.
//!
//! The daemon owns the *decision* to notify, because that policy is shared by
//! every application that attaches to it. Delivery is a `digest_due` event on
//! the subscription stream, which is the path the native applications use.
//! The local shell-out below exists so the daemon is useful on its own before
//! any of them ship, and is off unless explicitly enabled.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};

use super::queue::QueueEntry;

/// Whether a digest should fire now.
///
/// Never fires with an empty queue: a notification that says nothing is worse
/// than silence.
pub fn digest_due(
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_secs: u64,
    pending: usize,
) -> bool {
    if pending == 0 {
        return false;
    }
    match last_digest_at {
        None => true,
        Some(last) => now.signed_duration_since(last) >= Duration::seconds(interval_secs as i64),
    }
}

/// The digest line: how many sessions, and which projects they came from.
///
/// Built from labels only. A notification is rendered by a desktop
/// environment, may be logged by it, and must never contain a path.
pub fn digest_text(pending: &[&QueueEntry]) -> String {
    let count = pending.len();
    let noun = if count == 1 { "session" } else { "sessions" };
    let projects: BTreeSet<&str> = pending
        .iter()
        .map(|e| e.project_label.as_str())
        .filter(|l| !l.is_empty())
        .collect();
    if projects.is_empty() {
        return format!("{count} {noun} ready to contribute");
    }
    let named: Vec<&str> = projects.iter().take(3).copied().collect();
    let more = projects.len().saturating_sub(named.len());
    let list = named.join(", ");
    if more > 0 {
        format!("{count} {noun} ready to contribute from {list} and {more} more")
    } else {
        format!("{count} {noun} ready to contribute from {list}")
    }
}

/// Best-effort local OS notification.
///
/// Never fails the pipeline: a missing notifier, a headless machine, or a
/// daemon with no desktop session is a logged label, not a failed upload.
/// The text is passed as one argument, never interpolated into a shell.
pub fn emit_local(text: &str) {
    #[cfg(target_os = "macos")]
    let attempt = std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "display notification {} with title \"Trace Commons\"",
            applescript_string(text)
        ))
        .output();

    #[cfg(target_os = "linux")]
    let attempt = std::process::Command::new("notify-send")
        .arg("Trace Commons")
        .arg(text)
        .output();

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let attempt: std::io::Result<std::process::Output> = {
        let _ = text;
        Err(std::io::Error::other("unsupported-platform"))
    };

    match attempt {
        Ok(out) if out.status.success() => {}
        _ => tracing::debug!("notifier-unavailable"),
    }
}

/// Quote a string for embedding in an AppleScript literal.
#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::queue::{QueueEntry, QueueState, entry_id_for};
    use std::path::PathBuf;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn entry(label: &str) -> QueueEntry {
        QueueEntry {
            entry_id: entry_id_for(label),
            session_hash: format!("sha256:{label}"),
            source: "claude-code".into(),
            project_key: format!("/Users/z/code/{label}"),
            project_label: label.into(),
            path: PathBuf::from("/Users/z/.claude/projects/x/s.jsonl"),
            size_bytes: 10,
            discovered_at: at("2026-08-08T12:00:00Z"),
            state: QueueState::Pending,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
            approved_scopes: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 0,
            subagents_dropped: 0,
        }
    }

    #[test]
    fn a_digest_is_not_due_before_the_interval_elapses() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T14:00:00Z"),
            14400,
            3
        ));
    }

    #[test]
    fn a_digest_is_due_after_the_interval_with_pending_work() {
        assert!(digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T16:01:00Z"),
            14400,
            3
        ));
    }

    #[test]
    fn a_digest_is_never_due_with_an_empty_queue() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-09T12:00:00Z"),
            14400,
            0
        ));
        assert!(!digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 0));
    }

    #[test]
    fn the_first_digest_is_due_immediately_when_work_exists() {
        assert!(digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 1));
    }

    #[test]
    fn digest_text_names_projects_and_never_a_path() {
        let a = entry("proj");
        let b = entry("proj");
        let c = entry("other");
        let text = digest_text(&[&a, &b, &c]);
        assert!(text.contains("3 sessions"), "{text}");
        assert!(text.contains("proj"), "{text}");
        assert!(text.contains("other"), "{text}");
        assert!(
            !text.contains('/'),
            "digest text must not contain a path: {text}"
        );
    }

    #[test]
    fn digest_text_is_singular_for_one_session() {
        let a = entry("proj");
        let text = digest_text(&[&a]);
        assert!(text.contains("1 session ready"), "{text}");
        assert!(!text.contains("sessions"), "{text}");
    }

    #[test]
    fn digest_text_summarises_rather_than_listing_every_project() {
        let entries: Vec<QueueEntry> = ["a", "b", "c", "d", "e"].iter().map(|l| entry(l)).collect();
        let refs: Vec<&QueueEntry> = entries.iter().collect();
        let text = digest_text(&refs);
        assert!(text.contains("and 2 more"), "{text}");
    }

    #[test]
    fn digest_text_copes_with_missing_labels() {
        let mut a = entry("proj");
        a.project_label = String::new();
        let text = digest_text(&[&a]);
        assert_eq!(text, "1 session ready to contribute");
    }
}
