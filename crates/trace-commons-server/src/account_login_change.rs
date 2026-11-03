// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! F10: changing the NEAR AI login an account uses.
//!
//! Spec: `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`.
//! The routes live in the ingest service; this module holds the ceremony
//! payload and the types the database layer takes and answers. The wire
//! contract is `trace_commons_protocol::account_login_change`.

use sha2::{Digest, Sha256};
use trace_commons_protocol::account_login_change::IdentityKind;

/// The only purpose a change ceremony row carries. A tagged, required field
/// with `deny_unknown_fields` on every ceremony payload keeps a change
/// ceremony from being taken as a sign-up one and the reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebindPurpose {
    Rebind,
}

/// A change ceremony between start and finish. Everything finish checks
/// against; nothing the client supplies at finish. Holds no NEAR account
/// name: a wallet ceremony stores a commitment to the name typed at start.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearAiRebindPending {
    pub purpose: RebindPurpose,
    pub tenant_id: String,
    pub account_id: String,
    pub session_token_hash: String,
    pub device_public_key: String,
    pub nonce_hex: String,
    pub code_challenge: String,
    pub expires_at: i64,
    pub current_kind: IdentityKind,
    #[serde(default)]
    pub wallet_commitment: Option<String>,
}

/// The commitment a wallet ceremony stores for the account name typed at
/// start, checked again at finish.
pub fn wallet_commitment(network: &str, account: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"trace_commons.near_ai_rebind_wallet_commitment.v1\n");
    for part in [network.as_bytes(), account.as_bytes()] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    format!("sha256:{}", hex::encode(hash.finalize()))
}

/// The session a change runs under, from the auth context.
#[derive(Debug, Clone)]
pub struct RebindSession {
    pub tenant_id: String,
    pub account_id: uuid::Uuid,
    pub session_token_hash: String,
    pub device_key_id: String,
}

/// What start learns: the account's current identity, or why it cannot
/// change now. Refusal labels are for logs only, never for the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebindPrecheck {
    Ready { current_kind: IdentityKind },
    TooSoon,
    Refused(&'static str),
}

/// The proven current identity, as an anchor of this deployment.
#[derive(Debug, Clone)]
pub enum RebindCurrent {
    NearAiLogin {
        anchor_hash: String,
    },
    Wallet {
        anchor_hash: String,
        near_account_id: String,
    },
}

impl RebindCurrent {
    pub fn anchor_hash(&self) -> &str {
        match self {
            Self::NearAiLogin { anchor_hash } | Self::Wallet { anchor_hash, .. } => anchor_hash,
        }
    }

    pub fn kind(&self) -> IdentityKind {
        match self {
            Self::NearAiLogin { .. } => IdentityKind::NearAiLogin,
            Self::Wallet { .. } => IdentityKind::Wallet,
        }
    }
}

/// How a commit ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebindOutcome {
    Rebound {
        anchor_hash: String,
        auth_provider: String,
        switched_back: bool,
    },
    /// The new login already anchors an account.
    InUse,
    /// A change of this account inside the cooldown.
    TooSoon,
    /// A wallet-origin account holds a waiting payout link, which should not
    /// exist after #1862 and V147; an operator looks before anything moves.
    PayoutReviewRequired,
    /// Every other refusal: the uniform deny. Label for logs only.
    Refused(&'static str),
}

/// The days a change must wait after the previous one (owner, 2026-11-03).
pub use trace_commons_protocol::account_login_change::REBIND_COOLDOWN_DAYS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_ceremony_never_reads_as_a_sign_up_one_or_the_reverse() {
        let pending = NearAiRebindPending {
            purpose: RebindPurpose::Rebind,
            tenant_id: "nearai-x".into(),
            account_id: uuid::Uuid::nil().to_string(),
            session_token_hash: "sha256:00".into(),
            device_public_key: "d".into(),
            nonce_hex: "00".into(),
            code_challenge: "c".into(),
            expires_at: 1,
            current_kind: IdentityKind::NearAiLogin,
            wallet_commitment: None,
        };
        let value = serde_json::to_value(&pending).unwrap();
        assert!(
            serde_json::from_value::<crate::account_onboarding::NearAiLoginPending>(value.clone())
                .is_err()
        );
        assert!(
            serde_json::from_value::<crate::account_onboarding::NativeProvisioningPending>(
                value.clone()
            )
            .is_err()
        );
        let login = crate::account_onboarding::NearAiLoginPending {
            nonce_hex: "00".into(),
            code_challenge: "c".into(),
            device_public_key: "d".into(),
            expires_at: 1,
        };
        assert!(
            serde_json::from_value::<NearAiRebindPending>(serde_json::to_value(login).unwrap())
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<NearAiRebindPending>(value)
                .unwrap()
                .purpose,
            RebindPurpose::Rebind
        );
    }

    #[test]
    fn the_wallet_commitment_binds_network_and_name() {
        let a = wallet_commitment("testnet", "a.testnet");
        assert_ne!(a, wallet_commitment("mainnet", "a.testnet"));
        assert_ne!(a, wallet_commitment("testnet", "b.testnet"));
        assert!(a.starts_with("sha256:"));
    }
}
