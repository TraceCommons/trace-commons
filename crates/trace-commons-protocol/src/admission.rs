//! Account-bound admission evidence, separate from redaction certificates.
//! This wire module grants no trust; server and witness verify their own pins.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EVIDENCE_HEADER: &str = "x-trace-admission-evidence";
pub const SIGNATURE_HEADER: &str = "x-trace-admission-signature";
pub const REQUEST_METADATA_KEY: &str = "trace_commons_admission";
pub const EVIDENCE_DOMAIN: &str = "trace_commons_admission_evidence.v2";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionEvidence {
    pub profile: String,
    pub account_anchor_sha256: String,
    pub challenge_sha256: String,
    pub provider_signer: String,
    pub model: String,
    pub request_bytes: u64,
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
            || !is_hash(&self.provider_signer)
            || self.model.is_empty()
            || self.model.len() > 256
            || self.model.trim() != self.model
            || self.request_bytes == 0
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
            &self.model,
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
        bytes.extend_from_slice(&self.request_bytes.to_be_bytes());
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

/// Admission v2 accepts only a canonical Ed25519 key; ordinary ECDSA
/// receipts cannot acquire this identity by changing their wire discriminator.
pub fn receipt_identity(
    signer: &str,
    request_hash: &str,
    response_hash: &str,
) -> Result<String, EvidenceMalformed> {
    if !is_hash(signer) || !is_hash(request_hash) || !is_hash(response_hash) {
        return Err(EvidenceMalformed);
    }
    let key = hex::decode(signer).map_err(|_| EvidenceMalformed)?;
    let mut bytes = b"trace_commons_admission_receipt.v2.ed25519\0".to_vec();
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(&hex::decode(request_hash).map_err(|_| EvidenceMalformed)?);
    bytes.extend_from_slice(&hex::decode(response_hash).map_err(|_| EvidenceMalformed)?);
    Ok(hash_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_v2_signs_admission_policy_inputs_and_refuses_v1() {
        let evidence = AdmissionEvidence {
            profile: EVIDENCE_DOMAIN.into(),
            account_anchor_sha256: "a".repeat(64),
            challenge_sha256: "b".repeat(64),
            provider_signer: "c".repeat(64),
            model: "operator-approved-model".into(),
            request_bytes: 4096,
            request_sha256: "d".repeat(64),
            response_sha256: "e".repeat(64),
            receipt_sha256: "f".repeat(64),
            artifact_sha256: "0".repeat(64),
            witness_measurement: "measurement".into(),
            redaction_policy_version: "policy".into(),
            issued_at: 1,
            expires_at: 2,
        };
        let signed = evidence.signing_bytes().unwrap();
        let mut changed = evidence.clone();
        changed.model.push_str("-other");
        assert_ne!(signed, changed.signing_bytes().unwrap());
        changed = evidence.clone();
        changed.request_bytes += 1;
        assert_ne!(signed, changed.signing_bytes().unwrap());
        changed.request_bytes = 0;
        assert!(changed.signing_bytes().is_err());
        changed = evidence;
        changed.profile = "trace_commons_admission_evidence.v1".into();
        assert!(changed.signing_bytes().is_err());
    }
    #[test]
    fn binding_is_canonical_and_domain_separated() {
        let binding = AdmissionBinding {
            account_anchor_sha256: "a".repeat(64),
            nonce_hex: "b".repeat(64),
            expires_at: 12345,
        };
        let encoded = binding.encode().unwrap();
        assert_eq!(
            AdmissionBinding::parse(&encoded).unwrap().digest().unwrap(),
            binding.digest().unwrap()
        );
        for candidate in [
            encoded.to_uppercase(),
            encoded.replace(":12345", ":012345"),
            format!("{encoded} "),
            encoded.replace("tcad1:", "tcad2:"),
        ] {
            assert!(AdmissionBinding::parse(&candidate).is_err());
        }
        let mut another = binding.clone();
        another.account_anchor_sha256 = "c".repeat(64);
        assert_ne!(binding.digest().unwrap(), another.digest().unwrap());
    }
    #[test]
    fn receipt_identity_requires_ed25519_key_and_exact_body_digests() {
        let signer = "ab".repeat(32);
        let upper = "AB".repeat(32);
        let request = "1".repeat(64);
        let response = "2".repeat(64);
        assert!(receipt_identity(&upper, &request, &response).is_err());
        assert!(receipt_identity(&format!("0x{}", "ab".repeat(20)), &request, &response).is_err());
        assert_ne!(
            receipt_identity(&signer, &request, &response).unwrap(),
            receipt_identity(&signer, &response, &request).unwrap()
        );
        assert!(receipt_identity("0x1234", &request, &response).is_err());
    }
}
