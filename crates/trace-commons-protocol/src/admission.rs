//! Account-bound admission evidence, separate from redaction certificates.
//! This wire module grants no trust; server and witness verify their own pins.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EVIDENCE_HEADER: &str = "x-trace-admission-evidence";
pub const SIGNATURE_HEADER: &str = "x-trace-admission-signature";
pub const REQUEST_METADATA_KEY: &str = "trace_commons_admission";
pub const EVIDENCE_DOMAIN: &str = "trace_commons_admission_evidence.v2";

/// The refusals the hosted admission gates emit, as a closed set.
///
/// A refusal reaches a client as an HTTP status and a `{"error": "<label>"}`
/// body, and a client that reads only the status cannot tell one apart from
/// an outage: a `403` is an expired credential *or* a declined contribution,
/// and every non-2xx from the witness is a malformed response *or* a receipt
/// its trust configuration turned away. Both were reported as the wrong
/// thing, so the vocabulary lives here, where the gate that sends it and the
/// client that reads it share one spelling.
///
/// **The label is attacker-influenceable input.** Nothing here returns the
/// caller's string: [`AdmissionRefusal::from_response`] matches it against
/// this set and hands back a variant, and [`AdmissionRefusal::label`] returns
/// this crate's own constant. A body that says something else is simply not
/// recognised, which leaves the caller's existing behaviour in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdmissionRefusal {
    /// The ingest gate declined this submission: no anchor for the
    /// authenticated account, or evidence it would not accept.
    Refused,
    /// The account's admission budget for this window is spent.
    LimitReached,
    /// Another attempt at the same submission holds the lease.
    InProgress,
    /// This submission id is already bound to different bytes or a different
    /// account.
    IdentityConflict,
    /// The witness declined the receipt behind an evidence-bearing request:
    /// its signer, its model, or the size of the request it covers.
    EvidenceRefused,
}

impl AdmissionRefusal {
    /// Every refusal, for tests and exhaustive mappings.
    pub const ALL: [AdmissionRefusal; 5] = [
        AdmissionRefusal::Refused,
        AdmissionRefusal::LimitReached,
        AdmissionRefusal::InProgress,
        AdmissionRefusal::IdentityConflict,
        AdmissionRefusal::EvidenceRefused,
    ];

    /// The wire label, which is what a gate puts in the `error` field.
    pub const fn label(self) -> &'static str {
        match self {
            AdmissionRefusal::Refused => "admission_refused",
            AdmissionRefusal::LimitReached => "admission_limit_reached",
            AdmissionRefusal::InProgress => "admission_in_progress",
            AdmissionRefusal::IdentityConflict => "admission_identity_conflict",
            AdmissionRefusal::EvidenceRefused => "admission_evidence_refused",
        }
    }

    /// The status this refusal is sent with.
    pub const fn status(self) -> u16 {
        match self {
            AdmissionRefusal::Refused | AdmissionRefusal::EvidenceRefused => 403,
            AdmissionRefusal::LimitReached => 429,
            AdmissionRefusal::InProgress | AdmissionRefusal::IdentityConflict => 409,
        }
    }

    /// Recognise a refusal from a label alone.
    ///
    /// For a caller holding a label it has already recorded -- a queue row, a
    /// refusal that has been through a `String` -- where the status is long
    /// gone. Prefer [`AdmissionRefusal::from_response`] at the point a
    /// response is read.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.label() == label)
    }

    /// Recognise a refusal from a response, requiring the status and the
    /// label to agree.
    ///
    /// Fail-closed on purpose. A label on a status it is never sent with is
    /// not this refusal, and treating it as one would let a body decide how a
    /// status is handled -- which on the ingest path means a `403` body could
    /// talk a client out of reminting a genuinely expired claim.
    pub fn from_response(status: u16, label: &str) -> Option<Self> {
        Self::from_label(label).filter(|r| r.status() == status)
    }
}

/// Which signature the witness checked before certifying this call.
///
/// Recorded in the signed statement because it is the difference between an
/// attested model and a body-asserted one, and a holder of the evidence
/// cannot otherwise tell: re-deriving it from signer-set membership needs the
/// operator's configuration, which the holder does not have.
///
/// Only two values, deliberately. A receipt whose kind names no key source is
/// refused before any evidence is made, so an "unrecognised" arm here would
/// be a state this type can represent and the system cannot reach.
///
/// The wire spellings are NEAR AI's own, and
/// `trace_commons_server::admission_evidence` pins them against
/// `ReceiptSignatureKind::as_wire` so the two vocabularies cannot drift. This
/// crate does not depend on the attestation crate for one enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdmissionSignatureKind {
    /// The gateway's own attested key. Shared across every model behind it,
    /// so it binds the bytes and names no model.
    #[serde(rename = "gateway")]
    Gateway,
    /// The serving model's per-model attested key. The model is inside the
    /// signed text.
    #[serde(rename = "provider_tee")]
    ProviderTee,
}

impl AdmissionSignatureKind {
    /// The wire spelling, and what `signing_bytes` commits to.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::ProviderTee => "provider_tee",
        }
    }

    /// Whether a receipt of this kind evidences which model answered.
    ///
    /// The one question every consumer of this field actually asks. A gateway
    /// key vouches for every model behind it, so it answers for none of them.
    #[must_use]
    pub const fn binds_model(self) -> bool {
        matches!(self, Self::ProviderTee)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionEvidence {
    pub profile: String,
    pub account_anchor_sha256: String,
    pub challenge_sha256: String,
    pub provider_signer: String,
    /// Which signature the witness checked. See [`AdmissionSignatureKind`];
    /// [`AdmissionSignatureKind::binds_model`] is what [`Self::model`] must be
    /// read through.
    pub signature_kind: AdmissionSignatureKind,
    /// The model the admitted request **asked for**, which is what the
    /// operator's accepted-model list is checked against.
    ///
    /// Not evidence of which model answered. A gateway-signed receipt binds
    /// the request and response bytes and names no model at all -- the
    /// gateway key is shared across every model behind it -- so on such a
    /// call this string is the caller's own request body and nothing more.
    /// A consumer keying credit, scoring, pricing or eligibility off a served
    /// model must not read it from here; the server-side
    /// `VerifiedAdmissionCall::receipt_bound_model` is the accessor that
    /// distinguishes the two.
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
        // Fixed order, length-prefixed, and the kind sits beside the signer
        // it qualifies. Both the witness that signs and the ingest that
        // re-verifies walk this same list, so the only requirement is that it
        // is deterministic -- which an array literal is.
        for part in [
            self.profile.as_str(),
            self.account_anchor_sha256.as_str(),
            self.challenge_sha256.as_str(),
            self.provider_signer.as_str(),
            self.signature_kind.as_wire(),
            self.model.as_str(),
            self.request_sha256.as_str(),
            self.response_sha256.as_str(),
            self.receipt_sha256.as_str(),
            self.artifact_sha256.as_str(),
            self.witness_measurement.as_str(),
            self.redaction_policy_version.as_str(),
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
            signature_kind: AdmissionSignatureKind::ProviderTee,
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

    /// The kind is inside the bytes the witness signs.
    ///
    /// If it were outside them, a holder could rewrite `gateway` to
    /// `provider_tee` on a signed statement and the signature would still
    /// verify -- turning a body-asserted model into an attested one by
    /// editing a field. That is the whole reason it is in the signed
    /// statement rather than beside it.
    #[test]
    fn the_signature_kind_is_covered_by_the_signature() {
        let base = AdmissionEvidence {
            profile: EVIDENCE_DOMAIN.into(),
            account_anchor_sha256: "a".repeat(64),
            challenge_sha256: "b".repeat(64),
            provider_signer: "c".repeat(64),
            signature_kind: AdmissionSignatureKind::ProviderTee,
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
        let mut gateway = base.clone();
        gateway.signature_kind = AdmissionSignatureKind::Gateway;
        assert_ne!(
            base.signing_bytes().unwrap(),
            gateway.signing_bytes().unwrap(),
            "the kind is not covered, so it can be rewritten under a valid signature"
        );
        // Deterministic across calls: the witness signs these bytes and ingest
        // recomputes them, so an ordering that varied would fail at random.
        assert_eq!(base.signing_bytes().unwrap(), base.signing_bytes().unwrap());
        assert_eq!(
            gateway.signing_bytes().unwrap(),
            gateway.signing_bytes().unwrap()
        );
        // The spelling is what is committed to, not a discriminant that could
        // change with the enum's declaration order.
        let bytes = gateway.signing_bytes().unwrap();
        let needle = AdmissionSignatureKind::Gateway.as_wire().as_bytes();
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "the wire spelling is not what the signature covers"
        );
    }

    /// The labels are a wire contract with deployed clients, so they are
    /// pinned literally here rather than derived from the enum. A rename that
    /// only moves the constant changes what an older shell sees.
    #[test]
    fn every_refusal_keeps_its_wire_spelling_and_status() {
        assert_eq!(
            AdmissionRefusal::ALL.map(|r| (r.label(), r.status())),
            [
                ("admission_refused", 403),
                ("admission_limit_reached", 429),
                ("admission_in_progress", 409),
                ("admission_identity_conflict", 409),
                ("admission_evidence_refused", 403),
            ]
        );
    }

    /// A label is body content, and body content does not get to pick a code
    /// path. Unrecognised labels and labels on the wrong status both answer
    /// `None`, which leaves the caller doing whatever it did before.
    #[test]
    fn a_refusal_is_recognised_only_when_status_and_label_agree() {
        for refusal in AdmissionRefusal::ALL {
            assert_eq!(
                AdmissionRefusal::from_response(refusal.status(), refusal.label()),
                Some(refusal)
            );
            for other in AdmissionRefusal::ALL {
                if other.status() != refusal.status() {
                    assert_eq!(
                        AdmissionRefusal::from_response(other.status(), refusal.label()),
                        None,
                        "{} was recognised on a {} it is never sent with",
                        refusal.label(),
                        other.status()
                    );
                }
            }
        }
        for label in [
            "",
            "admission",
            "admission_refused ",
            "Admission_Refused",
            "not_a_label",
            "{\"error\":\"admission_refused\"}",
        ] {
            assert_eq!(AdmissionRefusal::from_label(label), None, "{label}");
            assert_eq!(AdmissionRefusal::from_response(403, label), None, "{label}");
        }
    }
}
