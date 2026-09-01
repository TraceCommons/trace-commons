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
/// Never fires with nothing to say: a notification that says nothing is worse
/// than silence.
///
/// "Nothing to say" used to mean an empty queue alone, which was right while
/// every upload passed through review -- an empty queue then meant an idle
/// period. It stopped being right once a project could be armed to contribute
/// without asking: an armed project never queues anything, so `pending` stays
/// 0 no matter how much it sends, and the contributor who most wanted to stop
/// supervising heard nothing at all. Contribution is now a reason to speak in
/// its own right.
///
/// `contributed` does not get its own clock. The interval is the whole point
/// of a digest -- one interruption per period, whatever the period held.
pub fn digest_due(
    last_digest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_secs: u64,
    pending: usize,
    contributed: usize,
) -> bool {
    if pending == 0 && contributed == 0 {
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

/// The contribution half of the digest: what went out without being asked
/// about since the last one, and what credit it is carrying.
///
/// Built from labels only, for the same reason `digest_text` is: a
/// notification is rendered by a desktop environment, may be logged by it,
/// and must never contain a path.
///
/// Credit is stated only when there is some. A trailing "0 credit pending"
/// reads as a failure rather than as a fresh start, and the first digest
/// after arming a project is exactly when that would show. It is always
/// named `pending`, never "earned" -- settlement is off on every deployment
/// shipped so far (see `docs/operator/settlement-mode.md`), so a bare figure
/// would be read as money that exists.
pub fn contribution_text(count: usize, projects: &BTreeSet<String>, credit_pending: f32) -> String {
    let noun = if count == 1 { "session" } else { "sessions" };
    let mut line = format!("{count} {noun} contributed");
    if !projects.is_empty() {
        let named: Vec<&str> = projects.iter().take(3).map(|s| s.as_str()).collect();
        let more = projects.len().saturating_sub(named.len());
        let list = named.join(", ");
        if more > 0 {
            line.push_str(&format!(" from {list} and {more} more"));
        } else {
            line.push_str(&format!(" from {list}"));
        }
    }
    // One decimal place: credit is a score, not an amount, and trailing
    // precision invites the reader to treat it as a balance.
    if credit_pending > 0.0 {
        line.push_str(&format!(". {credit_pending:.1} credit pending"));
    }
    line
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

    fn labels(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
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
            approved_verdict: None,
            approved_correction: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 0,
            subagents_dropped: 0,
            observed_modified_at: None,
        }
    }

    #[test]
    fn a_digest_is_not_due_before_the_interval_elapses() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T14:00:00Z"),
            14400,
            3,
            0
        ));
    }

    #[test]
    fn a_digest_is_due_after_the_interval_with_pending_work() {
        assert!(digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T16:01:00Z"),
            14400,
            3,
            0
        ));
    }

    #[test]
    fn a_digest_is_never_due_with_nothing_to_say() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-09T12:00:00Z"),
            14400,
            0,
            0
        ));
        assert!(!digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 0, 0));
    }

    #[test]
    fn the_first_digest_is_due_immediately_when_work_exists() {
        assert!(digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 1, 0));
    }

    /// The hole this closes. An armed project uploads without ever queuing
    /// anything, so `pending` stays 0 forever and the old gate refused every
    /// digest -- a contributor who armed everything heard nothing at all,
    /// which is the opposite of what arming is supposed to feel like.
    #[test]
    fn a_digest_is_due_for_contributions_alone_with_an_empty_queue() {
        assert!(digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T16:01:00Z"),
            14400,
            0,
            7
        ));
        assert!(digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 0, 1));
    }

    /// Contributions do not get their own faster clock. The interval is the
    /// whole point of a digest: one interruption per period, whatever the
    /// period contained.
    #[test]
    fn contributions_do_not_shorten_the_interval() {
        assert!(!digest_due(
            Some(at("2026-08-08T12:00:00Z")),
            at("2026-08-08T14:00:00Z"),
            14400,
            0,
            99
        ));
    }

    #[test]
    fn contribution_text_names_projects_and_never_a_path() {
        let text = contribution_text(3, &labels(&["proj", "other"]), 0.0);
        assert!(text.contains("3 sessions contributed"), "{text}");
        assert!(text.contains("proj"), "{text}");
        assert!(text.contains("other"), "{text}");
        assert!(!text.contains('/'), "must not contain a path: {text}");
    }

    #[test]
    fn contribution_text_is_singular_for_one_session() {
        let text = contribution_text(1, &labels(&["proj"]), 0.0);
        assert!(text.contains("1 session contributed"), "{text}");
        assert!(!text.contains("sessions"), "{text}");
    }

    #[test]
    fn contribution_text_summarises_rather_than_listing_every_project() {
        let text = contribution_text(9, &labels(&["a", "b", "c", "d", "e"]), 0.0);
        assert!(text.contains("and 2 more"), "{text}");
    }

    #[test]
    fn contribution_text_copes_with_missing_labels() {
        assert_eq!(
            contribution_text(2, &labels(&[]), 0.0),
            "2 sessions contributed"
        );
    }

    /// Credit is the other half of the value exchange and is the reason this
    /// line exists at all -- but only when there is some. A trailing
    /// "0 credit pending" reads as a failure rather than as a fresh start.
    #[test]
    fn contribution_text_states_credit_only_when_there_is_some() {
        let with = contribution_text(2, &labels(&["proj"]), 4.25);
        assert!(with.contains("4.2 credit pending"), "{with}");
        let without = contribution_text(2, &labels(&["proj"]), 0.0);
        assert!(!without.contains("credit"), "{without}");
    }

    /// The figure is pending, and saying so is the difference between a
    /// record and a promise. Settlement is off on every deployment shipped
    /// so far, so a bare number would be read as money.
    #[test]
    fn contribution_text_never_calls_pending_credit_earned() {
        let text = contribution_text(2, &labels(&["proj"]), 4.25);
        assert!(text.contains("pending"), "{text}");
        for word in ["earned", "paid", "settled", "worth"] {
            assert!(!text.contains(word), "must not say {word}: {text}");
        }
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
