// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Making, sealing and finding referral codes
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, section 2).
//!
//! A code is ten Crockford symbols drawn from the operating system's random
//! source (50 bits; `trace_commons_protocol::referral`). It is stored two
//! ways, as payout recipients are (`payout_recipient_seal`):
//!
//! - `code_index`, a keyed blind index under the NEAR account index pepper,
//!   in its own domain (`tc-referral-code-v1:`), unique across every tenant,
//!   which is how an applied code finds its owner;
//! - `sealed_code`, the code sealed under the account-name key and bound to
//!   that index, so its owner can list it again. Neither key is in the
//!   database.
//!
//! Like every keyed label in this crate, the index frames its one field with
//! its length after the domain.
//!
//! The referee hash names a referee in the referrer's tenant. It is unkeyed
//! on purpose: its inputs are a random tenant label and a random account id,
//! and it must be recomputable by the bonus pass without the pepper.

use sha2::{Digest, Sha256};
use trace_commons_protocol::referral::{REFERRAL_CODE_BITS, referral_code_from_bits};
use uuid::Uuid;

use crate::db::ReferralCodeWrite;
use crate::near_account_identity::NearAccountIdentity;

const CODE_INDEX_DOMAIN: &[u8] = b"tc-referral-code-v1:";

/// A fresh canonical code from the operating system's random source.
pub fn generate_referral_code() -> String {
    let mut bytes = [0u8; 8];
    rand::RngCore::fill_bytes(&mut aes_gcm::aead::OsRng, &mut bytes);
    referral_code_from_bits(u64::from_le_bytes(bytes) & ((1u64 << REFERRAL_CODE_BITS) - 1))
}

/// The keyed index of a canonical code, as `trace_referral_codes.code_index`.
pub fn referral_code_index(identity: &NearAccountIdentity, normalized: &str) -> String {
    identity.keyed_label(CODE_INDEX_DOMAIN, &[normalized.as_bytes()])
}

/// Seals a canonical code for a new row with id `code_id`.
pub fn seal_referral_code(
    identity: &NearAccountIdentity,
    code_id: Uuid,
    normalized: &str,
) -> anyhow::Result<ReferralCodeWrite> {
    let code_index = referral_code_index(identity, normalized);
    let sealed = identity.seal_bound_to(&code_index, normalized)?;
    Ok(ReferralCodeWrite {
        code_id,
        sealed_code: serde_json::to_value(sealed)?,
        code_index,
        code_key_ref: identity.key_ref_hash(),
        index_pepper_ref: identity.pepper_ref_hash().to_owned(),
    })
}

/// Opens a stored code. `None` for a seal under another key or pepper
/// (rotated away), one that does not decrypt, or one whose plaintext does
/// not reproduce the stored index (a substituted ciphertext).
pub fn open_referral_code(
    identity: &NearAccountIdentity,
    code_index: &str,
    sealed_code: &serde_json::Value,
    code_key_ref: &str,
    index_pepper_ref: &str,
) -> Option<String> {
    if index_pepper_ref != identity.pepper_ref_hash() || code_key_ref != identity.key_ref_hash() {
        return None;
    }
    let sealed = serde_json::from_value(sealed_code.clone()).ok()?;
    let opened = identity.open(code_index, &sealed).ok()?;
    if referral_code_index(identity, &opened) != code_index {
        return None;
    }
    Some(opened.as_str().to_owned())
}

/// `sha256("tc-referral-referee-v1:" || tenant_id || ":" || account_id)`,
/// the only name a referrer's rows hold for a referee (spec section 3).
pub fn referee_ref_hash(tenant_id: &str, account_id: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("tc-referral-referee-v1:{tenant_id}:{account_id}").as_bytes())
    )
}

/// The audit rows' hash of the account acting in its own tenant. Never the
/// account id.
pub fn referral_actor_hash(account_id: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("referral-actor-v1:{account_id}").as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::secrets::SecretsCrypto;
    use crate::trace_artifact_kek::{KmsKeyWrapper, LocalMasterKeyWrapper};
    use base64::Engine as _;
    use secrecy::SecretString;
    use trace_commons_protocol::referral::{
        CROCKFORD_ALPHABET, REFERRAL_CODE_SYMBOLS, normalize_referral_code,
    };

    fn identity(seed: u8, master: &str) -> NearAccountIdentity {
        let crypto = SecretsCrypto::new(SecretString::from(master.to_owned())).unwrap();
        let kek: Arc<dyn KmsKeyWrapper> =
            Arc::new(LocalMasterKeyWrapper::new(crypto, "referral-code-fixture"));
        NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([seed; 32])),
            Some(kek),
        )
        .unwrap()
    }

    #[test]
    fn generated_codes_are_canonical_and_differ() {
        let codes: std::collections::BTreeSet<String> =
            (0..64).map(|_| generate_referral_code()).collect();
        assert!(codes.len() > 60, "50 random bits do not repeat in 64 draws");
        for code in &codes {
            assert_eq!(code.len(), REFERRAL_CODE_SYMBOLS);
            assert!(code.bytes().all(|b| CROCKFORD_ALPHABET.contains(&b)));
            assert_eq!(
                normalize_referral_code(code).as_deref(),
                Some(code.as_str())
            );
        }
    }

    #[test]
    fn a_sealed_code_opens_only_under_its_own_keys_and_index() {
        let id = identity(7, &"a".repeat(32));
        let code = "7K3M9QX2RD";
        let write = seal_referral_code(&id, Uuid::new_v4(), code).unwrap();
        assert!(write.code_index.starts_with("sha256:") && write.code_index.len() == 71);
        let stored = serde_json::to_string(&write.sealed_code).unwrap();
        assert!(!stored.contains(code), "the code is never stored in clear");
        assert!(!format!("{write:?}").contains(code));
        let open = |identity: &NearAccountIdentity, write: &ReferralCodeWrite| {
            open_referral_code(
                identity,
                &write.code_index,
                &write.sealed_code,
                &write.code_key_ref,
                &write.index_pepper_ref,
            )
        };
        assert_eq!(open(&id, &write).as_deref(), Some(code));
        assert_eq!(open(&identity(8, &"a".repeat(32)), &write), None);
        assert_eq!(open(&identity(7, &"b".repeat(32)), &write), None);

        let other = seal_referral_code(&id, Uuid::new_v4(), "ABCDEFGH12").unwrap();
        let mut swapped = write.clone();
        swapped.sealed_code = other.sealed_code;
        assert_eq!(
            open(&id, &swapped),
            None,
            "a moved ciphertext does not open"
        );
    }

    #[test]
    fn the_index_is_keyed_and_in_its_own_domain() {
        let id = identity(7, &"a".repeat(32));
        let code = "7K3M9QX2RD";
        assert_eq!(
            referral_code_index(&id, code),
            referral_code_index(&id, code)
        );
        assert_ne!(
            referral_code_index(&id, code),
            referral_code_index(&identity(8, &"a".repeat(32)), code)
        );
        assert_ne!(referral_code_index(&id, code), id.login_index_label(code));
        assert_ne!(
            referral_code_index(&id, code),
            crate::payout_recipient_seal::payout_subject_index(&id, code)
        );
        assert_ne!(
            referral_code_index(&id, code),
            format!("sha256:{:x}", Sha256::digest(code.as_bytes())),
            "not computable without the pepper"
        );
    }

    #[test]
    fn the_referee_hash_matches_the_spec_preimage() {
        let account = Uuid::parse_str("6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f").unwrap();
        let expected = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                b"tc-referral-referee-v1:nearai-x:6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f"
            ))
        );
        assert_eq!(referee_ref_hash("nearai-x", account), expected);
        assert_ne!(referee_ref_hash("nearai-y", account), expected);
        assert_ne!(referral_actor_hash(account), expected);
        assert!(!referral_actor_hash(account).contains(&account.to_string()));
    }
}
