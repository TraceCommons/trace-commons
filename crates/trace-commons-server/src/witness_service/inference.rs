// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Attested inference, as a requirement the witness enforces.
//!
//! A witness certificate says an enclave redacted these bytes and reached this
//! verdict over them. It says nothing about where the transcript came from,
//! and nothing stops one being synthesised. A NEAR AI inference receipt is the
//! other half: it is signed by the inference enclave's key and binds
//! `SHA256(request_body_as_sent)` and `SHA256(response_body_as_received)`.
//!
//! This module is what lets a witness **require** one before it certifies
//! anything.
//!
//! # The transcript contains the bodies, so linkage is definitional
//!
//! The hard question about any receipt-carrying design is what binds a receipt
//! to *this* trace. Hashes bind a receipt to bodies; if the bodies were a
//! separate attachment, a contributor could attach any valid receipt they
//! could obtain, and every scheme for joining an attachment back to a
//! transcript is either a fuzzy rendering judgement or a check against an
//! assertion the contributor typed.
//!
//! This module joins nothing. **The raw bodies are part of the session the
//! witness was handed** -- a `TraceFile.http_exchanges` entry, carried through
//! `from_recorded_trace` into a
//! [`TraceContributionEventType::HttpExchange`] event whose
//! `structured_payload["request"]["body"]` is the request as sent and whose
//! `content` is the response as received. The witness reads those bytes out of
//! what it already holds and [`verify_receipt`] hashes them. There is one copy
//! of the bytes, the receipt binds them, and they are inside the material
//! being witnessed. Nothing is extracted, reassembled, or compared for
//! faithfulness.
//!
//! This also retires the objection that in-witness verification enlarges the
//! enclave's blast radius by shipping it raw HTTP bodies *in addition* to the
//! transcript. There is no addition: the enclave already holds exactly these
//! bytes, and it is the only party that does -- which is why it is the only
//! party positioned to verify at all.
//!
//! # The final call, chosen by the witness
//!
//! One body pair per trace, not one per turn. A chat-completions request body
//! repeats the whole conversation prefix, so per-turn bodies are O(N^2) in
//! session length -- and 7% of real sessions on this pilot already exceed the
//! 16 MB envelope cap at a 3.4:1 raw-to-envelope ratio.
//!
//! **Which call is attested is decided here, not by the caller.** The witness
//! takes the last `HttpExchange` event in the trace's own event order and
//! verifies the offered receipt against *that* exchange's bodies. A caller who
//! could nominate the exchange would nominate whichever one had a body that
//! suited them, and "the final call is attested" would be a claim about the
//! caller's choice.
//!
//! Two limits on that, both of which the certificate and every operator
//! surface must respect:
//!
//! - **Event order is the trace's own order.** The witness has no independent
//!   clock and no view of the session; a contributor who reorders events is
//!   describing a different session, and this module cannot tell. What it
//!   establishes is "the last inference call *this trace declares*".
//! - **Compaction breaks the transitive-coverage argument.** The reason one
//!   pair is worth having is that the final request body contains the
//!   conversation prefix -- true of an uncompacted linear session, false of one
//!   that summarised or truncated its context, and the witness cannot tell
//!   which it got. So nothing may say the attested call covers the history.
//!   The honest statement is about the bytes actually bound, and nothing more.
//!
//! Note what a later request does *not* give you: verifying the receipt for a
//! call needs that call's raw response body, and it cannot be recovered from
//! the next request body -- the receipt binds the raw HTTP response, not the
//! assistant message a later request quotes out of it.
//!
//! # Verification happens once, and cannot be repeated downstream
//!
//! The receipt binds the **raw** bodies; the witness emits a **redacted**
//! artifact. Redaction destroys the attested bytes -- that is what redaction
//! is -- so no party downstream of the witness can re-verify a receipt against
//! what it holds. This is inherent, not a gap to be closed.
//!
//! A witness that requires attested inference and issues a certificate anyway
//! is saying *a verified receipt was seen over the raw bytes at witness time*.
//! It is not saying the published artifact is attested, and nothing may imply
//! that a consumer can check it.
//!
//! # What it proves, exactly
//!
//! - An attested NEAR AI enclave produced this response for this request, and
//!   both bodies were inside the session that was certified, as its last
//!   declared inference call.
//! - It does **not** prove the session made the call. A contributor holding a
//!   receipt and its bodies can paste them into a trace they wrote. Closing
//!   that needs a capture-side change -- a nonce the contributor's identity
//!   determines, carried inside the request body, so the request hash commits
//!   to who called -- and nothing in any capture path sends one today.
//! - It says nothing about any other turn, tool result or file edit, and
//!   nothing about the conversation prefix (see compaction, above). An
//!   operator surface renders `n_of_m` and never "attested" or "genuine".
//! - It cannot detect a receipt replayed across two submissions. The witness
//!   holds nothing between requests by design. Dedup on the receipt signature
//!   belongs to ingest, which has state.
//!
//! # Capture must be byte-verbatim, and a bad capture is indistinguishable
//! # from a forgery
//!
//! The sharpest edge in the design. `HttpExchange`'s bodies are `String`s, and
//! whether a capture put verbatim wire bytes there or a re-serialisation is a
//! capture-side question this module cannot answer. SHA-256 answers one bit:
//! these bytes are the bytes, or they are not. A capture that pretty-printed
//! the JSON, reordered its keys, or normalised a line ending produces the same
//! failure as a receipt lifted from somewhere else, and the witness **cannot**
//! tell them apart.
//!
//! So [`WitnessError::InferenceReceiptUnverified`] is named for what was
//! observed -- the receipt did not verify against these bytes -- and not for
//! any conclusion about why. On an honest deployment a capture bug is the
//! likelier cause, and an operator must read it that way.
//!
//! # Two-part receipts are refused
//!
//! `ReceiptVerdict.model` is `None` for the two-part form, which binds no
//! model at all. A deployment that requires attested inference is requiring a
//! statement about which program served it, so the two-part form is refused by
//! name rather than admitted as a weaker pass. The model the receipt binds is
//! compared against the model named in the request body -- itself hash-bound by
//! the receipt, so the caller cannot choose it after the fact -- and then
//! against the deployment's allowlist.
//!
//! # Nothing here is logged
//!
//! Bodies, receipt text, signatures and signing addresses are all caller data.
//! Every refusal is a bare label with no count, no offset and no payload;
//! `ReceiptPayload` never reaches a `Debug` this module writes. The several
//! ways a receipt can fail to verify are deliberately folded into one label:
//! publishing which one occurred on an unauthenticated route would tell a
//! prober which of its guesses was closest.
//!
//! [`TraceContributionEventType::HttpExchange`]: trace_commons_protocol::trace_contribution::TraceContributionEventType::HttpExchange

use serde_json::Value;
use trace_commons_protocol::trace_contribution::{
    RawTraceContribution, RawTraceContributionEvent, TraceContributionEventType,
};

use crate::near_attestation::receipt::{ReceiptPayload, verify_receipt};

use super::WitnessError;

/// How large either attested body may be, in bytes.
///
/// 8 MiB. The attested request body is the whole conversation prefix, so this
/// has to clear a large session rather than a single turn, while still
/// bounding one receipt's work well under the request cap.
pub const DEFAULT_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// This deployment's attested-inference policy.
///
/// Private fields and two constructors: a policy assembled field by field
/// could be `required: true` with an empty everything, which reads as a
/// requirement and enforces nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferenceAttestationPolicy {
    required: bool,
    admissible_models: Vec<String>,
    max_body_bytes: usize,
}

/// The requirement was configured in a way that would not require anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the attested-inference requirement is configured to require nothing")]
pub struct PolicyMisconfigured;

impl InferenceAttestationPolicy {
    /// No requirement: a submission carrying no receipt is certified.
    ///
    /// A receipt that *is* offered still has to verify. Accepting an invalid
    /// receipt because none was required would be a silent downgrade, and the
    /// caller would have been told nothing.
    pub fn not_required() -> Self {
        Self {
            required: false,
            admissible_models: Vec::new(),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// Refuse any contribution whose last declared inference call does not
    /// carry a verified receipt.
    ///
    /// An empty `admissible_models` means any model the receipt binds is
    /// admissible -- the model is still required to be *bound*, which is the
    /// part the two-part form cannot do.
    pub fn required(
        admissible_models: Vec<String>,
        max_body_bytes: usize,
    ) -> Result<Self, PolicyMisconfigured> {
        if max_body_bytes == 0 {
            return Err(PolicyMisconfigured);
        }
        Ok(Self {
            required: true,
            admissible_models,
            max_body_bytes,
        })
    }

    /// Whether this deployment refuses an unattested contribution.
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// How large either attested body may be.
    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }
}

/// What the check established.
///
/// `verified` is 0 or 1: one call is attested, not one per turn.
/// `declared_calls` is how many `HttpExchange` events the trace carries --
/// what the trace *says* it did, never what the session actually did, since
/// nothing obliges a contributor to declare a call at all.
///
/// An operator surface renders this `n_of_m`. It may not render it
/// "attested" or "genuine": one verified receipt over a trace declaring nine
/// calls is `1_of_9`, and the other eight are unexamined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceAttestationOutcome {
    /// 1 when the last declared call carried a verified receipt, else 0.
    pub verified: usize,
    /// How many inference exchanges the trace declares.
    pub declared_calls: usize,
}

/// The session as the witness received it, **before** redaction.
///
/// Before, and this is the only order that can work: the receipt binds the raw
/// bodies and the redaction pass rewrites them. A witness that verified after
/// redacting would be hashing bytes no receipt was ever taken over.
///
/// Borrowed rather than owned: a session at the request cap is tens of
/// megabytes, and copying it would double the peak of the component that holds
/// every raw transcript passing through it.
pub enum WitnessedSession<'a> {
    /// The text route: an opaque transcript.
    ///
    /// It carries no event structure, so **which call was last cannot be
    /// established here**, and this module refuses rather than falling back to
    /// a caller-nominated exchange. A deployment that requires attested
    /// inference is a deployment whose contributors must use the structured
    /// route.
    Transcript,
    /// The structured route, where the exchanges are events.
    Contribution(&'a RawTraceContribution),
}

/// Verify the offered receipt against the last inference call the session
/// declares, and enforce the deployment's requirement.
///
/// Runs **before** the redaction pass on both witness paths, for two reasons
/// that point the same way: the receipt binds the raw bodies, and a submission
/// that is going to be refused should not first spend a metered classifier.
pub fn check_inference_attestation(
    policy: &InferenceAttestationPolicy,
    offered: Option<&ReceiptPayload>,
    session: &WitnessedSession<'_>,
) -> Result<InferenceAttestationOutcome, WitnessError> {
    let raw = match session {
        WitnessedSession::Contribution(raw) => raw,
        WitnessedSession::Transcript => {
            // Both arms fail closed. Offering a receipt on a route that cannot
            // say which call was last is refused rather than verified against
            // something the caller chose, and requiring attestation on that
            // route refuses too.
            if offered.is_some() || policy.required {
                return Err(WitnessError::InferenceAttestationUnavailable);
            }
            return Ok(InferenceAttestationOutcome {
                verified: 0,
                declared_calls: 0,
            });
        }
    };

    let declared_calls = raw
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::HttpExchange)
        .count();

    let Some(receipt) = offered else {
        if policy.required {
            return Err(WitnessError::InferenceAttestationMissing);
        }
        return Ok(InferenceAttestationOutcome {
            verified: 0,
            declared_calls,
        });
    };

    // The witness picks the exchange; the caller only supplies the receipt.
    let Some(final_call) = raw
        .events
        .iter()
        .rev()
        .find(|event| event.event_type == TraceContributionEventType::HttpExchange)
    else {
        // A trace with no declared inference call at all. Named separately
        // from a missing receipt because an operator does something different
        // about it: this contribution cannot satisfy the requirement in
        // principle, rather than having failed to.
        return Err(WitnessError::InferenceCallAbsent);
    };

    let (request_body, response_body) =
        exchange_bodies(final_call).ok_or(WitnessError::InferenceBodyNotInSession)?;

    if request_body.len() > policy.max_body_bytes || response_body.len() > policy.max_body_bytes {
        return Err(WitnessError::InferenceReceiptTooLarge);
    }

    // The model the receipt is checked against comes out of the request body,
    // which the receipt's own request hash commits to. Taking it from a field
    // beside the receipt would let the caller name whatever the receipt says
    // and turn `verify_receipt`'s model check into a tautology.
    let request: Value =
        serde_json::from_str(request_body).map_err(|_| WitnessError::InferenceBodyUnreadable)?;
    let requested_model = request
        .get("model")
        .and_then(Value::as_str)
        .ok_or(WitnessError::InferenceBodyUnreadable)?;

    // One label for every `ReceiptError`; see the module's note on why a
    // re-serialised capture is indistinguishable from a forgery here.
    let verdict = verify_receipt(
        receipt,
        request_body.as_bytes(),
        response_body.as_bytes(),
        requested_model,
    )
    .map_err(|_| WitnessError::InferenceReceiptUnverified)?;

    // The two-part form binds no model. See the module docs.
    let Some(bound_model) = verdict.model.as_deref() else {
        return Err(WitnessError::InferenceReceiptModelUnbound);
    };
    if !policy.admissible_models.is_empty()
        && !policy
            .admissible_models
            .iter()
            .any(|admissible| admissible == bound_model)
    {
        return Err(WitnessError::InferenceModelInadmissible);
    }

    Ok(InferenceAttestationOutcome {
        verified: 1,
        declared_calls,
    })
}

/// The two raw bodies an `HttpExchange` event carries, in the one place
/// `from_recorded_trace` writes them.
///
/// `structured_payload["request"]["body"]` and `content`. Both are present
/// only under the `include_tool_payloads` consent flag -- without it the
/// conversion writes a payload carrying method and status and no bodies at
/// all, so a contribution that withheld payloads cannot satisfy an
/// attestation requirement. That is a real cost of turning the requirement on,
/// and it is a refusal by name rather than a silent pass.
fn exchange_bodies(event: &RawTraceContributionEvent) -> Option<(&str, &str)> {
    let request = event
        .structured_payload
        .get("request")?
        .get("body")?
        .as_str()?;
    let response = event.content.as_deref()?;
    Some((request, response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use sha2::{Digest as _, Sha256};
    use sha3::Keccak256;
    use trace_commons_protocol::trace_contribution::{
        RawTraceCaptureTurn, RecordedTraceContributionOptions,
    };

    /// A fixed key, never generated: a random key makes a failure
    /// unreproducible, and every input to these tests has to be pinned.
    const INFERENCE_KEY_HEX: &str =
        "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const OTHER_KEY_HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    const MODEL: &str = "Qwen/Qwen3.6-27B-FP8";

    fn key(hex_bytes: &str) -> SigningKey {
        SigningKey::from_slice(&hex::decode(hex_bytes).expect("hex")).expect("scalar")
    }

    fn address(k: &SigningKey) -> String {
        let point = k.verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&point.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..]))
    }

    fn sign(k: &SigningKey, text: &str) -> String {
        let mut hasher = Keccak256::new();
        hasher.update(format!("\x19Ethereum Signed Message:\n{}", text.len()).as_bytes());
        hasher.update(text.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let (signature, recovery_id) = k.sign_prehash_recoverable(&digest).expect("sign");
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recovery_id.to_byte() + 27);
        format!("0x{}", hex::encode(raw))
    }

    fn sha256_hex(bytes: &str) -> String {
        hex::encode(Sha256::digest(bytes.as_bytes()))
    }

    /// A receipt over these exact bytes, in the three-part model-bound form.
    fn receipt(model: &str, request_body: &str, response_body: &str) -> ReceiptPayload {
        receipt_signed_by(INFERENCE_KEY_HEX, model, request_body, response_body)
    }

    fn receipt_signed_by(
        key_hex: &str,
        model: &str,
        request_body: &str,
        response_body: &str,
    ) -> ReceiptPayload {
        let signer = key(key_hex);
        let text = format!(
            "{model}:{}:{}",
            sha256_hex(request_body),
            sha256_hex(response_body)
        );
        ReceiptPayload {
            signature: sign(&signer, &text),
            signing_address: address(&signer),
            text,
        }
    }

    /// The two-part form, which binds no model.
    fn two_part_receipt(request_body: &str, response_body: &str) -> ReceiptPayload {
        let signer = key(INFERENCE_KEY_HEX);
        let text = format!("{}:{}", sha256_hex(request_body), sha256_hex(response_body));
        ReceiptPayload {
            signature: sign(&signer, &text),
            signing_address: address(&signer),
            text,
        }
    }

    fn request_body(model: &str, prompt: &str) -> String {
        format!(r#"{{"model":"{model}","messages":[{{"role":"user","content":"{prompt}"}}]}}"#)
    }

    fn response_body(answer: &str) -> String {
        format!(
            r#"{{"id":"c1","choices":[{{"message":{{"role":"assistant","content":"{answer}"}}}}]}}"#
        )
    }

    /// A contribution carrying `exchanges` as `HttpExchange` events, in order,
    /// in the shape `from_recorded_trace` writes them under
    /// `include_tool_payloads`.
    fn contribution(exchanges: &[(String, String)]) -> RawTraceContribution {
        let started = chrono::Utc::now();
        let mut raw = RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: "do the thing".to_string(),
                response: None,
                tool_calls: Vec::new(),
                started_at: started,
                completed_at: Some(started + chrono::Duration::milliseconds(10)),
                state: Some("Completed".to_string()),
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..RecordedTraceContributionOptions::default()
            },
        );
        for (request, response) in exchanges {
            raw.events
                .push(exchange_event(Some(request), Some(response)));
        }
        raw
    }

    fn exchange_event(request: Option<&str>, response: Option<&str>) -> RawTraceContributionEvent {
        let structured_payload = match request {
            Some(body) => serde_json::json!({
                "request": {
                    "method": "POST",
                    "url": "https://example.invalid/v1/chat/completions",
                    "body": body,
                },
                "response": {"status": 200},
            }),
            // What the conversion writes when `include_tool_payloads` is off:
            // method and status, and no bodies.
            None => serde_json::json!({
                "request": {"method": "POST"},
                "response": {"status": 200},
            }),
        };
        RawTraceContributionEvent {
            event_id: uuid::Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::HttpExchange,
            timestamp: chrono::Utc::now(),
            content: response.map(str::to_string),
            structured_payload,
            tool_name: Some("http".to_string()),
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: Some(true),
            failure_modes: Vec::new(),
        }
    }

    fn required() -> InferenceAttestationPolicy {
        InferenceAttestationPolicy::required(Vec::new(), DEFAULT_MAX_BODY_BYTES)
            .expect("a well formed policy")
    }

    fn check(
        policy: &InferenceAttestationPolicy,
        offered: Option<&ReceiptPayload>,
        raw: &RawTraceContribution,
    ) -> Result<InferenceAttestationOutcome, WitnessError> {
        check_inference_attestation(policy, offered, &WitnessedSession::Contribution(raw))
    }

    /// The positive control. Without it every refusal assertion below would
    /// also pass against a checker that refused everything.
    #[test]
    fn a_verified_receipt_over_the_final_call_is_admitted() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let outcome = check(
            &required(),
            Some(&receipt(MODEL, &request, &response)),
            &raw,
        )
        .expect("a verified receipt over the final call");
        assert_eq!(
            outcome,
            InferenceAttestationOutcome {
                verified: 1,
                declared_calls: 1
            }
        );
    }

    #[test]
    fn a_required_witness_refuses_a_contribution_that_carries_no_receipt() {
        let raw = contribution(&[(request_body(MODEL, "hello"), response_body("hi"))]);
        assert_eq!(
            check(&required(), None, &raw),
            Err(WitnessError::InferenceAttestationMissing)
        );
        // And the same contribution passes where nothing is required, so the
        // refusal is the policy and not the fixture.
        assert!(check(&InferenceAttestationPolicy::not_required(), None, &raw).is_ok());
    }

    /// The decisive one: the witness chooses the exchange, so a receipt for an
    /// earlier call does not satisfy the requirement even though it is a
    /// perfectly valid receipt over bodies that are in the session.
    #[test]
    fn a_receipt_for_an_earlier_call_does_not_attest_the_final_one() {
        let first = (request_body(MODEL, "first"), response_body("one"));
        let last = (request_body(MODEL, "second"), response_body("two"));
        let raw = contribution(&[first.clone(), last.clone()]);

        assert_eq!(
            check(&required(), Some(&receipt(MODEL, &first.0, &first.1)), &raw),
            Err(WitnessError::InferenceReceiptUnverified),
            "a receipt for a call the contributor would have preferred to \
             attest must not pass"
        );
        // The same trace, attested at the call the witness picks.
        assert!(
            check(&required(), Some(&receipt(MODEL, &last.0, &last.1)), &raw).is_ok(),
            "the final call must still verify, or the assertion above proves \
             nothing"
        );
    }

    #[test]
    fn a_two_part_receipt_binds_no_model_and_is_refused() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        assert_eq!(
            check(
                &required(),
                Some(&two_part_receipt(&request, &response)),
                &raw
            ),
            Err(WitnessError::InferenceReceiptModelUnbound)
        );
    }

    #[test]
    fn a_model_outside_the_allowlist_is_refused() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy =
            InferenceAttestationPolicy::required(vec![MODEL.to_string()], DEFAULT_MAX_BODY_BYTES)
                .expect("policy");
        assert!(
            check(&policy, Some(&receipt(MODEL, &request, &response)), &raw).is_ok(),
            "the admitted model must pass"
        );

        let other = "some-other/model";
        let other_request = request_body(other, "hello");
        let other_raw = contribution(&[(other_request.clone(), response.clone())]);
        assert_eq!(
            check(
                &policy,
                Some(&receipt(other, &other_request, &response)),
                &other_raw
            ),
            Err(WitnessError::InferenceModelInadmissible)
        );
    }

    /// A receipt whose bound model differs from the model named in the request
    /// body it binds. The model is read out of the hash-bound body precisely so
    /// this substitution has nowhere to hide.
    #[test]
    fn a_receipt_naming_a_different_model_than_the_request_body_is_refused() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        assert_eq!(
            check(
                &required(),
                Some(&receipt("some-cheaper/model", &request, &response)),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    #[test]
    fn a_receipt_signed_by_a_different_key_than_it_claims_is_refused() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let mut forged = receipt_signed_by(OTHER_KEY_HEX, MODEL, &request, &response);
        // Claim the honest signer while the signature is somebody else's.
        forged.signing_address = address(&key(INFERENCE_KEY_HEX));
        assert_eq!(
            check(&required(), Some(&forged), &raw),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    /// A capture that re-serialised the body it recorded. The receipt is
    /// honest; the bytes are not the bytes. The witness cannot tell this from a
    /// forgery, and the refusal is named for what it observed.
    #[test]
    fn a_reserialised_capture_is_refused_and_is_indistinguishable_from_a_forgery() {
        let sent = request_body(MODEL, "hello");
        let response = response_body("hi");
        let pretty_printed = serde_json::to_string_pretty(
            &serde_json::from_str::<Value>(&sent).expect("the fixture is JSON"),
        )
        .expect("re-serialises");
        assert_ne!(pretty_printed, sent, "the fixture must actually differ");

        let raw = contribution(&[(pretty_printed, response.clone())]);
        assert_eq!(
            check(&required(), Some(&receipt(MODEL, &sent, &response)), &raw),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    #[test]
    fn a_witness_that_requires_nothing_still_refuses_an_invalid_receipt() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        assert_eq!(
            check(
                &InferenceAttestationPolicy::not_required(),
                Some(&receipt(MODEL, &request, "some other response")),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified),
            "an offered receipt is verified even where none was required; \
             admitting a bad one would be a silent downgrade"
        );
    }

    #[test]
    fn a_contribution_declaring_no_inference_call_is_refused_by_its_own_name() {
        let raw = contribution(&[]);
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        assert_eq!(
            check(
                &required(),
                Some(&receipt(MODEL, &request, &response)),
                &raw
            ),
            Err(WitnessError::InferenceCallAbsent)
        );
    }

    /// The consent cost of the requirement, as a test rather than a comment: a
    /// contribution that withheld tool payloads carries no bodies, so it cannot
    /// satisfy the requirement.
    #[test]
    fn a_final_call_without_bodies_cannot_be_attested() {
        let mut raw = contribution(&[]);
        raw.events.push(exchange_event(None, None));
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        assert_eq!(
            check(
                &required(),
                Some(&receipt(MODEL, &request, &response)),
                &raw
            ),
            Err(WitnessError::InferenceBodyNotInSession)
        );
    }

    #[test]
    fn the_text_route_cannot_attest_anything_and_refuses_by_name() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let offered = receipt(MODEL, &request, &response);

        assert_eq!(
            check_inference_attestation(&required(), None, &WitnessedSession::Transcript),
            Err(WitnessError::InferenceAttestationUnavailable),
            "a witness that requires attestation must refuse the route that \
             cannot establish which call was last"
        );
        assert_eq!(
            check_inference_attestation(
                &InferenceAttestationPolicy::not_required(),
                Some(&offered),
                &WitnessedSession::Transcript,
            ),
            Err(WitnessError::InferenceAttestationUnavailable),
            "and must refuse an offered receipt there rather than verifying it \
             against an exchange the caller chose"
        );
        assert!(
            check_inference_attestation(
                &InferenceAttestationPolicy::not_required(),
                None,
                &WitnessedSession::Transcript,
            )
            .is_ok(),
            "an unattested transcript on an unrequiring witness is still served"
        );
    }

    #[test]
    fn a_body_larger_than_the_witness_will_hash_is_refused() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let tight = InferenceAttestationPolicy::required(Vec::new(), 8).expect("policy");
        assert_eq!(
            check(&tight, Some(&receipt(MODEL, &request, &response)), &raw),
            Err(WitnessError::InferenceReceiptTooLarge)
        );
    }

    /// `n_of_m`: `m` is what the trace declares, not what was attested.
    #[test]
    fn the_outcome_counts_every_declared_call_and_attests_one() {
        let last = (request_body(MODEL, "third"), response_body("three"));
        let raw = contribution(&[
            (request_body(MODEL, "first"), response_body("one")),
            (request_body(MODEL, "second"), response_body("two")),
            last.clone(),
        ]);
        let outcome = check(&required(), Some(&receipt(MODEL, &last.0, &last.1)), &raw)
            .expect("the final call verifies");
        assert_eq!(
            outcome,
            InferenceAttestationOutcome {
                verified: 1,
                declared_calls: 3
            },
            "one verified receipt over a trace declaring three calls is 1_of_3"
        );
    }

    #[test]
    fn a_policy_that_would_require_nothing_is_refused() {
        assert_eq!(
            InferenceAttestationPolicy::required(Vec::new(), 0),
            Err(PolicyMisconfigured)
        );
    }
}
