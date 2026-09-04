// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Receipt-bound admission evidence. Redaction v1 alone never grants admission.
use crate::near_attestation::receipt::{ReceiptPayload, decode_address, verify_receipt};
use crate::redaction_witness::verification::{VerifiedWitnessCertificate, WitnessPin};
use crate::witness_service::{Signer, WitnessContributionResponse};
use std::collections::BTreeSet;
use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, EVIDENCE_DOMAIN, REQUEST_METADATA_KEY, hash_hex,
    receipt_identity,
};
use trace_commons_protocol::trace_contribution::{
    RawTraceContribution, TraceContributionEventType,
};

#[derive(Clone)]
pub struct AdmissionProviderTrust {
    signers: BTreeSet<[u8; 20]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("admission_evidence_refused")]
pub struct AdmissionEvidenceError;
impl AdmissionProviderTrust {
    pub fn new(
        addresses: impl IntoIterator<Item = String>,
    ) -> Result<Self, AdmissionEvidenceError> {
        let signers = addresses
            .into_iter()
            .map(|s| decode_address(&s).ok_or(AdmissionEvidenceError))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if signers.is_empty() {
            return Err(AdmissionEvidenceError);
        }
        Ok(Self { signers })
    }
    pub fn accepts(&self, signer: &str) -> bool {
        decode_address(signer).is_some_and(|address| self.signers.contains(&address))
    }
}

/// Only constructed after receipt verification, exact body binding, and provider trust.
pub struct VerifiedAdmissionCall {
    binding: AdmissionBinding,
    provider_signer: String,
    request_hash: String,
    response_hash: String,
}

pub fn verify_admission_call(
    raw: &RawTraceContribution,
    receipt: &ReceiptPayload,
    trust: &AdmissionProviderTrust,
    now: i64,
    max_body_bytes: usize,
) -> Result<VerifiedAdmissionCall, AdmissionEvidenceError> {
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
    let verified = verify_receipt(receipt, request.as_bytes(), response.as_bytes(), model)
        .map_err(|_| AdmissionEvidenceError)?;
    if !trust.accepts(&verified.signing_address) {
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
    Ok(VerifiedAdmissionCall {
        binding,
        provider_signer: verified.signing_address,
        request_hash: verified.request_sha256,
        response_hash: verified.response_sha256,
    })
}

impl VerifiedAdmissionCall {
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
