//! Wire contract for linking a headless device to an account with a
//! one-time code (design: "Linking more devices to a NEAR AI account",
//! 2026-10-27; owner decision: the signed-in laptop mints, the server
//! redeems).
//!
//! - `POST /v1/account/device-codes` mints a code on the signed-in account
//!   session. The body carries a proof signed by the minting device's own
//!   key, so a stolen session token alone cannot mint.
//! - `GET /v1/account/device-codes/{code_id}` tells the minting device
//!   whether its code was used, and by which device key.
//! - `POST /v1/device-codes/redeem` is unauthenticated: the code is the
//!   credential. The redeeming device signs over the code with its own key,
//!   and the answer is the same sign-in a NEAR AI provisioning returns.
//! - `POST /v1/device-sessions/renew` is unauthenticated too: a device that
//!   joined by code has no NEAR AI login, so it renews its 12-hour account
//!   session by signing a fresh nonce with its device key. Only a device
//!   that joined by code, and is not revoked, gets a session.
//!
//! A code is `tcd1_{b64url(tenant_id)}.{secret}`. It is single-use, lives
//! [`DEVICE_CODE_TTL_SECS`], and only its hash is stored.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Prefix of every device code.
pub const DEVICE_CODE_PREFIX: &str = "tcd1_";

/// How long a minted code can be redeemed.
pub const DEVICE_CODE_TTL_SECS: i64 = 600;

/// How far a mint proof's `issued_at` may sit from the server's clock.
pub const DEVICE_CODE_MINT_SKEW_SECS: i64 = 120;

/// Longest label a minted code may carry, in characters.
pub const DEVICE_CODE_LABEL_MAX_CHARS: usize = 64;

/// The mint route, on the account session.
pub const DEVICE_CODE_MINT_PATH: &str = "/v1/account/device-codes";

/// The redeem route, unauthenticated.
pub const DEVICE_CODE_REDEEM_PATH: &str = "/v1/device-codes/redeem";

/// The device-key session renewal route, unauthenticated.
pub const DEVICE_SESSION_RENEW_PATH: &str = "/v1/device-sessions/renew";

/// How far a renewal proof's `issued_at` may sit from the server's clock.
pub const DEVICE_SESSION_RENEW_SKEW_SECS: i64 = 120;

/// The status route for one minted code, `{code_id}` substituted.
pub fn device_code_status_path(code_id: &str) -> String {
    format!("{DEVICE_CODE_MINT_PATH}/{code_id}")
}

fn length_prefixed(domain: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut bytes = domain.to_vec();
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
        bytes.extend_from_slice(part);
    }
    bytes
}

/// What the minting device signs. Binds the proof to the account the
/// session names, the minting device's key, a fresh nonce the server never
/// accepts twice, and the time.
pub fn device_code_mint_bytes(
    tenant_id: &str,
    account_id: &str,
    minter_device: &[u8; 32],
    nonce: &[u8; 32],
    issued_at: i64,
) -> Vec<u8> {
    let issued = issued_at.to_be_bytes();
    length_prefixed(
        b"trace_commons.account_device_code_mint.v1\n",
        &[
            tenant_id.as_bytes(),
            account_id.as_bytes(),
            &minter_device[..],
            &nonce[..],
            &issued[..],
        ],
    )
}

/// `sha256:<hex>` of the whole code string, as both sides sign it.
pub fn device_code_digest(code: &str) -> String {
    format!("sha256:{}", hex_lower(&Sha256::digest(code.as_bytes())))
}

/// What the redeeming device signs: the code it holds and its own key.
pub fn device_code_redeem_bytes(code: &str, device: &[u8; 32]) -> Vec<u8> {
    let digest = device_code_digest(code);
    length_prefixed(
        b"trace_commons.account_device_code_redeem.v1\n",
        &[digest.as_bytes(), &device[..]],
    )
}

/// What a device that joined by code signs to renew its account session:
/// its tenant, its key, a fresh nonce the server never accepts twice, and
/// the time.
pub fn device_session_renew_bytes(
    tenant_id: &str,
    device: &[u8; 32],
    nonce: &[u8; 32],
    issued_at: i64,
) -> Vec<u8> {
    let issued = issued_at.to_be_bytes();
    length_prefixed(
        b"trace_commons.account_device_session_renew.v1\n",
        &[tenant_id.as_bytes(), &device[..], &nonce[..], &issued[..]],
    )
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Body of `POST /v1/account/device-codes`. Keys and the nonce are standard
/// base64; `signature` is the minting device's Ed25519 signature over
/// [`device_code_mint_bytes`], standard base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCodeMintRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub minter_public_key: String,
    pub nonce: String,
    pub issued_at: i64,
    pub signature: String,
}

/// Answer to a mint: the code, shown once, and its id for the status route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCodeMintResponse {
    pub code_id: String,
    pub code: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Where a minted code stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCodeState {
    Pending,
    Redeemed,
    Expired,
}

/// Answer to `GET /v1/account/device-codes/{code_id}`. `device_key_id` is
/// set once the code is redeemed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCodeStatusResponse {
    pub state: DeviceCodeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_key_id: Option<String>,
}

/// Body of `POST /v1/device-codes/redeem`. `device_public_key` and
/// `signature` (over [`device_code_redeem_bytes`]) are standard base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCodeRedeemRequest {
    pub code: String,
    pub device_public_key: String,
    pub signature: String,
}

/// Body of `POST /v1/device-sessions/renew`. `device_public_key`, `nonce`
/// and `signature` (over [`device_session_renew_bytes`]) are standard base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSessionRenewRequest {
    pub tenant_id: String,
    pub device_public_key: String,
    pub nonce: String,
    pub issued_at: i64,
    pub signature: String,
}

/// Answer to a redeem or a renewal: the same sign-in a NEAR AI provisioning
/// finish returns, so the client stores it the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSignInResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in_secs: i64,
    pub account_id: String,
    pub tenant_id: String,
    pub device_key_id: String,
    pub anchor_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_proof_has_its_own_domain_and_binds_every_part() {
        let mint = device_code_mint_bytes("t", "a", &[1; 32], &[2; 32], 7);
        assert!(mint.starts_with(b"trace_commons.account_device_code_mint.v1\n"));
        assert_ne!(
            mint,
            device_code_mint_bytes("t", "a", &[1; 32], &[2; 32], 8)
        );
        assert_ne!(
            mint,
            device_code_mint_bytes("t", "b", &[1; 32], &[2; 32], 7)
        );
        assert_ne!(
            mint,
            device_code_mint_bytes("t", "a", &[3; 32], &[2; 32], 7)
        );
        // Length prefixes keep a moved boundary from colliding.
        assert_ne!(
            device_code_mint_bytes("ta", "", &[1; 32], &[2; 32], 7),
            device_code_mint_bytes("t", "a", &[1; 32], &[2; 32], 7)
        );
        let redeem = device_code_redeem_bytes("tcd1_x.y", &[1; 32]);
        assert!(redeem.starts_with(b"trace_commons.account_device_code_redeem.v1\n"));
        assert_ne!(redeem, device_code_redeem_bytes("tcd1_x.z", &[1; 32]));
        assert_ne!(redeem, device_code_redeem_bytes("tcd1_x.y", &[2; 32]));
        let renew = device_session_renew_bytes("t", &[1; 32], &[2; 32], 7);
        assert!(renew.starts_with(b"trace_commons.account_device_session_renew.v1\n"));
        assert_ne!(
            renew,
            device_session_renew_bytes("u", &[1; 32], &[2; 32], 7)
        );
        assert_ne!(
            renew,
            device_session_renew_bytes("t", &[1; 32], &[3; 32], 7)
        );
    }

    #[test]
    fn the_digest_is_prefixed_lowercase_sha256() {
        assert_eq!(
            device_code_digest("abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_mint_request_refuses_unknown_fields() {
        let body =
            r#"{"minter_public_key":"k","nonce":"n","issued_at":1,"signature":"s","extra":1}"#;
        assert!(serde_json::from_str::<DeviceCodeMintRequest>(body).is_err());
    }
}
