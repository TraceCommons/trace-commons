//! Account-bound admission evidence, separate from redaction certificates.
//! This wire module grants no trust; server and witness verify their own pins.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EVIDENCE_HEADER: &str = "x-trace-admission-evidence";
pub const SIGNATURE_HEADER: &str = "x-trace-admission-signature";
pub const REQUEST_METADATA_KEY: &str = "trace_commons_admission";
pub const EVIDENCE_DOMAIN: &str = "trace_commons_admission_evidence.v1";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionEvidence {
    pub profile: String,
    pub account_anchor_sha256: String,
    pub challenge_sha256: String,
    pub provider_signer: String,
    pub request_sha256: String,
    pub response_sha256: String,
    pub receipt_sha256: String,
    pub artifact_sha256: String,
    pub witness_measurement: String,
    pub redaction_policy_version: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("admission_evidence_malformed")]
pub struct EvidenceMalformed;

pub fn hash_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
pub fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl AdmissionEvidence {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, EvidenceMalformed> {
        if self.profile != EVIDENCE_DOMAIN
            || self.issued_at < 0
            || self.expires_at <= self.issued_at
            || ![
                &self.account_anchor_sha256,
                &self.challenge_sha256,
                &self.request_sha256,
                &self.response_sha256,
                &self.receipt_sha256,
                &self.artifact_sha256,
            ]
            .into_iter()
            .all(|s| is_hash(s))
            || self.provider_signer.len() != 42
            || !self.provider_signer.starts_with("0x")
            || !self.provider_signer[2..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.witness_measurement.is_empty()
            || self.witness_measurement.len() > 512
            || self.redaction_policy_version.is_empty()
            || self.redaction_policy_version.len() > 256
        {
            return Err(EvidenceMalformed);
        }
        let mut bytes = Vec::new();
        for part in [
            &self.profile,
            &self.account_anchor_sha256,
            &self.challenge_sha256,
            &self.provider_signer,
            &self.request_sha256,
            &self.response_sha256,
            &self.receipt_sha256,
            &self.artifact_sha256,
            &self.witness_measurement,
            &self.redaction_policy_version,
        ] {
            bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
            bytes.extend_from_slice(part.as_bytes());
        }
        bytes.extend_from_slice(&self.issued_at.to_be_bytes());
        bytes.extend_from_slice(&self.expires_at.to_be_bytes());
        Ok(bytes)
    }
}

/// Canonical metadata string inserted after all upstream request transforms.
/// Only its hash is retained by the admission ledger.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmissionBinding {
    pub account_anchor_sha256: String,
    pub nonce_hex: String,
    pub expires_at: i64,
}
impl AdmissionBinding {
    pub fn encode(&self) -> Result<String, EvidenceMalformed> {
        if !is_hash(&self.account_anchor_sha256)
            || !is_hash(&self.nonce_hex)
            || self.expires_at <= 0
        {
            return Err(EvidenceMalformed);
        }
        Ok(format!(
            "tcad1:{}:{}:{}",
            self.account_anchor_sha256, self.nonce_hex, self.expires_at
        ))
    }
    pub fn parse(value: &str) -> Result<Self, EvidenceMalformed> {
        if value.len() > 180 {
            return Err(EvidenceMalformed);
        }
        let parts: Vec<_> = value.split(':').collect();
        if parts.len() != 4 || parts[0] != "tcad1" {
            return Err(EvidenceMalformed);
        }
        let binding = Self {
            account_anchor_sha256: parts[1].into(),
            nonce_hex: parts[2].into(),
            expires_at: parts[3].parse().map_err(|_| EvidenceMalformed)?,
        };
        if binding.encode()? != value {
            return Err(EvidenceMalformed);
        }
        Ok(binding)
    }
    pub fn digest(&self) -> Result<String, EvidenceMalformed> {
        Ok(hash_hex(self.encode()?.as_bytes()))
    }
}

/// Canonical identity ignores signature spelling and recovered-id representation.
pub fn receipt_identity(
    signer: &str,
    request_hash: &str,
    response_hash: &str,
) -> Result<String, EvidenceMalformed> {
    if signer.len() != 42
        || !signer.starts_with("0x")
        || !is_hash(request_hash)
        || !is_hash(response_hash)
    {
        return Err(EvidenceMalformed);
    }
    let address = hex::decode(&signer[2..]).map_err(|_| EvidenceMalformed)?;
    let mut bytes = b"trace_commons_admission_receipt.v1\0".to_vec();
    bytes.extend_from_slice(&address);
    bytes.extend_from_slice(&hex::decode(request_hash).map_err(|_| EvidenceMalformed)?);
    bytes.extend_from_slice(&hex::decode(response_hash).map_err(|_| EvidenceMalformed)?);
    Ok(hash_hex(&bytes))
}
