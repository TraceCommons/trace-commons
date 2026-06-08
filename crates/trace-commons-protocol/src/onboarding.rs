//! Agent-driven pilot onboarding wire contract.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TRACE_ONBOARD_REQUEST_SCHEMA_VERSION: &str = "trace_commons.onboard_request.v1";
pub const TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION: &str = "trace_commons.onboard_response.v1";

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TraceOnboardErrorCode {
    InviteNotValid,
    InviteMalformed,
    DeviceKeyMalformed,
    OnboardRateLimited,
    OnboardAllowlistNotConfigured,
    OnboardRegistryNotConfigured,
    OnboardTenantConfigMissing,
    OnboardAllowlistStale,
}

impl TraceOnboardErrorCode {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::InviteNotValid => "InviteNotValid",
            Self::InviteMalformed => "InviteMalformed",
            Self::DeviceKeyMalformed => "DeviceKeyMalformed",
            Self::OnboardRateLimited => "OnboardRateLimited",
            Self::OnboardAllowlistNotConfigured => "OnboardAllowlistNotConfigured",
            Self::OnboardRegistryNotConfigured => "OnboardRegistryNotConfigured",
            Self::OnboardTenantConfigMissing => "OnboardTenantConfigMissing",
            Self::OnboardAllowlistStale => "OnboardAllowlistStale",
        }
    }
}

pub fn device_key_id_from_public_key_bytes(public_key_bytes: &[u8]) -> String {
    let digest = Sha256::digest(public_key_bytes);
    format!("sha256:{}", hex::encode(digest))
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
}
