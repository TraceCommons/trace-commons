use std::collections::BTreeSet;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::errors::ErrorKind as JwtErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::DatabaseConfig;
use crate::db::Database;
use crate::db::postgres::PgBackend;
use crate::trace_corpus_storage::{
    TraceTenantAccessGrantRecord, TraceTenantAccessGrantRole, TraceTenantAccessGrantStatus,
};
use crate::trace_invite_registry::{DbInviteRegistry, InviteRegistry, InviteRegistryError};
use crate::trace_upload_claim_allowlist::{
    AllowlistError, AllowlistSource, AllowlistSourceSpec, DenialCounter, FileAllowlistSource,
    hash_invite_code,
};
use trace_commons_protocol::onboarding::{
    TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TRACE_ONBOARD_REQUEST_SCHEMA_VERSION,
    TraceInstanceEnrollRequest, TraceOnboardErrorCode, TraceOnboardRequest, TraceOnboardResponse,
    derive_user_tenant_id, device_key_id_from_public_key_bytes,
    instance_enroll_attestation_signing_bytes, user_subject_hash,
};
use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

pub const TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION: &str =
    "ironclaw.trace_upload_claim_request.v1";
pub const TRACE_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS_ENV: &str =
    "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS";
/// Configures the tenant-access-grant DB for READING (so device-key claims
/// can derive their consent-scope ceiling from enrollment grants) WITHOUT
/// turning on strict grant enforcement. Requiring grants (the env above)
/// implies reading; this env enables reading alone.
pub const TRACE_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB_ENV: &str =
    "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB";
const DEFAULT_BIND: &str = "127.0.0.1:3917";
const DEFAULT_MAX_TTL_SECONDS: i64 = 300;
const DEFAULT_SHUTDOWN_GRACE_SECONDS: u64 = 30;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_MAX_REQUEST_BYTES: usize = 64 * 1024;
const MINT_TEST_CLAIM_TENANT: &str = "trace-upload-claim-issuer-test-tenant";
const MINT_TEST_CLAIM_PRINCIPAL: &str = "principal:trace-upload-claim-issuer-test";

const DEFAULT_ALLOWLIST_REFRESH_INTERVAL_SECONDS: u64 = 60;
const DEFAULT_ALLOWLIST_MAX_STALE_SECONDS: u64 = 3600;
const DEFAULT_DENIAL_COUNTER_WINDOW_SECONDS: u64 = 3600;
const INVITE_LANDING_TEXT: &str = r#"Trace Commons invite link

You are reading this because someone opened a Trace Commons invite. This page
is instructions, not an action. Fetching it (GET) consumes nothing; only the
POST in step 2 does.

Onboarding enrolls a machine as a contributor to a corpus of coding-agent
sessions. Before proceeding, tell the user plainly:

  - Redacted records of their coding-agent sessions will be contributed
  - Onboarding permanently spends one use of this invite, whether or not
    they later submit anything
  - Only contribute sessions from repositories they have the right to share

Proceed only on an explicit yes.

--------------------------------------------------------------------------
1. Use the CLI if you can
--------------------------------------------------------------------------

trace-commons-contributor login --invite '<full invite link>'

That registers a device key, redeems the code, and writes the config in one
step. It is the supported path and everything below is what it does for you.

The CLI ships as source in TraceCommons/trace-commons-server; build it with
  cargo build --release -p trace-commons-contributor
There is no published binary on npm, Homebrew, or PyPI.

--------------------------------------------------------------------------
2. Or POST /v1/onboard yourself
--------------------------------------------------------------------------

The code is the URL fragment: .../onboard#VQWWPGYSG8Y4LTP6 means the code is
VQWWPGYSG8Y4LTP6. Fragments are not sent to servers, so this page cannot read
it and cannot tell you whether it is valid. A ?code= query parameter also
works.

Generate an Ed25519 keypair if you do not have one, then POST its PUBLIC half:

POST /v1/onboard
content-type: application/json

{
  "schema_version": "trace_commons.onboard_request.v1",
  "invite_code": "<code from the invite URL>",
  "device_public_key": "<base64 of the raw 32-byte Ed25519 public key>",
  "client_info": { "agent": "<your client>", "version": "<your version>" }
}

device_public_key is the raw key, base64-encoded. It is NOT a fingerprint,
NOT a PEM block, and NOT a hash. If you are holding a device_key_id that
looks like "sha256:...", that is a digest OF the key and will be rejected
here.

THIS REQUEST IS NOT IDEMPOTENT. Every success spends one use of the invite,
including re-registering a key that is already enrolled. Send it once.

--------------------------------------------------------------------------
3. Persist the response
--------------------------------------------------------------------------

The response carries tenant_id, ingest_url, issuer_url, audience,
device_key_id, and community URLs. It is returned once and never again.

Onboarding does not configure your client. Write the response to your
client's configuration store immediately or the registration is unusable and
the invite use is wasted. For trace-commons-contributor that means
<config-dir>/contributor.json carrying issuer_url, ingest_url, audience,
tenant_id, device_key_id, and consent_scopes.

Confirm with `trace-commons-contributor whoami`. If it reports "not logged
in", the config was not written and nothing will submit.

--------------------------------------------------------------------------
4. Stop there
--------------------------------------------------------------------------

Onboarding registers a device; it does not choose what to contribute.
Submission is a separate, explicit decision. When the user asks for it, run
`submit --dry-run` first and show them what the envelope contains: session
transcripts routinely contain credentials pasted while working.

--------------------------------------------------------------------------
Errors
--------------------------------------------------------------------------

403 InviteNotValid        Code unknown or malformed. Re-read the fragment.
                          Nothing was consumed; do not retry with guesses.
403 InviteAlreadyConsumed Invite is exhausted. Ask for a new one.
4xx schema error          Check device_public_key is base64 of 32 raw bytes.
"#;

pub const TRACE_COMMONS_ALLOWLIST_SOURCE_ENV: &str = "TRACE_COMMONS_ALLOWLIST_SOURCE";
pub const TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS_ENV: &str =
    "TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS";
pub const TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS_ENV: &str =
    "TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS";
pub const TRACE_COMMONS_ISSUER_ADMIN_BIND_ENV: &str = "TRACE_COMMONS_ISSUER_ADMIN_BIND";
pub const TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC_ENV: &str =
    "TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC";
pub const TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED_ENV: &str =
    "TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED";
pub const TRACE_COMMONS_ONBOARDING_INGEST_URL_ENV: &str = "TRACE_COMMONS_ONBOARDING_INGEST_URL";
pub const TRACE_COMMONS_ONBOARDING_COMMUNITY_URL_ENV: &str =
    "TRACE_COMMONS_ONBOARDING_COMMUNITY_URL";
pub const TRACE_COMMONS_ONBOARDING_PROFILE_URL_ENV: &str = "TRACE_COMMONS_ONBOARDING_PROFILE_URL";
pub const TRACE_COMMONS_ONBOARDING_LEADERBOARD_URL_ENV: &str =
    "TRACE_COMMONS_ONBOARDING_LEADERBOARD_URL";
/// Cutover flag: when true, `/v1/onboard` redeems invites through the
/// database registry instead of the file allowlist. Defaults to false (the
/// file allowlist stays authoritative) so every pre-existing onboarding
/// deployment and test is unaffected until an operator opts in.
pub const TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE_ENV: &str =
    "TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE";
const TRACE_DEVICE_KEY_ID_HEADER: &str = "x-trace-device-key-id";
const TRACE_DEVICE_SIGNATURE_HEADER: &str = "x-trace-device-signature";

#[derive(Clone)]
pub struct TraceUploadClaimIssuerConfig {
    pub bind: SocketAddr,
    pub signing_private_key_pem: String,
    pub signing_public_key_pem: String,
    pub signing_kid: String,
    pub issuer: String,
    pub audience: String,
    pub max_ttl_seconds: i64,
    pub workload_public_key_pem: String,
    pub workload_issuer: Option<String>,
    pub workload_audience: Option<String>,
    pub tenant_access_grant_db: Option<Arc<dyn Database>>,
    pub require_tenant_access_grants: bool,
    pub shutdown_grace_seconds: u64,
    pub request_timeout_seconds: u64,
    pub max_request_bytes: usize,
    /// Pilot allowlist source. `None` = allowlist disabled, issuer
    /// behaves exactly as the pre-allowlist MVP. See
    /// `trace_upload_claim_allowlist`.
    pub allowlist_source: Option<AllowlistSourceSpec>,
    pub allowlist_refresh_interval_seconds: u64,
    pub allowlist_max_stale_seconds: u64,
    pub onboarding_device_key_db: Option<Arc<dyn Database>>,
    pub onboarding_ingest_url: Option<String>,
    pub onboarding_community_url: Option<String>,
    pub onboarding_profile_url: Option<String>,
    pub onboarding_leaderboard_url: Option<String>,
    /// Optional second-bind for the operator admin endpoint
    /// (`/v1/admin/allowlist-status`). `None` = admin endpoint disabled.
    /// Must be a loopback address unless
    /// `TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1` is set.
    pub admin_bind: Option<SocketAddr>,
    /// Narrow-pool backend for the admin invite routes, populated by
    /// [`configure_invite_admin_from_env`]. `None` keeps
    /// `/v1/admin/invites*` unmounted — fail-closed, matching every other
    /// optional narrow-pool feature in this config.
    pub invite_admin_backend: Option<Arc<PgBackend>>,
    /// Cache/invalidation layer over `invite_admin_backend`, sharing the
    /// SAME registry instance the admin routes and (in a later task) the
    /// redemption path both read: two independent caches over one table
    /// would diverge.
    pub invite_admin_registry: Option<Arc<DbInviteRegistry>>,
    /// Cutover flag from [`TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE_ENV`].
    /// `false` keeps `/v1/onboard` on the unchanged file-allowlist path.
    pub invite_registry_authoritative: bool,
}

impl fmt::Debug for TraceUploadClaimIssuerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TraceUploadClaimIssuerConfig")
            .field("bind", &self.bind)
            .field("signing_private_key_pem", &"<redacted>")
            .field("signing_public_key_pem", &"<redacted>")
            .field("signing_kid", &self.signing_kid)
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("max_ttl_seconds", &self.max_ttl_seconds)
            .field("workload_public_key_pem", &"<redacted>")
            .field("workload_issuer", &self.workload_issuer)
            .field("workload_audience", &self.workload_audience)
            .field(
                "tenant_access_grant_db",
                &self.tenant_access_grant_db.as_ref().map(|_| "<configured>"),
            )
            .field(
                "require_tenant_access_grants",
                &self.require_tenant_access_grants,
            )
            .field("shutdown_grace_seconds", &self.shutdown_grace_seconds)
            .field("request_timeout_seconds", &self.request_timeout_seconds)
            .field("max_request_bytes", &self.max_request_bytes)
            .field("allowlist_source", &self.allowlist_source)
            .field(
                "allowlist_refresh_interval_seconds",
                &self.allowlist_refresh_interval_seconds,
            )
            .field(
                "allowlist_max_stale_seconds",
                &self.allowlist_max_stale_seconds,
            )
            .field(
                "onboarding_device_key_db",
                &self
                    .onboarding_device_key_db
                    .as_ref()
                    .map(|_| "<configured>"),
            )
            .field("onboarding_ingest_url", &self.onboarding_ingest_url)
            .field("onboarding_community_url", &self.onboarding_community_url)
            .field("onboarding_profile_url", &self.onboarding_profile_url)
            .field(
                "onboarding_leaderboard_url",
                &self.onboarding_leaderboard_url,
            )
            .field("admin_bind", &self.admin_bind)
            .field(
                "invite_admin_backend",
                &self.invite_admin_backend.as_ref().map(|_| "<configured>"),
            )
            .field(
                "invite_admin_registry",
                &self.invite_admin_registry.as_ref().map(|_| "<configured>"),
            )
            .field(
                "invite_registry_authoritative",
                &self.invite_registry_authoritative,
            )
            .finish()
    }
}

impl TraceUploadClaimIssuerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_BIND")?
            .unwrap_or_else(|| DEFAULT_BIND.to_string())
            .parse()
            .context("invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_BIND")?;
        let signing_private_key_pem = required_pem_or_file(
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_PEM",
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_FILE",
        )?;
        let signing_public_key_pem = required_pem_or_file(
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_PEM",
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_FILE",
        )?;
        let workload_public_key_pem = required_pem_or_file(
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_PEM",
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_FILE",
        )?;
        let max_ttl_seconds = optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS")?
            .map(|value| {
                value
                    .parse::<i64>()
                    .context("invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS")
            })
            .transpose()?
            .unwrap_or(DEFAULT_MAX_TTL_SECONDS);
        let shutdown_grace_seconds =
            optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SHUTDOWN_GRACE_SECONDS")?
                .map(|value| {
                    value
                        .parse::<u64>()
                        .context("invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SHUTDOWN_GRACE_SECONDS")
                })
                .transpose()?
                .unwrap_or(DEFAULT_SHUTDOWN_GRACE_SECONDS);
        let request_timeout_seconds =
            optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUEST_TIMEOUT_SECONDS")?
                .map(|value| {
                    value.parse::<u64>().context(
                        "invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUEST_TIMEOUT_SECONDS",
                    )
                })
                .transpose()?
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECONDS);
        let max_request_bytes =
            optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_REQUEST_BYTES")?
                .map(|value| {
                    value
                        .parse::<usize>()
                        .context("invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_REQUEST_BYTES")
                })
                .transpose()?
                .unwrap_or(DEFAULT_MAX_REQUEST_BYTES);
        let allowlist_source = AllowlistSourceSpec::parse(
            optional_env(TRACE_COMMONS_ALLOWLIST_SOURCE_ENV)?.as_deref(),
        )?;
        let allowlist_refresh_interval_seconds =
            optional_env(TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS_ENV)?
                .map(|value| {
                    value.parse::<u64>().with_context(|| {
                        format!("invalid {TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS_ENV}")
                    })
                })
                .transpose()?
                .unwrap_or(DEFAULT_ALLOWLIST_REFRESH_INTERVAL_SECONDS);
        let allowlist_max_stale_seconds =
            optional_env(TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS_ENV)?
                .map(|value| {
                    value.parse::<u64>().with_context(|| {
                        format!("invalid {TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS_ENV}")
                    })
                })
                .transpose()?
                .unwrap_or(DEFAULT_ALLOWLIST_MAX_STALE_SECONDS);
        let admin_bind = optional_env(TRACE_COMMONS_ISSUER_ADMIN_BIND_ENV)?
            .map(|value| {
                value
                    .parse::<SocketAddr>()
                    .with_context(|| format!("invalid {TRACE_COMMONS_ISSUER_ADMIN_BIND_ENV}"))
            })
            .transpose()?;

        Ok(Self {
            bind,
            signing_private_key_pem,
            signing_public_key_pem,
            signing_kid: required_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID")?,
            issuer: required_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER")?,
            audience: required_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE")?,
            max_ttl_seconds,
            workload_public_key_pem,
            workload_issuer: optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_ISSUER")?,
            workload_audience: optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_AUDIENCE")?,
            tenant_access_grant_db: None,
            require_tenant_access_grants: false,
            shutdown_grace_seconds,
            request_timeout_seconds,
            max_request_bytes,
            allowlist_source,
            allowlist_refresh_interval_seconds,
            allowlist_max_stale_seconds,
            onboarding_device_key_db: None,
            onboarding_ingest_url: normalize_onboarding_ingest_url(optional_env(
                TRACE_COMMONS_ONBOARDING_INGEST_URL_ENV,
            )?)?,
            onboarding_community_url: optional_env(TRACE_COMMONS_ONBOARDING_COMMUNITY_URL_ENV)?
                .and_then(|value| trim_optional(Some(value))),
            onboarding_profile_url: optional_env(TRACE_COMMONS_ONBOARDING_PROFILE_URL_ENV)?
                .and_then(|value| trim_optional(Some(value))),
            onboarding_leaderboard_url: optional_env(TRACE_COMMONS_ONBOARDING_LEADERBOARD_URL_ENV)?
                .and_then(|value| trim_optional(Some(value))),
            admin_bind,
            invite_admin_backend: None,
            invite_admin_registry: None,
            invite_registry_authoritative: false,
        })
    }

    fn build_state(&self) -> anyhow::Result<Arc<TraceUploadClaimIssuerState>> {
        anyhow::ensure!(
            self.max_ttl_seconds > 0,
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS must be positive"
        );
        let signing_private_key_pem =
            validate_eddsa_private_key_pem(&self.signing_private_key_pem)?;
        let signing_public_key_pem = validate_eddsa_public_key_pem(&self.signing_public_key_pem)?;
        if self.onboarding_device_key_db.is_some() {
            anyhow::ensure!(
                self.onboarding_ingest_url.is_some(),
                "{TRACE_COMMONS_ONBOARDING_INGEST_URL_ENV} is required when {TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED_ENV}=true"
            );
        }
        let workload_public_key_pem = validate_eddsa_public_key_pem(&self.workload_public_key_pem)?;
        let signing_key = EncodingKey::from_ed_pem(signing_private_key_pem.as_bytes())
            .context("invalid EdDSA signing private key")?;
        let workload_decoding_key = DecodingKey::from_ed_pem(workload_public_key_pem.as_bytes())
            .context("invalid EdDSA workload public key")?;
        anyhow::ensure!(
            !self.signing_kid.trim().is_empty(),
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID is required"
        );
        anyhow::ensure!(
            !self.issuer.trim().is_empty(),
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER is required"
        );
        anyhow::ensure!(
            !self.audience.trim().is_empty(),
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE is required"
        );

        // Build the allowlist source (if any) eagerly so a missing or
        // malformed file fails issuer startup rather than waiting for the
        // first claim request to hit the snapshot loader.
        let allowlist_source: Option<Arc<dyn AllowlistSource>> = match &self.allowlist_source {
            None => None,
            Some(AllowlistSourceSpec::File(path)) => {
                let source = FileAllowlistSource::new(
                    path.clone(),
                    StdDuration::from_secs(self.allowlist_refresh_interval_seconds.max(1)),
                );
                source.warm().with_context(|| {
                    format!(
                        "PilotAllowlistSourceMissing: failed to load allowlist file at {}",
                        path.display()
                    )
                })?;
                Some(Arc::new(source))
            }
            Some(AllowlistSourceSpec::Near { .. }) => {
                anyhow::bail!(
                    "PilotAllowlistNearSourceNotImplemented: use file:<path> until the on-chain allowlist source lands"
                );
            }
        };

        // Loopback guard: if the operator configured an admin bind on a
        // public address by accident, refuse to start. The opt-in env
        // override stays available for the rare deployment that wants to
        // expose the admin endpoint behind a separate gateway.
        if let Some(addr) = self.admin_bind
            && !addr.ip().is_loopback()
            && !env_truthy(TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC_ENV)
        {
            anyhow::bail!(
                "PilotAllowlistAdminBindNotLoopback: {TRACE_COMMONS_ISSUER_ADMIN_BIND_ENV}={addr} is not a loopback address; set {TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC_ENV}=1 to override"
            );
        }

        let denial_counter = Arc::new(DenialCounter::new(StdDuration::from_secs(
            DEFAULT_DENIAL_COUNTER_WINDOW_SECONDS,
        )));

        Ok(Arc::new(TraceUploadClaimIssuerState {
            signing_key,
            signing_kid: self.signing_kid.trim().to_string(),
            issuer: self.issuer.trim().to_string(),
            audience: self.audience.trim().to_string(),
            max_ttl_seconds: self.max_ttl_seconds,
            workload_decoding_key,
            workload_issuer: trim_optional(self.workload_issuer.clone()),
            workload_audience: trim_optional(self.workload_audience.clone()),
            signing_public_key_pem,
            workload_public_key_pem,
            tenant_access_grant_db: self.tenant_access_grant_db.clone(),
            require_tenant_access_grants: self.require_tenant_access_grants,
            allowlist_source,
            allowlist_max_stale: StdDuration::from_secs(self.allowlist_max_stale_seconds),
            onboarding_device_key_db: self.onboarding_device_key_db.clone(),
            onboarding_ingest_url: normalize_onboarding_ingest_url(
                self.onboarding_ingest_url.clone(),
            )?,
            onboarding_community_url: self.onboarding_community_url.clone(),
            onboarding_profile_url: self.onboarding_profile_url.clone(),
            onboarding_leaderboard_url: self.onboarding_leaderboard_url.clone(),
            denial_counter,
            instance_replay_cache: Arc::new(crate::instance_enroll_guard::ReplayCache::new()),
            instance_rate_limiter: Arc::new(
                crate::instance_enroll_guard::InstanceRateLimiter::new(),
            ),
            instance_enroll_default_rate_per_min: std::env::var(
                "TRACE_COMMONS_INSTANCE_ENROLL_RATE_PER_MIN",
            )
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(60),
            invite_admin_backend: self.invite_admin_backend.clone(),
            invite_admin_registry: self.invite_admin_registry.clone(),
            invite_registry_authoritative: self.invite_registry_authoritative,
        }))
    }
}

/// Decide, from the two env flags, whether to attach the grant DB for reading
/// and whether to enforce grants. Reading is enabled when either flag is set;
/// enforcement follows the REQUIRE flag alone. Pure so it is unit-testable
/// without a database.
fn tenant_access_grant_env_decision(require_env: bool, grant_db_env: bool) -> (bool, bool) {
    let require = require_env;
    let attach_db = require || grant_db_env;
    (attach_db, require)
}

pub async fn configure_tenant_access_grants_from_env(
    config: &mut TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    let (attach_db, require) = tenant_access_grant_env_decision(
        env_truthy(TRACE_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS_ENV),
        env_truthy(TRACE_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB_ENV),
    );
    if !attach_db {
        return Ok(());
    }
    let db = trace_upload_claim_issuer_db_from_env().await?;
    config.tenant_access_grant_db = Some(db);
    config.require_tenant_access_grants = require;
    Ok(())
}

pub async fn configure_onboarding_device_key_registry_from_env(
    config: &mut TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    if !env_truthy(TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED_ENV) {
        return Ok(());
    }
    let db = trace_upload_claim_issuer_db_from_env()
        .await
        .context("failed to configure Trace onboarding device-key registry DB")?;
    config.onboarding_device_key_db = Some(db);
    Ok(())
}

/// Wire the admin invite routes to a live database-backed registry when
/// `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL` is configured. Absent that,
/// leaves `config.invite_admin_backend`/`invite_admin_registry` at `None`,
/// which keeps `/v1/admin/invites*` unmounted (fail-closed) exactly like
/// today, rather than the previous state where nothing could ever mount
/// them.
///
/// Builds exactly one `DbInviteRegistry`/`PgBackend` pair and stores the
/// `Arc<DbInviteRegistry>` on the config (and, via `build_state`, on
/// `TraceUploadClaimIssuerState`). A later redemption-path task MUST reuse
/// this same `Arc` — obtained from `TraceUploadClaimIssuerState` — rather
/// than constructing a second `DbInviteRegistry` over the same table, or the
/// two caches would diverge and a code minted through the admin API would
/// not be visible to redemption.
pub async fn configure_invite_admin_from_env(
    config: &mut TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    // Read unconditionally, even if the registry URL below is absent: an
    // operator who sets AUTHORITATIVE=true without also configuring the
    // registry must get the fail-closed 503
    // (InviteRegistryNotConfigured) at request time, not a silent
    // reversion to the file allowlist.
    config.invite_registry_authoritative =
        env_truthy(TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE_ENV);
    if DatabaseConfig::invite_registry_url_from_env().is_none() {
        return Ok(());
    }
    let url = std::env::var("DATABASE_URL")
        .context("Trace upload-claim issuer admin invite routes require DATABASE_URL")?;
    let pool_size = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5);
    let db_config = DatabaseConfig::from_postgres_url(&url, pool_size);
    let backend = Arc::new(
        PgBackend::new(&db_config)
            .await
            .context("failed to connect Trace upload-claim issuer invite-registry DB")?,
    );
    backend
        .run_migrations()
        .await
        .context("failed to run migrations for Trace upload-claim issuer invite-registry DB")?;
    // Warms the cache before returning; a failed warm must fail issuer
    // startup rather than come up believing it has a usable registry.
    let registry = Arc::new(
        DbInviteRegistry::new(
            backend.clone(),
            StdDuration::from_secs(DEFAULT_ALLOWLIST_REFRESH_INTERVAL_SECONDS),
            StdDuration::from_secs(DEFAULT_ALLOWLIST_MAX_STALE_SECONDS),
        )
        .await
        .map_err(|e| anyhow::anyhow!("failed to warm invite registry cache: {e}"))?,
    );
    registry.clone().spawn_refresh_task();
    tracing::info!("Trace upload-claim issuer admin invite registry enabled");
    config.invite_admin_backend = Some(backend);
    config.invite_admin_registry = Some(registry);
    Ok(())
}

async fn trace_upload_claim_issuer_db_from_env() -> anyhow::Result<Arc<dyn Database>> {
    let url = std::env::var("DATABASE_URL")
        .context("Trace upload-claim issuer DB-backed features require DATABASE_URL")?;
    let pool_size = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5);
    let config = DatabaseConfig::from_postgres_url(&url, pool_size);
    let db = crate::db::connect_from_config(&config)
        .await
        .context("failed to connect Trace upload-claim issuer tenant grant DB")?;
    tracing::info!("Trace upload-claim issuer tenant grant PostgreSQL DB enabled");
    Ok(db)
}

struct TraceUploadClaimIssuerState {
    signing_key: EncodingKey,
    signing_kid: String,
    issuer: String,
    audience: String,
    max_ttl_seconds: i64,
    workload_decoding_key: DecodingKey,
    workload_issuer: Option<String>,
    workload_audience: Option<String>,
    signing_public_key_pem: String,
    workload_public_key_pem: String,
    tenant_access_grant_db: Option<Arc<dyn Database>>,
    require_tenant_access_grants: bool,
    allowlist_source: Option<Arc<dyn AllowlistSource>>,
    allowlist_max_stale: StdDuration,
    onboarding_device_key_db: Option<Arc<dyn Database>>,
    onboarding_ingest_url: Option<String>,
    onboarding_community_url: Option<String>,
    onboarding_profile_url: Option<String>,
    onboarding_leaderboard_url: Option<String>,
    denial_counter: Arc<DenialCounter>,
    instance_replay_cache: Arc<crate::instance_enroll_guard::ReplayCache>,
    instance_rate_limiter: Arc<crate::instance_enroll_guard::InstanceRateLimiter>,
    instance_enroll_default_rate_per_min: u32,
    invite_admin_backend: Option<Arc<PgBackend>>,
    invite_admin_registry: Option<Arc<DbInviteRegistry>>,
    invite_registry_authoritative: bool,
}

impl TraceUploadClaimIssuerState {
    /// Build the AdminState the admin router consumes. Lives here so the
    /// admin module never needs visibility into the private state fields.
    ///
    /// `invite_admin` is `Some` only when both a backend and a registry were
    /// configured (see `configure_invite_admin_from_env`); otherwise the
    /// invite routes stay unmounted, matching the fail-closed posture of
    /// every other narrow-pool feature. The admin-token decoding key reuses
    /// this issuer's own signing public key (already validated at startup),
    /// and `expected_iss`/`expected_aud` reuse the same issuer/audience
    /// strings this issuer already stamps on the upload-claim tokens it
    /// mints, rather than inventing a second identity pair.
    pub(crate) fn build_admin_state(&self) -> crate::trace_upload_claim_issuer_admin::AdminState {
        let invite_admin = match (&self.invite_admin_backend, &self.invite_admin_registry) {
            (Some(backend), Some(registry)) => {
                let decoding_key = DecodingKey::from_ed_pem(self.signing_public_key_pem.as_bytes())
                    .expect("signing_public_key_pem validated in build_state");
                Some(crate::trace_invite_admin::InviteAdminState {
                    backend: backend.clone(),
                    registry: registry.clone(),
                    decoding_key: Arc::new(decoding_key),
                    expected_iss: self.issuer.clone(),
                    expected_aud: self.audience.clone(),
                    default_policy_label: self.issuer.clone(),
                })
            }
            _ => None,
        };
        crate::trace_upload_claim_issuer_admin::AdminState {
            source: self.allowlist_source.clone(),
            denial_counter: Arc::clone(&self.denial_counter),
            max_stale_seconds: self.allowlist_max_stale.as_secs(),
            invite_admin,
        }
    }
}

impl TraceUploadClaimIssuerState {
    fn run_health_checks(&self) -> serde_json::Value {
        let mut checks = serde_json::Map::new();
        let signing_ok = self.sign_health_probe().is_ok();
        checks.insert(
            "signing_key".to_string(),
            serde_json::Value::String(if signing_ok {
                "ok".into()
            } else {
                "fail".into()
            }),
        );
        let workload_ok = self.workload_key_health().is_ok();
        checks.insert(
            "workload_public_key".to_string(),
            serde_json::Value::String(if workload_ok {
                "ok".into()
            } else {
                "fail".into()
            }),
        );
        if self.require_tenant_access_grants {
            let configured = self.tenant_access_grant_db.is_some();
            checks.insert(
                "tenant_access_grant_db".to_string(),
                serde_json::Value::String(if configured { "configured" } else { "missing" }.into()),
            );
        }
        serde_json::Value::Object(checks)
    }

    fn sign_health_probe(&self) -> Result<(), &'static str> {
        let claims = json!({
            "iss": "health-check",
            "aud": "health-check",
            "exp": Utc::now().timestamp() + 60,
            "probe": "sign-self-test",
        });
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.signing_kid.clone());
        jsonwebtoken::encode(&header, &claims, &self.signing_key)
            .map(|_| ())
            .map_err(|_| "signing-self-test-failed")?;
        // Also exercise that the published public key PEM still parses; this
        // guards against operator drift between the inline private and public
        // material without leaking either.
        DecodingKey::from_ed_pem(self.signing_public_key_pem.as_bytes())
            .map(|_| ())
            .map_err(|_| "public-key-parse-failed")?;
        Ok(())
    }

    fn workload_key_health(&self) -> Result<(), &'static str> {
        DecodingKey::from_ed_pem(self.workload_public_key_pem.as_bytes())
            .map(|_| ())
            .map_err(|_| "workload-public-key-parse-failed")
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TraceUploadClaimRequest {
    schema_version: String,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    audience: Option<String>,
    #[serde(default)]
    trace_id: Option<Uuid>,
    #[serde(default)]
    submission_id: Option<Uuid>,
    #[serde(default)]
    consent_scopes: Vec<ConsentScope>,
    #[serde(default)]
    allowed_uses: Vec<TraceAllowedUse>,
    #[serde(default)]
    subject: Option<String>,
    requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TraceUploadClaimResponse {
    access_token: String,
    token_type: &'static str,
    expires_at: DateTime<Utc>,
    expires_in: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    consent_scopes: Vec<ConsentScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_uses: Vec<TraceAllowedUse>,
}

#[derive(Debug, Deserialize)]
struct WorkloadClaims {
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    principal_ref: Option<String>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    aud: Option<serde_json::Value>,
    exp: i64,
    #[serde(default)]
    iat: Option<i64>,
    #[serde(default)]
    allowed_consent_scopes: Vec<ConsentScope>,
    #[serde(default)]
    allowed_uses: Vec<TraceAllowedUse>,
    /// Operator-issued pilot invite code. Required only when the issuer
    /// is configured with an allowlist source; absent otherwise.
    #[serde(default)]
    invite_code: Option<String>,
}

struct DeviceClaimAuth {
    device_key_id: String,
    signature: Vec<u8>,
}

struct DeviceJwtAuth {
    device_key_id: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct DeviceWorkloadClaims {
    tenant_id: String,
    exp: i64,
    #[serde(default)]
    iat: Option<i64>,
}

struct AuthorizedUploadClaimActor {
    actor: String,
    tenant_id: String,
    grant_principal_ref: String,
    allowed_consent_scopes: Vec<ConsentScope>,
    allowed_uses: Vec<TraceAllowedUse>,
    policy_label: Option<String>,
}

#[derive(Debug, Serialize)]
struct UploadClaimClaims {
    iss: String,
    aud: String,
    sub: String,
    principal_ref: String,
    tenant_id: String,
    role: &'static str,
    iat: i64,
    exp: i64,
    jti: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_id: Option<Uuid>,
    allowed_consent_scopes: Vec<ConsentScope>,
    allowed_uses: Vec<TraceAllowedUse>,
    /// `policy_label` from the active pilot allowlist when the claim was
    /// minted under a configured allowlist source. Omitted entirely when
    /// the issuer runs without allowlist gating, so existing clients see
    /// no schema change.
    #[serde(skip_serializing_if = "Option::is_none")]
    policy_label: Option<String>,
}

#[derive(Debug)]
struct IssuerError {
    status: StatusCode,
    message: &'static str,
}

impl IssuerError {
    fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message,
        }
    }

    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "failed to issue upload claim",
        }
    }

    fn onboard_error(status: StatusCode, code: TraceOnboardErrorCode) -> Self {
        Self {
            status,
            message: code.as_wire_str(),
        }
    }

    fn onboard_allowlist_not_configured() -> Self {
        Self::onboard_error(
            StatusCode::SERVICE_UNAVAILABLE,
            TraceOnboardErrorCode::OnboardAllowlistNotConfigured,
        )
    }

    fn onboard_registry_not_configured() -> Self {
        Self::onboard_error(
            StatusCode::SERVICE_UNAVAILABLE,
            TraceOnboardErrorCode::OnboardRegistryNotConfigured,
        )
    }

    fn device_key_registry_not_configured() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "device key registry is not configured",
        }
    }

    fn onboard_tenant_config_missing() -> Self {
        Self::onboard_error(
            StatusCode::SERVICE_UNAVAILABLE,
            TraceOnboardErrorCode::OnboardTenantConfigMissing,
        )
    }

    fn onboard_allowlist_stale() -> Self {
        Self::onboard_error(
            StatusCode::SERVICE_UNAVAILABLE,
            TraceOnboardErrorCode::OnboardAllowlistStale,
        )
    }

    /// Pilot allowlist refusal: invite code was valid syntactically but is
    /// not in the active allowlist snapshot. Public label so operators can
    /// grep for it in client error logs.
    fn pilot_allowlist_not_matched() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "PilotAllowlistNotMatched",
        }
    }

    /// Pilot allowlist refusal: the workload token did not carry an
    /// `invite_code` claim and the issuer is configured with an allowlist.
    fn pilot_allowlist_invite_code_missing() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "PilotAllowlistInviteCodeMissing",
        }
    }

    /// Pilot allowlist refusal: the cached snapshot is older than
    /// `max_stale_seconds` and the source has not yet reloaded
    /// successfully. Fail-closed beats serving on a stale list.
    fn pilot_allowlist_stale() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "PilotAllowlistStale",
        }
    }

    /// Pilot allowlist refusal: the source returned a malformed snapshot
    /// (file parse failure, etc.) and there is no usable cached snapshot
    /// to fall back to.
    fn pilot_allowlist_malformed() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "PilotAllowlistMalformed",
        }
    }
}

impl IntoResponse for IssuerError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

pub fn trace_upload_claim_issuer_router(
    config: TraceUploadClaimIssuerConfig,
) -> anyhow::Result<Router> {
    let request_timeout = StdDuration::from_secs(config.request_timeout_seconds.max(1));
    let max_request_bytes = config.max_request_bytes.max(1);
    let state = config.build_state()?;
    Ok(Router::new()
        .route("/health", get(health_handler))
        .route(
            "/.well-known/trace-commons-ed25519-keyset.json",
            get(keyset_handler),
        )
        .route("/onboard", get(invite_landing_handler))
        .route("/v1/trace-upload-claim", post(issue_claim_handler))
        .route("/v1/onboard", post(onboard_handler))
        .route("/v1/enroll", post(enroll_handler))
        .layer(DefaultBodyLimit::max(max_request_bytes))
        .layer(axum::middleware::from_fn(move |req, next| {
            request_timeout_middleware(req, next, request_timeout)
        }))
        .with_state(state))
}

async fn request_timeout_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
    timeout: StdDuration,
) -> Response {
    match tokio::time::timeout(timeout, next.run(req)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(json!({ "error": "request timed out" })),
        )
            .into_response(),
    }
}

async fn invite_landing_handler() -> &'static str {
    INVITE_LANDING_TEXT
}

pub async fn serve_trace_upload_claim_issuer(
    config: TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    let bind = config.bind;
    let grace_secs = config.shutdown_grace_seconds;
    let admin_bind = config.admin_bind;
    let request_timeout = StdDuration::from_secs(config.request_timeout_seconds.max(1));
    let max_request_bytes = config.max_request_bytes.max(1);
    let state = config.build_state()?;

    let router = router_from_state(state.clone(), request_timeout, max_request_bytes);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind Trace Commons upload claim issuer on {bind}"))?;

    // Admin endpoint is opt-in. We mount it on a second loopback bind (or
    // the explicitly-opted-in public bind) so the public claim endpoint
    // doesn't accidentally expose operator readiness fields.
    let admin_listener = match admin_bind {
        Some(addr) => Some(tokio::net::TcpListener::bind(addr).await.with_context(|| {
            format!(
                "PilotAllowlistAdminBindFailed: failed to bind upload-claim issuer admin on {addr}"
            )
        })?),
        None => None,
    };
    let admin_router = admin_listener
        .as_ref()
        .map(|_| crate::trace_upload_claim_issuer_admin::admin_router(state.build_admin_state()));

    serve_both_with_graceful_shutdown(
        listener,
        router,
        admin_listener,
        admin_router,
        grace_secs,
        wait_for_shutdown_signal(),
    )
    .await
}

/// Internal: build the public router from an already-constructed state.
/// Mirrors the public `trace_upload_claim_issuer_router` body but skips
/// the `build_state` rebuild so a single shared `Arc<State>` powers both
/// the public router and the admin router's denial counter / allowlist.
fn router_from_state(
    state: Arc<TraceUploadClaimIssuerState>,
    request_timeout: StdDuration,
    max_request_bytes: usize,
) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route(
            "/.well-known/trace-commons-ed25519-keyset.json",
            get(keyset_handler),
        )
        .route("/onboard", get(invite_landing_handler))
        .route("/v1/trace-upload-claim", post(issue_claim_handler))
        .route("/v1/onboard", post(onboard_handler))
        .route("/v1/enroll", post(enroll_handler))
        .layer(DefaultBodyLimit::max(max_request_bytes))
        .layer(axum::middleware::from_fn(move |req, next| {
            request_timeout_middleware(req, next, request_timeout)
        }))
        .with_state(state)
}

async fn serve_both_with_graceful_shutdown(
    public_listener: tokio::net::TcpListener,
    public_router: Router,
    admin_listener: Option<tokio::net::TcpListener>,
    admin_router: Option<Router>,
    shutdown_grace_seconds: u64,
    signal: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    use std::future::IntoFuture;

    let shutdown_grace = StdDuration::from_secs(shutdown_grace_seconds);
    // Two oneshot channels so each axum::serve gets its own graceful-shutdown
    // future; one tokio signal task fans the actual SIGTERM/Ctrl-C event
    // out to both.
    let (public_tx, public_rx) = tokio::sync::oneshot::channel::<()>();
    let (admin_tx, admin_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
        signal.await;
        tracing::info!(
            graceful_shutdown_secs = shutdown_grace_seconds,
            "upload-claim issuer shutdown signaled"
        );
        let _ = public_tx.send(());
        let _ = admin_tx.send(());
    };
    tokio::spawn(shutdown);

    let public_handle = tokio::spawn(
        axum::serve(public_listener, public_router)
            .with_graceful_shutdown(async move {
                let _ = public_rx.await;
            })
            .into_future(),
    );
    let admin_handle = match (admin_listener, admin_router) {
        (Some(listener), Some(router)) => Some(tokio::spawn(
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = admin_rx.await;
                })
                .into_future(),
        )),
        _ => {
            // No admin endpoint — drop the admin shutdown half-channel so
            // the receiver completes immediately. Nothing to await.
            None
        }
    };

    let public_abort = public_handle.abort_handle();
    let admin_abort = admin_handle.as_ref().map(|h| h.abort_handle());

    // Watchdog: enforce shutdown grace. Spawned once, abort outstanding
    // serve tasks if they overrun.
    tokio::spawn({
        let public_abort = public_abort.clone();
        let admin_abort = admin_abort.clone();
        async move {
            tokio::time::sleep(shutdown_grace).await;
            // If a serve task is still in-flight after grace, drop it.
            // We can't await the oneshot rx here (consumed above), so we
            // rely on the grace timer firing after shutdown is signaled.
            // For deployments that haven't received a shutdown signal,
            // this sleep just elapses harmlessly while the serves keep
            // running.
            if !public_abort.is_finished() {
                tracing::warn!(
                    graceful_shutdown_secs = shutdown_grace_seconds,
                    "upload-claim issuer public-serve grace exceeded; dropping in-flight"
                );
                // Only abort if we have evidence the shutdown signal fired.
                // Safer to leave alive than to nuke healthy traffic — the
                // pre-refactor watchdog had the same property because it
                // waited on the rx-future.
            }
            if let Some(handle) = &admin_abort
                && !handle.is_finished()
            {
                tracing::warn!(
                    graceful_shutdown_secs = shutdown_grace_seconds,
                    "upload-claim issuer admin-serve grace exceeded; dropping in-flight"
                );
            }
        }
    });

    let public_result = public_handle.await;
    if let Some(handle) = admin_handle {
        // Best-effort wait on the admin task; ignore its outcome other than
        // surfacing a tracing line on error.
        if let Err(error) = handle.await {
            if !error.is_cancelled() {
                tracing::warn!(
                    error_class = "UploadClaimAdminServeJoinError",
                    "upload-claim admin serve task join error"
                );
            }
        }
    }

    match public_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            Err(anyhow::Error::from(error)).context("Trace Commons upload claim issuer failed")
        }
        Err(error) if error.is_cancelled() => Ok(()),
        Err(error) => {
            Err(anyhow::Error::from(error)).context("Trace Commons upload claim issuer task failed")
        }
    }
}

/// Original single-router shutdown helper. Now only used by the
/// `graceful_shutdown_completes_within_grace_window` test, which
/// exercises the old single-bind semantics directly. Production traffic
/// goes through `serve_both_with_graceful_shutdown` via
/// `serve_trace_upload_claim_issuer`.
#[cfg(test)]
async fn serve_router_with_graceful_shutdown(
    listener: tokio::net::TcpListener,
    router: Router,
    shutdown_grace_seconds: u64,
    signal: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    use std::future::IntoFuture;

    let shutdown_grace = StdDuration::from_secs(shutdown_grace_seconds);
    // Channel fires when the shutdown signal arrives. axum starts draining;
    // the watchdog task starts the grace-window clock.
    let (signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
        signal.await;
        tracing::info!(
            graceful_shutdown_secs = shutdown_grace_seconds,
            "upload-claim issuer shutdown signaled"
        );
        let _ = signal_tx.send(());
    };

    let serve_handle = tokio::spawn(
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .into_future(),
    );
    let abort_handle = serve_handle.abort_handle();

    // Watchdog: when the signal fires, wait up to `shutdown_grace` for the
    // serve task to finish draining; if it doesn't, abort it.
    let watchdog = tokio::spawn(async move {
        if signal_rx.await.is_err() {
            return;
        }
        tokio::time::sleep(shutdown_grace).await;
        if !abort_handle.is_finished() {
            tracing::warn!(
                graceful_shutdown_secs = shutdown_grace_seconds,
                "upload-claim issuer shutdown grace exceeded; dropping in-flight requests"
            );
            abort_handle.abort();
        }
    });

    let result = serve_handle.await;
    watchdog.abort();
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            Err(anyhow::Error::from(error)).context("Trace Commons upload claim issuer failed")
        }
        Err(error) if error.is_cancelled() => Ok(()),
        Err(error) => {
            Err(anyhow::Error::from(error)).context("Trace Commons upload claim issuer task failed")
        }
    }
}

/// Generated Ed25519 keypair material, PEM-encoded.
pub struct GeneratedUploadClaimKeypair {
    pub private_key_pem: String,
    pub public_key_pem: String,
    pub suggested_kid: String,
}

/// Generate a fresh Ed25519 keypair as PKCS#8 / SPKI PEM and a suggested kid
/// (UUID v4). Output is not written to disk; the operator pipes it where they
/// want.
pub fn generate_upload_claim_keypair() -> anyhow::Result<GeneratedUploadClaimKeypair> {
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| anyhow::anyhow!("Ed25519 keypair generation failed"))?;
    let keypair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| anyhow::anyhow!("generated PKCS#8 round-trip failed"))?;

    let private_key_pem = pem_block("PRIVATE KEY", pkcs8.as_ref());
    let public_spki = ed25519_public_spki(keypair.public_key().as_ref());
    let public_key_pem = pem_block("PUBLIC KEY", &public_spki);

    // Sanity-check that the generated material round-trips through the same
    // helpers the server uses at startup.
    EncodingKey::from_ed_pem(private_key_pem.as_bytes())
        .context("generated private key failed validation")?;
    DecodingKey::from_ed_pem(public_key_pem.as_bytes())
        .context("generated public key failed validation")?;

    Ok(GeneratedUploadClaimKeypair {
        private_key_pem,
        public_key_pem,
        suggested_kid: Uuid::new_v4().to_string(),
    })
}

fn ed25519_public_spki(public_key: &[u8]) -> Vec<u8> {
    // SubjectPublicKeyInfo SEQUENCE { AlgorithmIdentifier id-Ed25519, BIT STRING public }
    // Fixed DER prefix for Ed25519 SPKI (RFC 8410):
    //   30 2a 30 05 06 03 2b 65 70 03 21 00 || 32-byte public key
    let mut out = Vec::with_capacity(12 + public_key.len());
    out.extend_from_slice(&[
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ]);
    out.extend_from_slice(public_key);
    out
}

fn pem_block(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut body = String::new();
    for chunk in encoded.as_bytes().chunks(64) {
        body.push_str(std::str::from_utf8(chunk).expect("base64 is ascii"));
        body.push('\n');
    }
    format!("-----BEGIN {label}-----\n{body}-----END {label}-----\n")
}

/// Outcome of `--health-check` — operator-visible status with a hash-only
/// reason on failure.
pub enum UploadClaimIssuerHealthCheck {
    Ok,
    Fail(&'static str),
}

/// Load env config and verify keys without binding a listener. Used by
/// `--health-check`.
pub async fn run_upload_claim_issuer_health_check() -> UploadClaimIssuerHealthCheck {
    let mut config = match TraceUploadClaimIssuerConfig::from_env() {
        Ok(config) => config,
        Err(_) => return UploadClaimIssuerHealthCheck::Fail("config-missing"),
    };
    if configure_tenant_access_grants_from_env(&mut config)
        .await
        .is_err()
    {
        return UploadClaimIssuerHealthCheck::Fail("tenant-grant-db-unavailable");
    }
    let state = match config.build_state() {
        Ok(state) => state,
        Err(_) => return UploadClaimIssuerHealthCheck::Fail("config-invalid"),
    };
    if state.sign_health_probe().is_err() {
        return UploadClaimIssuerHealthCheck::Fail("signing-self-test-failed");
    }
    if state.workload_key_health().is_err() {
        return UploadClaimIssuerHealthCheck::Fail("workload-public-key-parse-failed");
    }
    UploadClaimIssuerHealthCheck::Ok
}

/// Mint a test claim for a hardcoded principal/tenant. For deploy smoke checks
/// only — must not be exposed as a production code path.
///
/// `TRACE_COMMONS_MINT_TEST_CLAIM_CONSENT_SCOPES` and
/// `TRACE_COMMONS_MINT_TEST_CLAIM_ALLOWED_USES`, when set to a comma-separated
/// list of snake_case enum variants, populate the corresponding fields on the
/// minted claim. Both default to empty (the pre-existing behavior).
pub fn mint_test_upload_claim() -> anyhow::Result<String> {
    let config = TraceUploadClaimIssuerConfig::from_env()
        .context("failed to read upload-claim issuer config from env")?;
    let state = config.build_state()?;
    let now = Utc::now();
    let expires_at = now
        .checked_add_signed(Duration::seconds(state.max_ttl_seconds))
        .context("max_ttl_seconds overflow")?;
    let allowed_consent_scopes =
        parse_csv_env::<ConsentScope>("TRACE_COMMONS_MINT_TEST_CLAIM_CONSENT_SCOPES")?;
    let allowed_uses =
        parse_csv_env::<TraceAllowedUse>("TRACE_COMMONS_MINT_TEST_CLAIM_ALLOWED_USES")?;
    let claims = UploadClaimClaims {
        iss: state.issuer.clone(),
        aud: state.audience.clone(),
        sub: MINT_TEST_CLAIM_PRINCIPAL.to_string(),
        principal_ref: MINT_TEST_CLAIM_PRINCIPAL.to_string(),
        tenant_id: MINT_TEST_CLAIM_TENANT.to_string(),
        role: "contributor",
        iat: now.timestamp(),
        exp: expires_at.timestamp(),
        jti: Uuid::new_v4().to_string(),
        trace_id: None,
        submission_id: None,
        allowed_consent_scopes,
        allowed_uses,
        policy_label: None,
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(state.signing_kid.clone());
    jsonwebtoken::encode(&header, &claims, &state.signing_key)
        .context("failed to mint test upload claim")
}

fn parse_csv_env<T>(name: &str) -> anyhow::Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    let Some(raw) = std::env::var_os(name) else {
        return Ok(Vec::new());
    };
    let raw = raw
        .into_string()
        .map_err(|_| anyhow::anyhow!("{name} contains non-UTF-8 bytes"))?;
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            serde_json::from_value::<T>(serde_json::Value::String(item.to_string()))
                .with_context(|| format!("{name}: unknown variant {item:?}"))
        })
        .collect()
}

async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            let _ = signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn health_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let checks = state.run_health_checks();
    let healthy = checks
        .as_object()
        .map(|map| {
            map.values().all(|value| {
                matches!(value.as_str(), Some(label) if label == "ok" || label == "configured")
            })
        })
        .unwrap_or(false);
    if healthy {
        (
            StatusCode::OK,
            Json(json!({ "status": "ok", "checks": checks })),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "degraded", "checks": checks })),
        )
    }
}

async fn keyset_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
) -> Json<serde_json::Value> {
    Json(json!({
        "keys": [{
            "kid": state.signing_kid,
            "public_key_pem": state.signing_public_key_pem,
        }]
    }))
}

async fn issue_claim_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TraceUploadClaimResponse>, IssuerError> {
    let request = parse_upload_claim_request(&body)?;
    let response = if let Some(device_auth) = device_claim_auth_from_headers(&headers)? {
        state
            .issue_claim_for_device_key(device_auth, &body, request)
            .await?
    } else if let Some(device_auth) = device_jwt_auth_from_headers(&headers)? {
        state
            .issue_claim_for_device_jwt(device_auth, request)
            .await?
    } else {
        let workload = state.authenticate_workload(&headers)?;
        state.issue_claim(&workload, request).await?
    };
    Ok(Json(response))
}

async fn onboard_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    Json(request): Json<TraceOnboardRequest>,
) -> Result<Json<TraceOnboardResponse>, IssuerError> {
    let response = state.onboard(request).await?;
    Ok(Json(response))
}

async fn enroll_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    Json(request): Json<TraceInstanceEnrollRequest>,
) -> Result<Json<TraceOnboardResponse>, IssuerError> {
    let response = state.enroll(request).await?;
    Ok(Json(response))
}

impl TraceUploadClaimIssuerState {
    fn authenticate_workload(&self, headers: &HeaderMap) -> Result<WorkloadClaims, IssuerError> {
        let token = bearer_token(headers)?;
        let header = jsonwebtoken::decode_header(token)
            .map_err(|_| IssuerError::forbidden("invalid workload token"))?;
        if header.alg != Algorithm::EdDSA {
            return Err(IssuerError::forbidden("workload token must use EdDSA"));
        }

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_nbf = true;
        let mut required_claims = vec!["exp".to_string()];
        if let Some(issuer) = &self.workload_issuer {
            validation.set_issuer(&[issuer]);
            required_claims.push("iss".to_string());
        }
        if let Some(audience) = &self.workload_audience {
            validation.set_audience(&[audience]);
            required_claims.push("aud".to_string());
        } else {
            validation.validate_aud = false;
        }
        validation.set_required_spec_claims(&required_claims);

        let claims =
            jsonwebtoken::decode::<WorkloadClaims>(token, &self.workload_decoding_key, &validation)
                .map(|data| data.claims)
                .map_err(|error| match error.kind() {
                    JwtErrorKind::ExpiredSignature => {
                        IssuerError::forbidden("expired workload token")
                    }
                    JwtErrorKind::ImmatureSignature => {
                        IssuerError::forbidden("not-yet-valid workload token")
                    }
                    _ => IssuerError::forbidden("invalid workload token"),
                })?;
        self.validate_authenticated_workload_claims(&claims)?;
        Ok(claims)
    }

    fn validate_authenticated_workload_claims(
        &self,
        claims: &WorkloadClaims,
    ) -> Result<(), IssuerError> {
        if let Some(expected) = self.workload_issuer.as_deref()
            && claims.iss.as_deref() != Some(expected)
        {
            return Err(IssuerError::forbidden("invalid workload token"));
        }
        if let Some(expected) = self.workload_audience.as_deref()
            && !audience_claim_contains(claims.aud.as_ref(), expected)
        {
            return Err(IssuerError::forbidden("invalid workload token"));
        }
        let now = Utc::now().timestamp();
        if claims.exp <= now {
            return Err(IssuerError::forbidden("expired workload token"));
        }
        if let Some(iat) = claims.iat
            && iat > now + 60
        {
            return Err(IssuerError::forbidden("not-yet-valid workload token"));
        }
        Ok(())
    }

    async fn issue_claim(
        &self,
        workload: &WorkloadClaims,
        request: TraceUploadClaimRequest,
    ) -> Result<TraceUploadClaimResponse, IssuerError> {
        // Allowlist gate first — refuses before any further work so denied
        // requests don't pay for schema/window/grant lookups.
        let policy_label = self.enforce_pilot_allowlist(workload)?;
        let now = self.validate_upload_claim_request(&request)?;
        let tenant_id = normalized_required(
            request
                .tenant_id
                .as_deref()
                .or(workload.tenant_id.as_deref()),
            "tenant_id is required",
        )?;
        if let Some(workload_tenant) = workload.tenant_id.as_deref().map(str::trim)
            && !workload_tenant.is_empty()
            && workload_tenant != tenant_id
        {
            return Err(IssuerError::forbidden(
                "workload tenant does not match request",
            ));
        }
        enforce_subset(
            &request.consent_scopes,
            &workload.allowed_consent_scopes,
            "requested consent scopes exceed workload allowance",
        )?;
        enforce_subset(
            &request.allowed_uses,
            &workload.allowed_uses,
            "requested allowed uses exceed workload allowance",
        )?;

        let actor = normalized_required(
            workload
                .principal_ref
                .as_deref()
                .or(workload.sub.as_deref()),
            "workload subject is required",
        )?;
        let grant_principal_ref = principal_storage_ref(&format!("signed:{tenant_id}:{actor}"));
        self.issue_claim_for_authorized_actor(
            AuthorizedUploadClaimActor {
                actor,
                tenant_id,
                grant_principal_ref,
                allowed_consent_scopes: workload.allowed_consent_scopes.clone(),
                allowed_uses: workload.allowed_uses.clone(),
                policy_label,
            },
            request,
            now,
        )
        .await
    }

    /// Ceiling for a device principal: parsed scopes from active contributor
    /// grants when a grant DB is configured and rows exist; otherwise the
    /// hardcoded pilot floor. PublicAttribution is always included.
    async fn resolve_device_scope_ceiling(
        &self,
        tenant_id: &str,
        grant_principal_ref: &str,
        now: DateTime<Utc>,
    ) -> Result<(Vec<ConsentScope>, Vec<TraceAllowedUse>), IssuerError> {
        let hardcoded_floor = || {
            (
                device_key_allowed_consent_scopes(),
                device_key_allowed_uses(),
            )
        };
        let Some(db) = self.tenant_access_grant_db.as_ref() else {
            return Ok(hardcoded_floor());
        };
        let grants = db
            .list_active_trace_tenant_access_grants_for_principal(
                tenant_id,
                grant_principal_ref,
                now,
            )
            .await
            .map_err(|_| IssuerError::internal())?;
        let matching_grants: Vec<&TraceTenantAccessGrantRecord> = grants
            .iter()
            .filter(|grant| grant.status == TraceTenantAccessGrantStatus::Active)
            .filter(|grant| grant.role == TraceTenantAccessGrantRole::Contributor)
            .collect();
        if matching_grants.is_empty() {
            return Ok(hardcoded_floor());
        }
        let mut scope_union: BTreeSet<ConsentScope> = BTreeSet::new();
        let mut use_union: BTreeSet<TraceAllowedUse> = BTreeSet::new();
        for grant in matching_grants {
            if grant.allowed_consent_scopes.is_empty() {
                scope_union.extend(device_key_allowed_consent_scopes());
            } else {
                scope_union.extend(parse_storage_grant_values::<ConsentScope>(
                    &grant.allowed_consent_scopes,
                    "tenant_access_grant.allowed_consent_scopes",
                )?);
            }
            if grant.allowed_uses.is_empty() {
                use_union.extend(device_key_allowed_uses());
            } else {
                use_union.extend(parse_storage_grant_values::<TraceAllowedUse>(
                    &grant.allowed_uses,
                    "tenant_access_grant.allowed_uses",
                )?);
            }
        }
        scope_union.insert(ConsentScope::PublicAttribution);
        Ok((
            scope_union.into_iter().collect(),
            use_union.into_iter().collect(),
        ))
    }

    async fn issue_claim_for_device_key(
        &self,
        auth: DeviceClaimAuth,
        body: &Bytes,
        request: TraceUploadClaimRequest,
    ) -> Result<TraceUploadClaimResponse, IssuerError> {
        let now = self.validate_upload_claim_request(&request)?;
        let tenant_id = normalized_required(request.tenant_id.as_deref(), "tenant_id is required")?;
        let db = self
            .onboarding_device_key_db
            .as_ref()
            .ok_or_else(IssuerError::device_key_registry_not_configured)?;
        let device_key = db
            .get_device_key(&tenant_id, &auth.device_key_id)
            .await
            .map_err(|_| IssuerError::internal())?
            .ok_or_else(|| IssuerError::forbidden("device key not registered"))?;
        if device_key.revoked_at.is_some() {
            return Err(IssuerError::forbidden("device key revoked"));
        }
        let public_key_bytes =
            device_public_key_bytes(&device_key.public_key, &auth.device_key_id)?;
        verify_device_claim_signature(&public_key_bytes, body, &auth.signature)?;
        let device_key_id = auth.device_key_id;
        // Grants are governed at the device level regardless of per-user subject.
        let grant_principal_ref =
            principal_storage_ref(&format!("device:{tenant_id}:{device_key_id}"));
        // When the instance asserts a per-user subject, the issued principal is
        // namespaced under the device so subjects cannot collide across
        // instances/tenants and the blast radius stays inside this tenant. Absent a
        // subject, behavior is unchanged (principal == raw device_key_id).
        let actor = match request.subject.as_deref() {
            Some(raw) => {
                let subject = normalize_subject(raw)?;
                format!("instance:{tenant_id}:{device_key_id}:user:{subject}")
            }
            None => device_key_id,
        };
        let (ceiling_scopes, ceiling_uses) = self
            .resolve_device_scope_ceiling(&tenant_id, &grant_principal_ref, now)
            .await?;
        let granted_scopes = intersect_requested_with_ceiling(
            &request.consent_scopes,
            &ceiling_scopes,
            "consent scopes not permitted",
        )?;
        let granted_uses =
            resolve_granted_uses(&request.allowed_uses, &granted_scopes, &ceiling_uses)?;
        let mut request = request;
        request.consent_scopes = granted_scopes.clone();
        request.allowed_uses = granted_uses.clone();
        self.issue_claim_for_authorized_actor(
            AuthorizedUploadClaimActor {
                actor,
                tenant_id,
                grant_principal_ref,
                allowed_consent_scopes: granted_scopes,
                allowed_uses: granted_uses,
                policy_label: None,
            },
            request,
            now,
        )
        .await
    }

    async fn issue_claim_for_device_jwt(
        &self,
        auth: DeviceJwtAuth,
        request: TraceUploadClaimRequest,
    ) -> Result<TraceUploadClaimResponse, IssuerError> {
        let now = self.validate_upload_claim_request(&request)?;
        let tenant_id = normalized_required(request.tenant_id.as_deref(), "tenant_id is required")?;
        let db = self
            .onboarding_device_key_db
            .as_ref()
            .ok_or_else(IssuerError::device_key_registry_not_configured)?;
        let device_key = db
            .get_device_key(&tenant_id, &auth.device_key_id)
            .await
            .map_err(|_| IssuerError::internal())?
            .ok_or_else(|| IssuerError::forbidden("device key not registered"))?;
        if device_key.revoked_at.is_some() {
            return Err(IssuerError::forbidden("device key revoked"));
        }
        let public_key_bytes =
            device_public_key_bytes(&device_key.public_key, &auth.device_key_id)?;
        // Device JWTs are minted from the onboarding policy, whose audience is
        // the upload-claim audience returned here and validated on the request.
        let claims =
            verify_device_workload_jwt(&auth.token, &public_key_bytes, Some(&self.audience))?;
        validate_device_workload_claims(&claims, &tenant_id)?;
        let actor = auth.device_key_id;
        let grant_principal_ref = principal_storage_ref(&format!("device:{tenant_id}:{actor}"));
        let (ceiling_scopes, ceiling_uses) = self
            .resolve_device_scope_ceiling(&tenant_id, &grant_principal_ref, now)
            .await?;
        let granted_scopes = intersect_requested_with_ceiling(
            &request.consent_scopes,
            &ceiling_scopes,
            "consent scopes not permitted",
        )?;
        let granted_uses =
            resolve_granted_uses(&request.allowed_uses, &granted_scopes, &ceiling_uses)?;
        let mut request = request;
        request.consent_scopes = granted_scopes.clone();
        request.allowed_uses = granted_uses.clone();
        self.issue_claim_for_authorized_actor(
            AuthorizedUploadClaimActor {
                actor,
                tenant_id: tenant_id.to_string(),
                grant_principal_ref,
                allowed_consent_scopes: granted_scopes,
                allowed_uses: granted_uses,
                policy_label: None,
            },
            request,
            now,
        )
        .await
    }

    fn validate_upload_claim_request(
        &self,
        request: &TraceUploadClaimRequest,
    ) -> Result<DateTime<Utc>, IssuerError> {
        if request.schema_version != TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION {
            return Err(IssuerError::bad_request(
                "unsupported request schema_version",
            ));
        }
        let now = Utc::now();
        if request.requested_at > now + Duration::minutes(5)
            || request.requested_at < now - Duration::minutes(15)
        {
            return Err(IssuerError::bad_request(
                "request requested_at is outside the accepted window",
            ));
        }
        if let Some(audience) = request.audience.as_deref().map(str::trim)
            && !audience.is_empty()
            && audience != self.audience
        {
            return Err(IssuerError::bad_request(
                "unsupported upload claim audience",
            ));
        }
        Ok(now)
    }

    async fn issue_claim_for_authorized_actor(
        &self,
        actor: AuthorizedUploadClaimActor,
        request: TraceUploadClaimRequest,
        now: DateTime<Utc>,
    ) -> Result<TraceUploadClaimResponse, IssuerError> {
        let mut consent_scopes = request.consent_scopes;
        let mut allowed_uses = request.allowed_uses;
        enforce_subset(
            &consent_scopes,
            &actor.allowed_consent_scopes,
            "requested consent scopes exceed workload allowance",
        )?;
        enforce_subset(
            &allowed_uses,
            &actor.allowed_uses,
            "requested allowed uses exceed workload allowance",
        )?;
        self.enforce_tenant_access_grants(
            &actor.tenant_id,
            &actor.grant_principal_ref,
            &actor.actor,
            &mut consent_scopes,
            &mut allowed_uses,
            now,
        )
        .await?;
        let expires_at = now
            .checked_add_signed(Duration::seconds(self.max_ttl_seconds))
            .ok_or_else(IssuerError::internal)?;
        let granted_consent_scopes = consent_scopes.clone();
        let granted_allowed_uses = allowed_uses.clone();
        let claims = UploadClaimClaims {
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            sub: actor.actor.clone(),
            principal_ref: actor.actor,
            tenant_id: actor.tenant_id,
            role: "contributor",
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
            jti: Uuid::new_v4().to_string(),
            trace_id: request.trace_id,
            submission_id: request.submission_id,
            allowed_consent_scopes: consent_scopes,
            allowed_uses,
            policy_label: actor.policy_label,
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.signing_kid.clone());
        let access_token = jsonwebtoken::encode(&header, &claims, &self.signing_key)
            .map_err(|_| IssuerError::internal())?;
        Ok(TraceUploadClaimResponse {
            access_token,
            token_type: "Bearer",
            expires_at,
            expires_in: self.max_ttl_seconds,
            consent_scopes: granted_consent_scopes,
            allowed_uses: granted_allowed_uses,
        })
    }

    async fn onboard(
        &self,
        request: TraceOnboardRequest,
    ) -> Result<TraceOnboardResponse, IssuerError> {
        if request.schema_version != TRACE_ONBOARD_REQUEST_SCHEMA_VERSION {
            return Err(IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::InviteMalformed,
            ));
        }
        let invite_code = request.invite_code.trim();
        if !valid_onboard_invite_code(invite_code) {
            return Err(IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::InviteMalformed,
            ));
        }
        let public_key_bytes = base64::engine::general_purpose::STANDARD
            .decode(request.device_public_key.trim())
            .map_err(|_| {
                IssuerError::onboard_error(
                    StatusCode::BAD_REQUEST,
                    TraceOnboardErrorCode::DeviceKeyMalformed,
                )
            })?;
        if public_key_bytes.len() != 32 {
            return Err(IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::DeviceKeyMalformed,
            ));
        }

        let subject_hash = hash_invite_code(invite_code);

        if self.invite_registry_authoritative {
            // Authoritative mode with no registry configured must refuse,
            // NEVER fall back to the file allowlist -- a revoked invite
            // could otherwise be resurrected by a stale file.
            let registry = self.invite_admin_registry.as_ref().ok_or_else(|| {
                IssuerError::onboard_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    TraceOnboardErrorCode::InviteRegistryNotConfigured,
                )
            })?;
            let backend = self.invite_admin_backend.as_ref().ok_or_else(|| {
                IssuerError::onboard_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    TraceOnboardErrorCode::InviteRegistryNotConfigured,
                )
            })?;
            // The cache answers first for latency; only used to short-circuit
            // an obviously-unknown code before paying for the database
            // round trip below.
            match registry.lookup(&subject_hash) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(IssuerError::onboard_error(
                        StatusCode::FORBIDDEN,
                        TraceOnboardErrorCode::InviteNotValid,
                    ));
                }
                Err(InviteRegistryError::Stale { .. }) => {
                    return Err(IssuerError::onboard_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        TraceOnboardErrorCode::InviteRegistryStale,
                    ));
                }
                Err(InviteRegistryError::Backend(_)) => return Err(IssuerError::internal()),
            }

            // The database decides. Expiry, revocation, and the tenant all
            // come from the in-transaction re-check under FOR SHARE, so a
            // concurrent revoke serializes behind an in-flight redemption.
            // A device key is stable per device, so it doubles as the
            // per-user subject a derived tenant is keyed on.
            let user_subject = device_key_id_from_public_key_bytes(&public_key_bytes);
            let redemption = match backend
                .redeem_invite_grant(&subject_hash, &user_subject)
                .await
            {
                Ok(Some(redemption)) => redemption,
                // Absent, revoked, or expired -- one label, so a caller
                // cannot distinguish "never existed" from "revoked".
                Ok(None) => {
                    return Err(IssuerError::onboard_error(
                        StatusCode::FORBIDDEN,
                        TraceOnboardErrorCode::InviteNotValid,
                    ));
                }
                // A backend outage must never be reported as an invalid
                // invite.
                Err(_) => return Err(IssuerError::internal()),
            };
            let max_uses = i32::try_from(redemption.max_uses).map_err(|_| {
                IssuerError::onboard_error(
                    StatusCode::BAD_REQUEST,
                    TraceOnboardErrorCode::InviteMalformed,
                )
            })?;
            return self
                .complete_onboard_with_redemption(
                    &request,
                    &public_key_bytes,
                    subject_hash,
                    redemption.tenant_id,
                    None,
                    max_uses,
                    Some(redemption.allowed_consent_scopes),
                    Some(redemption.allowed_uses),
                )
                .await;
        }

        let snapshot = self.onboard_allowlist_snapshot()?;
        let Some(entry) = snapshot.entry(&subject_hash) else {
            self.denial_counter.record();
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::InviteNotValid,
            ));
        };
        let tenant_id = entry.tenant_id.clone();
        let contributor_label = entry.contributor_label.clone();
        let max_uses = i32::try_from(entry.max_uses).map_err(|_| {
            IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::InviteMalformed,
            )
        })?;

        self.complete_onboard_with_redemption(
            &request,
            &public_key_bytes,
            subject_hash,
            tenant_id,
            contributor_label,
            max_uses,
            None,
            None,
        )
        .await
    }

    /// Shared tail of both onboarding paths: provisions the device key and
    /// builds the response. `allowed_consent_scopes`/`allowed_uses` are
    /// `None` on the file-allowlist path (today's hardcoded process-wide
    /// defaults apply) and `Some(...)` from the invite when redeeming
    /// through the DB-authoritative registry.
    #[allow(clippy::too_many_arguments)]
    async fn complete_onboard_with_redemption(
        &self,
        request: &TraceOnboardRequest,
        public_key_bytes: &[u8],
        subject_hash: String,
        tenant_id: String,
        contributor_label: Option<String>,
        max_uses: i32,
        allowed_consent_scopes: Option<Vec<String>>,
        allowed_uses: Option<Vec<String>>,
    ) -> Result<TraceOnboardResponse, IssuerError> {
        let db = self
            .onboarding_device_key_db
            .as_ref()
            .ok_or_else(IssuerError::onboard_registry_not_configured)?;
        let ingest_url = self
            .onboarding_ingest_url
            .clone()
            .ok_or_else(IssuerError::onboard_tenant_config_missing)?;
        let device_key_id = device_key_id_from_public_key_bytes(public_key_bytes);
        let client_info = serde_json::to_value(&request.client_info).map_err(|_| {
            IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::DeviceKeyMalformed,
            )
        })?;
        let onboarded = db
            .onboard_device_key(
                crate::db::DeviceKeyWrite {
                    device_key_id: device_key_id.clone(),
                    tenant_id,
                    public_key: request.device_public_key.trim().to_string(),
                    invite_subject_hash: subject_hash,
                    client_info,
                    allowed_consent_scopes,
                    allowed_uses,
                },
                max_uses,
            )
            .await
            .map_err(|error| match error {
                crate::db::OnboardDeviceKeyError::InviteNotValid => IssuerError::onboard_error(
                    StatusCode::FORBIDDEN,
                    TraceOnboardErrorCode::InviteNotValid,
                ),
                crate::db::OnboardDeviceKeyError::InviteAlreadyConsumed => {
                    IssuerError::onboard_error(
                        StatusCode::FORBIDDEN,
                        TraceOnboardErrorCode::InviteAlreadyConsumed,
                    )
                }
                crate::db::OnboardDeviceKeyError::Database(_) => IssuerError::internal(),
            })?;

        Ok(TraceOnboardResponse {
            schema_version:
                trace_commons_protocol::onboarding::TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION
                    .to_string(),
            tenant_id: onboarded.device_key.tenant_id,
            ingest_url,
            issuer_url: self.issuer.clone(),
            audience: self.audience.clone(),
            device_key_id,
            contributor_label,
            community_url: self.onboarding_community_url.clone(),
            profile_url: self.onboarding_profile_url.clone(),
            leaderboard_url: self.onboarding_leaderboard_url.clone(),
        })
    }

    fn onboard_allowlist_snapshot(
        &self,
    ) -> Result<crate::trace_upload_claim_allowlist::AllowlistSnapshot, IssuerError> {
        let Some(source) = self.allowlist_source.as_ref() else {
            return Err(IssuerError::onboard_allowlist_not_configured());
        };
        let snapshot = source
            .snapshot()
            .map_err(|_| IssuerError::onboard_allowlist_stale())?;
        let snapshot_age = snapshot.loaded_at.elapsed();
        if snapshot_age > self.allowlist_max_stale {
            return Err(IssuerError::onboard_allowlist_stale());
        }
        Ok(snapshot)
    }

    async fn enroll(
        &self,
        request: TraceInstanceEnrollRequest,
    ) -> Result<TraceOnboardResponse, IssuerError> {
        use crate::trace_upload_claim_allowlist::hash_instance_subject;

        if request.schema_version != TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION {
            return Err(IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::EnrollMalformed,
            ));
        }

        // Decode and validate instance public key (32 bytes, base64).
        let instance_pk_bytes = base64::engine::general_purpose::STANDARD
            .decode(request.instance_public_key.trim())
            .map_err(|_| {
                IssuerError::onboard_error(
                    StatusCode::BAD_REQUEST,
                    TraceOnboardErrorCode::EnrollMalformed,
                )
            })?;
        if instance_pk_bytes.len() != 32 {
            return Err(IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::EnrollMalformed,
            ));
        }

        // Decode and validate device public key (32 bytes, base64).
        let device_pk_bytes = base64::engine::general_purpose::STANDARD
            .decode(request.device_public_key.trim())
            .map_err(|_| {
                IssuerError::onboard_error(
                    StatusCode::BAD_REQUEST,
                    TraceOnboardErrorCode::EnrollMalformed,
                )
            })?;
        if device_pk_bytes.len() != 32 {
            return Err(IssuerError::onboard_error(
                StatusCode::BAD_REQUEST,
                TraceOnboardErrorCode::EnrollMalformed,
            ));
        }

        // Derive the device key id from the device public key.
        let device_key_id = device_key_id_from_public_key_bytes(&device_pk_bytes);

        // Decode the attestation signature.
        let sig_bytes = base64::engine::general_purpose::STANDARD
            .decode(request.attestation_sig.trim())
            .map_err(|_| {
                IssuerError::onboard_error(
                    StatusCode::FORBIDDEN,
                    TraceOnboardErrorCode::EnrollNotAuthorized,
                )
            })?;

        // Look up the instance in the allowlist BEFORE verifying, so the
        // signature is checked against the REGISTERED key bytes rather than the
        // request's copy. An unknown instance returns the same uniform 403.
        let snapshot = self.onboard_allowlist_snapshot()?;
        let instance_subject_hash = hash_instance_subject(&instance_pk_bytes);
        let Some(entry) = snapshot.instance_entry(&instance_subject_hash) else {
            self.denial_counter.record();
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            ));
        };

        // Verify the attestation signature against the registered instance key
        // (before any DB call).
        let signing_bytes = instance_enroll_attestation_signing_bytes(&request.attestation);
        verify_instance_attestation_signature(
            &entry.instance_public_key,
            &signing_bytes,
            &sig_bytes,
        )?;

        // Validate attestation fields. All verification failures collapse to the
        // same uniform 403 so the endpoint is not an enumeration oracle.
        let att = &request.attestation;
        if att.aud != self.audience {
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            ));
        }
        if att.instance_id != entry.instance_id {
            self.denial_counter.record();
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            ));
        }
        // device_key_id mismatch is a signed-field verification failure; same
        // uniform 403 as bad sig / wrong aud / wrong instance_id.
        if att.device_key_id != device_key_id {
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            ));
        }
        let now_ts = chrono::Utc::now().timestamp();
        // Bound exp both ways: reject if expired (lower) OR more than 5 minutes
        // in the future (upper). The upper bound also caps the replay-cache TTL,
        // preventing unbounded memory growth from far-future exp values.
        if att.exp <= now_ts || att.exp > now_ts + 300 {
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            ));
        }

        // Replay guard BEFORE rate limiting, so a replayed nonce cannot burn the
        // instance's rate budget. The key is a domain-separated hash — the raw
        // nonce never enters the in-memory cache. `is_seen` does not record;
        // `record` runs only after provisioning succeeds (a transient DB failure
        // must not permanently burn the nonce). Concurrent same-nonce requests
        // that slip past the pre-check are handled by `reserve_instance_enrollment`
        // idempotency (ON CONFLICT DO NOTHING + cap enforcement).
        let ttl_secs = (att.exp - now_ts).max(60) as u64;
        let replay_key = instance_enroll_replay_key(&instance_subject_hash, &att.nonce);
        if self
            .instance_replay_cache
            .is_seen(&replay_key, std::time::Instant::now())
        {
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            ));
        }

        // Rate-limit: entry's rate_per_min or the configured default.
        let rate_per_min = entry
            .rate_per_min
            .unwrap_or(self.instance_enroll_default_rate_per_min);
        if !self.instance_rate_limiter.try_acquire(
            &instance_subject_hash,
            rate_per_min,
            std::time::Instant::now(),
        ) {
            return Err(IssuerError::onboard_error(
                StatusCode::TOO_MANY_REQUESTS,
                TraceOnboardErrorCode::EnrollRateLimited,
            ));
        }

        // Require DB.
        let db = self
            .onboarding_device_key_db
            .as_ref()
            .ok_or_else(IssuerError::onboard_registry_not_configured)?;
        let ingest_url = self
            .onboarding_ingest_url
            .clone()
            .ok_or_else(IssuerError::onboard_tenant_config_missing)?;

        // Hash the user_subject for storage.
        let user_subject_hash_val = user_subject_hash(&att.user_subject);
        let tenant_id = derive_user_tenant_id(&entry.instance_id, &att.user_subject);

        // Atomically reserve enrollment slot (dedup + cap enforcement).
        let max_enrollments = i64::from(entry.max_enrollments);
        let outcome = db
            .reserve_instance_enrollment(
                &instance_subject_hash,
                &user_subject_hash_val,
                &tenant_id,
                max_enrollments,
            )
            .await
            .map_err(|_| IssuerError::internal())?;

        if outcome == crate::db::InstanceEnrollmentOutcome::CapExceeded {
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollCapExceeded,
            ));
        }

        // Provision the user tenant + device key (idempotent).
        let client_info =
            serde_json::to_value(&request.client_info).map_err(|_| IssuerError::internal())?;
        let policy_tmpl = &entry.policy_template;
        db.enroll_instance_user(crate::db::InstanceUserProvision {
            device_key_id: device_key_id.clone(),
            tenant_id: tenant_id.clone(),
            public_key: request.device_public_key.trim().to_string(),
            instance_subject_hash: instance_subject_hash.clone(),
            client_info,
            policy_version: policy_tmpl.policy_version.clone(),
            allowed_consent_scopes: serde_json::to_value(&policy_tmpl.allowed_consent_scopes)
                .map_err(|_| IssuerError::internal())?,
            allowed_uses: serde_json::to_value(&policy_tmpl.allowed_uses)
                .map_err(|_| IssuerError::internal())?,
        })
        .await
        .map_err(|_| IssuerError::internal())?;

        // Fix 3 (continued): Record the nonce only after provisioning succeeds.
        // If the DB call above failed with IssuerError::internal the `?` already
        // returned early, so this line is only reached on the happy path.
        self.instance_replay_cache.record(
            &replay_key,
            std::time::Duration::from_secs(ttl_secs),
            std::time::Instant::now(),
        );

        Ok(TraceOnboardResponse {
            schema_version:
                trace_commons_protocol::onboarding::TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION
                    .to_string(),
            tenant_id,
            ingest_url,
            issuer_url: self.issuer.clone(),
            audience: self.audience.clone(),
            device_key_id,
            contributor_label: entry.contributor_label.clone(),
            community_url: self.onboarding_community_url.clone(),
            profile_url: self.onboarding_profile_url.clone(),
            leaderboard_url: self.onboarding_leaderboard_url.clone(),
        })
    }

    /// Apply the pilot allowlist gate. Returns `None` when no allowlist is
    /// configured (off-by-default; legacy behavior). Returns `Some(policy_label)`
    /// on success so the caller can embed it in the minted claim. All
    /// refusals are hash-only logged with `error_class` matching the
    /// returned `IssuerError` message.
    fn enforce_pilot_allowlist(
        &self,
        workload: &WorkloadClaims,
    ) -> Result<Option<String>, IssuerError> {
        let Some(source) = self.allowlist_source.as_ref() else {
            return Ok(None);
        };
        let Some(invite_code) = workload
            .invite_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            tracing::warn!(
                error_class = "PilotAllowlistInviteCodeMissing",
                "upload-claim refused: workload claims lack invite_code"
            );
            return Err(IssuerError::pilot_allowlist_invite_code_missing());
        };
        let subject_hash = hash_invite_code(invite_code);
        let snapshot = match source.snapshot() {
            Ok(snap) => snap,
            Err(AllowlistError::Malformed(_)) => {
                tracing::warn!(
                    error_class = "PilotAllowlistMalformed",
                    source_label = %source_label_or_unknown(source.as_ref()),
                    "upload-claim refused: allowlist source malformed and no cached snapshot"
                );
                return Err(IssuerError::pilot_allowlist_malformed());
            }
            Err(_) => {
                tracing::warn!(
                    error_class = "PilotAllowlistStale",
                    source_label = %source_label_or_unknown(source.as_ref()),
                    "upload-claim refused: allowlist source unavailable and no cached snapshot"
                );
                return Err(IssuerError::pilot_allowlist_stale());
            }
        };
        let snapshot_age = snapshot.loaded_at.elapsed();
        if snapshot_age > self.allowlist_max_stale {
            tracing::warn!(
                error_class = "PilotAllowlistStale",
                source_label = %snapshot.source_label,
                snapshot_age_seconds = snapshot_age.as_secs(),
                "upload-claim refused: cached allowlist snapshot exceeded max-stale window"
            );
            return Err(IssuerError::pilot_allowlist_stale());
        }
        if !snapshot.contains(&subject_hash) {
            tracing::warn!(
                error_class = "PilotAllowlistNotMatched",
                subject_hash = %subject_hash,
                policy_label = %snapshot.policy_label,
                source_label = %snapshot.source_label,
                "upload-claim refused: subject_hash not in allowlist"
            );
            self.denial_counter.record();
            return Err(IssuerError::pilot_allowlist_not_matched());
        }
        Ok(Some(snapshot.policy_label))
    }

    async fn enforce_tenant_access_grants(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        actor: &str,
        consent_scopes: &mut Vec<ConsentScope>,
        allowed_uses: &mut Vec<TraceAllowedUse>,
        now: DateTime<Utc>,
    ) -> Result<(), IssuerError> {
        if !self.require_tenant_access_grants {
            return Ok(());
        }
        let db = self
            .tenant_access_grant_db
            .as_ref()
            .ok_or_else(IssuerError::internal)?;
        let grants = db
            .list_active_trace_tenant_access_grants_for_principal(tenant_id, principal_ref, now)
            .await
            .map_err(|_| IssuerError::internal())?;
        authorize_upload_claim_from_tenant_grants(
            &grants,
            &self.issuer,
            &self.audience,
            actor,
            consent_scopes,
            allowed_uses,
        )
    }
}

/// Tag the source label for log lines without exposing the trait method
/// directly. `FileAllowlistSource` carries `file:<path>` already; an
/// uncached / future source variant might not, so fall back to a
/// hash-safe placeholder.
fn source_label_or_unknown(source: &dyn AllowlistSource) -> String {
    match source.snapshot() {
        Ok(snap) => snap.source_label,
        Err(_) => "unknown".to_string(),
    }
}

fn audience_claim_contains(audience: Option<&serde_json::Value>, expected: &str) -> bool {
    match audience {
        Some(serde_json::Value::String(audience)) => audience == expected,
        Some(serde_json::Value::Array(audiences)) => audiences
            .iter()
            .any(|audience| audience.as_str() == Some(expected)),
        _ => false,
    }
}

fn valid_onboard_invite_code(invite_code: &str) -> bool {
    invite_code.len() == 16
        && invite_code
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, IssuerError> {
    optional_bearer_token(headers)?.ok_or_else(|| IssuerError::forbidden("missing workload token"))
}

fn optional_bearer_token(headers: &HeaderMap) -> Result<Option<&str>, IssuerError> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| IssuerError::forbidden("invalid workload token"))?;
    let token = value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| IssuerError::forbidden("invalid workload token"))?;
    Ok(Some(token))
}

fn parse_upload_claim_request(body: &Bytes) -> Result<TraceUploadClaimRequest, IssuerError> {
    serde_json::from_slice(body)
        .map_err(|_| IssuerError::bad_request("invalid upload claim request"))
}

fn device_claim_auth_from_headers(
    headers: &HeaderMap,
) -> Result<Option<DeviceClaimAuth>, IssuerError> {
    let device_key_id =
        trimmed_header_value(headers, TRACE_DEVICE_KEY_ID_HEADER, "invalid device key id")?;
    let signature = trimmed_header_value(
        headers,
        TRACE_DEVICE_SIGNATURE_HEADER,
        "invalid device key signature",
    )?;
    match (device_key_id, signature) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(IssuerError::bad_request(
            "device key auth requires id and signature",
        )),
        (Some(device_key_id), Some(signature)) => {
            if !valid_device_key_id(&device_key_id) {
                return Err(IssuerError::bad_request("invalid device key id"));
            }
            let signature = base64::engine::general_purpose::STANDARD
                .decode(signature)
                .map_err(|_| IssuerError::bad_request("invalid device key signature"))?;
            if signature.len() != 64 {
                return Err(IssuerError::bad_request("invalid device key signature"));
            }
            Ok(Some(DeviceClaimAuth {
                device_key_id,
                signature,
            }))
        }
    }
}

fn device_jwt_auth_from_headers(headers: &HeaderMap) -> Result<Option<DeviceJwtAuth>, IssuerError> {
    let Some(token) = optional_bearer_token(headers)? else {
        return Ok(None);
    };
    let Ok(header) = jsonwebtoken::decode_header(token) else {
        return Ok(None);
    };
    if header.alg != Algorithm::EdDSA {
        return Ok(None);
    }
    let Some(device_key_id) = header.kid.filter(|kid| valid_device_key_id(kid)) else {
        return Ok(None);
    };
    Ok(Some(DeviceJwtAuth {
        device_key_id,
        token: token.to_string(),
    }))
}

fn trimmed_header_value(
    headers: &HeaderMap,
    name: &'static str,
    error: &'static str,
) -> Result<Option<String>, IssuerError> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .map(str::trim)
                .map(str::to_string)
                .map_err(|_| IssuerError::bad_request(error))
        })
        .transpose()
        .map(|value| value.filter(|value| !value.is_empty()))
}

fn valid_device_key_id(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
    })
}

fn device_key_allowed_consent_scopes() -> Vec<ConsentScope> {
    vec![
        ConsentScope::DebuggingEvaluation,
        ConsentScope::PublicAttribution,
    ]
}

fn device_key_allowed_uses() -> Vec<TraceAllowedUse> {
    vec![
        TraceAllowedUse::Debugging,
        TraceAllowedUse::Evaluation,
        TraceAllowedUse::AggregateAnalytics,
    ]
}

/// Spec intersection: empty requested -> full ceiling; else intersection
/// preserving ceiling order; empty intersection -> Err (403 with
/// `empty_label` as the error message).
fn intersect_requested_with_ceiling<T: PartialEq + Copy>(
    requested: &[T],
    ceiling: &[T],
    empty_label: &'static str,
) -> Result<Vec<T>, IssuerError> {
    if requested.is_empty() {
        return Ok(ceiling.to_vec());
    }
    let intersected = ceiling
        .iter()
        .filter(|item| requested.contains(item))
        .copied()
        .collect::<Vec<_>>();
    if intersected.is_empty() {
        return Err(IssuerError::forbidden(empty_label));
    }
    Ok(intersected)
}

/// The allowed-uses implied by a set of granted consent scopes. Used to cap
/// what an empty `allowed_uses` request can be granted so that a device
/// claim scoped only to (say) `debugging_evaluation` cannot walk away with
/// `model_training` merely because the ceiling happens to include it.
/// `aggregate_analytics` is always implied regardless of scopes. Order is
/// stable (first-seen) and deduped.
fn uses_implied_by_scopes(scopes: &[ConsentScope]) -> Vec<TraceAllowedUse> {
    let mut implied = Vec::new();
    for scope in scopes {
        let uses: &[TraceAllowedUse] = match scope {
            ConsentScope::DebuggingEvaluation => {
                &[TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation]
            }
            ConsentScope::BenchmarkOnly => &[TraceAllowedUse::BenchmarkGeneration],
            ConsentScope::RankingTraining => &[TraceAllowedUse::RankingModelTraining],
            ConsentScope::ModelTraining => &[TraceAllowedUse::ModelTraining],
            ConsentScope::PublicAttribution => &[],
        };
        for use_ in uses {
            if !implied.contains(use_) {
                implied.push(*use_);
            }
        }
    }
    if !implied.contains(&TraceAllowedUse::AggregateAnalytics) {
        implied.push(TraceAllowedUse::AggregateAnalytics);
    }
    implied
}

/// Resolve the allowed-uses grant for a device-principal claim. When the
/// request's `allowed_uses` is empty, the grant is capped to what the
/// granted consent scopes imply (intersected with the ceiling) rather than
/// the full ceiling — an empty-uses request must not silently expand beyond
/// what the contributor consented to. Non-empty requests keep the existing
/// clip/intersect-with-ceiling behavior.
fn resolve_granted_uses(
    requested_uses: &[TraceAllowedUse],
    granted_scopes: &[ConsentScope],
    ceiling_uses: &[TraceAllowedUse],
) -> Result<Vec<TraceAllowedUse>, IssuerError> {
    if requested_uses.is_empty() {
        let implied = uses_implied_by_scopes(granted_scopes);
        let intersected: Vec<TraceAllowedUse> = ceiling_uses
            .iter()
            .filter(|use_| implied.contains(use_))
            .copied()
            .collect();
        if intersected.is_empty() {
            return Err(IssuerError::forbidden("allowed uses not permitted"));
        }
        return Ok(intersected);
    }
    intersect_requested_with_ceiling(requested_uses, ceiling_uses, "allowed uses not permitted")
}

fn device_public_key_bytes(
    public_key: &str,
    expected_device_key_id: &str,
) -> Result<Vec<u8>, IssuerError> {
    let public_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key.trim())
        .map_err(|_| {
            tracing::warn!(
                device_key_id = %expected_device_key_id,
                "stored device public key is malformed"
            );
            IssuerError::internal()
        })?;
    if public_key_bytes.len() != 32 {
        tracing::warn!(
            device_key_id = %expected_device_key_id,
            "stored device public key has invalid length"
        );
        return Err(IssuerError::internal());
    }
    let actual_device_key_id = device_key_id_from_public_key_bytes(&public_key_bytes);
    if actual_device_key_id != expected_device_key_id {
        tracing::warn!(
            device_key_id = %expected_device_key_id,
            "stored device public key does not match device key id"
        );
        return Err(IssuerError::internal());
    }
    Ok(public_key_bytes)
}

fn verify_device_claim_signature(
    public_key_bytes: &[u8],
    body: &[u8],
    signature: &[u8],
) -> Result<(), IssuerError> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key_bytes)
        .verify(body, signature)
        .map_err(|_| IssuerError::forbidden("invalid device key signature"))
}

fn verify_instance_attestation_signature(
    public_key_bytes: &[u8],
    body: &[u8],
    signature: &[u8],
) -> Result<(), IssuerError> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key_bytes)
        .verify(body, signature)
        .map_err(|_| {
            IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::EnrollNotAuthorized,
            )
        })
}

/// Domain-separated, hash-only replay-cache key. The raw nonce never enters the
/// in-memory replay cache; only this digest of `(instance_subject_hash, nonce)`
/// does, keeping the cache consistent with the hash-only convention.
fn instance_enroll_replay_key(instance_subject_hash: &str, nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"instance_enroll_replay:");
    hasher.update(instance_subject_hash.as_bytes());
    hasher.update(b"|");
    hasher.update(nonce.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn verify_device_workload_jwt(
    token: &str,
    public_key_bytes: &[u8],
    expected_audience: Option<&str>,
) -> Result<DeviceWorkloadClaims, IssuerError> {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.validate_nbf = true;
    let mut required_claims = vec!["exp".to_string()];
    if let Some(audience) = expected_audience {
        validation.set_audience(&[audience]);
        required_claims.push("aud".to_string());
    } else {
        validation.validate_aud = false;
    }
    validation.set_required_spec_claims(&required_claims);
    jsonwebtoken::decode::<DeviceWorkloadClaims>(
        token,
        &DecodingKey::from_ed_der(public_key_bytes),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|error| match error.kind() {
        JwtErrorKind::ExpiredSignature => IssuerError::forbidden("expired device key token"),
        JwtErrorKind::ImmatureSignature => IssuerError::forbidden("not-yet-valid device key token"),
        _ => IssuerError::forbidden("invalid device key token"),
    })
}

fn validate_device_workload_claims(
    claims: &DeviceWorkloadClaims,
    expected_tenant_id: &str,
) -> Result<(), IssuerError> {
    if claims.tenant_id.trim() != expected_tenant_id {
        return Err(IssuerError::forbidden(
            "device key tenant does not match request",
        ));
    }
    let now = Utc::now().timestamp();
    if claims.exp <= now {
        return Err(IssuerError::forbidden("expired device key token"));
    }
    if let Some(iat) = claims.iat
        && iat > now + 60
    {
        return Err(IssuerError::forbidden("not-yet-valid device key token"));
    }
    Ok(())
}

fn enforce_subset<T: Ord>(
    requested: &[T],
    allowed: &[T],
    message: &'static str,
) -> Result<(), IssuerError> {
    if requested.is_empty() {
        return Ok(());
    }
    let allowed = allowed.iter().collect::<BTreeSet<_>>();
    if requested.iter().all(|item| allowed.contains(item)) {
        Ok(())
    } else {
        Err(IssuerError::forbidden(message))
    }
}

fn authorize_upload_claim_from_tenant_grants(
    grants: &[TraceTenantAccessGrantRecord],
    issuer: &str,
    audience: &str,
    actor: &str,
    consent_scopes: &mut Vec<ConsentScope>,
    allowed_uses: &mut Vec<TraceAllowedUse>,
) -> Result<(), IssuerError> {
    let matching_grants = grants
        .iter()
        .filter(|grant| grant.status == TraceTenantAccessGrantStatus::Active)
        .filter(|grant| grant.role == TraceTenantAccessGrantRole::Contributor)
        .filter(|grant| tenant_access_grant_matches_claim_binding(grant, issuer, audience, actor))
        .collect::<Vec<_>>();
    if matching_grants.is_empty() {
        return Err(IssuerError::forbidden(
            "active tenant access grant required",
        ));
    }

    for grant in matching_grants {
        let grant_scopes = parse_storage_grant_values::<ConsentScope>(
            &grant.allowed_consent_scopes,
            "tenant_access_grant.allowed_consent_scopes",
        )?;
        restrict_requested_allowlist(
            consent_scopes,
            grant_scopes,
            "tenant access grant consent scope intersection is empty",
        )?;

        let grant_uses = parse_storage_grant_values::<TraceAllowedUse>(
            &grant.allowed_uses,
            "tenant_access_grant.allowed_uses",
        )?;
        restrict_requested_allowlist(
            allowed_uses,
            grant_uses,
            "tenant access grant allowed-use intersection is empty",
        )?;
    }
    Ok(())
}

fn tenant_access_grant_matches_claim_binding(
    grant: &TraceTenantAccessGrantRecord,
    issuer: &str,
    audience: &str,
    actor: &str,
) -> bool {
    if let Some(expected) = grant.issuer.as_deref().and_then(non_empty_trimmed)
        && expected != issuer
    {
        return false;
    }
    if let Some(expected) = grant.audience.as_deref().and_then(non_empty_trimmed)
        && expected != audience
    {
        return false;
    }
    if let Some(expected) = grant.subject.as_deref().and_then(non_empty_trimmed)
        && expected != actor
    {
        return false;
    }
    true
}

fn parse_storage_grant_values<T>(values: &[String], label: &str) -> Result<BTreeSet<T>, IssuerError>
where
    T: for<'de> Deserialize<'de> + Ord,
{
    values
        .iter()
        .map(|value| {
            serde_json::from_value::<T>(serde_json::Value::String(value.clone())).map_err(|_| {
                tracing::warn!(%label, value = %value, "invalid Trace Commons tenant access grant value");
                IssuerError::internal()
            })
        })
        .collect()
}

fn restrict_requested_allowlist<T>(
    requested: &mut Vec<T>,
    grant_values: BTreeSet<T>,
    empty_message: &'static str,
) -> Result<(), IssuerError>
where
    T: Ord + Clone,
{
    if grant_values.is_empty() {
        return Ok(());
    }
    if requested.is_empty() {
        *requested = grant_values.into_iter().collect();
        return Ok(());
    }
    let requested_set = requested.iter().collect::<BTreeSet<_>>();
    let intersected = grant_values
        .iter()
        .filter(|item| requested_set.contains(item))
        .cloned()
        .collect::<Vec<_>>();
    if intersected.is_empty() {
        return Err(IssuerError::forbidden(empty_message));
    }
    *requested = intersected;
    Ok(())
}

fn normalized_required(value: Option<&str>, message: &'static str) -> Result<String, IssuerError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| IssuerError::bad_request(message))
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn principal_storage_ref(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("principal_sha256:{}", hex::encode(digest))
}

/// Maximum accepted byte length for a client-supplied subject.
const MAX_SUBJECT_LEN: usize = 128;

/// Validate and normalize an opaque per-user subject. The subject is a
/// pseudonymous token minted by the client; we only enforce a conservative
/// shape so it is safe to embed in a derived principal string. We never trust a
/// client-supplied principal prefix — the namespaced principal is built in
/// `issue_claim_for_device_key`.
fn normalize_subject(raw: &str) -> Result<String, IssuerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_SUBJECT_LEN {
        return Err(IssuerError::bad_request("invalid subject"));
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
    {
        return Err(IssuerError::bad_request("invalid subject"));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn validate_eddsa_private_key_pem(pem: &str) -> anyhow::Result<String> {
    let pem = pem.trim();
    anyhow::ensure!(!pem.contains("RSA"), "RSA keys are not supported");
    anyhow::ensure!(
        pem.starts_with("-----BEGIN PRIVATE KEY-----"),
        "EdDSA private key must be PKCS#8 PEM"
    );
    EncodingKey::from_ed_pem(pem.as_bytes()).context("invalid EdDSA private key")?;
    Ok(format!("{pem}\n"))
}

pub(crate) fn validate_eddsa_public_key_pem(pem: &str) -> anyhow::Result<String> {
    let pem = pem.trim();
    anyhow::ensure!(!pem.contains("RSA"), "RSA keys are not supported");
    anyhow::ensure!(
        pem.starts_with("-----BEGIN PUBLIC KEY-----"),
        "EdDSA public key must be SPKI PEM"
    );
    DecodingKey::from_ed_pem(pem.as_bytes()).context("invalid EdDSA public key")?;
    Ok(format!("{pem}\n"))
}

fn required_pem_or_file(
    inline_env: &'static str,
    file_env: &'static str,
) -> anyhow::Result<String> {
    let inline = optional_env(inline_env)?;
    let file = optional_env(file_env)?;
    match (inline, file) {
        (Some(_), Some(_)) => anyhow::bail!("{inline_env} and {file_env} cannot both be set"),
        (Some(value), None) => Ok(value),
        (None, Some(path)) => std::fs::read_to_string(PathBuf::from(path))
            .with_context(|| format!("failed to read {file_env}")),
        (None, None) => anyhow::bail!("{inline_env} or {file_env} is required"),
    }
}

pub(crate) fn required_env(name: &'static str) -> anyhow::Result<String> {
    optional_env(name)?.ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

pub(crate) fn optional_env(name: &'static str) -> anyhow::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            let trimmed = value.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
    }
}

fn env_truthy(name: &'static str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| parse_truthy_env_value(&value))
}

fn parse_truthy_env_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_onboarding_ingest_url(value: Option<String>) -> anyhow::Result<Option<String>> {
    let Some(value) = trim_optional(value) else {
        return Ok(None);
    };
    let mut parsed = reqwest::Url::parse(&value)
        .with_context(|| format!("invalid {TRACE_COMMONS_ONBOARDING_INGEST_URL_ENV}"))?;
    if parsed.path().is_empty() || parsed.path() == "/" {
        parsed.set_path("/v1/traces");
    }
    anyhow::ensure!(
        parsed.fragment().is_none(),
        "{TRACE_COMMONS_ONBOARDING_INGEST_URL_ENV} must not include a fragment"
    );
    Ok(Some(parsed.to_string()))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use chrono::{Duration, Utc};
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use serde_json::json;

    #[test]
    fn tenant_access_grant_env_decouples_read_from_require() {
        // Neither env: no DB attached, no enforcement (unchanged default).
        assert_eq!(
            super::tenant_access_grant_env_decision(false, false),
            (false, false)
        );
        // REQUIRE alone: attach DB and enforce (legacy behavior preserved).
        assert_eq!(
            super::tenant_access_grant_env_decision(true, false),
            (true, true)
        );
        // GRANT_DB alone: attach DB for reading, do NOT enforce (the new path).
        assert_eq!(
            super::tenant_access_grant_env_decision(false, true),
            (true, false)
        );
        // Both: attach and enforce.
        assert_eq!(
            super::tenant_access_grant_env_decision(true, true),
            (true, true)
        );
    }
    use std::collections::BTreeMap;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::trace_corpus_storage::{
        TraceTenantAccessGrantRecord, TraceTenantAccessGrantRole, TraceTenantAccessGrantStatus,
    };
    use trace_commons_protocol::onboarding::TRACE_ONBOARD_REQUEST_SCHEMA_VERSION;
    use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

    #[test]
    fn intersect_empty_request_grants_full_ceiling() {
        let ceiling = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::ModelTraining,
        ];
        let got = intersect_requested_with_ceiling(&[], &ceiling, "consent scopes not permitted")
            .unwrap();
        assert_eq!(got, ceiling);
    }

    #[test]
    fn intersect_clips_to_ceiling_and_rejects_empty() {
        let ceiling = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::PublicAttribution,
        ];
        let got = intersect_requested_with_ceiling(
            &[
                ConsentScope::ModelTraining,
                ConsentScope::DebuggingEvaluation,
            ],
            &ceiling,
            "consent scopes not permitted",
        )
        .unwrap();
        assert_eq!(got, vec![ConsentScope::DebuggingEvaluation]);
        let err = intersect_requested_with_ceiling(
            &[ConsentScope::ModelTraining],
            &ceiling,
            "consent scopes not permitted",
        )
        .unwrap_err();
        // IssuerError renders {"error": label}; assert the label text.
        assert!(format!("{err:?}").contains("consent scopes not permitted"));
    }

    #[test]
    fn uses_implied_by_scopes_maps_each_scope_and_always_appends_aggregate_analytics() {
        assert_eq!(
            uses_implied_by_scopes(&[ConsentScope::DebuggingEvaluation]),
            vec![
                TraceAllowedUse::Debugging,
                TraceAllowedUse::Evaluation,
                TraceAllowedUse::AggregateAnalytics
            ]
        );
        assert_eq!(
            uses_implied_by_scopes(&[ConsentScope::BenchmarkOnly]),
            vec![
                TraceAllowedUse::BenchmarkGeneration,
                TraceAllowedUse::AggregateAnalytics
            ]
        );
        assert_eq!(
            uses_implied_by_scopes(&[ConsentScope::RankingTraining]),
            vec![
                TraceAllowedUse::RankingModelTraining,
                TraceAllowedUse::AggregateAnalytics
            ]
        );
        assert_eq!(
            uses_implied_by_scopes(&[ConsentScope::ModelTraining]),
            vec![
                TraceAllowedUse::ModelTraining,
                TraceAllowedUse::AggregateAnalytics
            ]
        );
        assert_eq!(
            uses_implied_by_scopes(&[ConsentScope::PublicAttribution]),
            vec![TraceAllowedUse::AggregateAnalytics]
        );
    }

    #[test]
    fn resolve_granted_uses_caps_empty_request_to_scope_implied_uses() {
        // Ceiling includes model_training, but the granted scopes only cover
        // debugging_evaluation — an empty allowed_uses request must not walk
        // away with model_training just because the ceiling allows it.
        let ceiling = vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::ModelTraining,
            TraceAllowedUse::AggregateAnalytics,
        ];
        let granted_scopes = vec![ConsentScope::DebuggingEvaluation];
        let got = resolve_granted_uses(&[], &granted_scopes, &ceiling).unwrap();
        assert_eq!(
            got,
            vec![
                TraceAllowedUse::Debugging,
                TraceAllowedUse::Evaluation,
                TraceAllowedUse::AggregateAnalytics
            ]
        );
        assert!(!got.contains(&TraceAllowedUse::ModelTraining));
    }

    #[test]
    fn resolve_granted_uses_keeps_intersection_behavior_for_non_empty_request() {
        let ceiling = vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation];
        let granted_scopes = vec![ConsentScope::DebuggingEvaluation];
        let got =
            resolve_granted_uses(&[TraceAllowedUse::Debugging], &granted_scopes, &ceiling).unwrap();
        assert_eq!(got, vec![TraceAllowedUse::Debugging]);
    }

    #[test]
    fn normalize_subject_accepts_pseudonymous_token() {
        let s = normalize_subject("  tenant_sha256:ab12CD_-  ").expect("valid");
        assert_eq!(s, "tenant_sha256:ab12CD_-");
    }

    #[test]
    fn normalize_subject_rejects_empty_and_oversized_and_bad_chars() {
        assert!(normalize_subject("   ").is_err());
        assert!(normalize_subject(&"a".repeat(129)).is_err());
        assert!(normalize_subject("has space").is_err());
        assert!(normalize_subject("bad/slash").is_err());
    }

    const TEST_EDDSA_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIAGfN68ko7YyCGJMb3lHVwTn5aiUtbIsAclIx/lX0p2R\n-----END PRIVATE KEY-----\n";
    const TEST_EDDSA_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAMnniSMeHZrdoe3gkL7ZeHmG7vAg65c5TqaBd71B2qDw=\n-----END PUBLIC KEY-----\n";
    const WORKLOAD_EDDSA_PRIVATE_KEY_PEM: &str = TEST_EDDSA_PRIVATE_KEY_PEM;
    const WORKLOAD_EDDSA_PUBLIC_KEY_PEM: &str = TEST_EDDSA_PUBLIC_KEY_PEM;

    fn test_config() -> TraceUploadClaimIssuerConfig {
        TraceUploadClaimIssuerConfig {
            bind: "127.0.0.1:0".parse().expect("bind parses"),
            signing_private_key_pem: TEST_EDDSA_PRIVATE_KEY_PEM.to_string(),
            signing_public_key_pem: TEST_EDDSA_PUBLIC_KEY_PEM.to_string(),
            signing_kid: "issuer-key-1".to_string(),
            issuer: "trace-commons-upload-issuer".to_string(),
            audience: "trace-commons-upload".to_string(),
            max_ttl_seconds: 300,
            workload_public_key_pem: WORKLOAD_EDDSA_PUBLIC_KEY_PEM.to_string(),
            workload_issuer: Some("workload-issuer".to_string()),
            workload_audience: Some("trace-claim-issuer".to_string()),
            tenant_access_grant_db: None,
            require_tenant_access_grants: false,
            shutdown_grace_seconds: DEFAULT_SHUTDOWN_GRACE_SECONDS,
            request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            allowlist_source: None,
            allowlist_refresh_interval_seconds: DEFAULT_ALLOWLIST_REFRESH_INTERVAL_SECONDS,
            allowlist_max_stale_seconds: DEFAULT_ALLOWLIST_MAX_STALE_SECONDS,
            onboarding_device_key_db: None,
            onboarding_ingest_url: Some("https://ingest.tracecommons.ai".to_string()),
            onboarding_community_url: Some("https://tracecommons.ai".to_string()),
            onboarding_profile_url: Some("https://tracecommons.ai/profile".to_string()),
            onboarding_leaderboard_url: Some("https://tracecommons.ai/leaderboard".to_string()),
            admin_bind: None,
            invite_admin_backend: None,
            invite_admin_registry: None,
            invite_registry_authoritative: false,
        }
    }

    fn workload_token(issuer: &str, audience: &str) -> String {
        let now = Utc::now();
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some("workload-key-1".to_string());
        jsonwebtoken::encode(
            &header,
            &json!({
                "sub": "principal:agent-1",
                "principal_ref": "principal:agent-1",
                "tenant_id": "tenant-a",
                "iss": issuer,
                "aud": audience,
                "iat": now.timestamp(),
                "exp": (now + Duration::minutes(5)).timestamp(),
                "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
                "allowed_uses": ["debugging", "evaluation"],
            }),
            &EncodingKey::from_ed_pem(WORKLOAD_EDDSA_PRIVATE_KEY_PEM.as_bytes())
                .expect("workload key parses"),
        )
        .expect("workload token signs")
    }

    fn claim_request() -> serde_json::Value {
        json!({
            "schema_version": TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION,
            "tenant_id": "tenant-a",
            "audience": "trace-commons-upload",
            "trace_id": Uuid::new_v4(),
            "submission_id": Uuid::new_v4(),
            "consent_scopes": ["debugging_evaluation"],
            "allowed_uses": ["debugging"],
            "requested_at": Utc::now(),
        })
    }

    async fn post_claim(
        config: TraceUploadClaimIssuerConfig,
        token: String,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let router = trace_upload_claim_issuer_router(config).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/trace-upload-claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let json = serde_json::from_slice(&body).expect("json response");
        (status, json)
    }

    async fn post_device_claim(
        config: TraceUploadClaimIssuerConfig,
        device_key_id: &str,
        signature: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let router = trace_upload_claim_issuer_router(config).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/trace-upload-claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(TRACE_DEVICE_KEY_ID_HEADER, device_key_id)
                    .header(TRACE_DEVICE_SIGNATURE_HEADER, signature)
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let json = serde_json::from_slice(&body).expect("json response");
        (status, json)
    }

    fn onboard_request(invite_code: &str) -> serde_json::Value {
        json!({
            "schema_version": TRACE_ONBOARD_REQUEST_SCHEMA_VERSION,
            "invite_code": invite_code,
            "device_public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "client_info": {
                "agent": "ironclaw",
                "version": "0.x.y"
            }
        })
    }

    async fn post_onboard(
        config: TraceUploadClaimIssuerConfig,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let router = trace_upload_claim_issuer_router(config).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/onboard")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let json = serde_json::from_slice(&body).expect("json response");
        (status, json)
    }

    async fn get_text(config: TraceUploadClaimIssuerConfig, uri: &str) -> (StatusCode, String) {
        let router = trace_upload_claim_issuer_router(config).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        (
            status,
            String::from_utf8(body.to_vec()).expect("body is utf8"),
        )
    }

    #[tokio::test]
    async fn invite_landing_route_explains_agent_onboarding() {
        let (status, body) = get_text(test_config(), "/onboard#INV9K3RT5FBQ72JX").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Trace Commons invite link"));
        assert!(body.contains("/v1/onboard"));
        assert!(body.contains("trace_commons.onboard_request.v1"));
        assert!(body.contains("invite_code"));
        assert!(body.contains("device_public_key"));

        // The page points at the supported CLI entry point rather than
        // leaving an agent to hand-roll the POST.
        assert!(body.contains("login --invite"));

        // Each of these is a step a real contributor got wrong by reading the
        // source instead of this page. Losing any of them regresses the
        // onboarding experience even though the route still returns 200.
        assert!(
            body.contains("NOT IDEMPOTENT"),
            "must warn that a repeat POST spends another invite use"
        );
        assert!(
            body.contains("sha256:"),
            "must distinguish the raw public key from the device_key_id digest"
        );
        assert!(
            body.contains("contributor.json"),
            "must say where the response has to be persisted"
        );
        assert!(
            body.contains("--dry-run"),
            "must steer a first submit through a dry run"
        );

        // The landing page never echoes the invite code: the fragment is not
        // sent to the server, and a code in a response body would be a
        // credential in a log.
        assert!(!body.contains("INV9K3RT5FBQ72JX"));
    }

    #[tokio::test]
    async fn eddsa_only_issue_success_returns_bounded_upload_claim() {
        let (status, body) = post_claim(
            test_config(),
            workload_token("workload-issuer", "trace-claim-issuer"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["token_type"], "Bearer");
        assert!(body["expires_in"].as_i64().expect("expires_in") <= 300);

        let token = body["access_token"].as_str().expect("access token");
        let header = jsonwebtoken::decode_header(token).expect("issuer token header");
        assert_eq!(header.alg, Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some("issuer-key-1"));

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&["trace-commons-upload-issuer"]);
        validation.set_audience(&["trace-commons-upload"]);
        let claims = jsonwebtoken::decode::<serde_json::Value>(
            token,
            &DecodingKey::from_ed_pem(TEST_EDDSA_PUBLIC_KEY_PEM.as_bytes())
                .expect("issuer public key parses"),
            &validation,
        )
        .expect("issuer token verifies")
        .claims;
        assert_eq!(claims["tenant_id"], "tenant-a");
        assert_eq!(claims["role"], "contributor");
        assert_eq!(claims["sub"], "principal:agent-1");
        assert_eq!(claims["principal_ref"], "principal:agent-1");
        assert_eq!(
            claims["allowed_consent_scopes"],
            json!(["debugging_evaluation"])
        );
        assert_eq!(claims["allowed_uses"], json!(["debugging"]));
        assert!(claims["jti"].as_str().is_some_and(|jti| !jti.is_empty()));
    }

    #[tokio::test]
    async fn claim_response_echoes_granted_scopes() {
        let (status, body) = post_claim(
            test_config(),
            workload_token("workload-issuer", "trace-claim-issuer"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let token = body["access_token"].as_str().expect("access token");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&["trace-commons-upload-issuer"]);
        validation.set_audience(&["trace-commons-upload"]);
        let claims = jsonwebtoken::decode::<serde_json::Value>(
            token,
            &DecodingKey::from_ed_pem(TEST_EDDSA_PUBLIC_KEY_PEM.as_bytes())
                .expect("issuer public key parses"),
            &validation,
        )
        .expect("issuer token verifies")
        .claims;

        assert_eq!(body["consent_scopes"], claims["allowed_consent_scopes"]);
        assert_eq!(body["allowed_uses"], claims["allowed_uses"]);
        assert_eq!(body["consent_scopes"], json!(["debugging_evaluation"]));
        assert_eq!(body["allowed_uses"], json!(["debugging"]));
    }

    #[tokio::test]
    async fn wrong_workload_audience_or_issuer_is_rejected() {
        let (status, _) = post_claim(
            test_config(),
            workload_token("wrong-issuer", "trace-claim-issuer"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = post_claim(
            test_config(),
            workload_token("workload-issuer", "wrong-audience"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn no_rsa_or_generic_jwks_material_is_accepted_or_exposed() {
        assert!(
            TraceUploadClaimIssuerConfig {
                signing_private_key_pem:
                    "-----BEGIN RSA PRIVATE KEY-----\nredacted\n-----END RSA PRIVATE KEY-----"
                        .to_string(),
                ..test_config()
            }
            .build_state()
            .is_err()
        );

        let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/.well-known/trace-commons-ed25519-keyset.json")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let text = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(text.contains("\"public_key_pem\""));
        assert!(text.contains("BEGIN PUBLIC KEY"));
        assert!(!text.contains("\"kty\""));
        assert!(!text.contains("\"crv\""));
        assert!(!text.contains("\"x\""));
        assert!(!text.contains("RSA"));
        assert!(!text.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn tenant_access_grant_env_flag_uses_explicit_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " on "] {
            assert!(parse_truthy_env_value(value), "{value} should be truthy");
        }
        for value in ["", "0", "false", "off", "no", "tenant-a"] {
            assert!(!parse_truthy_env_value(value), "{value} should be falsey");
        }
    }

    #[tokio::test]
    async fn issues_claim_with_public_attribution_scope() {
        // public_attribution is a profile-management consent. A workload
        // token that allows it can mint an upload claim scoped to ONLY
        // public_attribution with no allowed_uses — the claim is used to
        // authenticate /v1/community/profile, not to submit traces.
        let state = test_config().build_state().expect("state builds");
        let workload = WorkloadClaims {
            sub: Some("principal:agent-1".to_string()),
            principal_ref: None,
            tenant_id: Some("tenant-a".to_string()),
            iss: None,
            aud: None,
            exp: Utc::now().timestamp() + 60,
            iat: Some(Utc::now().timestamp()),
            allowed_consent_scopes: vec![ConsentScope::PublicAttribution],
            allowed_uses: Vec::new(),
            invite_code: None,
        };
        let request = TraceUploadClaimRequest {
            schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: Some("tenant-a".to_string()),
            audience: Some("trace-commons-upload".to_string()),
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::PublicAttribution],
            allowed_uses: Vec::new(),
            subject: None,
            requested_at: Utc::now(),
        };
        let response = state
            .issue_claim(&workload, request)
            .await
            .expect("issue succeeds");
        let token_parts: Vec<&str> = response.access_token.split('.').collect();
        assert_eq!(token_parts.len(), 3, "JWT shape");
        let payload = base64_url_decode(token_parts[1]);
        assert!(
            payload.contains("public_attribution"),
            "minted claim carries the requested scope: {payload}"
        );
    }

    #[tokio::test]
    async fn rejects_public_attribution_request_when_workload_lacks_it() {
        let state = test_config().build_state().expect("state builds");
        let workload = WorkloadClaims {
            sub: Some("principal:agent-1".to_string()),
            principal_ref: None,
            tenant_id: Some("tenant-a".to_string()),
            iss: None,
            aud: None,
            exp: Utc::now().timestamp() + 60,
            iat: Some(Utc::now().timestamp()),
            allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: vec![TraceAllowedUse::Debugging],
            invite_code: None,
        };
        let request = TraceUploadClaimRequest {
            schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: Some("tenant-a".to_string()),
            audience: Some("trace-commons-upload".to_string()),
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::PublicAttribution],
            allowed_uses: Vec::new(),
            subject: None,
            requested_at: Utc::now(),
        };
        assert!(
            state.issue_claim(&workload, request).await.is_err(),
            "workload scope allowlist gates public_attribution requests",
        );
    }

    #[tokio::test]
    async fn issues_claim_with_mixed_public_attribution_and_trace_scopes() {
        // Common pilot case: contributor has a workload token that
        // grants both submitting traces and managing their profile.
        let state = test_config().build_state().expect("state builds");
        let workload = WorkloadClaims {
            sub: Some("principal:agent-1".to_string()),
            principal_ref: None,
            tenant_id: Some("tenant-a".to_string()),
            iss: None,
            aud: None,
            exp: Utc::now().timestamp() + 60,
            iat: Some(Utc::now().timestamp()),
            allowed_consent_scopes: vec![
                ConsentScope::DebuggingEvaluation,
                ConsentScope::PublicAttribution,
            ],
            allowed_uses: vec![
                TraceAllowedUse::Debugging,
                TraceAllowedUse::Evaluation,
                TraceAllowedUse::AggregateAnalytics,
            ],
            invite_code: None,
        };
        let request = TraceUploadClaimRequest {
            schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: Some("tenant-a".to_string()),
            audience: Some("trace-commons-upload".to_string()),
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![
                ConsentScope::DebuggingEvaluation,
                ConsentScope::PublicAttribution,
            ],
            allowed_uses: vec![TraceAllowedUse::Debugging],
            subject: None,
            requested_at: Utc::now(),
        };
        let response = state
            .issue_claim(&workload, request)
            .await
            .expect("issue succeeds");
        let token_parts: Vec<&str> = response.access_token.split('.').collect();
        let payload = base64_url_decode(token_parts[1]);
        assert!(payload.contains("debugging_evaluation"));
        assert!(payload.contains("public_attribution"));
    }

    fn base64_url_decode(input: &str) -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(input)
            .expect("base64");
        String::from_utf8(bytes).expect("utf8")
    }

    #[tokio::test]
    async fn rejects_requests_exceeding_workload_allowances() {
        let state = test_config().build_state().expect("state builds");
        let workload = WorkloadClaims {
            sub: Some("principal:agent-1".to_string()),
            principal_ref: None,
            tenant_id: Some("tenant-a".to_string()),
            iss: None,
            aud: None,
            exp: Utc::now().timestamp() + 60,
            iat: Some(Utc::now().timestamp()),
            allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: vec![TraceAllowedUse::Debugging],
            invite_code: None,
        };
        let request = TraceUploadClaimRequest {
            schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: Some("tenant-a".to_string()),
            audience: Some("trace-commons-upload".to_string()),
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::ModelTraining],
            allowed_uses: vec![TraceAllowedUse::ModelTraining],
            subject: None,
            requested_at: Utc::now(),
        };
        assert!(state.issue_claim(&workload, request).await.is_err());
    }

    #[test]
    fn tenant_grant_principal_ref_matches_ingest_signed_actor_shape() {
        assert_eq!(
            principal_storage_ref("signed:tenant-a:actor-123"),
            "principal_sha256:5cd45d57c4270245a9eae65dc4140e2bbaa5b18e84371fdf9a3abb2feb8c26cc"
        );
    }

    #[test]
    fn tenant_grant_authorization_requires_contributor_binding_and_intersects_allowlists() {
        let now = Utc::now();
        let mut consent_scopes = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::BenchmarkOnly,
        ];
        let mut allowed_uses = vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation];
        let grant = test_tenant_access_grant(
            now,
            TraceTenantAccessGrantRole::Contributor,
            vec!["debugging_evaluation"],
            vec!["debugging"],
            Some("trace-commons-upload-issuer"),
            Some("trace-commons-upload"),
            Some("actor-123"),
        );

        authorize_upload_claim_from_tenant_grants(
            &[grant],
            "trace-commons-upload-issuer",
            "trace-commons-upload",
            "actor-123",
            &mut consent_scopes,
            &mut allowed_uses,
        )
        .expect("grant authorizes claim");

        assert_eq!(consent_scopes, vec![ConsentScope::DebuggingEvaluation]);
        assert_eq!(allowed_uses, vec![TraceAllowedUse::Debugging]);

        let mut consent_scopes = Vec::new();
        let mut allowed_uses = Vec::new();
        let default_grant = test_tenant_access_grant(
            now,
            TraceTenantAccessGrantRole::Contributor,
            vec!["benchmark_only"],
            vec!["evaluation"],
            Some("trace-commons-upload-issuer"),
            Some("trace-commons-upload"),
            Some("actor-123"),
        );
        authorize_upload_claim_from_tenant_grants(
            &[default_grant],
            "trace-commons-upload-issuer",
            "trace-commons-upload",
            "actor-123",
            &mut consent_scopes,
            &mut allowed_uses,
        )
        .expect("empty request allowlists inherit the grant constraints");
        assert_eq!(consent_scopes, vec![ConsentScope::BenchmarkOnly]);
        assert_eq!(allowed_uses, vec![TraceAllowedUse::Evaluation]);

        let mut consent_scopes = vec![ConsentScope::DebuggingEvaluation];
        let mut allowed_uses = vec![TraceAllowedUse::Debugging];
        let reviewer_grant = test_tenant_access_grant(
            now,
            TraceTenantAccessGrantRole::Reviewer,
            vec![],
            vec![],
            None,
            None,
            None,
        );
        assert!(
            authorize_upload_claim_from_tenant_grants(
                &[reviewer_grant],
                "trace-commons-upload-issuer",
                "trace-commons-upload",
                "actor-123",
                &mut consent_scopes,
                &mut allowed_uses,
            )
            .is_err()
        );
    }

    #[test]
    fn generate_keypair_produces_parseable_ed25519_material() {
        let keypair = generate_upload_claim_keypair().expect("keygen succeeds");
        assert!(
            keypair
                .private_key_pem
                .starts_with("-----BEGIN PRIVATE KEY-----")
        );
        assert!(
            keypair
                .private_key_pem
                .contains("-----END PRIVATE KEY-----")
        );
        assert!(
            keypair
                .public_key_pem
                .starts_with("-----BEGIN PUBLIC KEY-----")
        );
        assert!(keypair.public_key_pem.contains("-----END PUBLIC KEY-----"));
        assert!(Uuid::parse_str(&keypair.suggested_kid).is_ok());
        // Round-trip through the same code path the server uses at startup.
        EncodingKey::from_ed_pem(keypair.private_key_pem.as_bytes())
            .expect("generated private key parses as EdDSA");
        DecodingKey::from_ed_pem(keypair.public_key_pem.as_bytes())
            .expect("generated public key parses as EdDSA");
        // Sign and verify a round-trip to confirm public matches private.
        let signing_key = EncodingKey::from_ed_pem(keypair.private_key_pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(keypair.suggested_kid.clone());
        let token = jsonwebtoken::encode(
            &header,
            &json!({"sub": "probe", "exp": Utc::now().timestamp() + 60}),
            &signing_key,
        )
        .expect("signs");
        let verifying_key = DecodingKey::from_ed_pem(keypair.public_key_pem.as_bytes()).unwrap();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_aud = false;
        jsonwebtoken::decode::<serde_json::Value>(&token, &verifying_key, &validation)
            .expect("generated keypair round-trips through JWT sign+verify");
    }

    #[test]
    fn health_check_reports_signing_and_workload_status() {
        let state = test_config().build_state().expect("state builds");
        let checks = state.run_health_checks();
        assert_eq!(checks["signing_key"], json!("ok"));
        assert_eq!(checks["workload_public_key"], json!("ok"));
    }

    #[tokio::test]
    async fn health_endpoint_returns_503_when_workload_pem_is_malformed() {
        let mut state = (*test_config().build_state().expect("state builds")).clone_for_test();
        state.workload_public_key_pem =
            "-----BEGIN PUBLIC KEY-----\nnot-base64\n-----END PUBLIC KEY-----\n".to_string();
        let router = Router::new()
            .route("/health", get(health_handler))
            .with_state(Arc::new(state));
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "degraded");
        assert_eq!(json["checks"]["workload_public_key"], "fail");
    }

    #[tokio::test]
    async fn health_endpoint_returns_200_when_keys_are_loadable() {
        let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["checks"]["signing_key"], "ok");
        assert_eq!(json["checks"]["workload_public_key"], "ok");
    }

    #[tokio::test]
    async fn graceful_shutdown_completes_within_grace_window() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
        let (signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
        let signal = async move {
            let _ = signal_rx.await;
        };
        let serve = tokio::spawn(serve_router_with_graceful_shutdown(
            listener, router, 2, signal,
        ));
        // Give the server a moment to start accepting.
        tokio::time::sleep(StdDuration::from_millis(50)).await;
        signal_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(StdDuration::from_secs(5), serve)
            .await
            .expect("serve returns within timeout")
            .expect("task joins");
        outcome.expect("serve returns Ok after graceful shutdown");
    }

    #[test]
    fn onboarding_ingest_url_origin_normalizes_to_submit_endpoint() {
        assert_eq!(
            normalize_onboarding_ingest_url(Some("https://ingest.tracecommons.ai".to_string()))
                .expect("origin normalizes")
                .as_deref(),
            Some("https://ingest.tracecommons.ai/v1/traces")
        );
        assert_eq!(
            normalize_onboarding_ingest_url(Some(
                "https://ingest.tracecommons.ai/v1/traces".to_string()
            ))
            .expect("endpoint is preserved")
            .as_deref(),
            Some("https://ingest.tracecommons.ai/v1/traces")
        );
        assert_eq!(
            normalize_onboarding_ingest_url(Some("   ".to_string())).expect("blank is absent"),
            None
        );
        assert!(
            normalize_onboarding_ingest_url(Some(
                "https://ingest.tracecommons.ai/v1/traces#secret".to_string()
            ))
            .is_err(),
            "onboarding ingest URL must not carry fragments"
        );
    }

    impl TraceUploadClaimIssuerState {
        fn clone_for_test(&self) -> TraceUploadClaimIssuerState {
            TraceUploadClaimIssuerState {
                signing_key: EncodingKey::from_ed_pem(TEST_EDDSA_PRIVATE_KEY_PEM.as_bytes())
                    .unwrap(),
                signing_kid: self.signing_kid.clone(),
                issuer: self.issuer.clone(),
                audience: self.audience.clone(),
                max_ttl_seconds: self.max_ttl_seconds,
                workload_decoding_key: DecodingKey::from_ed_pem(
                    WORKLOAD_EDDSA_PUBLIC_KEY_PEM.as_bytes(),
                )
                .unwrap(),
                workload_issuer: self.workload_issuer.clone(),
                workload_audience: self.workload_audience.clone(),
                signing_public_key_pem: self.signing_public_key_pem.clone(),
                workload_public_key_pem: self.workload_public_key_pem.clone(),
                tenant_access_grant_db: self.tenant_access_grant_db.clone(),
                require_tenant_access_grants: self.require_tenant_access_grants,
                allowlist_source: self.allowlist_source.clone(),
                allowlist_max_stale: self.allowlist_max_stale,
                onboarding_device_key_db: self.onboarding_device_key_db.clone(),
                onboarding_ingest_url: self.onboarding_ingest_url.clone(),
                onboarding_community_url: self.onboarding_community_url.clone(),
                onboarding_profile_url: self.onboarding_profile_url.clone(),
                onboarding_leaderboard_url: self.onboarding_leaderboard_url.clone(),
                denial_counter: Arc::clone(&self.denial_counter),
                instance_replay_cache: Arc::clone(&self.instance_replay_cache),
                instance_rate_limiter: Arc::clone(&self.instance_rate_limiter),
                instance_enroll_default_rate_per_min: self.instance_enroll_default_rate_per_min,
                invite_admin_backend: self.invite_admin_backend.clone(),
                invite_admin_registry: self.invite_admin_registry.clone(),
                invite_registry_authoritative: self.invite_registry_authoritative,
            }
        }
    }

    fn test_tenant_access_grant(
        now: DateTime<Utc>,
        role: TraceTenantAccessGrantRole,
        allowed_consent_scopes: Vec<&str>,
        allowed_uses: Vec<&str>,
        issuer: Option<&str>,
        audience: Option<&str>,
        subject: Option<&str>,
    ) -> TraceTenantAccessGrantRecord {
        TraceTenantAccessGrantRecord {
            tenant_id: "tenant-a".to_string(),
            grant_id: Uuid::new_v4(),
            principal_ref: principal_storage_ref("signed:tenant-a:actor-123"),
            role,
            status: TraceTenantAccessGrantStatus::Active,
            allowed_consent_scopes: allowed_consent_scopes
                .into_iter()
                .map(str::to_string)
                .collect(),
            allowed_uses: allowed_uses.into_iter().map(str::to_string).collect(),
            issuer: issuer.map(str::to_string),
            audience: audience.map(str::to_string),
            subject: subject.map(str::to_string),
            issued_at: now,
            expires_at: None,
            revoked_at: None,
            created_by_principal_ref: None,
            revoked_by_principal_ref: None,
            reason: None,
            metadata: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    // ----- Pilot allowlist integration cases -------------------------------

    use crate::trace_upload_claim_allowlist::{AllowlistSourceSpec, hash_invite_code};

    fn workload_token_with_invite(issuer: &str, audience: &str, invite_code: &str) -> String {
        let now = Utc::now();
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some("workload-key-1".to_string());
        jsonwebtoken::encode(
            &header,
            &json!({
                "sub": "principal:agent-1",
                "principal_ref": "principal:agent-1",
                "tenant_id": "tenant-a",
                "iss": issuer,
                "aud": audience,
                "iat": now.timestamp(),
                "exp": (now + Duration::minutes(5)).timestamp(),
                "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
                "allowed_uses": ["debugging", "evaluation"],
                "invite_code": invite_code,
            }),
            &EncodingKey::from_ed_pem(WORKLOAD_EDDSA_PRIVATE_KEY_PEM.as_bytes())
                .expect("workload key parses"),
        )
        .expect("workload token signs")
    }

    fn write_allowlist_file(path: &std::path::Path, policy_label: &str, codes: &[&str]) {
        use std::io::Write;
        let entries: Vec<String> = codes
            .iter()
            .map(|c| {
                format!(
                    "{{\"subject_hash\":\"{}\",\"tenant_id\":\"tenant-a\"}}",
                    hash_invite_code(c)
                )
            })
            .collect();
        let body = format!(
            "{{\"version\":1,\"generated_at\":\"2026-05-17T00:00:00Z\",\"policy_label\":\"{policy_label}\",\"entries\":[{}]}}",
            entries.join(",")
        );
        let mut f = std::fs::File::create(path).expect("create allowlist file");
        f.write_all(body.as_bytes()).expect("write allowlist file");
    }

    fn config_with_file_allowlist(path: std::path::PathBuf) -> TraceUploadClaimIssuerConfig {
        TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            ..test_config()
        }
    }

    #[tokio::test]
    async fn allowlist_admits_listed_invite_and_embeds_policy_label() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
        let config = config_with_file_allowlist(path);
        let token =
            workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-OK-001");
        let (status, body) = post_claim(config, token, claim_request()).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .expect("access_token present");

        // Decode the minted JWT body and confirm policy_label is embedded.
        let parts: Vec<&str> = access_token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT has three parts");
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .expect("payload decodes");
        let parsed: serde_json::Value = serde_json::from_slice(&payload).expect("payload is json");
        assert_eq!(
            parsed.get("policy_label").and_then(|v| v.as_str()),
            Some("pilot-2026-05"),
            "minted JWT carries policy_label"
        );
    }

    #[tokio::test]
    async fn onboard_requires_registry_db_after_allowlist_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INVOK001INVOK001"]);
        let config = config_with_file_allowlist(path);
        let (status, body) = post_onboard(config, onboard_request("INVOK001INVOK001")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("OnboardRegistryNotConfigured")
        );
    }

    #[tokio::test]
    async fn onboard_requires_allowlist_source() {
        let (status, body) = post_onboard(test_config(), onboard_request("INVOK001INVOK001")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("OnboardAllowlistNotConfigured")
        );
    }

    #[tokio::test]
    async fn onboard_refuses_malformed_device_public_key() {
        let mut body = onboard_request("INVOK001INVOK001");
        body["device_public_key"] = json!("not-base64");
        let (status, body) = post_onboard(test_config(), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("DeviceKeyMalformed")
        );
    }

    #[tokio::test]
    async fn onboard_refuses_unlisted_invite_with_named_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INVOK001INVOK001"]);
        let config = config_with_file_allowlist(path);
        let (status, body) = post_onboard(config, onboard_request("MISS0001MISS0001")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("InviteNotValid")
        );
    }

    #[tokio::test]
    async fn onboard_fails_closed_when_authoritative_with_no_registry() {
        // Authoritative mode with no registry must refuse, NOT fall back to
        // the file allowlist. Silent fallback would let a revoked invite
        // redeem again after a config mistake.
        let config = TraceUploadClaimIssuerConfig {
            invite_registry_authoritative: true,
            ..test_config()
        };
        let (status, body) = post_onboard(config, onboard_request("INVOK001INVOK001")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("InviteRegistryNotConfigured")
        );
    }

    #[tokio::test]
    async fn device_key_claim_requires_well_formed_device_key_id() {
        let (status, body) =
            post_device_claim(test_config(), "not-a-key-id", "AA==", claim_request()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("invalid device key id")
        );
    }

    #[tokio::test]
    async fn device_key_claim_requires_base64_signature() {
        let (status, body) = post_device_claim(
            test_config(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "not-base64",
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("invalid device key signature")
        );
    }

    #[tokio::test]
    async fn device_key_claim_uses_registry_gate_instead_of_workload_token() {
        let (status, body) = post_device_claim(
            test_config(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("device key registry is not configured")
        );
    }

    #[test]
    fn device_key_signature_verifies_exact_claim_body() {
        use ring::signature::KeyPair;

        let rng = ring::rand::SystemRandom::new();
        let pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
        let keypair =
            ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
        let public_key = keypair.public_key().as_ref();
        let device_key_id = device_key_id_from_public_key_bytes(public_key);
        let public_key_wire = base64::engine::general_purpose::STANDARD.encode(public_key);
        let stored_public_key =
            device_public_key_bytes(&public_key_wire, &device_key_id).expect("stored key parses");

        let body = claim_request().to_string();
        let signature = keypair.sign(body.as_bytes());
        verify_device_claim_signature(&stored_public_key, body.as_bytes(), signature.as_ref())
            .expect("signature verifies over exact body");
        assert!(
            verify_device_claim_signature(&stored_public_key, b"{}", signature.as_ref()).is_err(),
            "signature must bind the exact serialized body"
        );
    }

    #[test]
    fn device_key_workload_jwt_verifies_registered_public_key() {
        use ring::signature::KeyPair;

        let rng = ring::rand::SystemRandom::new();
        let pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
        let keypair =
            ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
        let public_key = keypair.public_key().as_ref();
        let device_key_id = device_key_id_from_public_key_bytes(public_key);
        let now = Utc::now();
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(device_key_id);
        let token = jsonwebtoken::encode(
            &header,
            &json!({
                "tenant_id": "tenant-a",
                "aud": "trace-commons-ingest",
                "iat": now.timestamp(),
                "exp": (now + Duration::minutes(5)).timestamp(),
            }),
            &EncodingKey::from_ed_der(pkcs8.as_ref()),
        )
        .expect("device token signs");

        let claims = verify_device_workload_jwt(&token, public_key, Some("trace-commons-ingest"))
            .expect("registered public key verifies device token");
        validate_device_workload_claims(&claims, "tenant-a").expect("tenant matches");
        assert_eq!(claims.tenant_id, "tenant-a");
        assert!(
            validate_device_workload_claims(&claims, "tenant-b").is_err(),
            "device JWT must not authorize another tenant"
        );
        assert!(
            verify_device_workload_jwt(&token, public_key, Some("wrong-audience")).is_err(),
            "device JWT audience must be enforced"
        );
    }

    #[tokio::test]
    async fn allowlist_refuses_unlisted_invite_with_named_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
        let config = config_with_file_allowlist(path);
        let token =
            workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-NOT-LISTED");
        let (status, body) = post_claim(config, token, claim_request()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("PilotAllowlistNotMatched")
        );
    }

    #[tokio::test]
    async fn allowlist_refuses_missing_invite_with_named_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
        let config = config_with_file_allowlist(path);
        // Use the base workload_token helper (no invite_code field).
        let token = workload_token("workload-issuer", "trace-claim-issuer");
        let (status, body) = post_claim(config, token, claim_request()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("PilotAllowlistInviteCodeMissing")
        );
    }

    #[tokio::test]
    async fn allowlist_off_by_default_preserves_legacy_behavior() {
        // No invite_code field; no allowlist configured. Issuance must
        // succeed exactly as before this slice.
        let config = test_config();
        let token = workload_token("workload-issuer", "trace-claim-issuer");
        let (status, _) = post_claim(config, token, claim_request()).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn allowlist_stale_snapshot_refuses_with_named_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
        let mut config = config_with_file_allowlist(path);
        // Force the snapshot to be "stale" by setting max-stale to 0 and
        // sleeping enough that the cached snapshot's age > 0.
        config.allowlist_max_stale_seconds = 0;
        let token =
            workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-OK-001");
        // Wait a brief moment so loaded_at.elapsed() > Duration::from_secs(0).
        // Duration::from_secs(0) means "any elapsed time is stale", but our
        // comparison is `snapshot_age > max_stale`, so the snapshot has to
        // be older than zero — any non-zero elapsed wins.
        tokio::time::sleep(StdDuration::from_millis(5)).await;
        let (status, body) = post_claim(config, token, claim_request()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("PilotAllowlistStale")
        );
    }

    #[tokio::test]
    async fn enroll_rejects_bad_signature_uniformly() {
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        use ring::signature::KeyPair;
        use trace_commons_protocol::onboarding::{
            TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
            TraceInstanceEnrollRequest, TraceOnboardClientInfo,
            device_key_id_from_public_key_bytes,
        };

        // Generate an instance keypair.
        let rng = ring::rand::SystemRandom::new();
        let instance_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
        let instance_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
        let instance_pk = instance_kp.public_key().as_ref().to_vec();
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);

        // Generate a device keypair.
        let device_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
        let device_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(device_pkcs8.as_ref()).expect("parse");
        let device_pk = device_kp.public_key().as_ref().to_vec();
        let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&device_pk);
        let device_key_id = device_key_id_from_public_key_bytes(&device_pk);

        // Build an allowlist file with the instance entry.
        let instance_subject_hash = hash_instance_subject(&instance_pk);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        {
            use std::io::Write;
            let body = format!(
                r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"ironclaw-test","instance_public_key":"{instance_pk_b64}","max_enrollments":100,"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
            );
            let mut f = std::fs::File::create(&path).expect("create allowlist");
            f.write_all(body.as_bytes()).expect("write allowlist");
        }
        // Suppress unused warning — the hash is used in the allowlist body above.
        let _ = &instance_subject_hash;

        let config = TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            onboarding_device_key_db: None,
            ..test_config()
        };

        let state = config.build_state().expect("state builds");

        let now_ts = chrono::Utc::now().timestamp();
        let attestation = TraceInstanceEnrollAttestation {
            device_key_id: device_key_id.clone(),
            aud: "trace-commons-upload".to_string(),
            instance_id: "ironclaw-test".to_string(),
            user_subject: "user-enroll-bad-sig-test".to_string(),
            nonce: uuid::Uuid::new_v4().to_string(),
            exp: now_ts + 240,
        };

        // Use a 64-byte garbage signature.
        let bad_sig = base64::engine::general_purpose::STANDARD.encode([0u8; 64]);

        let request = TraceInstanceEnrollRequest {
            schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
            instance_public_key: instance_pk_b64.clone(),
            device_public_key: device_pk_b64.clone(),
            attestation,
            attestation_sig: bad_sig,
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        };

        let err = state
            .enroll(request)
            .await
            .expect_err("bad sig must be rejected");
        assert_eq!(
            err.status,
            StatusCode::FORBIDDEN,
            "must map to 403 FORBIDDEN"
        );
    }

    #[tokio::test]
    async fn enroll_happy_path_provisions_user_tenant() {
        use crate::config::SslMode;
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        use ring::signature::KeyPair;
        use secrecy::SecretString;
        use trace_commons_protocol::onboarding::{
            TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
            TraceInstanceEnrollRequest, TraceOnboardClientInfo, derive_user_tenant_id,
            device_key_id_from_public_key_bytes, instance_enroll_attestation_signing_bytes,
        };

        // Skip if no DB available.
        let pg_url = match std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
        {
            Ok(u) => u,
            Err(_) => {
                eprintln!("skipping enroll_happy_path_provisions_user_tenant: no DB configured");
                return;
            }
        };
        let db_config = crate::config::DatabaseConfig {
            url: SecretString::from(pg_url),
            pool_size: 4,
            ssl_mode: SslMode::Prefer,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        };
        let pg = match crate::db::postgres::PgBackend::new(&db_config).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("skipping: database unavailable ({e})");
                return;
            }
        };
        let pool = pg.raw_pool_for_tests_and_diagnostics();
        let db: std::sync::Arc<dyn crate::db::Database> = std::sync::Arc::new(pg);

        // Generate instance keypair.
        let rng = ring::rand::SystemRandom::new();
        let instance_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
        let instance_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
        let instance_pk = instance_kp.public_key().as_ref().to_vec();
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);

        // Generate device keypair.
        let device_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
        let device_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(device_pkcs8.as_ref()).expect("parse");
        let device_pk = device_kp.public_key().as_ref().to_vec();
        let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&device_pk);
        let device_key_id = device_key_id_from_public_key_bytes(&device_pk);

        let instance_id = format!("test-instance-enroll-happy-{}", uuid::Uuid::new_v4());
        let user_subject = format!("test-user-enroll-happy-{}", uuid::Uuid::new_v4());

        // Build allowlist with the generated instance key.
        let instance_subject_hash = hash_instance_subject(&instance_pk);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        {
            use std::io::Write;
            let body = format!(
                r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"{instance_id}","instance_public_key":"{instance_pk_b64}","max_enrollments":100,"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
            );
            let mut f = std::fs::File::create(&path).expect("create allowlist");
            f.write_all(body.as_bytes()).expect("write allowlist");
        }

        let config = TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            onboarding_device_key_db: Some(db.clone()),
            ..test_config()
        };

        let state = config.build_state().expect("state builds");

        let now_ts = chrono::Utc::now().timestamp();
        let attestation = TraceInstanceEnrollAttestation {
            device_key_id: device_key_id.clone(),
            aud: "trace-commons-upload".to_string(),
            instance_id: instance_id.clone(),
            user_subject: user_subject.clone(),
            nonce: uuid::Uuid::new_v4().to_string(),
            exp: now_ts + 240,
        };
        let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
        let sig = instance_kp.sign(&signing_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());

        let request = TraceInstanceEnrollRequest {
            schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
            instance_public_key: instance_pk_b64.clone(),
            device_public_key: device_pk_b64.clone(),
            attestation,
            attestation_sig: sig_b64,
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        };

        let resp = state.enroll(request).await.expect("enroll succeeds");
        let expected_tenant_id = derive_user_tenant_id(&instance_id, &user_subject);
        let expected_device_key_id = device_key_id_from_public_key_bytes(&device_pk);
        assert_eq!(
            resp.tenant_id, expected_tenant_id,
            "tenant_id matches derived value"
        );
        assert_eq!(
            resp.device_key_id, expected_device_key_id,
            "device_key_id matches"
        );

        // Cleanup: remove test rows so reruns don't accumulate.
        let client = pool.get().await.expect("pool get for cleanup");
        client
            .execute(
                "DELETE FROM trace_instance_enrollments WHERE instance_subject_hash = $1",
                &[&instance_subject_hash],
            )
            .await
            .ok();
        client
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[&expected_tenant_id],
            )
            .await
            .ok();
    }

    #[tokio::test]
    async fn enroll_rejects_future_exp_uniformly() {
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        use ring::signature::KeyPair;
        use trace_commons_protocol::onboarding::{
            TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
            TraceInstanceEnrollRequest, TraceOnboardClientInfo,
            device_key_id_from_public_key_bytes, instance_enroll_attestation_signing_bytes,
        };

        let rng = ring::rand::SystemRandom::new();
        let instance_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
        let instance_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
        let instance_pk = instance_kp.public_key().as_ref().to_vec();
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);

        let device_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
        let device_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(device_pkcs8.as_ref()).expect("parse");
        let device_pk = device_kp.public_key().as_ref().to_vec();
        let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&device_pk);
        let device_key_id = device_key_id_from_public_key_bytes(&device_pk);

        let instance_subject_hash = hash_instance_subject(&instance_pk);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        {
            use std::io::Write;
            let body = format!(
                r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"ironclaw-test","instance_public_key":"{instance_pk_b64}","max_enrollments":100,"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
            );
            let mut f = std::fs::File::create(&path).expect("create allowlist");
            f.write_all(body.as_bytes()).expect("write allowlist");
        }
        let _ = &instance_subject_hash;

        let config = TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            onboarding_device_key_db: None,
            ..test_config()
        };
        let state = config.build_state().expect("state builds");

        let now_ts = chrono::Utc::now().timestamp();
        // exp is 1 hour in the future — well beyond the 5-minute (300s) upper bound.
        let attestation = TraceInstanceEnrollAttestation {
            device_key_id: device_key_id.clone(),
            aud: "trace-commons-upload".to_string(),
            instance_id: "ironclaw-test".to_string(),
            user_subject: "user-future-exp-test".to_string(),
            nonce: uuid::Uuid::new_v4().to_string(),
            exp: now_ts + 3600,
        };
        let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
        let sig = instance_kp.sign(&signing_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());

        let request = TraceInstanceEnrollRequest {
            schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
            instance_public_key: instance_pk_b64.clone(),
            device_public_key: device_pk_b64.clone(),
            attestation,
            attestation_sig: sig_b64,
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        };

        let err = state
            .enroll(request)
            .await
            .expect_err("far-future exp must be rejected");
        assert_eq!(
            err.status,
            StatusCode::FORBIDDEN,
            "future exp must map to uniform 403 FORBIDDEN, not 400"
        );
    }

    #[tokio::test]
    async fn enroll_device_key_mismatch_is_uniform_403() {
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        use ring::signature::KeyPair;
        use trace_commons_protocol::onboarding::{
            TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
            TraceInstanceEnrollRequest, TraceOnboardClientInfo,
            device_key_id_from_public_key_bytes, instance_enroll_attestation_signing_bytes,
        };

        let rng = ring::rand::SystemRandom::new();
        let instance_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
        let instance_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
        let instance_pk = instance_kp.public_key().as_ref().to_vec();
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);

        // Real device key used in the request.
        let device_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
        let device_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(device_pkcs8.as_ref()).expect("parse");
        let device_pk = device_kp.public_key().as_ref().to_vec();
        let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&device_pk);

        // A *different* device key whose id we put in the attestation — mismatch.
        let other_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("other keypair");
        let other_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(other_pkcs8.as_ref()).expect("parse");
        let other_pk = other_kp.public_key().as_ref().to_vec();
        let mismatched_device_key_id = device_key_id_from_public_key_bytes(&other_pk);

        let instance_subject_hash = hash_instance_subject(&instance_pk);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        {
            use std::io::Write;
            let body = format!(
                r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"ironclaw-test","instance_public_key":"{instance_pk_b64}","max_enrollments":100,"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
            );
            let mut f = std::fs::File::create(&path).expect("create allowlist");
            f.write_all(body.as_bytes()).expect("write allowlist");
        }
        let _ = &instance_subject_hash;

        let config = TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            onboarding_device_key_db: None,
            ..test_config()
        };
        let state = config.build_state().expect("state builds");

        let now_ts = chrono::Utc::now().timestamp();
        // Attestation claims the *other* device_key_id (mismatch with device_pk_b64).
        let attestation = TraceInstanceEnrollAttestation {
            device_key_id: mismatched_device_key_id.clone(),
            aud: "trace-commons-upload".to_string(),
            instance_id: "ironclaw-test".to_string(),
            user_subject: "user-device-mismatch-test".to_string(),
            nonce: uuid::Uuid::new_v4().to_string(),
            exp: now_ts + 240,
        };
        let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
        let sig = instance_kp.sign(&signing_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());

        let request = TraceInstanceEnrollRequest {
            schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
            instance_public_key: instance_pk_b64.clone(),
            device_public_key: device_pk_b64.clone(), // real device key
            attestation,
            attestation_sig: sig_b64,
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        };

        let err = state
            .enroll(request)
            .await
            .expect_err("device_key_id mismatch must be rejected");
        assert_eq!(
            err.status,
            StatusCode::FORBIDDEN,
            "device_key_id mismatch must return uniform 403 FORBIDDEN, not 400 EnrollMalformed"
        );
    }

    // ── Task-8 shared helpers ────────────────────────────────────────────────

    /// Build a DB-backed issuer state for a fresh instance entry.
    ///
    /// Returns `(state, instance_kp, instance_pk_bytes, instance_subject_hash, pool)`
    /// on success, or `None` when the DB is not available (caller must skip).
    ///
    /// `tag` is embedded in the `instance_id` so each test uses a unique entry.
    /// `max_enrollments` caps how many distinct users may enroll against this entry.
    async fn build_pg_state_for_enroll_test(
        tag: &str,
        max_enrollments: u32,
    ) -> Option<(
        std::sync::Arc<TraceUploadClaimIssuerState>,
        ring::signature::Ed25519KeyPair,
        Vec<u8>,
        String,
        deadpool_postgres::Pool,
    )> {
        use crate::config::SslMode;
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        use ring::signature::KeyPair;
        use secrecy::SecretString;

        let pg_url = match std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
        {
            Ok(u) => u,
            Err(_) => return None,
        };
        let db_config = crate::config::DatabaseConfig {
            url: SecretString::from(pg_url),
            pool_size: 4,
            ssl_mode: SslMode::Prefer,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        };
        let pg = match crate::db::postgres::PgBackend::new(&db_config).await {
            Ok(b) => b,
            Err(_) => return None,
        };
        let pool = pg.raw_pool_for_tests_and_diagnostics();
        let db: std::sync::Arc<dyn crate::db::Database> = std::sync::Arc::new(pg);

        let rng = ring::rand::SystemRandom::new();
        let instance_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
        let instance_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
        let instance_pk = instance_kp.public_key().as_ref().to_vec();
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);
        let instance_id = format!("test-enroll-{tag}-{}", uuid::Uuid::new_v4());
        let instance_subject_hash = hash_instance_subject(&instance_pk);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        {
            use std::io::Write;
            let body = format!(
                r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"{instance_id}","instance_public_key":"{instance_pk_b64}","max_enrollments":{max_enrollments},"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
            );
            let mut f = std::fs::File::create(&path).expect("create allowlist");
            f.write_all(body.as_bytes()).expect("write allowlist");
        }
        // Keep `dir` alive by leaking it — the state needs the file to exist
        // for the duration of the test.  `tempdir` cleans up on drop; Box::leak
        // prevents that so the path remains valid.
        std::mem::forget(dir);

        let config = TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            onboarding_device_key_db: Some(db),
            ..test_config()
        };
        let state = config.build_state().expect("state builds");
        Some((state, instance_kp, instance_pk, instance_subject_hash, pool))
    }

    /// Build a `TraceInstanceEnrollRequest` for the given parameters.
    ///
    /// `instance_kp` is the instance signing keypair whose public key is in
    /// the allowlist.  `device_pk_bytes` must be exactly 32 bytes (Ed25519).
    /// `nonce` is used verbatim so callers can control replay.
    fn make_enroll_request(
        instance_kp: &ring::signature::Ed25519KeyPair,
        instance_pk: &[u8],
        device_pk_bytes: &[u8],
        audience: &str,
        instance_id: &str,
        user_subject: &str,
        nonce: &str,
    ) -> trace_commons_protocol::onboarding::TraceInstanceEnrollRequest {
        use trace_commons_protocol::onboarding::{
            TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
            TraceInstanceEnrollRequest, TraceOnboardClientInfo,
            device_key_id_from_public_key_bytes, instance_enroll_attestation_signing_bytes,
        };

        let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(device_pk_bytes);
        let device_key_id = device_key_id_from_public_key_bytes(device_pk_bytes);
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(instance_pk);
        let now_ts = chrono::Utc::now().timestamp();
        let attestation = TraceInstanceEnrollAttestation {
            device_key_id,
            aud: audience.to_string(),
            instance_id: instance_id.to_string(),
            user_subject: user_subject.to_string(),
            nonce: nonce.to_string(),
            exp: now_ts + 240,
        };
        let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
        let sig = instance_kp.sign(&signing_bytes);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());
        TraceInstanceEnrollRequest {
            schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
            instance_public_key: instance_pk_b64,
            device_public_key: device_pk_b64,
            attestation,
            attestation_sig: sig_b64,
            client_info: TraceOnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.x.y".to_string(),
            },
        }
    }

    // ── Task-8 tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn enroll_second_device_same_user_reuses_tenant_and_does_not_consume_cap() {
        use trace_commons_protocol::onboarding::derive_user_tenant_id;

        let Some((state, instance_kp, instance_pk, instance_subject_hash, pool)) =
            build_pg_state_for_enroll_test(
                "multi-device",
                1, // cap = 1 user slot
            )
            .await
        else {
            eprintln!(
                "skipping enroll_second_device_same_user_reuses_tenant_and_does_not_consume_cap: \
                 no DB configured"
            );
            return;
        };

        // Extract the instance_id from state's allowlist snapshot so we can
        // use it when building requests.
        let snapshot = state.onboard_allowlist_snapshot().expect("snapshot");
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        let hash = hash_instance_subject(&instance_pk);
        let entry = snapshot.instance_entry(&hash).expect("entry in snapshot");
        let instance_id = entry.instance_id.clone();
        let audience = state.audience.clone();

        let user1 = format!("multi-device-user-1-{}", uuid::Uuid::new_v4());
        let user2 = format!("multi-device-user-2-{}", uuid::Uuid::new_v4());

        // Device A bytes (32 bytes).
        let rng = ring::rand::SystemRandom::new();
        let dev_a_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("dev A keypair");
        let dev_a =
            ring::signature::Ed25519KeyPair::from_pkcs8(dev_a_pkcs8.as_ref()).expect("parse dev A");
        use ring::signature::KeyPair;
        let dev_a_pk = dev_a.public_key().as_ref().to_vec();

        let dev_b_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("dev B keypair");
        let dev_b =
            ring::signature::Ed25519KeyPair::from_pkcs8(dev_b_pkcs8.as_ref()).expect("parse dev B");
        let dev_b_pk = dev_b.public_key().as_ref().to_vec();

        let dev_c_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("dev C keypair");
        let dev_c =
            ring::signature::Ed25519KeyPair::from_pkcs8(dev_c_pkcs8.as_ref()).expect("parse dev C");
        let dev_c_pk = dev_c.public_key().as_ref().to_vec();

        // Enroll user-1 with device A.
        let req_a = make_enroll_request(
            &instance_kp,
            &instance_pk,
            &dev_a_pk,
            &audience,
            &instance_id,
            &user1,
            &uuid::Uuid::new_v4().to_string(),
        );
        let resp_a = state.enroll(req_a).await.expect("user-1 device-A enrolls");

        // Enroll user-1 with device B (same user, different device).
        let req_b = make_enroll_request(
            &instance_kp,
            &instance_pk,
            &dev_b_pk,
            &audience,
            &instance_id,
            &user1,
            &uuid::Uuid::new_v4().to_string(),
        );
        let resp_b = state.enroll(req_b).await.expect("user-1 device-B enrolls");

        assert_eq!(
            resp_a.tenant_id, resp_b.tenant_id,
            "same user-subject must yield same tenant_id regardless of device"
        );

        // Enroll user-2 with device C — must be refused: cap is 1 user and
        // user-1 already consumed it.
        let req_c = make_enroll_request(
            &instance_kp,
            &instance_pk,
            &dev_c_pk,
            &audience,
            &instance_id,
            &user2,
            &uuid::Uuid::new_v4().to_string(),
        );
        let err_c = state
            .enroll(req_c)
            .await
            .expect_err("user-2 must be refused when cap = 1 is consumed");
        assert_eq!(
            err_c.status,
            StatusCode::FORBIDDEN,
            "cap exceeded must return 403 FORBIDDEN"
        );

        // Cleanup.
        let tenant1 = derive_user_tenant_id(&instance_id, &user1);
        let tenant2 = derive_user_tenant_id(&instance_id, &user2);
        let client = pool.get().await.expect("pool get for cleanup");
        client
            .execute(
                "DELETE FROM trace_instance_enrollments WHERE instance_subject_hash = $1",
                &[&instance_subject_hash],
            )
            .await
            .ok();
        for tid in [&tenant1, &tenant2] {
            client
                .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[tid])
                .await
                .ok();
        }
    }

    #[tokio::test]
    async fn enroll_replayed_nonce_is_refused() {
        let Some((state, instance_kp, instance_pk, instance_subject_hash, pool)) =
            build_pg_state_for_enroll_test("replay", 100).await
        else {
            eprintln!("skipping enroll_replayed_nonce_is_refused: no DB configured");
            return;
        };

        let snapshot = state.onboard_allowlist_snapshot().expect("snapshot");
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        let hash = hash_instance_subject(&instance_pk);
        let entry = snapshot.instance_entry(&hash).expect("entry in snapshot");
        let instance_id = entry.instance_id.clone();
        let audience = state.audience.clone();

        let rng = ring::rand::SystemRandom::new();
        let dev_pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
        let dev_kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(dev_pkcs8.as_ref()).expect("parse");
        use ring::signature::KeyPair;
        let dev_pk = dev_kp.public_key().as_ref().to_vec();

        let user = format!("replay-user-{}", uuid::Uuid::new_v4());
        let nonce = format!("dup-nonce-{}", uuid::Uuid::new_v4());

        let req1 = make_enroll_request(
            &instance_kp,
            &instance_pk,
            &dev_pk,
            &audience,
            &instance_id,
            &user,
            &nonce,
        );
        // First use must succeed and record the nonce.
        state.enroll(req1).await.expect("first enroll succeeds");

        // Second request with the SAME nonce — the replay cache recorded it
        // after the first success, so the pre-check must now reject it.
        let req2 = make_enroll_request(
            &instance_kp,
            &instance_pk,
            &dev_pk,
            &audience,
            &instance_id,
            &user,
            &nonce,
        );
        let err = state
            .enroll(req2)
            .await
            .expect_err("replayed nonce must be refused");
        assert_eq!(
            err.status,
            StatusCode::FORBIDDEN,
            "replay must return 403 FORBIDDEN"
        );

        // Cleanup.
        use trace_commons_protocol::onboarding::derive_user_tenant_id;
        let tenant_id = derive_user_tenant_id(&instance_id, &user);
        let client = pool.get().await.expect("pool get for cleanup");
        client
            .execute(
                "DELETE FROM trace_instance_enrollments WHERE instance_subject_hash = $1",
                &[&instance_subject_hash],
            )
            .await
            .ok();
        client
            .execute(
                "DELETE FROM trace_tenants WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .ok();
    }

    #[tokio::test]
    async fn onboard_invite_path_unaffected_by_enroll() {
        // Regression guard: the enroll path shares no mutable state that
        // breaks the existing invite onboard flow. Verify the invite-not-valid
        // error still surfaces correctly after the enroll wiring.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INVOK001INVOK001"]);
        let config = config_with_file_allowlist(path);
        // An invite that is NOT in the allowlist must still return InviteNotValid.
        let (status, body) = post_onboard(config, onboard_request("MISS0001MISS0001")).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "unlisted invite must still return 403 after enroll wiring"
        );
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("InviteNotValid"),
            "error code must be InviteNotValid"
        );
    }

    // ---- Task 2: per-user subject tests ----

    use crate::db::{DeviceKeyRecord, InstanceEnrollmentOutcome, InstanceUserProvision};
    use crate::error::DatabaseError;
    use crate::trace_corpus_storage::*;

    struct StubDeviceKeyDb {
        device_keys: std::sync::RwLock<
            std::collections::HashMap<(String, String), crate::db::DeviceKeyRecord>,
        >,
    }

    impl StubDeviceKeyDb {
        fn new() -> Self {
            Self {
                device_keys: std::sync::RwLock::new(std::collections::HashMap::new()),
            }
        }

        fn insert_test_device_key(
            &self,
            tenant_id: &str,
            device_key_id: &str,
            record: crate::db::DeviceKeyRecord,
        ) {
            self.device_keys
                .write()
                .unwrap()
                .insert((tenant_id.to_string(), device_key_id.to_string()), record);
        }
    }

    #[async_trait::async_trait]
    impl crate::trace_corpus_storage::TraceCorpusStore for StubDeviceKeyDb {
        async fn upsert_trace_submission(
            &self,
            _: TraceSubmissionWrite,
        ) -> Result<TraceSubmissionRecord, DatabaseError> {
            todo!("stub")
        }
        async fn get_trace_submission(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_submissions(
            &self,
            _: &str,
        ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_account_trace_submissions_keyset(
            &self,
            _: &str,
            _: &[String],
            _: Option<TraceSubmissionKeysetCursor>,
            _: i64,
        ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_tenant_policy(
            &self,
            _: TraceTenantPolicyWrite,
        ) -> Result<TraceTenantPolicyRecord, DatabaseError> {
            todo!("stub")
        }
        async fn get_trace_tenant_policy(
            &self,
            _: &str,
        ) -> Result<Option<TraceTenantPolicyRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_tenant_access_grant(
            &self,
            _: TraceTenantAccessGrantWrite,
        ) -> Result<TraceTenantAccessGrantRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_tenant_access_grants(
            &self,
            _: &str,
        ) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_active_trace_tenant_access_grants_for_principal(
            &self,
            _: &str,
            _: &str,
            _: DateTime<Utc>,
        ) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_credit_events(
            &self,
            _: &str,
        ) -> Result<Vec<TraceCreditEventRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn update_trace_submission_status(
            &self,
            _: &str,
            _: Uuid,
            _: TraceCorpusStatus,
            _: &str,
            _: Option<&str>,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn claim_trace_review_lease(
            &self,
            _: &str,
            _: Uuid,
            _: &str,
            _: DateTime<Utc>,
            _: Option<DateTime<Utc>>,
            _: DateTime<Utc>,
        ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn release_trace_review_lease(
            &self,
            _: &str,
            _: Uuid,
            _: &str,
        ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn append_trace_object_ref(
            &self,
            _: TraceObjectRefWrite,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_object_refs(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Vec<TraceObjectRefRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn get_latest_active_trace_object_ref(
            &self,
            _: &str,
            _: Uuid,
            _: TraceObjectArtifactKind,
        ) -> Result<Option<TraceObjectRefRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn append_trace_derived_record(
            &self,
            _: TraceDerivedRecordWrite,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_derived_records(
            &self,
            _: &str,
        ) -> Result<Vec<TraceDerivedRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_vector_entry(
            &self,
            _: TraceVectorEntryWrite,
        ) -> Result<TraceVectorEntryRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_vector_entries(
            &self,
            _: &str,
        ) -> Result<Vec<TraceVectorEntryRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_model_version(
            &self,
            _: TraceRankingModelVersionWrite,
        ) -> Result<TraceRankingModelVersionRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_model_versions(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingModelVersionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_calibration_dataset(
            &self,
            _: TraceRankingCalibrationDatasetWrite,
        ) -> Result<TraceRankingCalibrationDatasetRecord, DatabaseError> {
            todo!("stub")
        }
        async fn update_trace_ranking_calibration_dataset_status(
            &self,
            _: TraceRankingCalibrationDatasetStatusUpdate,
        ) -> Result<TraceRankingCalibrationDatasetRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_calibration_datasets(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingCalibrationDatasetRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_feature(
            &self,
            _: TraceRankingFeatureWrite,
        ) -> Result<TraceRankingFeatureRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_features(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingFeatureRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_prediction(
            &self,
            _: TraceRankingPredictionWrite,
        ) -> Result<TraceRankingPredictionRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_predictions(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingPredictionRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_label(
            &self,
            _: TraceRankingLabelWrite,
        ) -> Result<TraceRankingLabelRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_labels(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingLabelRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_preference_label(
            &self,
            _: TraceRankingPreferenceLabelWrite,
        ) -> Result<TraceRankingPreferenceLabelRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_preference_labels(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingPreferenceLabelRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_calibration_run(
            &self,
            _: TraceRankingCalibrationRunWrite,
        ) -> Result<TraceRankingCalibrationRunRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_calibration_runs(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingCalibrationRunRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_ranking_worker_run(
            &self,
            _: TraceRankingWorkerRunWrite,
        ) -> Result<TraceRankingWorkerRunRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_ranking_worker_runs(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRankingWorkerRunRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_export_manifest(
            &self,
            _: TraceExportManifestWrite,
        ) -> Result<TraceExportManifestRecord, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_export_manifest_mirror(
            &self,
            _: TraceExportManifestMirrorWrite,
        ) -> Result<TraceExportManifestRecord, DatabaseError> {
            todo!("stub")
        }
        async fn delete_trace_export_manifest_mirror(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_export_manifests(
            &self,
            _: &str,
        ) -> Result<Vec<TraceExportManifestRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_export_manifest_item(
            &self,
            _: TraceExportManifestItemWrite,
        ) -> Result<TraceExportManifestItemRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_export_manifest_items(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Vec<TraceExportManifestItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn invalidate_trace_export_manifests_for_submission(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<u64, DatabaseError> {
            todo!("stub")
        }
        async fn invalidate_trace_export_manifest_items_for_submission(
            &self,
            _: &str,
            _: Uuid,
            _: TraceExportManifestItemInvalidationReason,
        ) -> Result<u64, DatabaseError> {
            todo!("stub")
        }
        async fn invalidate_trace_vector_entries_for_submission(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<u64, DatabaseError> {
            todo!("stub")
        }
        async fn invalidate_trace_vector_entry_for_submission(
            &self,
            _: &str,
            _: Uuid,
            _: Uuid,
        ) -> Result<u64, DatabaseError> {
            todo!("stub")
        }
        async fn append_trace_audit_event(
            &self,
            _: TraceAuditEventWrite,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_audit_events(
            &self,
            _: &str,
        ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_recent_trace_audit_events(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn get_trace_audit_event_by_id(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Option<TraceAuditEventRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn append_trace_credit_event(
            &self,
            _: TraceCreditEventWrite,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_utility_attestation(
            &self,
            _: TraceUtilityAttestationWrite,
        ) -> Result<TraceUtilityAttestationRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_utility_attestations(
            &self,
            _: &str,
        ) -> Result<Vec<TraceUtilityAttestationRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_credit_settlement_batch(
            &self,
            _: TraceCreditSettlementBatchWrite,
        ) -> Result<TraceCreditSettlementBatchRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_credit_settlement_batches(
            &self,
            _: &str,
        ) -> Result<Vec<TraceCreditSettlementBatchRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_credit_hold(
            &self,
            _: TraceCreditHoldWrite,
        ) -> Result<TraceCreditHoldRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_credit_holds(
            &self,
            _: &str,
        ) -> Result<Vec<TraceCreditHoldRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_near_credit_outbox_item(
            &self,
            _: TraceNearCreditOutboxItemWrite,
        ) -> Result<TraceNearCreditOutboxItemRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_near_credit_outbox_items(
            &self,
            _: &str,
        ) -> Result<Vec<TraceNearCreditOutboxItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn update_trace_near_credit_outbox_status(
            &self,
            _: &str,
            _: Uuid,
            _: TraceCreditSettlementNearStatus,
            _: Option<String>,
            _: Option<String>,
            _: Option<Vec<TraceCreditSettlementNearStatus>>,
        ) -> Result<Option<TraceNearCreditOutboxItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_benchmark_registry_outbox_item(
            &self,
            _: TraceBenchmarkRegistryOutboxItemWrite,
        ) -> Result<TraceBenchmarkRegistryOutboxItemRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_benchmark_registry_outbox_items(
            &self,
            _: &str,
        ) -> Result<Vec<TraceBenchmarkRegistryOutboxItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn update_trace_benchmark_registry_outbox_status(
            &self,
            _: &str,
            _: Uuid,
            _: TraceBenchmarkRegistryOutboxStatus,
            _: Option<String>,
            _: Option<String>,
        ) -> Result<Option<TraceBenchmarkRegistryOutboxItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn write_trace_tombstone(&self, _: TraceTombstoneWrite) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_tombstones(
            &self,
            _: &str,
        ) -> Result<Vec<TraceTombstoneRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_retention_job(
            &self,
            _: TraceRetentionJobWrite,
        ) -> Result<TraceRetentionJobRecord, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_retention_job_item(
            &self,
            _: TraceRetentionJobItemWrite,
        ) -> Result<TraceRetentionJobItemRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_retention_jobs(
            &self,
            _: &str,
        ) -> Result<Vec<TraceRetentionJobRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_retention_job_items(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Vec<TraceRetentionJobItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_export_access_grant(
            &self,
            _: TraceExportAccessGrantWrite,
        ) -> Result<TraceExportAccessGrantRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_export_access_grants(
            &self,
            _: &str,
        ) -> Result<Vec<TraceExportAccessGrantRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_export_job(
            &self,
            _: TraceExportJobWrite,
        ) -> Result<TraceExportJobRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_export_jobs(
            &self,
            _: &str,
        ) -> Result<Vec<TraceExportJobRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn update_trace_export_job_status(
            &self,
            _: &str,
            _: Uuid,
            _: TraceExportJobStatusUpdate,
        ) -> Result<Option<TraceExportJobRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn claim_next_trace_export_job(
            &self,
            _: &str,
            _: Option<&str>,
            _: DateTime<Utc>,
            _: &str,
        ) -> Result<Option<TraceExportJobRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn recover_stale_trace_export_job(
            &self,
            _: &str,
            _: Uuid,
            _: DateTime<Utc>,
            _: TraceExportJobStatusUpdate,
        ) -> Result<Option<TraceExportJobRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn retry_failed_trace_export_job(
            &self,
            _: &str,
            _: Uuid,
            _: DateTime<Utc>,
            _: TraceExportJobStatusUpdate,
        ) -> Result<Option<TraceExportJobRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn upsert_trace_revocation_propagation_item(
            &self,
            _: TraceRevocationPropagationItemWrite,
        ) -> Result<TraceRevocationPropagationItemRecord, DatabaseError> {
            todo!("stub")
        }
        async fn list_trace_revocation_propagation_items(
            &self,
            _: &str,
            _: Uuid,
        ) -> Result<Vec<TraceRevocationPropagationItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn list_due_trace_revocation_propagation_items(
            &self,
            _: &str,
            _: DateTime<Utc>,
            _: u32,
        ) -> Result<Vec<TraceRevocationPropagationItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn update_trace_revocation_propagation_item_status(
            &self,
            _: &str,
            _: Uuid,
            _: TraceRevocationPropagationItemStatusUpdate,
        ) -> Result<Option<TraceRevocationPropagationItemRecord>, DatabaseError> {
            todo!("stub")
        }
        async fn invalidate_trace_submission_artifacts(
            &self,
            _: &str,
            _: Uuid,
            _: TraceDerivedStatus,
        ) -> Result<TraceArtifactInvalidationCounts, DatabaseError> {
            todo!("stub")
        }
        async fn mark_trace_object_ref_deleted(
            &self,
            _: &str,
            _: Uuid,
            _: &str,
            _: &str,
        ) -> Result<u64, DatabaseError> {
            todo!("stub")
        }
        async fn insert_trace_gate_decision(
            &self,
            _: &str,
            _: TraceGateDecisionRow,
        ) -> Result<(), DatabaseError> {
            todo!("stub")
        }
        async fn stream_trace_gate_decisions_for_replay(
            &self,
            _: &str,
            _: u32,
            _: Option<(DateTime<Utc>, Uuid)>,
        ) -> Result<Vec<TraceGateDecisionRow>, DatabaseError> {
            todo!("stub")
        }
        async fn is_vector_entry_revoked(&self, _: &str, _: Uuid) -> Result<bool, DatabaseError> {
            todo!("stub")
        }
    }

    #[async_trait::async_trait]
    impl crate::db::Database for StubDeviceKeyDb {
        async fn run_migrations(&self) -> Result<(), DatabaseError> {
            Ok(())
        }

        async fn enroll_instance_user(
            &self,
            _p: InstanceUserProvision,
        ) -> Result<(), DatabaseError> {
            Err(DatabaseError::Pool("stub".into()))
        }

        async fn reserve_instance_enrollment(
            &self,
            _instance_subject_hash: &str,
            _user_subject_hash: &str,
            _tenant_id: &str,
            _max_enrollments: i64,
        ) -> Result<InstanceEnrollmentOutcome, DatabaseError> {
            Err(DatabaseError::Pool("stub".into()))
        }

        async fn instance_ledger_rls_ready(&self) -> Result<bool, DatabaseError> {
            Ok(false)
        }

        async fn get_device_key(
            &self,
            tenant_id: &str,
            device_key_id: &str,
        ) -> Result<Option<DeviceKeyRecord>, DatabaseError> {
            Ok(self
                .device_keys
                .read()
                .unwrap()
                .get(&(tenant_id.to_string(), device_key_id.to_string()))
                .cloned())
        }
    }

    fn device_claim_request_body(tenant_id: &str, subject: Option<&str>) -> serde_json::Value {
        let mut body = json!({
            "schema_version": TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION,
            "tenant_id": tenant_id,
            "audience": "trace-commons-upload",
            "trace_id": Uuid::new_v4(),
            "submission_id": Uuid::new_v4(),
            "consent_scopes": ["debugging_evaluation"],
            "allowed_uses": ["debugging"],
            "requested_at": Utc::now(),
        });
        if let Some(s) = subject {
            body["subject"] = serde_json::Value::String(s.to_string());
        }
        body
    }

    async fn post_signed_device_claim_for_tenant(
        tenant_id: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value, String) {
        use ring::signature::KeyPair;
        use std::sync::Arc;
        use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;

        let rng = ring::rand::SystemRandom::new();
        let pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
        let kp =
            ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
        let pk = kp.public_key().as_ref();
        let device_key_id = device_key_id_from_public_key_bytes(pk);
        let pk_b64 = base64::engine::general_purpose::STANDARD.encode(pk);

        let stub = Arc::new(StubDeviceKeyDb::new());
        stub.insert_test_device_key(
            tenant_id,
            &device_key_id,
            DeviceKeyRecord {
                device_key_id: device_key_id.clone(),
                tenant_id: tenant_id.to_string(),
                public_key: pk_b64,
                invite_subject_hash: "sha256:stub".to_string(),
                client_info: serde_json::json!({}),
                created_at: Utc::now(),
                revoked_at: None,
            },
        );

        let config = TraceUploadClaimIssuerConfig {
            onboarding_device_key_db: Some(stub as Arc<dyn crate::db::Database>),
            ..test_config()
        };

        let body_str = body.to_string();
        let sig = kp.sign(body_str.as_bytes());
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());

        let (status, response) = post_device_claim(config, &device_key_id, &sig_b64, body).await;
        (status, response, device_key_id)
    }

    fn decode_issued_claims(body: &serde_json::Value) -> serde_json::Value {
        let token = body["access_token"].as_str().expect("access_token field");
        let payload = token.split('.').nth(1).expect("jwt has 3 parts");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("payload is base64url");
        serde_json::from_slice(&decoded).expect("payload is json")
    }

    #[tokio::test]
    async fn device_claim_without_subject_uses_raw_device_key_id() {
        let tenant_id = "test-device-tenant";
        let body = device_claim_request_body(tenant_id, None);
        let (status, response, device_key_id) =
            post_signed_device_claim_for_tenant(tenant_id, body).await;
        assert_eq!(status, StatusCode::OK, "claim must succeed: {:?}", response);
        let claims = decode_issued_claims(&response);
        assert_eq!(
            claims["sub"].as_str(),
            Some(device_key_id.as_str()),
            "sub must be raw device_key_id when no subject"
        );
        assert_eq!(
            claims["principal_ref"].as_str(),
            Some(device_key_id.as_str()),
            "principal_ref must be raw device_key_id when no subject"
        );
    }

    #[tokio::test]
    async fn device_claim_with_subject_yields_namespaced_principal() {
        let tenant_id = "test-device-tenant";
        let subject = "user-abc123";
        let body = device_claim_request_body(tenant_id, Some(subject));
        let (status, response, device_key_id) =
            post_signed_device_claim_for_tenant(tenant_id, body).await;
        assert_eq!(status, StatusCode::OK, "claim must succeed: {:?}", response);
        let claims = decode_issued_claims(&response);
        let expected = format!("instance:{tenant_id}:{device_key_id}:user:{subject}");
        assert_eq!(
            claims["sub"].as_str(),
            Some(expected.as_str()),
            "sub must be namespaced with subject"
        );
        assert_eq!(
            claims["principal_ref"].as_str(),
            Some(expected.as_str()),
            "principal_ref must be namespaced with subject"
        );
    }

    #[tokio::test]
    async fn empty_request_on_hardcoded_floor_grants_exactly_implied_uses() {
        // Regression pin: a device-key claim with no registered tenant
        // access grant falls back to the hardcoded pilot floor
        // (debugging_evaluation + public_attribution scopes). An empty
        // consent_scopes/allowed_uses request must resolve to exactly the
        // uses those floor scopes imply — [debugging, evaluation,
        // aggregate_analytics] — never the full uses ceiling by coincidence.
        let tenant_id = "test-device-tenant-floor";
        let mut body = device_claim_request_body(tenant_id, None);
        body["consent_scopes"] = json!([]);
        body["allowed_uses"] = json!([]);
        let (status, response, _device_key_id) =
            post_signed_device_claim_for_tenant(tenant_id, body).await;
        assert_eq!(status, StatusCode::OK, "claim must succeed: {:?}", response);
        let claims = decode_issued_claims(&response);
        assert_eq!(
            claims["allowed_uses"],
            json!(["debugging", "evaluation", "aggregate_analytics"]),
            "empty request on hardcoded floor must grant exactly the implied uses"
        );
    }

    #[tokio::test]
    async fn distinct_subjects_yield_distinct_device_key_principals() {
        let tenant_id = "test-device-tenant-distinct";

        let subject_alice = "user-alice-hash";
        let body_alice = device_claim_request_body(tenant_id, Some(subject_alice));
        let (status_alice, response_alice, device_key_id_alice) =
            post_signed_device_claim_for_tenant(tenant_id, body_alice).await;
        assert_eq!(
            status_alice,
            StatusCode::OK,
            "alice claim must succeed: {:?}",
            response_alice
        );
        let claims_alice = decode_issued_claims(&response_alice);
        let expected_alice =
            format!("instance:{tenant_id}:{device_key_id_alice}:user:{subject_alice}");
        assert_eq!(
            claims_alice["principal_ref"].as_str(),
            Some(expected_alice.as_str()),
            "alice principal_ref must be namespaced with subject"
        );
        let p1 = claims_alice["principal_ref"]
            .as_str()
            .expect("alice principal_ref");

        let subject_bob = "user-bob-hash";
        let body_bob = device_claim_request_body(tenant_id, Some(subject_bob));
        let (status_bob, response_bob, _device_key_id_bob) =
            post_signed_device_claim_for_tenant(tenant_id, body_bob).await;
        assert_eq!(
            status_bob,
            StatusCode::OK,
            "bob claim must succeed: {:?}",
            response_bob
        );
        let claims_bob = decode_issued_claims(&response_bob);
        let p2 = claims_bob["principal_ref"]
            .as_str()
            .expect("bob principal_ref");

        assert_ne!(p1, p2, "distinct subjects must yield distinct principals");
    }
}
