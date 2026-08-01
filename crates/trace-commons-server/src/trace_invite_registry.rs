//! In-process invite registry: cache, invalidation, and code generation.
//!
//! The cache is a latency optimization and never a correctness boundary.
//! Expiry, revocation, and use-count are re-checked inside the redemption
//! transaction, so a revoke racing a redemption is resolved by the database.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rand::Rng;
use rand::rngs::OsRng;

/// Unambiguous invite-code alphabet. Matches the operator runbook's
/// `tr -dc 'A-Z2-9'`: no 0/O, no 1/I/L.
const INVITE_CODE_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ23456789";
const INVITE_CODE_LEN: usize = 16;

/// How the redeemed tenant is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteTenantMode {
    /// Use `fixed_tenant_id` verbatim. Imported pilot invites.
    Fixed,
    /// Derive from `tenant_template_id` plus the redeeming user subject.
    Derived,
}

#[derive(Debug, Clone)]
pub struct InviteEntry {
    pub invite_subject_hash: String,
    pub policy_label: String,
    pub tenant_mode: InviteTenantMode,
    pub fixed_tenant_id: Option<String>,
    pub tenant_template_id: Option<String>,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub max_uses: u32,
    pub expires_at: Option<DateTime<Utc>>,
    pub issuance_source: String,
    pub issued_by_label: Option<String>,
    pub credential_binding_hash: Option<String>,
    pub note_label: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl InviteEntry {
    /// Live means neither revoked nor past expiry. Checked in the cache for
    /// latency and re-checked in the redemption transaction for correctness.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|exp| exp > now)
    }
}

#[derive(Debug)]
pub enum InviteRegistryError {
    /// Cache is older than `max_stale` and has not reloaded. Fail closed.
    Stale {
        age_seconds: u64,
    },
    Backend(String),
}

impl std::fmt::Display for InviteRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale { age_seconds } => {
                write!(f, "invite registry cache stale ({age_seconds}s)")
            }
            // Never interpolate a connection string or row content here.
            Self::Backend(label) => write!(f, "invite registry backend error: {label}"),
        }
    }
}

impl std::error::Error for InviteRegistryError {}

#[derive(Debug, Clone)]
pub struct InviteRegistryStatus {
    pub live: usize,
    pub cache_age_seconds: u64,
    pub stale: bool,
    pub max_stale_seconds: u64,
}

pub trait InviteRegistry: Send + Sync {
    fn lookup(&self, invite_subject_hash: &str)
    -> Result<Option<InviteEntry>, InviteRegistryError>;
    fn note_write(&self, entry: InviteEntry);
    fn note_revoke(&self, invite_subject_hash: &str);
    fn status(&self) -> InviteRegistryStatus;
}

struct CacheInner {
    entries: HashMap<String, InviteEntry>,
    loaded_at: Instant,
}

/// Process-local cache of live invites. Restart-resets, exactly like the
/// allowlist `DenialCounter`; the database is the durable backstop.
pub struct InviteCache {
    inner: RwLock<CacheInner>,
    max_stale: Duration,
}

impl InviteCache {
    pub fn new(max_stale: Duration) -> Self {
        Self {
            inner: RwLock::new(CacheInner {
                entries: HashMap::new(),
                // Treat a never-loaded cache as maximally stale so lookups
                // fail closed until the first refresh succeeds.
                loaded_at: Instant::now() - max_stale - Duration::from_secs(1),
            }),
            max_stale,
        }
    }

    /// Install a fresh snapshot from the registry pool.
    pub fn replace_all(&self, entries: Vec<InviteEntry>, loaded_at: Instant) {
        let mut inner = self.inner.write().expect("InviteCache poisoned");
        inner.entries = entries
            .into_iter()
            .map(|e| (e.invite_subject_hash.clone(), e))
            .collect();
        inner.loaded_at = loaded_at;
    }

    pub fn lookup(
        &self,
        invite_subject_hash: &str,
    ) -> Result<Option<InviteEntry>, InviteRegistryError> {
        let inner = self.inner.read().expect("InviteCache poisoned");
        let age = inner.loaded_at.elapsed();
        // `>` not `>=`, matching FileAllowlistSource's staleness comparison
        // exactly so the two surfaces agree at sub-second precision.
        if age > self.max_stale {
            return Err(InviteRegistryError::Stale {
                age_seconds: age.as_secs(),
            });
        }
        let now = Utc::now();
        Ok(inner
            .entries
            .get(invite_subject_hash)
            .filter(|e| e.is_live(now))
            .cloned())
    }

    /// Make a freshly minted invite visible with no refresh window. Called
    /// immediately after the admin write commits.
    pub fn note_write(&self, entry: InviteEntry) {
        let mut inner = self.inner.write().expect("InviteCache poisoned");
        inner
            .entries
            .insert(entry.invite_subject_hash.clone(), entry);
    }

    pub fn note_revoke(&self, invite_subject_hash: &str) {
        let mut inner = self.inner.write().expect("InviteCache poisoned");
        inner.entries.remove(invite_subject_hash);
    }

    pub fn status(&self) -> InviteRegistryStatus {
        let inner = self.inner.read().expect("InviteCache poisoned");
        let age = inner.loaded_at.elapsed();
        let now = Utc::now();
        InviteRegistryStatus {
            live: inner.entries.values().filter(|e| e.is_live(now)).count(),
            cache_age_seconds: age.as_secs(),
            stale: age > self.max_stale,
            max_stale_seconds: self.max_stale.as_secs(),
        }
    }
}

/// Generate an invite code from the OS CSPRNG. The raw code exists in exactly
/// one admin response body and is never stored, logged, or retrievable
/// afterward; only `hash_invite_code()` of it reaches the database.
pub fn generate_invite_code() -> String {
    let mut rng = OsRng;
    (0..INVITE_CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..INVITE_CODE_ALPHABET.len());
            INVITE_CODE_ALPHABET[idx] as char
        })
        .collect()
}

use std::path::Path;
use std::sync::Arc;

use crate::db::postgres::PgBackend;
use crate::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use crate::trace_upload_claim_allowlist::AllowlistFile;

#[derive(Debug, Clone, Default)]
pub struct ImportSummary {
    pub imported: usize,
    pub already_present: usize,
    pub skipped_non_invite: usize,
}

/// One-time migration of file invite entries into the database. Idempotent on
/// `invite_subject_hash`, so re-running after a partial failure is safe.
/// Instance entries stay in the file and are counted, not imported.
///
/// Reports counts only: no raw codes, tenant secrets, or note text.
pub async fn import_file_invites(
    backend: &PgBackend,
    path: &Path,
    policy_label: &str,
) -> anyhow::Result<ImportSummary> {
    let raw = std::fs::read_to_string(path)?;
    let file: AllowlistFile = serde_json::from_str(&raw)?;
    if file.version != 1 {
        anyhow::bail!(
            "PilotAllowlistMalformed: unsupported version {} (expected 1)",
            file.version
        );
    }

    let mut summary = ImportSummary::default();
    for entry in file.entries {
        if entry.kind != "invite" {
            summary.skipped_non_invite += 1;
            continue;
        }
        let (Some(subject_hash), Some(tenant_id)) = (entry.subject_hash, entry.tenant_id) else {
            anyhow::bail!(
                "PilotAllowlistMalformed: invite entry missing subject_hash or tenant_id"
            );
        };
        let write = InviteGrantWrite {
            invite_subject_hash: subject_hash,
            policy_label: policy_label.to_string(),
            tenant_mode: InviteTenantMode::Fixed,
            fixed_tenant_id: Some(tenant_id),
            tenant_template_id: None,
            policy_version: "v1".to_string(),
            // Deliberately empty: the file format never carried per-invite
            // grant fields, so imported invites fall back to the
            // process-wide onboarding defaults (see
            // resolve_onboarding_scope_override in db/postgres.rs), which
            // preserves today's behavior exactly.
            allowed_consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            max_uses: entry.max_uses,
            expires_at: None,
            issuance_source: "import:file".to_string(),
            issued_by_label: None,
            credential_binding_hash: None,
            note_label: entry.note_label,
        };
        match backend.insert_invite_grant(write).await? {
            InviteGrantInsertOutcome::Inserted => summary.imported += 1,
            InviteGrantInsertOutcome::AlreadyExists => summary.already_present += 1,
            InviteGrantInsertOutcome::CredentialAlreadyBound => {
                anyhow::bail!("unexpected credential binding conflict during file import");
            }
        }
    }
    Ok(summary)
}

/// DB-backed invite registry. The cache is refreshed on a timer from the
/// narrow registry pool and invalidated synchronously by the admin write
/// path, so a minted code is redeemable in the same instant.
pub struct DbInviteRegistry {
    backend: Arc<PgBackend>,
    cache: InviteCache,
    refresh_interval: Duration,
}

impl DbInviteRegistry {
    /// Warms the cache once before returning. A failed warm is an error: the
    /// issuer must not come up believing it has a usable registry.
    pub async fn new(
        backend: Arc<PgBackend>,
        refresh_interval: Duration,
        max_stale: Duration,
    ) -> Result<Self, InviteRegistryError> {
        let registry = Self {
            backend,
            cache: InviteCache::new(max_stale),
            refresh_interval,
        };
        registry.refresh_once().await?;
        Ok(registry)
    }

    /// Reload every live invite. Returns the count loaded.
    pub async fn refresh_once(&self) -> Result<usize, InviteRegistryError> {
        let entries = self
            .backend
            .list_invite_grants()
            .await
            // Label only. Never let a connection string or row content reach
            // this string; it surfaces in operator-visible status output.
            .map_err(|_| {
                InviteRegistryError::Backend("invite-registry-query-failed".to_string())
            })?;
        let count = entries.len();
        self.cache.replace_all(entries, Instant::now());
        Ok(count)
    }

    /// Background refresh. A failed refresh leaves the previous snapshot in
    /// place and lets it age into staleness, which then fails closed —
    /// matching FileAllowlistSource's posture exactly.
    pub fn spawn_refresh_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = self.refresh_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // the first tick fires immediately; skip it
            loop {
                ticker.tick().await;
                let _ = self.refresh_once().await;
            }
        })
    }
}

impl InviteRegistry for DbInviteRegistry {
    fn lookup(
        &self,
        invite_subject_hash: &str,
    ) -> Result<Option<InviteEntry>, InviteRegistryError> {
        self.cache.lookup(invite_subject_hash)
    }

    fn note_write(&self, entry: InviteEntry) {
        self.cache.note_write(entry);
    }

    fn note_revoke(&self, invite_subject_hash: &str) {
        self.cache.note_revoke(invite_subject_hash);
    }

    fn status(&self) -> InviteRegistryStatus {
        self.cache.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, Utc};
    use std::time::{Duration, Instant};

    fn entry(hash: &str) -> InviteEntry {
        InviteEntry {
            invite_subject_hash: hash.to_string(),
            policy_label: "test-pool".to_string(),
            tenant_mode: InviteTenantMode::Derived,
            fixed_tenant_id: None,
            tenant_template_id: Some("tmpl-1".to_string()),
            policy_version: "v1".to_string(),
            allowed_consent_scopes: vec!["model_training".to_string()],
            allowed_uses: vec!["research".to_string()],
            max_uses: 3,
            expires_at: None,
            issuance_source: "operator".to_string(),
            issued_by_label: None,
            credential_binding_hash: None,
            note_label: None,
            revoked_at: None,
        }
    }

    #[test]
    fn note_write_makes_an_invite_immediately_visible() {
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(Vec::new(), Instant::now());
        assert!(cache.lookup("sha256:aa").unwrap().is_none());

        cache.note_write(entry("sha256:aa"));

        let found = cache.lookup("sha256:aa").unwrap();
        assert!(
            found.is_some(),
            "a minted invite must be redeemable with no refresh window"
        );
    }

    #[test]
    fn note_revoke_immediately_removes_an_invite() {
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(vec![entry("sha256:aa")], Instant::now());
        assert!(cache.lookup("sha256:aa").unwrap().is_some());

        cache.note_revoke("sha256:aa");

        assert!(cache.lookup("sha256:aa").unwrap().is_none());
    }

    #[test]
    fn expired_invites_are_not_returned() {
        let cache = InviteCache::new(Duration::from_secs(60));
        let mut expired = entry("sha256:aa");
        expired.expires_at = Some(Utc::now() - ChronoDuration::seconds(1));
        cache.replace_all(vec![expired], Instant::now());

        assert!(cache.lookup("sha256:aa").unwrap().is_none());
    }

    #[test]
    fn lookup_fails_closed_once_the_cache_is_stale() {
        let cache = InviteCache::new(Duration::from_secs(60));
        // Loaded 61 seconds ago: past max_stale.
        cache.replace_all(
            vec![entry("sha256:aa")],
            Instant::now() - Duration::from_secs(61),
        );

        match cache.lookup("sha256:aa") {
            Err(InviteRegistryError::Stale { age_seconds }) => {
                assert!(age_seconds >= 61);
            }
            other => panic!("stale cache must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn staleness_boundary_matches_the_allowlist_source() {
        // FileAllowlistSource treats `age > max_stale` as stale, so exactly
        // max_stale is still fresh. Keep the two in agreement.
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(
            vec![entry("sha256:aa")],
            Instant::now() - Duration::from_secs(59),
        );
        assert!(cache.lookup("sha256:aa").is_ok());
    }

    #[test]
    fn status_reports_live_count_and_staleness() {
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(vec![entry("sha256:aa"), entry("sha256:bb")], Instant::now());
        let status = cache.status();
        assert_eq!(status.live, 2);
        assert!(!status.stale);
        assert_eq!(status.max_stale_seconds, 60);
    }

    #[test]
    fn generated_codes_use_the_runbook_alphabet_and_length() {
        let code = generate_invite_code();
        assert_eq!(code.len(), 16);
        assert!(
            code.chars()
                .all(|c| c.is_ascii_uppercase() || ('2'..='9').contains(&c)),
            "codes must stay within [A-Z2-9]: {code}"
        );
    }

    #[test]
    fn generated_codes_do_not_repeat() {
        let a = generate_invite_code();
        let b = generate_invite_code();
        assert_ne!(a, b);
    }

    #[test]
    fn a_never_refreshed_registry_fails_closed() {
        // A cache that has never loaded must be treated as maximally stale,
        // so a registry whose first refresh failed cannot silently authorize
        // nothing-is-valid as everything-is-invalid-but-fresh.
        let cache = InviteCache::new(Duration::from_secs(60));
        match cache.lookup("sha256:aa") {
            Err(InviteRegistryError::Stale { .. }) => {}
            other => panic!("unloaded cache must be stale, got {other:?}"),
        }
    }
}
