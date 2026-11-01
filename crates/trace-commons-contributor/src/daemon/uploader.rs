//! Turning an approved queue entry into an upload.
//!
//! The uploader adds three things to the shared submit pipeline, all of them
//! consequences of the upload being unattended:
//!
//! 1. **A re-hash guard, and an approval-terms guard.** The contributor
//!    approves a description of a session -- a project, a size, a time --
//!    and, if they previewed it, a specific redacted envelope. Digests
//!    batch every few hours, so the file can grow between the offer and the
//!    approval; if it did, the approval does not cover the current content,
//!    so nothing is uploaded and a fresh offer is made instead.
//!
//!    The re-hash guard alone was not enough, and the comment that used to
//!    live here claimed more than the code did. It verifies the *input*.
//!    Everything else that determines what actually leaves the machine --
//!    the privacy-filter selection, the NEAR AI backend and model, the
//!    consent scopes, the identity and endpoints stamped onto the envelope,
//!    and the redaction service's own output -- could move between the
//!    preview and the send with the session hash unchanged, and the guard
//!    stayed silent. So the approval also pins an input fingerprint
//!    (`preview::input_fingerprint`), re-derived here before every upload;
//!    a mismatch re-offers the entry instead of uploading it.
//!
//!    For a previewed entry the artifact is not re-derived at all: the
//!    envelope the contributor was shown was written to disk
//!    (`daemon::approved_envelope`) and `submit_loaded` sends precisely
//!    those bytes. This replaced a digest comparison that was correct and
//!    unusable -- an LLM-backed privacy filter does not reproduce its own
//!    spans, so every previewed entry was refused forever. Together these
//!    are the central consent property of the whole daemon.
//! 2. **Revocation checks.** A cached claim stays valid for minutes after a
//!    logout, so enrollment is re-checked immediately before every upload
//!    rather than once at startup.
//! 3. **Volume caps.** A background process spending the contributor's
//!    bandwidth and privacy-filter budget stops at a daily ceiling.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::health::{
    HealthState, LABEL_ADMISSION_LIMIT_REACHED, LABEL_ADMISSION_REFUSED, LABEL_CANARY_FAILED,
    LABEL_CLAIM_MINT_FAILED, LABEL_INGEST_UNREACHABLE, LABEL_NEAR_AI_NOTICE_PENDING,
    LABEL_NOT_LOGGED_IN, LABEL_PII_FILTER_UNAVAILABLE,
};
use super::queue::QueueEntry;
use super::settings::DaemonSettings;
use super::state::DaemonState;
use crate::config::ConfigStore;
use crate::source::{SessionRef, TraceSource};
use crate::submit::{
    PRECONDITION_CANARY_FAILED, PRECONDITION_NEAR_AI_NOTICE_UNRECORDED, PRECONDITION_NOT_LOGGED_IN,
    SubmitContext, SubmitOutcome, SubmitPreconditionFailure,
};
use trace_commons_protocol::admission::AdmissionRefusal;
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

#[derive(Debug, PartialEq, Eq)]
pub enum UploadDecision {
    Uploaded {
        submission_id: Uuid,
        /// What the receipt fetch produced for this upload, so the daemon can
        /// correct the attestation mark on a row whose receipt did not
        /// arrive. See `attestation_mark::writeback_after_upload`.
        receipt: crate::submit::ReceiptShipped,
        /// For a session approved on the contributor's behalf, what its
        /// certificate shows about the redaction it had (K6), for the armed
        /// project's disclosure. `None` for one a person approved.
        unattended_redaction: Option<super::automatic_gate::SessionRedaction>,
    },
    /// Already delivered previously; nothing sent.
    AlreadySubmitted { submission_id: Uuid },
    /// The session changed after it was offered. Nothing was sent, and
    /// `new_hash` describes what is on disk now.
    Superseded { new_hash: String },
    /// The pipeline declined to send this, fail-closed.
    Refused { reason_label: String },
    /// The approval no longer covers what would be sent -- an
    /// envelope-determining input moved, or the envelope the pipeline built
    /// is not the one the contributor was shown. Nothing was sent; the
    /// entry goes back in front of the contributor under `reason_label`.
    ApprovalStale { reason_label: String },
    /// Approved on the contributor's behalf, certified by the witness, and
    /// held for a person because the certificate's verdict is not `low`.
    /// Nothing reached the commons. The certified review is already saved
    /// under the entry; `pin` is its digest.
    HeldForReview {
        reason_label: String,
        pin: String,
        attested_inference: Option<crate::witness::inference_record::InferenceAttestationRecord>,
        /// The serialized size, in bytes, of the certified bytes `pin` names
        /// (K10) -- the witness's own `envelope_bytes.len()`, measured fresh
        /// when this decision was reached from a witness response this call
        /// just received, or carried forward from `QueueEntry::would_send_bytes`
        /// when it instead re-affirms a pin an earlier pass already measured
        /// (see `Uploader::upload_entry`'s witness re-affirmation branch).
        /// `None` only when neither is available.
        would_send_bytes: Option<u64>,
    },
    /// Approved on the contributor's behalf and held for a person by the
    /// Scrub check (K4 of #1118): under Manual, before anything was built;
    /// under Automatic, because the envelope just built is worth a second
    /// look. Nothing was sent to the commons. `reasons` are that envelope's
    /// `second_look` labels, empty when nothing was built (Manual); labels
    /// only, for the log.
    ///
    /// `pin` is the digest of the built envelope, already saved under the
    /// entry through the same `approved_envelope::save` a preview pins with,
    /// and its counts. The queue pins that digest and records the counts
    /// beside it, so the held entry reads `scrubbed` with its real
    /// `second_look` and a person's review shows those bytes. `None` when
    /// nothing was built (Manual), on the witness path, or when the save
    /// failed: the entry is held all the same, and reads not yet scrubbed.
    HeldForSecondLook {
        reason_label: String,
        reasons: Vec<&'static str>,
        pin: Option<(String, super::second_look::ScrubCounts)>,
        /// The serialized size, in bytes, of the envelope `pin` names (K10).
        /// `Some` only alongside a `Some` pin, and `None` whenever `pin` is
        /// `None` for any of the reasons documented above.
        would_send_bytes: Option<u64>,
    },
    /// Network, auth, or transient classifier failure.
    Failed { reason_label: String },
    /// The witness is at capacity (`503 witness_saturated`) and judged
    /// nothing. Nothing was sent. The session keeps its approval and is
    /// tried again no sooner than `retry_after_secs`, the witness's own
    /// bounded `Retry-After`.
    WitnessSaturated { retry_after_secs: u32 },
    /// Ingest's per-principal rate limit turned the upload away (`429`, see
    /// `submit::REASON_RATE_LIMITED`). Nothing was judged and nothing is
    /// lost: the session keeps its approval and is tried again no sooner
    /// than `retry_after_secs`, the server's own bounded hint. The pass
    /// stops here, so the rest are not sent into the same limit.
    RateLimited { retry_after_secs: u32 },
    /// This daemon's own burst guard ([`burst_wait`]) held the upload back
    /// before anything was read or sent. Handled like
    /// [`UploadDecision::RateLimited`] -- the session keeps its approval and
    /// the pass stops -- except that it is not counted as an attempt, since
    /// nothing was tried.
    Paced { retry_after_secs: u32 },
}

/// How many uploads one daemon starts inside [`BURST_WINDOW`] at most.
///
/// Kept a little under ingest's default per-principal submission rate (30 a
/// minute), so a daemon draining a long approved backlog paces itself
/// rather than leaning on the server's `429` to slow it. Not a quota: the
/// window slides, and nothing is counted beyond it.
pub const BURST_MAX_UPLOADS: usize = 25;
// Ingest's compiled default per-principal submission rate, from the shared
// protocol constant, so a change to that default fails this build. It cannot
// see an operator's `TRACE_COMMONS_SUBMIT_PER_PRINCIPAL_RATE_PER_MIN`: under a
// lower limit the server's own `429` paces the daemon instead, ending the pass
// ([`UploadDecision::RateLimited`]).
const _: () = assert!(
    BURST_MAX_UPLOADS < trace_commons_protocol::submit_rate::SUBMIT_PER_PRINCIPAL_LIMIT as usize
);

/// The span [`BURST_MAX_UPLOADS`] is measured over.
pub const BURST_WINDOW: chrono::TimeDelta =
    chrono::TimeDelta::seconds(trace_commons_protocol::submit_rate::SUBMIT_RATE_WINDOW_SECS as i64);

/// Whole seconds until another upload fits the burst window, or `None` when
/// one may start now. Drops attempts that have left the window as it goes.
///
/// A full window answers with the time until its oldest attempt ages out,
/// rounded up and at least one second, which the caller reports as
/// [`UploadDecision::Paced`]: the same pause and silence as a server-side
/// `429`, with the session keeping its approval.
pub fn burst_wait(state: &mut DaemonState, now: DateTime<Utc>) -> Option<u32> {
    let floor = now - BURST_WINDOW;
    while state
        .recent_upload_attempts
        .front()
        .is_some_and(|at| *at <= floor)
    {
        state.recent_upload_attempts.pop_front();
    }
    // An attempt stamped after `now` means the wall clock stepped back. It is
    // moved to `now` rather than dropped: ingest counts on a monotonic clock
    // and still holds the burst, so an emptied window would send another one
    // straight into its `429`. Held this way, the window stays full for at
    // most one more window, instead of until the clock catches up.
    for at in &mut state.recent_upload_attempts {
        *at = (*at).min(now);
    }
    if state.recent_upload_attempts.len() < BURST_MAX_UPLOADS {
        return None;
    }
    let oldest = *state.recent_upload_attempts.front()?;
    // Rounded up, so the retry lands after the slot frees, not just before.
    let wait_ms = (oldest + BURST_WINDOW - now).num_milliseconds();
    let wait = (wait_ms + 999).div_euclid(1000).max(1);
    Some(u32::try_from(wait).unwrap_or(u32::MAX))
}

/// Save the certified review of a held session, so opening it shows the
/// witness's bytes rather than running the witness again (the spec's R5).
///
/// If it cannot be saved the entry is still held, only without the pin:
/// nothing is sent either way, and a person opening it re-runs the witness,
/// which is where this was before R5.
fn hold_for_review(
    store: &ConfigStore,
    entry_id: Uuid,
    session_hash: &str,
    inputs: &str,
    reason_label: String,
    witnessed: crate::witness::transport::WitnessedEnvelope,
    attested_inference: crate::witness::inference_record::InferenceAttestationRecord,
) -> UploadDecision {
    // K10: measured before `witnessed` moves into the artifact below -- the
    // exact wire bytes this held review's upload will carry, once a person
    // clears it.
    let would_send_bytes = Some(witnessed.envelope_bytes.len() as u64);
    let artifact = super::approved_envelope::WitnessReviewArtifact::new(
        witnessed,
        session_hash.to_string(),
        inputs.to_string(),
        None,
        None,
        Some(attested_inference.clone()),
    );
    match artifact.digest() {
        Ok(pin) if super::approved_envelope::save_witnessed(store, entry_id, &artifact).is_ok() => {
            UploadDecision::HeldForReview {
                reason_label,
                pin,
                attested_inference: Some(attested_inference),
                would_send_bytes,
            }
        }
        _ => UploadDecision::ApprovalStale { reason_label },
    }
}

/// Save the envelope the Scrub check held, so the entry can pin it (K4 of
/// #1118). The same `approved_envelope::save` and `preview::envelope_digest`
/// a preview pins with, so the held bytes are kept exactly like previewed
/// ones: swept when the entry resolves, released after the preview age limit
/// like any pinned `Pending` preview.
///
/// `None` when it cannot be saved. The entry is still held, only without
/// the pin, so it reads not yet scrubbed and a person's review builds again.
///
/// Also returns the envelope's serialized size (K10), measured from the
/// same value the digest and the save both already touch, so a held entry's
/// `would_send_bytes` is never `Some` without a `pin` beside it.
fn pin_second_look(
    store: &ConfigStore,
    entry_id: Uuid,
    envelope: &TraceContributionEnvelope,
) -> Option<(String, u64)> {
    let digest = super::preview::envelope_digest(envelope).ok()?;
    super::approved_envelope::save(store, entry_id, envelope).ok()?;
    let size = crate::envelope::envelope_size(envelope).ok()? as u64;
    Some((digest, size))
}

/// Map a pipeline outcome onto a daemon decision, so the queue records a
/// fixed label rather than pipeline internals.
fn decision_for(
    outcome: SubmitOutcome,
    receipt: crate::submit::ReceiptShipped,
    witness_retry_after: Option<u32>,
    rate_limit_retry_after: Option<u32>,
) -> UploadDecision {
    match outcome {
        SubmitOutcome::Failed { reason_label }
            if reason_label == crate::submit::REASON_RATE_LIMITED =>
        {
            UploadDecision::RateLimited {
                retry_after_secs: rate_limit_retry_after
                    .unwrap_or(crate::submit::RATE_LIMIT_DEFAULT_RETRY_SECS),
            }
        }
        SubmitOutcome::Failed { reason_label }
            if reason_label == crate::submit::REASON_WITNESS_SATURATED =>
        {
            UploadDecision::WitnessSaturated {
                retry_after_secs: witness_retry_after.unwrap_or(
                    trace_commons_protocol::witness_pacing::WITNESS_SATURATED_RETRY_AFTER_SECS,
                ),
            }
        }
        SubmitOutcome::Refused { reason_label, .. } | SubmitOutcome::Failed { reason_label }
            if matches!(
                reason_label.as_str(),
                "witness-grant-changed" | "witness-review-stale"
            ) =>
        {
            UploadDecision::ApprovalStale { reason_label }
        }
        SubmitOutcome::Submitted { submission_id, .. } => UploadDecision::Uploaded {
            submission_id,
            receipt,
            unattended_redaction: None,
        },
        SubmitOutcome::AlreadySubmitted { submission_id, .. } => {
            UploadDecision::AlreadySubmitted { submission_id }
        }
        SubmitOutcome::SkippedParseFailure { reason_label } => {
            UploadDecision::Refused { reason_label }
        }
        SubmitOutcome::Refused { reason_label, .. } => UploadDecision::Refused { reason_label },
        SubmitOutcome::Failed { reason_label } => UploadDecision::Failed { reason_label },
        // Turned into a decision by `Uploader::hold_for_review`, which has
        // the entry the certified review is saved under; never reaches here.
        SubmitOutcome::HeldForReview { reason_label, .. } => {
            UploadDecision::ApprovalStale { reason_label }
        }
        // Turned into a decision by `pin_second_look`, which has the entry
        // the envelope is saved under; never reaches here.
        SubmitOutcome::HeldForSecondLook {
            reason_label,
            reasons,
            ..
        } => UploadDecision::HeldForSecondLook {
            reason_label,
            reasons,
            pin: None,
            would_send_bytes: None,
        },
    }
}

/// The health label a decision implies, if any.
pub fn health_label_for(decision: &UploadDecision) -> Option<&'static str> {
    match decision {
        UploadDecision::WitnessSaturated { .. } => Some(super::health::LABEL_WITNESS_SATURATED),
        // A pause the next pass clears on its own, with the session kept.
        // Like a lease another attempt holds, nothing to tell anyone about;
        // above all not `ingest-unreachable`, which reads as an outage.
        UploadDecision::RateLimited { .. } | UploadDecision::Paced { .. } => None,
        UploadDecision::Refused { reason_label } => match reason_label.as_str() {
            "pii-filter-unavailable" => Some(LABEL_PII_FILTER_UNAVAILABLE),
            LABEL_NEAR_AI_NOTICE_PENDING => Some(LABEL_NEAR_AI_NOTICE_PENDING),
            LABEL_NOT_LOGGED_IN => Some(LABEL_NOT_LOGGED_IN),
            _ => None,
        },
        UploadDecision::Failed { reason_label } => match reason_label.as_str() {
            "claim-mint-failed" => Some(LABEL_CLAIM_MINT_FAILED),
            crate::submit::REASON_TRANSIENT_REDACTION => Some(LABEL_PII_FILTER_UNAVAILABLE),
            // A refusal the commons sent on purpose, before the catch-all
            // that reads everything else as an outage.
            other => match AdmissionRefusal::from_label(other) {
                Some(AdmissionRefusal::LimitReached | AdmissionRefusal::AccountLimitReached) => {
                    Some(LABEL_ADMISSION_LIMIT_REACHED)
                }
                // A lease another attempt holds, which the next retry
                // resolves. Nothing for a contributor to be told about.
                Some(AdmissionRefusal::InProgress) => None,
                Some(_) => Some(LABEL_ADMISSION_REFUSED),
                None => Some(LABEL_INGEST_UNREACHABLE),
            },
        },
        _ => None,
    }
}

/// The health label for a fail-closed precondition that aborted a submit
/// pass.
///
/// Reads the typed `SubmitPreconditionFailure` rather than matching on
/// error text. An error that is not one of those cannot currently arise
/// from `submit_one`, but if one ever does it must still block expiry
/// rather than silently letting the clock run, so the fallback is the
/// most conservative blocking label rather than `None`.
pub fn precondition_health_label(e: &anyhow::Error) -> &'static str {
    match e.downcast_ref::<SubmitPreconditionFailure>().map(|f| f.0) {
        Some(PRECONDITION_CANARY_FAILED) => LABEL_CANARY_FAILED,
        Some(PRECONDITION_NEAR_AI_NOTICE_UNRECORDED) => LABEL_NEAR_AI_NOTICE_PENDING,
        Some(PRECONDITION_NOT_LOGGED_IN) => LABEL_NOT_LOGGED_IN,
        _ => LABEL_PII_FILTER_UNAVAILABLE,
    }
}

/// Whether this store still holds a usable enrollment.
///
/// Checked immediately before every upload, not once at startup: a cached
/// claim outlives a logout by minutes, and the receipts file it would append
/// to is gone.
pub fn enrollment_is_live(store: &ConfigStore) -> bool {
    match store.load_config() {
        Ok(Some(_)) => store.load_device_key().ok().flatten().is_some(),
        _ => false,
    }
}

pub struct Uploader<'a, 'ctx> {
    pub ctx: &'a mut SubmitContext<'ctx>,
    pub store: &'a ConfigStore,
    pub settings: &'a DaemonSettings,
    pub state: &'a mut DaemonState,
    pub health: &'a mut HealthState,
}

impl Uploader<'_, '_> {
    /// The exact redacted envelope this entry was approved as, if it was
    /// previewed at all.
    ///
    /// `Ok(None)` -- never previewed (armed auto-upload, approve-all), so
    /// the pipeline builds the envelope as it always has.
    /// `Ok(Some(_))` -- previewed; send these bytes.
    /// `Err(label)` -- previewed, but the bytes are gone, unreadable, or
    /// not the ones the entry is pinned to. Fail-closed: the approval is
    /// revoked and the entry re-offered rather than rebuilt.
    ///
    /// The digest re-check is a consistency check on this crate's own
    /// storage -- a truncated file, a file crossed over from another entry
    /// -- not a check on redaction. Redaction is not re-run here at all.
    fn approved_witness_for(
        &self,
        entry: &QueueEntry,
    ) -> Result<super::approved_envelope::WitnessReviewArtifact> {
        let artifact = super::approved_envelope::load_witnessed(self.store, entry.entry_id)?
            .ok_or_else(|| anyhow::anyhow!("witness-review-stale"))?;
        if Some(artifact.digest()?.as_str()) != entry.previewed_envelope_digest.as_deref() {
            anyhow::bail!("witness-review-stale");
        }
        let fingerprint = super::preview::input_fingerprint(
            self.ctx.effective_cfg(),
            self.ctx.near_ai(),
            self.settings.ironwire_attested_bodies,
        );
        artifact.validate(
            self.ctx.effective_cfg(),
            &entry.session_hash,
            &fingerprint,
            entry.approved_verdict.as_deref(),
            entry.approved_correction.as_deref(),
        )?;
        Ok(artifact)
    }

    fn approved_envelope_for(
        &self,
        entry: &QueueEntry,
    ) -> Result<Option<TraceContributionEnvelope>, String> {
        let Some(pinned) = entry.previewed_envelope_digest.as_deref() else {
            return Ok(None);
        };
        let stored = super::approved_envelope::load(self.store, entry.entry_id)
            .map_err(|_| super::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE.to_string())?
            .ok_or_else(|| super::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE.to_string())?;
        let actual = super::preview::envelope_digest(&stored)
            .map_err(|_| super::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE.to_string())?;
        if actual != pinned {
            return Err(super::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE.to_string());
        }
        // AFTER the digest check, never before it. The check is a
        // consistency check on this crate's own storage and has to run
        // against the bytes as stored; applying the verdict first would make
        // a truncated or crossed-over file pass.
        //
        // This is the one deliberate divergence from "the upload sends
        // precisely the stored bytes", and it is bounded to
        // `outcome.task_success`. See this module's doc note.
        //
        // An unparseable stored verdict is ignored rather than refused: the
        // IPC boundary already validates, so a bad value here means a
        // hand-edited queue file, and refusing the upload would strand the
        // entry.
        let mut stored = stored;
        if let Some(name) = entry.approved_verdict.as_deref() {
            if let Some(verdict) = crate::envelope::ContributorVerdict::parse(name) {
                crate::envelope::apply_verdict(&mut stored, verdict);
            }
        }
        Ok(Some(stored))
    }

    /// Upload one queue entry, or explain why not.
    pub async fn upload_entry(
        &mut self,
        source: &dyn TraceSource,
        session_ref: &SessionRef,
        entry: &QueueEntry,
        now: DateTime<Utc>,
    ) -> Result<UploadDecision> {
        // The Manual Scrub check (K4 of #1118): nothing goes without a
        // person. The watcher approves nothing on anyone's behalf while it
        // is set; this catches an approval made before the switch, and
        // holds it before anything is read, built or sent.
        if entry.approved_unattended
            && self.settings.scrub_check == super::settings::ScrubCheck::Manual
        {
            return Ok(UploadDecision::HeldForSecondLook {
                reason_label: super::second_look::REASON_SCRUB_CHECK_MANUAL.to_string(),
                reasons: Vec::new(),
                pin: None,
                would_send_bytes: None,
            });
        }
        if !enrollment_is_live(self.store) {
            self.health.fail(LABEL_NOT_LOGGED_IN, now);
            return Ok(UploadDecision::Refused {
                reason_label: LABEL_NOT_LOGGED_IN.to_string(),
            });
        }
        // Enrollment is live, so retract the not-logged-in condition if it was set.
        self.health.resolve(LABEL_NOT_LOGGED_IN);

        if self.settings.near_ai.is_some() && !self.store.near_ai_notice_shown() {
            // The one-time notice is delivered interactively. Under a service
            // manager its output goes to a log nobody reads, so a daemon that
            // consumed the marker would send the contributor's text to a third
            // party with the notice never actually delivered.
            self.health.fail(LABEL_NEAR_AI_NOTICE_PENDING, now);
            return Ok(UploadDecision::Refused {
                reason_label: LABEL_NEAR_AI_NOTICE_PENDING.to_string(),
            });
        }
        // Near-AI notice requirement is met (either not configured or already shown),
        // so retract the notice-pending condition if it was set.
        self.health.resolve(LABEL_NEAR_AI_NOTICE_PENDING);

        // Pace the send rate under ingest's per-principal limit before any
        // work is spent on this entry. No health label: like the server's
        // own `429`, it clears itself on the next pass. The attempt itself is
        // counted only once it is about to go to ingest, below.
        if let Some(retry_after_secs) = burst_wait(self.state, now) {
            return Ok(UploadDecision::Paced { retry_after_secs });
        }
        // `now` is when this entry's turn came; the send starts later, after
        // the read and hash below.
        let entry_started = std::time::Instant::now();

        // Re-read and re-hash. The approval was for the content described by
        // entry.session_hash; if the file has moved on, that approval does not
        // transfer to the new content.
        // `source.load` reads the whole session file and hashes it -- blocking,
        // non-yielding work with no `.await` of its own, run once per approved
        // entry from inside the supervisor's task. Off-worker for the same
        // reason `watcher::tick`'s scan is; see `super::run_blocking`'s doc.
        //
        // A large session takes the process-wide gate before the read and
        // keeps it through the build in `submit_loaded`, which releases it
        // before the upload. A preview of another large session waits
        // meanwhile; a small one never does. See `crate::large_session_gate`.
        // The larger of the queued and the freshly discovered size, so a
        // session that grew since it was queued is still judged large.
        let large_session =
            crate::large_session_gate::admit(entry.size_bytes.max(session_ref.size_bytes)).await;
        let transcript = match super::run_blocking(|| source.load(session_ref)) {
            Ok(t) => t,
            Err(_) => {
                return Ok(UploadDecision::Refused {
                    reason_label: "parse-failed".to_string(),
                });
            }
        };
        if transcript.session_hash != entry.session_hash {
            return Ok(UploadDecision::Superseded {
                new_hash: transcript.session_hash,
            });
        }

        // The input half of the approval-terms guard, re-derived from what
        // this pipeline will actually use rather than from what the config
        // said at some earlier moment. `None` on an approved entry means
        // the terms were never recorded (an entry from before this field
        // existed, or an approval taken with no readable config), which is
        // "unknown, so re-ask": fail-closed.
        let inputs_now = super::preview::input_fingerprint(
            self.ctx.effective_cfg(),
            self.ctx.near_ai(),
            self.settings.ironwire_attested_bodies,
        );
        if entry.approved_inputs.as_deref() != Some(inputs_now.as_str()) {
            return Ok(UploadDecision::ApprovalStale {
                reason_label: super::preview::REASON_INPUTS_CHANGED.to_string(),
            });
        }

        // The artifact half. A previewed entry carries the envelope it was
        // previewed as, on disk; the pipeline sends exactly those bytes.
        // Nothing rebuilds and compares -- see
        // `SubmitContext::use_approved_envelope` and
        // `daemon::approved_envelope` for why a comparison cannot work
        // against an LLM-backed privacy filter.
        //
        // Fail-closed on anything unexpected: an entry pinned to a preview
        // whose bytes are gone, unreadable, or not the ones that were
        // pinned goes back in front of the contributor. It is never
        // silently rebuilt, because rebuilding is precisely how a
        // contributor ends up sending something they were never shown.
        if entry.holds_witness_certificate() {
            let result = self.approved_witness_for(entry);
            match result {
                Ok(artifact) => {
                    if artifact.token_bundle.is_some()
                        != self.settings.token_distributions_contribution
                    {
                        return Ok(UploadDecision::ApprovalStale {
                            reason_label: "token-distribution-consent-changed".into(),
                        });
                    }
                    // A pinned review is not a person's approve. A contributor
                    // can open a review on a waiting session in an automatic
                    // project, see a verdict that is not `low`, and close it;
                    // the watcher then approves the entry on their behalf with
                    // the pin still in place. The same hold as a fresh witness
                    // run, read from the certificate `approved_witness_for`
                    // has just validated, and before anything is handed to
                    // the pipeline.
                    if entry.approved_unattended
                        && !crate::submit::verdict_is_low(artifact.response())
                    {
                        if let Some(pin) = entry.previewed_envelope_digest.clone() {
                            return Ok(UploadDecision::HeldForReview {
                                reason_label: crate::submit::REASON_WITNESS_RISK_REVIEW_REQUIRED
                                    .to_string(),
                                pin,
                                attested_inference: entry.attested_inference.clone(),
                                // Re-affirming an already-pinned review, not
                                // measuring fresh bytes (K10): carry the
                                // entry's own recorded figure forward rather
                                // than reporting `None` for a bound that has
                                // not actually changed.
                                would_send_bytes: entry.would_send_bytes,
                            });
                        }
                        return Ok(UploadDecision::ApprovalStale {
                            reason_label: "witness-review-stale".to_string(),
                        });
                    }
                    self.ctx
                        .use_approved_token_bundle(artifact.token_bundle.clone());
                    if self
                        .ctx
                        .use_approved_witness(artifact.response().clone())
                        .is_err()
                    {
                        return Ok(UploadDecision::ApprovalStale {
                            reason_label: "witness-review-stale".to_string(),
                        });
                    }
                }
                Err(_) => {
                    return Ok(UploadDecision::ApprovalStale {
                        reason_label: "witness-review-stale".to_string(),
                    });
                }
            }
        } else {
            if self.settings.token_distributions_contribution {
                return Ok(UploadDecision::ApprovalStale {
                    reason_label: super::queue::REASON_TOKEN_DISTRIBUTION_REVIEW_REQUIRED.into(),
                });
            }
            match self.approved_envelope_for(entry) {
                Ok(approved) => self.ctx.use_approved_envelope(approved),
                Err(reason_label) => return Ok(UploadDecision::ApprovalStale { reason_label }),
            }
            // Nobody has looked at this session. If the witness is what
            // builds it, its verdict is the first point at which the risk is
            // known, and one that is not `low` stops here for a person.
            if entry.approved_unattended {
                self.ctx.hold_witnessed_unless_low_risk();
            }
        }

        // `submit_one` returns `Err` only for a fail-closed precondition
        // (`SubmitPreconditionFailure`) -- a privacy-filter canary that did
        // not catch its planted secret, an unrecordable NEAR AI notice, a
        // missing device identity. Each of those stops the whole pass, and
        // each must set its health label *before* the error propagates:
        // `LABEL_CANARY_FAILED` is in `EXPIRY_BLOCKING_LABELS` but was
        // never set by any production path, so during a filter outage the
        // daemon reported healthy, `blocks_expiry()` was false, and
        // `queue.expire` discarded pending traces as
        // expired-without-decision -- exactly what expiry suspension exists
        // to prevent.
        // `submit_loaded`, not `submit_one`: the transcript just loaded and
        // hashed above is the one that gets sent. `submit_one` would load
        // the file a third, independent time, and it was *that* read --
        // never hashed, never compared -- whose bytes went out. A session
        // appended to between the two reads passed the guard and shipped
        // content the guard had never seen.
        //
        // The Automatic Scrub check (K4 of #1118), armed last so no early
        // return above can leave the one-shot flag behind for an unrelated
        // session. The watcher approved this without building anything, so
        // its marks were not known then; `submit_loaded` decides the hold on
        // the envelope it builds, before the send. A person's approval is
        // never held here: they are the second look.
        //
        // Automatic is the default, and a settings file that never chose
        // loads as it. Manual has already held above, before anything was
        // read or built.
        if entry.approved_unattended
            && self.settings.scrub_check == super::settings::ScrubCheck::Automatic
        {
            self.ctx.hold_unless_scrub_is_clear(entry.subagents_dropped);
        }
        // K7: the entry's own provenance -- whether a person approved it or
        // an armed folder sent it without asking, and the verdict if any --
        // travels onto the receipt this call produces, and from there onto
        // the history row. Set immediately before the call so it cannot
        // apply to a submission this context runs later.
        self.ctx
            .set_upload_provenance(entry.approved_unattended, entry.approved_verdict.clone());
        self.ctx.hold_large_session_permit(large_session);
        // Counted in the burst window from here: ingest limits the
        // submissions it receives, and every return above sent nothing.
        // Stamped with the time the send starts, not when the pass did.
        let sent_at = now
            + chrono::TimeDelta::from_std(entry_started.elapsed())
                .unwrap_or(chrono::TimeDelta::zero());
        self.state.recent_upload_attempts.push_back(sent_at);
        let outcome = match self.ctx.submit_loaded(transcript).await {
            Ok(o) => o,
            Err(e) => {
                // A fail-closed precondition stops before the upload.
                self.state.recent_upload_attempts.pop_back();
                self.health.fail(precondition_health_label(&e), now);
                return Err(e);
            }
        };
        let decision = match outcome {
            SubmitOutcome::HeldForReview {
                reason_label,
                witnessed,
                attested_inference,
            } => hold_for_review(
                self.store,
                entry.entry_id,
                &entry.session_hash,
                &inputs_now,
                reason_label,
                *witnessed,
                *attested_inference,
            ),
            SubmitOutcome::HeldForSecondLook {
                reason_label,
                reasons,
                counts,
                envelope,
            } => {
                let pinned = envelope
                    .and_then(|envelope| pin_second_look(self.store, entry.entry_id, &envelope));
                UploadDecision::HeldForSecondLook {
                    reason_label,
                    reasons,
                    would_send_bytes: pinned.as_ref().map(|(_, size)| *size),
                    pin: pinned.map(|(digest, _)| (digest, counts)),
                }
            }
            outcome => decision_for(
                outcome,
                self.ctx.last_receipt_shipped(),
                self.ctx.last_witness_retry_after(),
                self.ctx.last_rate_limit_retry_after(),
            ),
        };
        // A hold, or a busy witness, stops before the upload: nothing
        // reached ingest, so it does not count against the window.
        if matches!(
            decision,
            UploadDecision::HeldForReview { .. }
                | UploadDecision::HeldForSecondLook { .. }
                | UploadDecision::WitnessSaturated { .. }
        ) {
            self.state.recent_upload_attempts.pop_back();
        }
        // K6: checked on this session's own certificate, the one it was
        // sent with, and only for a session nobody looked at.
        let decision = match decision {
            UploadDecision::Uploaded {
                submission_id,
                receipt,
                ..
            } if entry.approved_unattended => {
                let (witnessed, witness) = self.ctx.last_sent_witness();
                UploadDecision::Uploaded {
                    submission_id,
                    receipt,
                    unattended_redaction: Some(super::automatic_gate::session_redaction(
                        witnessed, witness,
                    )),
                }
            }
            other => other,
        };

        match &decision {
            UploadDecision::Uploaded { .. } => {
                self.state
                    .record_upload(&entry.path, &entry.session_hash, entry.size_bytes);
                self.health.clear();
            }
            other => {
                if let Some(label) = health_label_for(other) {
                    self.health.fail(label, now);
                }
            }
        }
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R5: a held session keeps the certified review it was held with, so a
    /// person opening it sees the witness's bytes without a second witness
    /// run. The pin in the decision names exactly what was saved.
    #[test]
    fn a_held_session_keeps_the_certified_review_it_was_held_with() {
        let (_d, store) = crate::config::tests_support::temp_store();
        let id = Uuid::new_v4();
        let witnessed = crate::witness::transport::WitnessedEnvelope {
            admission: None,
            envelope_bytes: b"{}".to_vec(),
            certificate_json: r#"{"residual_risk_verdict":"medium"}"#.into(),
            signature_hex: format!("0x{}", "ab".repeat(65)),
        };
        let record = crate::witness::inference_record::InferenceAttestationRecord::uncertified(
            crate::witness::inference_record::REASON_BODIES_WITHHELD,
        );
        let decision = hold_for_review(
            &store,
            id,
            "sha256:aa",
            "fp",
            crate::submit::REASON_WITNESS_RISK_REVIEW_REQUIRED.into(),
            witnessed.clone(),
            record.clone(),
        );
        let UploadDecision::HeldForReview {
            reason_label,
            pin,
            attested_inference,
            ..
        } = decision
        else {
            panic!("expected a hold, got {decision:?}");
        };
        assert_eq!(
            reason_label,
            crate::submit::REASON_WITNESS_RISK_REVIEW_REQUIRED
        );
        assert_eq!(attested_inference, Some(record));
        let saved = super::super::approved_envelope::load_witnessed(&store, id)
            .unwrap()
            .expect("the certified review is saved");
        assert_eq!(saved.digest().unwrap(), pin);
        assert_eq!(saved.response().envelope_bytes, witnessed.envelope_bytes);
    }
    use crate::config::tests_support::temp_store;
    use crate::daemon::queue::{QueueEntry, QueueState, entry_id_for};
    use crate::source::claude_code::ClaudeCodeSource;
    use std::path::PathBuf;

    use crate::daemon::test_support::at;

    /// Completed uploads this state has recorded, across every path.
    fn uploads_recorded(state: &DaemonState) -> u32 {
        state.prior_uploads.values().map(|p| p.upload_count).sum()
    }

    fn settings() -> DaemonSettings {
        DaemonSettings::default()
    }

    #[test]
    fn the_burst_guard_lets_uploads_through_below_its_ceiling() {
        let mut st = DaemonState::new();
        let now = at("2026-08-08T20:00:00Z");
        for _ in 0..BURST_MAX_UPLOADS - 1 {
            st.recent_upload_attempts.push_back(now);
        }
        assert_eq!(burst_wait(&mut st, now), None);
    }

    #[test]
    fn the_burst_guard_holds_a_full_window_until_its_oldest_attempt_ages_out() {
        let mut st = DaemonState::new();
        let start = at("2026-08-08T20:00:00Z");
        for _ in 0..BURST_MAX_UPLOADS {
            st.recent_upload_attempts.push_back(start);
        }
        let now = start + chrono::TimeDelta::seconds(15);
        assert_eq!(burst_wait(&mut st, now), Some(45));
        // Once the window has slid past them, the next upload fits, and the
        // aged-out attempts are forgotten rather than counted forever.
        assert_eq!(burst_wait(&mut st, start + BURST_WINDOW), None);
        assert!(st.recent_upload_attempts.is_empty());
    }

    #[test]
    fn the_burst_guard_is_measured_over_ingests_one_minute_window() {
        assert_eq!(
            BURST_WINDOW.num_seconds() as u64,
            trace_commons_protocol::submit_rate::SUBMIT_RATE_WINDOW_SECS
        );
    }

    /// A wall clock stepped back after a burst does not empty the window:
    /// ingest counts on a monotonic clock and still holds the burst, so
    /// attempts stamped after `now` are moved to `now` rather than dropped.
    /// The window stays full for at most one more window.
    #[test]
    fn a_clock_step_back_keeps_the_burst_window_full() {
        let mut st = DaemonState::new();
        let start = at("2026-08-08T20:00:00Z");
        for _ in 0..BURST_MAX_UPLOADS {
            st.recent_upload_attempts.push_back(start);
        }
        // The clock stepped back one second after the burst.
        let now = start - chrono::TimeDelta::seconds(1);
        assert_eq!(burst_wait(&mut st, now), Some(60));
        assert_eq!(st.recent_upload_attempts.len(), BURST_MAX_UPLOADS);
        assert!(st.recent_upload_attempts.iter().all(|at| *at <= now));
        assert_eq!(burst_wait(&mut st, now + BURST_WINDOW), None);
    }

    /// The wait is rounded up, so a retry never lands just before the
    /// oldest attempt ages out and takes a second one-second pause.
    #[test]
    fn a_full_burst_window_rounds_its_wait_up() {
        let mut st = DaemonState::new();
        let start = at("2026-08-08T20:00:00Z");
        for _ in 0..BURST_MAX_UPLOADS {
            st.recent_upload_attempts.push_back(start);
        }
        let now = start + chrono::TimeDelta::milliseconds(15_100);
        assert_eq!(burst_wait(&mut st, now), Some(45));
    }

    #[test]
    fn a_full_burst_window_waits_at_least_one_second() {
        let mut st = DaemonState::new();
        let now = at("2026-08-08T20:00:00Z");
        for _ in 0..BURST_MAX_UPLOADS {
            st.recent_upload_attempts
                .push_back(now - BURST_WINDOW + chrono::TimeDelta::milliseconds(200));
        }
        assert_eq!(burst_wait(&mut st, now), Some(1));
    }

    #[test]
    fn a_failed_upload_maps_to_an_ingest_health_label() {
        let d = UploadDecision::Failed {
            reason_label: "upload-failed".into(),
        };
        assert_eq!(health_label_for(&d), Some(LABEL_INGEST_UNREACHABLE));
    }

    #[test]
    fn a_claim_failure_is_distinguished_from_an_ingest_failure() {
        let d = UploadDecision::Failed {
            reason_label: "claim-mint-failed".into(),
        };
        assert_eq!(health_label_for(&d), Some(LABEL_CLAIM_MINT_FAILED));
    }

    /// A refusal the commons sent deliberately is not an outage.
    ///
    /// Every `Failed` label except the claim one used to become
    /// `ingest-unreachable`, so a declined contribution and a spent budget
    /// both told the contributor the service was down, in front of a queue
    /// that would never drain by retrying.
    #[test]
    fn an_admission_refusal_is_not_an_outage() {
        use trace_commons_protocol::admission::AdmissionRefusal;
        for refusal in AdmissionRefusal::ALL {
            let d = UploadDecision::Failed {
                reason_label: refusal.label().into(),
            };
            assert_ne!(
                health_label_for(&d),
                Some(LABEL_INGEST_UNREACHABLE),
                "{} is reported as an outage",
                refusal.label()
            );
        }
        assert_eq!(
            health_label_for(&UploadDecision::Failed {
                reason_label: AdmissionRefusal::Refused.label().into(),
            }),
            Some(LABEL_ADMISSION_REFUSED)
        );
        assert_eq!(
            health_label_for(&UploadDecision::Failed {
                reason_label: AdmissionRefusal::LimitReached.label().into(),
            }),
            Some(LABEL_ADMISSION_LIMIT_REACHED)
        );
        assert_eq!(
            health_label_for(&UploadDecision::Failed {
                reason_label: AdmissionRefusal::AccountLimitReached.label().into()
            }),
            Some(LABEL_ADMISSION_LIMIT_REACHED),
            "the shared allowance health condition promises no window or reset"
        );
        // A lease another attempt is holding is resolved by the retry that
        // follows it. Reporting a condition for it would put a banner up for
        // a race that clears itself.
        assert_eq!(
            health_label_for(&UploadDecision::Failed {
                reason_label: AdmissionRefusal::InProgress.label().into(),
            }),
            None
        );
    }

    #[test]
    fn a_successful_upload_implies_no_health_failure() {
        let d = UploadDecision::Uploaded {
            submission_id: Uuid::nil(),
            receipt: crate::submit::ReceiptShipped::NoCall,
            unattended_redaction: None,
        };
        assert_eq!(health_label_for(&d), None);
    }

    /// A claude-code session in a tempdir that can be grown on demand.
    struct GrowingSession {
        _dir: tempfile::TempDir,
        root: PathBuf,
        path: PathBuf,
    }

    impl GrowingSession {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("projects");
            let project = root.join("-Users-testuser-code-myproj");
            std::fs::create_dir_all(&project).unwrap();
            let path = project.join("33333333-3333-3333-3333-333333333333.jsonl");
            std::fs::write(
                &path,
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"first question\"},\
                 \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T10:00:00Z\",\
                 \"version\":\"2.0.1\",\"sessionId\":\"33333333-3333-3333-3333-333333333333\",\
                 \"uuid\":\"a1\"}\n",
            )
            .unwrap();
            Self {
                _dir: dir,
                root,
                path,
            }
        }

        fn source(&self) -> ClaudeCodeSource {
            ClaudeCodeSource::new(self.root.clone())
        }

        fn session_ref(&self) -> SessionRef {
            self.source().discover().unwrap().remove(0)
        }

        fn current_hash(&self) -> String {
            self.source()
                .load(&self.session_ref())
                .unwrap()
                .session_hash
        }

        fn append_more_events(&self) {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap();
            f.write_all(
                b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"much later work\"},\
                  \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T18:00:00Z\",\
                  \"version\":\"2.0.1\",\"sessionId\":\"33333333-3333-3333-3333-333333333333\",\
                  \"uuid\":\"a2\"}\n",
            )
            .unwrap();
        }

        /// Write a delegated transcript beside the session, stamped with its
        /// `sessionId` so it verifies as a member of the group.
        fn add_subagent(&self, agent: &str) {
            let session = "33333333-3333-3333-3333-333333333333";
            let subagents = self.path.parent().unwrap().join(session).join("subagents");
            std::fs::create_dir_all(&subagents).unwrap();
            std::fs::write(
                subagents.join(format!("{agent}.jsonl")),
                format!(
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"delegated work\"}},\
                     \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T18:00:00Z\",\
                     \"version\":\"2.0.1\",\"sessionId\":\"{session}\",\"uuid\":\"s1\"}}\n"
                ),
            )
            .unwrap();
        }

        /// An approved entry whose recorded terms match `cfg` -- i.e. the
        /// ordinary case where nothing moved between approval and upload.
        fn entry_for(&self, hash: &str, cfg: &crate::config::ContributorConfig) -> QueueEntry {
            QueueEntry {
                approved_inputs: Some(crate::daemon::preview::input_fingerprint(cfg, None, false)),
                ..self.entry(hash)
            }
        }

        fn entry(&self, hash: &str) -> QueueEntry {
            QueueEntry {
                entry_id: entry_id_for(hash),
                session_hash: hash.to_string(),
                source: "claude-code".into(),
                project_key: "/Users/testuser/code/myproj".into(),
                project_label: "myproj".into(),
                path: self.path.clone(),
                size_bytes: std::fs::metadata(&self.path).unwrap().len(),
                discovered_at: at("2026-08-08T12:00:00Z"),
                state: QueueState::Approved,
                ..Default::default()
            }
        }
    }

    fn dry_run_opts() -> crate::submit::SubmitOptions {
        crate::submit::SubmitOptions {
            dry_run: true,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: true,
            unenrolled_preview: false,
            remediate_quarantined: false,
            verdict: None,
        }
    }

    async fn upload_fixture(
        ctx: &mut SubmitContext<'_>,
        store: &ConfigStore,
        session: &GrowingSession,
        entry: &QueueEntry,
    ) -> (UploadDecision, DaemonState, HealthState) {
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx,
            store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };

        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();
        (decision, state, health)
    }

    #[tokio::test]
    async fn upload_refuses_when_a_delegated_transcript_appeared_after_approval() {
        // The same consent property as the test below, one level out: an
        // approval covers a conversation, and a conversation that has since
        // delegated work to a subagent is not the one that was approved. The
        // group hash is what makes the re-hash guard see it -- with a
        // parent-only hash this would ship silently.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        // No public profile claimed. These are cache fields, excluded
        // from the input fingerprint, so they cannot affect what this
        // test measures -- the re-hash guard seeing a new subagent.
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let offered_hash = session.current_hash();
        let entry = session.entry_for(&offered_hash, &cfg);
        session.add_subagent("agent-a");

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        match decision {
            UploadDecision::Superseded { new_hash } => {
                assert_ne!(new_hash, entry.session_hash);
                assert_eq!(new_hash, session.current_hash());
            }
            other => panic!("expected Superseded, got {other:?}"),
        }
        assert_eq!(
            uploads_recorded(&state),
            0,
            "nothing may be uploaded once the conversation has grown a delegate"
        );
    }

    #[tokio::test]
    async fn upload_refuses_when_the_session_grew_after_it_was_offered() {
        // The central consent property: approve a 1-event session, never
        // ship one that has since gained an afternoon of work.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let offered_hash = session.current_hash();
        let entry = session.entry(&offered_hash);
        session.append_more_events();

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        match decision {
            UploadDecision::Superseded { new_hash } => {
                assert_ne!(new_hash, entry.session_hash);
                assert_eq!(new_hash, session.current_hash());
            }
            other => panic!("expected Superseded, got {other:?}"),
        }
        assert_eq!(
            uploads_recorded(&state),
            0,
            "nothing may be uploaded when the hash no longer matches"
        );
    }

    #[tokio::test]
    async fn upload_proceeds_when_the_hash_still_matches() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let entry = session.entry_for(&session.current_hash(), &cfg);
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "got {decision:?}"
        );
        assert_eq!(uploads_recorded(&state), 1);
        assert!(health.ok());
    }

    // --- the Scrub check (K4 of #1118) ------------------------------------

    /// A one-turn session whose only text is `text`, so a test decides how
    /// many marks the scrubber will make.
    fn session_saying(text: &str) -> GrowingSession {
        let session = GrowingSession::new();
        std::fs::write(
            &session.path,
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}},\
                 \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T10:00:00Z\",\
                 \"version\":\"2.0.1\",\"sessionId\":\"33333333-3333-3333-3333-333333333333\",\
                 \"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
        session
    }

    /// Text the scrubber finds nothing in, and text it takes an email out of.
    const NOTHING_TO_MARK: &str = "first question";
    const ONE_EMAIL: &str = "please write to alice.smith@example.org about it";

    /// Upload `entry` of `session` under `settings`, through the uploader,
    /// against a store holding `fixture_cfg`.
    async fn upload_under(
        session: &GrowingSession,
        entry_of: impl FnOnce(&GrowingSession, &crate::config::ContributorConfig) -> QueueEntry,
        settings: DaemonSettings,
    ) -> (UploadDecision, DaemonState) {
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        upload_in(&store, &cfg, session, entry_of(session, &cfg), settings).await
    }

    /// [`upload_under`] against a store and config the caller keeps, for a
    /// test that needs what the upload saved.
    async fn upload_in(
        store: &ConfigStore,
        cfg: &crate::config::ContributorConfig,
        session: &GrowingSession,
        entry: QueueEntry,
        settings: DaemonSettings,
    ) -> (UploadDecision, DaemonState) {
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(store, cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let decision = Uploader {
            ctx: &mut ctx,
            store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        }
        .upload_entry(
            &session.source(),
            &session.session_ref(),
            &entry,
            at("2026-08-08T16:00:00Z"),
        )
        .await
        .unwrap();
        (decision, state)
    }

    /// Approved on the contributor's behalf, as the watcher does in an
    /// armed folder: nothing built, so no marks recorded.
    fn armed(session: &GrowingSession, cfg: &crate::config::ContributorConfig) -> QueueEntry {
        QueueEntry {
            approved_unattended: true,
            ..session.entry_for(&session.current_hash(), cfg)
        }
    }

    fn manual() -> DaemonSettings {
        DaemonSettings {
            scrub_check: crate::daemon::settings::ScrubCheck::Manual,
            ..DaemonSettings::default()
        }
    }

    /// Automatic, stated explicitly. It is also the default
    /// (`the_default_scrub_check_holds_a_path_only_armed_session`).
    fn automatic() -> DaemonSettings {
        DaemonSettings {
            scrub_check: crate::daemon::settings::ScrubCheck::Automatic,
            ..DaemonSettings::default()
        }
    }

    /// Zaki's decision on #1139: the Scrub check defaults to Automatic. A
    /// fresh install that never chose, with the project armed, holds a
    /// session whose only marks are paths for a second look instead of
    /// sending it.
    #[tokio::test]
    async fn the_default_scrub_check_holds_a_path_only_armed_session() {
        let session =
            session_saying("open /Users/alice/code/orchard-api/src/main.rs and fix the parser");
        let (decision, state) = upload_under(&session, armed, DaemonSettings::default()).await;
        let pin = assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_NOTHING_MATCHED],
        );
        assert!(pin.is_some(), "the built envelope is pinned");
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// A hold under `reason`, decided on `expected` second-look labels.
    /// Returns the pin, which only the envelope-building holds carry.
    fn assert_held(
        decision: &UploadDecision,
        reason: &str,
        expected: &[&'static str],
    ) -> Option<(String, crate::daemon::second_look::ScrubCounts)> {
        let UploadDecision::HeldForSecondLook {
            reason_label,
            reasons,
            pin,
            ..
        } = decision
        else {
            panic!("expected a hold, got {decision:?}");
        };
        assert_eq!(reason_label, reason);
        assert_eq!(reasons.as_slice(), expected);
        pin.clone()
    }

    use crate::daemon::second_look::{
        REASON_LOOKS_UNSURE, REASON_NOTHING_MATCHED, REASON_SECOND_LOOK_REVIEW_REQUIRED,
        REASON_TRIMMED_TO_FIT,
    };

    /// An address the scrubber takes out, and an obfuscated one it leaves
    /// and the unsure-span detector flags.
    const ONE_EMAIL_ONE_UNSURE: &str =
        "please write to alice.smith@example.org or ops [at] acme [dot] io";

    /// Manual: an armed session with plenty of marks still waits, before
    /// anything is built.
    #[tokio::test]
    async fn manual_holds_an_armed_session_however_clean_its_scrub() {
        let session = session_saying(ONE_EMAIL);
        let (decision, state) = upload_under(&session, armed, manual()).await;
        assert_held(
            &decision,
            crate::daemon::second_look::REASON_SCRUB_CHECK_MANUAL,
            &[],
        );
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// Manual holds approvals made on anyone's behalf, never a person's own.
    #[tokio::test]
    async fn manual_sends_what_a_person_approved() {
        let session = session_saying(NOTHING_TO_MARK);
        let (decision, state) = upload_under(
            &session,
            |s, cfg| s.entry_for(&s.current_hash(), cfg),
            manual(),
        )
        .await;
        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "{decision:?}"
        );
        assert_eq!(uploads_recorded(&state), 1);
    }

    /// Automatic: nothing matched, so it waits.
    #[tokio::test]
    async fn automatic_holds_an_armed_session_where_nothing_matched() {
        let session = session_saying(NOTHING_TO_MARK);
        let (decision, state) = upload_under(&session, armed, automatic()).await;
        assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_NOTHING_MATCHED],
        );
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// Automatic: something was removed, but the redacted body still holds a
    /// span that looks like personal data, so it waits.
    #[tokio::test]
    async fn automatic_holds_an_armed_session_that_looks_unsure() {
        let session = session_saying(ONE_EMAIL_ONE_UNSURE);
        let (decision, state) = upload_under(&session, armed, automatic()).await;
        assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_LOOKS_UNSURE],
        );
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// Reviewed on #1139: path removals alone are not a match. Nearly every
    /// real session carries absolute paths; a hold that counted them would
    /// almost never fire. This one was uploaded before the counts came from
    /// `ScrubCounts::of`.
    #[tokio::test]
    async fn automatic_holds_a_session_whose_only_marks_are_paths() {
        let session =
            session_saying("open /Users/alice/code/orchard-api/src/main.rs and fix the parser");
        let (decision, state) = upload_under(&session, armed, automatic()).await;
        let pin = assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_NOTHING_MATCHED],
        );
        let (_, counts) = pin.expect("the built envelope is pinned");
        assert!(counts.marks > 0, "the path was removed: {counts:?}");
        assert_eq!(counts.content_marks, 0, "and only the path: {counts:?}");
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// Reviewed on #1139: a survivor the unsure-span detector flags holds,
    /// with no content marks as well. This one was uploaded before the body
    /// was counted.
    #[tokio::test]
    async fn automatic_holds_a_session_with_an_unsure_survivor() {
        let session = session_saying("key from ops [at] acme [dot] io, rotate it tonight");
        let (decision, state) = upload_under(&session, armed, automatic()).await;
        assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_NOTHING_MATCHED, REASON_LOOKS_UNSURE],
        );
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// Automatic: trimmed to fit waits even with a clean scrub.
    #[tokio::test]
    async fn automatic_holds_an_armed_session_trimmed_to_fit() {
        let session = session_saying(ONE_EMAIL);
        let (decision, state) = upload_under(
            &session,
            |s, cfg| QueueEntry {
                subagents_dropped: 2,
                ..armed(s, cfg)
            },
            automatic(),
        )
        .await;
        assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_TRIMMED_TO_FIT],
        );
        assert_eq!(uploads_recorded(&state), 0, "nothing was sent");
    }

    /// Automatic: a scrubbed session with marks and nothing trimmed goes.
    #[tokio::test]
    async fn automatic_sends_an_armed_session_with_a_clean_scrub() {
        let session = session_saying(ONE_EMAIL);
        let (decision, state) = upload_under(&session, armed, automatic()).await;
        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "{decision:?}"
        );
        assert_eq!(uploads_recorded(&state), 1);
    }

    /// The hold is decided on the envelope actually built, never on the
    /// entry's own record. The watcher approved this without a preview: the
    /// entry carries no record, or a stale one claiming a clean scrub of
    /// some other build. Either way the built envelope has nothing in it, and
    /// it waits.
    #[tokio::test]
    async fn automatic_decides_on_the_built_envelope_not_the_entrys_record() {
        let clean_elsewhere = crate::daemon::second_look::ScrubRecord {
            envelope_digest: "sha256:another-build".into(),
            counts: crate::daemon::second_look::ScrubCounts {
                marks: 5,
                content_marks: 5,
                unsure_spans: 0,
                unsure_unreadable: false,
            },
        };
        for recorded in [None, Some(clean_elsewhere)] {
            let session = session_saying(NOTHING_TO_MARK);
            let (decision, state) = upload_under(
                &session,
                |s, cfg| QueueEntry {
                    scrub: recorded.clone(),
                    ..armed(s, cfg)
                },
                automatic(),
            )
            .await;
            assert_held(
                &decision,
                REASON_SECOND_LOOK_REVIEW_REQUIRED,
                &[REASON_NOTHING_MATCHED],
            );
            assert_eq!(
                uploads_recorded(&state),
                0,
                "recorded {recorded:?}: nothing sent"
            );
        }
    }

    /// The whole round: held, kept held by the queue against unattended
    /// approval, and sent by a person's approve.
    #[tokio::test]
    async fn a_held_session_waits_for_a_person_and_their_approve_sends_it() {
        let session = session_saying(NOTHING_TO_MARK);
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let entry = armed(&session, &cfg);
        let (decision, _) = upload_in(&store, &cfg, &session, entry.clone(), automatic()).await;
        let pin = assert_held(
            &decision,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            &[REASON_NOTHING_MATCHED],
        );
        let (digest, counts) = pin.expect("the built envelope is pinned");
        let saved = crate::daemon::approved_envelope::load(&store, entry.entry_id)
            .unwrap()
            .expect("the held bytes are saved under the entry");
        assert_eq!(
            crate::daemon::preview::envelope_digest(&saved).unwrap(),
            digest,
            "the pin names exactly the saved bytes"
        );

        // What `drain_approved` does with it.
        let mut q = crate::daemon::queue::Queue::default();
        q.upsert(entry.clone(), 10).unwrap();
        assert!(q.hold_with_scrub_pin(
            entry.entry_id,
            REASON_SECOND_LOOK_REVIEW_REQUIRED,
            Some((digest.as_str(), counts)),
            None
        ));
        let held = q.get(entry.entry_id).unwrap().clone();
        assert!(held.held_for_review());
        assert_eq!(
            held.previewed_envelope_digest.as_deref(),
            Some(digest.as_str())
        );
        assert_eq!(
            held.scrub(),
            crate::daemon::second_look::Scrub::Scrubbed(counts),
            "the scrub record is kept beside the pin"
        );
        assert_eq!(held.second_look_reasons(), vec![REASON_NOTHING_MATCHED]);
        assert!(
            !q.approve_unattended(entry.entry_id, &cfg.consent_scopes, None),
            "nobody approves it on the contributor's behalf"
        );

        // A person.
        assert!(q.approve(
            entry.entry_id,
            &cfg.consent_scopes,
            entry.approved_inputs.as_deref(),
            None,
            None,
            None
        ));
        let approved = q.get(entry.entry_id).unwrap().clone();
        assert!(!approved.approved_unattended);
        assert_eq!(
            approved.previewed_envelope_digest.as_deref(),
            Some(digest.as_str()),
            "the person approves the held bytes"
        );
        let (decision, state) = upload_in(&store, &cfg, &session, approved, automatic()).await;
        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "{decision:?}"
        );
        assert_eq!(uploads_recorded(&state), 1);
    }

    async fn spawn_stub(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn stub_claim_issuer() -> axum::Router {
        axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation", "model_training"],
                    "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                }))
            }),
        )
    }

    fn stub_ingest_accepts() -> axum::Router {
        axum::Router::new().route(
            "/v1/traces",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "status": "accepted",
                    "credit_points_pending": 0.0,
                    "explanation": []
                }))
            }),
        )
    }

    /// K7: `upload_entry` must hand the queue entry's own provenance to the
    /// submit pipeline before it uploads, so the receipt this real (not
    /// dry-run) upload writes -- and from it, the history row -- can say
    /// whether a person approved this session or an armed folder sent it
    /// without asking. Removing the `set_upload_provenance` call in
    /// `upload_entry` makes this fail: the receipt would read the default
    /// `(false, None)` for an entry that was, in fact, sent unattended.
    #[tokio::test]
    async fn upload_entry_records_the_entrys_own_provenance_on_the_receipt() {
        let issuer = spawn_stub(stub_claim_issuer()).await;
        let ingest = spawn_stub(stub_ingest_accepts()).await;
        // An address the scrubber removes, so the default (Automatic) Scrub
        // check lets this armed session through to the receipt.
        let session = session_saying(ONE_EMAIL);
        let (_d, store) = temp_store();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            consent_scopes_chosen: Some(true),
            witness_origin: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: issuer,
            ingest_url: ingest,
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        };
        store.save_config(&cfg).unwrap();

        let mut entry = session.entry_for(&session.current_hash(), &cfg);
        entry.approved_unattended = true;

        let opts = crate::submit::SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: true,
            unenrolled_preview: false,
            remediate_quarantined: false,
            verdict: None,
        };
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };

        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();
        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "got {decision:?}"
        );

        let receipts = store.load_receipts().unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(
            receipts[0].approved_unattended,
            Some(true),
            "the entry was sent by an armed folder, not a person"
        );
    }

    /// A large session waits for the process-wide gate before it is read,
    /// and lets go of it before the upload: the stub ingest takes the gate
    /// itself when the envelope arrives, which it could not do if the
    /// uploader were still holding it across the network transfer.
    #[tokio::test]
    async fn a_large_upload_takes_the_gate_to_build_and_releases_it_to_send() {
        use crate::large_session_gate::{LARGE_SESSION_BYTES, admit};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::Duration;

        // How many uploads found the gate free when they arrived, and how
        // many found it held.
        let free_at_upload = Arc::new(AtomicU32::new(0));
        let held_at_upload = Arc::new(AtomicU32::new(0));
        let ingest_router = {
            let free = free_at_upload.clone();
            let held = held_at_upload.clone();
            axum::Router::new().route(
                "/v1/traces",
                axum::routing::post(move || {
                    let free = free.clone();
                    let held = held.clone();
                    async move {
                        match tokio::time::timeout(
                            Duration::from_secs(10),
                            admit(LARGE_SESSION_BYTES + 1),
                        )
                        .await
                        {
                            Ok(_permit) => free.fetch_add(1, Ordering::SeqCst),
                            Err(_) => held.fetch_add(1, Ordering::SeqCst),
                        };
                        axum::Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                    }
                }),
            )
        };
        let issuer = spawn_stub(stub_claim_issuer()).await;
        let ingest = spawn_stub(ingest_router).await;
        let session = session_saying(ONE_EMAIL);
        let (_d, store) = temp_store();
        let mut cfg = fixture_cfg(&store);
        cfg.issuer_url = issuer;
        cfg.ingest_url = ingest;
        store.save_config(&cfg).unwrap();

        // Only the size the gate reads is large; the file itself is tiny.
        let mut entry = session.entry_for(&session.current_hash(), &cfg);
        entry.size_bytes = LARGE_SESSION_BYTES + 1;

        let opts = crate::submit::SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: true,
            unenrolled_preview: false,
            remediate_quarantined: false,
            verdict: None,
        };
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };

        // Another large build holds the gate: this one waits its turn.
        let other = admit(LARGE_SESSION_BYTES + 1).await;
        let waited = tokio::time::timeout(
            Duration::from_millis(200),
            up.upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            ),
        )
        .await;
        assert!(
            waited.is_err(),
            "a large upload must wait while another large build holds the gate"
        );
        assert_eq!(free_at_upload.load(Ordering::SeqCst), 0);
        assert_eq!(held_at_upload.load(Ordering::SeqCst), 0);
        drop(other);

        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();
        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "got {decision:?}"
        );
        assert_eq!(
            free_at_upload.load(Ordering::SeqCst),
            1,
            "the gate must be free by the time the envelope is on the wire"
        );
        assert_eq!(held_at_upload.load(Ordering::SeqCst), 0);
    }

    /// The config the fixture sessions above are approved and uploaded
    /// under.
    fn fixture_cfg(store: &ConfigStore) -> crate::config::ContributorConfig {
        let device = crate::identity::DeviceIdentity::load_or_generate(store).unwrap();
        crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            consent_scopes_chosen: Some(true),
            witness_origin: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        }
    }

    #[tokio::test]
    async fn upload_refuses_when_an_envelope_determining_input_moved_but_the_bytes_did_not() {
        // THE finding. The raw session file is byte-for-byte what was
        // approved -- the re-hash guard is perfectly happy -- but the
        // configuration that determines the envelope has moved underneath
        // the approval. Before this guard, the uploader re-hashed only the
        // transcript, then had `submit_loaded` rebuild the envelope from
        // whatever the config said at that moment, and the upload went out
        // silently under terms the contributor never saw.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let approved_under = fixture_cfg(&store);
        store.save_config(&approved_under).unwrap();
        let entry = session.entry_for(&session.current_hash(), &approved_under);

        // Not a byte of the session changes. Only an envelope-determining
        // input does: the contributor's identity is now stamped onto the
        // envelope under a different tenant.
        let mut cfg = approved_under.clone();
        cfg.tenant_id = "tenant-somebody-else".into();
        store.save_config(&cfg).unwrap();
        assert_eq!(
            session.current_hash(),
            entry.session_hash,
            "the raw session bytes must be unchanged for this test to mean anything"
        );

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert_eq!(
            decision,
            UploadDecision::ApprovalStale {
                reason_label: crate::daemon::preview::REASON_INPUTS_CHANGED.to_string(),
            },
            "an approval must not transfer to an envelope built from different inputs"
        );
        assert_eq!(uploads_recorded(&state), 0, "nothing may be uploaded");
    }

    /// Reviewed on #1024: an entry approved before a grant is voided is not
    /// sent after it. Voiding stops *new* unattended approvals; what stops an
    /// approval already made is this guard, so every change that voids a
    /// grant is run through the uploader here, not only compared as
    /// fingerprints. (The environment's filter reaches the same fingerprint
    /// through `env_filter_backend`; `grant_terms`'s test covers it without
    /// mutating the process environment.)
    #[tokio::test]
    async fn an_approval_taken_before_a_voiding_change_is_not_sent_after_it() {
        use crate::config::WitnessSettings;
        type Change = Box<dyn Fn(&mut crate::config::ContributorConfig, &mut SettingsLike)>;
        struct SettingsLike {
            near_ai: Option<crate::envelope::NearAiSettings>,
            attested_bodies: bool,
        }
        let changes: Vec<(&str, Change)> = vec![
            (
                "destination",
                Box::new(|c, _| c.ingest_url = "http://elsewhere.invalid".into()),
            ),
            (
                "identity",
                Box::new(|c, _| c.tenant_id = "tenant-other".into()),
            ),
            (
                "scopes widened",
                Box::new(|c, _| c.consent_scopes.push("model_training".into())),
            ),
            (
                "privacy filter",
                Box::new(|c, _| c.pii_filter = Some("near-ai".into())),
            ),
            (
                "receipt endpoint",
                Box::new(|c, _| c.inference_receipt_endpoint = Some("https://r.invalid".into())),
            ),
            (
                "witness",
                Box::new(|c, _| {
                    c.witness = Some(WitnessSettings {
                        admission_evidence: false,
                        url: "https://witness.invalid".into(),
                        signing_address: "0x0000000000000000000000000000000000000001".into(),
                        expected_measurements: Vec::new(),
                    })
                }),
            ),
            (
                "classifier",
                Box::new(|_, s| {
                    s.near_ai = Some(crate::envelope::NearAiSettings {
                        api_key: "k".into(),
                        base_url: Some("https://classifier.invalid".into()),
                        model: Some("m".into()),
                    })
                }),
            ),
            ("attested bodies", Box::new(|_, s| s.attested_bodies = true)),
        ];
        for (name, change) in &changes {
            let session = GrowingSession::new();
            let (_d, store) = temp_store();
            let approved_under = fixture_cfg(&store);
            store.save_config(&approved_under).unwrap();
            let entry = session.entry_for(&session.current_hash(), &approved_under);

            let mut cfg = approved_under.clone();
            let mut now = SettingsLike {
                near_ai: None,
                attested_bodies: false,
            };
            change(&mut cfg, &mut now);
            store.save_config(&cfg).unwrap();

            let opts = dry_run_opts();
            let mut ctx = SubmitContext::new(&store, &cfg, &opts, now.near_ai.clone()).unwrap();
            let mut state = DaemonState::new();
            let mut health = HealthState::default();
            let mut settings = settings();
            settings.ironwire_attested_bodies = now.attested_bodies;
            let mut up = Uploader {
                ctx: &mut ctx,
                store: &store,
                settings: &settings,
                state: &mut state,
                health: &mut health,
            };
            let decision = up
                .upload_entry(
                    &session.source(),
                    &session.session_ref(),
                    &entry,
                    at("2026-08-08T16:00:00Z"),
                )
                .await
                .unwrap();
            assert_eq!(
                decision,
                UploadDecision::ApprovalStale {
                    reason_label: crate::daemon::preview::REASON_INPUTS_CHANGED.to_string(),
                },
                "{name}: an approval from before the void must not be sent"
            );
            assert_eq!(uploads_recorded(&state), 0, "{name}");
        }
    }

    #[tokio::test]
    async fn upload_refuses_when_the_approved_envelope_bytes_are_gone() {
        // An entry pinned to a preview whose stored bytes are missing. The
        // daemon must NOT quietly rebuild the envelope -- rebuilding is
        // exactly what the stored bytes exist to avoid -- so it revokes the
        // approval and re-offers instead.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let entry = QueueEntry {
            previewed_envelope_digest: Some("sha256:pinned-but-never-stored".to_string()),
            ..session.entry_for(&session.current_hash(), &cfg)
        };

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert_eq!(
            decision,
            UploadDecision::ApprovalStale {
                reason_label: crate::daemon::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE
                    .to_string(),
            }
        );
        assert_eq!(uploads_recorded(&state), 0);
    }

    #[tokio::test]
    async fn a_witnessed_preview_refuses_by_name_rather_than_building_locally() {
        // The preview path has no claim -- it runs before the contributor has
        // answered -- and a witnessed envelope must carry the granted scopes
        // inside the bytes the certificate covers, which means minting first.
        //
        // Refusing rather than building locally is the point: these exact
        // bytes are what `use_approved_envelope` uploads later, so a
        // locally-redacted preview under a configured witness would be an
        // uncertified submission from a contributor who believes their
        // submissions are certified.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let mut cfg = fixture_cfg(&store);
        cfg.witness = Some(crate::config::WitnessSettings {
            admission_evidence: false,
            url: "http://witness.invalid".into(),
            signing_address: "0x1111111111111111111111111111111111111111".into(),
            expected_measurements: vec![format!(
                "mrtd={},mrconfigid={}",
                "aa".repeat(48),
                "bb".repeat(48)
            )],
        });
        store.save_config(&cfg).unwrap();

        let err = crate::daemon::preview::build_preview(
            &store,
            Some(&cfg),
            None,
            &session.source(),
            &session.session_ref(),
        )
        .await
        .expect_err("a witnessed preview has no claim to mint the grants from");
        assert_eq!(err.to_string(), "witness_claim_unavailable");
    }

    #[tokio::test]
    async fn an_unwitnessed_preview_still_builds() {
        // The positive control. Without it the refusal above would pass on a
        // preview path that had stopped working entirely.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        crate::daemon::preview::build_preview(
            &store,
            Some(&cfg),
            None,
            &session.source(),
            &session.session_ref(),
        )
        .await
        .expect("an unconfigured client previews exactly as it did before");
    }

    #[tokio::test]
    async fn upload_refuses_stored_bytes_that_are_not_the_ones_pinned() {
        // A consistency check on this crate's own storage, not on redaction:
        // a truncated file, or one crossed over from another entry, must not
        // be sent as though it were what the contributor approved.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let (_summary, _body, envelope) = crate::daemon::preview::build_preview(
            &store,
            Some(&cfg),
            None,
            &session.source(),
            &session.session_ref(),
        )
        .await
        .unwrap();
        let entry = QueueEntry {
            previewed_envelope_digest: Some("sha256:some-other-artifact".to_string()),
            ..session.entry_for(&session.current_hash(), &cfg)
        };
        crate::daemon::approved_envelope::save(&store, entry.entry_id, &envelope).unwrap();

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert_eq!(
            decision,
            UploadDecision::ApprovalStale {
                reason_label: crate::daemon::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE
                    .to_string(),
            }
        );
        assert_eq!(uploads_recorded(&state), 0);
    }

    #[tokio::test]
    async fn an_entry_uploads_the_stored_envelope_even_when_a_rebuild_would_differ() {
        // The case the previous design could not do at all. The stored
        // envelope deliberately is NOT what a rebuild produces -- one
        // redacted event body has been altered, which is precisely the shape
        // of an LLM-backed filter returning different spans the second time
        // -- and the upload must still go out, carrying those bytes. The
        // digest-comparison design refused this forever.
        //
        // `daemon_nondeterministic_filter.rs` drives the same property end
        // to end against a filter service that really does move.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let (_summary, _body, mut envelope) = crate::daemon::preview::build_preview(
            &store,
            Some(&cfg),
            None,
            &session.source(),
            &session.session_ref(),
        )
        .await
        .unwrap();
        envelope.events[0].redacted_content =
            Some("a redaction only this run produced".to_string());
        let pinned = crate::daemon::preview::envelope_digest(&envelope).unwrap();
        let entry = QueueEntry {
            previewed_envelope_digest: Some(pinned),
            ..session.entry_for(&session.current_hash(), &cfg)
        };
        crate::daemon::approved_envelope::save(&store, entry.entry_id, &envelope).unwrap();

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "a previewed entry must upload even though a rebuild would not \
             reproduce its envelope: got {decision:?}"
        );
        assert_eq!(uploads_recorded(&state), 1);
    }

    #[tokio::test]
    async fn an_entry_pinned_to_its_stored_envelope_uploads_normally() {
        // The ordinary case: a preview ran, its bytes were stored, the
        // contributor approved what it showed, nothing moved.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let (summary, _body, envelope) = crate::daemon::preview::build_preview(
            &store,
            Some(&cfg),
            None,
            &session.source(),
            &session.session_ref(),
        )
        .await
        .unwrap();
        let entry = QueueEntry {
            previewed_envelope_digest: Some(summary.envelope_digest.clone()),
            ..session.entry_for(&session.current_hash(), &cfg)
        };
        crate::daemon::approved_envelope::save(&store, entry.entry_id, &envelope).unwrap();

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "got {decision:?}"
        );
        assert_eq!(uploads_recorded(&state), 1);
    }

    #[tokio::test]
    async fn an_approved_entry_with_no_recorded_terms_is_re_offered_not_uploaded() {
        // Entries written before the approval terms existed, and approvals
        // taken with no readable config, record nothing. Unknown terms are
        // re-asked, never assumed to still hold.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let entry = session.entry(&session.current_hash());
        assert!(entry.approved_inputs.is_none());

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let (decision, state, _health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert_eq!(
            decision,
            UploadDecision::ApprovalStale {
                reason_label: crate::daemon::preview::REASON_INPUTS_CHANGED.to_string(),
            }
        );
        assert_eq!(uploads_recorded(&state), 0);
    }

    #[tokio::test]
    async fn upload_refuses_once_the_enrollment_is_gone() {
        // A cached claim outlives a logout by minutes.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let entry = session.entry_for(&session.current_hash(), &cfg);
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();

        // Log out underneath the running context.
        store.wipe().unwrap();

        let (decision, state, health) = upload_fixture(&mut ctx, &store, &session, &entry).await;

        assert_eq!(
            decision,
            UploadDecision::Refused {
                reason_label: LABEL_NOT_LOGGED_IN.to_string()
            }
        );
        assert_eq!(
            health.last_error_label.as_deref(),
            Some(LABEL_NOT_LOGGED_IN)
        );
        assert_eq!(uploads_recorded(&state), 0);
    }

    #[tokio::test]
    async fn upload_pauses_while_the_burst_window_is_full() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let entry = session.entry_for(&session.current_hash(), &cfg);
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let now = at("2026-08-08T16:00:00Z");
        for _ in 0..BURST_MAX_UPLOADS {
            state.recent_upload_attempts.push_back(now);
        }
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let decision = up
            .upload_entry(&session.source(), &session.session_ref(), &entry, now)
            .await
            .unwrap();

        assert_eq!(
            decision,
            UploadDecision::Paced {
                retry_after_secs: 60
            }
        );
        // The same quiet pause as the server's own limit: no health label,
        // nothing sent, and the refused attempt not counted against the
        // window.
        assert_eq!(health.last_error_label, None);
        assert_eq!(uploads_recorded(&state), 0);
        assert_eq!(state.recent_upload_attempts.len(), BURST_MAX_UPLOADS);
    }

    #[tokio::test]
    async fn every_upload_attempt_is_counted_in_the_burst_window() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let entry = session.entry_for(&session.current_hash(), &cfg);
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let now = at("2026-08-08T16:00:00Z");
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let _ = up
            .upload_entry(&session.source(), &session.session_ref(), &entry, now)
            .await
            .unwrap();
        assert_eq!(state.recent_upload_attempts.len(), 1);
        // Stamped when the send started, which is no earlier than the turn.
        assert!(state.recent_upload_attempts[0] >= now);
        assert!(state.recent_upload_attempts[0] < now + chrono::TimeDelta::seconds(30));
    }

    /// Only an attempt that goes on to ingest is counted: one stopped
    /// before the send, here because the session grew since it was
    /// approved, leaves the window as it was.
    #[tokio::test]
    async fn an_entry_stopped_before_the_send_is_not_counted_in_the_burst_window() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();

        let entry = session.entry_for("sha256:an-earlier-version", &cfg);
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let now = at("2026-08-08T16:00:00Z");
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let decision = up
            .upload_entry(&session.source(), &session.session_ref(), &entry, now)
            .await
            .unwrap();
        assert!(
            matches!(decision, UploadDecision::Superseded { .. }),
            "{decision:?}"
        );
        assert!(state.recent_upload_attempts.is_empty());
    }

    /// The setup the stored-envelope tests above build inline: a previewed
    /// session whose redacted envelope is on disk and pinned by its entry.
    async fn seeded_stored_envelope(
        session: &GrowingSession,
        store: &ConfigStore,
        cfg: &crate::config::ContributorConfig,
    ) -> (QueueEntry, TraceContributionEnvelope) {
        let (summary, _body, envelope) = crate::daemon::preview::build_preview(
            store,
            Some(cfg),
            None,
            &session.source(),
            &session.session_ref(),
        )
        .await
        .unwrap();
        let entry = QueueEntry {
            previewed_envelope_digest: Some(summary.envelope_digest.clone()),
            ..session.entry_for(&session.current_hash(), cfg)
        };
        crate::daemon::approved_envelope::save(store, entry.entry_id, &envelope).unwrap();
        (entry, envelope)
    }

    /// The verdict must reach the envelope that is actually sent.
    ///
    /// The daemon does not rebuild at upload time -- it sends the stored
    /// bytes -- so a verdict routed through `SubmitOptions` would pass every
    /// fresh-build test and be dropped on exactly this path. That was the
    /// original design error; this test is its regression guard.
    #[tokio::test]
    async fn a_verdict_reaches_a_stored_envelope() {
        use trace_commons_protocol::trace_contribution::TaskSuccess;

        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let (mut entry, _envelope) = seeded_stored_envelope(&session, &store, &cfg).await;
        entry.approved_verdict = Some("failed".to_string());

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let sent = up
            .approved_envelope_for(&entry)
            .expect("load succeeds")
            .expect("an envelope is stored");

        assert_eq!(sent.outcome.task_success, TaskSuccess::Failure);

        // The stored bytes themselves are untouched -- the verdict is
        // stamped on the loaded copy, not written back.
        let on_disk = crate::daemon::approved_envelope::load(&store, entry.entry_id)
            .unwrap()
            .expect("an envelope is stored");
        assert_eq!(on_disk.outcome.task_success, TaskSuccess::Unknown);
    }

    #[tokio::test]
    async fn no_verdict_leaves_a_stored_envelope_unknown() {
        use trace_commons_protocol::trace_contribution::TaskSuccess;

        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let (entry, _envelope) = seeded_stored_envelope(&session, &store, &cfg).await;
        assert_eq!(entry.approved_verdict, None);

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let sent = up
            .approved_envelope_for(&entry)
            .expect("load succeeds")
            .expect("an envelope is stored");

        assert_eq!(sent.outcome.task_success, TaskSuccess::Unknown);
    }

    /// A verdict is a judgement about the task, not content, so it must not
    /// move either consent declaration. #421 pins this for the build path
    /// (`a_verdict_declares_no_content`); this extends it to the daemon
    /// path, where the verdict is stamped onto an already-redacted envelope
    /// and could otherwise disturb flags that were derived before it
    /// arrived.
    #[tokio::test]
    async fn a_verdict_moves_neither_consent_flag_on_a_stored_envelope() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let (mut entry, stored) = seeded_stored_envelope(&session, &store, &cfg).await;
        let before = (
            stored.consent.message_text_included,
            stored.consent.tool_payloads_included,
        );
        entry.approved_verdict = Some("failed".to_string());

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let sent = up
            .approved_envelope_for(&entry)
            .expect("load succeeds")
            .expect("an envelope is stored");

        assert_eq!(
            (
                sent.consent.message_text_included,
                sent.consent.tool_payloads_included
            ),
            before,
            "a verdict is not content and must not move a consent declaration"
        );
    }

    /// The digest check guards this crate's own storage and must keep
    /// running against the bytes AS STORED. A verdict applied before it
    /// would make a tampered file pass.
    #[tokio::test]
    async fn a_tampered_stored_envelope_is_still_refused_with_a_verdict_present() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let cfg = fixture_cfg(&store);
        store.save_config(&cfg).unwrap();
        let (mut entry, _envelope) = seeded_stored_envelope(&session, &store, &cfg).await;
        entry.approved_verdict = Some("worked".to_string());
        entry.previewed_envelope_digest = Some("0".repeat(64));

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        assert!(up.approved_envelope_for(&entry).is_err());
    }
}
