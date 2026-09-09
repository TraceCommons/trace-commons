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
//! # The model, and which form of receipt binds it
//!
//! `verify_receipt` supports both forms of receipt text: the **two-part**
//! `<requestHash>:<responseHash>`, which binds no model, and the
//! **three-part** `{model}:{requestHash}:{responseHash}`, whose leading part
//! is covered by the signature.
//!
//! A hosted NEAR AI model returns the three-part form, so a real receipt names
//! its own model. That -- and only that -- is the model a pin is looked up by
//! below. The `model` query parameter on retrieval
//! (`GET /v1/signature/{chat_id}?model=...`) is not signed and is chosen by
//! whoever fetches the receipt, so it establishes nothing and is not read.
//!
//! An earlier version of this module asserted the provider signs only the
//! two-part form and concluded no model policy was possible. That was wrong
//! about the provider, and the pin built on that belief -- one provider-wide
//! gateway key -- could not have worked, because the signing key is per model.
//!
//! The model named in the attested request body is also hash-bound and is
//! read, but only as the value `verify_receipt` compares a three-part
//! receipt's bound model against; a mismatch surfaces as
//! [`WitnessError::InferenceReceiptUnverified`]. It is the model IronWire
//! asked for rather than the model that served, and on a policy swap those
//! differ -- so it is not what a pin is keyed by.
//!
//! # Who signed, not just that a signature verifies
//!
//! `verify_receipt` establishes that a well-formed signature over these bytes
//! checks out against the key the receipt itself names. Every key satisfies
//! that, the submitter's included -- so on its own it is a statement about
//! self-consistency, not about provenance.
//!
//! [`InferenceAttestationPolicy::pinning_model_keys`] is what turns it into
//! one. NEAR AI's attestation report binds an ed25519 signing key into a TDX
//! quote (`report_data == signing_address || nonce`), and the contributor
//! client compares a fetched receipt's signer against a freshly-nonced report.
//! A check the submitter runs on its own submission is not a bound: a patched
//! client skips it. The pin is the same comparison, made here, against keys
//! the operator configured.
//!
//! **Per model, and that is the first correction.** An earlier version of this
//! pinned a single `gateway_attestation.signing_address` and applied it to
//! every receipt. A Chat Completions receipt comes back `signature_kind:
//! "provider_tee"` with a signer that differs per model -- the key that
//! appears in the report's `model_attestations`, and only when the report was
//! requested with `signing_algo=ed25519`. One gateway key therefore cannot
//! match that signer, and the shipped pin refused every such receipt. The pin
//! is a map from model name to that model's attested key set, and the model is
//! read out of the **receipt's own text**, which is `{model}:{request_sha256}:
//! {response_sha256}`.
//!
//! **Both kinds, and that is the second.** Correcting the first one to consult
//! `model_attestations` *alone* over-corrected. NEAR AI issues two legitimate
//! kinds of receipt for the **same hosted model**, and the *protocol* decides
//! which:
//!
//! | request protocol | `signature_kind` | signer |
//! |---|---|---|
//! | Chat Completions (`/v1/chat/completions`) | `provider_tee` | the model's key in `model_attestations` |
//! | Responses API (`/v1/responses`) | `gateway` | the key in `gateway_attestation` |
//!
//! Both captured live against `Qwen/Qwen3.6-35B-A3B-FP8`. The Codex CLI --
//! which this project's own operator documentation recommends -- speaks the
//! Responses API exclusively, having dropped `wire_api = "chat"`, so
//! Codex-driven traffic is entirely gateway-signed and a model-pins-only
//! witness refuses all of it.
//!
//! So there are two pin sets, [`InferenceAttestationPolicy::pinning_model_keys`]
//! and [`InferenceAttestationPolicy::pinning_gateway_keys`], and the receipt's
//! own `signature_kind` selects exactly one of them. Never both: a
//! gateway-signed receipt admitted under a model pin, or the reverse, would
//! mean a key attested for one role could vouch for the other. A kind this
//! witness does not recognise selects nothing and is refused, and so is a kind
//! whose pin set was left unset on a witness that pins the other -- pinning
//! one kind is not agreement to accept the other unchecked.
//!
//! Pins rather than a live report fetch, because this module makes no
//! outbound call on the request path and one that did would be trusting a
//! report fetched over a path an attacker able to substitute a signing key is
//! also positioned to influence. The operator derives the keys out of band
//! with a nonce-bound `signing_algo=ed25519` report; see
//! `deploy/witness/README.md`. And ed25519 only: the ECDSA signer is in no
//! ed25519 attestation, so an ECDSA receipt cannot satisfy a pin.
//!
//! Dormant when unset, and a pin failure folds into
//! [`WitnessError::InferenceReceiptUnverified`] with every other receipt
//! failure -- a label of its own would make an unauthenticated route an oracle
//! for the pinned keys.
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

use std::collections::BTreeMap;

use crate::near_attestation::receipt::{
    ReceiptAlgo, ReceiptPayload, normalize_ed25519_key, signer_is_attested_for_model,
    verify_receipt,
};
use crate::near_attestation::signer_resolver::{
    AttestedSignerResolver, OperatorKeyPins, StaticPinResolver,
};

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
    /// Model name -> the ed25519 receipt-signing keys this deployment trusts
    /// for it, normalised. `None` is the dormant default; see
    /// [`Self::pinning_model_keys`].
    model_key_pins: Option<BTreeMap<String, Vec<String>>>,
    /// The ed25519 keys this deployment trusts to sign a **gateway**-kind
    /// receipt, normalised. `None` where none are pinned; see
    /// [`Self::pinning_gateway_keys`].
    gateway_key_pins: Option<Vec<String>>,
}

/// The requirement was configured in a way that would not require anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the attested-inference requirement is configured to require nothing")]
pub struct PolicyMisconfigured;

/// The configured model key pins are not a usable `model=key` set.
///
/// Carries nothing. The value is deployment configuration and, on a
/// misconfiguration, is quite possibly a key-material paste; it does not
/// belong in an error string that will be logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the pinned model signing keys are not a `model=key` set of 32-byte ed25519 keys")]
pub struct ModelKeyPinMalformed;

/// The configured gateway key pins are not a usable set of ed25519 keys.
///
/// A separate type from [`ModelKeyPinMalformed`] because the two variables
/// have different shapes -- one is `model=key`, the other is a bare key list
/// -- and an operator who mixed them up must be told which one is wrong.
/// Carries nothing, for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the pinned gateway signing keys are not a set of 32-byte ed25519 keys")]
pub struct GatewayKeyPinMalformed;

/// Parse the `key[,key...]` spelling an operator configures for the gateway.
///
/// Bare keys with no `model=` prefix, because a gateway receipt binds no
/// model: the gateway signs for whatever the Responses API served, and there
/// is no per-model entry to key a pin by. More than one is accepted for the
/// case a gateway is served by several enclaves, exactly as a model may pin
/// more than one key.
///
/// Whitespace around each element is trimmed. Everything else is refused: a
/// key that is not 32 bytes of hex, and an empty overall set. The `model=key`
/// spelling pasted into this variable by mistake is refused by that same
/// shape rule -- `=` is not a hex digit, so no element carrying one is a key.
/// There is deliberately no separate `=` branch: it could never be the arm
/// that refused, which is a control that cannot fail.
///
/// # Errors
///
/// [`GatewayKeyPinMalformed`] for each of those.
pub fn parse_gateway_key_pins(spec: &str) -> Result<Vec<String>, GatewayKeyPinMalformed> {
    let mut keys: Vec<String> = Vec::new();
    for element in spec.split(',') {
        let element = element.trim();
        // An empty element is skipped, not refused, so `key,,key` and a
        // trailing comma parse. Deliberate, and the same rule
        // `parse_model_key_pins` follows: a wrapped or hand-edited
        // environment value collects stray separators, and those are
        // punctuation rather than a claim about a key. What this cannot
        // become is a witness that appears configured and pins nothing --
        // the empty-set check below refuses a spec that was *only*
        // separators, so `,,` and `""` are still startup failures.
        if element.is_empty() {
            continue;
        }
        let key = normalize_ed25519_key(element).ok_or(GatewayKeyPinMalformed)?;
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        return Err(GatewayKeyPinMalformed);
    }
    Ok(keys)
}

/// Parse the `model=key[,model=key...]` spelling an operator configures.
///
/// One model may appear more than once, which is how a model served by
/// several enclaves is pinned: the keys accumulate into that model's set.
///
/// Whitespace around each element is trimmed, because a wrapped or
/// pretty-printed environment value is a spelling of the same set. Everything
/// else is refused: an element with no `=`, an empty model name, a key that is
/// not 32 bytes of hex, and an empty overall set. An operator who set the
/// variable to nothing would otherwise get a witness that pins nothing while
/// appearing configured.
///
/// # Errors
///
/// [`ModelKeyPinMalformed`] for each of those.
pub fn parse_model_key_pins(
    spec: &str,
) -> Result<BTreeMap<String, Vec<String>>, ModelKeyPinMalformed> {
    let mut pins: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for element in spec.split(',') {
        let element = element.trim();
        if element.is_empty() {
            continue;
        }
        // `split_once`, not `split`: a model name never contains `=`, but a
        // key never does either, and an element with two of them is not
        // something to guess at.
        let (model, key) = element.split_once('=').ok_or(ModelKeyPinMalformed)?;
        let model = model.trim();
        if model.is_empty() || key.contains('=') {
            return Err(ModelKeyPinMalformed);
        }
        let key = normalize_ed25519_key(key).ok_or(ModelKeyPinMalformed)?;
        let keys = pins.entry(model.to_string()).or_default();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if pins.is_empty() {
        return Err(ModelKeyPinMalformed);
    }
    Ok(pins)
}

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
            model_key_pins: None,
            gateway_key_pins: None,
        }
    }

    /// Refuse any contribution whose last declared inference call does not
    /// carry a verified receipt.
    ///
    /// There is no model parameter here: a *requirement* is about whether a
    /// receipt must be present, not about which model may have served. Which
    /// models are trusted, and under which keys, is
    /// [`Self::pinning_model_keys`], and the two are deliberately
    /// independent.
    pub fn required(max_body_bytes: usize) -> Result<Self, PolicyMisconfigured> {
        if max_body_bytes == 0 {
            return Err(PolicyMisconfigured);
        }
        Ok(Self {
            required: true,
            max_body_bytes,
            model_key_pins: None,
            gateway_key_pins: None,
        })
    }

    /// Trust a receipt only when the model it binds is pinned and its signer
    /// is one of that model's attested ed25519 keys.
    ///
    /// # What this closes, and what it does not
    ///
    /// Without it, `verify_receipt` establishes that *someone* signed these
    /// bytes and that the signature is self-consistent -- it does not
    /// establish *who*. NEAR AI's attestation report binds an ed25519 signing
    /// key into a TDX quote (`report_data == signing_address || nonce`), and
    /// the contributor client checks a fetched receipt's signer against a
    /// freshly-nonced report. But a client-side check is a check the submitter
    /// performs on itself: a patched client simply does not perform it. This
    /// is the same comparison made where the decision is enforced, against
    /// keys the *operator* configured.
    ///
    /// # Per model, because there is no single receipt-signing key
    ///
    /// This replaces a pin on `gateway_attestation.signing_address` that was
    /// applied to *every* receipt, which was wrong in a way that could not be
    /// made right by tightening it. The gateway key signs the Responses API
    /// form and nothing else; a Chat Completions receipt is `signature_kind:
    /// "provider_tee"` and its signer is the key that model's entry in
    /// `model_attestations` carries, which differs per model. One key applied
    /// to both kinds refused every `provider_tee` receipt.
    ///
    /// This set pins the `provider_tee` kind only. The gateway key is pinnable
    /// again under its own name -- see
    /// [`InferenceAttestationPolicy::pinning_gateway_keys`] -- and the two
    /// sets are never tried against each other.
    ///
    /// The model compared against is the one in the **receipt's own text**
    /// (`{model}:{request_sha256}:{response_sha256}`), not one read beside it,
    /// so the receipt commits to the model whose key it is checked against. A
    /// `provider_tee` receipt that binds no model cannot be placed against any
    /// model pin and is refused.
    ///
    /// # Pins rather than a fetch
    ///
    /// A witness makes no outbound network calls on the request path, and one
    /// that did would be trusting a report fetched over a path an attacker who
    /// could substitute a signing key is also positioned to influence. The
    /// operator derives each model's key set once, out of band, from a
    /// nonce-bound `signing_algo=ed25519` attestation report -- the same
    /// binding `trace_commons_attestation::receipt::model_ed25519_keys`
    /// enforces, and the same one the client applies live. See
    /// `deploy/witness/README.md` for the procedure.
    ///
    /// **ed25519 only, and that is the point.** Only ed25519 attestations
    /// carry a receipt-signing key; the ECDSA signer appears in none. A pin is
    /// therefore a 32-byte key, and an ECDSA receipt cannot satisfy one
    /// however well it verifies.
    ///
    /// Dormant by default. Unset pins leave every existing path exactly as it
    /// was.
    ///
    /// # Errors
    ///
    /// [`ModelKeyPinMalformed`] when the set is empty or any key is not 32
    /// bytes of hex -- including empty, which is how an operator who set the
    /// variable to nothing would otherwise get a witness that pins nothing
    /// while appearing configured. Refused here, where the policy is built, so
    /// a witness fails to start rather than holding pins that can never match.
    pub fn pinning_model_keys(
        mut self,
        pins: BTreeMap<String, Vec<String>>,
    ) -> Result<Self, ModelKeyPinMalformed> {
        if pins.is_empty() {
            return Err(ModelKeyPinMalformed);
        }
        let mut normalised: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (model, keys) in pins {
            if model.trim().is_empty() || keys.is_empty() {
                return Err(ModelKeyPinMalformed);
            }
            let keys = keys
                .iter()
                .map(|key| normalize_ed25519_key(key).ok_or(ModelKeyPinMalformed))
                .collect::<Result<Vec<_>, _>>()?;
            // The *trimmed* name, matching what was validated. Inserting the
            // padded one would give a programmatic caller a pin keyed by a
            // string no receipt can ever name -- configured, and unmatchable.
            // `parse_model_key_pins` already trims; this is for callers that
            // build the map themselves.
            normalised.insert(model.trim().to_string(), keys);
        }
        self.model_key_pins = Some(normalised);
        Ok(self)
    }

    /// Trust a `gateway`-kind receipt only when its signer is one of these
    /// attested ed25519 keys.
    ///
    /// # Why there are two pins and not one
    ///
    /// NEAR AI issues two legitimate kinds of receipt for the **same hosted
    /// model**, and the *protocol* decides which. A Chat Completions call gets
    /// `signature_kind: "provider_tee"`, signed by that model's key in
    /// `model_attestations`. A Responses API call gets
    /// `signature_kind: "gateway"`, signed by the single key in
    /// `gateway_attestation`. Both were captured live against
    /// `Qwen/Qwen3.6-35B-A3B-FP8`.
    ///
    /// That is not a curiosity. The Codex CLI speaks the Responses API
    /// exclusively -- it dropped `wire_api = "chat"` -- so a deployment whose
    /// contributors run Codex produces only gateway-signed receipts, and a
    /// witness pinning model keys alone refuses every one of them.
    ///
    /// # This is not the retired gateway pin
    ///
    /// An earlier release had a single `gateway_attestation.signing_address`
    /// pin that was applied to **every** receipt. That was wrong and is not
    /// coming back: it refused every `provider_tee` receipt, and reinstating
    /// its semantics would mean the gateway key could vouch for a model
    /// signer. These keys are consulted for `gateway`-kind receipts and for
    /// nothing else, and the model pins for `provider_tee` and nothing else.
    ///
    /// # Unset is refusal, once anything is pinned
    ///
    /// A witness that pins nothing at all is dormant, exactly as before. But a
    /// witness that pins *either* kind is a pinning witness, and a receipt of
    /// the kind it did not pin is checked against an empty set and refused.
    /// The alternative -- letting an unpinned kind through -- would mean an
    /// operator who pinned model keys silently accepted any gateway-signed
    /// receipt from any signer, which is the fail-open direction and precisely
    /// the gap a pin exists to close.
    ///
    /// # Errors
    ///
    /// [`GatewayKeyPinMalformed`] when the set is empty or any key is not 32
    /// bytes of hex, so a witness fails to start rather than holding pins that
    /// can never match.
    pub fn pinning_gateway_keys(
        mut self,
        keys: Vec<String>,
    ) -> Result<Self, GatewayKeyPinMalformed> {
        if keys.is_empty() {
            return Err(GatewayKeyPinMalformed);
        }
        let keys = keys
            .iter()
            .map(|key| normalize_ed25519_key(key).ok_or(GatewayKeyPinMalformed))
            .collect::<Result<Vec<_>, _>>()?;
        self.gateway_key_pins = Some(keys);
        Ok(self)
    }

    /// Whether this deployment refuses an unattested contribution.
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// How large either attested body may be.
    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }

    /// The pinned per-model signing keys, normalised, or `None` where no pins
    /// are configured.
    ///
    /// Public keys, so returning them is not a disclosure -- but they are
    /// still deployment configuration, and no caller may put one on an
    /// operational surface except as a hash prefix. The model names are
    /// likewise configuration; a count and a digest are what an operator gets.
    pub fn model_key_pins(&self) -> Option<&BTreeMap<String, Vec<String>>> {
        self.model_key_pins.as_ref()
    }

    /// The pinned gateway signing keys, normalised, or `None` where none are
    /// pinned. Same disclosure rule as [`Self::model_key_pins`]: public keys,
    /// but deployment configuration, so a count and a digest are what an
    /// operator gets.
    pub fn gateway_key_pins(&self) -> Option<&[String]> {
        self.gateway_key_pins.as_deref()
    }

    /// Whether this deployment pins receipt signers at all.
    ///
    /// True as soon as **either** kind is pinned, which is what makes the
    /// other kind refuse rather than pass unchecked. False leaves every path
    /// exactly as it was before pins existed.
    pub fn is_pinning(&self) -> bool {
        self.model_key_pins.is_some() || self.gateway_key_pins.is_some()
    }

    /// This deployment's static pins, in the shape a resolver consumes.
    ///
    /// The same two sets [`Self::model_key_pins`] and
    /// [`Self::gateway_key_pins`] return, carried across the seam so that
    /// [`check_inference_attestation`] can reach them through a
    /// [`StaticPinResolver`] and not by field access. Same disclosure rule:
    /// public keys, but deployment configuration, so no caller may render one
    /// except as a digest.
    pub fn operator_key_pins(&self) -> OperatorKeyPins {
        let mut pins = OperatorKeyPins::none();
        if let Some(model) = &self.model_key_pins {
            pins = pins.with_model_pins(model.clone());
        }
        if let Some(gateway) = &self.gateway_key_pins {
            pins = pins.with_gateway_pins(gateway.clone());
        }
        pins
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
    // The default, and it is the pre-existing behaviour exactly: the accepted
    // signer set is whatever the operator statically pinned, and a deployment
    // that pinned nothing enforces nothing. Substituting a report-backed
    // resolver here is a deliberate act by a caller of
    // [`check_inference_attestation_with`]; no configuration reachable from
    // this entry point does it.
    let resolver = StaticPinResolver::new(policy.operator_key_pins());
    // The clock a static resolver cannot consult. Passing zero rather than
    // reading a clock keeps this path free of a time dependency it does not
    // have -- and `StaticPinResolver` ignoring its clock argument is asserted,
    // so this cannot quietly become load-bearing.
    check_inference_attestation_with(policy, &resolver, offered, session, 0)
}

/// [`check_inference_attestation`], with the accepted signer set supplied by a
/// resolver rather than read off the policy.
///
/// The seam a server-side resolver substitutes at. `resolver` is a trait
/// object because the static-pin answer and the report-derived answer are the
/// same decision reached two ways -- see
/// [`crate::near_attestation::signer_resolver`] -- and a concrete type here
/// could not participate in that substitution.
///
/// `now_unix` is wall-clock time, and it is the resolver's business what to do
/// with it: a report-backed resolver ages its set out, a static one ignores it.
///
/// **Nothing in this repository calls this with a report-backed resolver
/// today.** It exists so that switching admission onto server-checked evidence
/// is a one-line change at a wiring site rather than a rewrite of this
/// function, and so that the switch can be reviewed on its own.
pub fn check_inference_attestation_with(
    policy: &InferenceAttestationPolicy,
    resolver: &dyn AttestedSignerResolver,
    offered: Option<&ReceiptPayload>,
    session: &WitnessedSession<'_>,
    now_unix: u64,
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

    // What a three-part receipt's bound model is compared against; the
    // two-part form binds no model, so `verify_receipt` ignores it there. It
    // is read out of the hash-bound request body rather than from a field
    // beside the receipt, so the comparison is against bytes the receipt
    // commits to instead of something the caller typed. An unparseable body is
    // not a refusal of its own: the hashes are what matter, and they are
    // checked next either way. Note this is the model IronWire *asked* for --
    // the pins below are keyed by the model the receipt itself names.
    let requested_model = serde_json::from_str::<Value>(request_body)
        .ok()
        .as_ref()
        .and_then(|request| request.get("model"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // One label for every `ReceiptError`; see the module's note on why a
    // re-serialised capture is indistinguishable from a forgery here.
    //
    // The verdict's hashes are the ones just checked, so they are not read
    // again. Its `model` and its `signing_address` both are: see the pins
    // below.
    let verdict = verify_receipt(
        receipt,
        request_body.as_bytes(),
        response_body.as_bytes(),
        &requested_model,
    )
    .map_err(|_| WitnessError::InferenceReceiptUnverified)?;

    // Who signed, not merely that the signature is self-consistent.
    //
    // `verify_receipt` alone says a well-formed signature over these bytes
    // verifies against the key the receipt itself names -- which any key
    // satisfies, including one the submitter holds. The pins are the
    // operator's statement of which keys may sign for which model. It is the
    // same comparison the contributor client makes against a freshly-fetched
    // attestation report, made here, where the decision is actually enforced
    // and where a patched client cannot skip it.
    //
    // The model comes from `verdict.model`, which is the leading part of the
    // receipt's signed `text` -- so the receipt itself commits to the model
    // whose key set it is checked against. A receipt binding no model (the
    // two-part form) can be placed against no pin and is refused; picking a
    // pin for it by any other means would be checking it against a model the
    // signature never covered.
    //
    // ed25519 only: the ECDSA signer is in no ed25519 attestation, so an ECDSA
    // receipt cannot satisfy a pin. The scheme is checked explicitly rather
    // than left to the string comparison, which would also reject it (a
    // 20-byte `0x` address never equals a 32-byte key) but for an incidental
    // reason that a future address format could stop being true of.
    //
    // Every one of these -- unpinned model, absent model, wrong scheme, wrong
    // signer -- folds into `InferenceReceiptUnverified`, the same single label
    // every other receipt failure uses. A distinct label would turn this
    // unauthenticated route into an oracle: a prober could learn from the
    // refusal alone which models this deployment pins and whether its receipt
    // carried a trusted key. Nothing is logged here for the same reason the
    // rest of this module logs nothing -- the keys and the signer are
    // deployment configuration on one side and caller-visible data on the
    // other, and neither belongs on a per-request surface.
    if resolver.is_enforcing() {
        // Which pin set applies is decided by the receipt's own
        // `signature_kind`, because NEAR AI signs with a different attested
        // key depending on the protocol the call used -- for the same hosted
        // model. Chat Completions yields `provider_tee`, signed per model;
        // the Responses API yields `gateway`, signed by the gateway key. The
        // Codex CLI speaks only the Responses API, so a witness that consults
        // the model pins alone refuses every Codex-driven receipt.
        //
        // Exactly one set per kind, and never both. A gateway-signed receipt
        // admitted under a model pin, or the reverse, would mean a key
        // attested for one role could vouch for the other -- which is the
        // property two separate attestations exist to deny.
        //
        // An unpinned kind resolves to the empty set and is therefore refused.
        // That is the fail-closed reading and it is deliberate: an operator
        // who pinned model keys did not thereby agree to accept any
        // gateway-signed receipt from any signer.
        //
        // Which of those the resolver applies is its own affair: the routing
        // by `signature_kind`, the empty answer for an unrecognised kind, and
        // the empty answer for an unpinned kind all live behind
        // `keys_for`. What this function keeps is the rule that an empty
        // answer refuses.
        let attested =
            resolver.keys_for(verdict.signature_kind, verdict.model.as_deref(), now_unix);
        let signed_by_a_pinned_key = verdict.signing_algo == ReceiptAlgo::Ed25519
            && signer_is_attested_for_model(&verdict.signing_address, &attested);
        if !signed_by_a_pinned_key {
            return Err(WitnessError::InferenceReceiptUnverified);
        }
    }

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

/// Verification consumes the raw carriers first. Remove them before the
/// classifier because none of these bytes belongs in the contributed artifact.
/// This also keeps a bounded model-call body from consuming metadata scan budget.
pub(super) fn strip_raw_inference_bodies(raw: &mut RawTraceContribution) {
    for event in &mut raw.events {
        if event.event_type != TraceContributionEventType::HttpExchange {
            continue;
        }
        event.content = None;
        for side in ["request", "response"] {
            if let Some(part) = event
                .structured_payload
                .get_mut(side)
                .and_then(Value::as_object_mut)
            {
                part.remove("body");
                part.remove("headers");
            }
        }
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
pub(crate) fn stream_was_restarted(event: &RawTraceContributionEvent) -> bool {
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
pub(crate) fn exchange_bodies(event: &RawTraceContributionEvent) -> Option<(&str, &str)> {
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
    use crate::near_attestation::receipt::{ReceiptAlgo, ReceiptSignatureKind};
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
            signing_algo: ReceiptAlgo::Ecdsa,
            signature_kind: ReceiptSignatureKind::Unrecognised,
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
            signing_algo: ReceiptAlgo::Ecdsa,
            signature_kind: ReceiptSignatureKind::Unrecognised,
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
        .expect("the two-part form still verifies where nothing is pinned");
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

    // ---------------------------------------------------------------
    // The per-model receipt signing-key pins.
    // ---------------------------------------------------------------

    /// Two fixed ed25519 seeds, for the same reason the secp256k1 key above
    /// is fixed: a generated key makes a failure unreproducible.
    const MODEL_A_SEED_HEX: &str =
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const IMPOSTOR_SEED_HEX: &str =
        "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
    const MODEL_B_SEED_HEX: &str =
        "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7";

    /// A second model, so per-model selection has something to select
    /// between.
    const OTHER_MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";

    /// The gateway's live ed25519 signing key, captured 2026-09-06. It is here
    /// to be *refused*: the shipped pin compared receipt signers against this
    /// key, and it signs no receipt.
    const LIVE_GATEWAY_KEY: &str =
        "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";
    /// The live `provider_tee` signer for `OTHER_MODEL`, same capture. A
    /// different key from the gateway's, which is the defect in one line.
    const LIVE_MODEL_KEY: &str = "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4";

    fn ed25519_pair(seed_hex: &str) -> ring::signature::Ed25519KeyPair {
        ring::signature::Ed25519KeyPair::from_seed_unchecked(&hex::decode(seed_hex).expect("hex"))
            .expect("seed")
    }

    /// The public key in the spelling NEAR AI renders and a pin takes: 64
    /// lowercase hex characters, no `0x`.
    fn ed25519_key_hex(seed_hex: &str) -> String {
        use ring::signature::KeyPair as _;
        hex::encode(ed25519_pair(seed_hex).public_key().as_ref())
    }

    /// A three-part ed25519 receipt over these exact bytes -- the form a
    /// hosted model returns, whose leading part is the model name the
    /// signature covers.
    fn ed25519_receipt(
        seed_hex: &str,
        model: &str,
        request_body: &str,
        response_body: &str,
    ) -> ReceiptPayload {
        let text = format!(
            "{model}:{}:{}",
            sha256_hex(request_body),
            sha256_hex(response_body)
        );
        let signature = ed25519_pair(seed_hex).sign(text.as_bytes());
        ReceiptPayload {
            signature: hex::encode(signature.as_ref()),
            signing_address: ed25519_key_hex(seed_hex),
            text,
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: ReceiptSignatureKind::ProviderTee,
        }
    }

    /// The two-part ed25519 form, which binds no model at all.
    fn two_part_ed25519_receipt(
        seed_hex: &str,
        request_body: &str,
        response_body: &str,
    ) -> ReceiptPayload {
        let text = format!("{}:{}", sha256_hex(request_body), sha256_hex(response_body));
        let signature = ed25519_pair(seed_hex).sign(text.as_bytes());
        ReceiptPayload {
            signature: hex::encode(signature.as_ref()),
            signing_address: ed25519_key_hex(seed_hex),
            text,
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: ReceiptSignatureKind::ProviderTee,
        }
    }

    fn pins(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<String>> {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (model, key) in entries {
            map.entry((*model).to_string())
                .or_default()
                .push((*key).to_string());
        }
        map
    }

    fn pinned(
        policy: InferenceAttestationPolicy,
        entries: &[(&str, &str)],
    ) -> InferenceAttestationPolicy {
        policy
            .pinning_model_keys(pins(entries))
            .expect("well formed pins")
    }

    /// The positive control for every refusal below: a receipt bound to a
    /// pinned model and signed by that model's pinned key is admitted, so the
    /// refusals are the pins and not the ed25519 fixture.
    #[test]
    fn a_receipt_signed_by_the_models_pinned_key_is_admitted() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);

        let outcome = check(
            &policy,
            Some(&ed25519_receipt(
                MODEL_A_SEED_HEX,
                MODEL,
                &request,
                &response,
            )),
            &raw,
        )
        .expect("a receipt from this model's pinned key");
        assert_eq!(
            outcome,
            InferenceAttestationOutcome {
                verified: 1,
                declared_calls: 1
            }
        );
    }

    /// The defect, as a witness-level assertion. The shipped code compared
    /// every receipt signer against one gateway key, whatever kind the receipt
    /// was. Pin that key as a *model* key -- for the right model, with
    /// everything else correct -- and a receipt signed by the model's real
    /// `provider_tee` key is refused. Both values are live captures, and they
    /// are different keys. (The gateway key does sign receipts, the Responses
    /// API ones; it signs no `provider_tee` receipt, which is what this pins.)
    #[test]
    fn the_gateway_key_pins_nothing_a_provider_tee_receipt_signs() {
        assert_ne!(
            LIVE_GATEWAY_KEY, LIVE_MODEL_KEY,
            "the gateway key and the model's receipt signer are different keys; \
             a pin on the first can never match a receipt from the second"
        );
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);

        // The shipped shape: the gateway key, trusted for this model.
        let gateway_pinned = pinned(required(), &[(MODEL, LIVE_GATEWAY_KEY)]);
        assert_eq!(
            check(
                &gateway_pinned,
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified),
            "a receipt signed by anything other than the gateway key is refused \
             under a gateway pin -- which is every chat-completions receipt"
        );

        // The fix: the model's own attested key.
        let model_pinned = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);
        assert!(
            check(
                &model_pinned,
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            )
            .is_ok()
        );
    }

    /// Per-model selection. Two models, two keys, one policy: each model's
    /// receipt is admitted under its own key and refused under the other's.
    /// A single pinned key cannot express this.
    #[test]
    fn each_models_receipt_is_checked_against_that_models_keys() {
        let policy = pinned(
            required(),
            &[
                (MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX)),
                (OTHER_MODEL, &ed25519_key_hex(MODEL_B_SEED_HEX)),
            ],
        );

        for (model, right_seed, wrong_seed) in [
            (MODEL, MODEL_A_SEED_HEX, MODEL_B_SEED_HEX),
            (OTHER_MODEL, MODEL_B_SEED_HEX, MODEL_A_SEED_HEX),
        ] {
            let request = request_body(model, "hello");
            let response = response_body("hi there");
            let raw = contribution(&[(request.clone(), response.clone())]);
            assert!(
                check(
                    &policy,
                    Some(&ed25519_receipt(right_seed, model, &request, &response)),
                    &raw
                )
                .is_ok(),
                "{model} must accept its own attested signer"
            );
            assert_eq!(
                check(
                    &policy,
                    Some(&ed25519_receipt(wrong_seed, model, &request, &response)),
                    &raw
                ),
                Err(WitnessError::InferenceReceiptUnverified),
                "{model} must not accept another model's attested signer"
            );
        }
    }

    /// A receipt bound to a model this deployment pins nothing for is refused
    /// rather than checked against some other model's keys. Both bodies and
    /// the receipt agree on the model; the only thing missing is a pin.
    #[test]
    fn a_receipt_for_an_unpinned_model_is_refused() {
        let request = request_body(OTHER_MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);

        assert_eq!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    OTHER_MODEL,
                    &request,
                    &response
                )),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified),
            "the signer is pinned, but not for this model"
        );
    }

    /// A receipt binding no model cannot be placed against any pin, so a
    /// pinning witness refuses it. Fail-closed: there is no model the
    /// signature covers, and picking one from the request body would be
    /// checking the receipt against something it never committed to.
    #[test]
    fn a_receipt_binding_no_model_cannot_satisfy_a_pin() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let two_part = two_part_ed25519_receipt(MODEL_A_SEED_HEX, &request, &response);

        // It verifies as a receipt: an unpinned witness admits it, so the
        // refusal below is the pin and not the fixture.
        assert!(check(&required(), Some(&two_part), &raw).is_ok());

        let policy = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);
        assert_eq!(
            check(&policy, Some(&two_part), &raw),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    /// The whole point. A receipt that is perfectly well signed, over exactly
    /// these bytes, for exactly this model, by a key that is not pinned.
    #[test]
    fn a_receipt_from_an_unpinned_key_is_refused_even_though_it_verifies() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let impostor = ed25519_receipt(IMPOSTOR_SEED_HEX, MODEL, &request, &response);

        // It verifies as a receipt: an unpinned witness admits it.
        assert!(
            check(&required(), Some(&impostor), &raw).is_ok(),
            "the impostor receipt must itself be valid, or the pin assertion \
             below proves nothing"
        );

        let policy = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);
        assert_eq!(
            check(&policy, Some(&impostor), &raw),
            Err(WitnessError::InferenceReceiptUnverified),
        );
    }

    /// Unset is unchanged. The same contribution and the same receipts, under
    /// a policy built the way every existing deployment builds one.
    #[test]
    fn unset_pins_leave_both_paths_exactly_as_they_were() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);

        assert!(required().model_key_pins().is_none());
        assert!(
            InferenceAttestationPolicy::not_required()
                .model_key_pins()
                .is_none()
        );
        // Every shape a witness with no pins admitted before it must keep
        // admitting: the two-part ed25519 form, the three-part one, and ECDSA.
        assert!(
            check(
                &required(),
                Some(&two_part_ed25519_receipt(
                    IMPOSTOR_SEED_HEX,
                    &request,
                    &response
                )),
                &raw
            )
            .is_ok()
        );
        assert!(
            check(
                &required(),
                Some(&ed25519_receipt(
                    IMPOSTOR_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            )
            .is_ok()
        );
        assert!(
            check(
                &required(),
                Some(&receipt(MODEL, &request, &response)),
                &raw
            )
            .is_ok()
        );
        // And a contribution carrying no receipt is unaffected either way.
        assert!(check(&InferenceAttestationPolicy::not_required(), None, &raw).is_ok());
    }

    /// The pins bind a receipt that is *offered*, not only one that is
    /// *required*. A deployment requiring nothing still must not certify a
    /// trace carrying a receipt from a key it does not trust; accepting it
    /// would be the same silent downgrade an invalid receipt already is not.
    #[test]
    fn the_pins_bind_an_offered_receipt_where_nothing_is_required() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = pinned(
            InferenceAttestationPolicy::not_required(),
            &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))],
        );

        assert_eq!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    IMPOSTOR_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified)
        );
        // A submission carrying no receipt at all is still certified: the pins
        // constrain which key is trusted, they do not impose a requirement.
        assert!(check(&policy, None, &raw).is_ok());
    }

    /// A pin is a 32-byte ed25519 key, and only an ed25519 attestation binds
    /// a receipt-signing key. An ECDSA receipt therefore cannot satisfy one,
    /// however well it verifies -- even bound to a pinned model.
    #[test]
    fn an_ecdsa_receipt_cannot_satisfy_a_model_key_pin() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);

        assert_eq!(
            check(&policy, Some(&receipt(MODEL, &request, &response)), &raw),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    /// Case and surrounding whitespace are configuration spellings of one
    /// key, not two keys.
    #[test]
    fn a_pin_is_normalised_before_it_is_compared() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let shouty = format!(
            "  {}  ",
            ed25519_key_hex(MODEL_A_SEED_HEX).to_ascii_uppercase()
        );
        let policy = pinned(required(), &[(MODEL, &shouty)]);
        assert_eq!(
            policy.model_key_pins().expect("pins").get(MODEL),
            Some(&vec![ed25519_key_hex(MODEL_A_SEED_HEX)])
        );
        assert!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            )
            .is_ok()
        );
    }

    /// Two enclaves serving one model: both attested keys are pinned under
    /// that model, and a receipt from either is admitted.
    #[test]
    fn a_model_may_pin_more_than_one_key() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = pinned(
            required(),
            &[
                (MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX)),
                (MODEL, &ed25519_key_hex(MODEL_B_SEED_HEX)),
            ],
        );
        for seed in [MODEL_A_SEED_HEX, MODEL_B_SEED_HEX] {
            assert!(
                check(
                    &policy,
                    Some(&ed25519_receipt(seed, MODEL, &request, &response)),
                    &raw
                )
                .is_ok()
            );
        }
        assert_eq!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    IMPOSTOR_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    /// Fail-closed configuration: pins that are not keys are refused where
    /// they are configured, so a witness cannot start holding ones that can
    /// never match. Empty is refused too -- an operator who set the variable
    /// to nothing must not get a witness that silently pins nothing.
    #[test]
    fn a_pin_that_is_not_a_key_is_refused_rather_than_ignored() {
        for malformed in [
            "",
            "   ",
            "0xcb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6",
            "cb6fc58f",
            "zzzzc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6",
        ] {
            assert_eq!(
                required()
                    .pinning_model_keys(pins(&[(MODEL, malformed)]))
                    .err(),
                Some(ModelKeyPinMalformed),
                "{malformed:?} is not a 32-byte hex key"
            );
        }
        assert_eq!(
            required().pinning_model_keys(BTreeMap::new()).err(),
            Some(ModelKeyPinMalformed),
            "an empty pin set is configured-but-pins-nothing"
        );
        assert_eq!(
            required()
                .pinning_model_keys(BTreeMap::from([(MODEL.to_string(), Vec::new())]))
                .err(),
            Some(ModelKeyPinMalformed),
            "a model with no keys would match nothing while reading as pinned"
        );
        assert_eq!(
            required()
                .pinning_model_keys(BTreeMap::from([(
                    "  ".to_string(),
                    vec![ed25519_key_hex(MODEL_A_SEED_HEX)]
                )]))
                .err(),
            Some(ModelKeyPinMalformed),
            "a blank model name names no model"
        );
    }

    /// The env-var spelling an operator actually types, parsed where a
    /// malformed value is a startup failure.
    #[test]
    fn the_pin_spec_parses_the_shape_an_operator_configures() {
        let spec = format!(
            " {MODEL}={} , {OTHER_MODEL}={} ",
            ed25519_key_hex(MODEL_A_SEED_HEX).to_ascii_uppercase(),
            ed25519_key_hex(MODEL_B_SEED_HEX)
        );
        let parsed = parse_model_key_pins(&spec).expect("a well formed spec");
        assert_eq!(
            parsed.get(MODEL),
            Some(&vec![ed25519_key_hex(MODEL_A_SEED_HEX)])
        );
        assert_eq!(
            parsed.get(OTHER_MODEL),
            Some(&vec![ed25519_key_hex(MODEL_B_SEED_HEX)])
        );

        // A model repeated accumulates keys rather than replacing them, and a
        // key repeated does not double.
        let repeated = format!(
            "{MODEL}={},{MODEL}={},{MODEL}={}",
            ed25519_key_hex(MODEL_A_SEED_HEX),
            ed25519_key_hex(MODEL_B_SEED_HEX),
            ed25519_key_hex(MODEL_A_SEED_HEX)
        );
        assert_eq!(
            parse_model_key_pins(&repeated).expect("repeats").get(MODEL),
            Some(&vec![
                ed25519_key_hex(MODEL_A_SEED_HEX),
                ed25519_key_hex(MODEL_B_SEED_HEX)
            ])
        );

        for bad in [
            "",
            "   ",
            ",,",
            // No `=` at all: a bare key names no model.
            &ed25519_key_hex(MODEL_A_SEED_HEX),
            // A model with no key, and a key with no model.
            &format!("{MODEL}="),
            &format!("={}", ed25519_key_hex(MODEL_A_SEED_HEX)),
            // A second `=` is not something to guess at.
            &format!("{MODEL}={}=x", ed25519_key_hex(MODEL_A_SEED_HEX)),
            &format!("{MODEL}=0x{}", ed25519_key_hex(MODEL_A_SEED_HEX)),
        ] {
            assert_eq!(
                parse_model_key_pins(bad).err(),
                Some(ModelKeyPinMalformed),
                "{bad:?} is not a usable pin spec"
            );
        }
    }

    /// Anti-oracle: every pin failure and a receipt bound to other bytes are
    /// the same `WitnessError`, so they are the same label on the wire. A
    /// prober cannot learn from a refusal whether it guessed a pinned key,
    /// nor which models this deployment pins.
    #[test]
    fn a_pin_failure_is_indistinguishable_from_any_other_receipt_failure() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);

        let unpinned_key = check(
            &policy,
            Some(&ed25519_receipt(
                IMPOSTOR_SEED_HEX,
                MODEL,
                &request,
                &response,
            )),
            &raw,
        );
        // A receipt from the pinned key, bound to a model that is not pinned.
        //
        // The bodies here are OTHER_MODEL's, deliberately. Reusing MODEL's
        // contribution would make `verify_receipt` return `ModelMismatch`
        // against the request body's model and this case would never reach
        // the pin branch at all -- it would assert the folding of a failure
        // it never provoked.
        let other_request = request_body(OTHER_MODEL, "hello");
        let other_raw = contribution(&[(other_request.clone(), response.clone())]);
        assert!(
            check(
                &required(),
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    OTHER_MODEL,
                    &other_request,
                    &response
                )),
                &other_raw
            )
            .is_ok(),
            "this receipt must be valid under no pins, or the refusal below is \
             a ModelMismatch rather than the pin branch"
        );
        let unpinned_model = check(
            &policy,
            Some(&ed25519_receipt(
                MODEL_A_SEED_HEX,
                OTHER_MODEL,
                &other_request,
                &response,
            )),
            &other_raw,
        );
        // A receipt from the pinned key, binding no model at all.
        let no_model = check(
            &policy,
            Some(&two_part_ed25519_receipt(
                MODEL_A_SEED_HEX,
                &request,
                &response,
            )),
            &raw,
        );
        // A receipt from the pinned key, over bodies that are not these.
        let wrong_bytes = check(
            &policy,
            Some(&ed25519_receipt(
                MODEL_A_SEED_HEX,
                MODEL,
                "some other request",
                &response,
            )),
            &raw,
        );
        assert_eq!(unpinned_key, wrong_bytes);
        assert_eq!(unpinned_model, wrong_bytes);
        assert_eq!(no_model, wrong_bytes);
        assert_eq!(unpinned_key, Err(WitnessError::InferenceReceiptUnverified));
    }

    // ---- Gateway-kind receipts ------------------------------------------
    //
    // The same hosted model signs with a different attested key depending on
    // the protocol: Chat Completions yields `provider_tee` (per-model key),
    // the Responses API yields `gateway` (the gateway key). Codex speaks only
    // the Responses API, so a witness with model pins alone refuses every
    // Codex-driven receipt.
    //
    // The receipts below are two-part, which is the shape a live gateway
    // receipt has -- it binds no model, because the gateway signs for
    // whatever was served.
    // ---------------------------------------------------------------------

    /// A gateway-kind receipt, in the shape the Responses API returns.
    fn gateway_receipt(seed_hex: &str, request_body: &str, response_body: &str) -> ReceiptPayload {
        let mut receipt = two_part_ed25519_receipt(seed_hex, request_body, response_body);
        receipt.signature_kind = ReceiptSignatureKind::Gateway;
        receipt
    }

    fn gateway_pinned(
        policy: InferenceAttestationPolicy,
        keys: &[&str],
    ) -> InferenceAttestationPolicy {
        policy
            .pinning_gateway_keys(keys.iter().map(|k| (*k).to_string()).collect())
            .expect("well formed gateway pins")
    }

    /// The case that was refused in production: a Codex-shaped receipt,
    /// signed by the attested gateway key, admitted by the gateway pin.
    #[test]
    fn a_gateway_receipt_signed_by_the_pinned_gateway_key_is_admitted() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = gateway_pinned(required(), &[&ed25519_key_hex(MODEL_A_SEED_HEX)]);

        let outcome = check(
            &policy,
            Some(&gateway_receipt(MODEL_A_SEED_HEX, &request, &response)),
            &raw,
        )
        .expect("a gateway receipt from the pinned gateway key");
        assert_eq!(
            outcome,
            InferenceAttestationOutcome {
                verified: 1,
                declared_calls: 1
            }
        );
    }

    /// The defect, at the witness: a witness pinning only model keys refuses
    /// a legitimate gateway receipt, however well it verifies. Adding the
    /// gateway pin is the whole fix -- same receipt, same bodies.
    #[test]
    fn a_gateway_receipt_is_refused_where_only_model_keys_are_pinned() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let receipt = gateway_receipt(MODEL_A_SEED_HEX, &request, &response);

        // The signer is a real pinned key -- for the *model* set. Routing is
        // what decides, not the key.
        let model_only = pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]);
        assert_eq!(
            check(&model_only, Some(&receipt), &raw),
            Err(WitnessError::InferenceReceiptUnverified),
            "an unpinned kind is refused, not admitted under the other pin"
        );

        let both = gateway_pinned(model_only, &[&ed25519_key_hex(MODEL_A_SEED_HEX)]);
        assert!(
            check(&both, Some(&receipt), &raw).is_ok(),
            "pinning the gateway key admits exactly this receipt"
        );
    }

    /// The mirror: a witness pinning only the gateway key refuses a
    /// `provider_tee` receipt, even one signed by that same key. Neither pin
    /// vouches for the other kind.
    #[test]
    fn a_provider_tee_receipt_is_refused_where_only_the_gateway_key_is_pinned() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = gateway_pinned(required(), &[&ed25519_key_hex(MODEL_A_SEED_HEX)]);

        assert_eq!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    /// A gateway receipt from a key that is not the pinned gateway key is
    /// refused. The pin is doing work, not merely selecting a set.
    #[test]
    fn a_gateway_receipt_from_an_unpinned_key_is_refused() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = gateway_pinned(required(), &[&ed25519_key_hex(MODEL_A_SEED_HEX)]);

        assert_eq!(
            check(
                &policy,
                Some(&gateway_receipt(IMPOSTOR_SEED_HEX, &request, &response)),
                &raw
            ),
            Err(WitnessError::InferenceReceiptUnverified)
        );
    }

    /// Both pins together admit both kinds, which is the configuration a
    /// deployment seeing chat-completions and Codex traffic needs.
    #[test]
    fn both_pins_together_admit_both_kinds() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = gateway_pinned(
            pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]),
            &[&ed25519_key_hex(MODEL_B_SEED_HEX)],
        );

        assert!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    MODEL_A_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            )
            .is_ok()
        );
        assert!(
            check(
                &policy,
                Some(&gateway_receipt(MODEL_B_SEED_HEX, &request, &response)),
                &raw
            )
            .is_ok()
        );
        // And the two sets do not leak into each other: the gateway's key
        // does not sign for the model, nor the model's for the gateway.
        assert!(
            check(
                &policy,
                Some(&ed25519_receipt(
                    MODEL_B_SEED_HEX,
                    MODEL,
                    &request,
                    &response
                )),
                &raw
            )
            .is_err()
        );
        assert!(
            check(
                &policy,
                Some(&gateway_receipt(MODEL_A_SEED_HEX, &request, &response)),
                &raw
            )
            .is_err()
        );
    }

    /// A receipt whose kind this witness does not recognise -- including one
    /// that named none -- is refused under pins, whatever key signed it. It
    /// names no source, and that is not a licence to try every pinned key.
    #[test]
    fn a_receipt_with_no_recognised_kind_is_refused_under_pins() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = gateway_pinned(
            pinned(required(), &[(MODEL, &ed25519_key_hex(MODEL_A_SEED_HEX))]),
            &[&ed25519_key_hex(MODEL_A_SEED_HEX)],
        );

        let mut receipt = ed25519_receipt(MODEL_A_SEED_HEX, MODEL, &request, &response);
        receipt.signature_kind = ReceiptSignatureKind::Unrecognised;
        assert_eq!(
            check(&policy, Some(&receipt), &raw),
            Err(WitnessError::InferenceReceiptUnverified),
            "both sets pin this exact key, and it is still refused"
        );
    }

    /// A witness that pins nothing is dormant, gateway receipts included.
    /// Pins are what make an unpinned kind refuse; their absence is not.
    #[test]
    fn a_gateway_receipt_is_admitted_where_nothing_is_pinned() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        assert!(!required().is_pinning());
        assert!(
            check(
                &required(),
                Some(&gateway_receipt(IMPOSTOR_SEED_HEX, &request, &response)),
                &raw
            )
            .is_ok()
        );
    }

    /// The gateway pin spec is bare keys, and the model-pin spelling in this
    /// variable is refused rather than half-parsed.
    #[test]
    fn the_gateway_pin_spec_parses_the_shape_an_operator_configures() {
        let spec = format!(
            " {} , {} ",
            ed25519_key_hex(MODEL_A_SEED_HEX).to_ascii_uppercase(),
            ed25519_key_hex(MODEL_B_SEED_HEX)
        );
        assert_eq!(
            parse_gateway_key_pins(&spec).expect("a well formed spec"),
            vec![
                ed25519_key_hex(MODEL_A_SEED_HEX),
                ed25519_key_hex(MODEL_B_SEED_HEX)
            ]
        );
        // A repeated key does not double.
        let repeated = format!(
            "{},{}",
            ed25519_key_hex(MODEL_A_SEED_HEX),
            ed25519_key_hex(MODEL_A_SEED_HEX)
        );
        assert_eq!(
            parse_gateway_key_pins(&repeated).expect("repeats"),
            vec![ed25519_key_hex(MODEL_A_SEED_HEX)]
        );

        // Stray separators are punctuation, not a claim about a key: a
        // doubled comma and a trailing one parse to the same set. A spec
        // that is *only* separators still fails, which is the case below.
        let stray = format!(
            ",{},,{},",
            ed25519_key_hex(MODEL_A_SEED_HEX),
            ed25519_key_hex(MODEL_B_SEED_HEX)
        );
        assert_eq!(
            parse_gateway_key_pins(&stray).expect("stray separators"),
            vec![
                ed25519_key_hex(MODEL_A_SEED_HEX),
                ed25519_key_hex(MODEL_B_SEED_HEX)
            ]
        );

        for bad in [
            "",
            "   ",
            ",,",
            // The model-pin spelling, in the wrong variable.
            &format!("{MODEL}={}", ed25519_key_hex(MODEL_A_SEED_HEX)),
            &format!("0x{}", ed25519_key_hex(MODEL_A_SEED_HEX)),
            "not-a-key",
        ] {
            assert_eq!(
                parse_gateway_key_pins(bad).err(),
                Some(GatewayKeyPinMalformed),
                "{bad:?} is not a usable gateway pin spec"
            );
        }
        // And a set that is empty, or holds a key that is not one, is
        // refused where the policy is built rather than becoming a control
        // that can never match.
        assert_eq!(
            InferenceAttestationPolicy::not_required()
                .pinning_gateway_keys(vec![])
                .err(),
            Some(GatewayKeyPinMalformed)
        );
        assert_eq!(
            InferenceAttestationPolicy::not_required()
                .pinning_gateway_keys(vec!["nope".to_string()])
                .err(),
            Some(GatewayKeyPinMalformed)
        );
    }

    /// Anti-oracle, for the gateway pin too: a gateway receipt refused by the
    /// pin is the same `WitnessError` as one bound to other bytes, so the two
    /// are the same label on the wire.
    #[test]
    fn a_gateway_pin_failure_is_indistinguishable_from_any_other_receipt_failure() {
        let request = request_body(MODEL, "hello");
        let response = response_body("hi there");
        let raw = contribution(&[(request.clone(), response.clone())]);
        let policy = gateway_pinned(required(), &[&ed25519_key_hex(MODEL_A_SEED_HEX)]);

        let unpinned_key = check(
            &policy,
            Some(&gateway_receipt(IMPOSTOR_SEED_HEX, &request, &response)),
            &raw,
        );
        let wrong_bytes = check(
            &policy,
            Some(&gateway_receipt(
                MODEL_A_SEED_HEX,
                "some other request",
                &response,
            )),
            &raw,
        );
        let unpinned_kind = check(
            &policy,
            Some(&ed25519_receipt(
                MODEL_A_SEED_HEX,
                MODEL,
                &request,
                &response,
            )),
            &raw,
        );
        assert_eq!(unpinned_key, wrong_bytes);
        assert_eq!(unpinned_kind, wrong_bytes);
        assert_eq!(unpinned_key, Err(WitnessError::InferenceReceiptUnverified));
    }
}
