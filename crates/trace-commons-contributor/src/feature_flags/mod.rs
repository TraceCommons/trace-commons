//! Remote configuration is advisory product policy, never an authorization grant.
mod cache;
mod runtime;
mod transport;
use chrono::{DateTime, Utc};
pub use runtime::run;
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
}
impl Snapshot {
    pub fn is_enabled(&self, id: &str) -> bool {
        self.flags.get(id).is_some_and(|d| d.enabled)
    }
}
struct State {
    manifest: Option<Manifest>,
    raw: Vec<u8>,
    deadline: Option<Instant>,
    expired: bool,
    latched: BTreeMap<String, bool>,
    snapshot: Snapshot,
}
pub struct Runtime {
    registry: Registry,
    profile: Profile,
    cache: cache::Cache,
    state: Mutex<State>,
    pub(crate) wake: tokio::sync::Notify,
    changed: tokio::sync::watch::Sender<u64>,
}
impl Runtime {
    pub fn open(
        registry: Registry,
        profile: Profile,
        dir: &Path,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Self> {
        registry.validate(&profile)?;
        let identity = profile.digest()?;
        let cache = cache::Cache::new(dir, &identity);
        let cached = cache.load(profile.environment);
        let (manifest, raw) = cached.map(|(m, r)| (Some(m), r)).unwrap_or_default();
        let expired = manifest.as_ref().is_some_and(|m| !m.valid_at(now));
        let flags = evaluate(
            &registry,
            &profile,
            manifest.as_ref().filter(|_| !expired),
            now,
        );
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
        };
        Ok(Self {
            registry,
            profile,
            cache,
            state: Mutex::new(State {
                manifest,
                raw,
                deadline,
                expired,
                latched,
                snapshot,
            }),
            wake: tokio::sync::Notify::new(),
            changed: tokio::sync::watch::channel(1).0,
        })
    }
    pub fn for_store(store: &crate::config::ConfigStore) -> anyhow::Result<Self> {
        Self::open(registry()?, embedded_profile()?, store.dir(), Utc::now())
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .expect("feature flags lock")
            .snapshot
            .clone()
    }
    /// Keep a product policy decision ordered with a synchronous publication.
    /// Callers must acquire this guard before their feature's lifecycle lock.
    pub(crate) fn with_snapshot<R>(&self, publish: impl FnOnce(&Snapshot) -> R) -> R {
        let state = self.state.lock().expect("feature flags lock");
        publish(&state.snapshot)
    }
    #[cfg(test)]
    pub(crate) fn try_snapshot(&self) -> Option<Snapshot> {
        self.state
            .try_lock()
            .ok()
            .map(|state| state.snapshot.clone())
    }
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
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
    pub fn accept(&self, raw: &[u8], now: DateTime<Utc>) -> anyhow::Result<()> {
        let manifest = parse_manifest(raw, self.profile.environment, now)?;
        let mut s = self.state.lock().expect("feature flags lock");
        if let Some(old) = s.manifest.as_ref() {
            anyhow::ensure!(
                manifest.revision >= old.revision,
                "feature-manifest-rollback"
            );
            if manifest.revision == old.revision {
                anyhow::ensure!(raw == s.raw, "feature-manifest-revision-conflict");
                self.recompute(&mut s, now, "remote");
                return Ok(());
            }
        }
        // Keep persistence and acceptance ordered: a failed atomic write cannot enable a flag.
        self.cache.save(raw)?;
        s.deadline = remaining_deadline(&manifest, now);
        s.expired = false;
        s.manifest = Some(manifest);
        s.raw = raw.to_vec();
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
        for (id, d) in &mut flags {
            if d.timing == Timing::NextStart {
                d.enabled = s.latched.get(id).copied().unwrap_or(false);
            }
        }
        let fresh = manifest.is_some();
        let source = if fresh { source } else { "default" };
        if flags != s.snapshot.flags
            || fresh != s.snapshot.fresh
            || source != s.snapshot.source
            || s.manifest.as_ref().map(|m| m.revision) != s.snapshot.revision
        {
            s.snapshot.generation = s.snapshot.generation.saturating_add(1);
            s.snapshot.sampled_at = now;
        }
        s.snapshot.flags = flags;
        s.snapshot.fresh = fresh;
        s.snapshot.source = source.into();
        s.snapshot.revision = s.manifest.as_ref().map(|m| m.revision);
        s.snapshot.expires_at = s.manifest.as_ref().map(|m| m.expires_at);
        self.changed.send_if_modified(|generation| {
            if *generation == s.snapshot.generation {
                return false;
            }
            *generation = s.snapshot.generation;
            true
        });
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
mod tests;
