//! A device asking the issuer which invite it was onboarded with.
//!
//! The legacy invite migration (#1066 and its client half) has the device sign
//! a statement naming the `invite_subject_hash` its tenant was onboarded
//! under. Clients never stored that hash, and for most invitees the raw code
//! was handed over out of band and is gone. So the issuer, which holds the
//! device registry, tells a device its own hash -- and only its own, only to
//! a request signed by that device's key.
//!
//! The request is authenticated exactly as a device-key upload claim is: the
//! device signs the exact JSON body bytes with raw Ed25519 and sends
//! `x-trace-device-key-id` and `x-trace-device-signature` beside it.
//! `issued_at` bounds replay: the issuer refuses a body more than
//! [`DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS`] from its own clock.

use serde::{Deserialize, Serialize};

/// Issuer route.
pub const DEVICE_INVITE_SUBJECT_PATH: &str = "/v1/device/invite-subject";

pub const DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION: &str =
    "trace_commons.device_invite_subject_request.v1";

/// How far `issued_at` may sit from the issuer's clock, either way.
pub const DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS: i64 = 300;

/// Refusal labels, in the issuer's `{"error": ...}` body.
pub const DEVICE_INVITE_SUBJECT_NOT_REGISTERED: &str = "device_key_not_registered";
pub const DEVICE_INVITE_SUBJECT_REVOKED: &str = "device_key_revoked";
pub const DEVICE_INVITE_SUBJECT_NOT_INVITE_ONBOARDED: &str = "device_not_invite_onboarded";
pub const DEVICE_INVITE_SUBJECT_STALE: &str = "device_invite_subject_request_stale";

/// `POST /v1/device/invite-subject` body. Signed as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInviteSubjectRequest {
    pub schema_version: String,
    pub tenant_id: String,
    /// Must equal the `x-trace-device-key-id` header.
    pub device_key_id: String,
    /// Unix seconds.
    pub issued_at: i64,
}

/// The device's own invite subject hash, `sha256:` + 64 lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInviteSubjectResponse {
    pub invite_subject_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_are_refused_in_both_directions() {
        assert!(
            serde_json::from_value::<DeviceInviteSubjectRequest>(serde_json::json!({
                "schema_version": DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION,
                "tenant_id": "tenant-a",
                "device_key_id": "sha256:00",
                "issued_at": 1,
                "invite_code": "SMUGGLED",
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DeviceInviteSubjectResponse>(serde_json::json!({
                "invite_subject_hash": "sha256:00",
                "tenant_id": "tenant-a",
            }))
            .is_err()
        );
    }
}
