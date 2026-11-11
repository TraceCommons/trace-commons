//! The labels a witness answers a non-success request with, and how a
//! client reads a status and a label together.
//!
//! One spelling of each label, shared by the witness and the contributor so
//! the two cannot drift. Before these were shared, every label the client
//! did not spell itself read as a malformed answer, and a malformed answer
//! is a terminal refusal: a witness outage refused sessions nothing had
//! judged.
//!
//! The capacity refusal lives in [`crate::witness_pacing`], the job-gone
//! label in [`crate::witness_jobs`], and the evidence refusal in
//! [`crate::admission`]; each keeps its own module.

/// `504`: the witness's own request deadline ran out and it certified
/// nothing.
pub const WITNESS_REQUEST_TIMED_OUT: &str = "witness_request_timed_out";
/// `413`: the request is larger than the witness accepts.
pub const WITNESS_REQUEST_TOO_LARGE: &str = "witness_request_too_large";

/// `503`: the redaction did not complete (a classifier unavailable or
/// erroring, or a residual secret the witness would not certify).
pub const WITNESS_REDACTION_FAILED: &str = "witness_redaction_failed";
/// `503`: the witness could not bind a digest to the artifact it produced.
pub const WITNESS_ARTIFACT_BINDING_FAILED: &str = "witness_artifact_binding_failed";
/// `503`: the enclave could not report its measurement.
pub const WITNESS_MEASUREMENT_UNAVAILABLE: &str = "witness_measurement_unavailable";
/// `503`: the signer refused or was unavailable.
pub const WITNESS_SIGNING_UNAVAILABLE: &str = "witness_signing_unavailable";
/// `503`: this deployment has no structured contribution path configured.
pub const WITNESS_CONTRIBUTION_PATH_UNAVAILABLE: &str = "witness_contribution_path_unavailable";

/// The `503` refusals that say the witness could not do its job right now.
/// None of them is about the contributor's input being wrong.
pub const WITNESS_UNAVAILABLE_REFUSALS: [&str; 5] = [
    WITNESS_REDACTION_FAILED,
    WITNESS_ARTIFACT_BINDING_FAILED,
    WITNESS_MEASUREMENT_UNAVAILABLE,
    WITNESS_SIGNING_UNAVAILABLE,
    WITNESS_CONTRIBUTION_PATH_UNAVAILABLE,
];

/// `403`: attested inference is required and no receipt was offered.
pub const WITNESS_INFERENCE_ATTESTATION_MISSING: &str = "witness_inference_attestation_missing";
/// `403`: the receipt's attestation could not be obtained.
pub const WITNESS_INFERENCE_ATTESTATION_UNAVAILABLE: &str =
    "witness_inference_attestation_unavailable";
/// `403`: the session holds no inference call the receipt could cover.
pub const WITNESS_INFERENCE_CALL_ABSENT: &str = "witness_inference_call_absent";
/// `403`: the inference call cannot be attested.
pub const WITNESS_INFERENCE_CALL_UNATTESTABLE: &str = "witness_inference_call_unattestable";
/// `403`: the receipt's body is not in the session.
pub const WITNESS_INFERENCE_BODY_NOT_IN_SESSION: &str = "witness_inference_body_not_in_session";
/// `403`: the receipt did not verify.
pub const WITNESS_INFERENCE_RECEIPT_UNVERIFIED: &str = "witness_inference_receipt_unverified";
/// `403`: the receipted body is larger than the deployment accepts.
pub const WITNESS_INFERENCE_BODY_TOO_LARGE: &str = "witness_inference_body_too_large";

/// The `403` attested-inference refusals: the deployment declining this
/// submission under the operator's policy, the same answer every time.
pub const WITNESS_INFERENCE_REFUSALS: [&str; 7] = [
    WITNESS_INFERENCE_ATTESTATION_MISSING,
    WITNESS_INFERENCE_ATTESTATION_UNAVAILABLE,
    WITNESS_INFERENCE_CALL_ABSENT,
    WITNESS_INFERENCE_CALL_UNATTESTABLE,
    WITNESS_INFERENCE_BODY_NOT_IN_SESSION,
    WITNESS_INFERENCE_RECEIPT_UNVERIFIED,
    WITNESS_INFERENCE_BODY_TOO_LARGE,
];

/// What a non-success witness answer says, read from its status and its
/// `error` label together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitnessRefusalClass {
    /// `503 witness_saturated`: at capacity, nothing judged.
    Saturated,
    /// A deadline ran out: `504 witness_request_timed_out`, or a `504` with
    /// no label (a gateway that gave up waiting).
    TimedOut,
    /// The witness, or what stands in front of it, cannot serve right now.
    /// Nothing was judged, so the same request may succeed later: any `5xx`
    /// not named above, and an unlabelled `404`, `408` or `429` (a gateway
    /// with no instance behind it, or pacing of its own).
    Unavailable,
    /// `413`: the request is larger than the witness accepts.
    TooLarge,
    /// `403` with one of [`WITNESS_INFERENCE_REFUSALS`].
    InferenceDeclined,
    /// Anything else: a client error this module does not name, or a label
    /// that does not agree with its status.
    Unrecognised,
}

/// Read a non-success answer.
///
/// The label is attacker-influenceable, so it never selects a class by
/// itself: a label counts only with the status it is sent with, and only
/// the status can make an answer retryable. A `4xx` stays non-retryable
/// whatever its body says, except the unlabelled gateway answers named in
/// [`WitnessRefusalClass::Unavailable`].
#[must_use]
pub fn classify_witness_refusal(status: u16, label: Option<&str>) -> WitnessRefusalClass {
    if label.is_some_and(|label| crate::witness_pacing::is_witness_saturation(status, label)) {
        return WitnessRefusalClass::Saturated;
    }
    match (status, label) {
        (504, None) | (504, Some(WITNESS_REQUEST_TIMED_OUT)) => WitnessRefusalClass::TimedOut,
        (413, None) | (413, Some(WITNESS_REQUEST_TOO_LARGE)) => WitnessRefusalClass::TooLarge,
        (403, Some(label)) if WITNESS_INFERENCE_REFUSALS.contains(&label) => {
            WitnessRefusalClass::InferenceDeclined
        }
        (500..=599, _) => WitnessRefusalClass::Unavailable,
        (404 | 408 | 429, None) => WitnessRefusalClass::Unavailable,
        _ => WitnessRefusalClass::Unrecognised,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_spelled_as_the_witness_sends_them() {
        assert_eq!(WITNESS_REQUEST_TIMED_OUT, "witness_request_timed_out");
        assert_eq!(WITNESS_REQUEST_TOO_LARGE, "witness_request_too_large");
        assert_eq!(
            WITNESS_UNAVAILABLE_REFUSALS,
            [
                "witness_redaction_failed",
                "witness_artifact_binding_failed",
                "witness_measurement_unavailable",
                "witness_signing_unavailable",
                "witness_contribution_path_unavailable",
            ]
        );
        assert_eq!(
            WITNESS_INFERENCE_REFUSALS,
            [
                "witness_inference_attestation_missing",
                "witness_inference_attestation_unavailable",
                "witness_inference_call_absent",
                "witness_inference_call_unattestable",
                "witness_inference_body_not_in_session",
                "witness_inference_receipt_unverified",
                "witness_inference_body_too_large",
            ]
        );
    }

    #[test]
    fn each_status_and_label_reads_as_one_class() {
        use WitnessRefusalClass::*;
        let mut table: Vec<(u16, Option<&str>, WitnessRefusalClass)> = vec![
            (503, Some("witness_saturated"), Saturated),
            (504, Some(WITNESS_REQUEST_TIMED_OUT), TimedOut),
            (504, None, TimedOut),
            (413, Some(WITNESS_REQUEST_TOO_LARGE), TooLarge),
            (413, None, TooLarge),
            // Gateway answers while the witness is down.
            (404, None, Unavailable),
            (408, None, Unavailable),
            (429, None, Unavailable),
            (500, None, Unavailable),
            (502, None, Unavailable),
            (503, None, Unavailable),
            // Any 5xx, whatever it is labelled.
            (500, Some("admission_evidence_refused"), Unavailable),
            (503, Some("something_new"), Unavailable),
            (504, Some("something_new"), Unavailable),
            // Client errors stay terminal.
            (400, Some("witness_request_malformed"), Unrecognised),
            (400, Some("witness_workload_malformed"), Unrecognised),
            (400, Some("witness_fetch_secret_malformed"), Unrecognised),
            (400, None, Unrecognised),
            (401, None, Unrecognised),
            (403, None, Unrecognised),
            // A label never moves a 4xx on its own.
            (404, Some(WITNESS_REDACTION_FAILED), Unrecognised),
            (429, Some("witness_saturated"), Unrecognised),
            (413, Some("something_else"), Unrecognised),
            (
                400,
                Some(WITNESS_INFERENCE_RECEIPT_UNVERIFIED),
                Unrecognised,
            ),
            (403, Some(WITNESS_REQUEST_TOO_LARGE), Unrecognised),
        ];
        for label in WITNESS_UNAVAILABLE_REFUSALS {
            table.push((503, Some(label), Unavailable));
        }
        for label in WITNESS_INFERENCE_REFUSALS {
            table.push((403, Some(label), InferenceDeclined));
            table.push((503, Some(label), Unavailable));
        }
        for (status, label, expected) in table {
            assert_eq!(
                classify_witness_refusal(status, label),
                expected,
                "{status} {label:?}"
            );
        }
    }
}
