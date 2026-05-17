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
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::errors::ErrorKind as JwtErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::DatabaseConfig;
use crate::db::Database;
use crate::trace_corpus_storage::{
    TraceTenantAccessGrantRecord, TraceTenantAccessGrantRole, TraceTenantAccessGrantStatus,
};
use crate::trace_upload_claim_allowlist::{
    AllowlistError, AllowlistSource, AllowlistSourceSpec, DenialCounter, FileAllowlistSource,
    hash_invite_code,
};
use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

pub const TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION: &str =
    "ironclaw.trace_upload_claim_request.v1";
pub const TRACE_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS_ENV: &str =
    "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS";
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

pub const TRACE_COMMONS_ALLOWLIST_SOURCE_ENV: &str = "TRACE_COMMONS_ALLOWLIST_SOURCE";
pub const TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS_ENV: &str =
    "TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS";
pub const TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS_ENV: &str =
    "TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS";
pub const TRACE_COMMONS_ISSUER_ADMIN_BIND_ENV: &str = "TRACE_COMMONS_ISSUER_ADMIN_BIND";
pub const TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC_ENV: &str =
    "TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC";

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
    /// Optional second-bind for the operator admin endpoint
    /// (`/v1/admin/allowlist-status`). `None` = admin endpoint disabled.
    /// Must be a loopback address unless
    /// `TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1` is set.
    pub admin_bind: Option<SocketAddr>,
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
            .field("admin_bind", &self.admin_bind)
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
        let allowlist_source =
            AllowlistSourceSpec::parse(optional_env(TRACE_COMMONS_ALLOWLIST_SOURCE_ENV)?.as_deref())?;
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
            admin_bind,
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
            denial_counter,
        }))
    }
}

pub async fn configure_tenant_access_grants_from_env(
    config: &mut TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    if !env_truthy(TRACE_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS_ENV) {
        return Ok(());
    }
    let db = trace_upload_claim_issuer_db_from_env().await?;
    config.tenant_access_grant_db = Some(db);
    config.require_tenant_access_grants = true;
    Ok(())
}

async fn trace_upload_claim_issuer_db_from_env() -> anyhow::Result<Arc<dyn Database>> {
    let url = std::env::var("DATABASE_URL")
        .context("Trace upload-claim issuer tenant access grants require DATABASE_URL")?;
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
    denial_counter: Arc<DenialCounter>,
}

impl TraceUploadClaimIssuerState {
    // Slice 3 uses these accessors from the admin module; until then
    // they're dead-code-visible only.
    /// Read access to the denial counter so the admin router can render
    /// `denials_last_hour` without needing its own `Arc` clone path.
    #[allow(dead_code)]
    pub(crate) fn denial_counter(&self) -> &Arc<DenialCounter> {
        &self.denial_counter
    }

    #[allow(dead_code)]
    pub(crate) fn allowlist_source(&self) -> Option<&Arc<dyn AllowlistSource>> {
        self.allowlist_source.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn allowlist_max_stale_seconds(&self) -> u64 {
        self.allowlist_max_stale.as_secs()
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
    requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TraceUploadClaimResponse {
    access_token: String,
    token_type: &'static str,
    expires_at: DateTime<Utc>,
    expires_in: i64,
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
    /// is configured with an allowlist source; absent otherwise. Read by
    /// the allowlist check in Slice 2; `#[allow(dead_code)]` until then.
    #[serde(default)]
    #[allow(dead_code)]
    invite_code: Option<String>,
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

    /// Pilot allowlist refusal: invite code was valid syntactically but is
    /// not in the active allowlist snapshot. Public label so operators can
    /// grep for it in client error logs.
    //
    // All four pilot_allowlist_* constructors are dead code until Slice 2
    // wires the snapshot check into the issuance handler. Defined now so
    // the error vocabulary is one self-contained slice.
    #[allow(dead_code)]
    fn pilot_allowlist_not_matched() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "PilotAllowlistNotMatched",
        }
    }

    /// Pilot allowlist refusal: the workload token did not carry an
    /// `invite_code` claim and the issuer is configured with an allowlist.
    #[allow(dead_code)]
    fn pilot_allowlist_invite_code_missing() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "PilotAllowlistInviteCodeMissing",
        }
    }

    /// Pilot allowlist refusal: the cached snapshot is older than
    /// `max_stale_seconds` and the source has not yet reloaded
    /// successfully. Fail-closed beats serving on a stale list.
    #[allow(dead_code)]
    fn pilot_allowlist_stale() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "PilotAllowlistStale",
        }
    }

    /// Pilot allowlist refusal: the source returned a malformed snapshot
    /// (file parse failure, etc.) and there is no usable cached snapshot
    /// to fall back to.
    #[allow(dead_code)]
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
        .route("/v1/trace-upload-claim", post(issue_claim_handler))
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

pub async fn serve_trace_upload_claim_issuer(
    config: TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    let bind = config.bind;
    let grace_secs = config.shutdown_grace_seconds;
    let router = trace_upload_claim_issuer_router(config)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind Trace Commons upload claim issuer on {bind}"))?;
    serve_router_with_graceful_shutdown(listener, router, grace_secs, wait_for_shutdown_signal())
        .await
}

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
pub fn mint_test_upload_claim() -> anyhow::Result<String> {
    let config = TraceUploadClaimIssuerConfig::from_env()
        .context("failed to read upload-claim issuer config from env")?;
    let state = config.build_state()?;
    let now = Utc::now();
    let expires_at = now
        .checked_add_signed(Duration::seconds(state.max_ttl_seconds))
        .context("max_ttl_seconds overflow")?;
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
        allowed_consent_scopes: Vec::new(),
        allowed_uses: Vec::new(),
        policy_label: None,
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(state.signing_kid.clone());
    jsonwebtoken::encode(&header, &claims, &state.signing_key)
        .context("failed to mint test upload claim")
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
    Json(request): Json<TraceUploadClaimRequest>,
) -> Result<Json<TraceUploadClaimResponse>, IssuerError> {
    let workload = state.authenticate_workload(&headers)?;
    let response = state.issue_claim(&workload, request).await?;
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
        let mut consent_scopes = request.consent_scopes;
        let mut allowed_uses = request.allowed_uses;
        self.enforce_tenant_access_grants(
            &tenant_id,
            &grant_principal_ref,
            &actor,
            &mut consent_scopes,
            &mut allowed_uses,
            now,
        )
        .await?;
        let expires_at = now
            .checked_add_signed(Duration::seconds(self.max_ttl_seconds))
            .ok_or_else(IssuerError::internal)?;
        let claims = UploadClaimClaims {
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            sub: actor.clone(),
            principal_ref: actor,
            tenant_id,
            role: "contributor",
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
            jti: Uuid::new_v4().to_string(),
            trace_id: request.trace_id,
            submission_id: request.submission_id,
            allowed_consent_scopes: consent_scopes,
            allowed_uses,
            policy_label,
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

fn bearer_token(headers: &HeaderMap) -> Result<&str, IssuerError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| IssuerError::forbidden("missing workload token"))?
        .to_str()
        .map_err(|_| IssuerError::forbidden("invalid workload token"))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| IssuerError::forbidden("invalid workload token"))
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

fn validate_eddsa_private_key_pem(pem: &str) -> anyhow::Result<String> {
    let pem = pem.trim();
    anyhow::ensure!(!pem.contains("RSA"), "RSA keys are not supported");
    anyhow::ensure!(
        pem.starts_with("-----BEGIN PRIVATE KEY-----"),
        "EdDSA private key must be PKCS#8 PEM"
    );
    EncodingKey::from_ed_pem(pem.as_bytes()).context("invalid EdDSA private key")?;
    Ok(format!("{pem}\n"))
}

fn validate_eddsa_public_key_pem(pem: &str) -> anyhow::Result<String> {
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

fn required_env(name: &'static str) -> anyhow::Result<String> {
    optional_env(name)?.ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn optional_env(name: &'static str) -> anyhow::Result<Option<String>> {
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

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use chrono::{Duration, Utc};
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use serde_json::json;
    use std::collections::BTreeMap;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::trace_corpus_storage::{
        TraceTenantAccessGrantRecord, TraceTenantAccessGrantRole, TraceTenantAccessGrantStatus,
    };
    use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

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
            admin_bind: None,
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
                denial_counter: Arc::clone(&self.denial_counter),
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

    fn write_allowlist_file(
        path: &std::path::Path,
        policy_label: &str,
        codes: &[&str],
    ) {
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
        let token = workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-OK-001");
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
        let parsed: serde_json::Value =
            serde_json::from_slice(&payload).expect("payload is json");
        assert_eq!(
            parsed.get("policy_label").and_then(|v| v.as_str()),
            Some("pilot-2026-05"),
            "minted JWT carries policy_label"
        );
    }

    #[tokio::test]
    async fn allowlist_refuses_unlisted_invite_with_named_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
        let config = config_with_file_allowlist(path);
        let token = workload_token_with_invite(
            "workload-issuer",
            "trace-claim-issuer",
            "INV-NOT-LISTED",
        );
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
        let token = workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-OK-001");
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
}
