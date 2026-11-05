//! The signed list of witness measurements a client accepts in addition to
//! the pins it was given at sign-up.
//!
//! Spec 2026-11-05 (witness at submission), section 7. A device writes its
//! witness pins once, when it joins. A witness redeploy changes the image
//! measurement, and without a way to learn the new one every existing device
//! refuses the witness until someone re-pins it by hand. The commons
//! therefore serves one small signed document at
//! [`WITNESS_ACCEPTED_LIST_PATH`]; clients fetch it, verify it against
//! list-signing public keys compiled into the client, and accept a witness
//! whose measurement **and** signing address match any entry that is valid at
//! the time of the check.
//!
//! # Shape
//!
//! The served file is a [`SignedAcceptedList`]:
//!
//! ```json
//! {
//!   "payload": "{\"entries\":[...],\"issued_at\":\"...\",...}",
//!   "signatures": [{ "key_id": "<16 hex>", "signature": "<128 hex>" }]
//! }
//! ```
//!
//! `payload` is the signed JSON text itself, carried as a string, so the
//! bytes that were signed are the bytes that are verified: nothing is
//! re-serialised before verification, and the key order of whatever JSON map
//! a build happens to use cannot move a signature. The payload parses into an
//! [`AcceptedList`] only after a signature over it has verified.
//!
//! # What it can and cannot do
//!
//! - It only **adds** candidates. The sign-up pins stay in force beside it,
//!   and a client with no pins is not made pinned by a list.
//! - It never names a different witness: its `witness_url` must equal the
//!   configured witness URL, or the list is ignored.
//! - It is accepted only with a strictly higher `version` than the one held
//!   ([`accepts_version`]), so a replayed older list cannot bring back an
//!   entry the operator removed.
//! - Entries are judged against their validity window when they are used,
//!   not when the list arrives ([`AcceptedEntry::is_valid_at`]).
//!
//! Signatures are Ed25519 over the payload bytes, with no digest step: that
//! is what a Google Cloud KMS `ec-sign-ed25519` key signs (the operator
//! runbook is `docs/operator/witness-accepted-list.md`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The public ingest route the signed list is served on. Unauthenticated and
/// outside tenant context: the list is public, and a client reads it before
/// it holds any credential that would matter.
pub const WITNESS_ACCEPTED_LIST_PATH: &str = "/v1/public/witness-accepted-list";

/// The largest served document a client or the server reads. A list of a few
/// entries is about a kilobyte; this bounds the read, it is not a policy
/// value.
pub const WITNESS_ACCEPTED_LIST_MAX_BYTES: usize = 64 * 1024;

/// The payload's `schema`. A field added to the payload is a new schema:
/// every type here denies unknown fields, so an older client refuses a list
/// it cannot fully read rather than skipping a field that might narrow trust.
pub const WITNESS_ACCEPTED_LIST_SCHEMA: &str = "trace-commons.witness-accepted-list.v1";

/// Most entries one list may carry. Rollover needs two (old and new).
pub const WITNESS_ACCEPTED_LIST_MAX_ENTRIES: usize = 32;

/// Most signatures one list may carry. More than one is for a key rotation,
/// when a list is signed by the outgoing and the incoming key.
pub const WITNESS_ACCEPTED_LIST_MAX_SIGNATURES: usize = 4;

/// The longest measurement-set string accepted. Six 96-hex registers with
/// their keys and commas is under 640 characters.
const MEASUREMENTS_MAX_LEN: usize = 1024;

/// The longest witness URL accepted.
const WITNESS_URL_MAX_LEN: usize = 2048;

/// Length in hex of a [`key_id_for`] value.
const KEY_ID_HEX_LEN: usize = 16;

/// The signed document as served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedAcceptedList {
    /// The exact JSON text that was signed.
    pub payload: String,
    /// One or more Ed25519 signatures over `payload`'s UTF-8 bytes.
    pub signatures: Vec<ListSignature>,
}

/// One signature over the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSignature {
    /// [`key_id_for`] of the public key that made it.
    pub key_id: String,
    /// The 64-byte Ed25519 signature, lowercase hex.
    pub signature: String,
}

/// The signed payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedList {
    /// Always [`WITNESS_ACCEPTED_LIST_SCHEMA`].
    pub schema: String,
    /// Monotonically increasing. A client accepts a list only with a version
    /// strictly higher than the one it holds.
    pub version: u64,
    /// When the operator built it. Informational; nothing is judged by it.
    pub issued_at: DateTime<Utc>,
    /// The witness base URL these measurements belong to. A client whose
    /// configured witness URL differs ignores the list.
    pub witness_url: String,
    /// The accepted measurement sets.
    pub entries: Vec<AcceptedEntry>,
}

/// One accepted witness deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedEntry {
    /// One measurement set, in the client pin spelling:
    /// `mrtd=<96 hex>,mrconfigid=<96 hex>`. Parsed by the client with the
    /// same parser as its sign-up pins; one that does not parse refuses the
    /// whole list.
    pub measurements: String,
    /// The address the witness signs certificates with, `0x` and 40 hex.
    pub signing_address: String,
    /// The entry is not accepted before this instant.
    pub not_before: DateTime<Utc>,
    /// The entry is not accepted at or after this instant.
    pub not_after: DateTime<Utc>,
    /// The commit the image was built from, for deploy provenance. Not
    /// judged by the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

impl AcceptedEntry {
    /// Whether the entry is in force at `now`: `not_before <= now < not_after`.
    #[must_use]
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        self.not_before <= now && now < self.not_after
    }
}

impl AcceptedList {
    /// Whether this list names `configured_url`. Compared exactly, apart from
    /// trailing slashes, which name the same base URL.
    #[must_use]
    pub fn names_witness(&self, configured_url: &str) -> bool {
        normalize_url(&self.witness_url) == normalize_url(configured_url)
    }

    /// The entries in force at `now`.
    pub fn entries_valid_at(&self, now: DateTime<Utc>) -> impl Iterator<Item = &AcceptedEntry> {
        self.entries.iter().filter(move |entry| entry.is_valid_at(now))
    }

    /// The bytes an operator signs: the payload as key-sorted, compact JSON.
    ///
    /// Only the operator tool calls this. A client never re-serialises: it
    /// verifies the payload text it received.
    pub fn canonical_payload(&self) -> Result<String, AcceptedListError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| AcceptedListError::Malformed)?;
        crate::canonical_json::to_canonical_string(&value).map_err(|_| AcceptedListError::Malformed)
    }

    /// Every rule a list must meet besides its signature.
    pub fn validate(&self) -> Result<(), AcceptedListError> {
        if self.schema != WITNESS_ACCEPTED_LIST_SCHEMA {
            return Err(AcceptedListError::SchemaUnsupported);
        }
        if self.version == 0 {
            return Err(AcceptedListError::Invalid);
        }
        if !witness_url_shaped(&self.witness_url) {
            return Err(AcceptedListError::Invalid);
        }
        if self.entries.len() > WITNESS_ACCEPTED_LIST_MAX_ENTRIES {
            return Err(AcceptedListError::Invalid);
        }
        for entry in &self.entries {
            let measurements_ok = !entry.measurements.trim().is_empty()
                && entry.measurements.len() <= MEASUREMENTS_MAX_LEN
                && entry.measurements.is_ascii();
            let commit_ok = entry.commit.as_deref().is_none_or(|commit| {
                (7..=64).contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit())
            });
            if !measurements_ok
                || !signing_address_shaped(&entry.signing_address)
                || entry.not_before >= entry.not_after
                || !commit_ok
            {
                return Err(AcceptedListError::Invalid);
            }
        }
        Ok(())
    }
}

/// Why a list was not accepted. Labels only: no variant carries any part of
/// the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AcceptedListError {
    /// Larger than [`WITNESS_ACCEPTED_LIST_MAX_BYTES`].
    #[error("witness-accepted-list-too-large")]
    TooLarge,
    /// Not the served shape, or a payload that is not an [`AcceptedList`].
    #[error("witness-accepted-list-malformed")]
    Malformed,
    /// No signature verified under any trusted key. Also the answer when the
    /// trusted set is empty.
    #[error("witness-accepted-list-signature-invalid")]
    SignatureInvalid,
    /// A payload `schema` this build does not read.
    #[error("witness-accepted-list-schema-unsupported")]
    SchemaUnsupported,
    /// A signed payload that breaks a rule in [`AcceptedList::validate`].
    #[error("witness-accepted-list-invalid")]
    Invalid,
}

impl AcceptedListError {
    /// The fixed label for logs and health.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::TooLarge => "witness-accepted-list-too-large",
            Self::Malformed => "witness-accepted-list-malformed",
            Self::SignatureInvalid => "witness-accepted-list-signature-invalid",
            Self::SchemaUnsupported => "witness-accepted-list-schema-unsupported",
            Self::Invalid => "witness-accepted-list-invalid",
        }
    }
}

/// The key id a signature names: the first 8 bytes of SHA-256 over the raw
/// 32-byte public key, as 16 lowercase hex characters.
#[must_use]
pub fn key_id_for(public_key: &[u8; 32]) -> String {
    hex::encode(&Sha256::digest(public_key)[..KEY_ID_HEX_LEN / 2])
}

/// Parse a 32-byte Ed25519 public key from 64 hex characters.
pub fn parse_public_key_hex(hex_key: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_key.trim()).ok()?;
    bytes.try_into().ok()
}

/// Whether a list at `candidate` may replace the one held at `held`: only a
/// strictly higher version, so neither a replay nor a repeat is applied.
#[must_use]
pub fn accepts_version(held: Option<u64>, candidate: u64) -> bool {
    held.is_none_or(|held| candidate > held)
}

/// Verify a served document and return its payload.
///
/// In order: the size bound, the served shape, a signature from any key in
/// `trusted_keys` over the payload bytes, then the payload's own rules. An
/// empty `trusted_keys` verifies nothing, so a client built without a key is
/// inert.
pub fn verify_signed_list(
    document: &[u8],
    trusted_keys: &[[u8; 32]],
) -> Result<AcceptedList, AcceptedListError> {
    if document.len() > WITNESS_ACCEPTED_LIST_MAX_BYTES {
        return Err(AcceptedListError::TooLarge);
    }
    // An object only: serde would also read a struct from an array.
    let value: serde_json::Value =
        serde_json::from_slice(document).map_err(|_| AcceptedListError::Malformed)?;
    if !value.is_object() {
        return Err(AcceptedListError::Malformed);
    }
    let signed: SignedAcceptedList =
        serde_json::from_value(value).map_err(|_| AcceptedListError::Malformed)?;
    if signed.signatures.is_empty() || signed.signatures.len() > WITNESS_ACCEPTED_LIST_MAX_SIGNATURES
    {
        return Err(AcceptedListError::Malformed);
    }
    if !signed
        .signatures
        .iter()
        .any(|signature| signature_verifies(signed.payload.as_bytes(), signature, trusted_keys))
    {
        return Err(AcceptedListError::SignatureInvalid);
    }
    let payload: serde_json::Value =
        serde_json::from_str(&signed.payload).map_err(|_| AcceptedListError::Malformed)?;
    if !payload.is_object() {
        return Err(AcceptedListError::Malformed);
    }
    if payload.get("schema").and_then(serde_json::Value::as_str)
        != Some(WITNESS_ACCEPTED_LIST_SCHEMA)
    {
        return Err(AcceptedListError::SchemaUnsupported);
    }
    let list: AcceptedList =
        serde_json::from_value(payload).map_err(|_| AcceptedListError::Malformed)?;
    list.validate()?;
    Ok(list)
}

/// Assemble a served document from a payload and its signatures. Used by the
/// operator tool after signing; the result is checked with
/// [`verify_signed_list`] before it is written.
pub fn assemble_signed_list(payload: String, signatures: Vec<ListSignature>) -> Vec<u8> {
    let signed = SignedAcceptedList {
        payload,
        signatures,
    };
    let value = serde_json::to_value(&signed).expect("plain strings serialize");
    let mut text =
        crate::canonical_json::to_canonical_string(&value).expect("plain strings serialize");
    text.push('\n');
    text.into_bytes()
}

fn signature_verifies(
    payload: &[u8],
    signature: &ListSignature,
    trusted_keys: &[[u8; 32]],
) -> bool {
    let Ok(signature_bytes) = hex::decode(&signature.signature) else {
        return false;
    };
    if signature_bytes.len() != 64 {
        return false;
    }
    trusted_keys
        .iter()
        .filter(|key| key_id_for(key) == signature.key_id)
        .any(|key| {
            ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
                .verify(payload, &signature_bytes)
                .is_ok()
        })
}

fn normalize_url(url: &str) -> &str {
    url.trim().trim_end_matches('/')
}

fn witness_url_shaped(url: &str) -> bool {
    if url.len() > WITNESS_URL_MAX_LEN || url.trim() != url {
        return false;
    }
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !url.chars().any(char::is_whitespace)
}

fn signing_address_shaped(address: &str) -> bool {
    address
        .strip_prefix("0x")
        .is_some_and(|hex| hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 0, 0, 0).unwrap()
    }

    fn test_key() -> Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn public(key: &Ed25519KeyPair) -> [u8; 32] {
        key.public_key().as_ref().try_into().unwrap()
    }

    fn entry() -> AcceptedEntry {
        AcceptedEntry {
            measurements: format!("mrtd={},mrconfigid={}", "aa".repeat(48), "bb".repeat(48)),
            signing_address: format!("0x{}", "11".repeat(20)),
            not_before: at(1),
            not_after: at(20),
            commit: Some("85173de19".into()),
        }
    }

    fn list(version: u64) -> AcceptedList {
        AcceptedList {
            schema: WITNESS_ACCEPTED_LIST_SCHEMA.into(),
            version,
            issued_at: at(5),
            witness_url: "https://witness.example".into(),
            entries: vec![entry()],
        }
    }

    fn sign_payload(key: &Ed25519KeyPair, payload: &str) -> ListSignature {
        ListSignature {
            key_id: key_id_for(&public(key)),
            signature: hex::encode(key.sign(payload.as_bytes()).as_ref()),
        }
    }

    fn signed(key: &Ed25519KeyPair, list: &AcceptedList) -> Vec<u8> {
        let payload = list.canonical_payload().unwrap();
        let signature = sign_payload(key, &payload);
        assemble_signed_list(payload, vec![signature])
    }

    #[test]
    fn a_good_signature_verifies_and_returns_the_payload() {
        let key = test_key();
        let document = signed(&key, &list(3));
        let verified = verify_signed_list(&document, &[public(&key)]).unwrap();
        assert_eq!(verified, list(3));
    }

    #[test]
    fn an_empty_trusted_set_verifies_nothing() {
        let key = test_key();
        let document = signed(&key, &list(3));
        assert_eq!(
            verify_signed_list(&document, &[]),
            Err(AcceptedListError::SignatureInvalid)
        );
    }

    #[test]
    fn a_signature_from_an_untrusted_key_is_refused() {
        let signer = test_key();
        let trusted = test_key();
        let document = signed(&signer, &list(3));
        assert_eq!(
            verify_signed_list(&document, &[public(&trusted)]),
            Err(AcceptedListError::SignatureInvalid)
        );
    }

    #[test]
    fn a_trusted_key_id_with_a_forged_signature_is_refused() {
        // The signature names the trusted key's id but was made by another.
        let trusted = test_key();
        let forger = test_key();
        let payload = list(3).canonical_payload().unwrap();
        let mut signature = sign_payload(&forger, &payload);
        signature.key_id = key_id_for(&public(&trusted));
        let document = assemble_signed_list(payload, vec![signature]);
        assert_eq!(
            verify_signed_list(&document, &[public(&trusted)]),
            Err(AcceptedListError::SignatureInvalid)
        );
    }

    #[test]
    fn a_flipped_signature_bit_is_refused() {
        let key = test_key();
        let payload = list(3).canonical_payload().unwrap();
        let mut signature = sign_payload(&key, &payload);
        let mut bytes = hex::decode(&signature.signature).unwrap();
        bytes[10] ^= 1;
        signature.signature = hex::encode(bytes);
        let document = assemble_signed_list(payload, vec![signature]);
        assert_eq!(
            verify_signed_list(&document, &[public(&key)]),
            Err(AcceptedListError::SignatureInvalid)
        );
    }

    #[test]
    fn a_tampered_payload_is_refused() {
        let key = test_key();
        let payload = list(3).canonical_payload().unwrap();
        let signature = sign_payload(&key, &payload);
        // Same document, version raised after signing.
        let tampered = payload.replace("\"version\":3", "\"version\":4");
        assert_ne!(tampered, payload);
        let document = assemble_signed_list(tampered, vec![signature]);
        assert_eq!(
            verify_signed_list(&document, &[public(&key)]),
            Err(AcceptedListError::SignatureInvalid)
        );
    }

    #[test]
    fn a_reformatted_payload_is_refused_though_it_means_the_same() {
        // The bytes are what is signed, not the meaning.
        let key = test_key();
        let payload = list(3).canonical_payload().unwrap();
        let signature = sign_payload(&key, &payload);
        let reformatted = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
        )
        .unwrap();
        let document = assemble_signed_list(reformatted, vec![signature]);
        assert_eq!(
            verify_signed_list(&document, &[public(&key)]),
            Err(AcceptedListError::SignatureInvalid)
        );
    }

    #[test]
    fn any_trusted_signature_among_several_is_enough() {
        let outgoing = test_key();
        let incoming = test_key();
        let payload = list(3).canonical_payload().unwrap();
        let document = assemble_signed_list(
            payload.clone(),
            vec![
                sign_payload(&outgoing, &payload),
                sign_payload(&incoming, &payload),
            ],
        );
        assert!(verify_signed_list(&document, &[public(&incoming)]).is_ok());
        assert!(verify_signed_list(&document, &[public(&outgoing)]).is_ok());
    }

    #[test]
    fn an_unsigned_document_is_refused() {
        let payload = list(3).canonical_payload().unwrap();
        let document = assemble_signed_list(payload, vec![]);
        assert_eq!(
            verify_signed_list(&document, &[public(&test_key())]),
            Err(AcceptedListError::Malformed)
        );
        // A bare payload served on its own is not the signed shape.
        let bare = list(3).canonical_payload().unwrap();
        assert_eq!(
            verify_signed_list(bare.as_bytes(), &[public(&test_key())]),
            Err(AcceptedListError::Malformed)
        );
    }

    #[test]
    fn an_oversized_document_is_refused_before_parsing() {
        let document = vec![b' '; WITNESS_ACCEPTED_LIST_MAX_BYTES + 1];
        assert_eq!(
            verify_signed_list(&document, &[]),
            Err(AcceptedListError::TooLarge)
        );
    }

    #[test]
    fn a_signed_payload_with_an_unknown_field_is_refused() {
        let key = test_key();
        let mut value = serde_json::to_value(list(3)).unwrap();
        value["entries"][0]["extra"] = serde_json::json!("x");
        let payload = crate::canonical_json::to_canonical_string(&value).unwrap();
        let document = assemble_signed_list(payload.clone(), vec![sign_payload(&key, &payload)]);
        assert_eq!(
            verify_signed_list(&document, &[public(&key)]),
            Err(AcceptedListError::Malformed)
        );
    }

    #[test]
    fn a_signed_payload_of_another_schema_is_refused() {
        let key = test_key();
        let mut value = serde_json::to_value(list(3)).unwrap();
        value["schema"] = serde_json::json!("trace-commons.witness-accepted-list.v2");
        let payload = crate::canonical_json::to_canonical_string(&value).unwrap();
        let document = assemble_signed_list(payload.clone(), vec![sign_payload(&key, &payload)]);
        assert_eq!(
            verify_signed_list(&document, &[public(&key)]),
            Err(AcceptedListError::SchemaUnsupported)
        );
    }

    #[test]
    fn signed_but_invalid_payloads_are_refused() {
        let key = test_key();
        let cases: Vec<Box<dyn Fn(&mut AcceptedList)>> = vec![
            Box::new(|l| l.version = 0),
            Box::new(|l| l.witness_url = "witness.example".into()),
            Box::new(|l| l.entries[0].signing_address = "0x12".into()),
            Box::new(|l| l.entries[0].measurements = " ".into()),
            Box::new(|l| l.entries[0].not_after = l.entries[0].not_before),
            Box::new(|l| l.entries[0].commit = Some("not-hex".into())),
            Box::new(|l| {
                l.entries = vec![entry(); WITNESS_ACCEPTED_LIST_MAX_ENTRIES + 1];
            }),
        ];
        for (index, mutate) in cases.iter().enumerate() {
            let mut bad = list(3);
            mutate(&mut bad);
            assert_eq!(
                bad.canonical_payload(),
                Err(AcceptedListError::Invalid),
                "case {index}"
            );
            // Signed by hand, bypassing the builder's own check.
            let value = serde_json::to_value(&bad).unwrap();
            let payload = crate::canonical_json::to_canonical_string(&value).unwrap();
            let document =
                assemble_signed_list(payload.clone(), vec![sign_payload(&key, &payload)]);
            assert_eq!(
                verify_signed_list(&document, &[public(&key)]),
                Err(AcceptedListError::Invalid),
                "case {index}"
            );
        }
    }

    #[test]
    fn versions_only_move_up() {
        assert!(accepts_version(None, 1));
        assert!(accepts_version(Some(3), 4));
        assert!(!accepts_version(Some(3), 3), "a repeat is not applied");
        assert!(!accepts_version(Some(3), 2), "a rollback is refused");
    }

    #[test]
    fn an_entry_is_valid_from_not_before_until_just_before_not_after() {
        let entry = entry();
        assert!(!entry.is_valid_at(at(1) - chrono::Duration::seconds(1)));
        assert!(entry.is_valid_at(at(1)));
        assert!(entry.is_valid_at(at(19)));
        assert!(!entry.is_valid_at(at(20)));
        let mut list = list(3);
        let mut later = self::entry();
        later.not_before = at(25);
        later.not_after = at(28);
        list.entries.push(later);
        assert_eq!(list.entries_valid_at(at(10)).count(), 1);
        assert_eq!(list.entries_valid_at(at(26)).count(), 1);
        assert_eq!(list.entries_valid_at(at(29)).count(), 0);
    }

    #[test]
    fn the_witness_url_is_compared_exactly_but_for_trailing_slashes() {
        let list = list(3);
        assert!(list.names_witness("https://witness.example"));
        assert!(list.names_witness("https://witness.example/"));
        assert!(!list.names_witness("https://other.example"));
        assert!(!list.names_witness("http://witness.example"));
    }

    #[test]
    fn the_canonical_payload_is_key_sorted_and_compact() {
        let payload = list(3).canonical_payload().unwrap();
        assert!(payload.starts_with("{\"entries\":[{\"commit\":\"85173de19\""));
        assert!(!payload.contains(": "));
        assert!(!payload.contains('\n'));
        // And stable: the same list, the same bytes.
        assert_eq!(payload, list(3).canonical_payload().unwrap());
    }

    #[test]
    fn key_ids_and_hex_public_keys_round_trip() {
        let key = test_key();
        let raw = public(&key);
        assert_eq!(parse_public_key_hex(&hex::encode(raw)), Some(raw));
        assert_eq!(parse_public_key_hex("abcd"), None);
        assert_eq!(key_id_for(&raw).len(), KEY_ID_HEX_LEN);
        assert_ne!(key_id_for(&raw), key_id_for(&public(&test_key())));
    }

    #[test]
    fn error_labels_match_their_display() {
        for error in [
            AcceptedListError::TooLarge,
            AcceptedListError::Malformed,
            AcceptedListError::SignatureInvalid,
            AcceptedListError::SchemaUnsupported,
            AcceptedListError::Invalid,
        ] {
            assert_eq!(error.label(), error.to_string());
        }
    }
}
