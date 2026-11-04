//! Wire contract for changing the NEAR AI login an account uses (F10).
//!
//! Spec: `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`.
//!
//! - `POST /v1/account/near-ai/rebind/start`, on the signed-in native
//!   session, from the session's own device. Answers a single-use ceremony,
//!   and for an account whose current identity is a NEAR wallet, the NEP-413
//!   challenge the wallet signs.
//! - `POST /v1/account/near-ai/rebind/finish`, on the same session. Carries
//!   the proof of the current identity (a NEAR AI access token, or a wallet
//!   signature), the new NEAR AI login's access token, and the device's
//!   signature over [`rebind_device_bytes`], which binds both proofs to the
//!   ceremony.
//!
//! Both halves call the functions here; neither writes its own byte layout.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The start route.
pub const REBIND_START_PATH: &str = "/v1/account/near-ai/rebind/start";
/// The finish route.
pub const REBIND_FINISH_PATH: &str = "/v1/account/near-ai/rebind/finish";

/// How long a change ceremony lives. It spans two browser legs (a wallet
/// approval or a NEAR AI sign-in, then a NEAR AI sign-in), so it is longer
/// than the five minutes a sign-up ceremony gets.
pub const REBIND_CEREMONY_TTL_SECS: i64 = 900;

/// Days between two changes of one account (owner decision 9, 2026-11-03).
pub const REBIND_COOLDOWN_DAYS: i64 = 7;

/// Refusal labels a finish or start answers by name. Every other refusal is
/// the uniform deny.
pub const REBIND_IN_USE: &str = "near_ai_login_in_use";
pub const REBIND_TOO_SOON: &str = "login_change_too_soon";
pub const REBIND_WALLET_ACCOUNT_REQUIRED: &str = "wallet_account_required";
pub const REBIND_PAYOUT_REVIEW_REQUIRED: &str = "payout_link_review_required";
/// A sign-in with a login a change retired (provisioning finish).
pub const NEAR_AI_LOGIN_RETIRED: &str = "near_ai_login_retired";

/// What the account signs in with: the identity kind a device records
/// (decision 4), not read from the tenant prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    Wallet,
    NearAiLogin,
}

impl IdentityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wallet => "wallet",
            Self::NearAiLogin => "near_ai_login",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RebindStartRequest {
    pub device_public_key: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    /// The NEAR account name, typed by the person, when the account's current
    /// identity is a wallet. Checked at finish, never answered at start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_account_id: Option<String>,
}

/// The NEP-413 request a wallet signs, for a wallet current identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RebindWalletChallenge {
    pub message: String,
    pub recipient: String,
    pub wallet_url: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RebindStartResponse {
    pub ceremony_id: String,
    /// 32 bytes, standard base64, as the provisioning ceremonies send it.
    pub nonce: String,
    pub expires_at: i64,
    pub current_kind: IdentityKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet: Option<RebindWalletChallenge>,
}

/// The proof of the account's current identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentProof {
    NearAiLogin {
        access_token: String,
    },
    Wallet {
        account_id: String,
        wallet_public_key: String,
        wallet_signature: String,
    },
}

impl CurrentProof {
    /// The digest the device signature covers.
    pub fn digest(&self) -> [u8; 32] {
        match self {
            Self::NearAiLogin { access_token } => proof_digest(b"near_ai_login", access_token),
            Self::Wallet {
                wallet_signature, ..
            } => proof_digest(b"wallet", wallet_signature),
        }
    }

    pub fn kind(&self) -> IdentityKind {
        match self {
            Self::NearAiLogin { .. } => IdentityKind::NearAiLogin,
            Self::Wallet { .. } => IdentityKind::Wallet,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RebindFinishRequest {
    pub ceremony_id: String,
    pub code_verifier: String,
    pub device_public_key: String,
    pub device_signature: String,
    pub current_proof: CurrentProof,
    pub new_access_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RebindFinishResponse {
    pub tenant_id: String,
    pub account_id: String,
    pub anchor_hash: String,
    pub auth_provider: String,
    pub identity: IdentityKind,
    pub switched_back: bool,
}

/// The digest of the new login's access token the device signature covers.
pub fn new_login_digest(access_token: &str) -> [u8; 32] {
    proof_digest(b"new_login", access_token)
}

fn proof_digest(kind: &[u8], value: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"trace_commons.near_ai_rebind_proof.v1\n");
    for part in [kind, value.as_bytes()] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.finalize().into()
}

/// The text a NEAR wallet signs to prove it is the account's current
/// identity. Its purpose line keeps it apart from the sign-up message
/// (`onboarding::near_provisioning_message`), so neither signature stands in
/// for the other.
pub fn rebind_wallet_message(
    network: &str,
    account: &str,
    device: &[u8; 32],
    expires_at: i64,
) -> String {
    format!(
        "Change the NEAR AI login my account uses\nPurpose: trace_commons.near_ai_rebind.v1\nNetwork: {network}\nAccount: {account}\nDevice: sha256:{}\nExpires: {expires_at}",
        hex::encode(Sha256::digest(device))
    )
}

/// The bytes the account's device signs at finish. Every part is
/// length-prefixed, the integer included, as
/// `onboarding::near_ai_provisioning_device_bytes` does.
pub fn rebind_device_bytes(
    nonce: &[u8; 32],
    ceremony_id: &str,
    device: &[u8; 32],
    code_challenge: &str,
    expires_at: i64,
    current_proof: &[u8; 32],
    new_login: &[u8; 32],
) -> Vec<u8> {
    let expiry = expires_at.to_be_bytes();
    let mut bytes = b"trace_commons.near_ai_rebind_device.v1\n".to_vec();
    for part in [
        &nonce[..],
        ceremony_id.as_bytes(),
        &device[..],
        code_challenge.as_bytes(),
        &expiry[..],
        &current_proof[..],
        &new_login[..],
    ] {
        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
        bytes.extend_from_slice(part);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_bytes_differ_from_both_provisioning_ceremonies() {
        let nonce = [7u8; 32];
        let device = [9u8; 32];
        let rebind = rebind_device_bytes(&nonce, "c", &device, "ch", 100, &[1; 32], &[2; 32]);
        let near_ai =
            crate::onboarding::near_ai_provisioning_device_bytes(&nonce, "c", &device, "ch", 100);
        assert!(rebind.starts_with(b"trace_commons.near_ai_rebind_device.v1\n"));
        assert!(!rebind.starts_with(b"trace_commons.near_ai_provisioning_device.v1\n"));
        assert_ne!(rebind, near_ai);
    }

    #[test]
    fn device_bytes_bind_both_proofs() {
        let base = rebind_device_bytes(&[0; 32], "c", &[0; 32], "ch", 1, &[1; 32], &[2; 32]);
        assert_ne!(
            base,
            rebind_device_bytes(&[0; 32], "c", &[0; 32], "ch", 1, &[3; 32], &[2; 32])
        );
        assert_ne!(
            base,
            rebind_device_bytes(&[0; 32], "c", &[0; 32], "ch", 1, &[1; 32], &[3; 32])
        );
    }

    /// The exact layout, so a second implementation cannot quietly disagree.
    #[test]
    fn device_bytes_layout_is_pinned() {
        let bytes = rebind_device_bytes(&[0; 32], "c", &[0; 32], "x", 1, &[0; 32], &[0; 32]);
        let mut expected = b"trace_commons.near_ai_rebind_device.v1\n".to_vec();
        for part in [
            vec![0u8; 32],
            b"c".to_vec(),
            vec![0u8; 32],
            b"x".to_vec(),
            1i64.to_be_bytes().to_vec(),
            vec![0u8; 32],
            vec![0u8; 32],
        ] {
            expected.extend_from_slice(&(part.len() as u64).to_le_bytes());
            expected.extend_from_slice(&part);
        }
        assert_eq!(bytes, expected);
    }

    #[test]
    fn the_wallet_message_is_not_the_sign_up_message() {
        let device = [9u8; 32];
        let change = rebind_wallet_message("testnet", "a.testnet", &device, 5);
        let signup = crate::onboarding::near_provisioning_message(
            "testnet",
            "a.testnet",
            &device,
            &[1; 32],
            5,
        );
        assert_ne!(change, signup);
        assert!(change.contains("Purpose: trace_commons.near_ai_rebind.v1"));
        assert!(!change.contains("Trace Commons"));
    }

    #[test]
    fn a_proof_digest_depends_on_its_kind() {
        let token = CurrentProof::NearAiLogin {
            access_token: "same".into(),
        };
        let wallet = CurrentProof::Wallet {
            account_id: "a.testnet".into(),
            wallet_public_key: "ed25519:x".into(),
            wallet_signature: "same".into(),
        };
        assert_ne!(token.digest(), wallet.digest());
        assert_ne!(token.digest(), new_login_digest("same"));
    }

    #[test]
    fn finish_requests_round_trip_and_refuse_unknown_fields() {
        let request = RebindFinishRequest {
            ceremony_id: "c".into(),
            code_verifier: "v".into(),
            device_public_key: "d".into(),
            device_signature: "s".into(),
            current_proof: CurrentProof::Wallet {
                account_id: "a.testnet".into(),
                wallet_public_key: "ed25519:x".into(),
                wallet_signature: "w".into(),
            },
            new_access_token: "t".into(),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["current_proof"]["kind"], "wallet");
        let back: RebindFinishRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(back, request);
        let mut extra = value;
        extra["account_id"] = "smuggled".into();
        assert!(serde_json::from_value::<RebindFinishRequest>(extra).is_err());
    }
}
