//! Environment-wide feature decisions. Flags never grant authority or consent.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MANIFEST_BYTES: usize = 65_536;
pub const MAX_VALIDITY_SECONDS: i64 = 7 * 24 * 60 * 60;
/// How far a manifest's `issued_at` may be ahead of the client's clock.
/// Without it a client whose clock runs behind refuses a fresh manifest,
/// and keeps applying the previous one, which may still enable the flag
/// the new revision turns off.
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5 * 60;
/// Most signatures one manifest may carry; more than one is for a key rotation.
pub const MAX_MANIFEST_SIGNATURES: usize = 4;
/// Prefixed to the payload before signing, so a signature made for any other
/// document under the same key cannot be presented as a manifest signature.
pub const MANIFEST_SIGNING_DOMAIN: &[u8] = b"trace-commons.feature-manifest.v1\n";
const KEY_ID_HEX_LEN: usize = 16;
#[derive(Debug, thiserror::Error)]
#[error("invalid feature flag configuration: {0}")]
pub struct FlagError(pub &'static str);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Production,
    Staging,
    Testing,
}
impl Environment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Staging => "staging",
            Self::Testing => "testing",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Timing {
    Live,
    NextStart,
}
/// What a flag may gate. Behavior that touches consent, what leaves the
/// machine, upload or the witness is `privacy_sensitive`; it ships through a
/// release, never a remote enable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagClass {
    Product,
    PrivacySensitive,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub description: String,
    pub owner: String,
    pub class: FlagClass,
    pub default: bool,
    pub required_capabilities: BTreeSet<String>,
    pub timing: Timing,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    pub registry_version: u32,
    #[serde(deserialize_with = "unique_map")]
    pub flags: BTreeMap<String, Definition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub registry_version: u32,
    pub environment: Environment,
    pub capabilities: BTreeSet<String>,
    pub remote_enable: BTreeSet<String>,
    /// Raw Ed25519 public keys, lowercase hex, whose signature this build
    /// accepts on its environment's manifest.
    pub signing_keys: BTreeSet<String>,
}
impl Profile {
    pub fn trusted_keys(&self) -> Result<Vec<[u8; 32]>, FlagError> {
        self.signing_keys
            .iter()
            .map(|k| parse_public_key_hex(k).ok_or(FlagError("invalid signing key")))
            .collect()
    }
    pub fn digest(&self) -> Result<String, FlagError> {
        let bytes = serde_json::to_vec(self).map_err(|_| FlagError("profile serialization"))?;
        Ok(digest(&bytes))
    }
}
impl Registry {
    pub fn validate(&self, profile: &Profile) -> Result<(), FlagError> {
        if self.schema_version != 1
            || profile.schema_version != 1
            || self.registry_version != profile.registry_version
        {
            return Err(FlagError("registry/profile version mismatch"));
        }
        if self.flags.iter().any(|(id, d)| {
            id.is_empty()
                || id.len() > 100
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
                || d.description.trim().is_empty()
                || d.owner.trim().is_empty()
        }) {
            return Err(FlagError("invalid registry entry"));
        }
        if profile
            .remote_enable
            .iter()
            .any(|id| !self.flags.contains_key(id))
        {
            return Err(FlagError("unknown profile flag"));
        }
        self_check_keys(profile)?;
        for id in &profile.remote_enable {
            let definition = &self.flags[id];
            if definition.class == FlagClass::PrivacySensitive {
                return Err(FlagError(
                    "privacy-sensitive flag cannot be remotely enabled",
                ));
            }
            if definition.default {
                return Err(FlagError("remotely enabled flag must default off"));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub environment: Environment,
    pub revision: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(deserialize_with = "unique_map")]
    pub flags: BTreeMap<String, bool>,
}
impl Manifest {
    pub fn valid_at(&self, now: DateTime<Utc>) -> bool {
        self.issued_at <= now + chrono::Duration::seconds(MAX_CLOCK_SKEW_SECONDS)
            && now < self.expires_at
    }
}
/// The document as served: the signed manifest text, carried as a string so
/// the bytes that were signed are the bytes that are verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedManifest {
    pub payload: String,
    pub signatures: Vec<ManifestSignature>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSignature {
    /// [`key_id_for`] of the signing key.
    pub key_id: String,
    /// The 64-byte Ed25519 signature over [`signing_input`], lowercase hex.
    pub signature: String,
}
/// The bytes a key signs: [`MANIFEST_SIGNING_DOMAIN`] then the payload.
pub fn signing_input(payload: &[u8]) -> Vec<u8> {
    [MANIFEST_SIGNING_DOMAIN, payload].concat()
}
/// The first 8 bytes of SHA-256 over the raw public key, lowercase hex.
pub fn key_id_for(public_key: &[u8; 32]) -> String {
    hex::encode(&Sha256::digest(public_key)[..KEY_ID_HEX_LEN / 2])
}
pub fn parse_public_key_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    hex::decode(text).ok()?.try_into().ok()
}
pub fn assemble_signed_manifest(payload: String, signatures: Vec<ManifestSignature>) -> Vec<u8> {
    serde_json::to_vec(&SignedManifest {
        payload,
        signatures,
    })
    .expect("plain strings serialize")
}
/// Verify a served document against the profile's pinned keys and return
/// the payload text. Nothing in the payload is read before a signature over
/// it has verified.
pub fn open_signed_manifest(bytes: &[u8], profile: &Profile) -> Result<String, FlagError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(FlagError("manifest too large"));
    }
    let keys = profile.trusted_keys()?;
    if keys.is_empty() {
        return Err(FlagError("no trusted manifest signing key"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| FlagError("manifest JSON"))?;
    if !value.is_object() {
        return Err(FlagError("manifest JSON"));
    }
    let signed: SignedManifest =
        serde_json::from_value(value).map_err(|_| FlagError("manifest JSON"))?;
    if signed.signatures.is_empty() || signed.signatures.len() > MAX_MANIFEST_SIGNATURES {
        return Err(FlagError("manifest signature count"));
    }
    let message = signing_input(signed.payload.as_bytes());
    let verified = signed.signatures.iter().any(|signature| {
        let Ok(bytes) = hex::decode(&signature.signature) else {
            return false;
        };
        bytes.len() == 64
            && keys
                .iter()
                .filter(|key| key_id_for(key) == signature.key_id)
                .any(|key| {
                    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
                        .verify(&message, &bytes)
                        .is_ok()
                })
    });
    if !verified {
        return Err(FlagError("manifest signature"));
    }
    Ok(signed.payload)
}
/// A verified, current manifest.
pub fn parse_manifest(
    bytes: &[u8],
    profile: &Profile,
    now: DateTime<Utc>,
) -> Result<Manifest, FlagError> {
    Ok(parse_signed_manifest(bytes, profile, now)?.0)
}
/// [`parse_manifest`] plus the digest of the signed payload, which identifies
/// a revision's content independently of who signed it.
pub fn parse_signed_manifest(
    bytes: &[u8],
    profile: &Profile,
    now: DateTime<Utc>,
) -> Result<(Manifest, String), FlagError> {
    let (manifest, payload_digest) = decode_manifest(bytes, profile)?;
    if !manifest.valid_at(now) {
        return Err(FlagError("manifest not current"));
    }
    Ok((manifest, payload_digest))
}
/// Signature and structure, but not time: also used to retain an expired
/// cache's revision watermark.
pub fn decode_manifest(bytes: &[u8], profile: &Profile) -> Result<(Manifest, String), FlagError> {
    let payload = open_signed_manifest(bytes, profile)?;
    let manifest = decode_payload(payload.as_bytes(), profile.environment)?;
    Ok((manifest, digest(payload.as_bytes())))
}
/// Structural validation of an unsigned manifest, as an operator writes it
/// before signing.
pub fn decode_payload(bytes: &[u8], environment: Environment) -> Result<Manifest, FlagError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(FlagError("manifest too large"));
    }
    let m: Manifest = serde_json::from_slice(bytes).map_err(|_| FlagError("manifest JSON"))?;
    if m.schema_version != 1 || m.environment != environment || m.revision == 0 {
        return Err(FlagError("manifest identity"));
    }
    let validity = m.expires_at.signed_duration_since(m.issued_at);
    if validity <= chrono::Duration::zero()
        || validity > chrono::Duration::seconds(MAX_VALIDITY_SECONDS)
    {
        return Err(FlagError("manifest validity"));
    }
    Ok(m)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub enabled: bool,
    pub pending: bool,
    pub capability_blocked: bool,
    pub timing: Timing,
}
pub fn evaluate(
    registry: &Registry,
    profile: &Profile,
    manifest: Option<&Manifest>,
    now: DateTime<Utc>,
) -> BTreeMap<String, Decision> {
    let manifest = manifest.filter(|m| m.environment == profile.environment && m.valid_at(now));
    registry
        .flags
        .iter()
        .map(|(id, definition)| {
            let requested = manifest
                .and_then(|m| m.flags.get(id))
                .copied()
                .unwrap_or(definition.default);
            let blocked = !definition
                .required_capabilities
                .is_subset(&profile.capabilities)
                || !profile.remote_enable.contains(id);
            let enabled = requested && !blocked;
            (
                id.clone(),
                Decision {
                    enabled,
                    pending: enabled,
                    capability_blocked: requested && blocked,
                    timing: definition.timing,
                },
            )
        })
        .collect()
}
fn self_check_keys(profile: &Profile) -> Result<(), FlagError> {
    profile.trusted_keys()?;
    if !profile.remote_enable.is_empty() && profile.signing_keys.is_empty() {
        return Err(FlagError("remote enable requires a manifest signing key"));
    }
    Ok(())
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map with unique keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, T>()? {
                if result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate key"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}
#[cfg(test)]
mod tests;
