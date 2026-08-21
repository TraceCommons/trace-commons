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

use std::path::{Path, PathBuf};

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
    /// When the contributor approved this entry, and therefore when its
    /// post-approval hold started.
    ///
    /// The design offers a five-second undo after an approval -- "Sending…
    /// [Undo]" -- on the reasoning that a misclick should be a non-event
    /// rather than a permanent one, because what is being sent is the
    /// contributor's own work. That undo did not exist: `approve` set
    /// `Approved` and the very next `drain_approved` pass uploaded the
    /// entry, so on a machine with a working network the upload could
    /// complete inside the window a client was still counting down. The
    /// client offered a choice the daemon had already taken away.
    ///
    /// `drain_approved` now skips any entry whose approval is younger than
    /// `DaemonSettings::approval_hold_secs`, so the window is a property of
    /// the entry rather than a race the client hopes to win, and `cancel`
    /// is guaranteed to succeed for its whole duration (nothing can have
    /// claimed the entry). See `QueueEntry::hold_until`.
    ///
    /// `None` means "no hold applies", and there are exactly two ways to
    /// get it: an entry written before this field existed, and an entry
    /// auto-approved by a project's standing `auto_upload` opt-in. The
    /// latter is deliberate -- see `Queue::approve`.
    #[serde(default)]
    pub approved_at: Option<DateTime<Utc>>,
    /// How many delegated subagent transcripts this entry's session hash
    /// covers, and how many were left out because the conversation exceeded
    /// the source's raw byte budget.
    ///
    /// A card standing for 114 transcripts must be able to say so: what is
    /// being consented to is the whole conversation, and its extent is part
    /// of the description. `subagents_dropped` is the honest half of that --
    /// a deliberately trimmed conversation says it was trimmed rather than
    /// presenting as complete.
    ///
    /// Zero on every entry written before these fields existed, and on every
    /// source with no such structure (codex, trajectory). Both are
    /// `#[serde(default)]` because `Queue::load` drops a line it cannot
    /// parse: a non-defaulted addition here would silently empty a
    /// contributor's queue on upgrade.
    #[serde(default)]
    pub subagent_count: u32,
    #[serde(default)]
    pub subagents_dropped: u32,
    /// The `modified_at` of the observation this entry was built from --
    /// the group mtime for a claude-code session, the file's own mtime for
    /// every single-file source. Pairs with `size_bytes`, which is the
    /// observed size, to record the *whole* observation the watcher hashed.
    ///
    /// The watcher polls every discovered session on every pass, and until
    /// this field existed it had no cheap way to tell "already offered,
    /// nothing has moved" from "offered, then grew". So it read, parsed and
    /// hashed every queued session again on every poll -- on a real corpus,
    /// 11 GB of transcripts re-hashed every sixty seconds -- and threw the
    /// result away at `replace_live_at_path`, which then found the entry
    /// already tracked. `Queue::unchanged_offer_at_path` answers that
    /// question from this pair instead.
    ///
    /// It lives on the entry rather than in `DaemonState` deliberately: the
    /// question being asked is "what observation is this queue entry made
    /// of", so the answer belongs to the entry and dies with it. A parallel
    /// map in the state file would be a second source of truth that a
    /// dropped queue line, a wipe, or a supersede could leave disagreeing
    /// with the queue -- and disagreeing in the unsafe direction, claiming
    /// an offer exists for content nothing was offered for.
    ///
    /// `None` on every entry written before this field existed and on the
    /// re-offer `Queue::supersede` mints, whose content the watcher never
    /// observed. `None` never matches, so those entries take the load path
    /// exactly as they did before: the fast path fails open.
    #[serde(default)]
    pub observed_modified_at: Option<DateTime<Utc>>,
}

impl QueueEntry {
    /// The instant this entry's post-approval hold ends, i.e. the deadline a
    /// client counts down to and the instant `drain_approved` becomes
    /// willing to upload it.
    ///
    /// `None` means nothing is holding this entry: it carries no
    /// `approved_at` (never approved, approved by a standing opt-in, or
    /// written before the field existed) or the hold is configured off.
    /// Reported to clients on the `approve` response so a UI counts against
    /// the daemon's clock rather than its own -- a UI counting its own five
    /// seconds while the daemon holds for some other interval is the same
    /// class of bug the hold exists to fix.
    pub fn hold_until(&self, hold_secs: u64) -> Option<DateTime<Utc>> {
        if hold_secs == 0 {
            return None;
        }
        self.approved_at
            .map(|at| at + Duration::seconds(hold_secs as i64))
    }

    /// Whether the post-approval hold is still running at `now`.
    ///
    /// The comparison is `now < deadline`, so an entry is released exactly
    /// at its reported deadline and not a tick later: a client that waits
    /// out the deadline it was given has waited out precisely the hold.
    pub fn hold_active(&self, now: DateTime<Utc>, hold_secs: u64) -> bool {
        self.hold_until(hold_secs).is_some_and(|until| now < until)
    }
}

/// A stable id for a queue entry, derived from the session hash so the same
/// session keeps the same id across daemon restarts and across a queue file
/// rewritten from scratch.
pub fn entry_id_for(session_hash: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, session_hash.as_bytes())
}

/// The reason label both supersede paths record: the session no longer
/// matches the description it was offered under.
pub const REASON_CHANGED: &str = "session-changed-after-offer";

/// The reason label `dismiss` records: the contributor looked at this
/// session and said no.
///
/// Load-bearing, not decoration. `Refused` has two authors -- the
/// contributor, via this label, and the pipeline, which records its own
/// labels when it will not send some bytes (a residual secret, an
/// unavailable privacy filter). Only the first is a decision about the
/// *conversation*, and `Queue::dismissed_at_path` distinguishes them by
/// this exact string. Changing it in one place and not the other would
/// silently start re-offering declined sessions again.
pub const REASON_DISMISSED: &str = "dismissed-by-contributor";

/// The reason label an approve records when the envelope it built is past
/// `envelope::MAX_ENVELOPE_BYTES`.
///
/// The exact string the `approve` response already reports in `skipped`,
/// and the one all three shells translate into "too large to send". It is
/// reused verbatim for the persisted refusal rather than given a second
/// name, because the toast and the entry describe one fact to one person:
/// a contributor told their trace was too large, who then goes looking for
/// it, must not find it filed under a different word.
///
/// The CLI's own `session-too-large` is deliberately left alone. That is a
/// one-shot line printed by `submit`, carrying the measured size and the
/// limit with it, and it is never written to the queue -- a different
/// surface with a different audience, and renaming it would churn a
/// documented CLI contract for no gain to the daemon.
///
/// Unlike `REASON_DISMISSED` this label suppresses nothing at the path
/// level. See `dismissed_at_path` for why: this is a verdict on one set of
/// bytes under one set of consent scopes, not a decision about the
/// conversation.
pub const REASON_TOO_LARGE: &str = "envelope-too-large";

/// Strip an entry back to a fresh offer, keeping only provenance.
///
/// Factored out so `supersede` and any future re-offer path cannot drift:
/// a term of approval added to `QueueEntry` and cleared in one of them but
/// not the other would carry a stale approval onto content it never
/// covered, which is precisely the failure this whole mechanism exists to
/// prevent.
fn reoffered_from(old: QueueEntry) -> QueueEntry {
    QueueEntry {
        state: QueueState::Pending,
        reason_label: None,
        attempts: 0,
        retry_after: None,
        submission_id: None,
        // Provenance carries over; the approval and every term it was given
        // under -- scopes, envelope-determining inputs, and the artifact
        // that was shown -- do not.
        approved_scopes: None,
        approved_inputs: None,
        approved_at: None,
        previewed_envelope_digest: None,
        // The caller found content the watcher never observed (the
        // uploader's re-hash guard is the only path here), so this entry is
        // not made of any observation the poll loop can match against.
        // `None` sends the next poll down the load path, which is the
        // fail-open direction.
        observed_modified_at: None,
        ..old
    }
}

/// What `Queue::replace_live_at_path` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceOutcome {
    /// How many stale live offers at the same path were retired.
    pub superseded: usize,
    /// Whether the entry was added, as opposed to already being tracked
    /// under this session hash. The caller reports a genuinely new offer
    /// differently from re-observing one it already made.
    pub inserted: bool,
}

/// How many `Superseded` entries the queue file keeps.
///
/// Every other resolved state records something that happened to a trace: it
/// was uploaded (and carries the receipt's `submission_id`), refused,
/// failed, or aged out without a decision. `Superseded` records only that an
/// offer was replaced by a newer offer that is itself in the file, so it is
/// the one resolved state produced mechanically rather than by an outcome --
/// and the only one whose volume scales with how much an agent delegates
/// rather than with how much a contributor contributes. With grouping, one
/// long delegating conversation mints a fresh hash per delegation, so
/// unbounded retention means a permanently growing `daemon-queue.jsonl` that
/// the daemon re-parses at every start.
///
/// Keeping the most recent handful preserves what the state is actually good
/// for -- explaining why a card the contributor remembers is no longer
/// there -- while bounding the file. Nothing else is ever compacted: a
/// receipt is not bookkeeping.
const MAX_SUPERSEDED_ENTRIES: usize = 50;

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

    /// Has the contributor declined the session living at `path`?
    ///
    /// "Not this one" is a decision about the conversation, not about the
    /// byte range it happened to have when the card was drawn. `dismiss`
    /// records it on one entry, and an entry is identified by content hash
    /// -- so the *hash* was declined, and the moment the contributor typed
    /// the next message the session hashed to something else, the watcher
    /// had nothing telling it a decision had been made, and it offered the
    /// same conversation again. On a session still being worked in, that
    /// is a card that comes back every poll for the rest of the day. In an
    /// armed project it was worse than an annoyance: the re-offer landed
    /// `Approved`, and the declined conversation uploaded unattended.
    ///
    /// The path is the daemon's one stable address for a conversation --
    /// `replace_live_at_path`, `unchanged_offer_at_path` and `load_can_land`
    /// all key on it, and for claude-code it deliberately stays the parent
    /// file even as delegated transcripts come and go beside it. So the
    /// decision is answered from the path too, and it is answered from the
    /// queue rather than from a parallel map in `DaemonState`: the queue
    /// file is where the dismissal is already durably recorded, which means
    /// dismissals made before this existed are honoured with no migration,
    /// and there is no second source of truth to drift.
    ///
    /// Only `REASON_DISMISSED` counts. `Refused` has a second author -- the
    /// pipeline, refusing to send some bytes over a residual secret or an
    /// unavailable privacy filter -- and that is a verdict on content, not
    /// on the conversation, so those sessions must still be re-offered when
    /// they grow.
    ///
    /// Permanent, by design. There is no un-dismiss, and nothing compacts
    /// `Refused` (only `Superseded` is ever dropped, see
    /// `MAX_SUPERSEDED_ENTRIES`), so the answer does not decay. The
    /// contributor who wants a declined conversation after all still has
    /// every other route to it; the daemon simply stops asking. That is the
    /// fail-closed direction: the cost of honouring a "no" too well is a
    /// trace that never uploads, and the cost of honouring it too poorly is
    /// uploading something a contributor explicitly declined.
    pub fn dismissed_at_path(&self, path: &Path) -> bool {
        self.entries.iter().any(|e| {
            e.path == path
                && e.state == QueueState::Refused
                && e.reason_label.as_deref() == Some(REASON_DISMISSED)
        })
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
    ///
    /// `approved_at` starts the post-approval hold: `Some(now)` for an
    /// approval a contributor just made, which is the one that needs an
    /// undo window, and `None` for a project's standing `auto_upload`
    /// opt-in. The opt-in case is `None` on purpose: it is a decision taken
    /// in advance and separately audited, no client is showing a countdown
    /// for it, and there is no click to take back -- holding it would only
    /// delay every unattended upload by a poll interval for no consent
    /// benefit. See `QueueEntry::approved_at`.
    pub fn approve(
        &mut self,
        entry_id: Uuid,
        scopes: &[String],
        inputs: Option<&str>,
        approved_at: Option<DateTime<Utc>>,
    ) -> bool {
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
        e.approved_at = approved_at;
        true
    }

    /// Every entry that still needs its stored preview envelope kept on
    /// disk: live (not yet resolved) and still pinned to a preview.
    ///
    /// `daemon::approved_envelope::sweep` deletes everything else. An entry
    /// that reached a terminal state, or whose approval was revoked or
    /// undone (both of which clear the pin), leaves no redacted trace
    /// content behind.
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
        e.approved_at = None;
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
            Some(REASON_CHANGED.into()),
        );
        Some(QueueEntry {
            entry_id: entry_id_for(new_hash),
            session_hash: new_hash.to_string(),
            size_bytes: new_size,
            discovered_at: now,
            ..reoffered_from(old)
        })
    }

    /// The queue entry a fresh observation of `path` would land on, when
    /// nothing about that observation has moved since the entry was built.
    ///
    /// This is the poll loop's cheap pre-check. `eligibility::evaluate`
    /// cannot answer it: it sees the observation, the previous poll's size
    /// and any *prior upload*, and a session sitting here `Pending` has
    /// never uploaded, so it has no prior and comes back `Eligible` on
    /// every single poll. The watcher then paid `source.load` -- read,
    /// parse, hash the whole group -- only for `replace_live_at_path` to
    /// find the entry already tracked and discard the work.
    ///
    /// Answering `Some` means, and must only mean, that a load would find
    /// the same `session_hash` this queue already holds, so the load can be
    /// skipped without changing what the queue ends up containing. Two
    /// rules keep that true:
    ///
    /// - The match is on the **whole** observation -- `size_bytes` and
    ///   `observed_modified_at` together -- and for claude-code both are
    ///   group-wide (`SessionRef::size_bytes`, `SessionRef::group_modified_at`).
    ///   Matching on the parent file's own stat would miss a delegated
    ///   transcript still being written beside it, which is precisely the
    ///   bug the group fields were added to fix.
    /// - A live offer (`Pending`/`Approved`) at this path built from a
    ///   *different* observation forces `None`, because that is the offer
    ///   `replace_live_at_path` exists to retire: the supersede path must
    ///   still fire, and it can only fire from a real load.
    ///
    /// An entry with no recorded observation (`observed_modified_at ==
    /// None`: written before the field existed, or minted by `supersede`)
    /// never matches, so it takes the load path exactly as before.
    pub fn unchanged_offer_at_path(
        &self,
        path: &Path,
        size_bytes: u64,
        modified_at: DateTime<Utc>,
    ) -> Option<&QueueEntry> {
        let same_observation = |e: &QueueEntry| {
            e.size_bytes == size_bytes && e.observed_modified_at == Some(modified_at)
        };
        let at_path: Vec<&QueueEntry> = self.entries.iter().filter(|e| e.path == path).collect();
        let live = |e: &QueueEntry| matches!(e.state, QueueState::Pending | QueueState::Approved);
        if at_path.iter().any(|e| live(e) && !same_observation(e)) {
            return None;
        }
        at_path
            .iter()
            .find(|e| same_observation(e) && live(e))
            .or_else(|| at_path.iter().find(|e| same_observation(e)))
            .copied()
    }

    /// Can a load at `path` still produce something this queue would keep?
    ///
    /// The companion to `unchanged_offer_at_path`, for the other -- and much
    /// larger -- population of sessions the poll pays for and throws away:
    /// the ones that are eligible and are not in the queue at all, because
    /// the queue is at `max_entries`. `unchanged_offer_at_path` answers
    /// `None` for every one of them (there is no offer here to be
    /// unchanged), so the pass went on to `source.load` -- read, parse and
    /// hash the whole group -- only for `replace_live_at_path` to refuse it
    /// `queue-full`. With a corpus larger than the cap, which is the normal
    /// state for a real user, that is thousands of full group hashes every
    /// poll, forever.
    ///
    /// Answering `false` means, and must only mean, that
    /// `replace_live_at_path` would refuse whatever the load produced, so
    /// skipping the load cannot change what the queue ends up containing.
    /// Two rules keep that true, and both halves are load-bearing:
    ///
    /// - The occupancy counted is the same one `replace_live_at_path`
    ///   counts against the cap: live entries, `Pending` or `Approved`.
    ///   Below the cap there is room for a new offer, so the load must
    ///   happen.
    /// - A live entry at this same path forces `true` even at capacity,
    ///   because `replace_live_at_path` *supersedes* it, and the cap is
    ///   counted against the entries that would survive -- so the
    ///   replacement lands in the slot the stale card vacates. Dropping
    ///   this half would mean a grown session (a new delegated transcript
    ///   included) could never supersede once the queue filled up, and the
    ///   contributor's card would describe content that has moved on,
    ///   permanently.
    ///
    /// Not covered, deliberately: an entry with this exact `session_hash`
    /// living at some *other* path makes `replace_live_at_path` return `Ok`
    /// without any capacity check. Reaching that needs the hash, which is
    /// the load this exists to avoid, and the only thing that path does is
    /// re-apply a standing opt-in -- which the next poll after the queue
    /// drains below the cap will do anyway.
    pub fn load_can_land(&self, path: &Path, max_entries: usize) -> bool {
        let live = |e: &&QueueEntry| matches!(e.state, QueueState::Pending | QueueState::Approved);
        if self.entries.iter().filter(live).count() < max_entries {
            return true;
        }
        self.entries.iter().filter(live).any(|e| e.path == path)
    }

    /// Add `entry` and, in the same step, retire every live entry at the same
    /// path whose hash is no longer `entry.session_hash`.
    ///
    /// `upsert` dedups on `session_hash` alone, so a session that grew
    /// between being offered and being decided already produced a *second*
    /// `Pending` card while the first sat there: two cards, one file. That
    /// was rare when a session was one file. With subagent grouping,
    /// membership changes every time the agent delegates, so it would be the
    /// normal case -- one conversation accumulating a card per delegation.
    ///
    /// Retiring the stale offer is the consent invariant applied to
    /// membership rather than to content: the contributor approved a
    /// description, the description moved, so the old offer dies and a fresh
    /// preview is made. `Uploading` is deliberately not touched -- an upload
    /// may already be in flight, and the uploader's own re-hash guard is
    /// what covers that race.
    ///
    /// The retirement and the insert are one operation because doing them in
    /// sequence left a window with no offer at all. Retiring first and then
    /// hitting the queue cap meant the replacement never landed, so a
    /// conversation whose only live card had just been superseded had none
    /// until capacity freed up -- and nothing would re-retire it, since the
    /// stale card was already gone. This refuses before mutating anything,
    /// so a `queue-full` leaves the previous offer exactly where it was: a
    /// full queue delays the new offer rather than destroying the old one.
    ///
    /// The cap is counted against the entries that would *survive* this call,
    /// because the stale cards this is about to retire are being replaced,
    /// not added to. Counting them as occupants would let a busy conversation
    /// fill the queue with its own superseded predecessors.
    pub fn replace_live_at_path(
        &mut self,
        entry: QueueEntry,
        max_entries: usize,
    ) -> Result<ReplaceOutcome> {
        let stale: Vec<Uuid> = self
            .entries
            .iter()
            .filter(|e| {
                e.path == entry.path
                    && e.session_hash != entry.session_hash
                    && matches!(e.state, QueueState::Pending | QueueState::Approved)
            })
            .map(|e| e.entry_id)
            .collect();
        let already_tracked = self
            .entries
            .iter()
            .any(|e| e.session_hash == entry.session_hash);
        if !already_tracked {
            let live = self
                .entries
                .iter()
                .filter(|e| {
                    matches!(e.state, QueueState::Pending | QueueState::Approved)
                        && !stale.contains(&e.entry_id)
                })
                .count();
            if live >= max_entries {
                bail!("queue-full");
            }
        }
        for entry_id in &stale {
            self.set_state(
                *entry_id,
                QueueState::Superseded,
                Some(REASON_CHANGED.to_string()),
            );
        }
        if !already_tracked {
            self.entries.push(entry);
        }
        Ok(ReplaceOutcome {
            superseded: stale.len(),
            inserted: !already_tracked,
        })
    }

    /// Return an approved entry to pending, backing the "undo" window on an
    /// approval. Clears everything the approval established, the pin
    /// included, so the next approval is a fresh decision about freshly
    /// built bytes. Refuses once the entry has moved past `Approved` --
    /// notably `Uploading`, where an upload may already be in flight and an
    /// undo racing it would be indistinguishable from data loss.
    ///
    /// Throughout the post-approval hold this cannot refuse: nothing claims
    /// a held entry, so it is still `Approved` by construction. That is the
    /// whole point of the hold -- before it existed, an undo offered for
    /// five seconds could find the upload already sent, and `cancel`
    /// answered `not-cancelable` for a decision the contributor had been
    /// told was still theirs to make.
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
        e.approved_at = None;
        // The pin goes with the approval, exactly as in `revoke_approval`.
        // A pin is a binding between an approval and the precise bytes it
        // covered; an undo withdraws the approval, so the binding cannot
        // survive it.
        //
        // Keeping it was load-bearing in the wrong direction once `approve`
        // started building envelopes for entries nobody previewed. The
        // sequence is: one-click Submit builds and pins an artifact the
        // contributor never saw, the toast offers Undo, Undo returns the
        // entry to `Pending` -- and a second Submit would then find it
        // already pinned, skip the rebuild, and approve the artifact built
        // at the FIRST click. If the session grew in between, that sends
        // stale bytes; and because a pre-pinned entry contributes no counts,
        // the second click reports `redactions: {}` / `flagged: 0`, so the
        // contributor is shown nothing either time.
        //
        // Clearing it makes the next approval rebuild from the session as
        // it now stands. The stored envelope is dropped from
        // `pinned_entry_ids` by the same rule that drops a revoked one, so
        // the next sweep deletes redacted trace content nothing refers to
        // any more -- which is the right outcome for withdrawn consent, and
        // is not premature: no path reads a `Pending` entry's stored
        // envelope.
        e.previewed_envelope_digest = None;
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

    /// Drop all but the `MAX_SUPERSEDED_ENTRIES` most recent `Superseded`
    /// entries, returning how many were removed.
    ///
    /// Nothing else is touched. In particular an `Uploaded` entry is never
    /// removed: it carries the `submission_id` that ties a local decision to
    /// a server receipt, and the history view joins on it.
    ///
    /// Recency is `discovered_at`, which for a superseded entry is when its
    /// offer was made, with insertion order breaking ties -- `sort_by_key` is
    /// stable and entries are appended, so two offers made inside the same
    /// clock tick retire oldest-first rather than arbitrarily. See
    /// `MAX_SUPERSEDED_ENTRIES` for why this state alone is compacted.
    pub fn compact_superseded(&mut self) -> usize {
        let superseded: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.state == QueueState::Superseded)
            .map(|(i, _)| i)
            .collect();
        if superseded.len() <= MAX_SUPERSEDED_ENTRIES {
            return 0;
        }
        let mut by_age = superseded.clone();
        by_age.sort_by_key(|&i| self.entries[i].discovered_at);
        let doomed: std::collections::HashSet<usize> = by_age
            .into_iter()
            .take(superseded.len() - MAX_SUPERSEDED_ENTRIES)
            .collect();
        let mut index = 0usize;
        self.entries.retain(|_| {
            let keep = !doomed.contains(&index);
            index += 1;
            keep
        });
        doomed.len()
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
            approved_at: None,
            subagent_count: 0,
            subagents_dropped: 0,
            observed_modified_at: None,
        }
    }

    /// `entry`, plus the observation it is supposed to be made of.
    fn observed_entry(hash: &str, size_bytes: u64, observed: &str) -> QueueEntry {
        QueueEntry {
            size_bytes,
            observed_modified_at: Some(at(observed)),
            ..entry(hash, "2026-08-08T12:00:00Z")
        }
    }

    fn queue_of(entries: Vec<QueueEntry>) -> Queue {
        let mut q = Queue::new();
        for e in entries {
            q.upsert(e, 5000).unwrap();
        }
        q
    }

    fn the_path() -> PathBuf {
        PathBuf::from("/Users/z/.claude/projects/x/s.jsonl")
    }

    #[test]
    fn an_unchanged_observation_finds_the_offer_it_produced() {
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        let hit = q
            .unchanged_offer_at_path(&the_path(), 100, at("2026-08-08T11:00:00Z"))
            .expect("the same observation must match");
        assert_eq!(hit.session_hash, "sha256:aa");
        assert_eq!(hit.state, QueueState::Pending);
    }

    #[test]
    fn a_grown_group_does_not_match_and_must_be_reloaded() {
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        assert!(
            q.unchanged_offer_at_path(&the_path(), 140, at("2026-08-08T11:00:00Z"))
                .is_none(),
            "a larger group is a different description and must supersede"
        );
    }

    #[test]
    fn a_rewritten_member_at_the_same_total_size_still_does_not_match() {
        // Size alone is not the observation. A delegated transcript
        // rewritten to the same length moves the group mtime and nothing
        // else, and the content it hashes to is different.
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        assert!(
            q.unchanged_offer_at_path(&the_path(), 100, at("2026-08-08T11:05:00Z"))
                .is_none()
        );
    }

    #[test]
    fn an_entry_with_no_recorded_observation_never_matches() {
        // Entries written before the field existed, and the re-offer
        // `supersede` mints. They take the load path exactly as before:
        // the pre-check fails open.
        let q = queue_of(vec![entry("sha256:aa", "2026-08-08T12:00:00Z")]);
        assert_eq!(q.all()[0].observed_modified_at, None);
        assert!(
            q.unchanged_offer_at_path(&the_path(), 100, at("2026-08-08T11:00:00Z"))
                .is_none()
        );
    }

    #[test]
    fn a_dismissal_is_remembered_against_the_session_not_the_hash() {
        let mut q = queue_of(vec![entry("sha256:aa", "2026-08-08T12:00:00Z")]);
        assert!(!q.dismissed_at_path(&the_path()));
        q.set_state(
            entry_id_for("sha256:aa"),
            QueueState::Refused,
            Some(REASON_DISMISSED.into()),
        );
        assert!(
            q.dismissed_at_path(&the_path()),
            "the decision is about the conversation at this path"
        );
    }

    #[test]
    fn a_pipeline_refusal_is_not_a_dismissal() {
        // `Refused` also carries the daemon's own verdicts on content -- a
        // residual secret, an unavailable privacy filter. Those must not
        // silence the offer for the whole session.
        let mut q = queue_of(vec![entry("sha256:aa", "2026-08-08T12:00:00Z")]);
        q.set_state(
            entry_id_for("sha256:aa"),
            QueueState::Refused,
            Some("residual-secret".into()),
        );
        assert!(!q.dismissed_at_path(&the_path()));
    }

    #[test]
    fn a_dismissal_says_nothing_about_another_session() {
        let mut q = queue_of(vec![entry("sha256:aa", "2026-08-08T12:00:00Z")]);
        q.set_state(
            entry_id_for("sha256:aa"),
            QueueState::Refused,
            Some(REASON_DISMISSED.into()),
        );
        assert!(!q.dismissed_at_path(Path::new("/Users/z/.claude/projects/x/other.jsonl")));
    }

    #[test]
    fn a_live_offer_built_from_a_different_observation_forces_a_reload() {
        // A resolved entry happens to match the current observation while a
        // live one at the same path does not. The live one is exactly what
        // `replace_live_at_path` exists to retire, so the load must happen.
        let mut q = queue_of(vec![
            observed_entry("sha256:old", 100, "2026-08-08T11:00:00Z"),
            observed_entry("sha256:new", 140, "2026-08-08T11:30:00Z"),
        ]);
        q.set_state(entry_id_for("sha256:old"), QueueState::Refused, None);
        assert!(
            q.unchanged_offer_at_path(&the_path(), 100, at("2026-08-08T11:00:00Z"))
                .is_none()
        );
    }

    #[test]
    fn a_resolved_entry_matching_the_observation_still_suppresses_the_load() {
        // An expired or refused offer with no live sibling: a load would
        // re-derive a hash the queue already tracks and insert nothing, so
        // it is pure waste. This is the case that made expired entries churn
        // forever.
        let mut q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        q.set_state(entry_id_for("sha256:aa"), QueueState::Expired, None);
        let hit = q
            .unchanged_offer_at_path(&the_path(), 100, at("2026-08-08T11:00:00Z"))
            .expect("nothing live here, and a load would change nothing");
        assert_eq!(hit.state, QueueState::Expired);
    }

    #[test]
    fn a_matching_live_offer_is_preferred_over_a_matching_resolved_one() {
        // The caller re-applies a standing opt-in to what comes back, so it
        // must come back the entry that can still be approved.
        let mut q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        q.set_state(entry_id_for("sha256:aa"), QueueState::Superseded, None);
        q.upsert(
            QueueEntry {
                entry_id: entry_id_for("sha256:bb"),
                session_hash: "sha256:bb".into(),
                ..observed_entry("sha256:bb", 100, "2026-08-08T11:00:00Z")
            },
            5000,
        )
        .unwrap();
        let hit = q
            .unchanged_offer_at_path(&the_path(), 100, at("2026-08-08T11:00:00Z"))
            .unwrap();
        assert_eq!(hit.session_hash, "sha256:bb");
        assert_eq!(hit.state, QueueState::Pending);
    }

    #[test]
    fn a_path_with_no_entry_at_all_never_matches() {
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        assert!(
            q.unchanged_offer_at_path(
                Path::new("/Users/z/.claude/projects/x/other.jsonl"),
                100,
                at("2026-08-08T11:00:00Z")
            )
            .is_none()
        );
    }

    #[test]
    fn the_reoffer_supersede_mints_carries_no_observation() {
        // Its content is what the uploader's re-hash found, which the
        // watcher never observed. Recording the old entry's observation
        // against it would claim an offer exists for content nothing was
        // offered for.
        let mut q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        let fresh = q
            .supersede(
                entry_id_for("sha256:aa"),
                "sha256:bb",
                900,
                at("2026-08-08T16:00:00Z"),
            )
            .unwrap();
        assert_eq!(fresh.observed_modified_at, None);
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
    fn a_queue_line_written_before_the_subagent_fields_still_loads() {
        // `Queue::load` drops any line it cannot parse, with one warning. A
        // field added without `#[serde(default)]` would therefore empty a
        // contributor's whole queue on upgrade -- silently, from their point
        // of view. This is the regression test for exactly that.
        let (_d, store) = temp_store();
        let mut value = serde_json::to_value(entry("sha256:aa", "2026-08-08T12:00:00Z")).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("subagent_count");
        object.remove("subagents_dropped");
        store
            .write_daemon_file(DAEMON_QUEUE_FILE, format!("{value}\n").as_bytes())
            .unwrap();

        let loaded = Queue::load(&store).unwrap();
        assert_eq!(loaded.all().len(), 1, "the entry must survive the upgrade");
        assert_eq!(loaded.all()[0].subagent_count, 0);
        assert_eq!(loaded.all()[0].subagents_dropped, 0);
    }

    #[test]
    fn replacing_retires_only_stale_live_entries_at_that_path() {
        // `upsert` dedups on hash alone, so without this a conversation
        // would collect a fresh card every time it delegated. Entries that
        // already match the new hash, sit at another path, or have reached a
        // terminal state are all untouched -- the last of those is history.
        let mut q = Queue::new();
        let path = PathBuf::from("/Users/z/.claude/projects/x/s.jsonl");

        let mut stale = entry("sha256:old", "2026-08-08T12:00:00Z");
        stale.path = path.clone();
        q.upsert(stale, 500).unwrap();

        let mut elsewhere = entry("sha256:other", "2026-08-08T12:00:00Z");
        elsewhere.path = PathBuf::from("/Users/z/.claude/projects/x/t.jsonl");
        q.upsert(elsewhere, 500).unwrap();

        let mut done = entry("sha256:done", "2026-08-08T12:00:00Z");
        done.path = path.clone();
        q.upsert(done, 500).unwrap();
        q.set_state(entry_id_for("sha256:done"), QueueState::Uploaded, None);

        let mut current = entry("sha256:new", "2026-08-08T12:00:00Z");
        current.path = path.clone();
        let outcome = q.replace_live_at_path(current, 500).unwrap();
        assert_eq!(outcome.superseded, 1);
        assert!(outcome.inserted);
        assert_eq!(
            q.get(entry_id_for("sha256:old")).unwrap().state,
            QueueState::Superseded
        );
        assert_eq!(
            q.get(entry_id_for("sha256:old"))
                .unwrap()
                .reason_label
                .as_deref(),
            Some(REASON_CHANGED)
        );
        assert_eq!(
            q.get(entry_id_for("sha256:new")).unwrap().state,
            QueueState::Pending
        );
        assert_eq!(
            q.get(entry_id_for("sha256:other")).unwrap().state,
            QueueState::Pending
        );
        assert_eq!(
            q.get(entry_id_for("sha256:done")).unwrap().state,
            QueueState::Uploaded,
            "a resolved entry is history and must not be rewritten"
        );
    }

    #[test]
    fn a_full_queue_keeps_the_offer_it_could_not_replace() {
        // The window this closes: retire-then-insert meant a `queue-full` on
        // the insert left the conversation with no live offer at all, and
        // nothing to re-retire later. Refusing before mutating means a full
        // queue delays the new offer instead of destroying the old one.
        let mut q = Queue::new();
        let path = PathBuf::from("/Users/z/.claude/projects/x/s.jsonl");

        let mut live = entry("sha256:old", "2026-08-08T12:00:00Z");
        live.path = path.clone();
        q.upsert(live, 2).unwrap();
        // A second live entry elsewhere, so retiring the first would not by
        // itself free the slot this call needs.
        let mut other = entry("sha256:other", "2026-08-08T12:00:00Z");
        other.path = PathBuf::from("/Users/z/.claude/projects/x/t.jsonl");
        q.upsert(other, 2).unwrap();

        let mut third = entry("sha256:third", "2026-08-08T12:00:00Z");
        third.path = PathBuf::from("/Users/z/.claude/projects/x/u.jsonl");
        assert!(
            q.replace_live_at_path(third, 2).is_err(),
            "the cap still applies"
        );
        assert_eq!(
            q.get(entry_id_for("sha256:old")).unwrap().state,
            QueueState::Pending,
            "the existing offer must survive a refused replacement"
        );
        assert_eq!(q.all().len(), 2);
    }

    #[test]
    fn a_replacement_does_not_have_to_wait_for_the_offer_it_retires() {
        // The stale card is being replaced, not joined, so counting it as an
        // occupant would let one busy conversation wedge itself out of the
        // queue with its own predecessors.
        let mut q = Queue::new();
        let path = PathBuf::from("/Users/z/.claude/projects/x/s.jsonl");

        let mut live = entry("sha256:old", "2026-08-08T12:00:00Z");
        live.path = path.clone();
        q.upsert(live, 1).unwrap();

        let mut fresh = entry("sha256:new", "2026-08-08T12:05:00Z");
        fresh.path = path.clone();
        let outcome = q.replace_live_at_path(fresh, 1).unwrap();
        assert!(outcome.inserted);
        assert_eq!(outcome.superseded, 1);
    }

    #[test]
    fn re_observing_the_same_hash_inserts_nothing_and_retires_nothing() {
        let mut q = Queue::new();
        let e = entry("sha256:aa", "2026-08-08T12:00:00Z");
        let path = e.path.clone();
        q.upsert(e.clone(), 500).unwrap();

        let outcome = q.replace_live_at_path(e, 500).unwrap();
        assert!(!outcome.inserted);
        assert_eq!(outcome.superseded, 0);
        assert_eq!(q.all().len(), 1);
        assert_eq!(q.all()[0].path, path);
    }

    #[test]
    fn superseded_entries_are_compacted_but_receipts_never_are() {
        // A superseded row records only that an offer was replaced by one
        // that is still in the file, and grouping mints one per delegation.
        // Uploaded rows carry the receipt that history joins on, so they are
        // never what gets discarded -- see `MAX_SUPERSEDED_ENTRIES`.
        let mut q = Queue::new();
        for i in 0..(MAX_SUPERSEDED_ENTRIES + 10) {
            let hash = format!("sha256:s{i:03}");
            // Ascending timestamps, so "most recent" is unambiguous and the
            // oldest are the ones expected to go.
            let discovered = format!("2026-08-{:02}T12:00:00Z", (i % 28) + 1);
            q.upsert(entry(&hash, &discovered), 5000).unwrap();
            q.set_state(entry_id_for(&hash), QueueState::Superseded, None);
        }
        for state in [
            QueueState::Uploaded,
            QueueState::Failed,
            QueueState::Refused,
            QueueState::Expired,
        ] {
            let hash = format!("sha256:{state:?}");
            q.upsert(entry(&hash, "2026-07-01T12:00:00Z"), 5000)
                .unwrap();
            q.set_state(entry_id_for(&hash), state, None);
        }
        let live = entry("sha256:live", "2026-07-01T12:00:00Z");
        q.upsert(live, 5000).unwrap();

        assert_eq!(q.compact_superseded(), 10);
        assert_eq!(
            q.all()
                .iter()
                .filter(|e| e.state == QueueState::Superseded)
                .count(),
            MAX_SUPERSEDED_ENTRIES
        );
        for state in [
            QueueState::Uploaded,
            QueueState::Failed,
            QueueState::Refused,
            QueueState::Expired,
            QueueState::Pending,
        ] {
            assert!(
                q.all().iter().any(|e| e.state == state),
                "compaction must only ever touch Superseded, lost {state:?}"
            );
        }
        // Idempotent: a second pass with nothing over the bound is a no-op.
        assert_eq!(q.compact_superseded(), 0);
    }

    #[test]
    fn compaction_keeps_the_most_recent_superseded_entries() {
        let mut q = Queue::new();
        for i in 0..(MAX_SUPERSEDED_ENTRIES + 5) {
            let hash = format!("sha256:s{i:03}");
            let discovered = format!("2026-08-01T12:{i:02}:00Z");
            q.upsert(entry(&hash, &discovered), 5000).unwrap();
            q.set_state(entry_id_for(&hash), QueueState::Superseded, None);
        }
        q.compact_superseded();
        for i in 0..5 {
            assert!(
                q.get(entry_id_for(&format!("sha256:s{i:03}"))).is_none(),
                "the five oldest offers are the ones dropped"
            );
        }
        assert!(
            q.get(entry_id_for(&format!(
                "sha256:s{:03}",
                MAX_SUPERSEDED_ENTRIES + 4
            )))
            .is_some(),
            "the newest must survive"
        );
    }

    #[test]
    fn queue_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(Queue::load(&store).unwrap(), Queue::new());
    }

    #[test]
    fn an_approval_records_when_it_was_given_and_holds_until_the_window_ends() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        let at = at("2026-08-08T12:00:00Z");
        assert!(q.approve(id, &[], None, Some(at)));

        let e = q.get(id).unwrap();
        assert_eq!(e.approved_at, Some(at));
        assert_eq!(e.hold_until(10), Some(at + Duration::seconds(10)));
        assert!(e.hold_active(at, 10));
        assert!(e.hold_active(at + Duration::seconds(9), 10));
        // Released exactly at the deadline the client was given, so waiting
        // out the reported instant is waiting out precisely the hold.
        assert!(!e.hold_active(at + Duration::seconds(10), 10));
    }

    #[test]
    fn a_standing_opt_in_approval_is_not_held() {
        // `None` for `approved_at` is the auto-upload path, and it is
        // deliberate rather than an omission -- see `Queue::approve`.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        assert!(q.approve(id, &[], None, None));
        let e = q.get(id).unwrap();
        assert_eq!(e.hold_until(10), None);
        assert!(!e.hold_active(at("2026-08-08T12:00:00Z"), 10));
    }

    #[test]
    fn a_zero_hold_setting_reports_no_deadline_at_all() {
        // A client must be able to tell "no undo window" from "a window I
        // have to compute myself": zero reports no deadline rather than one
        // equal to the approval instant.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        let at = at("2026-08-08T12:00:00Z");
        assert!(q.approve(id, &[], None, Some(at)));
        assert_eq!(q.get(id).unwrap().hold_until(0), None);
        assert!(!q.get(id).unwrap().hold_active(at, 0));
    }

    #[test]
    fn cancel_and_revocation_both_clear_the_hold() {
        // A re-offered entry must not carry the previous approval's
        // deadline: the next approval starts its own window.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        let at = at("2026-08-08T12:00:00Z");

        assert!(q.approve(id, &[], None, Some(at)));
        q.cancel(id).unwrap();
        assert!(q.get(id).unwrap().approved_at.is_none());

        assert!(q.approve(id, &[], None, Some(at)));
        q.revoke_approval(id, "approval-inputs-changed");
        assert!(q.get(id).unwrap().approved_at.is_none());
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
    fn cancel_clears_the_pin_so_the_next_approval_rebuilds() {
        // Undo withdraws the approval, and the pin is the approval's
        // binding to the exact bytes it covered. Left behind, it makes a
        // second Submit approve the artifact built at the first click --
        // stale bytes if the session grew, and no counts to show either
        // time, because `approve` does not rebuild an entry that is
        // already pinned.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500)
            .unwrap();
        let id = entry_id_for("sha256:aa");
        assert!(q.record_previewed_envelope(id, "sha256:envelope"));
        assert!(q.approve(id, &[], None, Some(at("2026-08-08T12:00:00Z"))));
        assert!(
            q.get(id).unwrap().previewed_envelope_digest.is_some(),
            "the fixture must actually pin something, or this proves nothing"
        );

        q.cancel(id).unwrap();
        assert_eq!(
            q.get(id).unwrap().previewed_envelope_digest,
            None,
            "an undone approval must leave no pin behind"
        );
        assert!(
            !q.pinned_entry_ids().contains(&id),
            "and the bytes it named must stop being kept on disk"
        );
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
    #[test]
    fn a_load_can_land_while_the_queue_is_below_the_cap() {
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        assert!(q.load_can_land(&PathBuf::from("/some/other/s.jsonl"), 2));
    }

    #[test]
    fn a_load_for_an_unheld_path_cannot_land_once_the_queue_is_at_the_cap() {
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        assert!(!q.load_can_land(&PathBuf::from("/some/other/s.jsonl"), 1));
    }

    #[test]
    fn a_load_for_a_path_with_a_live_entry_can_land_even_at_the_cap() {
        // `replace_live_at_path` supersedes the entry sitting here, which
        // frees the slot the replacement takes. Refusing this load would
        // strand a grown session on a stale card forever.
        let q = queue_of(vec![observed_entry(
            "sha256:aa",
            100,
            "2026-08-08T11:00:00Z",
        )]);
        assert!(q.load_can_land(&the_path(), 1));
    }

    #[test]
    fn a_dead_entry_at_the_path_does_not_make_a_full_queue_loadable() {
        // Only a live entry can be superseded, so only a live entry frees a
        // slot. A superseded card here is just an occupant of history.
        let mut q = queue_of(vec![
            observed_entry("sha256:aa", 100, "2026-08-08T11:00:00Z"),
            QueueEntry {
                path: PathBuf::from("/Users/z/.claude/projects/x/other.jsonl"),
                ..observed_entry("sha256:bb", 100, "2026-08-08T11:00:00Z")
            },
        ]);
        let dead = entry_id_for("sha256:aa");
        q.set_state(dead, QueueState::Superseded, None);
        assert!(
            !q.load_can_land(&the_path(), 1),
            "one live entry elsewhere still fills a cap of one"
        );
    }
}
