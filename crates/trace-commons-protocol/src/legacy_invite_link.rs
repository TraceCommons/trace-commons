//! Linking a legacy `tenant-…` invite identity to a NEAR account.
//!
//! The holder of a legacy invite device key signs a statement naming the
//! legacy tenant and the NEAR account it is moving to. Ingest verifies that
//! signature, records the link, and countersigns the record, so the daemon
//! can later prove -- to itself -- that the old identity chose to move onto
//! this account before it re-baselines armed folders (see the consent spec,
//! "The invite-to-account migration re-baselines rather than voids").
//!
//! Both byte encodings below are the single source of truth for signer and
//! verifier. Every field is length-prefixed (u64 little-endian) after a
//! newline-terminated domain tag, so no boundary shift can make two different
//! statements encode to the same bytes, and a statement can never be replayed
//! as a record or vice versa.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Domain tag of the statement the legacy device key signs.
pub const LEGACY_INVITE_LINK_STATEMENT_DOMAIN: &str = "trace-commons.legacy-invite-link.v1";

/// Domain tag of the record ingest countersigns.
pub const LEGACY_INVITE_LINK_RECORD_DOMAIN: &str = "trace-commons.legacy-invite-link-record.v1";

/// What the legacy device key signs. `account_tenant_id` and `account_id` are
/// the NEAR account the session is signed into; the server fills them from
/// the authenticated session and never from the request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyInviteLinkStatement {
    pub legacy_tenant_id: String,
    pub device_key_id: String,
    pub invite_subject_hash: String,
    pub account_tenant_id: String,
    pub account_id: Uuid,
    /// Lowercase hex of the server-issued challenge nonce.
    pub nonce: String,
    /// Unix seconds, exactly as the challenge response stated it.
    pub issued_at: i64,
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_le_bytes());
    out.extend_from_slice(field);
}

fn push_domain(out: &mut Vec<u8>, domain: &str) {
    out.extend_from_slice(domain.as_bytes());
    out.push(b'\n');
}

fn push_statement_fields(out: &mut Vec<u8>, s: &LegacyInviteLinkStatement) {
    push_field(out, s.legacy_tenant_id.as_bytes());
    push_field(out, s.device_key_id.as_bytes());
    push_field(out, s.invite_subject_hash.as_bytes());
    push_field(out, s.account_tenant_id.as_bytes());
    push_field(out, s.account_id.hyphenated().to_string().as_bytes());
    push_field(out, s.nonce.as_bytes());
    push_field(out, s.issued_at.to_string().as_bytes());
}

/// The exact bytes the legacy device key signs with raw Ed25519.
pub fn legacy_invite_link_statement_bytes(s: &LegacyInviteLinkStatement) -> Vec<u8> {
    let mut out = Vec::new();
    push_domain(&mut out, LEGACY_INVITE_LINK_STATEMENT_DOMAIN);
    push_statement_fields(&mut out, s);
    out
}

/// The stored link as the server countersigns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyInviteLinkRecord {
    pub link_id: Uuid,
    pub statement: LegacyInviteLinkStatement,
    /// Standard base64 of the 64-byte device signature over the statement.
    pub device_signature: String,
    /// Unix seconds at which the server recorded the link.
    pub linked_at: i64,
    /// Key id of the ingest attestation key that countersigned.
    pub server_kid: String,
}

/// The exact bytes ingest signs with raw Ed25519 to countersign a record.
pub fn legacy_invite_link_record_bytes(r: &LegacyInviteLinkRecord) -> Vec<u8> {
    let mut out = Vec::new();
    push_domain(&mut out, LEGACY_INVITE_LINK_RECORD_DOMAIN);
    push_field(&mut out, r.link_id.hyphenated().to_string().as_bytes());
    push_statement_fields(&mut out, &r.statement);
    push_field(&mut out, r.device_signature.as_bytes());
    push_field(&mut out, r.linked_at.to_string().as_bytes());
    push_field(&mut out, r.server_kid.as_bytes());
    out
}

/// `POST /v1/account/invites/legacy-link/challenge` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyInviteLinkChallenge {
    pub nonce: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// `POST /v1/account/invites/legacy-link` request. The account side of the
/// statement is taken from the session, so it is not a request field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyInviteLinkRequest {
    pub legacy_tenant_id: String,
    pub device_key_id: String,
    /// Standard base64 of the 32-byte Ed25519 device public key.
    pub device_public_key: String,
    pub invite_subject_hash: String,
    pub nonce: String,
    pub issued_at: i64,
    /// Standard base64 of the 64-byte signature over the statement bytes.
    pub signature: String,
}

/// `POST /v1/account/invites/legacy-link` success response: the countersigned
/// record the client keeps, plus the account's invite trust after the link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyInviteLinkResponse {
    pub record: LegacyInviteLinkRecord,
    /// Standard base64 of the 64-byte ingest signature over the record bytes.
    pub server_signature: String,
    pub authority: String,
    pub trust_version: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement() -> LegacyInviteLinkStatement {
        LegacyInviteLinkStatement {
            legacy_tenant_id: "tenant-example".into(),
            device_key_id: format!("sha256:{}", "a".repeat(64)),
            invite_subject_hash: format!("sha256:{}", "b".repeat(64)),
            account_tenant_id: format!("near-{}", "c".repeat(64)),
            account_id: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            nonce: "d".repeat(64),
            issued_at: 1_790_000_000,
        }
    }

    #[test]
    fn statement_bytes_start_with_the_domain_and_length_prefix_every_field() {
        let bytes = legacy_invite_link_statement_bytes(&statement());
        let domain = b"trace-commons.legacy-invite-link.v1\n";
        assert_eq!(&bytes[..domain.len()], domain);
        let mut rest = &bytes[domain.len()..];
        let mut fields = Vec::new();
        while !rest.is_empty() {
            let len = u64::from_le_bytes(rest[..8].try_into().unwrap()) as usize;
            fields.push(String::from_utf8(rest[8..8 + len].to_vec()).unwrap());
            rest = &rest[8 + len..];
        }
        assert_eq!(
            fields,
            vec![
                "tenant-example".to_string(),
                format!("sha256:{}", "a".repeat(64)),
                format!("sha256:{}", "b".repeat(64)),
                format!("near-{}", "c".repeat(64)),
                "01234567-89ab-cdef-0123-456789abcdef".to_string(),
                "d".repeat(64),
                "1790000000".to_string(),
            ]
        );
    }

    #[test]
    fn a_boundary_shift_changes_the_statement_bytes() {
        let a = statement();
        let mut b = statement();
        b.legacy_tenant_id = "tenant-exampl".into();
        b.device_key_id = format!("e{}", a.device_key_id);
        assert_ne!(
            legacy_invite_link_statement_bytes(&a),
            legacy_invite_link_statement_bytes(&b)
        );
    }

    #[test]
    fn every_statement_field_is_bound() {
        let base = legacy_invite_link_statement_bytes(&statement());
        let mutations: Vec<Box<dyn Fn(&mut LegacyInviteLinkStatement)>> = vec![
            Box::new(|s| s.legacy_tenant_id.push('x')),
            Box::new(|s| s.device_key_id.push('x')),
            Box::new(|s| s.invite_subject_hash.push('x')),
            Box::new(|s| s.account_tenant_id.push('x')),
            Box::new(|s| s.account_id = Uuid::nil()),
            Box::new(|s| s.nonce.push('x')),
            Box::new(|s| s.issued_at += 1),
        ];
        for mutate in mutations {
            let mut s = statement();
            mutate(&mut s);
            assert_ne!(legacy_invite_link_statement_bytes(&s), base);
        }
    }

    #[test]
    fn a_record_never_encodes_as_a_statement() {
        let record = LegacyInviteLinkRecord {
            link_id: Uuid::nil(),
            statement: statement(),
            device_signature: "sig".into(),
            linked_at: 1_790_000_001,
            server_kid: "ingest-1".into(),
        };
        let record_bytes = legacy_invite_link_record_bytes(&record);
        assert!(record_bytes.starts_with(b"trace-commons.legacy-invite-link-record.v1\n"));
        assert!(!record_bytes.starts_with(b"trace-commons.legacy-invite-link.v1\n"));
        let mut other = record.clone();
        other.linked_at += 1;
        assert_ne!(legacy_invite_link_record_bytes(&other), record_bytes);
        let mut other = record.clone();
        other.server_kid = "ingest-2".into();
        assert_ne!(legacy_invite_link_record_bytes(&other), record_bytes);
        let mut other = record;
        other.device_signature = "sih".into();
        assert_ne!(legacy_invite_link_record_bytes(&other), record_bytes);
    }
}
