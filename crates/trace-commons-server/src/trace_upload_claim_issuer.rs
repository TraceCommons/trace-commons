// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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
use trace_commons_protocol::device_invite_subject::{
    DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS, DEVICE_INVITE_SUBJECT_NOT_INVITE_ONBOARDED,
    DEVICE_INVITE_SUBJECT_NOT_REGISTERED, DEVICE_INVITE_SUBJECT_PATH,
    DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION, DEVICE_INVITE_SUBJECT_REVOKED,
    DEVICE_INVITE_SUBJECT_STALE, DeviceInviteSubjectRequest, DeviceInviteSubjectResponse,
};
use trace_commons_protocol::invite_lookup::{
    INVITE_LOOKUP_PATH, INVITE_LOOKUP_REASON_MALFORMED, INVITE_LOOKUP_REASON_NOT_FOUND,
    INVITE_LOOKUP_REQUEST_SCHEMA_VERSION, InviteLookupRequest, InviteLookupResponse,
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

The CLI ships as source in TraceCommons/trace-commons; build it with
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
    /// The NEAR AI sign-up policy template
    /// ([`crate::near_ai_signup_policy::NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV`],
    /// the same value ingest provisions with). When set, a device-key claim
    /// from a NEAR AI-provisioned device that has no contributor grant --
    /// one provisioned before sign-up wrote grants -- first writes the grant
    /// sign-up would have written, then takes its ceiling from it. Requires
    /// `tenant_access_grant_db`; `build_state` refuses the pair otherwise.
    /// `None` leaves such devices on the floor.
    pub near_ai_signup_policy_template:
        Option<crate::near_ai_signup_policy::NearAiSignupPolicyTemplate>,
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
            .field(
                "near_ai_signup_policy_template",
                &self
                    .near_ai_signup_policy_template
                    .as_ref()
                    .map(|template| template.policy_version().to_string()),
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
            near_ai_signup_policy_template:
                crate::near_ai_signup_policy::NearAiSignupPolicyTemplate::from_env()?,
        })
    }

    fn build_state(&self) -> anyhow::Result<Arc<TraceUploadClaimIssuerState>> {
        anyhow::ensure!(
            self.max_ttl_seconds > 0,
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS must be positive"
        );
        // A template the issuer cannot read grants with would be inert: the
        // ceiling is never taken from a grant without the grant database, so
        // NEAR AI devices would stay on the floor while the configuration
        // says otherwise. Refuse the pair rather than run it.
        anyhow::ensure!(
            self.near_ai_signup_policy_template.is_none() || self.tenant_access_grant_db.is_some(),
            "{} requires {TRACE_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB_ENV}=true",
            crate::near_ai_signup_policy::NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV
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
            near_ai_signup_policy_template: self.near_ai_signup_policy_template.clone(),
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
            invite_lookup_rate_per_min: std::env::var("TRACE_COMMONS_INVITE_LOOKUP_RATE_PER_MIN")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(DEFAULT_INVITE_LOOKUP_RATE_PER_MIN),
            invite_lookup_client_ip_header: crate::invite_lookup::parse_client_ip_header(
                std::env::var("TRACE_COMMONS_INVITE_LOOKUP_CLIENT_IP_HEADER")
                    .ok()
                    .as_deref(),
            )?,
            invite_lookup_per_client_rate_per_min: std::env::var(
                "TRACE_COMMONS_INVITE_LOOKUP_PER_CLIENT_RATE_PER_MIN",
            )
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(DEFAULT_INVITE_LOOKUP_PER_CLIENT_RATE_PER_MIN),
            invite_lookup_client_limiter: Arc::new(
                crate::invite_lookup::LookupClientRateLimiter::new(
                    crate::invite_lookup::MAX_TRACKED_LOOKUP_CLIENTS,
                ),
            ),
            invite_admin_backend: self.invite_admin_backend.clone(),
            invite_admin_registry: self.invite_admin_registry.clone(),
            invite_registry_authoritative: self.invite_registry_authoritative,
            near_legion_claim: crate::near_legion_claim::NearLegionClaimState::from_env(
                self.invite_admin_backend.clone(),
                self.invite_admin_registry.clone(),
            ),
            celestine_sloth_claim: crate::celestine_sloth_claim::CelestineSlothClaimState::from_env(
                self.invite_admin_backend.clone(),
                self.invite_admin_registry.clone(),
            ),
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
    near_ai_signup_policy_template:
        Option<crate::near_ai_signup_policy::NearAiSignupPolicyTemplate>,
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
    /// Lookups per minute across ALL callers of `POST /v1/invite/lookup`: one
    /// shared bucket (the same limiter as enroll, under its own key). It caps
    /// how fast the route can be used to guess invite codes, and it stays the
    /// outer cap when per-client keying below is on.
    invite_lookup_rate_per_min: u32,
    /// Header the operator's trusted proxy writes the caller's address into
    /// (`TRACE_COMMONS_INVITE_LOOKUP_CLIENT_IP_HEADER`). `None`, the default,
    /// means no header is trusted and only the shared bucket applies: the
    /// issuer does not see client addresses itself.
    invite_lookup_client_ip_header: Option<axum::http::HeaderName>,
    /// Lookups per minute per client when the header above is configured.
    invite_lookup_per_client_rate_per_min: u32,
    invite_lookup_client_limiter: Arc<crate::invite_lookup::LookupClientRateLimiter>,
    invite_admin_backend: Option<Arc<PgBackend>>,
    invite_admin_registry: Option<Arc<DbInviteRegistry>>,
    invite_registry_authoritative: bool,
    /// Self-serve NEAR Legion claim surface. `None` — routes unmounted —
    /// unless the feature is enabled AND NEAR sign-in and the invite backend
    /// are both configured. Built once here so the in-flight challenge store
    /// is shared across requests rather than rebuilt per call.
    near_legion_claim: Option<crate::near_legion_claim::NearLegionClaimState>,
    /// Self-serve Celestine Sloth Society claim surface. `None` — routes
    /// unmounted — unless the feature is enabled AND the collection, the LCD
    /// endpoint, the tenant template and the invite backend are all configured.
    /// Built once here so the in-flight challenge store is shared across
    /// requests rather than rebuilt per call.
    celestine_sloth_claim: Option<crate::celestine_sloth_claim::CelestineSlothClaimState>,
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

    /// Tenant-binding refusal: the workload token carried no usable
    /// `tenant_id`, so it conveys authority over no tenant at all.
    ///
    /// Forbidden rather than bad-request on purpose: the request is
    /// well-formed, it is the presented credential that authorises nothing.
    /// Naming the missing control keeps the refusal greppable without
    /// echoing the token or the requested tenant.
    fn workload_tenant_missing() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "WorkloadTenantMissing",
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
    Ok(router_from_state(state, request_timeout, max_request_bytes))
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
    // Built before `with_state` because the claim sub-router carries its own
    // state; merging it in keeps the self-serve surface unmounted (404) on any
    // deployment that has not configured it.
    let near_legion = state.near_legion_claim.clone();
    let celestine_sloths = state.celestine_sloth_claim.clone();
    let router = Router::new()
        .route("/health", get(health_handler))
        .route(
            "/.well-known/trace-commons-ed25519-keyset.json",
            get(keyset_handler),
        )
        .route("/onboard", get(invite_landing_handler))
        .route("/v1/trace-upload-claim", post(issue_claim_handler))
        .route("/v1/onboard", post(onboard_handler))
        .route("/v1/enroll", post(enroll_handler))
        .route(INVITE_LOOKUP_PATH, post(invite_lookup_handler))
        .route(
            DEVICE_INVITE_SUBJECT_PATH,
            post(device_invite_subject_handler),
        )
        .with_state(state);

    let router = match near_legion {
        Some(claim_state) => router.merge(crate::near_legion_claim::near_legion_claim_router(
            claim_state,
        )),
        None => router,
    };

    let router = match celestine_sloths {
        Some(claim_state) => router.merge(
            crate::celestine_sloth_claim::celestine_sloth_claim_router(claim_state),
        ),
        None => router,
    };

    router
        .layer(DefaultBodyLimit::max(max_request_bytes))
        .layer(axum::middleware::from_fn(move |req, next| {
            request_timeout_middleware(req, next, request_timeout)
        }))
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
    // Build identity is additive and sits beside `checks` rather than inside
    // it: `checks` is scanned for the "ok"/"configured" labels that decide the
    // status code above, and an identity string is not a check. It is reported
    // on the degraded response too -- knowing which build is broken is exactly
    // what a degraded issuer is being asked.
    let status = if healthy { "ok" } else { "degraded" };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": status,
            "checks": checks,
            "build_commit": trace_commons_build_info::COMMIT,
            "build_time": trace_commons_build_info::BUILD_TIME,
            "build_version": env!("CARGO_PKG_VERSION"),
        })),
    )
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

/// Default cap on invite lookups per minute, all callers together.
const DEFAULT_INVITE_LOOKUP_RATE_PER_MIN: u32 = 30;

/// Default cap on invite lookups per minute from one client, applied only when
/// the operator names a trusted client-address header.
const DEFAULT_INVITE_LOOKUP_PER_CLIENT_RATE_PER_MIN: u32 = 10;

/// Key of the shared lookup bucket in the issuer's rate limiter.
const INVITE_LOOKUP_RATE_KEY: &str = "invite-lookup";

/// `POST /v1/invite/lookup`: is this invite usable, who issued it and what does
/// it pay? Read-only: spends no use, mints nothing, writes nothing. The code
/// rides in the body (never the URL, so it cannot reach an access log), and
/// the log line carries only the invite's subject hash.
async fn invite_lookup_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<InviteLookupResponse>, IssuerError> {
    let result = state.invite_lookup(&headers, &body).await;
    match &result {
        Ok((subject_hash, response)) => tracing::info!(
            invite_subject_hash = subject_hash.as_deref().unwrap_or("none"),
            valid = response.valid,
            reason = response.reason_label.as_deref().unwrap_or("none"),
            "invite lookup"
        ),
        Err(error) => tracing::info!(outcome = error.message, "invite lookup refused"),
    }
    result.map(|(_, response)| Json(response))
}

impl TraceUploadClaimIssuerState {
    /// Returns the presented invite's subject hash (when the code was
    /// well-formed) beside the answer, for the caller's hash-only log line.
    async fn invite_lookup(
        &self,
        headers: &HeaderMap,
        body: &Bytes,
    ) -> Result<(Option<String>, InviteLookupResponse), IssuerError> {
        // Throttle before any parsing or database work: an unthrottled
        // lookup is a free oracle for guessing bearer codes.
        let rate_limited = || IssuerError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "invite_lookup_rate_limited",
        };
        let now = std::time::Instant::now();
        // Per client first, so a caller already over its own budget is
        // refused without spending the shared budget everyone else needs.
        if let Some(header) = &self.invite_lookup_client_ip_header {
            let client = crate::invite_lookup::lookup_client_key(headers, header);
            if !self.invite_lookup_client_limiter.try_acquire(
                &client,
                self.invite_lookup_per_client_rate_per_min,
                now,
            ) {
                return Err(rate_limited());
            }
        }
        if !self.instance_rate_limiter.try_acquire(
            INVITE_LOOKUP_RATE_KEY,
            self.invite_lookup_rate_per_min,
            now,
        ) {
            return Err(rate_limited());
        }
        let request: InviteLookupRequest = serde_json::from_slice(body)
            .map_err(|_| IssuerError::bad_request("invalid invite lookup request"))?;
        if request.schema_version != INVITE_LOOKUP_REQUEST_SCHEMA_VERSION {
            return Err(IssuerError::bad_request("invalid invite lookup request"));
        }
        let invite_code = request.invite_code.trim();
        if !valid_onboard_invite_code(invite_code) {
            return Ok((
                None,
                InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_MALFORMED),
            ));
        }
        // Fail closed: without the invite database there is nothing to
        // answer from, and the file allowlist carries no issuer or range.
        // The database must also be what `/v1/onboard` redeems against. When
        // it is not authoritative, onboarding reads the file allowlist, so a
        // row here says nothing about whether the invite would work: a
        // database-only invite would read as valid and then be refused, and
        // a revocation recorded here would not stop a redemption.
        let backend = self
            .invite_admin_backend
            .as_ref()
            .filter(|_| self.invite_registry_authoritative)
            .ok_or(IssuerError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "invite_registry_not_configured",
            })?;
        let registry = self.invite_admin_registry.as_ref().ok_or(IssuerError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "invite_registry_not_configured",
        })?;
        let subject_hash = hash_invite_code(invite_code);
        // Ask the in-process cache first, exactly as `/v1/onboard` does, so
        // the two agree on whether a code is usable: a stale cache is a 503
        // there and here, and a code the cache does not hold is refused there
        // (an invite minted by a separate `--mint-invites` process is
        // invisible to it until the next refresh) so it must not read valid
        // here. Only a `valid` answer is overridden. The database is still
        // consulted for the refusal label, since the cache drops revoked and
        // expired rows and a holder is told which of those it was.
        let cached = match registry.lookup(&subject_hash) {
            Ok(cached) => cached,
            Err(InviteRegistryError::Stale { .. }) => {
                return Err(IssuerError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    message: "invite_registry_stale",
                });
            }
            Err(InviteRegistryError::Backend(_)) => return Err(IssuerError::internal()),
        };
        let peek = backend
            .peek_invite_grant(&subject_hash)
            .await
            .map_err(|_| IssuerError::internal())?;
        let mut response = crate::invite_lookup::classify_invite(peek.as_ref(), Utc::now());
        if response.valid && cached.is_none() {
            response = InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_NOT_FOUND);
        }
        Ok((Some(subject_hash), response))
    }
}

async fn enroll_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    Json(request): Json<TraceInstanceEnrollRequest>,
) -> Result<Json<TraceOnboardResponse>, IssuerError> {
    let response = state.enroll(request).await?;
    Ok(Json(response))
}

/// `POST /v1/device/invite-subject`: a device reads back the invite subject
/// hash it was onboarded under, so it can sign the legacy invite link
/// statement (#1066). Only its own, and only to a request signed by its own
/// registered key over the exact body. See
/// `trace_commons_protocol::device_invite_subject`.
///
/// Fail-closed: no registry refuses with 503; an unknown, revoked or
/// invite-less device refuses by name. The log line is hash-only: the device
/// and tenant appear as storage refs, and never the hash returned.
async fn device_invite_subject_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<DeviceInviteSubjectResponse>, IssuerError> {
    let result = state.device_invite_subject(&headers, &body).await;
    let outcome = match &result {
        Ok(_) => "returned",
        Err(error) => error.message,
    };
    let device_ref = trimmed_header_value(&headers, TRACE_DEVICE_KEY_ID_HEADER, "")
        .ok()
        .flatten()
        .map(|id| principal_storage_ref(&format!("device:{id}")));
    tracing::info!(
        device_ref = device_ref.as_deref().unwrap_or("none"),
        outcome,
        "device invite subject read"
    );
    result.map(Json)
}

impl TraceUploadClaimIssuerState {
    async fn device_invite_subject(
        &self,
        headers: &HeaderMap,
        body: &Bytes,
    ) -> Result<DeviceInviteSubjectResponse, IssuerError> {
        let auth = device_claim_auth_from_headers(headers)?
            .ok_or_else(|| IssuerError::bad_request("device key auth requires id and signature"))?;
        let request: DeviceInviteSubjectRequest = serde_json::from_slice(body)
            .map_err(|_| IssuerError::bad_request("invalid device invite subject request"))?;
        if request.schema_version != DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION {
            return Err(IssuerError::bad_request(
                "invalid device invite subject request",
            ));
        }
        if request.device_key_id != auth.device_key_id {
            return Err(IssuerError::bad_request("device key id mismatch"));
        }
        if !device_invite_subject_is_fresh(Utc::now().timestamp(), request.issued_at) {
            return Err(IssuerError::bad_request(DEVICE_INVITE_SUBJECT_STALE));
        }
        let tenant_id =
            normalized_required(Some(request.tenant_id.as_str()), "tenant_id is required")?;
        let db = self
            .onboarding_device_key_db
            .as_ref()
            .ok_or_else(IssuerError::device_key_registry_not_configured)?;
        let device_key = db
            .get_device_key(&tenant_id, &auth.device_key_id)
            .await
            .map_err(|_| IssuerError::internal())?
            .ok_or_else(|| IssuerError::forbidden(DEVICE_INVITE_SUBJECT_NOT_REGISTERED))?;
        // The signature before anything about the record is disclosed: an
        // unsigned caller learns only that the key is registered, which the
        // claim route already answers the same way.
        let public_key_bytes =
            device_public_key_bytes(&device_key.public_key, &auth.device_key_id)?;
        verify_device_claim_signature(&public_key_bytes, body, &auth.signature)?;
        if device_key.revoked_at.is_some() {
            return Err(IssuerError::forbidden(DEVICE_INVITE_SUBJECT_REVOKED));
        }
        let invite_subject_hash = device_key
            .invite_subject_hash
            .ok_or_else(|| IssuerError::forbidden(DEVICE_INVITE_SUBJECT_NOT_INVITE_ONBOARDED))?;
        Ok(DeviceInviteSubjectResponse {
            invite_subject_hash,
        })
    }
}

/// Whether `issued_at` sits within [`DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS`]
/// of `now`, either way, bound included. The clock is a parameter so the
/// bound is tested exactly rather than against a clock that moves while the
/// request is in flight. `abs_diff`, because `issued_at` is the caller's: a
/// subtraction overflows for one near `i64::MIN`.
fn device_invite_subject_is_fresh(now: i64, issued_at: i64) -> bool {
    now.abs_diff(issued_at) <= DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS.unsigned_abs()
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
        // The workload token is the ONLY authority for the tenant on this
        // path. The request body is caller-controlled, so it may confirm the
        // tenant but must never supply one: a token that carries no tenant
        // conveys authority over no tenant, and falling back to the request
        // would let any validly signed token mint a claim for any tenant.
        //
        // Refused before the request is consulted, and unconditionally --
        // the binding below is the security property this function exists to
        // enforce, so it must not be contingent on an optional field being
        // present. Mirrors the device-key path, where `DeviceWorkloadClaims`
        // takes a required `tenant_id` and `validate_device_workload_claims`
        // compares it with no `is_some()` guard.
        let Some(tenant_id) = workload
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty())
            .map(str::to_string)
        else {
            tracing::warn!(
                error_class = "WorkloadTenantMissing",
                "upload-claim refused: workload token carries no tenant_id"
            );
            return Err(IssuerError::workload_tenant_missing());
        };
        // Attribution only: when the request names a tenant it must agree
        // exactly with the token's. A blank-but-present value stays a
        // bad-request, as before.
        if let Some(requested_tenant) = request.tenant_id.as_deref() {
            let requested_tenant =
                normalized_required(Some(requested_tenant), "tenant_id is required")?;
            if requested_tenant != tenant_id {
                return Err(IssuerError::forbidden(
                    "workload tenant does not match request",
                ));
            }
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
        // A device-shaped actor is reserved for the device-key paths, which
        // write one only after verifying that device's signature. Ingest
        // resolves the device's account from it, so a workload token must
        // not be able to name a device it never proved.
        if crate::account_trust::is_device_shaped_actor(&actor) {
            return Err(IssuerError::forbidden("workload subject is reserved"));
        }
        let grant_principal_ref = signed_claim_auth_principal_ref(&tenant_id, &actor);
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
    ///
    /// A device with no contributor grant gets one first when it is a NEAR AI
    /// sign-up and the issuer has the sign-up policy template: the backfill
    /// for accounts NEAR AI provisioned before sign-up wrote grants (owner
    /// decision 2026-10-24). After it the grant exists, so it writes at most
    /// once per device, and it never replaces a grant row, so a revoked grant
    /// still leaves the floor.
    async fn resolve_device_scope_ceiling(
        &self,
        tenant_id: &str,
        device_key_id: &str,
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
        let mut matching_grants =
            active_contributor_grants(db.as_ref(), tenant_id, grant_principal_ref, now).await?;
        if matching_grants.is_empty()
            && let Some(template) = self.near_ai_signup_policy_template.as_ref()
        {
            let eligible = db
                .backfill_near_ai_signup_device_grant(tenant_id, device_key_id, template, now)
                .await
                .map_err(|_| IssuerError::internal())?;
            if eligible {
                tracing::info!(
                    device_key_id = %device_key_id,
                    policy_version = %template.policy_version(),
                    "NEAR AI sign-up contributor grant backfilled at claim time"
                );
                matching_grants =
                    active_contributor_grants(db.as_ref(), tenant_id, grant_principal_ref, now)
                        .await?;
            }
        }
        if matching_grants.is_empty() {
            return Ok(hardcoded_floor());
        }
        let mut scope_union: BTreeSet<ConsentScope> = BTreeSet::new();
        let mut use_union: BTreeSet<TraceAllowedUse> = BTreeSet::new();
        for grant in &matching_grants {
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
            crate::account_trust::device_principal_ref(&tenant_id, &device_key_id);
        // When the instance asserts a per-user subject, the issued principal is
        // namespaced under the device so subjects cannot collide across
        // instances/tenants and the blast radius stays inside this tenant. Absent a
        // subject, behavior is unchanged (principal == raw device_key_id).
        // Ingest parses the device key id back out of this actor to find the
        // device's account (`account_trust::device_claim_key_id`).
        let subject = request
            .subject
            .as_deref()
            .map(normalize_subject)
            .transpose()?;
        let actor = crate::account_trust::device_claim_actor(
            &tenant_id,
            &device_key_id,
            subject.as_deref(),
        );
        let (ceiling_scopes, ceiling_uses) = self
            .resolve_device_scope_ceiling(&tenant_id, &device_key_id, &grant_principal_ref, now)
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
        let response = self
            .issue_claim_for_authorized_actor(
                AuthorizedUploadClaimActor {
                    actor: actor.clone(),
                    tenant_id: tenant_id.clone(),
                    grant_principal_ref,
                    allowed_consent_scopes: granted_scopes,
                    allowed_uses: granted_uses,
                    policy_label: None,
                },
                request,
                now,
            )
            .await?;
        // Keep financial ownership tied to the authenticated device, while
        // preserving the claim principal as submission provenance. Never
        // return a usable claim if its ownership association could not persist.
        db.bind_device_upload_principal(&tenant_id, &device_key_id, &actor)
            .await
            .map_err(|_| IssuerError::internal())?;
        Ok(response)
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
        let actor = crate::account_trust::device_claim_actor(&tenant_id, &auth.device_key_id, None);
        let grant_principal_ref =
            crate::account_trust::device_principal_ref(&tenant_id, &auth.device_key_id);
        let (ceiling_scopes, ceiling_uses) = self
            .resolve_device_scope_ceiling(
                &tenant_id,
                &auth.device_key_id,
                &grant_principal_ref,
                now,
            )
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
        let response = self
            .issue_claim_for_authorized_actor(
                AuthorizedUploadClaimActor {
                    actor: actor.clone(),
                    tenant_id: tenant_id.to_string(),
                    grant_principal_ref,
                    allowed_consent_scopes: granted_scopes,
                    allowed_uses: granted_uses,
                    policy_label: None,
                },
                request,
                now,
            )
            .await?;
        db.bind_device_upload_principal(&tenant_id, &auth.device_key_id, &actor)
            .await
            .map_err(|_| IssuerError::internal())?;
        Ok(response)
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
            // round trip below. A use in the separate ingest process may leave
            // this entry cached, but onboard_device_key's final transaction
            // still enforces the durable global use counter.
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
        // Explicit NEAR device identity can obtain narrowly scoped claims before
        // submission admission. This does not grant an invite or window budget;
        // ingest's separately configured admission ledger must authorize each upload.
        if std::env::var("TRACE_COMMONS_NEAR_PROVISIONING_ENABLED").is_ok_and(|v| v == "true")
            && db
                .get_near_provisioned_anchor(tenant_id, principal_ref)
                .await
                .map_err(|_| IssuerError::internal())?
                .is_some()
        {
            if consent_scopes.is_empty() || allowed_uses.is_empty() {
                return Err(IssuerError::forbidden("native claim scopes required"));
            }
            return Ok(());
        }
        // Dual-read canonical method-bound refs and pre-#209 signed hashes so
        // grants provisioned under either shape still authorize.
        let mut principal_refs = vec![principal_ref.to_string()];
        let legacy_signed = legacy_signed_claim_principal_ref(tenant_id, actor);
        if legacy_signed != principal_ref {
            principal_refs.push(legacy_signed);
        }
        let mut grants = Vec::new();
        for candidate in &principal_refs {
            let matched = db
                .list_active_trace_tenant_access_grants_for_principal(tenant_id, candidate, now)
                .await
                .map_err(|_| IssuerError::internal())?;
            for grant in matched {
                if grants
                    .iter()
                    .all(|existing: &TraceTenantAccessGrantRecord| {
                        existing.grant_id != grant.grant_id
                    })
                {
                    grants.push(grant);
                }
            }
        }
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

pub fn valid_onboard_invite_code(invite_code: &str) -> bool {
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

/// Active `contributor` grants for a device principal.
async fn active_contributor_grants(
    db: &dyn Database,
    tenant_id: &str,
    grant_principal_ref: &str,
    now: DateTime<Utc>,
) -> Result<Vec<TraceTenantAccessGrantRecord>, IssuerError> {
    let grants = db
        .list_active_trace_tenant_access_grants_for_principal(tenant_id, grant_principal_ref, now)
        .await
        .map_err(|_| IssuerError::internal())?;
    Ok(grants
        .into_iter()
        .filter(|grant| grant.status == TraceTenantAccessGrantStatus::Active)
        .filter(|grant| grant.role == TraceTenantAccessGrantRole::Contributor)
        .collect())
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

fn method_bound_principal_material(method: &str, identity_material: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        method.len(),
        method,
        identity_material.len(),
        identity_material
    )
}

fn method_bound_principal_ref(method: &str, identity_material: &str) -> String {
    principal_storage_ref(&method_bound_principal_material(method, identity_material))
}

fn signed_claim_identity_material(tenant_id: &str, actor_ref: &str) -> String {
    format!("signed:{tenant_id}:{actor_ref}")
}

fn signed_claim_auth_principal_ref(tenant_id: &str, actor_ref: &str) -> String {
    method_bound_principal_ref(
        "signed_claim",
        &signed_claim_identity_material(tenant_id, actor_ref),
    )
}

fn legacy_signed_claim_principal_ref(tenant_id: &str, actor_ref: &str) -> String {
    principal_storage_ref(&signed_claim_identity_material(tenant_id, actor_ref))
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
    // Only inspect the PEM header line for the algorithm label, never the
    // base64-encoded key body: a body is effectively random text and can
    // contain the literal substring "RSA" by chance for a wholly valid
    // Ed25519 key (roughly 1-in-a-few-hundred keys).
    let header_line = pem.lines().next().unwrap_or("");
    anyhow::ensure!(!header_line.contains("RSA"), "RSA keys are not supported");
    anyhow::ensure!(
        pem.starts_with("-----BEGIN PRIVATE KEY-----"),
        "EdDSA private key must be PKCS#8 PEM"
    );
    EncodingKey::from_ed_pem(pem.as_bytes()).context("invalid EdDSA private key")?;
    Ok(format!("{pem}\n"))
}

pub(crate) fn validate_eddsa_public_key_pem(pem: &str) -> anyhow::Result<String> {
    let pem = pem.trim();
    // See validate_eddsa_private_key_pem: check only the header line, not the
    // base64 body, for the algorithm label.
    let header_line = pem.lines().next().unwrap_or("");
    anyhow::ensure!(!header_line.contains("RSA"), "RSA keys are not supported");
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

pub(crate) fn env_truthy(name: &'static str) -> bool {
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
#[path = "trace_upload_claim_issuer_tests.rs"]
pub(crate) mod tests;
