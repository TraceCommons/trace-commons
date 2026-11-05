//! Environment-wide feature decisions. Flags never grant authority or consent.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MANIFEST_BYTES: usize = 65_536;
pub const MAX_VALIDITY_SECONDS: i64 = 7 * 24 * 60 * 60;
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub description: String,
    pub owner: String,
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
}
impl Profile {
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
        self.issued_at <= now && now < self.expires_at
    }
}
pub fn parse_manifest(
    bytes: &[u8],
    environment: Environment,
    now: DateTime<Utc>,
) -> Result<Manifest, FlagError> {
    let manifest = decode_manifest(bytes, environment)?;
    if !manifest.valid_at(now) {
        return Err(FlagError("manifest not current"));
    }
    Ok(manifest)
}
/// Structural validation also used to retain an expired cache's revision watermark.
pub fn decode_manifest(bytes: &[u8], environment: Environment) -> Result<Manifest, FlagError> {
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
