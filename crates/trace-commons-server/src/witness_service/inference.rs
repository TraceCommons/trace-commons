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
//! # The attested bytes are not the bytes the harness sent
//!
//! The single most important sentence for anyone writing a surface on top of
//! this. The receipt binds **what the upstream provider received and
//! returned**, and on an IronWire route that is not the agent's own request:
//!
//! - a policy model swap re-serialises the request;
//! - the privacy filter re-serialises it wholesale, so the attested body holds
//!   filter **placeholders** where the original held real values;
//! - a cross-family route synthesises a different document entirely -- the
//!   attested request on a NEAR AI route may be a Chat Completions document
//!   built from an Anthropic one.
//!
//! And the attested response is the provider's own raw stream, not the frames
//! the client saw. Streaming is the normal case: NEAR AI's reference verifier
//! hashes the entire raw concatenated SSE text, so the response body here is
//! an event-stream document and **must not be parsed** -- reassembled content
//! would never hash to the same digest. Nothing in this module reads it; it is
//! bytes to be hashed.
//!
//! So the honest claim is *these are the bytes the provider hashed*, never
//! "this is the request the agent made". No wording anywhere may let a reader
//! assume otherwise.
//!
//! One consequence runs in our favour: because capture sits downstream of the
//! privacy filter, the attested bytes are already filtered.
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
//! # A restarted stream is unattestable
//!
//! IronWire's resilience guard restarts a stalled stream, and a restarted
//! stream records no digest -- so no receipt exists for it, and none ever
//! will. Combined with attesting the final call, a trace whose **last** call
//! was restarted mid-stream can never satisfy the requirement. That is a
//! coverage hole rather than an edge case, and it gets its own name,
//! [`WitnessError::InferenceCallUnattestable`], so an operator is not left
//! reading it as a contributor who withheld a receipt.
//!
//! The witness recognises it from [`STREAM_RESTARTED_MARKER`] on the exchange.
//! That marker is a **contract the capture side must write**, not something
//! IronWire emits today; until it does, a restarted final call reaches an
//! operator as [`WitnessError::InferenceAttestationMissing`], which is
//! fail-closed but less informative.
//!
//! # The model is not bound, and this witness cannot make it so
//!
//! `verify_receipt` supports a three-part receipt text whose leading part is
//! the model, and an earlier version of this module refused anything else.
//! That was wrong about the provider. The current NEAR AI API signs the
//! **two-part** form -- `<requestHash>:<responseHash>`, no model prefix -- and
//! supplies the model as a query parameter on retrieval
//! (`GET /v1/signature/{chat_id}?model=...`). A query parameter is not signed
//! and is chosen by whoever fetches the receipt, so it establishes nothing. A
//! policy refusing the two-part form would therefore refuse every real
//! receipt, and a model allowlist checked against it would be a control that
//! cannot fail.
//!
//! So there is no model policy here, and its absence is a **limitation of the
//! provider API**, not a decision this deployment made. The model named in the
//! attested request body is hash-bound and can be read, but it is the model
//! IronWire asked for rather than the model that served -- and on a policy
//! swap those differ, which is exactly the substitution a bound model would
//! have caught. When a three-part receipt does arrive, `verify_receipt`
//! compares its bound model against the request body's `model` and a mismatch
//! surfaces as [`WitnessError::InferenceReceiptUnverified`]; nothing more is
//! enforced until the provider signs one.
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
    RawTraceContribution, RawTraceContributionEvent, TraceContributionEnvelope,
    TraceContributionEventType,
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
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// Refuse any contribution whose last declared inference call does not
    /// carry a verified receipt.
    ///
    /// There is no model parameter, and its absence is the provider's doing
    /// rather than this deployment's: the receipts NEAR AI signs today bind no
    /// model, so an allowlist checked against one would be a control that
    /// cannot fail. See the module docs.
    pub fn required(max_body_bytes: usize) -> Result<Self, PolicyMisconfigured> {
        if max_body_bytes == 0 {
            return Err(PolicyMisconfigured);
        }
        Ok(Self {
            required: true,
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

    // Before the bodies: a restarted stream has no digest and therefore no
    // receipt, and saying so is more useful to an operator than reporting the
    // receipt they could not have obtained as missing.
    if stream_was_restarted(final_call) {
        return Err(WitnessError::InferenceCallUnattestable);
    }

    let (request_body, response_body) =
        exchange_bodies(final_call).ok_or(WitnessError::InferenceBodyNotInSession)?;

    if request_body.len() > policy.max_body_bytes || response_body.len() > policy.max_body_bytes {
        return Err(WitnessError::InferenceReceiptTooLarge);
    }

    // Best-effort, and unused by every receipt the provider signs today: the
    // two-part form binds no model, so `verify_receipt` ignores this. It is
    // read out of the hash-bound request body rather than from a field beside
    // the receipt so that if a three-part receipt ever does arrive, the model
    // it is checked against is one the receipt itself commits to instead of
    // one the caller typed. An unparseable body is not a refusal of its own:
    // the hashes are what matter, and they are checked next either way.
    let requested_model = serde_json::from_str::<Value>(request_body)
        .ok()
        .as_ref()
        .and_then(|request| request.get("model"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // One label for every `ReceiptError`; see the module's note on why a
    // re-serialised capture is indistinguishable from a forgery here.
    // The verdict is discarded rather than inspected. Its `model` is `None`
    // for every receipt the provider signs today, its hashes are the ones just
    // checked, and its `signing_address` is the inference enclave's key --
    // which this witness does not pin, because it has no attested value to pin
    // it against and a self-reported one would prove nothing.
    verify_receipt(
        receipt,
        request_body.as_bytes(),
        response_body.as_bytes(),
        &requested_model,
    )
    .map_err(|_| WitnessError::InferenceReceiptUnverified)?;

    Ok(InferenceAttestationOutcome {
        verified: 1,
        declared_calls,
    })
}

/// Remove the inference bodies from a redacted envelope, and every header map
/// beside them.
///
/// Runs after the redaction pass and **before** the digest is taken. The
/// certificate must cover the artifact the contributor actually receives, so
/// the order is redact, strip, hash, sign; a digest taken before this ran
/// would name bytes nobody holds and every downstream verification would fail
/// as an artifact mismatch. There is a test for that ordering, because it is
/// the kind of mistake that looks right.
///
/// # Why the bodies go, rather than being kept for a second verifier
///
/// They are already worthless by the time they are here. The witness redacts
/// the session it is given, bodies included, so what survives the pass no
/// longer hashes to what the receipt binds -- a downstream party trying to
/// re-verify gets `RequestHashMismatch` and has no way to tell that from
/// tampering. The only way to keep them verifiable would be to **exempt them
/// from redaction**, which means shipping raw prompts and raw completions to
/// ingest and to storage. That is strictly worse than useless bodies.
///
/// So the third option is the right one: the bodies were only ever input to a
/// check that happens once, inside the enclave, over bytes only the enclave
/// ever holds. Once that check has run they have no reader, and an artifact
/// carrying them is carrying risk in exchange for nothing.
///
/// A consequence worth stating: the bodies never reach ingest or storage at
/// all, so the payload that would have pushed an attested trace past the
/// 16 MB envelope cap does not exist downstream.
///
/// # What is stripped, and what is not
///
/// The **body fields and the header maps**, not the event. Method, URL and
/// status are ordinary trace content and a consumer may legitimately want
/// them, so they stay.
///
/// Headers go with the bodies rather than staying with the method, and that is
/// a deliberate departure from "strip only the bodies". An inference request
/// carries its credential in `Authorization`, and this repository has already
/// measured that opaque bearer tokens are **not** reliably redacted -- the
/// deterministic detector does not match them. Keeping a header map here would
/// mean shipping a live token downstream under a function whose whole purpose
/// is to remove what should not travel.
///
/// # How much of this the redaction pass already did, measured
///
/// Most of it now, and it used to be "some of it, conditionally, and the
/// condition is a string a capture chose". `BROWSER_RULES` in
/// `trace-commons-protocol` drops `body` and redacts `headers` for any event
/// whose **tool name** contains `http`, `browser` or `web`, and a name
/// matching no profile at all used to run no structural rules whatsoever --
/// so an `HttpExchange` named `inference`, or whatever an IronWire capture
/// picked, kept its request body and its `Authorization` header all the way
/// through the pass. That fallback now falls closed: an unrecognised tool
/// gets the most restrictive profile rather than none.
///
/// This function is still not redundant, for three reasons. It **removes**
/// the fields rather than leaving `[REDACTED:...]` markers a reader has to
/// reason about, so nothing downstream can mistake a marker for evidence.
/// It runs before the digest, so the certificate covers the bytes the
/// contributor actually holds. And it is enforcement inside the enclave
/// rather than a classifier table two crates away, which is where a
/// guarantee of this kind should live. Deleting it is what
/// `the_returned_artifact_carries_no_inference_bodies_or_headers` fails on.
///
/// So the guarantee moves from "a classifier profile matched the tool name" to
/// "the witness removed them", which is where a guarantee should live.
///
/// # This is what makes refusal-only enforcement compose
///
/// The certificate carries no attested-inference field, because adding one
/// needs a v2 profile with its own signing domain and a flag day across three
/// implementations of the wire format. With the bodies stripped, that
/// limitation no longer touches the artifact: a certificate exists **if and
/// only if** attestation passed, since a requiring witness issues none
/// otherwise, and the artifact carries nothing a downstream reader could
/// mistake for re-verifiable evidence.
///
/// What it does **not** fix, and stripping does not make worse: a server still
/// cannot distinguish a requiring witness from a permissive one at the same
/// measurement. The measurement pins the image, not the environment, so the
/// measurement plus the deployment's configuration is now the entire basis of
/// the claim.
pub fn strip_inference_bodies(envelope: &mut TraceContributionEnvelope) {
    for event in &mut envelope.events {
        if event.event_type != TraceContributionEventType::HttpExchange {
            continue;
        }
        // The response body, as `from_recorded_trace` places it.
        event.redacted_content = None;
        for side in ["request", "response"] {
            let Some(part) = event
                .structured_payload
                .get_mut(side)
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            // `remove`, not "set to null": a null `body` is still a body field
            // a reader has to reason about, and the point is that there is
            // nothing here to reason about.
            part.remove("body");
            part.remove("headers");
        }
        // A payload that put the bodies at the top level rather than under a
        // side. Nothing in this tree writes that shape today; it is removed
        // anyway, because the cost of being wrong about which shape a capture
        // used is a raw prompt in storage.
        if let Some(payload) = event.structured_payload.as_object_mut() {
            payload.remove("body");
            payload.remove("headers");
        }
    }
}

/// The flag a capture sets on an exchange whose stream was restarted.
///
/// Read at `structured_payload["response"][STREAM_RESTARTED_MARKER]`, as a
/// boolean. **A contract, not an observation**: nothing writes it today, and
/// this module cannot detect a restart any other way -- a restarted stream
/// looks like a stream. Until the capture side sets it, a restarted final call
/// is refused as a missing receipt rather than as an unattestable one.
pub const STREAM_RESTARTED_MARKER: &str = "stream_restarted";

/// Whether the exchange declares that its stream was restarted.
///
/// Absent means "not declared", never "did not happen". The witness has no
/// view of the stream and cannot check this claim; what it can do is refuse
/// under an accurate name when the claim is made.
fn stream_was_restarted(event: &RawTraceContributionEvent) -> bool {
    event
        .structured_payload
        .get("response")
        .and_then(|response| response.get(STREAM_RESTARTED_MARKER))
        .and_then(Value::as_bool)
        .unwrap_or(false)
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
        InferenceAttestationPolicy::required(DEFAULT_MAX_BODY_BYTES).expect("a well formed policy")
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

    /// The form the provider actually signs. An earlier version of this
    /// module refused it; refusing it would refuse every real receipt.
    #[test]
    fn a_two_part_receipt_is_the_normal_form_and_is_admitted() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let outcome = check(
            &required(),
            Some(&two_part_receipt(&request, &response)),
            &raw,
        )
        .expect("the two-part form is what NEAR AI signs today");
        assert_eq!(outcome.verified, 1);
    }

    /// A request body that is not JSON at all still verifies on its hashes.
    /// The model read out of it is best-effort and unused by a two-part
    /// receipt, so an unparseable body must not be a refusal of its own.
    #[test]
    fn an_unparseable_request_body_still_verifies_on_its_hashes() {
        let request = "not json at all, but these are the bytes that were sent";
        let response = response_body("hi");
        let raw = contribution(&[(request.to_string(), response.clone())]);
        assert_eq!(
            check(
                &required(),
                Some(&two_part_receipt(request, &response)),
                &raw
            )
            .expect("the hashes are what matter")
            .verified,
            1
        );
    }

    /// A streamed response is a raw SSE document, not JSON, and the receipt
    /// binds the whole concatenated text. Nothing here may parse it.
    #[test]
    fn a_raw_event_stream_response_verifies_as_the_bytes_it_is() {
        let request = request_body(MODEL, "hello");
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let raw = contribution(&[(request.clone(), stream.to_string())]);
        assert_eq!(
            check(&required(), Some(&two_part_receipt(&request, stream)), &raw)
                .expect("a stream is bytes like any other")
                .verified,
            1
        );
    }

    /// A restarted stream has no digest and so no receipt, ever. It gets its
    /// own name rather than reading as a contributor who withheld one.
    #[test]
    fn a_restarted_final_stream_is_unattestable_by_a_name_of_its_own() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi");
        let mut raw = contribution(&[(request.clone(), response.clone())]);
        let last = raw.events.last_mut().expect("the exchange");
        last.structured_payload["response"][STREAM_RESTARTED_MARKER] = serde_json::json!(true);

        assert_eq!(
            check(
                &required(),
                Some(&receipt(MODEL, &request, &response)),
                &raw
            ),
            Err(WitnessError::InferenceCallUnattestable),
            "a restarted stream must not be reported as a missing receipt"
        );
        // Without the marker the same exchange verifies, so the refusal is the
        // marker and not the fixture.
        let clean = contribution(&[(request.clone(), response.clone())]);
        assert!(
            check(
                &required(),
                Some(&receipt(MODEL, &request, &response)),
                &clean
            )
            .is_ok()
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
        let tight = InferenceAttestationPolicy::required(8).expect("policy");
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
            InferenceAttestationPolicy::required(0),
            Err(PolicyMisconfigured)
        );
    }
}
