//! Closed provenance for a contribution's final declared inference call.
//!
//! An attested value binds one final declared call's request and response bytes
//! to a pinned receipt signer. It does not authenticate the surrounding session,
//! prove that no earlier calls were omitted, or prevent receipt replay.

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

const V2_DOMAIN: &[u8] = b"trace_commons.redaction_witness_certificate.v2\n";
const RECEIPT_IDENTITY_DOMAIN: &[u8] = b"trace_commons.final_call_receipt_identity.v1\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationClass {
    Unattested,
    ProviderTeeFinalCall,
    GatewayFinalCall,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct FinalCallAttestation {
    class: AttestationClass,
    receipt_sha256: String,
    model: Option<String>,
    receipt_signer: String,
    request_sha256: String,
    response_sha256: String,
}

impl std::fmt::Debug for FinalCallAttestation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FinalCallAttestation")
            .field("class", &self.class)
            .field("receipt_sha256", &self.receipt_sha256)
            .field("request_sha256", &self.request_sha256)
            .field("response_sha256", &self.response_sha256)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvenanceError {
    #[error("an attested final call requires a provider TEE or gateway class")]
    UnattestedClass,
    #[error("a digest must be exactly 64 lowercase hexadecimal characters")]
    InvalidDigest,
    #[error("a receipt signer must be a nonempty lowercase hexadecimal string")]
    InvalidSigner,
    #[error("the model bound by a receipt must not be empty")]
    EmptyModel,
}

impl FinalCallAttestation {
    /// Build a verified final-call claim. The caller must supply a model only
    /// when the verified receipt actually bound one; this type cannot inspect
    /// the receipt and never infers a model from a request or route.
    pub fn new(
        class: AttestationClass,
        receipt_sha256: String,
        model: Option<String>,
        receipt_signer: String,
        request_sha256: String,
        response_sha256: String,
    ) -> Result<Self, ProvenanceError> {
        if class == AttestationClass::Unattested {
            return Err(ProvenanceError::UnattestedClass);
        }
        if [&receipt_sha256, &request_sha256, &response_sha256]
            .iter()
            .any(|digest| !is_lower_hex(digest, Some(64)))
        {
            return Err(ProvenanceError::InvalidDigest);
        }
        // Ed25519 signer is 32 bytes; a recovered ECDSA address is 20 bytes.
        // Neither is inferred from the receipt's untrusted claimed address.
        let signer_hex = receipt_signer.strip_prefix("0x").unwrap_or(&receipt_signer);
        if ![40, 64].contains(&signer_hex.len()) || !is_lower_hex(signer_hex, None) {
            return Err(ProvenanceError::InvalidSigner);
        }
        if model.as_deref().is_some_and(str::is_empty) {
            return Err(ProvenanceError::EmptyModel);
        }
        Ok(Self {
            class,
            receipt_sha256,
            model,
            receipt_signer,
            request_sha256,
            response_sha256,
        })
    }

    pub fn class(&self) -> AttestationClass {
        self.class
    }
    pub fn receipt_sha256(&self) -> &str {
        &self.receipt_sha256
    }
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
    pub fn receipt_signer(&self) -> &str {
        &self.receipt_signer
    }
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }
    pub fn response_sha256(&self) -> &str {
        &self.response_sha256
    }
}

fn is_lower_hex(value: &str, width: Option<usize>) -> bool {
    width.is_none_or(|expected| value.len() == expected)
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFinalCallAttestation {
    class: AttestationClass,
    receipt_sha256: String,
    #[serde(deserialize_with = "deserialize_required_model")]
    model: Option<String>,
    receipt_signer: String,
    request_sha256: String,
    response_sha256: String,
}

fn deserialize_required_model<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

impl<'de> Deserialize<'de> for FinalCallAttestation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawFinalCallAttestation::deserialize(deserializer)?;
        Self::new(
            raw.class,
            raw.receipt_sha256,
            raw.model,
            raw.receipt_signer,
            raw.request_sha256,
            raw.response_sha256,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "final_call", rename_all = "snake_case")]
pub enum InferenceProvenance {
    Unattested,
    Attested(FinalCallAttestation),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvenance {
    status: String,
    #[serde(default, deserialize_with = "deserialize_present_final_call")]
    final_call: Option<FinalCallAttestation>,
}

fn deserialize_present_final_call<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<FinalCallAttestation>, D::Error> {
    FinalCallAttestation::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for InferenceProvenance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawProvenance::deserialize(deserializer)?;
        match (raw.status.as_str(), raw.final_call) {
            ("unattested", None) => Ok(Self::Unattested),
            ("attested", Some(call)) => Ok(Self::Attested(call)),
            _ => Err(serde::de::Error::custom(
                "unknown or malformed provenance variant",
            )),
        }
    }
}

/// Parse only the two known shapes, rejecting extra fields and malformed calls.
pub fn parse_inference_provenance_json(
    json: &str,
) -> Result<InferenceProvenance, serde_json::Error> {
    serde_json::from_str(json)
}

/// Render the closed JSON wire shape; signing always uses the binary encoder.
pub fn inference_provenance_json(value: &InferenceProvenance) -> String {
    serde_json::to_string(value).expect("closed provenance is JSON serializable")
}

/// The five existing certificate fields, in their v1 signing order.
#[derive(Debug, Clone, Copy)]
pub struct WitnessCertificateV2Base<'a> {
    pub redacted_sha256: &'a str,
    pub redaction_policy_version: &'a str,
    pub witness_measurement: &'a str,
    pub residual_risk_tag: u8,
    pub timestamp: i64,
}

fn append_length_prefixed(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_le_bytes());
    out.extend_from_slice(field);
}

/// Canonical v2 certificate preimage, shared by issuer and verifiers.
/// Tags: 0 unattested, 1 attested; within attested 1 provider TEE, 2 gateway;
/// model tag 0 absent, 1 present. Every variable field carries a u64 LE length.
pub fn witness_certificate_v2_signing_bytes(
    base: WitnessCertificateV2Base<'_>,
    provenance: &InferenceProvenance,
) -> Vec<u8> {
    let mut out = V2_DOMAIN.to_vec();
    for field in [
        base.redacted_sha256,
        base.redaction_policy_version,
        base.witness_measurement,
    ] {
        append_length_prefixed(&mut out, field.as_bytes());
    }
    out.push(base.residual_risk_tag);
    out.extend_from_slice(&base.timestamp.to_le_bytes());
    match provenance {
        InferenceProvenance::Unattested => out.push(0),
        InferenceProvenance::Attested(call) => {
            out.push(1);
            out.push(match call.class {
                AttestationClass::ProviderTeeFinalCall => 1,
                AttestationClass::GatewayFinalCall => 2,
                AttestationClass::Unattested => {
                    unreachable!("constructor rejects unattested class")
                }
            });
            append_length_prefixed(&mut out, call.receipt_sha256.as_bytes());
            match &call.model {
                None => out.push(0),
                Some(model) => {
                    out.push(1);
                    append_length_prefixed(&mut out, model.as_bytes());
                }
            }
            append_length_prefixed(&mut out, call.receipt_signer.as_bytes());
            append_length_prefixed(&mut out, call.request_sha256.as_bytes());
            append_length_prefixed(&mut out, call.response_sha256.as_bytes());
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptSigningAlgorithm {
    Ecdsa,
    Ed25519,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptSignatureKind {
    ProviderTee,
    Gateway,
}

/// Audit identity of the exact verified receipt inputs. The signer must be the
/// verified signer bytes, not an address claim parsed from the wire. This hash
/// correlates receipts; it does not establish uniqueness or prevent replay.
pub fn receipt_identity_sha256(
    text: &[u8],
    signature: &[u8],
    verified_signer: &[u8],
    algorithm: ReceiptSigningAlgorithm,
    kind: ReceiptSignatureKind,
) -> String {
    let mut bytes = RECEIPT_IDENTITY_DOMAIN.to_vec();
    for field in [text, signature, verified_signer] {
        append_length_prefixed(&mut bytes, field);
    }
    bytes.push(match algorithm {
        ReceiptSigningAlgorithm::Ecdsa => 1,
        ReceiptSigningAlgorithm::Ed25519 => 2,
    });
    bytes.push(match kind {
        ReceiptSignatureKind::ProviderTee => 1,
        ReceiptSignatureKind::Gateway => 2,
    });
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> WitnessCertificateV2Base<'static> {
        WitnessCertificateV2Base {
            redacted_sha256: "a",
            redaction_policy_version: "bc",
            witness_measurement: "d",
            residual_risk_tag: 2,
            timestamp: 1,
        }
    }

    fn call(class: AttestationClass, model: Option<&str>) -> FinalCallAttestation {
        FinalCallAttestation::new(
            class,
            "a".repeat(64),
            model.map(str::to_string),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
        )
        .unwrap()
    }

    #[test]
    fn witness_provenance_v2_frozen_vector() {
        let actual = witness_certificate_v2_signing_bytes(base(), &InferenceProvenance::Unattested);
        let expected = concat!(
            "74726163655f636f6d6d6f6e732e726564616374696f6e5f7769746e6573735f63657274696669636174652e76320a",
            "0100000000000000610200000000000000626301000000000000006402010000000000000000"
        );
        assert_eq!(hex::encode(actual), expected);
    }

    #[test]
    fn witness_provenance_attested_variants_and_mutations_are_distinct() {
        let provider = InferenceProvenance::Attested(call(
            AttestationClass::ProviderTeeFinalCall,
            Some("model"),
        ));
        let gateway =
            InferenceProvenance::Attested(call(AttestationClass::GatewayFinalCall, Some("model")));
        let absent_model =
            InferenceProvenance::Attested(call(AttestationClass::ProviderTeeFinalCall, None));
        let original = witness_certificate_v2_signing_bytes(base(), &provider);
        let frozen = concat!(
            "74726163655f636f6d6d6f6e732e726564616374696f6e5f7769746e6573735f63657274696669636174652e76320a01000000000000006102000000",
            "000000006263010000000000000064020100000000000000010140000000000000006161616161616161616161616161616161616161616161616161",
            "61616161616161616161616161616161616161616161616161616161616161616161616161610105000000000000006d6f64656c4000000000000000",
            "626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262",
            "626262624000000000000000636363636363636363636363636363636363636363636363636363636363636363636363636363636363636363636363",
            "636363636363636363636363636363634000000000000000646464646464646464646464646464646464646464646464646464646464646464646464",
            "64646464646464646464646464646464646464646464646464646464",
        );
        assert_eq!(hex::encode(&original), frozen);
        assert_ne!(
            original,
            witness_certificate_v2_signing_bytes(base(), &gateway)
        );
        assert_ne!(
            original,
            witness_certificate_v2_signing_bytes(base(), &absent_model)
        );
        assert_ne!(
            original,
            witness_certificate_v2_signing_bytes(base(), &InferenceProvenance::Unattested)
        );
        for field in 0..5 {
            let mut changed = call(AttestationClass::ProviderTeeFinalCall, Some("model"));
            match field {
                0 => changed.receipt_sha256 = "e".repeat(64),
                1 => changed.model = Some("other".to_string()),
                2 => changed.receipt_signer = "e".repeat(64),
                3 => changed.request_sha256 = "e".repeat(64),
                _ => changed.response_sha256 = "e".repeat(64),
            }
            assert_ne!(
                original,
                witness_certificate_v2_signing_bytes(
                    base(),
                    &InferenceProvenance::Attested(changed)
                )
            );
        }
    }

    #[test]
    fn witness_provenance_adjacent_boundary_shifts_do_not_collide() {
        let first = WitnessCertificateV2Base {
            redacted_sha256: "ab",
            redaction_policy_version: "c",
            ..base()
        };
        let second = WitnessCertificateV2Base {
            redacted_sha256: "a",
            redaction_policy_version: "bc",
            ..base()
        };
        assert_ne!(
            witness_certificate_v2_signing_bytes(first, &InferenceProvenance::Unattested),
            witness_certificate_v2_signing_bytes(second, &InferenceProvenance::Unattested)
        );
        let first = WitnessCertificateV2Base {
            redaction_policy_version: "ab",
            witness_measurement: "c",
            ..base()
        };
        let second = WitnessCertificateV2Base {
            redaction_policy_version: "a",
            witness_measurement: "bc",
            ..base()
        };
        assert_ne!(
            witness_certificate_v2_signing_bytes(first, &InferenceProvenance::Unattested),
            witness_certificate_v2_signing_bytes(second, &InferenceProvenance::Unattested)
        );
    }

    #[test]
    fn witness_provenance_closed_wire_rejects_unknown_and_malformed() {
        for json in [
            r#"{"status":"bogus"}"#,
            r#"{"status":"unattested","extra":1}"#,
            r#"{"status":"attested"}"#,
            r#"{"status":"unattested","final_call":{}}"#,
            r#"{"status":"unattested","final_call":null}"#,
            r#"{"status":"attested","final_call":{"class":"bogus"}}"#,
            r#"{"status":"attested","final_call":{"class":"gateway_final_call","receipt_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","receipt_signer":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","response_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}}"#,
            r#"{"status":"attested","final_call":{"class":"unattested","receipt_sha256":"a","model":null,"receipt_signer":"b","request_sha256":"c","response_sha256":"d"}}"#,
        ] {
            assert!(parse_inference_provenance_json(json).is_err(), "{json}");
        }
        for value in [
            InferenceProvenance::Unattested,
            InferenceProvenance::Attested(call(AttestationClass::ProviderTeeFinalCall, None)),
            InferenceProvenance::Attested(call(AttestationClass::GatewayFinalCall, Some("model"))),
        ] {
            assert_eq!(
                parse_inference_provenance_json(&inference_provenance_json(&value)).unwrap(),
                value
            );
        }
        assert_eq!(
            FinalCallAttestation::new(
                AttestationClass::Unattested,
                "a".repeat(64),
                None,
                "b".repeat(64),
                "c".repeat(64),
                "d".repeat(64)
            ),
            Err(ProvenanceError::UnattestedClass)
        );
        assert_eq!(
            FinalCallAttestation::new(
                AttestationClass::GatewayFinalCall,
                "A".repeat(64),
                None,
                "b".repeat(64),
                "c".repeat(64),
                "d".repeat(64),
            ),
            Err(ProvenanceError::InvalidDigest)
        );
        assert_eq!(
            FinalCallAttestation::new(
                AttestationClass::GatewayFinalCall,
                "a".repeat(64),
                None,
                "B".repeat(64),
                "c".repeat(64),
                "d".repeat(64),
            ),
            Err(ProvenanceError::InvalidSigner)
        );
        let debug = format!(
            "{:?}",
            call(AttestationClass::ProviderTeeFinalCall, Some("secret-model"))
        );
        assert!(!debug.contains("secret-model"));
        assert!(!debug.contains(&"b".repeat(64)));
    }

    #[test]
    fn witness_provenance_receipt_identity_is_frozen_and_binary_sensitive() {
        let digest = receipt_identity_sha256(
            b"text",
            b"\x00\xff",
            b"signer",
            ReceiptSigningAlgorithm::Ed25519,
            ReceiptSignatureKind::ProviderTee,
        );
        assert_eq!(
            digest,
            "e3234c67fbcc2d5aacd91c82f294828d5b305aca4a5caa1ddc234c33ad38cf3c"
        );
        assert_ne!(
            digest,
            receipt_identity_sha256(
                b"tex",
                b"t\x00\xff",
                b"signer",
                ReceiptSigningAlgorithm::Ed25519,
                ReceiptSignatureKind::ProviderTee
            )
        );
        assert_ne!(
            digest,
            receipt_identity_sha256(
                b"text",
                b"\x00\xff",
                b"signer",
                ReceiptSigningAlgorithm::Ed25519,
                ReceiptSignatureKind::Gateway
            )
        );
    }
}
