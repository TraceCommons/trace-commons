//! Does this session carry proof of the model call that produced it?
//!
//! # Two questions, one classification
//!
//! `contribution_eligibility` answers *may I send this?* -- a permission
//! question, and one an invited contributor does not have. It is correctly
//! silent for them. Underneath it, though, is a second question that every
//! contributor has, all of the time:
//!
//! > Does this trace carry a verbatim copy of the model call it came from,
//! > of the kind a witness can check a receipt against?
//!
//! That is a fact about the **trace**, not about the contributor's
//! permission, and nothing about an invite makes it not worth knowing. It is
//! about to stop being provenance metadata: the credit scoring function is
//! expected to weight attestations, and once it does, a contributor who
//! cannot see which of their sessions carry one cannot act on it. Withholding
//! that from an invited contributor -- while showing the very same
//! classification to an evidence-admitted one under a different name -- is
//! the defect this module removes.
//!
//! # Why this module owns the classification
//!
//! [`evaluate`] here is the whole classifier, and
//! `contribution_eligibility::evaluate` is a thin derivation from it, gated
//! on the signup flag. They cannot drift, because there is only one of them.
//! Running the classification twice, once per framing, is how the two would
//! come to disagree about the same session -- and a queue that says a trace
//! carries proof while refusing to send it for want of that proof is worse
//! than either answer alone.
//!
//! # Its own states, the shared reasons
//!
//! The `STATE_*` labels next door read as refusals -- `ineligible_permanent`
//! is a "no" to a request. For a contributor who was invited, nothing was
//! requested and nothing was refused: their session simply has no attached
//! call, and a row telling them they are ineligible would be answering a
//! question they never asked. So the marks below are their own vocabulary,
//! partitioning the same facts under words that are a **description** rather
//! than a verdict.
//!
//! The `REASON_*` labels are not split, because they never were verdicts.
//! Each one names a property of the recorded session -- no call was made, the
//! copy is incomplete, the digest disagrees -- and those facts are the same
//! fact whichever question is being asked of them. They live here and
//! `contribution_eligibility` re-exports them, so a rename moves one place.
//! Their *sentences* are separate, and deliberately: the eligibility copy
//! says "cannot be sent", which is false for an invited contributor whose
//! session sends perfectly well and merely arrives unattested.
//!
//! Nothing here runs a check. Both halves were already paid for by the load
//! that built the transcript this entry came from; this reads their results.
//! Label-only throughout: every value produced is a fixed string carrying no
//! path, digest, identifier or anything a contributor wrote.

use crate::routing::RoutedExchange;
use crate::routing::attested::{AttestedCall, Unattestable, ledger_only_final_call};

/// The session carries a faithful copy of its final model call, marked for
/// the receipt that goes with it.
pub const MARK_ATTESTED: &str = "attested";
/// The session carries no such copy, and nothing the contributor does will
/// change that -- the bytes it would have needed were sent long ago.
pub const MARK_UNATTESTED_PERMANENT: &str = "unattested_permanent";
/// The session carries no such copy, and a setting decides whether the ones
/// recorded from now on will. The only mark whose sentence names a setting.
pub const MARK_UNATTESTED_CONFIGURATION: &str = "unattested_configuration";
/// Not worked out. Never a silent stand-in for "no": an entry written before
/// this field existed answers this, and degrading it into an unattested mark
/// would tell a contributor something false about their own work.
pub const MARK_UNKNOWN: &str = "unknown";

/// [`Unattestable::NoCall`]: the session joined no inference hops.
pub const REASON_NO_CALL: &str = "no_inference_call";
/// [`Unattestable::CaptureOff`]: the final hop recorded no bodies.
pub const REASON_CAPTURE_OFF: &str = "capture_off";
/// [`Unattestable::DigestAbsent`]: a restarted, cancelled or truncated stream.
pub const REASON_DIGEST_ABSENT: &str = "digest_absent";
/// [`Unattestable::UpstreamIdAbsent`]: no provider identifier, so no receipt.
pub const REASON_UPSTREAM_ID_ABSENT: &str = "upstream_id_absent";
/// [`Unattestable::DigestMismatch`]: the bytes disagree with the record.
pub const REASON_DIGEST_MISMATCH: &str = "digest_mismatch";
/// [`Unattestable::ReferenceMalformed`]: the stored reference is not one the
/// store could have written.
pub const REASON_REFERENCE_MALFORMED: &str = "reference_malformed";
/// [`Unattestable::BodiesUnreadable`]: named by the record, not readable.
pub const REASON_BODIES_UNREADABLE: &str = "bodies_unreadable";
/// [`Unattestable::BodyNotUtf8`]: no faithful representation in the carrier.
pub const REASON_BODY_NOT_UTF8: &str = "body_not_utf8";
/// [`Unattestable::BodyTooLarge`]: past the carried-body bound.
pub const REASON_BODY_TOO_LARGE: &str = "body_too_large";
/// No verbatim body store is configured, so no session on this machine can
/// carry the evidence. A setting, and the only reason that is about the
/// machine rather than about this session.
pub const REASON_EVIDENCE_CAPTURE_OFF: &str = "evidence_capture_off";
/// The call was carried whole and its request does not carry the marker the
/// receipt is fetched against. Recorded before the marker existed, or made by
/// a tool that does not add it.
pub const REASON_MARKER_ABSENT: &str = "marker_absent";
/// The call carries the marker, and the receipt that has to accompany it
/// could not be obtained. Not a fact about this session: the receipt is
/// fetched from elsewhere and elsewhere can be down, which is why it answers
/// [`MARK_UNKNOWN`] rather than an unattested mark.
pub const REASON_RECEIPT_UNAVAILABLE: &str = "receipt_unavailable";
/// The request could not be read as the shape the marker lives in. Refused
/// rather than retried, the same rule `submit::admission_profile_for_request`
/// follows.
pub const REASON_REQUEST_MALFORMED: &str = "request_malformed";

/// One session's attestation answer: a mark and, unless it is attested, why
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// One of the four `MARK_*` labels.
    pub state: &'static str,
    /// One of the `REASON_*` labels. `None` for [`MARK_ATTESTED`], which has
    /// nothing to explain.
    pub reason: Option<&'static str>,
}

impl Mark {
    pub(super) fn attested() -> Self {
        Self {
            state: MARK_ATTESTED,
            reason: None,
        }
    }

    pub(super) fn permanent(reason: &'static str) -> Self {
        Self {
            state: MARK_UNATTESTED_PERMANENT,
            reason: Some(reason),
        }
    }

    pub(super) fn configuration(reason: &'static str) -> Self {
        Self {
            state: MARK_UNATTESTED_CONFIGURATION,
            reason: Some(reason),
        }
    }

    pub(super) fn unknown(reason: &'static str) -> Self {
        Self {
            state: MARK_UNKNOWN,
            reason: Some(reason),
        }
    }
}

/// Classify one session's attestation, for every contributor.
///
/// **No flag, and no `Option`.** Every session has an answer to this and it
/// is always reported; the only thing an absent field could mean here is a
/// build too old to have one.
///
/// `rows` are the ledger hops joined to this session, `attested` the pair the
/// overlay carried, and `refusal` what the overlay's own full check said when
/// it carried nothing. `attested` being `None` beside a `None` `refusal`
/// means no body store is configured, which is [`REASON_EVIDENCE_CAPTURE_OFF`]
/// -- a fact about the machine, and the one a contributor can act on.
#[must_use]
pub fn evaluate(
    rows: &[RoutedExchange],
    attested: Option<&AttestedCall>,
    refusal: Option<Unattestable>,
) -> Mark {
    // The cheap group first, and from the rows rather than from `refusal`.
    // These four are the answers that stay available if the expensive half is
    // ever made lazy, and they contain the case that actually matters --
    // `NoCall`, the session that predates attested inference entirely.
    if let Err(cheap) = ledger_only_final_call(rows) {
        return mark_for(cheap);
    }
    if let Some(refusal) = refusal {
        return mark_for(refusal);
    }
    let Some(call) = attested else {
        // The row is usable and nothing refused it, so nothing ran: this
        // machine keeps no verbatim bodies. Configuration, and the sentence
        // may name the setting.
        return Mark::configuration(REASON_EVIDENCE_CAPTURE_OFF);
    };
    // The same call the submit path makes, over the same bytes. The marker is
    // what selects the profile that carries these bodies to a witness at all
    // (`submit::witness_input_for_profile`), so a call recorded without it
    // reaches nobody who could attest it -- which is why it is part of this
    // answer and not only of the permission one.
    match crate::submit::admission_profile_for_request(true, Some(call.request_body())) {
        Ok(true) => Mark::attested(),
        Ok(false) => Mark::permanent(REASON_MARKER_ABSENT),
        Err(_) => Mark::permanent(REASON_REQUEST_MALFORMED),
    }
}

/// The mark and reason for one refusal.
///
/// [`Unattestable::CaptureOff`] is the single configuration case: the record
/// exists and holds no bodies, which is a setting on the thing that wrote it.
/// Every other variant is a fact about bytes that were already sent, or about
/// bytes already on disk, and no setting reaches back into either.
fn mark_for(refusal: Unattestable) -> Mark {
    match refusal {
        Unattestable::CaptureOff => Mark::configuration(REASON_CAPTURE_OFF),
        Unattestable::NoCall => Mark::permanent(REASON_NO_CALL),
        Unattestable::DigestAbsent => Mark::permanent(REASON_DIGEST_ABSENT),
        Unattestable::UpstreamIdAbsent => Mark::permanent(REASON_UPSTREAM_ID_ABSENT),
        Unattestable::DigestMismatch => Mark::permanent(REASON_DIGEST_MISMATCH),
        Unattestable::ReferenceMalformed => Mark::permanent(REASON_REFERENCE_MALFORMED),
        Unattestable::BodiesUnreadable => Mark::permanent(REASON_BODIES_UNREADABLE),
        Unattestable::BodyNotUtf8 => Mark::permanent(REASON_BODY_NOT_UTF8),
        Unattestable::BodyTooLarge => Mark::permanent(REASON_BODY_TOO_LARGE),
    }
}

/// What a refused submission proved about this entry's attestation.
///
/// The mirror of `contribution_eligibility::writeback_for`, and it exists for
/// the same reason: a row that goes on saying a session carries proof, after
/// a send of that session was turned away for want of that proof, reproduces
/// the defect this surface removes -- now with the contributor's own attempt
/// as the evidence against it.
///
/// `None` for every other refusal label. A daily cap, an unreachable server
/// and a filter outage say nothing whatever about what a session carries.
#[must_use]
pub fn writeback_for(reason_label: &str) -> Option<Mark> {
    match reason_label {
        "admission_request_malformed" => Some(Mark::permanent(REASON_REQUEST_MALFORMED)),
        "admission_receipt_unavailable" => Some(Mark::unknown(REASON_RECEIPT_UNAVAILABLE)),
        _ => None,
    }
}

/// Every mark this module can produce, for tests that pin the set.
pub const ALL_MARKS: [&str; 4] = [
    MARK_ATTESTED,
    MARK_UNATTESTED_PERMANENT,
    MARK_UNATTESTED_CONFIGURATION,
    MARK_UNKNOWN,
];

/// Every reason label this module can produce, for tests that pin the set.
///
/// Shared with `contribution_eligibility`, which re-exports it: the reasons
/// are facts about a session, not answers to either question.
pub const ALL_REASONS: [&str; 13] = [
    REASON_NO_CALL,
    REASON_CAPTURE_OFF,
    REASON_DIGEST_ABSENT,
    REASON_UPSTREAM_ID_ABSENT,
    REASON_DIGEST_MISMATCH,
    REASON_REFERENCE_MALFORMED,
    REASON_BODIES_UNREADABLE,
    REASON_BODY_NOT_UTF8,
    REASON_BODY_TOO_LARGE,
    REASON_EVIDENCE_CAPTURE_OFF,
    REASON_MARKER_ABSENT,
    REASON_REQUEST_MALFORMED,
    REASON_RECEIPT_UNAVAILABLE,
];
