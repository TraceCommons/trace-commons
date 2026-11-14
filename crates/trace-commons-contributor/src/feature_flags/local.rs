//! Local product choices are separate from signed environment configuration.
use super::*;
use std::{io::Read, path::PathBuf};
const MAX_LOCAL_BYTES: u64 = 65_536;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    environment: Environment,
    profile_digest: String,
    #[serde(deserialize_with = "unique_flags")]
    flags: BTreeMap<String, bool>,
}

pub(super) struct LocalOverrides {
    dir: PathBuf,
    path: PathBuf,
    identity: String,
    environment: Environment,
}
impl LocalOverrides {
    pub fn new(dir: &Path, identity: &str, environment: Environment) -> Self {
        Self {
            dir: dir.to_owned(),
            path: dir.join(format!("feature-flags-local-{identity}.json")),
            identity: identity.into(),
            environment,
        }
    }
    pub fn load(&self, registry: &Registry, profile: &Profile) -> BTreeMap<String, bool> {
        self.load_valid(registry, profile).unwrap_or_default()
    }
    fn load_valid(&self, registry: &Registry, profile: &Profile) -> Option<BTreeMap<String, bool>> {
        let mut bytes = Vec::new();
        let file =
            trace_commons_contributor_platform::local_files::open_import_file(&self.path).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        file.take(MAX_LOCAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_LOCAL_BYTES {
            return None;
        }
        let document: Document = serde_json::from_slice(&bytes).ok()?;
        if document.schema_version != 1
            || document.environment != self.environment
            || document.profile_digest != self.identity
            || document
                .flags
                .keys()
                .any(|id| !allowed(registry, profile, id))
        {
            return None;
        }
        Some(document.flags)
    }
    pub fn save(&self, flags: &BTreeMap<String, bool>) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(&Document {
            schema_version: 1,
            environment: self.environment,
            profile_digest: self.identity.clone(),
            flags: flags.clone(),
        })?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_LOCAL_BYTES,
            "feature-local-overrides-too-large"
        );
        crate::config::write_atomic_0600(&self.dir, &self.path, &bytes)
    }
    /// Discard previous profiles so installing an older profile cannot revive a choice.
    pub fn remove_other_profiles(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(identity) = name
                .to_str()
                .and_then(|s| s.strip_prefix("feature-flags-local-"))
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            if identity != self.identity
                && identity.len() == 64
                && identity.bytes().all(|b| b.is_ascii_hexdigit())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) fn allowed(registry: &Registry, profile: &Profile, id: &str) -> bool {
    registry.flags.get(id).is_some_and(|d| {
        d.class == FlagClass::Product && d.required_capabilities.is_subset(&profile.capabilities)
    })
}

pub(super) fn apply(flags: &mut BTreeMap<String, Decision>, overrides: &BTreeMap<String, bool>) {
    // Every entry was validated against the exact registry/profile on load or mutation.
    for (id, value) in overrides {
        if let Some(decision) = flags.get_mut(id) {
            decision.enabled = *value;
            decision.pending = *value;
            decision.capability_blocked = false;
        }
    }
}

pub(super) fn unique_flags<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, bool>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, bool>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique boolean feature flags")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut flags = BTreeMap::new();
            while let Some((id, value)) = map.next_entry::<String, bool>()? {
                if flags.insert(id, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate flag"));
                }
            }
            Ok(flags)
        }
    }
    deserializer.deserialize_map(Visitor)
}
