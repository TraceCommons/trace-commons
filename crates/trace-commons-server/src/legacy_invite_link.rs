// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Linking a legacy `tenant-…` invite tenant to the NEAR account the caller
//! is signed into (V81). See docs/operator/legacy-invite-migration.md.
//!
//! The device signature is verified here, with the same raw Ed25519 check the
//! issuer applies to `x-trace-device-signature`, against the public key the
//! client presents. The database function then proves that key is the
//! legacy tenant's registered, live, invite-onboarded device and decides.
//! The countersignature is computed before that call so it can be stored in
//! the same transaction as the link it covers.

use base64::Engine as _;
use rand::RngCore as _;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use sha2::{Digest, Sha256};
use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkChallenge, LegacyInviteLinkRecord, LegacyInviteLinkRecordKind,
    LegacyInviteLinkRequest, LegacyInviteLinkResponse, LegacyInviteLinkStatement,
    legacy_invite_link_record_bytes, legacy_invite_link_statement_bytes,
};
use uuid::Uuid;

use crate::db::Database;

/// How long a challenge nonce stays redeemable.
pub const LEGACY_INVITE_LINK_CHALLENGE_TTL_SECONDS: i64 = 300;

/// Unspent, unexpired challenges one account may hold at once.
pub const LEGACY_INVITE_LINK_MAX_OPEN_CHALLENGES: i64 = 5;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Safe refusal labels. None names a tenant, account, device, or reason
/// beyond the class of refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkRefusal {
    #[error("legacy_link_request_invalid")]
    InvalidRequest,
    #[error("legacy_link_signature_invalid")]
    SignatureInvalid,
    #[error("legacy_link_challenge_invalid")]
    ChallengeInvalid,
    #[error("legacy_link_challenges_exhausted")]
    TooManyChallenges,
    #[error("legacy_link_account_ineligible")]
    AccountIneligible,
    #[error("legacy_link_device_not_eligible")]
    DeviceNotEligible,
    #[error("legacy_link_tenant_pooled")]
    TenantPooled,
    #[error("legacy_link_invite_revoked")]
    InviteRevoked,
    #[error("legacy_link_tenant_claimed")]
    TenantClaimed,
    /// The account already holds this legacy tenant's link, but under a
    /// different invite than the one this device joined with. That invite
    /// was never granted to the account, so this device cannot attest under
    /// the link (V104).
    #[error("legacy_link_invite_not_linked")]
    InviteNotLinked,
    #[error("legacy_link_unavailable")]
    Unavailable,
}

impl LinkRefusal {
    pub fn label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "legacy_link_request_invalid",
            Self::SignatureInvalid => "legacy_link_signature_invalid",
            Self::ChallengeInvalid => "legacy_link_challenge_invalid",
            Self::TooManyChallenges => "legacy_link_challenges_exhausted",
            Self::AccountIneligible => "legacy_link_account_ineligible",
            Self::DeviceNotEligible => "legacy_link_device_not_eligible",
            Self::TenantPooled => "legacy_link_tenant_pooled",
            Self::InviteRevoked => "legacy_link_invite_revoked",
            Self::TenantClaimed => "legacy_link_tenant_claimed",
            Self::InviteNotLinked => "legacy_link_invite_not_linked",
            Self::Unavailable => "legacy_link_unavailable",
        }
    }
}

/// Countersigning key: ingest's published attestation key, used for raw
/// Ed25519 over the domain-separated record bytes. The domain tag keeps these
/// signatures disjoint from the JWS score attestations the same key signs.
pub struct LegacyInviteLinkSigner {
    key: Ed25519KeyPair,
    kid: String,
}

impl std::fmt::Debug for LegacyInviteLinkSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyInviteLinkSigner")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

fn pem_body(pem: &str, label: &str) -> anyhow::Result<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let trimmed = pem.trim();
    let inner = trimmed
        .strip_prefix(&begin)
        .and_then(|rest| rest.strip_suffix(&end))
        .ok_or_else(|| anyhow::anyhow!("expected a single {label} PEM block"))?;
    let joined: String = inner.split_whitespace().collect();
    B64.decode(joined)
        .map_err(|_| anyhow::anyhow!("{label} PEM body is not base64"))
}

impl LegacyInviteLinkSigner {
    /// Build from the attestation key's PKCS#8 private PEM and SPKI public
    /// PEM. Refuses a public key that is not the private key's own, so the
    /// published keyset always verifies what this signs.
    pub fn from_pem(private_pem: &str, public_pem: &str, kid: &str) -> anyhow::Result<Self> {
        let pkcs8 = pem_body(private_pem, "PRIVATE KEY")?;
        let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&pkcs8)
            .map_err(|_| anyhow::anyhow!("attestation private key is not Ed25519 PKCS#8"))?;
        let spki = pem_body(public_pem, "PUBLIC KEY")?;
        // RFC 8410 SubjectPublicKeyInfo for Ed25519: a fixed 12-byte prefix
        // followed by the 32-byte key.
        anyhow::ensure!(
            spki.len() == 44 && spki[12..] == *key.public_key().as_ref(),
            "attestation public key does not match the private key"
        );
        anyhow::ensure!(!kid.trim().is_empty(), "attestation kid must not be blank");
        Ok(Self {
            key,
            kid: kid.trim().to_string(),
        })
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    pub fn public_key_bytes(&self) -> &[u8] {
        self.key.public_key().as_ref()
    }

    fn countersign(&self, record: &LegacyInviteLinkRecord) -> String {
        B64.encode(
            self.key
                .sign(&legacy_invite_link_record_bytes(record))
                .as_ref(),
        )
    }
}

/// Operator switch for the link routes. Off unless set: the pooled-tenant
/// markers must be in place before any legacy tenant can be claimed (see the
/// runbook), and deploying this code must not open the routes by itself.
pub const TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED_ENV: &str =
    "TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED";

/// `None`/`false`/`0` is off, `true`/`1` is on, anything else is an operator
/// error that fails startup.
pub fn parse_enabled(raw: Option<&str>) -> anyhow::Result<bool> {
    match raw.map(str::trim) {
        None | Some("") | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(_) => anyhow::bail!("legacy_invite_link_enabled_invalid"),
    }
}

/// The signer when the routes are enabled. Enabling them without the ingest
/// attestation key fails startup: there is no unsigned link record.
pub fn signer_from_env() -> anyhow::Result<Option<LegacyInviteLinkSigner>> {
    let raw = match std::env::var(TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED_ENV) {
        Ok(raw) => Some(raw),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("legacy_invite_link_enabled_invalid")
        }
    };
    if !parse_enabled(raw.as_deref())? {
        return Ok(None);
    }
    let config = crate::trace_score_attestation::AttestationConfig::from_env()?
        .ok_or_else(|| anyhow::anyhow!("legacy_invite_link_requires_attestation_key"))?;
    Ok(Some(LegacyInviteLinkSigner::from_pem(
        &config.signing_private_key_pem,
        &config.signing_public_key_pem,
        &config.signing_kid,
    )?))
}

/// What the database function decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDbOutcome {
    /// `linked` or `already_linked`: the caller's own stored record.
    Linked {
        record: Box<LegacyInviteLinkRecord>,
        server_signature: String,
        trust_version: i64,
    },
    Refused(LinkRefusal),
}

/// Everything the database function needs; built only after the device
/// signature verified.
#[derive(Debug, Clone)]
pub struct LinkDbAttempt {
    /// A link record (`kind` is `Link`).
    pub record: LegacyInviteLinkRecord,
    pub device_public_key: Vec<u8>,
    /// Countersignature over `record` as a link.
    pub server_signature: String,
    /// Countersignature over the same record as a `DeviceAttestation`,
    /// under the attestation domain. Used only when the link already exists
    /// for this account from another device, and the database records an
    /// attestation instead (V91/V104). Both are computed before the call so
    /// either can be stored in the transaction that decides.
    pub attestation_server_signature: String,
}

/// A new challenge row for the database to store, hash-only.
#[derive(Debug, Clone)]
pub struct ChallengeWrite {
    pub account_tenant: String,
    pub account_id: Uuid,
    pub nonce_hash: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

fn nonce_hash(nonce: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(nonce.as_bytes())))
}

fn is_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn is_nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_legacy_tenant(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !trace_commons_protocol::admission::is_anchored_tenant(value)
        && !value.starts_with("near-")
        && !value.starts_with("nearai-")
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Issue a single-use challenge to the authenticated NEAR account.
pub async fn issue_challenge(
    db: &dyn Database,
    account_tenant: &str,
    account_id: Uuid,
) -> Result<LegacyInviteLinkChallenge, LinkRefusal> {
    if !trace_commons_protocol::admission::is_anchored_tenant(account_tenant) {
        return Err(LinkRefusal::AccountIneligible);
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let nonce = hex::encode(bytes);
    let issued_at = now_unix();
    let expires_at = issued_at + LEGACY_INVITE_LINK_CHALLENGE_TTL_SECONDS;
    let stored = db
        .store_legacy_invite_link_challenge(&ChallengeWrite {
            account_tenant: account_tenant.to_string(),
            account_id,
            nonce_hash: nonce_hash(&nonce),
            issued_at,
            expires_at,
        })
        .await
        .map_err(|_| LinkRefusal::Unavailable)?;
    if !stored {
        return Err(LinkRefusal::TooManyChallenges);
    }
    Ok(LegacyInviteLinkChallenge {
        nonce,
        issued_at,
        expires_at,
    })
}

/// Verify, countersign and record a link from `request`'s legacy tenant to
/// the authenticated account. `account_tenant` and `account_id` must come
/// from the authenticated session, never from the request.
pub async fn link_legacy_invite(
    db: &dyn Database,
    signer: &LegacyInviteLinkSigner,
    account_tenant: &str,
    account_id: Uuid,
    request: &LegacyInviteLinkRequest,
) -> Result<LegacyInviteLinkResponse, LinkRefusal> {
    if !trace_commons_protocol::admission::is_anchored_tenant(account_tenant) {
        return Err(LinkRefusal::AccountIneligible);
    }
    if !is_legacy_tenant(&request.legacy_tenant_id)
        || !is_hash(&request.device_key_id)
        || !is_hash(&request.invite_subject_hash)
        || !is_nonce(&request.nonce)
    {
        return Err(LinkRefusal::InvalidRequest);
    }
    let public_key = B64
        .decode(request.device_public_key.trim())
        .map_err(|_| LinkRefusal::InvalidRequest)?;
    if public_key.len() != 32
        || trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&public_key)
            != request.device_key_id
    {
        return Err(LinkRefusal::InvalidRequest);
    }
    let signature = B64
        .decode(request.signature.trim())
        .map_err(|_| LinkRefusal::InvalidRequest)?;
    if signature.len() != 64 {
        return Err(LinkRefusal::InvalidRequest);
    }
    let statement = LegacyInviteLinkStatement {
        legacy_tenant_id: request.legacy_tenant_id.clone(),
        device_key_id: request.device_key_id.clone(),
        invite_subject_hash: request.invite_subject_hash.clone(),
        account_tenant_id: account_tenant.to_string(),
        account_id,
        nonce: request.nonce.clone(),
        issued_at: request.issued_at,
    };
    UnparsedPublicKey::new(&ED25519, &public_key)
        .verify(&legacy_invite_link_statement_bytes(&statement), &signature)
        .map_err(|_| LinkRefusal::SignatureInvalid)?;

    let record = LegacyInviteLinkRecord {
        link_id: Uuid::new_v4(),
        statement,
        device_signature: B64.encode(&signature),
        linked_at: now_unix(),
        server_kid: signer.kid().to_string(),
        kind: LegacyInviteLinkRecordKind::Link,
    };
    let server_signature = signer.countersign(&record);
    let attestation_server_signature = signer.countersign(&LegacyInviteLinkRecord {
        kind: LegacyInviteLinkRecordKind::DeviceAttestation,
        ..record.clone()
    });
    let outcome = db
        .link_legacy_invite(&LinkDbAttempt {
            record,
            device_public_key: public_key,
            server_signature,
            attestation_server_signature,
        })
        .await
        .map_err(|_| LinkRefusal::Unavailable)?;
    match outcome {
        LinkDbOutcome::Linked {
            record,
            server_signature,
            trust_version,
        } => Ok(LegacyInviteLinkResponse {
            record: *record,
            server_signature,
            authority: "invited".to_string(),
            trust_version,
        }),
        LinkDbOutcome::Refused(refusal) => Err(refusal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_refuses_a_mismatched_public_key() {
        let a = crate::trace_upload_claim_issuer::generate_upload_claim_keypair().unwrap();
        let b = crate::trace_upload_claim_issuer::generate_upload_claim_keypair().unwrap();
        assert!(
            LegacyInviteLinkSigner::from_pem(&a.private_key_pem, &a.public_key_pem, "k").is_ok()
        );
        assert!(
            LegacyInviteLinkSigner::from_pem(&a.private_key_pem, &b.public_key_pem, "k").is_err()
        );
        assert!(
            LegacyInviteLinkSigner::from_pem(&a.private_key_pem, &a.public_key_pem, " ").is_err()
        );
    }

    #[test]
    fn the_switch_is_off_unless_explicitly_on() {
        assert!(!parse_enabled(None).unwrap());
        assert!(!parse_enabled(Some("")).unwrap());
        assert!(!parse_enabled(Some("false")).unwrap());
        assert!(!parse_enabled(Some("0")).unwrap());
        assert!(parse_enabled(Some("true")).unwrap());
        assert!(parse_enabled(Some(" 1 ")).unwrap());
        assert!(parse_enabled(Some("yes")).is_err());
        assert!(parse_enabled(Some("TRUE")).is_err());
    }

    #[test]
    fn legacy_tenant_shape_excludes_the_account_namespaces() {
        assert!(is_legacy_tenant("tenant-deepak-jangir"));
        assert!(!is_legacy_tenant(&format!("near-{}", "a".repeat(64))));
        assert!(!is_legacy_tenant("nearai-anything"));
        assert!(!is_legacy_tenant(""));
        assert!(!is_legacy_tenant("tenant with space"));
        assert!(!is_legacy_tenant(&"t".repeat(129)));
    }
}
