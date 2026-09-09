// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Receipt-bound admission evidence. Redaction v1 alone never grants admission.
use crate::near_attestation::receipt::{
    ReceiptAlgo, ReceiptPayload, ReceiptSignatureKind, verify_receipt,
};
use crate::redaction_witness::verification::{VerifiedWitnessCertificate, WitnessPin};
use crate::witness_service::{Signer, WitnessContributionResponse};
use std::collections::BTreeSet;
use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, AdmissionSignatureKind, EVIDENCE_DOMAIN,
    REQUEST_METADATA_KEY, hash_hex, is_hash, receipt_identity,
};
use trace_commons_protocol::trace_contribution::{
    RawTraceContribution, TraceContributionEventType,
};

/// Which keys may vouch for an admission receipt, kept in one set per
/// signature kind and never merged.
///
/// The same reasoning as `check_inference_attestation`
/// (`witness_service/inference.rs`): NEAR AI signs with a different attested
/// key depending on the protocol the call used, and a key attested for one
/// role must not be able to vouch for the other -- that is the property two
/// separate attestations exist to deny.
///
/// It matters more here than there, because the two forms do not bind the
/// same thing. The three-part provider-TEE receipt commits to the model; the
/// two-part gateway receipt binds no model at all, so `accepts_request` can
/// only compare the accepted-model list against a model read out of the
/// request body the caller supplied. Admitting a gateway receipt against the
/// provider-TEE set would silently downgrade that binding from
/// provider-attested to body-asserted. An operator who wants the weaker form
/// says so by configuring `..._GATEWAY_SIGNERS`; absent, the set is empty and
/// every gateway receipt is refused.
#[derive(Clone)]
pub struct AdmissionProviderTrust {
    provider_tee_signers: BTreeSet<String>,
    gateway_signers: BTreeSet<String>,
    models: BTreeSet<String>,
    min_request_bytes: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("admission_evidence_refused")]
pub struct AdmissionEvidenceError;
impl AdmissionProviderTrust {
    pub fn new(
        provider_tee_keys: impl IntoIterator<Item = String>,
        gateway_keys: impl IntoIterator<Item = String>,
        models: impl IntoIterator<Item = String>,
        min_request_bytes: u64,
    ) -> Result<Self, AdmissionEvidenceError> {
        let provider_tee_signers: BTreeSet<_> = provider_tee_keys.into_iter().collect();
        let gateway_signers: BTreeSet<_> = gateway_keys.into_iter().collect();
        let models: BTreeSet<_> = models.into_iter().collect();
        // Neither signer set is required, but between them they must name at
        // least one key -- a deployment trusting no signer admits nothing and
        // is a configuration mistake, not a policy. A gateway-only deployment
        // is expressible, and so is a provider-TEE-only one.
        //
        // `models` may now be empty, because after the kind split it only
        // governs provider-TEE receipts. An operator running gateway-only has
        // no list to write, and requiring one would mean writing a list that
        // is never consulted.
        if (provider_tee_signers.is_empty() && gateway_signers.is_empty())
            || provider_tee_signers
                .union(&gateway_signers)
                .any(|s| !is_hash(s))
            // A key in both sets is one key holding both roles, which is the
            // thing the split exists to make impossible to express.
            || provider_tee_signers.intersection(&gateway_signers).count() != 0
            // An empty list is "no provider-TEE receipt is admissible", which
            // is coherent. A list containing junk is a typo, and admitting it
            // would silently refuse the model the operator meant to allow.
            || models
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.trim() != s)
            || min_request_bytes == 0
        {
            return Err(AdmissionEvidenceError);
        }
        Ok(Self {
            provider_tee_signers,
            gateway_signers,
            models,
            min_request_bytes,
        })
    }
    pub fn from_env(prefix: &str) -> Result<Self, AdmissionEvidenceError> {
        let read = |suffix| {
            std::env::var(format!("{prefix}_{suffix}")).map_err(|_| AdmissionEvidenceError)
        };
        let split = |raw: String| {
            raw.split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        Self::new(
            split(read("PROVIDER_SIGNERS")?),
            // Optional, and empty means "no gateway receipt is admissible".
            // That is the fail-closed reading and it is deliberate: an
            // operator who pinned provider-TEE keys did not thereby agree to
            // accept a receipt that binds no model.
            std::env::var(format!("{prefix}_GATEWAY_SIGNERS"))
                .map(split)
                .unwrap_or_default(),
            split(read("ACCEPTED_MODELS")?),
            read("MIN_REQUEST_BYTES")?
                .parse()
                .map_err(|_| AdmissionEvidenceError)?,
        )
    }
    /// Whether `signer` may vouch for a receipt of this kind. `Unrecognised`
    /// names no key source, so there is nothing to check it against -- not a
    /// licence to try both sets.
    pub fn accepts_kind(&self, kind: ReceiptSignatureKind, signer: &str) -> bool {
        let signers = match kind {
            ReceiptSignatureKind::ProviderTee => &self.provider_tee_signers,
            ReceiptSignatureKind::Gateway => &self.gateway_signers,
            ReceiptSignatureKind::Unrecognised => return false,
        };
        is_hash(signer) && signers.contains(signer)
    }
    /// The kind this **signer** establishes, independent of anything the
    /// caller claimed.
    ///
    /// `signature_kind` on the receipt is a wire label NEAR AI does not sign.
    /// It is sound as a *selector* -- a wrong claim only sends the receipt at
    /// a stricter set and fails -- and unsound as a *record*: writing it into
    /// the evidence would have the witness signing a statement whose kind
    /// field is attacker-chosen text that happened not to change the
    /// accept/reject decision, and a consumer would read that as something the
    /// witness vouched for.
    ///
    /// The signer answers it instead, and can, because
    /// [`AdmissionProviderTrust::new`] refuses a key present in both sets. The
    /// sets are therefore disjoint by construction, at most one contains any
    /// signer, and the order of the two lookups below cannot matter.
    ///
    /// **Do not replace this with `receipt.signature_kind`.** It is the same
    /// value in every accepted case, which is exactly what makes the
    /// substitution look harmless.
    pub fn established_kind(&self, signer: &str) -> Option<AdmissionSignatureKind> {
        if !is_hash(signer) {
            return None;
        }
        if self.provider_tee_signers.contains(signer) {
            return Some(AdmissionSignatureKind::ProviderTee);
        }
        if self.gateway_signers.contains(signer) {
            return Some(AdmissionSignatureKind::Gateway);
        }
        None
    }

    /// Kind-agnostic membership, for the ingest-side re-check only.
    ///
    /// [`AdmissionEvidence`] carries no signature kind, so ingest cannot
    /// reconstruct which set applied. It does not need to: the kind was
    /// decided by [`accepts_kind`](Self::accepts_kind) at certification time,
    /// inside the witness, and the witness pin's signature over the evidence
    /// is what ingest verifies. This is a defence-in-depth membership check on
    /// top of that signature, not the control that separates the two roles.
    pub fn accepts(&self, signer: &str) -> bool {
        is_hash(signer)
            && (self.provider_tee_signers.contains(signer) || self.gateway_signers.contains(signer))
    }
    /// The size floor always; the accepted-model list only where the model is
    /// attested.
    ///
    /// **The model list is not a control on a gateway receipt.** That
    /// receipt's model string is read out of the request body the caller
    /// supplied and nothing signed it, so checking it filters on the caller's
    /// own claim -- it refuses an honest caller naming an unlisted model and
    /// stops nobody who names a listed one. What it did do was refuse
    /// frontier traffic, which is the opposite of the intent.
    ///
    /// On a provider-TEE receipt the model is inside the signed text, checked
    /// by [`verify_receipt`] against a key attested for that model, so the
    /// list means what it says and still applies.
    ///
    /// `MIN_REQUEST_BYTES` applies to both: it is a fact about the exchange
    /// the receipt covers, not about anything the caller asserts.
    pub fn accepts_request(
        &self,
        kind: ReceiptSignatureKind,
        model: &str,
        request_bytes: u64,
    ) -> bool {
        if request_bytes < self.min_request_bytes {
            return false;
        }
        match kind {
            ReceiptSignatureKind::ProviderTee => self.models.contains(model),
            ReceiptSignatureKind::Gateway => true,
            // Names no key source, so it is refused wherever one is needed.
            ReceiptSignatureKind::Unrecognised => false,
        }
    }
}

/// What, if anything, evidences the model an admitted call names.
///
/// The two forms of receipt do not bind the same thing, and the difference is
/// the whole reason this type exists rather than a bare `String`. A three-part
/// provider-TEE receipt puts the model inside the signed text, signed by a key
/// attested for that model: the model is evidenced. A two-part gateway receipt
/// binds the bytes and names no model, and the gateway key is shared across
/// every hosted model behind it, so the only thing saying which model answered
/// is the request body the caller supplied.
///
/// Credit scoring, per-model tiers, and any served-model field must read
/// [`VerifiedAdmissionCall::receipt_bound_model`] and never
/// [`VerifiedAdmissionCall::requested_model`]. Nothing consumes either today;
/// that is exactly why the distinction is recorded now, while it is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAttribution {
    /// The receipt's own signed text committed to this model.
    ReceiptBound,
    /// The request asked for this model. Nothing attested that it answered.
    RequestAsserted,
}

/// Only constructed after receipt verification, exact body binding, and provider trust.
pub struct VerifiedAdmissionCall {
    binding: AdmissionBinding,
    provider_signer: String,
    request_hash: String,
    response_hash: String,
    /// The model named in the request body. `model_attribution` says whether
    /// anything beyond that body vouches for it; read it through the
    /// accessors rather than directly.
    model: String,
    model_attribution: ModelAttribution,
    /// Which signature was checked. Recorded so [`Self::certify`] can put it
    /// in the signed evidence rather than leaving a holder to re-derive it.
    signature_kind: AdmissionSignatureKind,
    request_bytes: u64,
}

pub fn verify_admission_call(
    raw: &RawTraceContribution,
    receipt: &ReceiptPayload,
    trust: &AdmissionProviderTrust,
    now: i64,
    max_body_bytes: usize,
) -> Result<VerifiedAdmissionCall, AdmissionEvidenceError> {
    // This receipt profile binds exactly one exchange. Companion events are
    // not evidence-covered, even if their source claims they are the same
    // session. Ordinary invited contributions do not use this boundary.
    if raw.events.len() != 1 {
        return Err(AdmissionEvidenceError);
    }
    let event = raw
        .events
        .iter()
        .rev()
        .find(|e| e.event_type == TraceContributionEventType::HttpExchange)
        .ok_or(AdmissionEvidenceError)?;
    if crate::witness_service::inference::stream_was_restarted(event) {
        return Err(AdmissionEvidenceError);
    }
    let (request, response) =
        crate::witness_service::inference::exchange_bodies(event).ok_or(AdmissionEvidenceError)?;
    if max_body_bytes == 0 || request.len() > max_body_bytes || response.len() > max_body_bytes {
        return Err(AdmissionEvidenceError);
    }
    let body: serde_json::Value =
        serde_json::from_str(request).map_err(|_| AdmissionEvidenceError)?;
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    // `signature_kind` is the caller's wire label here, and it can only ever
    // narrow: claiming `ProviderTee` adds the model check, and claiming
    // `Gateway` drops it but then requires a signer in the gateway set, which
    // `accepts_kind` enforces below. A caller cannot use the label to escape
    // a list its own signer is subject to.
    if receipt.signing_algo != ReceiptAlgo::Ed25519
        || !trust.accepts_request(receipt.signature_kind, model, request.len() as u64)
    {
        return Err(AdmissionEvidenceError);
    }
    let verified = verify_receipt(receipt, request.as_bytes(), response.as_bytes(), model)
        .map_err(|_| AdmissionEvidenceError)?;
    // `signature_kind` is a client-supplied wire label, not covered by the
    // receipt signature, so it cannot be trusted to *widen* anything. It is
    // used only to select which set the signer must be in, and every set is
    // narrower than their union; claiming the other kind therefore only ever
    // costs the caller its own admission. `Unrecognised` selects no set.
    if !trust.accepts_kind(receipt.signature_kind, &verified.signing_address) {
        return Err(AdmissionEvidenceError);
    }
    let binding = AdmissionBinding::parse(
        body.get("metadata")
            .and_then(|v| v.get(REQUEST_METADATA_KEY))
            .and_then(|v| v.as_str())
            .ok_or(AdmissionEvidenceError)?,
    )
    .map_err(|_| AdmissionEvidenceError)?;
    if now < 0 || binding.expires_at <= now {
        return Err(AdmissionEvidenceError);
    }
    // Both conditions, and neither alone. `verdict.model` is `Some` only for
    // the three-part text, so the signature covers the model -- but a gateway
    // key signing a three-part text still says nothing about which model
    // answered, because that one key vouches for every model behind the
    // gateway. `ProviderTee` alone is not enough either: the kind is a wire
    // label the signature does not cover, so on its own it would let a caller
    // *claim* its way to an attributed model. Requiring both means the label
    // can only ever narrow what this call is credited with.
    let model_attribution = match (verified.signature_kind, verified.model.as_deref()) {
        (ReceiptSignatureKind::ProviderTee, Some(_)) => ModelAttribution::ReceiptBound,
        _ => ModelAttribution::RequestAsserted,
    };
    // From the signer, never from `verified.signature_kind` -- which is the
    // caller's own label carried through `verify_receipt` unchanged
    // (`receipt.rs` assigns `payload.signature_kind` verbatim). The evidence
    // below is signed by the witness, so what goes into it has to be
    // something the server established.
    let signature_kind = trust
        .established_kind(&verified.signing_address)
        .ok_or(AdmissionEvidenceError)?;
    Ok(VerifiedAdmissionCall {
        binding,
        provider_signer: verified.signing_address,
        request_hash: verified.request_sha256,
        response_hash: verified.response_sha256,
        model: model.to_string(),
        model_attribution,
        signature_kind,
        request_bytes: request.len() as u64,
    })
}

impl VerifiedAdmissionCall {
    /// The model the receipt itself commits to, or `None` when nothing but
    /// the request body says which model answered.
    ///
    /// This is the only accessor a served-model claim may be derived from.
    pub fn receipt_bound_model(&self) -> Option<&str> {
        match self.model_attribution {
            ModelAttribution::ReceiptBound => Some(&self.model),
            ModelAttribution::RequestAsserted => None,
        }
    }

    /// The model the request asked for.
    ///
    /// Admission policy compares this against the operator's accepted-model
    /// list, which is a statement about what this deployment is willing to
    /// admit -- not a statement about what served the call. Do not put it in
    /// a scoring input, a per-model tier or a served-model field; use
    /// [`receipt_bound_model`](Self::receipt_bound_model) there.
    pub fn requested_model(&self) -> &str {
        &self.model
    }

    /// A receipt does not certify the importer's tool outcomes, replay claims,
    /// cost estimates or companion metadata. Retain only the verified exchange
    /// and source provenance before redaction and contributor review.
    pub(crate) fn restrict_contribution(
        &self,
        raw: &mut RawTraceContribution,
    ) -> Result<(), AdmissionEvidenceError> {
        if raw.events.len() != 1 {
            return Err(AdmissionEvidenceError);
        }
        let (request, response) =
            crate::witness_service::inference::exchange_bodies(&raw.events[0])
                .ok_or(AdmissionEvidenceError)?;
        if hash_hex(request.as_bytes()) != self.request_hash
            || hash_hex(response.as_bytes()) != self.response_hash
        {
            return Err(AdmissionEvidenceError);
        }
        let response = response.to_owned();
        let payload = serde_json::json!({"request": {"body": request}, "response": {}});
        raw.outcome = Default::default();
        raw.value = Default::default();
        raw.embedding_analysis = None;
        raw.conversation_id = None;
        raw.replay = trace_commons_protocol::trace_contribution::ReplayMetadata {
            replayable: false,
            required_tools: Vec::new(),
            tool_manifest_hashes: Default::default(),
            expected_assertions: Vec::new(),
            replay_notes: Vec::new(),
        };
        raw.ironclaw.feature_flags.retain(|key, _| key == "agent");
        // Only when the receipt committed to it. A gateway-admitted call
        // leaves this `None` rather than repeating the request body's claim
        // as though the server had verified it: the envelope is what a
        // consumer reads, and an unattested model there is indistinguishable
        // from an attested one.
        raw.ironclaw.model_name = self.receipt_bound_model().map(str::to_string);
        raw.ironclaw.engine_version = None;
        let event = &mut raw.events[0]; // verify_admission_call required exactly one.
        event.structured_payload = payload;
        event.content = Some(response);
        event.parent_event_id = None;
        event.tool_name = None;
        event.tool_call_id = None;
        event.latency_ms = None;
        event.token_counts = None;
        event.cost_usd = None;
        event.success = None;
        event.failure_modes.clear();
        Ok(())
    }

    /// Called only after the redactor has produced the immutable returned envelope.
    pub fn certify(
        self,
        response: &WitnessContributionResponse,
        signer: &dyn Signer,
        now: i64,
    ) -> Result<(AdmissionEvidence, String), AdmissionEvidenceError> {
        let evidence = AdmissionEvidence {
            profile: EVIDENCE_DOMAIN.into(),
            account_anchor_sha256: self.binding.account_anchor_sha256.clone(),
            challenge_sha256: self.binding.digest().map_err(|_| AdmissionEvidenceError)?,
            provider_signer: self.provider_signer.clone(),
            signature_kind: self.signature_kind,
            model: self.model,
            request_bytes: self.request_bytes,
            request_sha256: self.request_hash.clone(),
            response_sha256: self.response_hash.clone(),
            receipt_sha256: receipt_identity(
                &self.provider_signer,
                &self.request_hash,
                &self.response_hash,
            )
            .map_err(|_| AdmissionEvidenceError)?,
            artifact_sha256: hash_hex(&response.envelope_bytes),
            witness_measurement: response.certificate.claimed_witness_measurement().into(),
            redaction_policy_version: response
                .certificate
                .claimed_redaction_policy_version()
                .into(),
            issued_at: now,
            expires_at: self.binding.expires_at,
        };
        let signature = signer
            .sign_eip191(
                &evidence
                    .signing_bytes()
                    .map_err(|_| AdmissionEvidenceError)?,
            )
            .map_err(|_| AdmissionEvidenceError)?;
        Ok((evidence, signature))
    }
}

/// A verified v1 artifact is necessary, but only this additional signature profile
/// binds an account challenge and trusted provider receipt to that artifact.
pub fn verify_admission_evidence(
    evidence: &AdmissionEvidence,
    signature: &str,
    artifact: &VerifiedWitnessCertificate,
    witness_pin: &WitnessPin,
    provider_trust: &AdmissionProviderTrust,
    account_anchor: &str,
    now: i64,
) -> Result<(), AdmissionEvidenceError> {
    let bytes = evidence
        .signing_bytes()
        .map_err(|_| AdmissionEvidenceError)?;
    if !witness_pin.verifies_detached(&bytes, signature)
        || !provider_trust.accepts(&evidence.provider_signer)
        // The kind is inside the bytes the witness pin signed, so this side
        // can apply the same rule the witness did rather than the
        // kind-agnostic one it was stuck with.
        || !provider_trust.accepts_request(
            match evidence.signature_kind {
                AdmissionSignatureKind::ProviderTee => ReceiptSignatureKind::ProviderTee,
                AdmissionSignatureKind::Gateway => ReceiptSignatureKind::Gateway,
            },
            &evidence.model,
            evidence.request_bytes,
        )
        || evidence.account_anchor_sha256 != account_anchor
        || evidence.artifact_sha256 != artifact.redacted_sha256()
        || evidence.witness_measurement != artifact.witness_measurement()
        || evidence.redaction_policy_version != artifact.redaction_policy_version()
        || evidence.issued_at > now
        || evidence.expires_at <= now
        || receipt_identity(
            &evidence.provider_signer,
            &evidence.request_sha256,
            &evidence.response_sha256,
        )
        .map_err(|_| AdmissionEvidenceError)?
            != evidence.receipt_sha256
    {
        return Err(AdmissionEvidenceError);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_requires_explicit_canonical_keys_models_and_positive_floor() {
        let key = "a".repeat(64);
        let none = Vec::new;
        let good = || ["operator-approved-model".to_string()];
        assert!(AdmissionProviderTrust::new([key.clone()], none(), good(), 1).is_ok());
        assert!(AdmissionProviderTrust::new(none(), none(), good(), 1).is_err());
        assert!(
            AdmissionProviderTrust::new([format!("0x{}", "a".repeat(40))], none(), good(), 1)
                .is_err()
        );
        assert!(AdmissionProviderTrust::new([key.to_uppercase()], none(), good(), 1).is_err());
        // An empty model list is now a policy, not a mistake: after the kind
        // split it governs provider-TEE receipts only, so a gateway-only
        // deployment has no list to write. It means "no provider-TEE receipt
        // is admissible", which `a_gateway_receipt_is_not_filtered_by_the_model_list`
        // pins.
        assert!(AdmissionProviderTrust::new([key.clone()], none(), none(), 1).is_ok());
        assert!(AdmissionProviderTrust::new(none(), [key.clone()], none(), 1).is_ok());
        assert!(AdmissionProviderTrust::new([key.clone()], none(), ["".into()], 1).is_err());
        assert!(
            AdmissionProviderTrust::new([key.clone()], none(), [" padded ".into()], 1).is_err()
        );
        assert!(AdmissionProviderTrust::new([key.clone()], none(), good(), 0).is_err());
        // A gateway set is optional but is held to the same key shape, and a
        // key may not hold both roles.
        assert!(
            AdmissionProviderTrust::new([key.clone()], ["not-a-hash".into()], good(), 1).is_err()
        );
        assert!(AdmissionProviderTrust::new([key.clone()], [key.clone()], good(), 1).is_err());
    }

    /// The accepted-model list is a control on an attested model and nothing
    /// on an asserted one.
    ///
    /// A gateway receipt's model string is read out of the request body the
    /// caller supplied, and no signature covers it. Filtering on it refused an
    /// honest caller naming an unlisted model while stopping nobody who names
    /// a listed one -- so all it did was turn away frontier traffic. The size
    /// floor is a fact about the exchange the receipt covers and still
    /// applies to both.
    #[test]
    fn a_gateway_receipt_is_not_filtered_by_the_model_list() {
        let key = "a".repeat(64);
        let trust = AdmissionProviderTrust::new(
            [key.clone()],
            [key.replace('a', "b")],
            ["operator-approved-model".to_string()],
            10,
        )
        .expect("a deployment trusting both kinds");

        for kind in [
            ReceiptSignatureKind::ProviderTee,
            ReceiptSignatureKind::Gateway,
        ] {
            assert!(
                trust.accepts_request(kind, "operator-approved-model", 10),
                "a listed model at the floor is admissible either way"
            );
            assert!(
                !trust.accepts_request(kind, "operator-approved-model", 9),
                "the size floor is about the exchange, so it binds both kinds"
            );
        }
        assert!(
            trust.accepts_request(ReceiptSignatureKind::Gateway, "some-frontier-model", 10),
            "an unlisted model on a gateway receipt was refused by a filter \
             applied to the caller's own unattested string"
        );
        assert!(
            !trust.accepts_request(ReceiptSignatureKind::ProviderTee, "some-frontier-model", 10),
            "the list still governs the kind whose model the signature covers"
        );
        assert!(
            !trust.accepts_request(
                ReceiptSignatureKind::Unrecognised,
                "operator-approved-model",
                10
            ),
            "a kind naming no key source is refused wherever one is needed"
        );
    }

    /// A deployment that trusts only gateway keys needs no model list, and one
    /// with an empty list admits no provider-TEE receipt at all.
    #[test]
    fn an_empty_model_list_refuses_every_attested_model() {
        let key = "a".repeat(64);
        let trust = AdmissionProviderTrust::new([], [key], [], 1)
            .expect("a gateway-only deployment is expressible");
        assert!(trust.accepts_request(ReceiptSignatureKind::Gateway, "anything", 1));
        assert!(!trust.accepts_request(ReceiptSignatureKind::ProviderTee, "anything", 1));
    }

    /// A mislabelled receipt is refused outright, never admitted under the
    /// looser rule.
    ///
    /// This is what makes the kind-aware model check safe. A caller holding a
    /// provider-TEE key whose model is not on the list could otherwise claim
    /// `gateway` and skip the check. They cannot: the label only selects which
    /// set the signer must be in, and a provider-TEE signer is not in the
    /// gateway set, so the receipt is refused rather than taking the gateway
    /// path.
    ///
    /// It rests entirely on the two sets being disjoint, which
    /// [`AdmissionProviderTrust::new`] enforces. Remove that and this fails --
    /// verified by mutation, not assumed: allowing a key in both sets makes
    /// `accepts_kind(Gateway, provider_key)` true and the mislabelled receipt
    /// is admitted.
    #[test]
    fn a_mislabelled_receipt_is_refused_rather_than_admitted_under_the_looser_rule() {
        let provider = "a".repeat(64);
        let gateway = "b".repeat(64);
        let trust = AdmissionProviderTrust::new(
            [provider.clone()],
            [gateway.clone()],
            ["operator-approved-model".to_string()],
            1,
        )
        .expect("both kinds configured");

        // The claim that would skip the model list, from a key that is not a
        // gateway key.
        assert!(
            !trust.accepts_kind(ReceiptSignatureKind::Gateway, &provider),
            "a provider-TEE signer claiming gateway would take the looser path"
        );
        // And the mirror: a gateway key cannot buy the attested reading.
        assert!(!trust.accepts_kind(ReceiptSignatureKind::ProviderTee, &gateway));

        // Whatever a caller claims, the recorded kind follows the signer.
        assert_eq!(
            trust.established_kind(&provider),
            Some(AdmissionSignatureKind::ProviderTee)
        );
        assert_eq!(
            trust.established_kind(&gateway),
            Some(AdmissionSignatureKind::Gateway)
        );
        assert_eq!(trust.established_kind(&"c".repeat(64)), None);
        assert_eq!(trust.established_kind("not-a-hash"), None);
    }

    /// The two vocabularies must not drift.
    ///
    /// `AdmissionSignatureKind` lives in the protocol crate, which does not
    /// depend on the attestation crate, so the wire spellings are written
    /// twice. This is the only place both types are visible; without it a
    /// rename on either side would leave evidence naming a kind the other
    /// half cannot recognise, and the signature would still verify.
    #[test]
    fn the_signed_kind_spells_what_the_receipt_kind_spells() {
        assert_eq!(
            AdmissionSignatureKind::Gateway.as_wire(),
            ReceiptSignatureKind::Gateway.as_wire().expect("a spelling")
        );
        assert_eq!(
            AdmissionSignatureKind::ProviderTee.as_wire(),
            ReceiptSignatureKind::ProviderTee
                .as_wire()
                .expect("a spelling")
        );
        assert_eq!(ReceiptSignatureKind::Unrecognised.as_wire(), None);
        assert!(AdmissionSignatureKind::ProviderTee.binds_model());
        assert!(!AdmissionSignatureKind::Gateway.binds_model());
    }

    /// Selecting the pin set by `signature_kind` is the whole of the split.
    /// A gateway-signed receipt admitted under a provider-TEE key, or the
    /// reverse, would mean a key attested for one role could vouch for the
    /// other. Deleting either arm of `accepts_kind` fails here.
    #[test]
    fn a_signer_vouches_only_for_the_kind_it_was_configured_under() {
        let provider = "a".repeat(64);
        let gateway = "b".repeat(64);
        let trust = AdmissionProviderTrust::new(
            [provider.clone()],
            [gateway.clone()],
            ["operator-approved-model".to_string()],
            1,
        )
        .unwrap();

        assert!(trust.accepts_kind(ReceiptSignatureKind::ProviderTee, &provider));
        assert!(trust.accepts_kind(ReceiptSignatureKind::Gateway, &gateway));
        assert!(!trust.accepts_kind(ReceiptSignatureKind::ProviderTee, &gateway));
        assert!(!trust.accepts_kind(ReceiptSignatureKind::Gateway, &provider));
        for signer in [&provider, &gateway] {
            assert!(!trust.accepts_kind(ReceiptSignatureKind::Unrecognised, signer));
        }
    }

    /// Absent gateway configuration is an empty set, not an unchecked one.
    #[test]
    fn no_gateway_configuration_refuses_every_gateway_receipt() {
        let provider = "a".repeat(64);
        let trust = AdmissionProviderTrust::new(
            [provider.clone()],
            Vec::new(),
            ["operator-approved-model".to_string()],
            1,
        )
        .unwrap();
        assert!(!trust.accepts_kind(ReceiptSignatureKind::Gateway, &provider));
    }

    const FIXTURE_MODEL: &str = "fixture-model";

    /// One admissible call: a single `HttpExchange`, a binding the request
    /// body carries, and a receipt over exactly those bytes. `bound_model`
    /// picks the receipt's signed form -- `Some` is the three-part text that
    /// commits to a model, `None` the two-part text that commits to none.
    fn admission_fixture(
        bound_model: Option<&str>,
        kind: ReceiptSignatureKind,
    ) -> (RawTraceContribution, ReceiptPayload, String, i64) {
        use ring::signature::KeyPair as _;
        use trace_commons_protocol::admission::AdmissionBinding;
        use trace_commons_protocol::trace_contribution::{
            RawTraceCaptureTurn, RawTraceContributionEvent, RecordedTraceContributionOptions,
        };
        // Fixed seed: a generated key makes a failure unreproducible.
        let provider = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
        let signer = hex::encode(provider.public_key().as_ref());
        let now = chrono::Utc::now().timestamp();
        let binding = AdmissionBinding {
            account_anchor_sha256: "a".repeat(64),
            nonce_hex: "b".repeat(64),
            expires_at: now + 60,
        };
        let request_body = serde_json::json!({
            "model": FIXTURE_MODEL,
            "metadata": {REQUEST_METADATA_KEY: binding.encode().unwrap()},
            "messages": [{"role": "user", "content": "hello"}],
        })
        .to_string();
        let response_body = r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#;
        let request_hash = hash_hex(request_body.as_bytes());
        let response_hash = hash_hex(response_body.as_bytes());
        let text = match bound_model {
            Some(model) => format!("{model}:{request_hash}:{response_hash}"),
            None => format!("{request_hash}:{response_hash}"),
        };
        let receipt = ReceiptPayload {
            signature: hex::encode(provider.sign(text.as_bytes()).as_ref()),
            signing_address: signer.clone(),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: kind,
            text,
        };
        let started = chrono::Utc::now();
        let mut raw = RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: "asked the model something".to_string(),
                response: None,
                tool_calls: Vec::new(),
                started_at: started,
                completed_at: Some(started + chrono::Duration::milliseconds(10)),
                state: Some("Completed".to_string()),
            }],
            RecordedTraceContributionOptions::default(),
        );
        raw.events = vec![RawTraceContributionEvent {
            event_id: uuid::Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::HttpExchange,
            timestamp: started,
            content: Some(response_body.to_string()),
            structured_payload: serde_json::json!({
                "request": {"method": "POST", "body": request_body},
                "response": {"status": 200},
            }),
            tool_name: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
        }];
        (raw, receipt, signer, now)
    }

    /// A gateway receipt proves the bytes were served through the gateway. It
    /// does not prove *which model* served them: the two-part signed text
    /// names no model, and the gateway key is shared across every hosted
    /// model behind it. So on a gateway-admitted call the model is only what
    /// the request body asked for, and nothing downstream -- credit scoring,
    /// a per-model tier, a `served_model` field -- may read it as what
    /// answered.
    ///
    /// The only model claim this crate stamps is
    /// `restrict_contribution`'s `ironclaw.model_name`, so that is where the
    /// distinction is observable. Both arms are asserted: dropping the
    /// `signature_kind` read makes a gateway receipt stamp a served model,
    /// and stamping nothing at all would pass a one-sided test while losing
    /// the claim a provider-TEE receipt genuinely earns.
    #[test]
    fn a_gateway_receipt_never_evidences_which_model_served_the_call() {
        let max_body = 1024 * 1024;

        // Provider-TEE: the signature covers the model, so the claim stands.
        let (raw, receipt, signer, now) =
            admission_fixture(Some(FIXTURE_MODEL), ReceiptSignatureKind::ProviderTee);
        let trust =
            AdmissionProviderTrust::new([signer], Vec::new(), [FIXTURE_MODEL.to_string()], 1)
                .unwrap();
        let call = verify_admission_call(&raw, &receipt, &trust, now, max_body).unwrap();
        assert_eq!(call.receipt_bound_model(), Some(FIXTURE_MODEL));
        assert_eq!(call.requested_model(), FIXTURE_MODEL);
        let mut restricted = raw.clone();
        call.restrict_contribution(&mut restricted).unwrap();
        assert_eq!(
            restricted.ironclaw.model_name.as_deref(),
            Some(FIXTURE_MODEL),
            "a receipt whose signed text commits to the model does evidence it"
        );

        // Gateway, both signed forms. The three-part case matters: a model in
        // the text is not model evidence when the key that signed it vouches
        // for every model behind the gateway.
        for bound_model in [None, Some(FIXTURE_MODEL)] {
            let (raw, receipt, signer, now) =
                admission_fixture(bound_model, ReceiptSignatureKind::Gateway);
            let trust = AdmissionProviderTrust::new(
                ["f".repeat(64)],
                [signer],
                [FIXTURE_MODEL.to_string()],
                1,
            )
            .unwrap();
            // Admitted -- the refusal below is about the model claim, not
            // about the call.
            let call = verify_admission_call(&raw, &receipt, &trust, now, max_body).unwrap();
            assert_eq!(
                call.receipt_bound_model(),
                None,
                "the gateway key vouches for every model behind it, so a model \
                 in its signed text is still not evidence of which one answered"
            );
            // Unchanged: admission policy still knows what was asked for.
            assert_eq!(call.requested_model(), FIXTURE_MODEL);
            let mut restricted = raw.clone();
            call.restrict_contribution(&mut restricted).unwrap();
            assert_eq!(
                restricted.ironclaw.model_name, None,
                "a gateway receipt names no model that served the call"
            );
        }
    }
}
