//! Agent-driven pilot onboarding wire contract.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TRACE_ONBOARD_REQUEST_SCHEMA_VERSION: &str = "trace_commons.onboard_request.v1";
pub const TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION: &str = "trace_commons.onboard_response.v1";
pub const TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION: &str =
    "trace_commons.instance_enroll_request.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceOnboardClientInfo {
    pub agent: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceOnboardRequest {
    pub schema_version: String,
    pub invite_code: String,
    pub device_public_key: String,
    pub client_info: TraceOnboardClientInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceOnboardResponse {
    pub schema_version: String,
    pub tenant_id: String,
    pub ingest_url: String,
    pub issuer_url: String,
    pub audience: String,
    pub device_key_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributor_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaderboard_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceInstanceEnrollAttestation {
    pub device_key_id: String,
    pub aud: String,
    pub instance_id: String,
    pub user_subject: String,
    pub nonce: String,
    pub exp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceInstanceEnrollRequest {
    pub schema_version: String,
    pub instance_public_key: String,
    pub device_public_key: String,
    pub attestation: TraceInstanceEnrollAttestation,
    pub attestation_sig: String,
    pub client_info: TraceOnboardClientInfo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TraceOnboardErrorCode {
    InviteNotValid,
    InviteAlreadyConsumed,
    InviteMalformed,
    DeviceKeyMalformed,
    OnboardRateLimited,
    OnboardAllowlistNotConfigured,
    OnboardRegistryNotConfigured,
    OnboardTenantConfigMissing,
    OnboardAllowlistStale,
    EnrollMalformed,
    EnrollNotAuthorized,
    EnrollRateLimited,
    EnrollCapExceeded,
    InviteRegistryNotConfigured,
    InviteRegistryStale,
}

impl TraceOnboardErrorCode {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::InviteNotValid => "InviteNotValid",
            Self::InviteAlreadyConsumed => "InviteAlreadyConsumed",
            Self::InviteMalformed => "InviteMalformed",
            Self::DeviceKeyMalformed => "DeviceKeyMalformed",
            Self::OnboardRateLimited => "OnboardRateLimited",
            Self::OnboardAllowlistNotConfigured => "OnboardAllowlistNotConfigured",
            Self::OnboardRegistryNotConfigured => "OnboardRegistryNotConfigured",
            Self::OnboardTenantConfigMissing => "OnboardTenantConfigMissing",
            Self::OnboardAllowlistStale => "OnboardAllowlistStale",
            Self::EnrollMalformed => "EnrollMalformed",
            Self::EnrollNotAuthorized => "EnrollNotAuthorized",
            Self::EnrollRateLimited => "EnrollRateLimited",
            Self::EnrollCapExceeded => "EnrollCapExceeded",
            Self::InviteRegistryNotConfigured => "InviteRegistryNotConfigured",
            Self::InviteRegistryStale => "InviteRegistryStale",
        }
    }
}

pub fn device_key_id_from_public_key_bytes(public_key_bytes: &[u8]) -> String {
    let digest = Sha256::digest(public_key_bytes);
    format!("sha256:{}", hex::encode(digest))
}

/// Canonical, unambiguous signing bytes for an enrollment attestation. Each
/// field is length-prefixed (u64-le) so no field-boundary shift can collide.
/// This is the single source of truth shared by the Ironclaw signer and the
/// issuer verifier — keep it the only encoder.
pub fn instance_enroll_attestation_signing_bytes(a: &TraceInstanceEnrollAttestation) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"trace_commons.instance_enroll.v1\n");
    for field in [
        a.device_key_id.as_str(),
        a.aud.as_str(),
        a.instance_id.as_str(),
        a.user_subject.as_str(),
        a.nonce.as_str(),
    ] {
        out.extend_from_slice(&(field.len() as u64).to_le_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out.extend_from_slice(&a.exp.to_le_bytes());
    out
}

/// Derive the per-user tenant id. `0x1F` (unit separator) between the two
/// fields makes the concatenation injective. The result is a hash, so it is
/// non-identifying. One function, no drift — shared by signer and server.
pub fn derive_user_tenant_id(instance_id: &str, user_subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(instance_id.as_bytes());
    hasher.update([0x1F]);
    hasher.update(user_subject.as_bytes());
    format!("tenant-{}", hex::encode(hasher.finalize()))
}

/// Hash-only form of the per-user subject for the enrollment ledger.
pub fn user_subject_hash(user_subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"user_subject:");
    hasher.update(user_subject.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboard_request_round_trips() {
        let request = TraceOnboardRequest {
            schema_version: TRACE_ONBOARD_REQUEST_SCHEMA_VERSION.to_string(),
            invite_code: "INV9K3RT5FBQ72JX".to_string(),
            device_public_key: "cHVibGljLWtleS1ieXRlcw==".to_string(),
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        };
        let encoded = serde_json::to_string(&request).expect("serialize request");
        let decoded: TraceOnboardRequest =
            serde_json::from_str(&encoded).expect("deserialize request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn onboard_response_round_trips_with_optional_community_urls() {
        let response = TraceOnboardResponse {
            schema_version: TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION.to_string(),
            tenant_id: "tenant-zaki-pilot".to_string(),
            ingest_url: "https://ingest.tracecommons.ai".to_string(),
            issuer_url: "https://issuer.tracecommons.ai".to_string(),
            audience: "trace-commons-ingest".to_string(),
            device_key_id:
                "sha256:ad745f4e0af66a2c7ba9e95cf8ea65addb47d86ed989854c6f84f62fc177bd83"
                    .to_string(),
            contributor_label: Some("closed-alpha-batch-1".to_string()),
            community_url: Some("https://tracecommons.ai".to_string()),
            profile_url: Some("https://tracecommons.ai/profile".to_string()),
            leaderboard_url: Some("https://tracecommons.ai/leaderboard".to_string()),
        };
        let encoded = serde_json::to_string(&response).expect("serialize response");
        assert!(encoded.contains("profile_url"));
        let decoded: TraceOnboardResponse =
            serde_json::from_str(&encoded).expect("deserialize response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn onboard_error_codes_use_exact_wire_names() {
        assert_eq!(
            serde_json::to_string(&TraceOnboardErrorCode::InviteNotValid).unwrap(),
            "\"InviteNotValid\""
        );
        assert_eq!(
            serde_json::to_string(&TraceOnboardErrorCode::InviteAlreadyConsumed).unwrap(),
            "\"InviteAlreadyConsumed\""
        );
        assert_eq!(
            serde_json::from_str::<TraceOnboardErrorCode>("\"OnboardRateLimited\"").unwrap(),
            TraceOnboardErrorCode::OnboardRateLimited
        );
        assert_eq!(
            serde_json::to_string(&TraceOnboardErrorCode::OnboardAllowlistNotConfigured).unwrap(),
            "\"OnboardAllowlistNotConfigured\""
        );
        assert_eq!(
            TraceOnboardErrorCode::OnboardRegistryNotConfigured.as_wire_str(),
            "OnboardRegistryNotConfigured"
        );
        assert_eq!(
            TraceOnboardErrorCode::OnboardTenantConfigMissing.as_wire_str(),
            "OnboardTenantConfigMissing"
        );
        assert_eq!(
            TraceOnboardErrorCode::OnboardAllowlistStale.as_wire_str(),
            "OnboardAllowlistStale"
        );
    }

    #[test]
    fn device_key_id_hashes_raw_public_key_bytes() {
        let id = device_key_id_from_public_key_bytes(b"public-key-bytes");
        assert_eq!(
            id,
            "sha256:ad745f4e0af66a2c7ba9e95cf8ea65addb47d86ed989854c6f84f62fc177bd83"
        );
    }

    #[test]
    fn derive_user_tenant_id_is_stable_and_separator_safe() {
        let a = derive_user_tenant_id("inst", "user-1");
        assert_eq!(a, derive_user_tenant_id("inst", "user-1"));
        assert!(a.starts_with("tenant-"));
        // The 0x1F separator prevents (a,bc) colliding with (ab,c).
        assert_ne!(
            derive_user_tenant_id("a", "bc"),
            derive_user_tenant_id("ab", "c")
        );
    }

    #[test]
    fn user_subject_hash_is_sha256_shaped() {
        let h = user_subject_hash("user-1");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
        assert_eq!(h, user_subject_hash("user-1"));
        assert_ne!(h, user_subject_hash("user-2"));
    }

    #[test]
    fn attestation_signing_bytes_are_unambiguous() {
        let base = TraceInstanceEnrollAttestation {
            device_key_id: "sha256:aa".into(),
            aud: "trace-commons-ingest".into(),
            instance_id: "inst".into(),
            user_subject: "user-1".into(),
            nonce: "n".into(),
            exp: 100,
        };
        let mut moved = base.clone();
        moved.device_key_id = "sha256:a".into();
        moved.aud = "atrace-commons-ingest".into();
        // Field-boundary shift must change the signing bytes.
        assert_ne!(
            instance_enroll_attestation_signing_bytes(&base),
            instance_enroll_attestation_signing_bytes(&moved)
        );
    }

    #[test]
    fn instance_enroll_request_round_trips() {
        let req = TraceInstanceEnrollRequest {
            schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
            instance_public_key: "cHVia2V5".into(),
            device_public_key: "ZGV2a2V5".into(),
            attestation: TraceInstanceEnrollAttestation {
                device_key_id: "sha256:aa".into(),
                aud: "trace-commons-ingest".into(),
                instance_id: "inst".into(),
                user_subject: "user-1".into(),
                nonce: "n".into(),
                exp: 100,
            },
            attestation_sig: "c2ln".into(),
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".into(),
                version: "0.x".into(),
            },
        };
        let encoded = serde_json::to_string(&req).unwrap();
        let decoded: TraceInstanceEnrollRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, req);
    }
}
