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
use crate::routing::attested::{
    AttestedCall, ProviderIdentifier, Unattestable, classify_upstream_id, ledger_only_final_call,
};

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
/// No receipt exists for the call and none will. Two ways it is established,
/// and the sentence covers both: a [`ProviderIdentifier::Foreign`] identifier
/// at discovery -- the answer came back under another provider's identifier,
/// so NEAR AI never ran the enclave that would have signed it -- and a
/// definite 404 ([`ReceiptFetchError::ReceiptNotFound`]) from the receipt
/// endpoint at submission. Permanent, because nothing on this machine and no
/// setting anywhere reaches back to where those bytes were sent.
///
/// [`ReceiptFetchError::ReceiptNotFound`]: crate::routing::receipt::ReceiptFetchError::ReceiptNotFound
pub const REASON_RECEIPT_NOT_ISSUED: &str = "receipt_not_issued";

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

    /// Unknown, with no reason of this client's to offer.
    ///
    /// The `REASON_*` labels are facts about a session, established by
    /// reading it. A refusal from the commons is not one of those: what this
    /// client learned is that its previous answer no longer stands, and the
    /// reason it would have to invent belongs to a party that did not send
    /// one. `unknown` with an absent reason is already on this wire -- a row
    /// written before the field existed carries exactly this -- so every
    /// shell renders it today.
    pub(super) fn retracted() -> Self {
        Self {
            state: MARK_UNKNOWN,
            reason: None,
        }
    }

    /// Unknown, because the question has not been settled yet.
    ///
    /// The identifier the final call came back under is a shape this build
    /// cannot place -- neither the provider's own nor one it knows to be
    /// another provider's -- so whether a receipt exists is not known until
    /// the submit path asks for one. The same wire value as
    /// [`Self::retracted`], and deliberately: both mean "no answer of this
    /// client's to offer", and a shell already renders it.
    pub(super) fn unresolved() -> Self {
        Self::retracted()
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
        Ok(true) => {}
        Ok(false) => return Mark::permanent(REASON_MARKER_ABSENT),
        Err(_) => return Mark::permanent(REASON_REQUEST_MALFORMED),
    }
    // Last, and only after everything a receipt would need is in place: can a
    // receipt exist for this call at all? The bodies can be faithful and the
    // marker present and the answer still no, because NEAR AI signs only the
    // calls it served itself, and it names those by its own identifier. A
    // call it passed on to another provider carries that provider's
    // identifier and 404s at the receipt endpoint forever. The mark used to
    // read the row blind to this and told a person running Claude or GPT
    // through NEAR AI that their trace carried proof it could never carry.
    //
    // Fails toward not claiming. A shape this build does not know is not
    // refused -- the submit path still fetches, and `writeback_after_upload`
    // records what it got -- but it is not promised either.
    match classify_upstream_id(call.upstream_id()) {
        ProviderIdentifier::Hosted => Mark::attested(),
        ProviderIdentifier::Foreign => Mark::permanent(REASON_RECEIPT_NOT_ISSUED),
        ProviderIdentifier::Unrecognised => Mark::unresolved(),
    }
}

/// What the receipt fetch at submission proved about this entry's mark.
///
/// The second half of the defect. The receipt fetch used to fail into a debug
/// log while the trace shipped anyway, and the row went on saying `attested`
/// -- a person believed they had contributed attested work and had not. The
/// submission still ships (an invited contributor's upload is valid without
/// a receipt, and refusing it would withhold a whole contribution over a
/// credit weighting), but the mark now records what actually went.
///
/// - Attached: the call carried a receipt. Resolves an [`MARK_UNKNOWN`]
///   left by an unrecognised identifier; an already-attested row is
///   unchanged and an unattested one is not promoted, because the mark also
///   answers for the marker and the bodies, which a receipt says nothing
///   about.
/// - Not found: the endpoint answered and holds nothing. Permanent, for the
///   reason [`REASON_RECEIPT_NOT_ISSUED`] gives.
/// - Anything else: the endpoint was not reached, refused this machine, or
///   served something unverifiable. [`MARK_UNKNOWN`] with
///   [`REASON_RECEIPT_UNAVAILABLE`], the one reason whose sentence says it
///   may work later -- never a permanent mark from a transient failure.
///
/// `None` when nothing changes: no attested call shipped, or the receipt
/// arrived on a row that already said so.
#[must_use]
pub fn writeback_after_upload(
    shipped: crate::submit::ReceiptShipped,
    current_state: Option<&str>,
) -> Option<Mark> {
    use crate::routing::receipt::ReceiptFetchError;
    use crate::submit::ReceiptShipped;
    match shipped {
        ReceiptShipped::NoCall => None,
        ReceiptShipped::Attached => (current_state == Some(MARK_UNKNOWN)).then(Mark::attested),
        ReceiptShipped::Omitted(ReceiptFetchError::ReceiptNotFound) => {
            Some(Mark::permanent(REASON_RECEIPT_NOT_ISSUED))
        }
        ReceiptShipped::Omitted(_) => Some(Mark::unknown(REASON_RECEIPT_UNAVAILABLE)),
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
/// `None` for every other refusal label. A daily cap, an unreachable server,
/// a spent admission budget and a filter outage say nothing whatever about
/// what a session carries.
#[must_use]
pub fn writeback_for(reason_label: &str) -> Option<Mark> {
    use trace_commons_protocol::admission::AdmissionRefusal;
    match reason_label {
        "admission_request_malformed" => Some(Mark::permanent(REASON_REQUEST_MALFORMED)),
        "admission_receipt_unavailable" => Some(Mark::unknown(REASON_RECEIPT_UNAVAILABLE)),
        "admission_receipt_not_issued" => Some(Mark::permanent(REASON_RECEIPT_NOT_ISSUED)),
        // The two above are this client's own answers about bytes it read.
        // These two are the server's, about a send it turned away for want of
        // the proof -- and the row must stop claiming to carry any.
        other
            if matches!(
                AdmissionRefusal::from_label(other),
                Some(AdmissionRefusal::Refused | AdmissionRefusal::EvidenceRefused)
            ) =>
        {
            Some(Mark::retracted())
        }
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
pub const ALL_REASONS: [&str; 14] = [
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
    REASON_RECEIPT_NOT_ISSUED,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::contribution_eligibility as ce;
    use chrono::{TimeZone, Utc};

    /// A request carrying the marker, built through the protocol constant
    /// rather than spelled out, so a rename moves both sides at once.
    fn marked_request() -> String {
        serde_json::json!({
            "model": "a-model",
            "metadata": {
                trace_commons_protocol::admission::REQUEST_METADATA_KEY: "opaque",
            },
        })
        .to_string()
    }

    fn unmarked_request() -> String {
        serde_json::json!({"model": "a-model"}).to_string()
    }

    fn row() -> RoutedExchange {
        RoutedExchange {
            id: Some(1),
            started_at: Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap(),
            client_session_id: Some("session".to_string()),
            total_ms: Some(10),
            facade: "openai".to_string(),
            backend: "nearai".to_string(),
            requested_model: Some("a-model".to_string()),
            served_model: Some("a-model".to_string()),
            upstream_id: Some("abcdef0123456789".to_string()),
            request_sha256: Some("00".repeat(32)),
            response_sha256: Some("11".repeat(32)),
            body_ref: Some("00000000000000000001-000000".to_string()),
            rung: "full".to_string(),
            attempts: 1,
            input_tokens: Some(1),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(1),
            cost_usd: Some(0.0),
            status: 200,
        }
    }

    /// Builds a real [`AttestedCall`] over `request`, through the same
    /// function the submit path uses.
    fn call_with(request: &str) -> (AttestedCall, tempfile::TempDir) {
        use sha2::{Digest as _, Sha256};
        let response = "data: [DONE]\n\n";
        let mut row = row();
        let reference = row.body_ref.clone().expect("a reference");
        row.request_sha256 = Some(format!("{:x}", Sha256::digest(request.as_bytes())));
        row.response_sha256 = Some(format!("{:x}", Sha256::digest(response.as_bytes())));
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(format!("{reference}.req")), request).expect("req");
        std::fs::write(dir.path().join(format!("{reference}.res")), response).expect("res");
        let call = crate::routing::attested::attested_final_call(&[row], dir.path())
            .expect("the fixture is attestable");
        (call, dir)
    }

    /// Every input this module classifies, once, so the tests below and the
    /// cross-check against eligibility run over the same matrix rather than
    /// two hand-kept lists that could drift apart.
    fn every_input() -> Vec<(Vec<RoutedExchange>, Option<Unattestable>, Option<String>)> {
        let mut no_body = row();
        no_body.body_ref = None;
        let mut cases: Vec<(Vec<RoutedExchange>, Option<Unattestable>, Option<String>)> = vec![
            (vec![row()], None, Some(marked_request())),
            (vec![row()], None, Some(unmarked_request())),
            (vec![row()], None, Some("{".to_string())),
            (vec![], None, None),
            (vec![no_body], None, None),
            (vec![row()], None, None),
        ];
        for refusal in [
            Unattestable::NoCall,
            Unattestable::CaptureOff,
            Unattestable::DigestAbsent,
            Unattestable::UpstreamIdAbsent,
            Unattestable::DigestMismatch,
            Unattestable::ReferenceMalformed,
            Unattestable::BodiesUnreadable,
            Unattestable::BodyNotUtf8,
            Unattestable::BodyTooLarge,
        ] {
            cases.push((vec![row()], Some(refusal), None));
        }
        cases
    }

    /// Runs one case of [`every_input`] through both entry points, holding
    /// the built call alive for the length of the call.
    fn both_answers(
        case: &(Vec<RoutedExchange>, Option<Unattestable>, Option<String>),
        admission_evidence: bool,
    ) -> (Mark, Option<ce::Verdict>) {
        let (rows, refusal, request) = case;
        let held = request.as_deref().map(call_with);
        let call = held.as_ref().map(|(c, _dir)| c);
        (
            evaluate(rows, call, *refusal),
            ce::evaluate(admission_evidence, rows, call, *refusal),
        )
    }

    /// The whole point. An invited contributor's queue answers nothing about
    /// eligibility -- and still answers this, for every input.
    #[test]
    fn the_mark_is_answered_with_the_signup_flag_off() {
        for case in every_input() {
            let (mark, verdict) = both_answers(&case, false);
            assert_eq!(verdict, None, "eligibility spoke with the flag off");
            assert!(
                ALL_MARKS.contains(&mark.state),
                "{} is not a pinned mark",
                mark.state
            );
        }
    }

    /// A session that carries its call says so, rather than only failing to
    /// complain. The positive case is the one that matters here, and it is
    /// the case the eligibility surface never had to render.
    #[test]
    fn a_marked_final_call_is_attested() {
        let (call, _dir) = call_with(&marked_request());
        assert_eq!(
            evaluate(&[row()], Some(&call), None),
            Mark {
                state: MARK_ATTESTED,
                reason: None,
            }
        );
    }

    /// The case the surface exists for, and the commonest one: history
    /// recorded before the contributor routed anything through attested
    /// inference at all. It is a description, not a refusal.
    #[test]
    fn a_session_with_no_inference_hops_is_permanently_unattested() {
        assert_eq!(
            evaluate(&[], None, None),
            Mark {
                state: MARK_UNATTESTED_PERMANENT,
                reason: Some(REASON_NO_CALL),
            }
        );
    }

    /// The two configuration answers: one about the record, one about the
    /// machine. Both name something a contributor can change for future
    /// sessions, which is why they are not permanent.
    #[test]
    fn the_two_configuration_answers_keep_their_own_reasons() {
        let mut no_body = row();
        no_body.body_ref = None;
        assert_eq!(
            evaluate(&[no_body], None, None),
            Mark {
                state: MARK_UNATTESTED_CONFIGURATION,
                reason: Some(REASON_CAPTURE_OFF),
            }
        );
        assert_eq!(
            evaluate(&[row()], None, None),
            Mark {
                state: MARK_UNATTESTED_CONFIGURATION,
                reason: Some(REASON_EVIDENCE_CAPTURE_OFF),
            }
        );
    }

    /// A call carried whole whose request never carried the marker. The
    /// marker is what selects the profile that carries these bodies to a
    /// witness at all, so without it nothing could attest the call.
    #[test]
    fn an_unmarked_final_call_is_permanently_unattested() {
        let (call, _dir) = call_with(&unmarked_request());
        assert_eq!(
            evaluate(&[row()], Some(&call), None),
            Mark {
                state: MARK_UNATTESTED_PERMANENT,
                reason: Some(REASON_MARKER_ABSENT),
            }
        );
    }

    /// Every refusal keeps its own name rather than collapsing into one
    /// "could not carry it". A contributor does something different about
    /// each, and the reason label is the only place the distinction survives.
    #[test]
    fn every_refusal_keeps_its_own_reason() {
        let cases = [
            (Unattestable::DigestMismatch, REASON_DIGEST_MISMATCH),
            (Unattestable::ReferenceMalformed, REASON_REFERENCE_MALFORMED),
            (Unattestable::BodiesUnreadable, REASON_BODIES_UNREADABLE),
            (Unattestable::BodyNotUtf8, REASON_BODY_NOT_UTF8),
            (Unattestable::BodyTooLarge, REASON_BODY_TOO_LARGE),
        ];
        for (refusal, reason) in cases {
            assert_eq!(
                evaluate(&[row()], None, Some(refusal)),
                Mark {
                    state: MARK_UNATTESTED_PERMANENT,
                    reason: Some(reason),
                },
                "{refusal:?} lost its reason"
            );
        }
    }

    /// The cheap group outranks a recorded refusal, so the answer a list can
    /// always produce is the one it produces.
    #[test]
    fn the_cheap_group_is_consulted_first() {
        assert_eq!(
            evaluate(&[], None, Some(Unattestable::BodyTooLarge)),
            Mark {
                state: MARK_UNATTESTED_PERMANENT,
                reason: Some(REASON_NO_CALL),
            }
        );
    }

    /// **The mutation proof for the refactor.** Eligibility is now derived
    /// from the mark rather than classified separately, and this pins that
    /// the derivation is a rename of four labels and nothing else: same
    /// partition, same reason, for every input, in both framings.
    ///
    /// A change that made eligibility decide anything differently -- for
    /// anyone, on any input -- has to break this or the literal expectations
    /// in `contribution_eligibility`'s own tests.
    #[test]
    fn eligibility_is_this_answer_renamed_and_nothing_else() {
        for case in every_input() {
            let (mark, verdict) = both_answers(&case, true);
            let verdict = verdict.expect("eligibility answers while the flag is on");
            let expected_state = match mark.state {
                MARK_ATTESTED => ce::STATE_ELIGIBLE,
                MARK_UNATTESTED_PERMANENT => ce::STATE_INELIGIBLE_PERMANENT,
                MARK_UNATTESTED_CONFIGURATION => ce::STATE_INELIGIBLE_CONFIGURATION,
                _ => ce::STATE_UNKNOWN,
            };
            assert_eq!(
                verdict.state, expected_state,
                "{} renamed wrong",
                mark.state
            );
            assert_eq!(verdict.reason, mark.reason, "the reason did not travel");
        }
    }

    /// The server's own refusals, on the same rule.
    ///
    /// Before this, `writeback_for` knew only the two labels this client
    /// raises itself, so a row went on saying a session carried proof after
    /// the commons had turned that session away for want of it -- with the
    /// contributor's own attempt as the evidence against the row.
    ///
    /// The retraction carries no reason. The `REASON_*` vocabulary is a
    /// vocabulary of facts about a session, established by reading it, and a
    /// refusal from somewhere else is not one of those: what the client
    /// learned is that it no longer knows, which is exactly what `unknown`
    /// with no reason already says on this wire.
    #[test]
    fn a_server_admission_refusal_retracts_the_mark() {
        use trace_commons_protocol::admission::AdmissionRefusal;
        for refusal in [AdmissionRefusal::Refused, AdmissionRefusal::EvidenceRefused] {
            assert_eq!(
                writeback_for(refusal.label()),
                Some(Mark {
                    state: MARK_UNKNOWN,
                    reason: None,
                }),
                "{} left the row claiming its proof",
                refusal.label()
            );
        }
        // A spent budget, a held lease and a submission-id collision are
        // facts about a request, not about what the session carries.
        for refusal in [
            AdmissionRefusal::LimitReached,
            AdmissionRefusal::InProgress,
            AdmissionRefusal::IdentityConflict,
        ] {
            assert_eq!(
                writeback_for(refusal.label()),
                None,
                "{} invented an answer about the session",
                refusal.label()
            );
        }
    }

    /// The same rule on the write-back side: a submission turned away for
    /// want of the proof leaves the row unable to claim it carries any.
    #[test]
    fn an_admission_refusal_retracts_the_mark() {
        assert_eq!(
            writeback_for("admission_request_malformed"),
            Some(Mark {
                state: MARK_UNATTESTED_PERMANENT,
                reason: Some(REASON_REQUEST_MALFORMED),
            })
        );
        assert_eq!(
            writeback_for("admission_receipt_unavailable"),
            Some(Mark {
                state: MARK_UNKNOWN,
                reason: Some(REASON_RECEIPT_UNAVAILABLE),
            })
        );
        for label in [
            "upload-failed",
            "claim-mint-failed",
            "pii-filter-unavailable",
            "witness-review-stale",
            "parse-failed",
            "",
        ] {
            assert_eq!(writeback_for(label), None, "{label} rewrote the row");
        }
    }

    /// Nothing this module produces is outside the pinned sets. The copy
    /// tables and the wire contract are both written against them.
    #[test]
    fn every_mark_uses_a_pinned_label() {
        for case in every_input() {
            let (mark, _) = both_answers(&case, true);
            assert!(
                ALL_MARKS.contains(&mark.state),
                "{} is not a pinned mark",
                mark.state
            );
            if let Some(reason) = mark.reason {
                assert!(
                    ALL_REASONS.contains(&reason),
                    "{reason} is not a pinned reason"
                );
            }
        }
    }

    /// `unknown` ships in the contract without this slice producing it from
    /// a classification, exactly as its eligibility twin does: lazy
    /// evaluation is the obvious later optimisation, and a shell that has
    /// never seen the label will render it wrong on the day it first
    /// arrives.
    #[test]
    fn nothing_this_slice_classifies_answers_unknown() {
        for case in every_input() {
            let (mark, _) = both_answers(&case, true);
            assert_ne!(mark.state, MARK_UNKNOWN);
        }
    }

    /// The discovery path, end to end, for one ledger row: bodies on disk,
    /// the full check run exactly as `RoutingEnrichedSource::load` runs it,
    /// and the mark read off whichever half it produced.
    fn discovered_mark(row: RoutedExchange) -> Mark {
        use sha2::{Digest as _, Sha256};
        let request = marked_request();
        let response = "data: [DONE]\n\n";
        let mut row = row;
        let reference = row.body_ref.clone().expect("a reference");
        row.request_sha256 = Some(format!("{:x}", Sha256::digest(request.as_bytes())));
        row.response_sha256 = Some(format!("{:x}", Sha256::digest(response.as_bytes())));
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(format!("{reference}.req")), &request).expect("req");
        std::fs::write(dir.path().join(format!("{reference}.res")), response).expect("res");
        let rows = vec![row];
        let (call, refusal) = match crate::routing::attested::attested_final_call(&rows, dir.path())
        {
            Ok(call) => (Some(call), None),
            Err(refusal) => (None, Some(refusal)),
        };
        evaluate(&rows, call.as_ref(), refusal)
    }

    /// The defect this exists to remove. NEAR AI answers
    /// `GET /v1/signature/{chat_id}` only for calls it served from its own
    /// enclave, and those carry its own bare-hex identifier. A call it passed
    /// on to Anthropic or OpenAI comes back under THAT provider's identifier
    /// -- `msg_…`, `chatcmpl-…` -- and 404s forever. The mark used to read
    /// the row blind to this, and a person running Claude or GPT through
    /// NEAR AI was told their trace carried proof it could never carry.
    #[test]
    fn a_call_answered_under_another_providers_identifier_is_never_attested() {
        for foreign in ["msg_011CejCasYpbXmB5Zqu3fJsc", "chatcmpl-9x2Fz8Q"] {
            let mut row = row();
            row.upstream_id = Some(foreign.to_string());
            let mark = discovered_mark(row);
            assert_ne!(
                mark.state, MARK_ATTESTED,
                "{foreign}: a call NEAR AI did not serve was marked attested"
            );
        }
    }

    /// The positive control for the test above: the same row under NEAR AI's
    /// own identifier IS attested, so the refusal is about the identifier and
    /// not about the fixture.
    #[test]
    fn the_same_call_under_the_providers_own_identifier_is_attested() {
        let mut row = row();
        row.upstream_id = Some("ee64b242d74f4c7eb59b05b046f33f7b".to_string());
        assert_eq!(discovered_mark(row).state, MARK_ATTESTED);
    }
}
