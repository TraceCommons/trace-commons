// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sealing a payout recipient, and opening it again for the pass
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5, "Storage").
//!
//! A send needs the raw recipient (for `near_ai`, the org UUID), so the
//! recipient is sealed, not hashed: the same envelope and key as
//! `trace_near_account_anchors.sealed_account_name`
//! ([`NearAccountIdentity`]), bound to a keyed blind index of its own
//! domain, with the key and pepper refs it was sealed under. Neither key is
//! in the database.
//!
//! Three keyed labels, each under its own domain so none can be joined to
//! another or to a login anchor:
//!
//! - the recipient index (`rail`, `recipient`), which binds the seal;
//! - the subject index of the NEAR AI subject that proved the recipient,
//!   which lets an operator see one subject linked several accounts without
//!   opening anything. It is deliberately not the login anchor's value, so
//!   a links row cannot be joined to a tenant's anchor;
//! - the request handle, recomputable from the request so a replayed start
//!   answers the handle it answered first.

use std::sync::Arc;

use trace_commons_gate_api::RecipientRef;

use crate::db::{PayoutLink, SealedRecipient};
use crate::near_account_identity::NearAccountIdentity;
use crate::payout_pass::PayoutRecipientOpener;

const RECIPIENT_INDEX_DOMAIN: &[u8] = b"trace_commons.payout_recipient_index.v1\n";
const SUBJECT_INDEX_DOMAIN: &[u8] = b"trace_commons.payout_subject_index.v1\n";
const LINK_HANDLE_DOMAIN: &[u8] = b"trace_commons.payout_link_handle.v1\n";

/// The keyed index a recipient's seal is bound to.
pub fn payout_recipient_index(
    identity: &NearAccountIdentity,
    rail: &str,
    recipient: &str,
) -> String {
    identity.keyed_label(
        RECIPIENT_INDEX_DOMAIN,
        &[rail.as_bytes(), recipient.as_bytes()],
    )
}

/// The keyed index of the subject that proved a recipient. Not the login
/// anchor ([`NearAccountIdentity::login_index_label`]).
pub fn payout_subject_index(identity: &NearAccountIdentity, subject_id: &str) -> String {
    identity.keyed_label(SUBJECT_INDEX_DOMAIN, &[subject_id.as_bytes()])
}

/// The handle a start call returns, as 64 lowercase hex: keyed over the
/// tenant, account, rail and idempotency key, so the same request always
/// gets the same handle and nobody can compute one without the pepper.
pub fn payout_link_handle(
    identity: &NearAccountIdentity,
    tenant: &str,
    account: uuid::Uuid,
    rail: &str,
    idempotency_key: uuid::Uuid,
) -> String {
    let label = identity.keyed_label(
        LINK_HANDLE_DOMAIN,
        &[
            tenant.as_bytes(),
            account.to_string().as_bytes(),
            rail.as_bytes(),
            idempotency_key.to_string().as_bytes(),
        ],
    );
    label.trim_start_matches("sha256:").to_owned()
}

/// Seals `recipient` for `rail`, recording `subject_id`'s keyed index and
/// `recipient_ref` beside it.
pub fn seal_payout_recipient(
    identity: &NearAccountIdentity,
    rail: &str,
    recipient: &str,
    subject_id: &str,
    recipient_ref: Option<String>,
) -> anyhow::Result<SealedRecipient> {
    let recipient_index = payout_recipient_index(identity, rail, recipient);
    let sealed = identity.seal_bound_to(&recipient_index, recipient)?;
    Ok(SealedRecipient {
        sealed: serde_json::to_value(sealed)?,
        recipient_index,
        recipient_key_ref: identity.key_ref_hash(),
        index_pepper_ref: identity.pepper_ref_hash().to_owned(),
        subject_index: payout_subject_index(identity, subject_id),
        recipient_ref,
    })
}

/// Opens a stored recipient. `None` for a seal under another key or pepper
/// (rotated away), one that does not decrypt, or one whose plaintext does
/// not reproduce the stored index (a substituted ciphertext).
pub fn open_payout_recipient(
    identity: &NearAccountIdentity,
    rail: &str,
    recipient: &SealedRecipient,
) -> Option<RecipientRef> {
    if recipient.index_pepper_ref != identity.pepper_ref_hash()
        || recipient.recipient_key_ref != identity.key_ref_hash()
    {
        return None;
    }
    let sealed = serde_json::from_value(recipient.sealed.clone()).ok()?;
    let opened = identity.open(&recipient.recipient_index, &sealed).ok()?;
    if payout_recipient_index(identity, rail, &opened) != recipient.recipient_index {
        return None;
    }
    RecipientRef::new(opened.as_str()).ok()
}

/// The opener a deployment with the recipient key uses.
pub struct SealedRecipientOpener {
    identity: Arc<NearAccountIdentity>,
}

impl SealedRecipientOpener {
    pub fn new(identity: Arc<NearAccountIdentity>) -> Self {
        Self { identity }
    }
}

impl std::fmt::Debug for SealedRecipientOpener {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SealedRecipientOpener(<sealed>)")
    }
}

impl PayoutRecipientOpener for SealedRecipientOpener {
    fn open(&self, link: &PayoutLink) -> Option<RecipientRef> {
        open_payout_recipient(&self.identity, link.rail_id.as_str(), &link.recipient)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PayoutLinkState;
    use crate::secrets::SecretsCrypto;
    use crate::trace_artifact_kek::{KmsKeyWrapper, LocalMasterKeyWrapper};
    use base64::Engine as _;
    use secrecy::SecretString;
    use trace_commons_gate_api::{RailId, RecipientKind};
    use uuid::Uuid;

    const ORG: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";
    const SUBJECT: &str = "synthetic-subject-1";

    fn identity(seed: u8, master: &str) -> NearAccountIdentity {
        let crypto = SecretsCrypto::new(SecretString::from(master.to_owned())).unwrap();
        let kek: Arc<dyn KmsKeyWrapper> =
            Arc::new(LocalMasterKeyWrapper::new(crypto, "payout-seal-fixture"));
        NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([seed; 32])),
            Some(kek),
        )
        .unwrap()
    }

    fn link(recipient: SealedRecipient) -> PayoutLink {
        PayoutLink {
            account_id: Uuid::new_v4(),
            rail_id: RailId::near_ai(),
            link_id: Uuid::new_v4(),
            recipient_kind: RecipientKind::near_ai_org(),
            state: PayoutLinkState::Active,
            recipient,
            program: "tc1-0".into(),
            final_paid: None,
            referral_program: None,
            final_paid_referral: None,
            unlink_requested_at: None,
            recipient_unavailable_at: None,
            version: 1,
        }
    }

    #[test]
    fn a_sealed_recipient_opens_only_under_its_own_keys() {
        let id = Arc::new(identity(7, &"a".repeat(32)));
        let sealed = seal_payout_recipient(&id, "near_ai", ORG, SUBJECT, None).unwrap();
        let stored = serde_json::to_string(&sealed.sealed).unwrap();
        assert!(!stored.contains(ORG), "the org is never stored in clear");
        assert!(!format!("{sealed:?}").contains(ORG));
        let opener = SealedRecipientOpener::new(id.clone());
        assert_eq!(opener.open(&link(sealed.clone())).unwrap().expose(), ORG);

        // Another pepper, or another key: nothing opens.
        let other_pepper = SealedRecipientOpener::new(Arc::new(identity(8, &"a".repeat(32))));
        assert!(other_pepper.open(&link(sealed.clone())).is_none());
        let other_key = SealedRecipientOpener::new(Arc::new(identity(7, &"b".repeat(32))));
        assert!(other_key.open(&link(sealed.clone())).is_none());

        // A ciphertext moved onto another row's index does not open.
        let other =
            seal_payout_recipient(&id, "near_ai", &Uuid::new_v4().to_string(), SUBJECT, None)
                .unwrap();
        let mut swapped = sealed.clone();
        swapped.sealed = other.sealed;
        assert!(opener.open(&link(swapped)).is_none());

        // The same recipient on another rail has another index.
        assert_ne!(
            payout_recipient_index(&id, "near_ai", ORG),
            payout_recipient_index(&id, "test_keyed", ORG)
        );
    }

    #[test]
    fn the_subject_index_is_not_the_login_anchor() {
        let id = identity(7, &"a".repeat(32));
        let subject = payout_subject_index(&id, SUBJECT);
        assert!(subject.starts_with("sha256:") && subject.len() == 71);
        assert_ne!(subject, id.login_index_label(SUBJECT));
        assert_eq!(subject, payout_subject_index(&id, SUBJECT));
        assert_ne!(
            subject,
            payout_subject_index(&identity(8, &"a".repeat(32)), SUBJECT)
        );
    }

    #[test]
    fn the_handle_is_stable_per_request_and_keyed() {
        let id = identity(7, &"a".repeat(32));
        let account = Uuid::new_v4();
        let key = Uuid::new_v4();
        let handle = payout_link_handle(&id, "tenant-x", account, "near_ai", key);
        assert_eq!(handle.len(), 64);
        assert!(
            handle
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_eq!(
            handle,
            payout_link_handle(&id, "tenant-x", account, "near_ai", key)
        );
        assert_ne!(
            handle,
            payout_link_handle(&id, "tenant-x", account, "near_ai", Uuid::new_v4())
        );
        assert_ne!(
            handle,
            payout_link_handle(&id, "tenant-y", account, "near_ai", key)
        );
        assert_ne!(
            handle,
            payout_link_handle(
                &identity(8, &"a".repeat(32)),
                "tenant-x",
                account,
                "near_ai",
                key
            )
        );
    }
}
