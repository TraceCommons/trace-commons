// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Receipt-bound admission evidence. Redaction v1 alone never grants admission.
use crate::near_attestation::receipt::{ReceiptAlgo, ReceiptPayload, verify_receipt};
use crate::redaction_witness::verification::{VerifiedWitnessCertificate, WitnessPin};
use crate::witness_service::{Signer, WitnessContributionResponse};
use std::collections::BTreeSet;
use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, EVIDENCE_DOMAIN, REQUEST_METADATA_KEY, hash_hex, is_hash,
    receipt_identity,
};
use trace_commons_protocol::trace_contribution::{
    RawTraceContribution, TraceContributionEventType,
};

#[derive(Clone)]
pub struct AdmissionProviderTrust {
    signers: BTreeSet<String>,
    models: BTreeSet<String>,
    min_request_bytes: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("admission_evidence_refused")]
pub struct AdmissionEvidenceError;
impl AdmissionProviderTrust {
    pub fn new(
        keys: impl IntoIterator<Item = String>,
        models: impl IntoIterator<Item = String>,
        min_request_bytes: u64,
    ) -> Result<Self, AdmissionEvidenceError> {
        let signers: BTreeSet<_> = keys.into_iter().collect();
        let models: BTreeSet<_> = models.into_iter().collect();
        if signers.is_empty()
            || signers.iter().any(|s| !is_hash(s))
            || models.is_empty()
            || models
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.trim() != s)
            || min_request_bytes == 0
        {
            return Err(AdmissionEvidenceError);
        }
        Ok(Self {
            signers,
            models,
            min_request_bytes,
        })
    }
    pub fn from_env(prefix: &str) -> Result<Self, AdmissionEvidenceError> {
        let read = |suffix| {
            std::env::var(format!("{prefix}_{suffix}")).map_err(|_| AdmissionEvidenceError)
        };
        Self::new(
            read("PROVIDER_SIGNERS")?
                .split(',')
                .map(str::trim)
                .map(str::to_string),
            read("ACCEPTED_MODELS")?
                .split(',')
                .map(str::trim)
                .map(str::to_string),
            read("MIN_REQUEST_BYTES")?
                .parse()
                .map_err(|_| AdmissionEvidenceError)?,
        )
    }
    pub fn accepts(&self, signer: &str) -> bool {
        is_hash(signer) && self.signers.contains(signer)
    }
    pub fn accepts_request(&self, model: &str, request_bytes: u64) -> bool {
        self.models.contains(model) && request_bytes >= self.min_request_bytes
    }
}

/// Only constructed after receipt verification, exact body binding, and provider trust.
pub struct VerifiedAdmissionCall {
    binding: AdmissionBinding,
    provider_signer: String,
    request_hash: String,
    response_hash: String,
    model: String,
    request_bytes: u64,
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
    if receipt.signing_algo != ReceiptAlgo::Ed25519
        || !trust.accepts_request(model, request.len() as u64)
    {
        return Err(AdmissionEvidenceError);
    }
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
        model: model.to_string(),
        request_bytes: request.len() as u64,
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
        || !provider_trust.accepts_request(&evidence.model, evidence.request_bytes)
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
        let good = || ["operator-approved-model".to_string()];
        assert!(AdmissionProviderTrust::new([key.clone()], good(), 1).is_ok());
        assert!(AdmissionProviderTrust::new(Vec::new(), good(), 1).is_err());
        assert!(AdmissionProviderTrust::new([format!("0x{}", "a".repeat(40))], good(), 1).is_err());
        assert!(AdmissionProviderTrust::new([key.to_uppercase()], good(), 1).is_err());
        assert!(AdmissionProviderTrust::new([key.clone()], Vec::new(), 1).is_err());
        assert!(AdmissionProviderTrust::new([key.clone()], ["".into()], 1).is_err());
        assert!(AdmissionProviderTrust::new([key.clone()], [" padded ".into()], 1).is_err());
        assert!(AdmissionProviderTrust::new([key], good(), 0).is_err());
    }
}
