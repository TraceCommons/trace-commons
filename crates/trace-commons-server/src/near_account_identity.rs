// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Keyed identity material for NEAR wallet provisioning: a peppered blind index
//! that replaces the anchor's identity role, and a KMS-sealed copy of the NEAR
//! account name so the index can be recomputed when the pepper rotates.
//!
//! # Why this module exists
//!
//! `trace_near_account_anchors.anchor_hash` used to be
//! `sha256(domain || network || account_name)`. Every input is public and the
//! domain prefix is open source, so the digest took no secret; and
//! `V58__near_account_provisioning.sql` bound `tenant_id = 'near-' ||
//! substring(anchor_hash from 8)`. Anyone holding an anchor -- and one leaves
//! the server today, inside the admission binding handed to a witness -- could
//! test candidate NEAR account names offline and learn which contributor it
//! belongs to.
//!
//! The fix is not to pepper that derivation in place. `anchor_hash` is half a
//! primary key and the target of a foreign key, so a peppered primary key could
//! never be rotated. Instead:
//!
//! - the tenant id becomes random and is derived from nothing (see
//!   [`random_near_tenant_id`]), and
//! - the anchor becomes a **blind index**, `HMAC-SHA256(pepper, network ||
//!   account_name)`, whose only job is to find an existing anchor when a
//!   contributor logs in again from a new device.
//!
//! Rotating a lookup column is an ordinary data migration, but only if the
//! account name can be read back. Storing it in plaintext would defeat the
//! pepper outright: the threat the pepper answers is a database disclosure
//! *without* application secrets -- a leaked backup, a stolen replica, an
//! injection read -- and in exactly that scenario a plaintext column hands over
//! every wallet identity while the pepper sits elsewhere guarding nothing. So
//! the name is sealed under a KMS-held key, reusing the KEK/DEK envelope the
//! trace artifact store already uses ([`KmsKeyWrapper`], [`WrappedDek`],
//! `aead_encrypt_with_dek`). A database disclosure alone then yields
//! ciphertext and a keyed index, and neither key is in the database.
//!
//! # Fail-closed
//!
//! There is deliberately no way to compute an index without a pepper and no way
//! to store an account name without a key. Both are constructor arguments, not
//! options with defaults: [`NearAccountIndexPepper`] cannot be built from
//! absent configuration, and [`NearAccountIdentity`] cannot be built without a
//! key wrapper. A caller that lacks either gets a
//! [`MissingNearIdentityControl`] carrying a stable, non-identifying label, and
//! the provisioning path refuses. An unsalted or unsealed fallback would
//! silently reinstate the defect, which is worse than refusing.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::hmac;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::trace_artifact_kek::{KekContext, KmsKeyWrapper, WrappedDek};
use crate::trace_artifact_store::{
    TraceArtifactKind, aead_decrypt_with_dek, aead_encrypt_with_dek, generate_dek,
};

/// Environment variable holding the base64-encoded blind-index pepper.
pub const TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER: &str = "TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER";

/// Minimum accepted pepper length. HMAC accepts any key length, so nothing but
/// this check stops an operator shipping a four-byte "pepper" that a database
/// reader could brute-force alongside the account-name dictionary.
pub const NEAR_ACCOUNT_INDEX_PEPPER_MIN_BYTES: usize = 32;

/// Schema tag written into every sealed name, so a future envelope change can be
/// distinguished on read rather than guessed at.
pub const SEALED_NEAR_ACCOUNT_NAME_SCHEMA_V1: &str = "trace_commons.sealed_near_account_name.v1";

/// Domain separation for the blind index. Distinct from the retired
/// `trace_commons.near_account_anchor.v1` prefix so a value computed under the
/// old, unsalted scheme can never be mistaken for one computed under this one.
const BLIND_INDEX_DOMAIN: &[u8] = b"trace_commons.near_account_blind_index.v1\n";

/// Domain separation for the **login** blind index (#836), which anchors a
/// NEAR AI account id rather than a wallet account name.
///
/// A separate domain under the same pepper, so the two identity systems occupy
/// disjoint keyspaces. Sharing a domain would mean a wallet account name and a
/// NEAR AI subject id that happened to be equal as strings produced the same
/// anchor -- one contributor's abuse budget silently spent by another, and two
/// identity systems collapsed into one keyspace. Nothing about the pepper
/// prevents that; only the domain does.
const LOGIN_BLIND_INDEX_DOMAIN: &[u8] = b"trace_commons.near_ai_login_blind_index.v1\n";

/// Domain separation for the pepper's own identifier, which is published in
/// logs and stored beside each row so rotation can select rows still indexed
/// under a superseded pepper. It is an HMAC under the pepper rather than a hash
/// *of* the pepper, so the stored value is not a verifier for a guessed secret.
const PEPPER_REF_DOMAIN: &[u8] = b"trace_commons.near_account_index_pepper_ref.v1\n";

/// A control that must be configured before wallet provisioning may run. The
/// `Display` form is the stable label; it names the missing control and nothing
/// else -- no key reference, no account, no tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MissingNearIdentityControl {
    #[error("near_account_index_pepper_unconfigured")]
    IndexPepperUnconfigured,
    #[error("near_account_index_pepper_invalid")]
    IndexPepperInvalid,
    #[error("near_account_name_key_unconfigured")]
    AccountNameKeyUnconfigured,
}

impl MissingNearIdentityControl {
    /// Stable label safe to place in an operator-facing response or a log line.
    pub fn label(&self) -> &'static str {
        match self {
            Self::IndexPepperUnconfigured => "near_account_index_pepper_unconfigured",
            Self::IndexPepperInvalid => "near_account_index_pepper_invalid",
            Self::AccountNameKeyUnconfigured => "near_account_name_key_unconfigured",
        }
    }
}

/// The blind-index secret. Holds a prepared HMAC key; the raw bytes are dropped
/// once the key is built, and the type has no accessor that returns them.
pub struct NearAccountIndexPepper {
    key: hmac::Key,
    ref_hash: String,
}

impl NearAccountIndexPepper {
    /// Build from raw key bytes. Rejects anything shorter than
    /// [`NEAR_ACCOUNT_INDEX_PEPPER_MIN_BYTES`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MissingNearIdentityControl> {
        if bytes.len() < NEAR_ACCOUNT_INDEX_PEPPER_MIN_BYTES {
            return Err(MissingNearIdentityControl::IndexPepperInvalid);
        }
        let key = hmac::Key::new(hmac::HMAC_SHA256, bytes);
        let ref_hash = format!(
            "sha256:{}",
            hex::encode(hmac::sign(&key, PEPPER_REF_DOMAIN).as_ref())
        );
        Ok(Self { key, ref_hash })
    }

    /// Build from the operator-supplied base64 value, absent or present. An
    /// absent, empty, or malformed value is a refusal, never a default.
    pub fn from_env_value(raw: Option<&str>) -> Result<Self, MissingNearIdentityControl> {
        let raw = raw
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or(MissingNearIdentityControl::IndexPepperUnconfigured)?;
        let bytes = Zeroizing::new(
            STANDARD
                .decode(raw)
                .map_err(|_| MissingNearIdentityControl::IndexPepperInvalid)?,
        );
        Self::from_bytes(&bytes)
    }

    /// Stable, non-invertible identifier for this pepper. Safe to log and to
    /// store beside a row so rotation can find rows indexed under an old one.
    pub fn ref_hash(&self) -> &str {
        &self.ref_hash
    }

    /// `HMAC-SHA256(pepper, framed(network, account_name))`.
    ///
    /// Fields are length-prefixed before hashing so that `("mainnet", "a.b")`
    /// and `("mainnet.a", "b")` cannot collide. This mirrors the framing the
    /// retired anchor used; what is added is the key.
    fn index(&self, network: &str, account_name: &str) -> [u8; 32] {
        let mut ctx = hmac::Context::with_key(&self.key);
        ctx.update(BLIND_INDEX_DOMAIN);
        for field in [network.as_bytes(), account_name.as_bytes()] {
            ctx.update(&(field.len() as u64).to_le_bytes());
            ctx.update(field);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(ctx.sign().as_ref());
        out
    }

    /// `HMAC-SHA256(pepper, framed(subject_id))` under the login domain.
    ///
    /// One field rather than two: a NEAR AI account id is already global, with
    /// no network to qualify it. It is still length-prefixed, so the framing
    /// stays unambiguous if a second field is ever added.
    fn login_index(&self, subject_id: &str) -> [u8; 32] {
        let mut ctx = hmac::Context::with_key(&self.key);
        ctx.update(LOGIN_BLIND_INDEX_DOMAIN);
        let field = subject_id.as_bytes();
        ctx.update(&(field.len() as u64).to_le_bytes());
        ctx.update(field);
        let mut out = [0u8; 32];
        out.copy_from_slice(ctx.sign().as_ref());
        out
    }

    /// The login anchor as stored in `trace_near_account_anchors.anchor_hash`.
    ///
    /// Same `sha256:` prefix and same column as the wallet anchor: the storage
    /// and wire shape are shared on purpose, so every consumer downstream of
    /// the anchor keeps working unchanged. What is not shared is the preimage
    /// domain, which is what keeps the two unconfusable.
    pub fn login_index_label(&self, subject_id: &str) -> String {
        format!("sha256:{}", hex::encode(self.login_index(subject_id)))
    }

    /// The blind index as stored in `trace_near_account_anchors.anchor_hash`.
    ///
    /// The `sha256:` prefix is retained deliberately. The value is an
    /// HMAC-SHA256 tag, not a bare digest, but the prefix is load-bearing wire
    /// and schema format: `V58` and `V59` constrain it by regex, the contributor
    /// daemon validates it, and the admission binding carries it. Changing the
    /// prefix would be a protocol break that buys no security. What changed is
    /// the preimage, which now requires a secret -- do not read this prefix as
    /// licence to recompute an anchor from public inputs.
    pub fn index_label(&self, network: &str, account_name: &str) -> String {
        format!("sha256:{}", hex::encode(self.index(network, account_name)))
    }
}

/// Opaque, matching [`SealedNearAccountName`] below and for the same reason.
/// The prepared HMAC key is not printable and the pepper's own reference is
/// reachable through [`NearAccountIndexPepper::ref_hash`] when an operator
/// surface genuinely wants it; a derived `Debug` would carry it into every log
/// line that happens to format a containing struct.
impl std::fmt::Debug for NearAccountIndexPepper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NearAccountIndexPepper(<sealed>)")
    }
}

/// Opaque for the same reason as the pepper it holds.
impl std::fmt::Debug for NearAccountIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NearAccountIdentity(<sealed>)")
    }
}

/// An account name sealed under a KMS-wrapped per-row DEK. Reuses the trace
/// artifact envelope: the DEK is wrapped by the configured [`KmsKeyWrapper`]
/// and the plaintext is AES-256-GCM encrypted under it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealedNearAccountName {
    pub schema_version: String,
    pub wrapped_dek: WrappedDek,
    pub ciphertext_base64: String,
}

/// Deliberately opaque. The ciphertext is not secret, but a `Debug` that prints
/// it invites it into a log line beside the row's other columns, and the
/// hash-only logging rule is easier to keep if the type simply cannot do that.
impl std::fmt::Debug for SealedNearAccountName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealedNearAccountName(<sealed>)")
    }
}

/// The pepper and the account-name key, held together because every caller
/// needs both: an index without a recoverable name cannot be rotated, and a
/// sealed name without an index cannot be found.
pub struct NearAccountIdentity {
    pepper: NearAccountIndexPepper,
    kek: Arc<dyn KmsKeyWrapper>,
}

impl NearAccountIdentity {
    pub fn new(pepper: NearAccountIndexPepper, kek: Arc<dyn KmsKeyWrapper>) -> Self {
        Self { pepper, kek }
    }

    /// Assemble from operator configuration. Both arguments are `Option` so the
    /// caller can hand over whatever it found; this function is where absence
    /// becomes a named refusal rather than a silent downgrade.
    pub fn from_parts(
        pepper_raw: Option<&str>,
        kek: Option<Arc<dyn KmsKeyWrapper>>,
    ) -> Result<Self, MissingNearIdentityControl> {
        let pepper = NearAccountIndexPepper::from_env_value(pepper_raw)?;
        let kek = kek.ok_or(MissingNearIdentityControl::AccountNameKeyUnconfigured)?;
        Ok(Self::new(pepper, kek))
    }

    pub fn pepper(&self) -> &NearAccountIndexPepper {
        &self.pepper
    }

    pub fn pepper_ref_hash(&self) -> &str {
        self.pepper.ref_hash()
    }

    /// Hash of the KMS key reference in use. Safe to log and to store.
    pub fn key_ref_hash(&self) -> String {
        self.kek.safe_status().key_ref_hash
    }

    pub fn index_label(&self, network: &str, account_name: &str) -> String {
        self.pepper.index_label(network, account_name)
    }

    /// The login anchor for a NEAR AI subject id (#836). Domain-separated from
    /// [`Self::index_label`]; see [`NearAccountIndexPepper::login_index_label`].
    pub fn login_index_label(&self, subject_id: &str) -> String {
        self.pepper.login_index_label(subject_id)
    }

    /// Bind a sealed name to the row it belongs to.
    ///
    /// `KekContext` was built for artifacts, and its `tenant_storage_ref` field
    /// carries the blind index here rather than a storage reference. There is no
    /// tenant at seal time -- decoupling the tenant from the account is the
    /// entire point -- and the index is the strongest binding available: a
    /// wrapped DEK lifted from one anchor row cannot be replayed against
    /// another, because unwrapping checks this hash.
    fn context(index_label: &str) -> KekContext {
        KekContext {
            tenant_storage_ref: index_label.to_string(),
            artifact_kind: TraceArtifactKind::NearAccountName,
        }
    }

    /// Seal `account_name`, returning it alongside the blind index it is bound
    /// to. Both are written to the same row.
    pub fn seal(
        &self,
        network: &str,
        account_name: &str,
    ) -> anyhow::Result<(String, SealedNearAccountName)> {
        let index_label = self.index_label(network, account_name);
        let dek = generate_dek();
        let wrapped_dek = self.kek.wrap_dek(&dek, &Self::context(&index_label))?;
        let ciphertext = aead_encrypt_with_dek(&dek, account_name.as_bytes())?;
        Ok((
            index_label,
            SealedNearAccountName {
                schema_version: SEALED_NEAR_ACCOUNT_NAME_SCHEMA_V1.into(),
                wrapped_dek,
                ciphertext_base64: STANDARD.encode(ciphertext),
            },
        ))
    }

    /// Seal a NEAR AI subject id (#836), bound to its **login** blind index.
    ///
    /// The same envelope and the same row shape as [`Self::seal`], so rotation
    /// reads a login row and a wallet row identically. What differs is the
    /// index it is bound to: the KEK context is the login anchor, so a sealed
    /// subject cannot be unwrapped against a wallet row's anchor even under the
    /// same key. The two identity systems stay separated inside the envelope as
    /// well as in the keyspace.
    pub fn seal_login_subject(&self, subject_id: &str) -> anyhow::Result<SealedNearAccountName> {
        let index_label = self.login_index_label(subject_id);
        let dek = generate_dek();
        let wrapped_dek = self.kek.wrap_dek(&dek, &Self::context(&index_label))?;
        let ciphertext = aead_encrypt_with_dek(&dek, subject_id.as_bytes())?;
        Ok(SealedNearAccountName {
            schema_version: SEALED_NEAR_ACCOUNT_NAME_SCHEMA_V1.into(),
            wrapped_dek,
            ciphertext_base64: STANDARD.encode(ciphertext),
        })
    }

    /// Recover a sealed account name. `index_label` is the row's stored anchor,
    /// which the unwrap verifies against the context recorded at seal time.
    pub fn open(
        &self,
        index_label: &str,
        sealed: &SealedNearAccountName,
    ) -> anyhow::Result<Zeroizing<String>> {
        anyhow::ensure!(
            sealed.schema_version == SEALED_NEAR_ACCOUNT_NAME_SCHEMA_V1,
            "SealedNearAccountNameSchemaRejected: unknown schema_version"
        );
        let dek = self
            .kek
            .unwrap_dek(&sealed.wrapped_dek, &Self::context(index_label))?;
        let ciphertext = STANDARD
            .decode(&sealed.ciphertext_base64)
            .map_err(|_| anyhow::anyhow!("SealedNearAccountNameOpenFailed: base64 decode error"))?;
        let plaintext = aead_decrypt_with_dek(&dek, &ciphertext)?;
        let name = std::str::from_utf8(plaintext.as_slice())
            .map_err(|_| anyhow::anyhow!("SealedNearAccountNameOpenFailed: not utf-8"))?;
        Ok(Zeroizing::new(name.to_string()))
    }
}

/// One row of `trace_near_account_anchors`, in the shape rotation reads and
/// writes it back.
///
/// The tenant id is carried here so that the rotation function is total: it
/// takes a row and returns a row, and the property that matters -- that the
/// tenant is untouched -- is a property of a pure function that a test can
/// assert, rather than something the rotation worker's SQL happens not to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearAnchorRow {
    pub tenant_id: String,
    pub index_label: String,
    pub sealed_account_name: SealedNearAccountName,
    pub index_pepper_ref: String,
    pub account_name_key_ref: String,
}

/// Re-key one anchor row: open the name under `current`, recompute the blind
/// index under `next`, and re-seal under `next`'s key.
///
/// The recovered name is checked back against the row's stored index under the
/// *current* pepper before anything is recomputed. Without that check a
/// substituted ciphertext -- one an attacker with write access to the row
/// placed there -- would be silently promoted into the new index, moving an
/// existing tenant under an account name of the attacker's choosing.
///
/// `tenant_id` is copied across unchanged and is an input to nothing. That is
/// the whole reason the tenant was decoupled from the anchor: a rotation that
/// could move a tenant would be a rotation nobody could run, because
/// `tenant_id` is a foreign key across the identity graph.
pub fn rotate_near_anchor_row(
    current: &NearAccountIdentity,
    next: &NearAccountIdentity,
    network: &str,
    row: &NearAnchorRow,
) -> anyhow::Result<NearAnchorRow> {
    anyhow::ensure!(
        row.index_pepper_ref == current.pepper_ref_hash(),
        "NearAccountRotationRejected: row is not indexed under the current pepper"
    );
    let name = current.open(&row.index_label, &row.sealed_account_name)?;
    anyhow::ensure!(
        current.index_label(network, &name) == row.index_label,
        "NearAccountRotationRejected: sealed name does not reproduce the stored index"
    );
    let (index_label, sealed_account_name) = next.seal(network, &name)?;
    Ok(NearAnchorRow {
        tenant_id: row.tenant_id.clone(),
        index_label,
        sealed_account_name,
        index_pepper_ref: next.pepper_ref_hash().to_string(),
        account_name_key_ref: next.key_ref_hash(),
    })
}

/// A wallet tenant id, drawn from the OS RNG and derived from nothing.
///
/// The shape (`near-` plus 64 lowercase hex) matches what the retired
/// `CHECK (tenant_id = 'near-' || substring(anchor_hash from 8))` produced, so
/// every consumer that recognises the wallet namespace by prefix keeps working.
/// What is gone is the second half of that constraint: the suffix is now 32
/// random bytes and no longer a function of the account name.
pub fn random_near_tenant_id() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut aes_gcm::aead::OsRng, &mut bytes);
    format!("near-{}", hex::encode(bytes))
}

/// The tenant namespace for a NEAR AI login (#836).
///
/// A distinct prefix from the wallet's `near-`, so the two identity systems
/// are distinguishable at a glance and by `anchor()`'s namespace test. Random
/// for the same reason V61 made the wallet one random: a tenant id derived
/// from anything public is computable offline by anyone holding that public
/// value (#716).
pub fn random_near_ai_tenant_id() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut aes_gcm::aead::OsRng, &mut bytes);
    format!("nearai-{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretsCrypto;
    use crate::trace_artifact_kek::LocalMasterKeyWrapper;
    use secrecy::SecretString;
    use sha2::Digest;

    /// Distinct master keys produce distinct wrappers, so a rotation test can
    /// prove the account name really was re-sealed under the second one.
    fn wrapper(master: &str) -> Arc<dyn KmsKeyWrapper> {
        let crypto =
            SecretsCrypto::new(SecretString::new(master.into())).expect("fixture SecretsCrypto");
        Arc::new(LocalMasterKeyWrapper::new(
            crypto,
            "near-account-identity-fixture",
        ))
    }

    fn pepper_b64(seed: u8) -> String {
        STANDARD.encode([seed; 32])
    }

    fn identity(seed: u8, master: &str) -> NearAccountIdentity {
        NearAccountIdentity::from_parts(Some(&pepper_b64(seed)), Some(wrapper(master)))
            .expect("fixture identity")
    }

    /// The defect this module exists to remove: `tenant_id` used to be
    /// `'near-' || substring(anchor_hash from 8)` over an unkeyed digest of the
    /// network and the account name, so anyone holding a NEAR account name
    /// could compute that contributor's tenant id offline.
    ///
    /// There is no function here from public inputs to a tenant id, so what a
    /// test can assert is the observable consequence: the same account,
    /// provisioned twice, does not reproduce the same tenant id, and no tenant
    /// id contains the account's blind index -- which is itself the closest
    /// thing to a public input that reaches the row.
    #[test]
    fn tenant_id_is_independent_of_every_public_input() {
        let id = identity(1, "a".repeat(32).as_str());
        let index = id.index_label("mainnet", "alice.near");

        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let tenant = random_near_tenant_id();
            assert!(
                tenant.starts_with("near-") && tenant.len() == 69,
                "tenant id must keep the wallet namespace shape consumers match on: {tenant}"
            );
            assert!(
                !tenant.contains(index.trim_start_matches("sha256:")),
                "tenant id reproduced the blind index, so it is a function of the account again"
            );
            assert!(
                seen.insert(tenant),
                "a tenant id repeated across 64 draws, so it is not being drawn at random"
            );
        }
    }

    /// The blind index is keyed. Two operators with different peppers index the
    /// same account to different values, which is what makes an offline
    /// dictionary attack over account names useless without the pepper.
    #[test]
    fn the_blind_index_depends_on_the_pepper_and_on_framed_inputs() {
        let a = identity(1, "a".repeat(32).as_str());
        let b = identity(2, "a".repeat(32).as_str());

        assert_ne!(
            a.index_label("mainnet", "alice.near"),
            b.index_label("mainnet", "alice.near"),
            "the index did not change with the pepper, so it takes no secret"
        );
        assert_eq!(
            a.index_label("mainnet", "alice.near"),
            a.index_label("mainnet", "alice.near"),
            "the index must be stable, or a returning contributor is never found"
        );
        assert_ne!(
            a.index_label("mainnet", "alice.near"),
            a.index_label("testnet", "alice.near")
        );
        assert_ne!(
            a.index_label("mainnet", "alice.near"),
            a.index_label("mainnet", "bob.near")
        );
        // Length framing: ("mainnet", "a.near") and ("mainnet.a", ".near")
        // must not collide into one identity.
        assert_ne!(
            a.index_label("mainnet", "a.near"),
            a.index_label("mainneta", ".near")
        );
        assert!(a.index_label("mainnet", "alice.near").len() == 71);
    }

    #[test]
    fn a_sealed_account_name_opens_only_under_its_own_index_and_key() {
        let id = identity(1, "a".repeat(32).as_str());
        let (index, sealed) = id.seal("mainnet", "alice.near").expect("seal");

        assert_eq!(&*id.open(&index, &sealed).expect("open"), "alice.near");
        assert!(
            !sealed.ciphertext_base64.contains("alice"),
            "the account name survived into the stored record in the clear"
        );
        // Lifting a wrapped DEK onto another row must not open it: the KekContext
        // binds the seal to the index it is stored beside.
        let other = id.index_label("mainnet", "bob.near");
        assert!(id.open(&other, &sealed).is_err());
        // A different account-name key cannot open it either.
        let rekeyed = identity(1, "b".repeat(32).as_str());
        assert!(rekeyed.open(&index, &sealed).is_err());
    }

    /// Rotation's whole purpose is to move the index without moving identity.
    /// If a rotation could change a tenant id it would be unrunnable, because
    /// `tenant_id` is a foreign key across the identity graph.
    #[test]
    fn rotation_moves_the_index_and_leaves_the_tenant_byte_identical() {
        let current = identity(1, "a".repeat(32).as_str());
        let next = identity(2, "b".repeat(32).as_str());

        let (index_label, sealed_account_name) = current.seal("mainnet", "alice.near").unwrap();
        let tenant_id = random_near_tenant_id();
        let row = NearAnchorRow {
            tenant_id: tenant_id.clone(),
            index_label,
            sealed_account_name,
            index_pepper_ref: current.pepper_ref_hash().to_string(),
            account_name_key_ref: current.key_ref_hash(),
        };

        let rotated = rotate_near_anchor_row(&current, &next, "mainnet", &row).expect("rotate");

        assert_eq!(
            rotated.tenant_id.as_bytes(),
            tenant_id.as_bytes(),
            "rotation moved the tenant id, which would break every foreign key referencing it"
        );
        assert_ne!(
            rotated.index_label, row.index_label,
            "the index did not change, so the pepper was not actually rotated"
        );
        assert_eq!(
            rotated.index_label,
            next.index_label("mainnet", "alice.near"),
            "the rotated index is not the one the new pepper would compute for this account, \
             so a returning contributor would no longer be found"
        );
        assert_eq!(
            &*next
                .open(&rotated.index_label, &rotated.sealed_account_name)
                .expect("open after rotation"),
            "alice.near",
            "the name is no longer recoverable, so this row can never be rotated again"
        );
        assert_eq!(rotated.index_pepper_ref, next.pepper_ref_hash());
        assert_eq!(rotated.account_name_key_ref, next.key_ref_hash());
    }

    /// A substituted ciphertext must not be promoted into the new index.
    /// Without the read-back check an attacker with write access to one row
    /// could move an existing tenant under an account name of their choosing --
    /// rotation would sign off on the substitution.
    #[test]
    fn rotation_refuses_a_sealed_name_that_does_not_reproduce_the_stored_index() {
        let current = identity(1, "a".repeat(32).as_str());
        let next = identity(2, "b".repeat(32).as_str());

        // A seal legitimately made for bob, filed under alice's index. The
        // KekContext binding is satisfied because the attacker sealed it against
        // alice's index; only the read-back check catches this.
        let alice_index = current.index_label("mainnet", "alice.near");
        let dek = generate_dek();
        let wrapped_dek = wrapper("a".repeat(32).as_str())
            .wrap_dek(
                &dek,
                &KekContext {
                    tenant_storage_ref: alice_index.clone(),
                    artifact_kind: TraceArtifactKind::NearAccountName,
                },
            )
            .unwrap();
        let substituted = SealedNearAccountName {
            schema_version: SEALED_NEAR_ACCOUNT_NAME_SCHEMA_V1.into(),
            wrapped_dek,
            ciphertext_base64: STANDARD.encode(aead_encrypt_with_dek(&dek, b"bob.near").unwrap()),
        };

        let row = NearAnchorRow {
            tenant_id: random_near_tenant_id(),
            index_label: alice_index,
            sealed_account_name: substituted,
            index_pepper_ref: current.pepper_ref_hash().to_string(),
            account_name_key_ref: current.key_ref_hash(),
        };
        let error = rotate_near_anchor_row(&current, &next, "mainnet", &row)
            .expect_err("rotation accepted a sealed name that does not match the stored index");
        assert!(error.to_string().contains("NearAccountRotationRejected"));
    }

    /// Fail-closed on the pepper. Absent, empty, malformed, and short are all
    /// refusals with a named control -- never a default, and never an unkeyed
    /// index, which would silently restore the offline-enumeration defect.
    #[test]
    fn an_absent_or_weak_pepper_refuses_with_a_named_control() {
        assert_eq!(
            NearAccountIndexPepper::from_env_value(None).unwrap_err(),
            MissingNearIdentityControl::IndexPepperUnconfigured
        );
        assert_eq!(
            NearAccountIndexPepper::from_env_value(Some("   ")).unwrap_err(),
            MissingNearIdentityControl::IndexPepperUnconfigured
        );
        assert_eq!(
            NearAccountIndexPepper::from_env_value(Some("not base64!!")).unwrap_err(),
            MissingNearIdentityControl::IndexPepperInvalid
        );
        // 31 bytes: HMAC would accept it, so nothing but the length floor stops
        // an operator shipping a pepper a database reader could brute-force.
        assert_eq!(
            NearAccountIndexPepper::from_env_value(Some(&STANDARD.encode([7u8; 31]))).unwrap_err(),
            MissingNearIdentityControl::IndexPepperInvalid
        );
        assert!(NearAccountIndexPepper::from_env_value(Some(&STANDARD.encode([7u8; 32]))).is_ok());

        assert_eq!(
            NearAccountIdentity::from_parts(None, Some(wrapper("a".repeat(32).as_str())))
                .err()
                .map(|e| e.label()),
            Some("near_account_index_pepper_unconfigured")
        );
    }

    /// Fail-closed on the account-name key. There is no path that stores a name
    /// unsealed: without a key wrapper the identity cannot be constructed at
    /// all, so no caller can reach `seal`.
    #[test]
    fn an_absent_account_name_key_refuses_rather_than_storing_plaintext() {
        let error = match NearAccountIdentity::from_parts(Some(&pepper_b64(1)), None) {
            Ok(_) => panic!("an identity was built with no account-name key"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            MissingNearIdentityControl::AccountNameKeyUnconfigured
        );
        assert_eq!(error.label(), "near_account_name_key_unconfigured");
    }

    /// Every operator-facing label names a control and nothing else. A label
    /// that carried an account, a tenant, or a key reference would put identity
    /// into the logs the hash-only rule keeps it out of.
    #[test]
    fn missing_control_labels_carry_no_identity() {
        for control in [
            MissingNearIdentityControl::IndexPepperUnconfigured,
            MissingNearIdentityControl::IndexPepperInvalid,
            MissingNearIdentityControl::AccountNameKeyUnconfigured,
        ] {
            assert_eq!(control.label(), control.to_string());
            assert!(control.label().is_ascii());
            assert!(!control.label().contains(':'));
        }
    }

    /// The pepper's own identifier is stored beside every row and printed in
    /// operator output, so it must not be a verifier for a guessed pepper.
    #[test]
    fn the_pepper_ref_is_an_hmac_under_the_pepper_not_a_hash_of_it() {
        let id = identity(1, "a".repeat(32).as_str());
        let raw = [1u8; 32];
        assert_ne!(
            id.pepper_ref_hash(),
            format!("sha256:{}", hex::encode(sha2::Sha256::digest(raw))),
            "the pepper ref is a plain digest of the pepper, so it verifies a guess"
        );
        assert_eq!(
            id.pepper_ref_hash(),
            identity(1, "b".repeat(32).as_str()).pepper_ref_hash(),
            "the pepper ref must identify the pepper alone, or rotation cannot select rows by it"
        );
        assert_ne!(
            id.pepper_ref_hash(),
            identity(2, "a".repeat(32).as_str()).pepper_ref_hash()
        );
    }

    /// The sealed record must not be printable. `Debug` is derived on the rows
    /// it sits inside, and a derived `Debug` here would carry the ciphertext
    /// into any log line that formats one.
    #[test]
    fn a_sealed_name_does_not_print_its_contents() {
        let id = identity(1, "a".repeat(32).as_str());
        let (_, sealed) = id.seal("mainnet", "alice.near").unwrap();
        let printed = format!("{sealed:?}");
        assert_eq!(printed, "SealedNearAccountName(<sealed>)");
        assert!(!printed.contains(&sealed.ciphertext_base64));
    }

    #[test]
    fn an_unknown_sealed_schema_version_refuses() {
        let id = identity(1, "a".repeat(32).as_str());
        let (index, mut sealed) = id.seal("mainnet", "alice.near").unwrap();
        sealed.schema_version = "trace_commons.sealed_near_account_name.v2".into();
        let error = id.open(&index, &sealed).expect_err("unknown schema opened");
        assert!(
            error
                .to_string()
                .contains("SealedNearAccountNameSchemaRejected")
        );
    }
}
