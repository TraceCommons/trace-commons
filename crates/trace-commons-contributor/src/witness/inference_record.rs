//! Whether a witnessed review carried attested inference, recorded where
//! the fact is known.
//!
//! # The gap this closes
//!
//! The attestation *mark* (`daemon::attestation_mark`) says a session
//! carries a checkable copy of its final model call. That is computed at
//! discovery, before any receipt is fetched and before any witness is
//! contacted. What happens next was recorded nowhere: the submit path
//! fetches a receipt, hands it to the witness with the bodies, the witness
//! verifies it inside the enclave and issues a certificate -- and the
//! certificate carries no field saying so (`CertificateDetails` is verdict,
//! policy version, measurement and timestamp), the receipt is discarded,
//! and ingest stores nothing about it either. A contributor whose session
//! showed `attested` in the queue, and whose receipt fetch then failed,
//! shipped an unattested trace with every surface still reading as
//! attested.
//!
//! This record is the client's own note of what actually happened, kept
//! beside the stored review (`WitnessReviewArtifact`) and mirrored onto the
//! queue entry for as long as that review is pinned.
//!
//! # Written from the fact, never from the intention
//!
//! [`InferenceAttestationRecord::certified`] is constructed in exactly one
//! place: `submit::witness_envelope`, after the witness has answered with a
//! certificate *and* a receipt was among what it was handed. A witness that
//! is offered a receipt verifies it over the raw bodies before certifying
//! anything -- an offered receipt that does not verify is a refusal, not a
//! certificate -- so "certificate issued, receipt offered" is the fact this
//! record states. Requesting a receipt is not that fact; neither is holding
//! an attested call. Both of those produce an [`STATE_UNCERTIFIED`] record
//! with a reason.
//!
//! What it does not state: whether the receipt's *signer* was one the
//! witness pins. That is decided by the witness's own configuration and is
//! not visible to the client; a certificate from a pinning witness and one
//! from a dormant witness read the same here.
//!
//! # Absent is unknown
//!
//! A stored review written before this record existed has no field, and it
//! loads as `None`. That is the third presence convention this codebase
//! already carries, and it is load-bearing here: an older review is not
//! "uncertified", because for all anyone can tell it may have carried a
//! verified receipt; and it is not "certified", because nothing says so. A
//! reader that flattened `None` into either answer would be telling a
//! contributor something false about their own work.

use serde::{Deserialize, Serialize};

/// A receipt was offered with the bodies and the witness issued a
/// certificate over them.
pub const STATE_CERTIFIED: &str = "certified";
/// The review was certified without attested inference; `reason` says why.
pub const STATE_UNCERTIFIED: &str = "uncertified";

/// No attested call was attached to the session: it joined no inference
/// hop, or the hop recorded no verbatim bodies.
pub const REASON_NO_ATTESTED_CALL: &str = "no_attested_call";
/// The session carried an attested call and the review was requested
/// without inference bodies, so nothing was offered.
pub const REASON_BODIES_WITHHELD: &str = "bodies_withheld";
/// The session carried an attested call and no receipt could be obtained
/// for it -- no endpoint configured, the provider answered without one, or
/// the fetch failed. The trace was certified without it.
pub const REASON_RECEIPT_UNAVAILABLE: &str = "receipt_unavailable";

/// Every state, for tests and for a consumer that renders a table.
pub const ALL_STATES: [&str; 2] = [STATE_CERTIFIED, STATE_UNCERTIFIED];
/// Every reason, likewise.
pub const ALL_REASONS: [&str; 3] = [
    REASON_NO_ATTESTED_CALL,
    REASON_BODIES_WITHHELD,
    REASON_RECEIPT_UNAVAILABLE,
];

/// One witnessed review's attested-inference answer.
///
/// Label-only: a state and, unless certified, a reason. Nothing here names a
/// model, a provider identifier, a digest or a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceAttestationRecord {
    /// One of the `STATE_*` labels.
    pub state: String,
    /// One of the `REASON_*` labels. `None` for [`STATE_CERTIFIED`], which
    /// has nothing to explain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl InferenceAttestationRecord {
    /// Only `submit::witness_envelope` may build this, and only after the
    /// witness has certified a request that carried a receipt.
    pub(crate) fn certified() -> Self {
        Self {
            state: STATE_CERTIFIED.to_string(),
            reason: None,
        }
    }

    pub(crate) fn uncertified(reason: &'static str) -> Self {
        Self {
            state: STATE_UNCERTIFIED.to_string(),
            reason: Some(reason.to_string()),
        }
    }

    #[must_use]
    pub fn is_certified(&self) -> bool {
        self.state == STATE_CERTIFIED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certified_record_carries_no_reason_and_an_uncertified_one_always_does() {
        let certified = InferenceAttestationRecord::certified();
        assert!(certified.is_certified());
        assert_eq!(certified.reason, None);
        for reason in ALL_REASONS {
            let record = InferenceAttestationRecord::uncertified(reason);
            assert!(!record.is_certified());
            assert_eq!(record.reason.as_deref(), Some(reason));
            assert!(ALL_STATES.contains(&record.state.as_str()));
        }
    }

    #[test]
    fn the_wire_shape_is_state_and_reason_only() {
        let json = serde_json::to_value(InferenceAttestationRecord::certified()).unwrap();
        assert_eq!(json, serde_json::json!({"state": "certified"}));
        let json = serde_json::to_value(InferenceAttestationRecord::uncertified(
            REASON_RECEIPT_UNAVAILABLE,
        ))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"state": "uncertified", "reason": "receipt_unavailable"})
        );
        assert!(
            serde_json::from_value::<InferenceAttestationRecord>(
                serde_json::json!({"state": "certified", "model": "x"})
            )
            .is_err(),
            "an unknown field is refused, so nothing identifying can ride along"
        );
    }
}
