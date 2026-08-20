//! The typed half of `trace_commons.daemon.v1_1`.
//!
//! Every field here exists on the wire in
//! `docs/contributor-daemon-ipc-v1_1.md`. Nothing is invented, and nothing
//! that the contract keeps off the wire -- a filesystem path, a token, a
//! project key -- has a home in these types, so a rendering mistake cannot
//! put one on screen.
//!
//! Deserialization is deliberately tolerant of unknown fields: the contract
//! is additive, and a shell that refused a daemon newer than itself would
//! break on the next additive revision.

use serde::Deserialize;

/// `status`.
#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    #[serde(default)]
    pub logged_in: bool,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub consent_scopes: Vec<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub queue_depth: u64,
    #[serde(default)]
    pub next_digest_at: Option<String>,
    #[serde(default)]
    pub health: Health,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Health {
    #[serde(default)]
    pub last_error_label: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
}

/// One queue entry, as `list_pending` and the `snapshot` event carry it.
///
/// `project_key` and `path` are absent from the wire by design; they are
/// absent from this struct for the same reason.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueEntry {
    pub entry_id: String,
    #[serde(default)]
    pub session_hash: String,
    /// `claude-code`, `codex`, or `trajectory`: which agent produced this.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub project_label: String,
    /// The size of the session file on disk. This is **not** what would be
    /// sent; `PreviewSummary::would_send_bytes` is, and it is usually
    /// larger. Never label this one "would send".
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub discovered_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub reason_label: Option<String>,
}

impl QueueEntry {
    /// The agent that produced the session, in the words a contributor uses
    /// for it.
    pub fn agent_label(&self) -> &str {
        match self.source.as_str() {
            "claude-code" => "Claude Code",
            "codex" => "Codex",
            "trajectory" => "Trajectory",
            other => other,
        }
    }
}

/// `preview`, and the in-process preview the hosting shell can run.
#[derive(Debug, Clone, Deserialize)]
pub struct PreviewSummary {
    #[serde(default)]
    pub would_send_bytes: u64,
    #[serde(default)]
    pub raw_session_bytes: u64,
    #[serde(default)]
    pub event_count: u64,
    /// Redacted trace content, and the one place the contract permits it.
    #[serde(default)]
    pub opening_prompt: String,
    #[serde(default)]
    pub redactions: std::collections::BTreeMap<String, u32>,
    #[serde(default)]
    pub pii_labels_present: Vec<String>,
    #[serde(default)]
    pub consent_scopes: Vec<String>,
    #[serde(default)]
    pub residual_risk: String,
    #[serde(default)]
    pub envelope_digest: String,
    #[serde(default)]
    pub input_fingerprint: String,
    /// `false` means this preview was built from a placeholder identity and
    /// nothing was pinned: an illustration, not something to approve
    /// against.
    #[serde(default)]
    pub enrolled: bool,
}

impl PreviewSummary {
    /// The redaction receipt line from the shared spec, e.g.
    /// `scrubbed: 12 secrets, 4 tokens, 31 paths`.
    ///
    /// Category names come from the daemon; they are labels, never matched
    /// text. A count of zero is reported as `scrubbed: nothing` rather than
    /// hidden -- the whole point of the receipt is that `0` on a session
    /// that obviously touched a `.env` is a signal the contributor can act
    /// on.
    pub fn scrubbed_line(&self) -> String {
        let total: u32 = self.redactions.values().sum();
        if total == 0 {
            return "scrubbed: nothing".to_string();
        }
        // Several daemon-side categories map onto one word a contributor
        // uses -- `aws_secret_key` and `generic_secret` are both "secrets".
        // Their counts are summed rather than listed twice: "1 secrets, 1
        // secrets" reads as a bug, and it is one.
        let mut totals: std::collections::BTreeMap<String, u32> = Default::default();
        for (kind, n) in self.redactions.iter().filter(|(_, n)| **n > 0) {
            *totals.entry(humanize_redaction_kind(kind)).or_default() += n;
        }
        // Ordered as the shared spec writes the line -- "12 secrets, 4
        // tokens, 31 paths" -- most alarming first, rather than
        // alphabetically. What a contributor scans for is whether a secret
        // was in there, not whether a path was.
        let mut ordered: Vec<(String, u32)> = totals.into_iter().collect();
        ordered.sort_by_key(|(word, _)| (severity_rank(word), word.clone()));
        let parts: Vec<String> = ordered
            .iter()
            .map(|(word, n)| format!("{n} {}", pluralize(word, *n)))
            .collect();
        format!("scrubbed: {}", parts.join(", "))
    }
}

fn severity_rank(word: &str) -> u8 {
    match word {
        "secrets" => 0,
        "keys" => 1,
        "tokens" => 2,
        "email addresses" => 3,
        "URLs" => 4,
        "paths" => 5,
        _ => 6,
    }
}

/// The categories above are written plural, since that is the common case;
/// a count of one gets the singular back.
fn pluralize(word: &str, n: u32) -> String {
    if n != 1 {
        return word.to_string();
    }
    match word {
        "email addresses" => "email address".to_string(),
        w => w.strip_suffix('s').unwrap_or(w).to_string(),
    }
}

/// Turn a daemon-side redaction category into ordinary words. Unknown
/// categories fall through with their underscores softened rather than
/// being dropped: an unrecognized category is still a real redaction and
/// hiding it would understate the receipt.
fn humanize_redaction_kind(kind: &str) -> String {
    let base = match kind {
        k if k.contains("secret") => "secrets",
        k if k.contains("token") => "tokens",
        k if k.contains("key") => "keys",
        k if k.contains("path") => "paths",
        k if k.contains("email") => "email addresses",
        k if k.contains("url") => "URLs",
        _ => return kind.replace('_', " "),
    };
    base.to_string()
}

/// `approve`.
#[derive(Debug, Clone, Deserialize)]
pub struct ApproveResult {
    #[serde(default)]
    pub approved: u64,
    #[serde(default)]
    pub hold_secs: u64,
    /// The instant the daemon will first consider the entry for upload.
    ///
    /// A countdown runs against **this**, never against a duration the
    /// shell picked. `None` means no undo may be offered at all.
    #[serde(default)]
    pub hold_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// `list_projects`.
#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub project_id: String,
    #[serde(default)]
    pub project_label: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub configured: bool,
    /// The row holding sessions whose working directory had no usable final
    /// segment. It can be silenced but never armed -- `Policy` refuses
    /// `auto_upload` for it in two independent places -- so a shell reports
    /// that rather than enforcing it.
    ///
    /// The daemon says so explicitly because it is the only side that knows
    /// it for free. Deriving it would mean re-deriving `project_id_for`'s
    /// hash, and the IPC contract states clients MUST NOT recognise the row
    /// by `project_label`: that string is display text, and every shell
    /// rewords it precisely because the raw slug is not something a
    /// contributor should read.
    #[serde(default)]
    pub is_unresolved_bucket: bool,
}

/// `list_history`.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryRecord {
    /// The server's id for this submission, and the only handle `withdraw`
    /// takes. Not an identity and not a path -- an opaque uuid the daemon
    /// already put on the wire. `#[serde(default)]` like every other field
    /// here, so a daemon that stopped sending it degrades to "this row has
    /// no withdraw button" rather than to a history screen that will not
    /// parse.
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub project_label: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub credit_points_pending: f32,
    #[serde(default)]
    pub credit_points_final: Option<f32>,
    /// The server's own prose. Rendered verbatim; a status word is a poor
    /// substitute for "held because a passage looked like a personal
    /// address".
    #[serde(default)]
    pub explanations: Vec<String>,
}

/// `history_rollup`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryRollup {
    #[serde(default)]
    pub week: Counts,
    #[serde(default)]
    pub month: Counts,
    #[serde(default)]
    pub all_time: Counts,
    #[serde(default)]
    pub credit_pending: f32,
    #[serde(default)]
    pub credit_final: f32,
    #[serde(default)]
    pub quarantined: u32,
    /// `null` renders as "Not synced yet", never as a confident `0.0`.
    #[serde(default)]
    pub last_refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Counts {
    #[serde(default)]
    pub submitted: u32,
    #[serde(default)]
    pub accepted: u32,
    #[serde(default)]
    pub quarantined: u32,
    #[serde(default)]
    pub other: u32,
}

/// `get_settings`. The three booleans are configured-or-not facts; the
/// underlying credential and paths never cross the socket and have no field
/// here to land in.
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub quiescence_secs: u64,
    #[serde(default)]
    pub digest_interval_secs: u64,
    #[serde(default)]
    pub approval_hold_secs: u64,
    #[serde(default)]
    pub local_notifications: bool,
    #[serde(default)]
    pub near_ai_configured: bool,
    #[serde(default)]
    pub claude_root_configured: bool,
    #[serde(default)]
    pub codex_root_configured: bool,
}

/// `consent_options`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsentScope {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub always_on: bool,
    #[serde(default)]
    pub grants_data_use: bool,
}

/// Format a byte count for a contributor deciding whether to send it.
pub fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} bytes")
    }
}

/// "3 hours ago", for a queue row. Never an absolute timestamp: a
/// contributor placing a session in their own day thinks in elapsed time.
pub fn human_when(then: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let Some(then) = then else {
        return "just now".to_string();
    };
    let mins = (chrono::Utc::now() - then).num_minutes().max(0);
    match mins {
        0..=1 => "just now".to_string(),
        2..=59 => format!("{mins} minutes ago"),
        60..=119 => "an hour ago".to_string(),
        120..=1439 => format!("{} hours ago", mins / 60),
        1440..=2879 => "yesterday".to_string(),
        _ => format!("{} days ago", mins / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_receipt_with_no_redactions_says_so_rather_than_going_quiet() {
        let s = PreviewSummary {
            would_send_bytes: 0,
            raw_session_bytes: 0,
            event_count: 0,
            opening_prompt: String::new(),
            redactions: Default::default(),
            pii_labels_present: vec![],
            consent_scopes: vec![],
            residual_risk: String::new(),
            envelope_digest: String::new(),
            input_fingerprint: String::new(),
            enrolled: true,
        };
        assert_eq!(s.scrubbed_line(), "scrubbed: nothing");
    }

    #[test]
    fn the_receipt_reads_as_the_shared_spec_writes_it() {
        let mut redactions = std::collections::BTreeMap::new();
        redactions.insert("aws_secret_key".to_string(), 12);
        redactions.insert("bearer_token".to_string(), 4);
        redactions.insert("home_path".to_string(), 31);
        let s = PreviewSummary {
            would_send_bytes: 86016,
            raw_session_bytes: 1,
            event_count: 1,
            opening_prompt: String::new(),
            redactions,
            pii_labels_present: vec![],
            consent_scopes: vec![],
            residual_risk: "pattern-based".to_string(),
            envelope_digest: String::new(),
            input_fingerprint: String::new(),
            enrolled: true,
        };
        assert_eq!(
            s.scrubbed_line(),
            "scrubbed: 12 secrets, 4 tokens, 31 paths"
        );
        assert_eq!(human_bytes(s.would_send_bytes), "84 KB");
    }

    #[test]
    fn an_unknown_redaction_category_still_appears_in_the_receipt() {
        let mut redactions = std::collections::BTreeMap::new();
        redactions.insert("some_new_shape".to_string(), 2);
        let s = PreviewSummary {
            would_send_bytes: 0,
            raw_session_bytes: 0,
            event_count: 0,
            opening_prompt: String::new(),
            redactions,
            pii_labels_present: vec![],
            consent_scopes: vec![],
            residual_risk: String::new(),
            envelope_digest: String::new(),
            input_fingerprint: String::new(),
            enrolled: true,
        };
        assert_eq!(s.scrubbed_line(), "scrubbed: 2 some new shape");
    }

    #[test]
    fn categories_that_mean_the_same_word_are_summed_not_listed_twice() {
        let mut redactions = std::collections::BTreeMap::new();
        redactions.insert("aws_secret_key".to_string(), 1);
        redactions.insert("generic_secret".to_string(), 1);
        redactions.insert("email".to_string(), 1);
        let s = PreviewSummary {
            would_send_bytes: 0,
            raw_session_bytes: 0,
            event_count: 0,
            opening_prompt: String::new(),
            redactions,
            pii_labels_present: vec![],
            consent_scopes: vec![],
            residual_risk: String::new(),
            envelope_digest: String::new(),
            input_fingerprint: String::new(),
            enrolled: true,
        };
        assert_eq!(s.scrubbed_line(), "scrubbed: 2 secrets, 1 email address");
    }
}
