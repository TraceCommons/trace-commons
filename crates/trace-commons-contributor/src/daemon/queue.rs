//! The pending queue: sessions that are ready to upload and waiting on the
//! contributor.
//!
//! The queue is durable because a notification is not. Someone who ignores a
//! digest, closes a window, or reboots must still find their pending traces
//! where they left them, and someone who never looks must not accumulate an
//! unbounded backlog.
//!
//! Three distinct ways of saying "no" coexist here on purpose, because they
//! answer different questions: `Ignore` is a standing decision about a project
//! (handled in `policy`), `dismiss` is a decision about one session, and
//! `Expired` is a record of inaction. Consumers render each differently.
//!
//! Expiry is suspended while the daemon is unhealthy. A privacy-filter outage
//! is not the contributor declining to upload, and letting a two-week clock
//! run through one would silently discard traces nobody chose to discard.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{ConfigStore, DAEMON_QUEUE_FILE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    /// Waiting on the contributor.
    Pending,
    /// The contributor said yes; not yet uploaded.
    Approved,
    /// Upload in flight.
    Uploading,
    /// Delivered to the server.
    Uploaded,
    /// The pipeline refused it, e.g. a residual secret or an unavailable
    /// privacy filter.
    Refused,
    /// Network or auth failure after retries.
    Failed,
    /// Aged out of the queue without a decision.
    Expired,
    /// The session changed after this entry was offered; a fresh entry
    /// replaced it.
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub entry_id: Uuid,
    pub session_hash: String,
    pub source: String,
    /// The full local working directory. Local-only, like `path`.
    pub project_key: String,
    /// What consumers display.
    pub project_label: String,
    /// The session file. Present so the uploader can re-read and re-hash it.
    /// It never leaves this file: not into a receipt, a history record, a log
    /// line, or the wire.
    pub path: PathBuf,
    pub size_bytes: u64,
    pub discovered_at: DateTime<Utc>,
    pub state: QueueState,
    /// A fixed label, never a message body or response text.
    pub reason_label: Option<String>,
    pub attempts: u32,
    pub retry_after: Option<DateTime<Utc>>,
    pub submission_id: Option<Uuid>,
    /// The consent scopes in force at the moment this entry was approved.
    ///
    /// Consent scopes live in the contributor config and can be rewritten
    /// at any time (`set_consent_scopes`), with nothing coupling that
    /// rewrite to entries already approved. Without this field, a preview
    /// could show one scope set, the contributor approve on the strength of
    /// it, someone widen the scopes, and the very same trace upload under
    /// the wider set. The uploader refuses in that case and returns the
    /// entry to `Pending`, exactly as the re-hash guard does when the
    /// *content* moves after approval -- an approval covers a description,
    /// and the scopes are part of that description.
    ///
    /// `None` on entries written before this field existed, and on entries
    /// that have not been approved yet. `None` on an approved entry is
    /// treated as "unknown, so re-ask": fail-closed.
    #[serde(default)]
    pub approved_scopes: Option<Vec<String>>,
    /// The fingerprint of everything outside the session file that
    /// determines the envelope, as of the moment this entry was approved
    /// (`preview::input_fingerprint`).
    ///
    /// `approved_scopes` was a narrow version of this -- it caught one
    /// envelope-determining input moving between approval and send. Every
    /// other one moved silently: the PII filter selection, the NEAR AI
    /// backend and model, the identity and endpoints the envelope is
    /// stamped with. The raw-hash guard could not see any of it, because it
    /// re-hashes the *input*, not the artifact.
    ///
    /// `None` on entries written before this field existed and on
    /// unapproved entries. `None` on an approved entry is "unknown, so
    /// re-ask": fail-closed.
    #[serde(default)]
    pub approved_inputs: Option<String>,
    /// The digest of the redacted envelope the contributor was actually
    /// shown (`preview::envelope_digest`), recorded when a preview is run
    /// for this entry and carried through an approval.
    ///
    /// `Some` means **the envelope itself is on disk** under
    /// `daemon::approved_envelope`, and the upload sends precisely those
    /// bytes rather than building a second envelope. The digest identifies
    /// that file; it is never compared against a rebuild, because a
    /// rebuild through an LLM-backed privacy filter legitimately differs
    /// and comparing made previewed entries permanently unuploadable.
    ///
    /// If the stored bytes are missing or unusable when the upload comes to
    /// read them, the approval is revoked and the entry re-offered. The
    /// daemon does not fall back to rebuilding.
    ///
    /// `None` when the entry was never previewed -- an armed auto-upload
    /// project, or an approve-all. Those are approvals given without seeing
    /// the artifact in the first place, so there is nothing stored to send
    /// and the pipeline builds the envelope as it always did; the input
    /// fingerprint is what covers them.
    #[serde(default)]
    pub previewed_envelope_digest: Option<String>,
}

/// A stable id for a queue entry, derived from the session hash so the same
/// session keeps the same id across daemon restarts and across a queue file
/// rewritten from scratch.
pub fn entry_id_for(session_hash: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, session_hash.as_bytes())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    entries: Vec<QueueEntry>,
}

impl Queue {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_QUEUE_FILE)? else {
            return Ok(Self::new());
        };
        let text = String::from_utf8(body).context("queue file is not utf-8")?;
        let mut entries = Vec::new();
        let mut skipped = 0usize;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<QueueEntry>(line) {
                Ok(e) => entries.push(e),
                Err(_) => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(skipped, "skipped unparseable queue lines");
        }
        Ok(Self { entries })
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let mut body = String::new();
        for e in &self.entries {
            body.push_str(&serde_json::to_string(e).context("serializing queue entry")?);
            body.push('\n');
        }
        store.write_daemon_file(DAEMON_QUEUE_FILE, body.as_bytes())
    }

    pub fn all(&self) -> &[QueueEntry] {
        &self.entries
    }

    pub fn pending(&self) -> Vec<&QueueEntry> {
        self.entries
            .iter()
            .filter(|e| e.state == QueueState::Pending)
            .collect()
    }

    pub fn get(&self, entry_id: Uuid) -> Option<&QueueEntry> {
        self.entries.iter().find(|e| e.entry_id == entry_id)
    }

    /// Add an entry, or leave the existing one alone if this session is
    /// already tracked. Idempotent because the watcher re-observes the same
    /// quiesced session on every poll.
    pub fn upsert(&mut self, entry: QueueEntry, max_entries: usize) -> Result<()> {
        if self
            .entries
            .iter()
            .any(|e| e.session_hash == entry.session_hash)
        {
            return Ok(());
        }
        let live = self
            .entries
            .iter()
            .filter(|e| matches!(e.state, QueueState::Pending | QueueState::Approved))
            .count();
        if live >= max_entries {
            bail!("queue-full");
        }
        self.entries.push(entry);
        Ok(())
    }

    pub fn set_state(&mut self, entry_id: Uuid, state: QueueState, reason_label: Option<String>) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) {
            e.state = state;
            e.reason_label = reason_label;
        }
    }

    /// Move an entry from `Pending` to `Approved`, recording the terms the
    /// approval was given under: the consent scopes, and a fingerprint of
    /// everything else outside the session file that determines the
    /// envelope. Returns whether anything changed.
    ///
    /// This is the only way an entry becomes `Approved`, so an approved
    /// entry always carries the terms of its own approval -- see
    /// `QueueEntry::approved_scopes` and `QueueEntry::approved_inputs`.
    ///
    /// Any envelope digest already recorded from a preview of this entry is
    /// left in place: it is what the contributor was shown, and the
    /// approval is an approval of exactly that.
    /// `inputs` is `None` when the fingerprint could not be derived at all
    /// (no readable config). Every call site expresses "unknown" the same
    /// way -- `None`, never `Some("")` -- so the uploader's fail-closed
    /// check has exactly one shape to recognize.
    pub fn approve(&mut self, entry_id: Uuid, scopes: &[String], inputs: Option<&str>) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) else {
            return false;
        };
        if e.state != QueueState::Pending {
            return false;
        }
        e.state = QueueState::Approved;
        e.reason_label = None;
        e.approved_scopes = Some(scopes.to_vec());
        e.approved_inputs = inputs.map(str::to_string);
        true
    }

    /// Every entry that still needs its stored preview envelope kept on
    /// disk: live (not yet resolved) and still pinned to a preview.
    ///
    /// `daemon::approved_envelope::sweep` deletes everything else. An entry
    /// that reached a terminal state, or whose approval was revoked (which
    /// clears the pin), leaves no redacted trace content behind.
    pub fn pinned_entry_ids(&self) -> std::collections::HashSet<Uuid> {
        self.entries
            .iter()
            .filter(|e| {
                e.previewed_envelope_digest.is_some()
                    && matches!(
                        e.state,
                        QueueState::Pending | QueueState::Approved | QueueState::Uploading
                    )
            })
            .map(|e| e.entry_id)
            .collect()
    }

    /// Record the digest of the redacted envelope a preview just showed for
    /// this entry, so an approval that follows is pinned to that exact
    /// artifact. Returns whether the entry exists.
    ///
    /// Callers must have persisted the envelope itself first
    /// (`daemon::approved_envelope::save`): "digest recorded" is what the
    /// uploader reads as "the bytes are on disk", and recording one without
    /// the other turns an ordinary upload into a fail-closed re-offer.
    ///
    /// Only meaningful while the entry is still `Pending`: an entry already
    /// approved has had its terms fixed, and a later preview must not
    /// silently re-pin them to something else.
    pub fn record_previewed_envelope(&mut self, entry_id: Uuid, digest: &str) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) else {
            return false;
        };
        if e.state != QueueState::Pending {
            return false;
        }
        e.previewed_envelope_digest = Some(digest.to_string());
        true
    }

    /// Claim an approved entry for upload, atomically, under the caller's
    /// existing lock. Returns false when the entry is no longer `Approved`
    /// -- because a `cancel` landed after the caller snapshotted the
    /// approved set, which is precisely the race `cancel` exists to win.
    ///
    /// Nothing set `Uploading` in production before this: `drain_approved`
    /// snapshotted the approved set and left every entry `Approved` for the
    /// whole upload, so a mid-pass `cancel` returned `ok: true`, set
    /// `Pending`, and then watched the upload it had just "cancelled"
    /// proceed from the snapshot and overwrite `Pending` with `Uploaded`.
    /// The contributor was told an upload was cancelled after it had been
    /// sent.
    pub fn claim_for_upload(&mut self, entry_id: Uuid) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) else {
            return false;
        };
        if e.state != QueueState::Approved {
            return false;
        }
        e.state = QueueState::Uploading;
        true
    }

    /// Return every entry still claimed for upload to `Approved`.
    ///
    /// `Uploading` is a transient, in-pass state that only `claim_for_upload`
    /// sets and only a terminal outcome clears. An upload pass that breaks
    /// early -- a daily cap, a fail-closed precondition -- or a daemon that
    /// dies mid-pass would otherwise strand entries in a state nothing ever
    /// moves them out of: never uploaded, never offered again. Called at the
    /// end of every pass and again at `DaemonShared::load`, so a crash
    /// recovers on the next start. Returns whether anything changed.
    pub fn release_in_flight(&mut self) -> bool {
        let mut changed = false;
        for e in self.entries.iter_mut() {
            if e.state == QueueState::Uploading {
                e.state = QueueState::Approved;
                changed = true;
            }
        }
        changed
    }

    /// Revoke an approval and put the entry back in front of the
    /// contributor, because the terms it was approved under no longer hold.
    /// Returns whether anything changed.
    pub fn revoke_approval(&mut self, entry_id: Uuid, reason_label: &str) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) else {
            return false;
        };
        e.state = QueueState::Pending;
        e.reason_label = Some(reason_label.to_string());
        e.approved_scopes = None;
        e.approved_inputs = None;
        // The artifact the contributor was shown is no longer the one that
        // would be sent, so the re-offer must be previewed afresh.
        e.previewed_envelope_digest = None;
        true
    }

    /// Update an entry's display label in place, e.g. when a newly-seen
    /// project causes a previously-unique basename to start colliding.
    /// `Queue::upsert` never rewrites an existing entry, so this is the only
    /// way an already-queued entry's label can change.
    pub fn set_project_label(&mut self, entry_id: Uuid, label: String) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) {
            e.project_label = label;
        }
    }

    pub fn set_submission_id(&mut self, entry_id: Uuid, submission_id: Uuid) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) {
            e.submission_id = Some(submission_id);
        }
    }

    pub fn record_attempt(&mut self, entry_id: Uuid, retry_after: Option<DateTime<Utc>>) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) {
            e.attempts = e.attempts.saturating_add(1);
            e.retry_after = retry_after;
        }
    }

    /// Mark an entry superseded and produce a fresh pending entry describing
    /// the session as it is now.
    ///
    /// This is what happens when a session grows between being offered and
    /// being approved. The contributor approved a description; if the content
    /// no longer matches it, the approval does not carry over to the new
    /// content, so a new offer is made instead.
    pub fn supersede(
        &mut self,
        entry_id: Uuid,
        new_hash: &str,
        new_size: u64,
        now: DateTime<Utc>,
    ) -> Option<QueueEntry> {
        let old = self
            .entries
            .iter()
            .find(|e| e.entry_id == entry_id)?
            .clone();
        self.set_state(
            entry_id,
            QueueState::Superseded,
            Some("session-changed-after-offer".to_string()),
        );
        Some(QueueEntry {
            entry_id: entry_id_for(new_hash),
            session_hash: new_hash.to_string(),
            size_bytes: new_size,
            discovered_at: now,
            state: QueueState::Pending,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
            // Provenance carries over; the approval and every term it was
            // given under -- scopes, envelope-determining inputs, and the
            // artifact that was shown -- do not.
            approved_scopes: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            ..old
        })
    }

    /// Return an approved entry to pending, backing a short "undo" window on
    /// an approval. Refuses once the entry has moved past `Approved` --
    /// notably `Uploading`, where an upload may already be in flight and an
    /// undo racing it would be indistinguishable from data loss.
    pub fn cancel(&mut self, entry_id: Uuid) -> Result<()> {
        let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) else {
            bail!("unknown-entry-id");
        };
        if e.state != QueueState::Approved {
            bail!("not-approved");
        }
        e.state = QueueState::Pending;
        e.reason_label = None;
        e.approved_scopes = None;
        e.approved_inputs = None;
        Ok(())
    }

    /// Age out undecided entries. Returns how many expired.
    ///
    /// `blocked_on_health` suspends the clock entirely: an entry the daemon
    /// could not have uploaded even with permission has not been declined.
    pub fn expire(&mut self, now: DateTime<Utc>, ttl_days: i64, blocked_on_health: bool) -> usize {
        if blocked_on_health {
            return 0;
        }
        let cutoff = now - Duration::days(ttl_days);
        let mut expired = 0;
        for e in self.entries.iter_mut() {
            if e.state == QueueState::Pending && e.discovered_at < cutoff {
                e.state = QueueState::Expired;
                e.reason_label = Some("expired-without-decision".to_string());
                expired += 1;
            }
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn entry(hash: &str, discovered: &str) -> QueueEntry {
        QueueEntry {
            entry_id: entry_id_for(hash),
            session_hash: hash.into(),
            source: "claude-code".into(),
            project_key: "/Users/z/code/proj".into(),
            project_label: "proj".into(),
            path: PathBuf::from("/Users/z/.claude/projects/x/s.jsonl"),
            size_bytes: 100,
            discovered_at: at(discovered),
            state: QueueState::Pending,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
            approved_scopes: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
        }
    }

    #[test]
    fn entry_id_is_stable_for_a_session_hash() {
        assert_eq!(entry_id_for("sha256:aa"), entry_id_for("sha256:aa"));
        assert_ne!(entry_id_for("sha256:aa"), entry_id_for("sha256:bb"));
    }

    #[test]
    fn upsert_is_idempotent_on_session_hash() {
        // The watcher re-observes the same quiesced session every poll.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        q.upsert(entry("sha256:aa", "2026-08-08T13:00:00Z"), 500)
            .unwrap();
        assert_eq!(q.pending().len(), 1);
    }

    #[test]
    fn upsert_refuses_past_the_queue_cap() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 1)
            .unwrap();
        let err = q
            .upsert(entry("sha256:bb", "2026-08-08T12:00:00Z"), 1)
            .unwrap_err();
        assert!(err.to_string().contains("queue-full"));
    }

    #[test]
    fn the_cap_counts_only_live_entries() {
        // Resolved entries are history, not backlog.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 1)
            .unwrap();
        q.set_state(entry_id_for("sha256:aa"), QueueState::Uploaded, None);
        q.upsert(entry("sha256:bb", "2026-08-08T12:00:00Z"), 1)
            .unwrap();
        assert_eq!(q.pending().len(), 1);
    }

    #[test]
    fn pending_entries_expire_after_the_ttl() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-07-01T12:00:00Z"), 500)
            .unwrap();
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, false), 1);
        assert_eq!(
            q.get(entry_id_for("sha256:aa")).unwrap().state,
            QueueState::Expired
        );
    }

    #[test]
    fn expiry_is_suspended_while_blocked_on_health() {
        // A privacy-filter outage must not silently discard two weeks of
        // traces the contributor never declined.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-07-01T12:00:00Z"), 500)
            .unwrap();
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, true), 0);
        assert_eq!(
            q.get(entry_id_for("sha256:aa")).unwrap().state,
            QueueState::Pending
        );
    }

    #[test]
    fn entries_inside_the_ttl_do_not_expire() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-01T12:00:00Z"), 500)
            .unwrap();
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, false), 0);
    }

    #[test]
    fn resolved_entries_are_never_expired() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-07-01T12:00:00Z"), 500)
            .unwrap();
        q.set_state(entry_id_for("sha256:aa"), QueueState::Uploaded, None);
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, false), 0);
    }

    #[test]
    fn supersede_marks_the_old_entry_and_returns_a_fresh_pending_one() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let fresh = q
            .supersede(
                entry_id_for("sha256:aa"),
                "sha256:bb",
                900,
                at("2026-08-08T16:00:00Z"),
            )
            .unwrap();
        assert_eq!(
            q.get(entry_id_for("sha256:aa")).unwrap().state,
            QueueState::Superseded
        );
        assert_eq!(fresh.session_hash, "sha256:bb");
        assert_eq!(fresh.size_bytes, 900);
        assert_eq!(fresh.state, QueueState::Pending);
        assert_eq!(fresh.entry_id, entry_id_for("sha256:bb"));
        // Provenance carries over; approval does not.
        assert_eq!(fresh.project_key, "/Users/z/code/proj");
        assert_eq!(fresh.attempts, 0);
        assert!(fresh.submission_id.is_none());
    }

    #[test]
    fn supersede_of_an_unknown_entry_is_a_no_op() {
        let mut q = Queue::new();
        assert!(
            q.supersede(
                entry_id_for("sha256:missing"),
                "sha256:bb",
                900,
                at("2026-08-08T16:00:00Z")
            )
            .is_none()
        );
    }

    #[test]
    fn attempts_accumulate_across_retries() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        q.record_attempt(id, Some(at("2026-08-08T12:05:00Z")));
        q.record_attempt(id, Some(at("2026-08-08T12:15:00Z")));
        assert_eq!(q.get(id).unwrap().attempts, 2);
        assert_eq!(
            q.get(id).unwrap().retry_after,
            Some(at("2026-08-08T12:15:00Z"))
        );
    }

    #[test]
    fn queue_round_trips_through_the_store() {
        let (_d, store) = temp_store();
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        q.save(&store).unwrap();
        assert_eq!(Queue::load(&store).unwrap(), q);
    }

    #[test]
    fn a_corrupt_queue_line_is_skipped_rather_than_losing_the_file() {
        let (_d, store) = temp_store();
        let good = serde_json::to_string(&entry("sha256:aa", "2026-08-08T12:00:00Z")).unwrap();
        store
            .write_daemon_file(DAEMON_QUEUE_FILE, format!("{good}\nnot json\n").as_bytes())
            .unwrap();
        assert_eq!(Queue::load(&store).unwrap().pending().len(), 1);
    }

    #[test]
    fn queue_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(Queue::load(&store).unwrap(), Queue::new());
    }

    #[test]
    fn cancel_returns_an_approved_entry_to_pending() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        q.set_state(id, QueueState::Approved, None);
        q.cancel(id).unwrap();
        assert_eq!(q.get(id).unwrap().state, QueueState::Pending);
    }

    #[test]
    fn cancel_refuses_an_entry_that_is_not_approved() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        q.set_state(id, QueueState::Uploading, None);
        let err = q.cancel(id).unwrap_err();
        assert!(err.to_string().contains("not-approved"));
        assert_eq!(q.get(id).unwrap().state, QueueState::Uploading);
    }

    #[test]
    fn cancel_refuses_an_unknown_entry_id() {
        let mut q = Queue::new();
        let err = q.cancel(entry_id_for("sha256:missing")).unwrap_err();
        assert!(err.to_string().contains("unknown-entry-id"));
    }
}
