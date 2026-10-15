// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Signed cumulative-credit statements for the pull delivery mode.
//!
//! In pull mode NEAR AI fetches, per bound organization, a statement of the
//! cumulative Trace Commons credit we have accrued for it, verifies our
//! Ed25519 signature against a pinned public key, and applies the total
//! through the same monotonic write the push route uses. The statement is
//! cumulative, so a stale or replayed one can never raise the limit past a
//! newer one and never lowers it.
//!
//! The signed bytes are a fixed, line-separated preimage -- not JSON -- so a
//! verifier in another language reproduces them without a canonicalisation
//! library. Nothing in a statement names a tenant.

use base64::Engine;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const DOMAIN: &str = "trace-commons:near-ai-credit-statement:v1";

/// One organization's cumulative credit, as of `issued_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditStatement {
    pub org_id: String,
    /// Decimal nano-USD; a string so no JSON number precision is lost.
    pub cumulative_nano_usd: String,
    pub statement_ref: String,
    /// RFC 3339, UTC.
    pub issued_at: String,
}

impl CreditStatement {
    /// The exact bytes signed.
    pub fn preimage(&self) -> Vec<u8> {
        format!(
            "{DOMAIN}\n{}\n{}\n{}\n{}",
            self.org_id, self.cumulative_nano_usd, self.statement_ref, self.issued_at
        )
        .into_bytes()
    }
}

/// A statement with its signature and the key it was signed with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedCreditStatement {
    pub statement: CreditStatement,
    /// `sha256:` over the raw 32-byte public key.
    pub key_id: String,
    /// Base64 (standard, padded) Ed25519 signature over the preimage.
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StatementError {
    #[error("near_ai_credit_statement_key_invalid")]
    KeyInvalid,
    #[error("near_ai_credit_statement_signature_invalid")]
    SignatureInvalid,
}

/// The signing key. `Debug` prints only the key id.
#[derive(Clone)]
pub struct StatementSigner {
    pair: Arc<Ed25519KeyPair>,
}

impl std::fmt::Debug for StatementSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StatementSigner({})", self.key_id())
    }
}

impl StatementSigner {
    /// From a base64 (standard) PKCS#8 v2 Ed25519 document.
    pub fn from_base64_pkcs8(text: &str) -> Result<Self, StatementError> {
        let der = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .map_err(|_| StatementError::KeyInvalid)?;
        let pair = Ed25519KeyPair::from_pkcs8(&der).map_err(|_| StatementError::KeyInvalid)?;
        Ok(Self {
            pair: Arc::new(pair),
        })
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }

    pub fn key_id(&self) -> String {
        key_id_of(self.pair.public_key().as_ref())
    }

    pub fn sign(&self, statement: CreditStatement) -> SignedCreditStatement {
        let signature = self.pair.sign(&statement.preimage());
        SignedCreditStatement {
            statement,
            key_id: self.key_id(),
            signature: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
        }
    }
}

fn key_id_of(public_key: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(public_key)))
}

/// Verify a statement against a pinned public key, as NEAR AI would.
pub fn verify(signed: &SignedCreditStatement, public_key: &[u8]) -> Result<(), StatementError> {
    if signed.key_id != key_id_of(public_key) {
        return Err(StatementError::SignatureInvalid);
    }
    let signature = base64::engine::general_purpose::STANDARD
        .decode(&signed.signature)
        .map_err(|_| StatementError::SignatureInvalid)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&signed.statement.preimage(), &signature)
        .map_err(|_| StatementError::SignatureInvalid)
}

#[cfg(test)]
pub(crate) mod tests_support {
    use base64::Engine;

    pub(crate) fn fresh_pkcs8_base64() -> String {
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(doc.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement() -> CreditStatement {
        CreditStatement {
            org_id: "6f1c0b6a-0000-4000-8000-00000000abcd".into(),
            cumulative_nano_usd: "25000000000".into(),
            statement_ref: "sha256:ab".into(),
            issued_at: "2026-10-14T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_signed_statement_verifies_against_its_key_only() {
        let signer =
            StatementSigner::from_base64_pkcs8(&tests_support::fresh_pkcs8_base64()).unwrap();
        let signed = signer.sign(statement());
        assert_eq!(verify(&signed, &signer.public_key()), Ok(()));
        let other =
            StatementSigner::from_base64_pkcs8(&tests_support::fresh_pkcs8_base64()).unwrap();
        assert_eq!(
            verify(&signed, &other.public_key()),
            Err(StatementError::SignatureInvalid)
        );
    }

    #[test]
    fn any_edit_to_the_statement_breaks_the_signature() {
        let signer =
            StatementSigner::from_base64_pkcs8(&tests_support::fresh_pkcs8_base64()).unwrap();
        let signed = signer.sign(statement());
        for edit in [
            |s: &mut CreditStatement| s.cumulative_nano_usd = "25000000001".into(),
            |s: &mut CreditStatement| s.org_id = "00000000-0000-4000-8000-000000000000".into(),
            |s: &mut CreditStatement| s.statement_ref = "sha256:ac".into(),
            |s: &mut CreditStatement| s.issued_at = "2026-10-15T00:00:00Z".into(),
        ] {
            let mut tampered = signed.clone();
            edit(&mut tampered.statement);
            assert_eq!(
                verify(&tampered, &signer.public_key()),
                Err(StatementError::SignatureInvalid)
            );
        }
    }

    #[test]
    fn a_malformed_key_is_refused_and_the_signer_debug_shows_only_the_key_id() {
        assert_eq!(
            StatementSigner::from_base64_pkcs8("AAAA").unwrap_err(),
            StatementError::KeyInvalid
        );
        let key = tests_support::fresh_pkcs8_base64();
        let signer = StatementSigner::from_base64_pkcs8(&key).unwrap();
        let debug = format!("{signer:?}");
        assert!(debug.starts_with("StatementSigner(sha256:"));
        assert!(!debug.contains(&key));
    }

    #[test]
    fn the_preimage_is_line_separated_and_domain_tagged() {
        assert_eq!(
            String::from_utf8(statement().preimage()).unwrap(),
            "trace-commons:near-ai-credit-statement:v1\n6f1c0b6a-0000-4000-8000-00000000abcd\n25000000000\nsha256:ab\n2026-10-14T00:00:00Z"
        );
    }
}
