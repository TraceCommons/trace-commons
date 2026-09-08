//! Can this pending session actually be contributed?
//!
//! # The surface this exists for
//!
//! `list_pending` used to return the whole queue with nothing on an entry
//! that could support an eligibility filter. Eligibility was decided per
//! submission, at submit time, from whether that session's attested final
//! call carried the admission request marker. So a contributor admitted on
//! evidence rather than an invite saw every session on their computer, picked
//! one, sent it, and learned only then that it was inadmissible. The list
//! looked like a menu and was not one.
//!
//! An invited contributor never sees any of this, because everything in their
//! queue is contributable. That is why it stayed invisible, and it is why
//! [`evaluate`] answers `None` -- an absent field, not a state -- when
//! `admission_evidence` is off. A contributor with an invite has no
//! eligibility question, and a field answering one they do not have would
//! invite three shells to render an answer to it.
//!
//! # Two costs, not one
//!
//! [`Unattestable`] splits along exactly the line this surface needs. Four
//! variants are answerable from a ledger row already in memory; five need
//! every captured body read off disk and hashed. A list may run the first
//! group and may not run the second.
//!
//! Nothing here runs either. Both were already paid for by the load that
//! built the transcript this entry came from: the overlay joins the ledger
//! rows and, when a body store is configured, runs the full check once and
//! keeps its answer (`SessionTranscript::attested_refusal`). This module
//! reads those results. [`ledger_only_final_call`] is still the primary
//! classifier, because it is the one that stays answerable if the expensive
//! half is ever made lazy.
//!
//! # Permanent, provisional, or configuration
//!
//! The distinction that matters to a contributor is not eligible/ineligible.
//! It is whether anything they could do would change the answer:
//!
//! - [`STATE_INELIGIBLE_PERMANENT`] -- nothing will. Retrying is wasted work
//!   and the sentence says so.
//! - [`STATE_INELIGIBLE_CONFIGURATION`] -- this session stays ineligible, but
//!   a setting governs whether *future* ones will be. The only state whose
//!   sentence names a setting; a row stays about its own session, and advice
//!   about the next one is guidance rather than status.
//! - [`STATE_UNKNOWN`] -- not evaluated. Distinct from ineligible for the
//!   reason `credential_unreported` is distinct from `credential_absent`:
//!   degrading "could not tell" into "no" invites a contributor to conclude
//!   something false about their own work.
//!
//! # What `eligible` does and does not promise
//!
//! [`STATE_ELIGIBLE`] is a well-founded expectation, never a guarantee. The
//! server decides admission and this reports the existing rule earlier; it
//! does not alter it. If this client's answer and the server's decision
//! disagree, the server is right and the optimism here was the bug -- which
//! is what the write-back on the queue entry is for.
//!
//! Label-only throughout. Every value this module produces is a fixed string
//! from the set below; none of them carries a path, a digest, an identifier
//! or anything a contributor wrote.

use crate::routing::RoutedExchange;
use crate::routing::attested::{AttestedCall, Unattestable, ledger_only_final_call};

/// The cheap checks pass, a faithful pair was carried, and its request
/// carries the admission marker.
pub const STATE_ELIGIBLE: &str = "eligible";
/// Nothing the contributor does will make this session contributable.
pub const STATE_INELIGIBLE_PERMANENT: &str = "ineligible_permanent";
/// This session stays ineligible; a setting decides whether future ones are.
pub const STATE_INELIGIBLE_CONFIGURATION: &str = "ineligible_configuration";
/// Not evaluated. Never a silent default -- see the module docs.
pub const STATE_UNKNOWN: &str = "unknown";

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
/// server admits on. Recorded before the marker existed, or made by a tool
/// that does not add it.
pub const REASON_MARKER_ABSENT: &str = "marker_absent";
/// The call carries the marker, and the receipt that has to accompany it
/// could not be obtained. Not a fact about this session: the receipt is
/// fetched from elsewhere and elsewhere can be down, which is why it answers
/// [`STATE_UNKNOWN`] rather than an ineligibility.
pub const REASON_RECEIPT_UNAVAILABLE: &str = "receipt_unavailable";
/// The request could not be read as the shape the marker lives in. Refused
/// rather than retried as an ordinary submission -- the same rule
/// `submit::admission_profile_for_request` follows.
pub const REASON_REQUEST_MALFORMED: &str = "request_malformed";

/// One entry's answer: a state label and, unless it is eligible, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// One of the four `STATE_*` labels.
    pub state: &'static str,
    /// One of the `REASON_*` labels. `None` for [`STATE_ELIGIBLE`], which has
    /// nothing to explain.
    pub reason: Option<&'static str>,
}

impl Verdict {
    fn eligible() -> Self {
        Self {
            state: STATE_ELIGIBLE,
            reason: None,
        }
    }

    fn permanent(reason: &'static str) -> Self {
        Self {
            state: STATE_INELIGIBLE_PERMANENT,
            reason: Some(reason),
        }
    }

    fn configuration(reason: &'static str) -> Self {
        Self {
            state: STATE_INELIGIBLE_CONFIGURATION,
            reason: Some(reason),
        }
    }

    fn unknown(reason: &'static str) -> Self {
        Self {
            state: STATE_UNKNOWN,
            reason: Some(reason),
        }
    }
}

/// What a refused submission proved about the entry it was sent for.
///
/// **The row must stop claiming `eligible`.** A row that goes on saying a
/// session can be sent, after a send of that session was refused for an
/// admission reason, is the exact defect this surface exists to remove --
/// reproduced one layer up, and now with the contributor's own attempt as the
/// evidence against it.
///
/// `None` for every other refusal label, and that is the common case: a
/// daily cap, an unreachable server, a stale approval and a filter outage all
/// say nothing whatever about whether this session is admissible, and
/// rewriting the row from one of them would be inventing an answer.
///
/// The two that do say something say different things.
/// `admission_request_malformed` is about bytes that were sent long ago and
/// cannot change: permanent. `admission_receipt_unavailable` is about a
/// receipt fetched from a service that can be down, so it retracts the
/// `eligible` claim without replacing it with a "no" -- see
/// [`REASON_RECEIPT_UNAVAILABLE`]. This is the one path in this slice that
/// produces [`STATE_UNKNOWN`], and it is why the label ships in the contract.
#[must_use]
pub fn writeback_for(reason_label: &str) -> Option<Verdict> {
    match reason_label {
        "admission_request_malformed" => Some(Verdict::permanent(REASON_REQUEST_MALFORMED)),
        "admission_receipt_unavailable" => Some(Verdict::unknown(REASON_RECEIPT_UNAVAILABLE)),
        _ => None,
    }
}

/// Whether the eligibility question applies to this contributor at all.
///
/// **One rule, two callers, deliberately.** `entry_value` decides from this
/// whether the wire carries an `eligibility` field, and `handle_approve`
/// decides from this whether a group-level approve filters. If those could
/// disagree, a shell would be shown seven sendable rows and told three were
/// sent, with nothing in the response explaining the other four -- which is
/// the defect this surface exists to remove, wearing a different hat.
///
/// A config that could not be read answers `false`: the wire suppresses the
/// field in that case, so a queue the daemon declined to describe is one it
/// must not silently filter either.
#[must_use]
pub fn evidence_flag(cfg: Option<&crate::config::ContributorConfig>) -> bool {
    cfg.and_then(|c| c.witness.as_ref())
        .is_some_and(|w| w.admission_evidence)
}

/// Whether a **group-level** submit may include this entry.
///
/// A group-level submit means "all eligible", never "all". The alternative
/// either fails partway or succeeds at sending exactly what the surface just
/// finished saying could not be sent -- and it is not reachable by the
/// per-row gate at all, because a per-project approve has no row to check.
/// The gate is the invariant; a disabled control is only how it is usually
/// expressed.
///
/// A row with no recorded eligibility is excluded. It renders as `unknown`,
/// which offers no control, so including it in a bulk send would send
/// precisely the row a shell was told not to offer.
///
/// This is NOT applied to a single-entry approve. Naming one entry is an
/// explicit act about a session the contributor is looking at, the shell's
/// per-row gate already covers it, and the server decides admission either
/// way. A daemon that refused a named entry would be enforcing an
/// expectation as though it were the answer.
#[must_use]
pub fn contributable_in_a_group(eligibility: Option<&str>) -> bool {
    eligibility == Some(STATE_ELIGIBLE)
}

/// Classify one session.
///
/// `admission_evidence` is `WitnessSettings::admission_evidence` -- the signup
/// flag. `rows` are the ledger hops joined to this session, `attested` the
/// pair the overlay carried, and `refusal` what the overlay's own full check
/// said when it carried nothing.
///
/// **`None` means the field is absent from the wire**, not that the answer is
/// unknown. It is returned for exactly one input: the flag being off.
///
/// `attested` being `None` beside a `None` `refusal` means no body store is
/// configured, which is [`REASON_EVIDENCE_CAPTURE_OFF`] -- a fact about the
/// machine, and the reason a contributor can act on.
#[must_use]
pub fn evaluate(
    admission_evidence: bool,
    rows: &[RoutedExchange],
    attested: Option<&AttestedCall>,
    refusal: Option<Unattestable>,
) -> Option<Verdict> {
    if !admission_evidence {
        return None;
    }
    // The cheap group first, and from the rows rather than from `refusal`.
    // These four are the answers that stay available if the expensive half is
    // ever made lazy, and they contain the case that actually matters --
    // `NoCall`, the session that predates attested inference entirely.
    if let Err(cheap) = ledger_only_final_call(rows) {
        return Some(verdict_for(cheap));
    }
    if let Some(refusal) = refusal {
        return Some(verdict_for(refusal));
    }
    let Some(call) = attested else {
        // The row is usable and nothing refused it, so nothing ran: this
        // machine keeps no verbatim bodies. Configuration, and the sentence
        // may name the setting.
        return Some(Verdict::configuration(REASON_EVIDENCE_CAPTURE_OFF));
    };
    // The same call the submit path makes, over the same bytes. Divergence
    // here is the defect this surface exists to remove.
    match crate::submit::admission_profile_for_request(true, Some(call.request_body())) {
        Ok(true) => Some(Verdict::eligible()),
        Ok(false) => Some(Verdict::permanent(REASON_MARKER_ABSENT)),
        Err(_) => Some(Verdict::permanent(REASON_REQUEST_MALFORMED)),
    }
}

/// The state and reason for one refusal.
///
/// [`Unattestable::CaptureOff`] is the single configuration case: the record
/// exists and holds no bodies, which is a setting on the thing that wrote it.
/// Every other variant is a fact about bytes that were already sent, or about
/// bytes already on disk, and no setting reaches back into either.
fn verdict_for(refusal: Unattestable) -> Verdict {
    match refusal {
        Unattestable::CaptureOff => Verdict::configuration(REASON_CAPTURE_OFF),
        Unattestable::NoCall => Verdict::permanent(REASON_NO_CALL),
        Unattestable::DigestAbsent => Verdict::permanent(REASON_DIGEST_ABSENT),
        Unattestable::UpstreamIdAbsent => Verdict::permanent(REASON_UPSTREAM_ID_ABSENT),
        Unattestable::DigestMismatch => Verdict::permanent(REASON_DIGEST_MISMATCH),
        Unattestable::ReferenceMalformed => Verdict::permanent(REASON_REFERENCE_MALFORMED),
        Unattestable::BodiesUnreadable => Verdict::permanent(REASON_BODIES_UNREADABLE),
        Unattestable::BodyNotUtf8 => Verdict::permanent(REASON_BODY_NOT_UTF8),
        Unattestable::BodyTooLarge => Verdict::permanent(REASON_BODY_TOO_LARGE),
    }
}

/// Every state label this module can produce, for tests that pin the set.
pub const ALL_STATES: [&str; 4] = [
    STATE_ELIGIBLE,
    STATE_INELIGIBLE_PERMANENT,
    STATE_INELIGIBLE_CONFIGURATION,
    STATE_UNKNOWN,
];

/// Every reason label this module can produce, for tests that pin the set.
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    /// A request carrying the marker the server admits on, built through the
    /// protocol constant rather than spelled out, so a rename moves both
    /// sides at once.
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
            upstream_id: Some("chatcmpl-1".to_string()),
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
    /// function the submit path uses, so the bytes under test are the bytes
    /// a submission would read.
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

    /// The whole reason the field is optional. An invited contributor has no
    /// eligibility question, and answering one they do not have -- with
    /// `unknown` or with anything else -- would put three shells to work
    /// rendering it.
    #[test]
    fn an_invited_contributor_is_asked_nothing() {
        let (call, _dir) = call_with(&marked_request());
        assert_eq!(evaluate(false, &[row()], Some(&call), None), None);
        // Not merely for the eligible case: nothing about the inputs changes
        // the answer while the flag is off.
        assert_eq!(evaluate(false, &[], None, None), None);
        assert_eq!(
            evaluate(false, &[], None, Some(Unattestable::DigestMismatch)),
            None
        );
    }

    #[test]
    fn a_marked_final_call_is_eligible() {
        let (call, _dir) = call_with(&marked_request());
        assert_eq!(
            evaluate(true, &[row()], Some(&call), None),
            Some(Verdict {
                state: STATE_ELIGIBLE,
                reason: None,
            })
        );
    }

    /// A call carried whole whose request never carried the marker. Permanent:
    /// the bytes were sent long ago and nothing reaches back into them.
    #[test]
    fn an_unmarked_final_call_is_permanently_ineligible() {
        let (call, _dir) = call_with(&unmarked_request());
        assert_eq!(
            evaluate(true, &[row()], Some(&call), None),
            Some(Verdict {
                state: STATE_INELIGIBLE_PERMANENT,
                reason: Some(REASON_MARKER_ABSENT),
            })
        );
    }

    /// The case the surface exists for: history recorded before the
    /// contributor routed anything through attested inference at all.
    #[test]
    fn a_session_with_no_inference_hops_is_permanently_ineligible() {
        assert_eq!(
            evaluate(true, &[], None, None),
            Some(Verdict {
                state: STATE_INELIGIBLE_PERMANENT,
                reason: Some(REASON_NO_CALL),
            })
        );
    }

    /// The one variant a setting governs, and the only state whose sentence
    /// may name one.
    #[test]
    fn a_call_with_no_captured_bodies_is_a_configuration_answer() {
        let mut row = row();
        row.body_ref = None;
        assert_eq!(
            evaluate(true, &[row], None, None),
            Some(Verdict {
                state: STATE_INELIGIBLE_CONFIGURATION,
                reason: Some(REASON_CAPTURE_OFF),
            })
        );
    }

    /// A usable row, nothing carried and nothing refused: no body store is
    /// configured on this machine. Configuration, not permanent -- turning
    /// the store on makes future sessions eligible.
    #[test]
    fn a_machine_that_keeps_no_bodies_is_a_configuration_answer() {
        assert_eq!(
            evaluate(true, &[row()], None, None),
            Some(Verdict {
                state: STATE_INELIGIBLE_CONFIGURATION,
                reason: Some(REASON_EVIDENCE_CAPTURE_OFF),
            })
        );
    }

    /// Every expensive-group refusal keeps its own name rather than
    /// collapsing into one "could not carry it". An operator does something
    /// different about each, and the reason label is the only place that
    /// distinction survives.
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
                evaluate(true, &[row()], None, Some(refusal)),
                Some(Verdict {
                    state: STATE_INELIGIBLE_PERMANENT,
                    reason: Some(reason),
                }),
                "{refusal:?} lost its reason"
            );
        }
    }

    /// The cheap group outranks a recorded refusal, so the answer a list can
    /// always produce is the one it produces. The two cannot disagree in
    /// practice -- the full check runs the prefix first -- and this pins that
    /// the ordering here does not depend on that staying true.
    #[test]
    fn the_cheap_group_is_consulted_first() {
        assert_eq!(
            evaluate(true, &[], None, Some(Unattestable::BodyTooLarge)),
            Some(Verdict {
                state: STATE_INELIGIBLE_PERMANENT,
                reason: Some(REASON_NO_CALL),
            })
        );
    }

    /// Nothing this module produces is outside the pinned sets. The copy
    /// tables and the wire contract are both written against them.
    #[test]
    fn every_verdict_uses_a_pinned_label() {
        let (marked, _a) = call_with(&marked_request());
        let (unmarked, _b) = call_with(&unmarked_request());
        let mut no_body = row();
        no_body.body_ref = None;
        let mut verdicts = vec![
            evaluate(true, &[row()], Some(&marked), None),
            evaluate(true, &[row()], Some(&unmarked), None),
            evaluate(true, &[], None, None),
            evaluate(true, &[no_body], None, None),
            evaluate(true, &[row()], None, None),
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
            verdicts.push(evaluate(true, &[row()], None, Some(refusal)));
        }
        for verdict in verdicts.into_iter().flatten() {
            assert!(
                ALL_STATES.contains(&verdict.state),
                "{} is not a pinned state",
                verdict.state
            );
            if let Some(reason) = verdict.reason {
                assert!(
                    ALL_REASONS.contains(&reason),
                    "{reason} is not a pinned reason"
                );
            }
        }
    }

    /// Decision 1, pinned. A submission refused for an admission reason
    /// leaves the row unable to claim `eligible`.
    #[test]
    fn an_admission_refusal_retracts_the_claim() {
        assert_eq!(
            writeback_for("admission_request_malformed"),
            Some(Verdict {
                state: STATE_INELIGIBLE_PERMANENT,
                reason: Some(REASON_REQUEST_MALFORMED),
            })
        );
        assert_eq!(
            writeback_for("admission_receipt_unavailable"),
            Some(Verdict {
                state: STATE_UNKNOWN,
                reason: Some(REASON_RECEIPT_UNAVAILABLE),
            })
        );
        for verdict in [
            writeback_for("admission_request_malformed"),
            writeback_for("admission_receipt_unavailable"),
        ]
        .into_iter()
        .flatten()
        {
            assert_ne!(verdict.state, STATE_ELIGIBLE);
        }
    }

    /// And a refusal that says nothing about admissibility rewrites nothing.
    /// An unreachable server is not evidence about a contributor's session.
    #[test]
    fn an_unrelated_refusal_rewrites_nothing() {
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

    /// The daemon never emits `unknown` for a session it evaluates: every
    /// input above produces a real answer. (The one path that does produce
    /// it is [`writeback_for`], which retracts a claim rather than
    /// classifying a session.) The label ships in the contract anyway,
    /// because lazy evaluation is the obvious later optimisation and a shell
    /// that has never seen `unknown` will render it wrong on the day it first
    /// arrives.
    #[test]
    fn nothing_this_slice_evaluates_answers_unknown() {
        let (call, _dir) = call_with(&marked_request());
        let mut no_body = row();
        no_body.body_ref = None;
        let inputs = [
            evaluate(true, &[row()], Some(&call), None),
            evaluate(true, &[], None, None),
            evaluate(true, &[no_body], None, None),
            evaluate(true, &[row()], None, None),
            evaluate(true, &[row()], None, Some(Unattestable::DigestMismatch)),
        ];
        for verdict in inputs.into_iter().flatten() {
            assert_ne!(verdict.state, STATE_UNKNOWN);
        }
    }
}
