//! Remote configuration is advisory product policy, never an authorization grant.
mod cache;
mod environment;
mod local;
mod runtime;
mod transport;
use chrono::{DateTime, Utc};
pub use runtime::run;
/// JSON object of per-process product flag choices, read at daemon startup.
pub const ENVIRONMENT_OVERRIDE_VARIABLE: &str = "TRACE_COMMONS_FEATURE_FLAGS";
/// The refusal when the local choices file on disk did not validate.
pub const LOCAL_OVERRIDES_INVALID: &str = "feature-local-overrides-invalid";
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, sync::Mutex, time::Instant};
pub use trace_commons_protocol::feature_flags::*;

// A misselected release profile is a compilation error, not a late runtime fallback.
const _: () = {
    if let Some(selected) = option_env!("TC_FEATURE_PROFILE") {
        match selected.as_bytes() {
            b"production" => assert!(
                !cfg!(feature = "near-ai-staging"),
                "feature-profile-channel-mismatch"
            ),
            b"staging" | b"testing" => assert!(
                cfg!(feature = "near-ai-staging"),
                "feature-profile-channel-mismatch"
            ),
            _ => panic!("unknown-feature-profile"),
        }
    }
};

pub fn registry() -> anyhow::Result<Registry> {
    Ok(serde_json::from_str(include_str!(
        "../../feature-flags/registry.json"
    ))?)
}
pub fn select_environment(selected: Option<&str>, channel: &str) -> anyhow::Result<Environment> {
    match (selected.unwrap_or(channel), channel) {
        ("production", "production") => Ok(Environment::Production),
        ("staging", "staging") => Ok(Environment::Staging),
        ("testing", "staging") => Ok(Environment::Testing),
        _ => anyhow::bail!("feature-profile-channel-mismatch"),
    }
}
pub fn embedded_profile() -> anyhow::Result<Profile> {
    let env = select_environment(option_env!("TC_FEATURE_PROFILE"), crate::channel::CHANNEL)?;
    profile_for_environment(env)
}
/// Local publishing validator uses any environment's current source profile.
/// This does not select the runtime's environment or permit a runtime override.
pub fn profile_for_environment(env: Environment) -> anyhow::Result<Profile> {
    let raw = match env {
        Environment::Production => {
            include_str!("../../feature-flags/profiles/production.json")
        }
        Environment::Staging => {
            include_str!("../../feature-flags/profiles/staging.json")
        }
        Environment::Testing => {
            include_str!("../../feature-flags/profiles/testing.json")
        }
    };
    let profile: Profile = serde_json::from_str(raw)?;
    anyhow::ensure!(
        profile.environment == env,
        "feature-profile-environment-mismatch"
    );
    registry()?.validate(&profile)?;
    Ok(profile)
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildProfileIdentity {
    pub schema_version: u32,
    pub environment: Environment,
    pub profile_digest: String,
}
pub fn build_profile_identity() -> anyhow::Result<BuildProfileIdentity> {
    let p = embedded_profile()?;
    Ok(BuildProfileIdentity {
        schema_version: 1,
        environment: p.environment,
        profile_digest: p.digest()?,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub environment: Environment,
    pub profile_digest: String,
    pub instance_id: String,
    pub generation: u64,
    pub sampled_at: DateTime<Utc>,
    pub source: String,
    pub revision: Option<u64>,
    pub expires_at: Option<DateTime<Utc>>,
    pub fresh: bool,
    pub flags: BTreeMap<String, Decision>,
    /// Per-machine selection; absence means follow the environment.
    #[serde(default)]
    pub local_overrides: BTreeMap<String, bool>,
    #[serde(default)]
    pub local_override_allowed: BTreeMap<String, bool>,
    #[serde(default)]
    pub environment_variable_overrides: BTreeMap<String, bool>,
    #[serde(default)]
    pub environment_variable_error: Option<String>,
}
impl Snapshot {
    pub fn is_enabled(&self, id: &str) -> bool {
        self.flags.get(id).is_some_and(|d| d.enabled)
    }
}
struct State {
    manifest: Option<Manifest>,
    watermark: Option<cache::Watermark>,
    deadline: Option<Instant>,
    expired: bool,
    latched: BTreeMap<String, bool>,
    local_overrides: BTreeMap<String, bool>,
    /// A local choices file is present but did not validate at startup.
    local_unreadable: bool,
    snapshot: Snapshot,
}
pub struct Runtime {
    registry: Registry,
    profile: Profile,
    cache: cache::Cache,
    local: local::LocalOverrides,
    state: Mutex<State>,
    pub(crate) wake: tokio::sync::Notify,
}
impl Runtime {
    pub fn open(
        registry: Registry,
        profile: Profile,
        dir: &Path,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Self> {
        Self::open_with_environment(registry, profile, dir, now, None, true)
    }
    /// `prune` removes other profiles' signed caches and local choices. Only
    /// the daemon prunes: a read-only status view from another build must
    /// not delete the running build's persisted choices.
    fn open_with_environment(
        registry: Registry,
        profile: Profile,
        dir: &Path,
        now: DateTime<Utc>,
        environment_value: Option<&str>,
        prune: bool,
    ) -> anyhow::Result<Self> {
        registry.validate(&profile)?;
        let (environment_variable_overrides, environment_variable_error) =
            match environment::parse(environment_value, &registry, &profile) {
                Ok(flags) => (flags, None),
                Err(error) => (BTreeMap::new(), Some(error.to_owned())),
            };
        let identity = profile.digest()?;
        let cache = cache::Cache::new(dir, &identity, profile.environment);
        let local = local::LocalOverrides::new(dir, &identity, profile.environment);
        if prune {
            cache.remove_other_profiles();
            local.remove_other_profiles();
        }
        let (local_overrides, local_unreadable) = local.load_checked(&registry, &profile);
        let local_override_allowed = registry
            .flags
            .keys()
            .map(|id| (id.clone(), local::allowed(&registry, &profile, id)))
            .collect();
        let cached = cache.load(&profile);
        // The higher of the stored watermark and the cached revision: either
        // write may have been the one that failed.
        let watermark = [
            cache.load_watermark(),
            cached.as_ref().map(|(m, _, d)| cache::Watermark {
                revision: m.revision,
                payload_digest: d.clone(),
            }),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|w| w.revision);
        let manifest = cached.map(|(m, _, _)| m);
        let expired = manifest.as_ref().is_some_and(|m| !m.valid_at(now));
        let mut flags = evaluate(
            &registry,
            &profile,
            manifest.as_ref().filter(|_| !expired),
            now,
        );
        local::apply(&mut flags, &local_overrides);
        local::apply(&mut flags, &environment_variable_overrides);
        let latched = flags
            .iter()
            .map(|(id, d)| (id.clone(), d.enabled))
            .collect();
        let deadline = manifest
            .as_ref()
            .filter(|_| !expired)
            .and_then(|m| remaining_deadline(m, now));
        let snapshot = Snapshot {
            schema_version: 1,
            environment: profile.environment,
            profile_digest: identity,
            instance_id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
            sampled_at: now,
            source: if manifest.is_some() && !expired {
                "cache"
            } else {
                "default"
            }
            .into(),
            revision: manifest.as_ref().map(|m| m.revision),
            expires_at: manifest.as_ref().map(|m| m.expires_at),
            fresh: manifest.is_some() && !expired,
            flags,
            local_overrides: local_overrides.clone(),
            local_override_allowed,
            environment_variable_overrides,
            environment_variable_error,
        };
        Ok(Self {
            registry,
            profile,
            cache,
            local,
            state: Mutex::new(State {
                manifest,
                watermark,
                deadline,
                expired,
                latched,
                local_overrides,
                local_unreadable,
                snapshot,
            }),
            wake: tokio::sync::Notify::new(),
        })
    }
    /// The daemon's runtime: prunes other profiles' files.
    pub fn for_store(store: &crate::config::ConfigStore) -> anyhow::Result<Self> {
        Self::for_store_with(store, true)
    }
    /// A status view with no daemon running. Deletes nothing.
    pub fn for_store_read_only(store: &crate::config::ConfigStore) -> anyhow::Result<Self> {
        Self::for_store_with(store, false)
    }
    fn for_store_with(store: &crate::config::ConfigStore, prune: bool) -> anyhow::Result<Self> {
        let value = match std::env::var(ENVIRONMENT_OVERRIDE_VARIABLE) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            // An invalid Unicode value is an invalid configuration, not an absent one.
            Err(std::env::VarError::NotUnicode(_)) => Some(String::new()),
        };
        Self::open_with_environment(
            registry()?,
            embedded_profile()?,
            store.dir(),
            Utc::now(),
            value.as_deref(),
            prune,
        )
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .expect("feature flags lock")
            .snapshot
            .clone()
    }
    /// Read one decision without cloning the public snapshot.
    pub(crate) fn is_enabled(&self, id: &str) -> bool {
        self.state
            .lock()
            .map(|state| state.snapshot.is_enabled(id))
            .unwrap_or(false)
    }
    /// The daemon owns all mutations; persistence completes before the live decision changes.
    pub fn set_local_override(
        &self,
        id: &str,
        value: Option<bool>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Snapshot> {
        anyhow::ensure!(
            local::allowed(&self.registry, &self.profile, id),
            "feature-local-override-unavailable"
        );
        let mut s = self.state.lock().expect("feature flags lock");
        anyhow::ensure!(!s.local_unreadable, LOCAL_OVERRIDES_INVALID);
        let mut updated = s.local_overrides.clone();
        match value {
            Some(value) => {
                updated.insert(id.to_owned(), value);
            }
            None => {
                updated.remove(id);
            }
        }
        if updated != s.local_overrides {
            self.local.save(&updated)?;
            s.local_overrides = updated;
        }
        let source = s.snapshot.source.clone();
        self.recompute(&mut s, now, &source);
        Ok(s.snapshot.clone())
    }
    /// Whether a present local choices file failed validation at startup;
    /// `set_local_override` refuses until it is fixed or removed and the
    /// daemon restarts.
    pub fn local_overrides_unreadable(&self) -> bool {
        self.state
            .lock()
            .expect("feature flags lock")
            .local_unreadable
    }
    pub fn request_refresh(&self) {
        self.wake.notify_one();
    }
    pub fn has_flags(&self) -> bool {
        !self.registry.flags.is_empty()
    }
    pub fn environment(&self) -> Environment {
        self.profile.environment
    }
    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .expect("feature flags lock")
            .snapshot
            .generation
    }
    /// When the held manifest stops applying by the monotonic clock, while
    /// one applies. Nothing else about the decision changes with time.
    pub fn next_expiry(&self) -> Option<Instant> {
        let s = self.state.lock().expect("feature flags lock");
        s.manifest.as_ref().filter(|_| !s.expired)?;
        s.deadline
    }
    pub fn accept(&self, raw: &[u8], now: DateTime<Utc>) -> anyhow::Result<()> {
        let (manifest, payload_digest) = parse_signed_manifest(raw, &self.profile, now)?;
        let mut s = self.state.lock().expect("feature flags lock");
        if let Some(mark) = s.watermark.as_ref() {
            anyhow::ensure!(
                manifest.revision >= mark.revision,
                "feature-manifest-rollback"
            );
            if manifest.revision == mark.revision {
                anyhow::ensure!(
                    payload_digest == mark.payload_digest,
                    "feature-manifest-revision-conflict"
                );
                if s.manifest
                    .as_ref()
                    .is_some_and(|held| held.revision == manifest.revision)
                {
                    self.recompute(&mut s, now, "remote");
                    return Ok(());
                }
            }
        }
        let watermark = cache::Watermark {
            revision: manifest.revision,
            payload_digest,
        };
        // Keep persistence and acceptance ordered: a failed atomic write cannot enable a flag.
        self.cache.save_watermark(&watermark)?;
        self.cache.save(raw)?;
        s.deadline = remaining_deadline(&manifest, now);
        s.expired = false;
        s.manifest = Some(manifest);
        s.watermark = Some(watermark);
        self.recompute(&mut s, now, "remote");
        Ok(())
    }
    pub fn tick(&self, now: DateTime<Utc>) {
        let mut s = self.state.lock().expect("feature flags lock");
        let source = s.snapshot.source.clone();
        self.recompute(&mut s, now, &source);
    }
    fn recompute(&self, s: &mut State, now: DateTime<Utc>, source: &str) {
        s.expired |= s.manifest.as_ref().is_some_and(|m| !m.valid_at(now))
            || s.deadline.is_some_and(|d| Instant::now() >= d);
        let manifest = s.manifest.as_ref().filter(|_| !s.expired);
        let mut flags = evaluate(&self.registry, &self.profile, manifest, now);
        local::apply(&mut flags, &s.local_overrides);
        local::apply(&mut flags, &s.snapshot.environment_variable_overrides);
        for (id, d) in &mut flags {
            if d.timing == Timing::NextStart {
                d.enabled = s.latched.get(id).copied().unwrap_or(false);
            }
        }
        let fresh = manifest.is_some();
        let source = if fresh { source } else { "default" };
        if s.local_overrides != s.snapshot.local_overrides
            || flags != s.snapshot.flags
            || fresh != s.snapshot.fresh
            || source != s.snapshot.source
            || s.manifest.as_ref().map(|m| m.revision) != s.snapshot.revision
        {
            s.snapshot.generation = s.snapshot.generation.saturating_add(1);
            s.snapshot.sampled_at = now;
        }
        s.snapshot.flags = flags;
        s.snapshot.local_overrides = s.local_overrides.clone();
        s.snapshot.fresh = fresh;
        s.snapshot.source = source.into();
        s.snapshot.revision = s.manifest.as_ref().map(|m| m.revision);
        s.snapshot.expires_at = s.manifest.as_ref().map(|m| m.expires_at);
    }
}
fn remaining_deadline(manifest: &Manifest, now: DateTime<Utc>) -> Option<Instant> {
    Instant::now().checked_add(
        manifest
            .expires_at
            .signed_duration_since(now)
            .to_std()
            .ok()?,
    )
}
#[cfg(test)]
mod environment_tests;
#[cfg(test)]
mod local_tests;
#[cfg(test)]
mod tests;
/// Signing for tests anywhere in the crate. The key is a fixed throwaway,
/// never a real manifest key, and is pinned only into test profiles.
#[cfg(test)]
pub(crate) mod test_signing {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};
    pub(crate) fn key() -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("fixed test seed")
    }
    /// The profile, also trusting the test key.
    pub(crate) fn pin(mut profile: Profile) -> Profile {
        profile
            .signing_keys
            .insert(hex::encode(key().public_key().as_ref()));
        profile
    }
    /// The served document for `payload`, signed with the test key.
    pub(crate) fn sign(payload: &[u8]) -> Vec<u8> {
        let key = key();
        let public: [u8; 32] = key.public_key().as_ref().try_into().expect("32 bytes");
        assemble_signed_manifest(
            String::from_utf8(payload.to_vec()).expect("UTF-8 manifest"),
            vec![ManifestSignature {
                key_id: key_id_for(&public),
                signature: hex::encode(key.sign(&signing_input(payload)).as_ref()),
            }],
        )
    }
}
