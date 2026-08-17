//! Self-serve invite claims for NEAR Legion token holders.
//!
//! A holder proves control of a NEAR account with a NEP-413 wallet signature
//! and receives one invite code redeemable a fixed number of times. The proof
//! half reuses [`crate::account_near`] wholesale — the same NEP-413 encoder and
//! the same FullAccess-key binding check the authenticated account ceremony
//! uses. What is new here is the eligibility half: the token-ownership view
//! call, the denylist, the global cap, and the grant shape.
//!
//! Two properties are load-bearing and deliberately not application logic:
//!
//! - **One live grant per account** is the V42 partial unique index on
//!   `(policy_label, credential_binding_hash) WHERE revoked_at IS NULL`. A
//!   concurrent double-claim loses to Postgres, not to a check-then-act race.
//! - **The raw NEAR account ID is never stored.** It is recorded only as
//!   [`credential_binding_hash`], matching the site's stated privacy posture.
//!
//! The global cap is a soft bound: the count and the insert are not one
//! transaction, so concurrent claims can overshoot by the in-flight count.
//! Tightening it would mean a counter row and a lock for a bound that is
//! already an arbitrary operator number.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router};
use serde::Deserialize;

use crate::config::NearConfig;
use crate::trace_invite_registry::InviteRegistry as _;

/// Invite pool these grants live in. Distinct from the operator pool so the
/// unique index scopes per-account claims to Legion claims only, and so an
/// operator can list or revoke the cohort as a unit.
pub const POLICY_LABEL: &str = "near-legion";

/// Recorded on every grant so audit can separate self-serve claims from
/// operator-minted invites.
pub const ISSUANCE_SOURCE: &str = "near-legion-sbt";

/// Operator free text; never returned to non-admin callers.
pub const ISSUED_BY_LABEL: &str = "near-legion-claim";

/// The human-readable message the wallet displays and signs. Distinct from the
/// account-enroll message so a signature captured from one ceremony can never
/// be replayed into the other.
pub const CLAIM_MESSAGE: &str = "Claim Trace Commons invite codes for this NEAR account. This does not authorize a transaction.";

pub const DEFAULT_CONTRACT: &str = "nearlegion.nfts.tg";
pub const DEFAULT_CAP: u32 = 100;
pub const DEFAULT_MAX_USES: u32 = 3;
pub const DEFAULT_GRANT_TTL_DAYS: i64 = 30;

/// Accounts that hold Legion tokens but are not people. The treasury holds the
/// large majority of supply and `intents.near` is a contract that receives
/// tokens through swaps; without this list either could claim.
pub const DEFAULT_DENYLIST: &[&str] = &["nearlegion.near", "intents.near"];

/// Upper bound on an accepted NEAR account ID. The protocol maximum is 64.
const MAX_ACCOUNT_ID_LEN: usize = 64;

/// Resolved configuration for the claim surface. Constructed only when the
/// feature is explicitly enabled AND a tenant template is configured; a partial
/// set fails closed so the routes stay 404 rather than half-working.
#[derive(Debug, Clone)]
pub struct NearLegionConfig {
    pub contract: String,
    pub denylist: Vec<String>,
    pub cap: u32,
    pub tenant_template_id: String,
    pub max_uses: u32,
    pub grant_ttl_days: i64,
}

/// Every way a claim can be refused. Each maps to one public label and one
/// status; the labels are stable wire values the `/legion` page switches on to
/// render distinct copy, so a cap-reached refusal never reads as a signature
/// failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimError {
    AccountIdMalformed,
    ChallengeNonceInvalid,
    SignatureInvalid,
    PublicKeyNotFullAccess,
    AccountHoldsNoLegionToken,
    AccountNotEligible,
    InviteCredentialAlreadyBound,
    NearLegionClaimCapReached,
    NearRpcUnavailable,
    ClaimBackendUnavailable,
}

impl ClaimError {
    /// Stable public wire label. Never includes internal detail.
    pub fn public_label(self) -> &'static str {
        match self {
            Self::AccountIdMalformed => "AccountIdMalformed",
            Self::ChallengeNonceInvalid => "ChallengeNonceInvalid",
            Self::SignatureInvalid => "SignatureInvalid",
            Self::PublicKeyNotFullAccess => "PublicKeyNotFullAccess",
            Self::AccountHoldsNoLegionToken => "AccountHoldsNoLegionToken",
            Self::AccountNotEligible => "AccountNotEligible",
            Self::InviteCredentialAlreadyBound => "InviteCredentialAlreadyBound",
            Self::NearLegionClaimCapReached => "NearLegionClaimCapReached",
            Self::NearRpcUnavailable => "NearRpcUnavailable",
            Self::ClaimBackendUnavailable => "ClaimBackendUnavailable",
        }
    }

    /// HTTP status for this refusal.
    pub fn status(self) -> u16 {
        match self {
            Self::AccountIdMalformed
            | Self::ChallengeNonceInvalid
            | Self::SignatureInvalid
            | Self::PublicKeyNotFullAccess
            | Self::AccountHoldsNoLegionToken
            | Self::AccountNotEligible => 400,
            Self::InviteCredentialAlreadyBound | Self::NearLegionClaimCapReached => 409,
            Self::NearRpcUnavailable | Self::ClaimBackendUnavailable => 503,
        }
    }
}

impl NearLegionConfig {
    /// Load from the environment. Returns `None` unless
    /// `TRACE_COMMONS_NEAR_LEGION_ENABLED` is truthy and a non-blank
    /// `TRACE_COMMONS_NEAR_LEGION_TENANT_TEMPLATE` is present. Malformed
    /// numeric values fall back to the defaults rather than failing the
    /// process; an unparseable cap must not open the surface wider than
    /// intended, so the default is used and the value is ignored.
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("TRACE_COMMONS_NEAR_LEGION_ENABLED")
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                v == "1" || v == "true" || v == "yes"
            })
            .unwrap_or(false);
        if !enabled {
            return None;
        }

        let tenant_template_id = non_blank_env("TRACE_COMMONS_NEAR_LEGION_TENANT_TEMPLATE")?;

        let contract = non_blank_env("TRACE_COMMONS_NEAR_LEGION_CONTRACT")
            .unwrap_or_else(|| DEFAULT_CONTRACT.to_string());

        let denylist = match non_blank_env("TRACE_COMMONS_NEAR_LEGION_DENYLIST") {
            Some(raw) => parse_denylist(&raw),
            None => DEFAULT_DENYLIST.iter().map(|s| s.to_string()).collect(),
        };

        let cap = non_blank_env("TRACE_COMMONS_NEAR_LEGION_CLAIM_CAP")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|c| *c > 0)
            .unwrap_or(DEFAULT_CAP);

        let max_uses = non_blank_env("TRACE_COMMONS_NEAR_LEGION_MAX_USES")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|m| *m > 0)
            .unwrap_or(DEFAULT_MAX_USES);

        let grant_ttl_days = non_blank_env("TRACE_COMMONS_NEAR_LEGION_GRANT_TTL_DAYS")
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|d| *d > 0 && *d <= 3650)
            .unwrap_or(DEFAULT_GRANT_TTL_DAYS);

        Some(Self {
            contract,
            denylist,
            cap,
            tenant_template_id,
            max_uses,
            grant_ttl_days,
        })
    }

    /// True iff this account is excluded from claiming regardless of holdings.
    /// NEAR account IDs are canonically lowercase; the comparison is
    /// case-insensitive so a mixed-case denylist entry still bites.
    pub fn is_denylisted(&self, account_id: &str) -> bool {
        let candidate = account_id.trim();
        self.denylist
            .iter()
            .any(|d| d.trim().eq_ignore_ascii_case(candidate))
    }
}

fn non_blank_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn parse_denylist(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Hash a NEAR account ID for storage as `credential_binding_hash`.
///
/// The `near-account:` prefix is domain separation: it guarantees this digest
/// can never collide with an invite-code digest (which is prefixed `invite:`)
/// or with a bare SHA-256 of the account ID computed elsewhere. The raw account
/// ID is never persisted.
pub fn credential_binding_hash(account_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"near-account:");
    hasher.update(account_id.trim().as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Shape check on a NEAR account ID before it reaches an RPC call.
///
/// Mirrors the NEAR protocol rule: 2-64 chars, lowercase alphanumeric with
/// `.`, `-`, `_` separators, no leading/trailing/doubled separator. This is a
/// cheap reject for junk input, not a claim that the account exists.
pub fn is_valid_near_account_id(account_id: &str) -> bool {
    let id = account_id;
    if id.len() < 2 || id.len() > MAX_ACCOUNT_ID_LEN {
        return false;
    }
    let bytes = id.as_bytes();
    let is_sep = |b: u8| b == b'.' || b == b'-' || b == b'_';
    if is_sep(bytes[0]) || is_sep(bytes[bytes.len() - 1]) {
        return false;
    }
    let mut prev_sep = false;
    for &b in bytes {
        let alnum = b.is_ascii_lowercase() || b.is_ascii_digit();
        if alnum {
            prev_sep = false;
        } else if is_sep(b) {
            if prev_sep {
                return false;
            }
            prev_sep = true;
        } else {
            return false;
        }
    }
    true
}

/// Extract the token count from a `nft_supply_for_owner` JSON-RPC response.
///
/// The NEAR `call_function` envelope returns the contract's return value as a
/// byte array under `result.result`; those bytes are JSON. NEP-171 renders
/// `U128`-ish counts as a JSON *string* (`"3"`), but a contract returning a
/// bare number is also valid JSON, so both are accepted.
///
/// A JSON-RPC-level `error` (rate limiting, a bad contract, gas exhaustion)
/// yields `Err`, which the caller fails closed on rather than reading as zero.
pub fn parse_nft_supply_response(response: &serde_json::Value) -> Result<u64> {
    if response.get("error").is_some() {
        return Err(anyhow!("NEAR RPC returned a JSON-RPC error"));
    }
    let result = response
        .get("result")
        .ok_or_else(|| anyhow!("NEAR RPC response had no result"))?;
    // A contract-level failure (e.g. GasLimitExceeded) is reported inside
    // `result.error` while the envelope itself is a success.
    if result.get("error").is_some() {
        return Err(anyhow!("NEAR RPC reported a contract execution error"));
    }
    let bytes = result
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow!("NEAR RPC result had no return value"))?;
    let raw: Vec<u8> = bytes
        .iter()
        .map(|b| {
            b.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| anyhow!("NEAR RPC return value was not a byte array"))
        })
        .collect::<Result<_>>()?;
    let text = String::from_utf8(raw).context("NEAR RPC return value was not UTF-8")?;
    let value: serde_json::Value =
        serde_json::from_str(text.trim()).context("NEAR RPC return value was not JSON")?;
    match value {
        serde_json::Value::String(s) => s
            .trim()
            .parse::<u64>()
            .context("NEAR token supply was not a number"),
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| anyhow!("NEAR token supply was not a u64")),
        _ => Err(anyhow!("NEAR token supply had an unexpected JSON type")),
    }
}

/// Test seam for the token-ownership check, mirroring
/// [`crate::account_near::NearAccessKeyChecker`].
///
/// The live implementation is a network call that cannot run in a hermetic
/// test. The handler consults an optional override first and falls back to
/// [`near_account_holds_legion_token`] when none is installed. The production
/// path installs no override.
#[async_trait::async_trait]
pub trait NearLegionTokenChecker: Send + Sync {
    /// Return `true` iff `account_id` holds at least one token of the
    /// configured collection. Any error is fail-closed by the caller.
    async fn holds_legion_token(
        &self,
        near_cfg: &NearConfig,
        legion_cfg: &NearLegionConfig,
        account_id: &str,
    ) -> Result<bool>;
}

/// Live `nft_supply_for_owner` view call against the configured collection.
///
/// Bounded timeout: a hanging NEAR RPC must surface as `NearRpcUnavailable`
/// rather than holding the request open. Public NEAR RPC rate-limits
/// aggressively, so `NearConfig::rpc_url` should point at a keyed provider.
pub async fn near_account_holds_legion_token(
    near_cfg: &NearConfig,
    legion_cfg: &NearLegionConfig,
    account_id: &str,
) -> Result<bool> {
    use base64::Engine as _;

    let args = serde_json::json!({ "account_id": account_id }).to_string();
    let args_base64 = base64::engine::general_purpose::STANDARD.encode(args.as_bytes());

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "tc",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": legion_cfg.contract,
            "method_name": "nft_supply_for_owner",
            "args_base64": args_base64,
        },
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("NEAR RPC client build failed")?;
    let resp = client
        .post(&near_cfg.rpc_url)
        .json(&body)
        .send()
        .await
        .context("NEAR RPC request failed")?
        .error_for_status()
        .context("NEAR RPC returned error status")?;
    let json: serde_json::Value = resp
        .json()
        .await
        .context("NEAR RPC response was not valid JSON")?;

    Ok(parse_nft_supply_response(&json)? > 0)
}

/// The grant-store operations a claim needs, behind a trait so the handler is
/// testable without a live Postgres.
///
/// The production implementation is [`PgBackend`]; `count_live` is
/// `count_live_invite_grants` and `insert` is `insert_invite_grant`, so the
/// one-live-grant-per-account rule stays where it belongs — the V42 partial
/// unique index — rather than being re-implemented here.
#[async_trait::async_trait]
pub trait InviteGrantSink: Send + Sync {
    async fn count_live(&self, policy_label: &str) -> Result<u32>;
    async fn insert(
        &self,
        write: crate::db::InviteGrantWrite,
    ) -> Result<crate::db::InviteGrantInsertOutcome>;
}

#[async_trait::async_trait]
impl InviteGrantSink for crate::db::postgres::PgBackend {
    async fn count_live(&self, policy_label: &str) -> Result<u32> {
        self.count_live_invite_grants(policy_label)
            .await
            .context("invite grant count failed")
    }

    async fn insert(
        &self,
        write: crate::db::InviteGrantWrite,
    ) -> Result<crate::db::InviteGrantInsertOutcome> {
        self.insert_invite_grant(write)
            .await
            .context("invite grant insert failed")
    }
}

/// A pending claim challenge. Held single-use and TTL-bounded in a
/// [`crate::account_passkey::CeremonyStore`].
///
/// `account_id` is bound at issue time so a challenge minted for one account
/// cannot be redeemed against another. The signature check alone would already
/// catch that, but binding lets the mismatch be refused before any network call.
#[derive(Debug, Clone)]
pub struct NearLegionChallenge {
    pub nonce: [u8; 32],
    pub recipient: String,
    pub account_id: String,
}

/// State for the claim sub-router. Cheap to clone (all `Arc`).
///
/// The two `*_override` fields are test seams. Production constructs this with
/// both `None` and takes the live RPC paths; they exist as ordinary `Option`
/// fields rather than `#[cfg(test)]` ones so the issuer's construction site does
/// not need a second cfg-gated shape.
#[derive(Clone)]
pub struct NearLegionClaimState {
    pub sink: Arc<dyn InviteGrantSink>,
    pub registry: Option<Arc<crate::trace_invite_registry::DbInviteRegistry>>,
    pub near: Arc<NearConfig>,
    pub legion: Arc<NearLegionConfig>,
    pub challenges: Arc<crate::account_passkey::CeremonyStore<NearLegionChallenge>>,
    pub access_key_override: Option<Arc<dyn crate::account_near::NearAccessKeyChecker>>,
    pub token_checker_override: Option<Arc<dyn NearLegionTokenChecker>>,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeRequest {
    #[serde(rename = "accountId")]
    pub account_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ClaimRequest {
    #[serde(rename = "challengeId")]
    pub challenge_id: String,
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
    pub signature: String,
}

fn claim_refusal(error: ClaimError) -> (StatusCode, Json<serde_json::Value>) {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        Json(serde_json::json!({ "error": error.public_label() })),
    )
}

/// Allowed browser origins for the claim surface.
///
/// The `/legion` page is served from the community site, a different origin
/// from this issuer, so these routes need CORS of their own rather than relying
/// on a reverse proxy to add it. Defaults match the community surface's own
/// default origin list.
fn near_legion_cors_layer() -> tower_http::cors::CorsLayer {
    use axum::http::HeaderValue;
    use axum::http::header::{ACCEPT, CONTENT_TYPE};

    let configured = std::env::var("TRACE_COMMONS_NEAR_LEGION_CORS_ORIGINS").unwrap_or_else(|_| {
        "https://tracecommons.ai,http://localhost:4321,http://localhost:8788".to_string()
    });
    let origins: Vec<HeaderValue> = configured
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::list(origins))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([ACCEPT, CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(600))
}

/// Build the self-serve claim sub-router. Merged into the issuer's public
/// router only when the feature is configured, so an unconfigured deployment
/// serves 404 rather than a half-working surface.
pub fn near_legion_claim_router(state: NearLegionClaimState) -> Router {
    Router::new()
        .route(
            "/v1/onboard/near-legion/challenge",
            axum::routing::post(challenge_handler),
        )
        .route(
            "/v1/onboard/near-legion/claim",
            axum::routing::post(claim_handler),
        )
        .route(
            "/v1/onboard/near-legion/status",
            axum::routing::get(status_handler),
        )
        .layer(near_legion_cors_layer())
        .with_state(state)
}

impl NearLegionClaimState {
    /// Assemble from environment configuration and an already-connected invite
    /// backend. Returns `None` — leaving the routes unmounted — unless the
    /// feature is enabled, NEAR sign-in is configured, and an invite backend
    /// exists. A half-configured deployment 404s rather than half-working.
    pub fn from_env(
        backend: Option<Arc<crate::db::postgres::PgBackend>>,
        registry: Option<Arc<crate::trace_invite_registry::DbInviteRegistry>>,
    ) -> Option<Self> {
        let legion = NearLegionConfig::from_env()?;
        let near = NearConfig::from_env()?;
        let backend = backend?;
        Some(Self {
            sink: backend,
            registry,
            near: Arc::new(near),
            legion: Arc::new(legion),
            challenges: Arc::new(crate::account_passkey::CeremonyStore::new()),
            access_key_override: None,
            token_checker_override: None,
        })
    }
}

/// `POST /v1/onboard/near-legion/challenge` — issue a NEP-413 challenge.
///
/// Returns the challenge id in the RESPONSE BODY rather than a cookie. The
/// `/legion` page is served from a different origin than this issuer, and the
/// account ceremony's `SameSite=Strict` cookie would simply not be sent on that
/// cross-site request. The id is high-entropy, single-use and TTL-bounded, so
/// it is a capability handle in its own right.
async fn challenge_handler(
    State(state): State<NearLegionClaimState>,
    Json(request): Json<ChallengeRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    use rand::RngCore as _;

    let account_id = request.account_id.trim().to_string();
    if !is_valid_near_account_id(&account_id) {
        return claim_refusal(ClaimError::AccountIdMalformed);
    }

    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let recipient = state.near.recipient.clone();
    let challenge_id = crate::account_passkey::new_ceremony_id();

    state.challenges.put(
        challenge_id.clone(),
        NearLegionChallenge {
            nonce,
            recipient: recipient.clone(),
            account_id: account_id.clone(),
        },
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challengeId": challenge_id,
            "message": CLAIM_MESSAGE,
            // Hex on the wire; the stored bytes remain the verification truth.
            "nonce": hex::encode(nonce),
            "recipient": recipient,
        })),
    )
}

/// `POST /v1/onboard/near-legion/claim` — verify and mint.
///
/// Check order is deliberate. The signature comes first so nothing downstream
/// is observable without proving control of a key. The cap is checked next
/// because it is a cheap local read and a full pool should not spend two NEAR
/// RPC round-trips to refuse. Ownership — the expensive check — comes last
/// before the write.
async fn claim_handler(
    State(state): State<NearLegionClaimState>,
    Json(request): Json<ClaimRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let account_id = request.account_id.trim().to_string();
    if !is_valid_near_account_id(&account_id) {
        return claim_refusal(ClaimError::AccountIdMalformed);
    }

    // Single-use `take`: a replayed challenge id is already gone.
    let Some(challenge) = state.challenges.take(&request.challenge_id) else {
        return claim_refusal(ClaimError::ChallengeNonceInvalid);
    };
    if challenge.account_id != account_id {
        return claim_refusal(ClaimError::ChallengeNonceInvalid);
    }

    // Proof of key possession over the server-issued nonce.
    if crate::account_near::verify_nep413(
        &request.public_key,
        CLAIM_MESSAGE,
        &challenge.nonce,
        &challenge.recipient,
        None,
        &request.signature,
    )
    .is_err()
    {
        return claim_refusal(ClaimError::SignatureInvalid);
    }

    // Cheap local bound before any network call.
    let live = match state.sink.count_live(POLICY_LABEL).await {
        Ok(n) => n,
        Err(_) => return claim_refusal(ClaimError::ClaimBackendUnavailable),
    };
    if live >= state.legion.cap {
        return claim_refusal(ClaimError::NearLegionClaimCapReached);
    }

    if state.legion.is_denylisted(&account_id) {
        return claim_refusal(ClaimError::AccountNotEligible);
    }

    // BINDING CHECK: the signature proves possession of a key, not control of
    // the named account. Fail closed on any RPC error.
    let has_full_access = match &state.access_key_override {
        Some(checker) => {
            checker
                .has_full_access_key(&state.near, &account_id, &request.public_key)
                .await
        }
        None => {
            crate::account_near::near_account_has_full_access_key(
                &state.near,
                &account_id,
                &request.public_key,
            )
            .await
        }
    };
    match has_full_access {
        Ok(true) => {}
        Ok(false) => return claim_refusal(ClaimError::PublicKeyNotFullAccess),
        Err(_) => return claim_refusal(ClaimError::NearRpcUnavailable),
    }

    let holds = match &state.token_checker_override {
        Some(checker) => {
            checker
                .holds_legion_token(&state.near, &state.legion, &account_id)
                .await
        }
        None => near_account_holds_legion_token(&state.near, &state.legion, &account_id).await,
    };
    match holds {
        Ok(true) => {}
        Ok(false) => return claim_refusal(ClaimError::AccountHoldsNoLegionToken),
        // An RPC failure must not read as "holds nothing": that would be
        // indistinguishable from a genuine non-holder and would tell the user
        // to go mint a token they may already own.
        Err(_) => return claim_refusal(ClaimError::NearRpcUnavailable),
    }

    // The raw code exists here and in exactly one response body. It is never
    // stored, logged, or retrievable afterward.
    let code = crate::trace_invite_registry::generate_invite_code();
    let invite_subject_hash = crate::trace_upload_claim_allowlist::hash_invite_code(&code);
    let expires_at = chrono::Duration::try_days(state.legion.grant_ttl_days)
        .and_then(|d| chrono::Utc::now().checked_add_signed(d));

    let write = crate::db::InviteGrantWrite {
        invite_subject_hash: invite_subject_hash.clone(),
        policy_label: POLICY_LABEL.to_string(),
        tenant_mode: crate::trace_invite_registry::InviteTenantMode::Derived,
        fixed_tenant_id: None,
        tenant_template_id: Some(state.legion.tenant_template_id.clone()),
        policy_version: "v1".to_string(),
        allowed_consent_scopes: Vec::new(),
        allowed_uses: Vec::new(),
        max_uses: state.legion.max_uses,
        expires_at,
        issuance_source: ISSUANCE_SOURCE.to_string(),
        issued_by_label: Some(ISSUED_BY_LABEL.to_string()),
        // The only record of which account claimed. The raw ID is not stored.
        credential_binding_hash: Some(credential_binding_hash(&account_id)),
        note_label: None,
    };

    match state.sink.insert(write).await {
        Ok(crate::db::InviteGrantInsertOutcome::Inserted) => {
            // Invalidate AFTER the commit so the cache never advertises an
            // invite the database rejected.
            if let Some(registry) = &state.registry {
                registry.note_write(crate::trace_invite_registry::InviteEntry {
                    invite_subject_hash: invite_subject_hash.clone(),
                    policy_label: POLICY_LABEL.to_string(),
                    tenant_mode: crate::trace_invite_registry::InviteTenantMode::Derived,
                    fixed_tenant_id: None,
                    tenant_template_id: Some(state.legion.tenant_template_id.clone()),
                    policy_version: "v1".to_string(),
                    allowed_consent_scopes: Vec::new(),
                    allowed_uses: Vec::new(),
                    max_uses: state.legion.max_uses,
                    expires_at,
                    issuance_source: ISSUANCE_SOURCE.to_string(),
                    issued_by_label: Some(ISSUED_BY_LABEL.to_string()),
                    credential_binding_hash: Some(credential_binding_hash(&account_id)),
                    note_label: None,
                    revoked_at: None,
                });
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "inviteCode": code,
                    "maxUses": state.legion.max_uses,
                    "expiresAt": expires_at,
                })),
            )
        }
        // The V42 partial unique index refused a second live grant for this
        // account. This is the one-claim-per-account rule firing.
        Ok(crate::db::InviteGrantInsertOutcome::CredentialAlreadyBound) => {
            claim_refusal(ClaimError::InviteCredentialAlreadyBound)
        }
        // A hash collision on a CSPRNG code is not a real event; never return a
        // code whose grant fields belong to someone else.
        Ok(crate::db::InviteGrantInsertOutcome::AlreadyExists) | Err(_) => {
            claim_refusal(ClaimError::ClaimBackendUnavailable)
        }
    }
}

/// `GET /v1/onboard/near-legion/status` — public remaining-count.
///
/// Counts only. Lets the `/legion` page show scarcity up front instead of
/// surfacing it as a surprise 409 after a user has already signed.
async fn status_handler(
    State(state): State<NearLegionClaimState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let claimed = match state.sink.count_live(POLICY_LABEL).await {
        Ok(n) => n,
        Err(_) => return claim_refusal(ClaimError::ClaimBackendUnavailable),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "claimed": claimed,
            "cap": state.legion.cap,
            "remaining": state.legion.cap.saturating_sub(claimed),
            "maxUses": state.legion.max_uses,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> NearLegionConfig {
        NearLegionConfig {
            contract: DEFAULT_CONTRACT.to_string(),
            denylist: DEFAULT_DENYLIST.iter().map(|s| s.to_string()).collect(),
            cap: DEFAULT_CAP,
            tenant_template_id: "tpl".to_string(),
            max_uses: DEFAULT_MAX_USES,
            grant_ttl_days: DEFAULT_GRANT_TTL_DAYS,
        }
    }

    /// Wrap a contract return value the way the JSON-RPC envelope does.
    fn rpc_ok(return_value: &str) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "tc",
            "result": {
                "block_height": 1,
                "logs": [],
                "result": return_value.as_bytes().to_vec(),
            }
        })
    }

    #[test]
    fn credential_binding_hash_is_stable_namespaced_and_collision_free() {
        let a = credential_binding_hash("alice.near");
        assert_eq!(a, credential_binding_hash("alice.near"));
        assert!(a.starts_with("sha256:"));
        assert_eq!(a.len(), "sha256:".len() + 64);
        assert_ne!(a, credential_binding_hash("bob.near"));
        // Surrounding whitespace must not mint a second grant for one account.
        assert_eq!(a, credential_binding_hash("  alice.near  "));

        // Domain separation: the prefix means this can never collide with an
        // invite-code digest or a bare sha256 of the same string.
        let bare = {
            use sha2::{Digest, Sha256};
            format!("sha256:{}", hex::encode(Sha256::digest(b"alice.near")))
        };
        assert_ne!(a, bare);
        assert_ne!(
            a,
            crate::trace_upload_claim_allowlist::hash_invite_code("alice.near")
        );
    }

    #[test]
    fn credential_binding_hash_matches_the_v42_column_shape() {
        // The column CHECK is `^sha256:[0-9a-f]{64}$`. A hash that does not
        // match is rejected at insert time, not at claim time.
        let h = credential_binding_hash("alice.near");
        let hex_part = h.strip_prefix("sha256:").expect("sha256 prefix");
        assert_eq!(hex_part.len(), 64);
        assert!(
            hex_part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    #[test]
    fn denylist_rejects_treasury_and_contract_accounts() {
        let c = cfg();
        // The treasury holds the large majority of supply; without this it
        // could claim.
        assert!(c.is_denylisted("nearlegion.near"));
        assert!(c.is_denylisted("intents.near"));
        assert!(c.is_denylisted("  nearlegion.near  "));
        assert!(c.is_denylisted("NearLegion.near"));
        assert!(!c.is_denylisted("alice.near"));
        // A near-miss must not be swept up.
        assert!(!c.is_denylisted("nearlegion.tg"));
        assert!(!c.is_denylisted("notnearlegion.near"));
    }

    #[test]
    fn account_id_validation_accepts_real_shapes_and_rejects_junk() {
        for ok in [
            "alice.near",
            "nearlegion.nfts.tg",
            "a1-b2_c3.near",
            "6722398454.tg",
            "ab",
        ] {
            assert!(is_valid_near_account_id(ok), "should accept {ok}");
        }
        for bad in [
            "",
            "a",
            ".alice.near",
            "alice.near.",
            "alice..near",
            "Alice.near",
            "alice near",
            "alice@near",
            "alice/near",
            // Over the 64-char protocol maximum.
            &"a".repeat(65),
        ] {
            assert!(!is_valid_near_account_id(bad), "should reject {bad:?}");
        }
        // Exactly at the maximum is allowed.
        assert!(is_valid_near_account_id(&"a".repeat(64)));
    }

    #[test]
    fn nft_supply_parses_both_string_and_numeric_returns() {
        // NEP-171 renders counts as a JSON string.
        assert_eq!(parse_nft_supply_response(&rpc_ok("\"3\"")).unwrap(), 3);
        assert_eq!(parse_nft_supply_response(&rpc_ok("\"0\"")).unwrap(), 0);
        // A bare number is also valid JSON and must not be a parse failure.
        assert_eq!(parse_nft_supply_response(&rpc_ok("7")).unwrap(), 7);
        assert_eq!(parse_nft_supply_response(&rpc_ok("  \"12\" ")).unwrap(), 12);
    }

    /// Gold-standard vector: a byte-for-byte capture of what mainnet actually
    /// returned for `nft_supply_for_owner` on 2026-08-13, rather than a shape
    /// we assumed. `dimadze.near` held 50 and the treasury held 2683.
    #[test]
    fn nft_supply_parses_a_captured_mainnet_response() {
        let captured = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "tc",
            "result": {
                "block_hash": "4w9S7ecNq4Co7nADcU311cstYqSvXR8MFB23bpwVywXF",
                "block_height": 211_193_517u64,
                "logs": [],
                // The literal bytes of `"50"` — a JSON string, not a number.
                "result": [34, 53, 48, 34],
            }
        });
        assert_eq!(parse_nft_supply_response(&captured).unwrap(), 50);
    }

    #[test]
    fn nft_supply_fails_closed_on_every_error_envelope() {
        // A JSON-RPC transport error must never read as "holds nothing" — that
        // would be indistinguishable from a real non-holder, so the caller
        // needs an Err to turn into a 503 rather than a 400.
        let jsonrpc_error = serde_json::json!({
            "jsonrpc": "2.0", "id": "tc",
            "error": { "code": -32000, "message": "rate limited" }
        });
        assert!(parse_nft_supply_response(&jsonrpc_error).is_err());

        // A contract-level failure is reported inside a successful envelope.
        // This is the exact shape public RPC returns on gas exhaustion.
        let contract_error = serde_json::json!({
            "jsonrpc": "2.0", "id": "tc",
            "result": {
                "block_height": 1, "logs": [],
                "error": "wasm execution failed with error: HostError(GasLimitExceeded)"
            }
        });
        assert!(parse_nft_supply_response(&contract_error).is_err());

        for malformed in [
            serde_json::json!({}),
            serde_json::json!({ "result": {} }),
            serde_json::json!({ "result": { "result": "not-an-array" } }),
            serde_json::json!({ "result": { "result": [300] } }),
        ] {
            assert!(
                parse_nft_supply_response(&malformed).is_err(),
                "should reject {malformed}"
            );
        }
        // Valid bytes that are not a number.
        assert!(parse_nft_supply_response(&rpc_ok("\"abc\"")).is_err());
        assert!(parse_nft_supply_response(&rpc_ok("{}")).is_err());
    }

    #[test]
    fn error_labels_and_statuses_are_distinct_and_correctly_classed() {
        use ClaimError::*;
        let all = [
            AccountIdMalformed,
            ChallengeNonceInvalid,
            SignatureInvalid,
            PublicKeyNotFullAccess,
            AccountHoldsNoLegionToken,
            AccountNotEligible,
            InviteCredentialAlreadyBound,
            NearLegionClaimCapReached,
            NearRpcUnavailable,
            ClaimBackendUnavailable,
        ];
        // The /legion page switches on these to render distinct copy, so a
        // duplicate label would silently collapse two situations into one.
        let mut labels: Vec<&str> = all.iter().map(|e| e.public_label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "duplicate public label");

        // Conflicts are the two "you cannot have one" cases and must be 409 so
        // the page can distinguish them from bad input.
        assert_eq!(InviteCredentialAlreadyBound.status(), 409);
        assert_eq!(NearLegionClaimCapReached.status(), 409);
        // Upstream unavailability is retryable, not the caller's fault.
        assert_eq!(NearRpcUnavailable.status(), 503);
        assert_eq!(ClaimBackendUnavailable.status(), 503);
        assert_eq!(SignatureInvalid.status(), 400);
        assert_eq!(AccountHoldsNoLegionToken.status(), 400);
    }

    #[test]
    fn claim_message_is_distinct_from_the_account_enroll_message() {
        // A signature captured from one ceremony must not verify in the other.
        // Both flows share the encoder, the recipient, and the key; the
        // message is what separates them.
        assert_ne!(CLAIM_MESSAGE, "trace-commons-account-near-enroll");
        assert!(!CLAIM_MESSAGE.is_empty());
    }

    #[test]
    fn denylist_parsing_trims_and_drops_empties() {
        assert_eq!(
            parse_denylist("a.near, b.near ,,  c.near  "),
            vec!["a.near", "b.near", "c.near"]
        );
        assert!(parse_denylist("  ,, ").is_empty());
    }
}

/// End-to-end tests over the claim router, driven with real Ed25519 signatures.
///
/// The grant store, the access-key check, and the token check are all behind
/// traits, so every refusal path is exercised hermetically — no database and no
/// network.
#[cfg(test)]
mod router_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::sync::Mutex;
    use tower::ServiceExt as _;

    const TEST_RECIPIENT: &str = "tracecommons.ai";
    const HOLDER: &str = "alice.near";

    /// Independent mirror of the NEP-413 payload layout.
    ///
    /// Deliberately NOT reusing `account_near`'s private encoder: this pins the
    /// wire layout from the outside, so a change to the production struct that
    /// alters what wallets sign fails here instead of passing silently.
    #[derive(borsh::BorshSerialize)]
    struct TestNep413Payload {
        tag: u32,
        message: String,
        nonce: [u8; 32],
        recipient: String,
        callback_url: Option<String>,
    }

    fn keypair() -> Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn pubkey_string(kp: &Ed25519KeyPair) -> String {
        format!(
            "ed25519:{}",
            bs58::encode(kp.public_key().as_ref()).into_string()
        )
    }

    fn sign_claim(kp: &Ed25519KeyPair, nonce: &[u8; 32], recipient: &str, message: &str) -> String {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let payload = TestNep413Payload {
            tag: 2_147_484_061,
            message: message.to_string(),
            nonce: *nonce,
            recipient: recipient.to_string(),
            callback_url: None,
        };
        let bytes = borsh::to_vec(&payload).unwrap();
        let digest = Sha256::digest(&bytes);
        base64::engine::general_purpose::STANDARD.encode(kp.sign(digest.as_slice()).as_ref())
    }

    /// In-memory stand-in for the grant table. Reproduces the one behaviour the
    /// handler depends on: the V42 partial unique index refusing a second live
    /// grant for the same credential in the same pool.
    #[derive(Default)]
    struct FakeSink {
        grants: Mutex<Vec<crate::db::InviteGrantWrite>>,
        fail_count: bool,
        fail_insert: bool,
    }

    #[async_trait::async_trait]
    impl InviteGrantSink for FakeSink {
        async fn count_live(&self, policy_label: &str) -> Result<u32> {
            if self.fail_count {
                return Err(anyhow!("count unavailable"));
            }
            let g = self.grants.lock().unwrap();
            Ok(g.iter().filter(|w| w.policy_label == policy_label).count() as u32)
        }

        async fn insert(
            &self,
            write: crate::db::InviteGrantWrite,
        ) -> Result<crate::db::InviteGrantInsertOutcome> {
            if self.fail_insert {
                return Err(anyhow!("insert unavailable"));
            }
            let mut g = self.grants.lock().unwrap();
            let bound = g.iter().any(|w| {
                w.policy_label == write.policy_label
                    && w.credential_binding_hash.is_some()
                    && w.credential_binding_hash == write.credential_binding_hash
            });
            if bound {
                return Ok(crate::db::InviteGrantInsertOutcome::CredentialAlreadyBound);
            }
            g.push(write);
            Ok(crate::db::InviteGrantInsertOutcome::Inserted)
        }
    }

    #[derive(Clone, Copy)]
    enum Answer {
        Yes,
        No,
        Fails,
    }

    struct FakeKeyChecker(Answer);

    #[async_trait::async_trait]
    impl crate::account_near::NearAccessKeyChecker for FakeKeyChecker {
        async fn has_full_access_key(
            &self,
            _cfg: &NearConfig,
            _account_id: &str,
            _public_key: &str,
        ) -> Result<bool> {
            match self.0 {
                Answer::Yes => Ok(true),
                Answer::No => Ok(false),
                Answer::Fails => Err(anyhow!("rpc down")),
            }
        }
    }

    struct FakeTokenChecker(Answer);

    #[async_trait::async_trait]
    impl NearLegionTokenChecker for FakeTokenChecker {
        async fn holds_legion_token(
            &self,
            _near: &NearConfig,
            _legion: &NearLegionConfig,
            _account_id: &str,
        ) -> Result<bool> {
            match self.0 {
                Answer::Yes => Ok(true),
                Answer::No => Ok(false),
                Answer::Fails => Err(anyhow!("rpc down")),
            }
        }
    }

    struct Harness {
        state: NearLegionClaimState,
        sink: Arc<FakeSink>,
    }

    fn harness_with(cap: u32, key: Answer, token: Answer, sink: FakeSink) -> Harness {
        let sink = Arc::new(sink);
        let state = NearLegionClaimState {
            sink: sink.clone(),
            registry: None,
            near: Arc::new(NearConfig {
                rpc_url: "http://127.0.0.1:1/unused".to_string(),
                network: "mainnet".to_string(),
                recipient: TEST_RECIPIENT.to_string(),
            }),
            legion: Arc::new(NearLegionConfig {
                contract: DEFAULT_CONTRACT.to_string(),
                denylist: DEFAULT_DENYLIST.iter().map(|s| s.to_string()).collect(),
                cap,
                tenant_template_id: "tpl-legion".to_string(),
                max_uses: 3,
                grant_ttl_days: 30,
            }),
            challenges: Arc::new(crate::account_passkey::CeremonyStore::new()),
            access_key_override: Some(Arc::new(FakeKeyChecker(key))),
            token_checker_override: Some(Arc::new(FakeTokenChecker(token))),
        };
        Harness { state, sink }
    }

    fn harness() -> Harness {
        harness_with(100, Answer::Yes, Answer::Yes, FakeSink::default())
    }

    async fn call(
        state: &NearLegionClaimState,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let app = near_legion_claim_router(state.clone());
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(
                body.map(|b| Body::from(b.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// Issue a challenge and return `(challenge_id, nonce)`.
    async fn start_challenge(state: &NearLegionClaimState, account_id: &str) -> (String, [u8; 32]) {
        let (status, body) = call(
            state,
            "POST",
            "/v1/onboard/near-legion/challenge",
            Some(serde_json::json!({ "accountId": account_id })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "challenge failed: {body}");
        let id = body["challengeId"].as_str().unwrap().to_string();
        let nonce_hex = body["nonce"].as_str().unwrap();
        let mut nonce = [0u8; 32];
        hex::decode_to_slice(nonce_hex, &mut nonce).unwrap();
        (id, nonce)
    }

    fn error_of(body: &serde_json::Value) -> &str {
        body["error"].as_str().unwrap_or("<no error field>")
    }

    #[tokio::test]
    async fn a_holder_claims_a_code_and_the_grant_has_the_specified_shape() {
        let h = harness();
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;

        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED, "claim failed: {body}");
        let code = body["inviteCode"].as_str().expect("invite code returned");
        assert!(!code.is_empty());
        assert_eq!(body["maxUses"].as_u64(), Some(3));

        // The grant is shaped as the design specifies, and — the point of the
        // whole privacy posture — the raw account ID is nowhere in it.
        let grants = h.sink.grants.lock().unwrap();
        assert_eq!(grants.len(), 1);
        let g = &grants[0];
        assert_eq!(g.policy_label, POLICY_LABEL);
        assert_eq!(g.issuance_source, ISSUANCE_SOURCE);
        assert_eq!(g.max_uses, 3);
        assert_eq!(
            g.tenant_mode,
            crate::trace_invite_registry::InviteTenantMode::Derived
        );
        assert_eq!(g.tenant_template_id.as_deref(), Some("tpl-legion"));
        assert!(g.fixed_tenant_id.is_none());
        assert_eq!(
            g.credential_binding_hash.as_deref(),
            Some(credential_binding_hash(HOLDER).as_str())
        );
        assert!(g.expires_at.is_some());
        // The stored hash must not be reversible to, or contain, the account.
        assert!(!format!("{g:?}").contains(HOLDER));
        // The raw code is never persisted — only its hash.
        assert!(!format!("{g:?}").contains(code));
        assert_eq!(
            g.invite_subject_hash,
            crate::trace_upload_claim_allowlist::hash_invite_code(code)
        );
    }

    #[tokio::test]
    async fn a_challenge_is_single_use() {
        let h = harness();
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;
        let sig = sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE);
        let body = serde_json::json!({
            "challengeId": id,
            "accountId": HOLDER,
            "publicKey": pubkey_string(&kp),
            "signature": sig,
        });

        let (first, _) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(body.clone()),
        )
        .await;
        assert_eq!(first, StatusCode::CREATED);

        // Replaying the exact same signed request must not mint a second code.
        let (second, second_body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(body),
        )
        .await;
        assert_eq!(second, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&second_body), "ChallengeNonceInvalid");
        assert_eq!(h.sink.grants.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_challenge_issued_for_one_account_cannot_be_claimed_for_another() {
        let h = harness();
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;

        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": "mallory.near",
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "ChallengeNonceInvalid");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unknown_challenge_id_is_refused() {
        let h = harness();
        let kp = keypair();
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": "not-a-real-ceremony-id",
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &[7u8; 32], TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "ChallengeNonceInvalid");
    }

    #[tokio::test]
    async fn signatures_from_the_wrong_key_nonce_recipient_or_message_are_refused() {
        // Each case is a distinct way an attacker or a buggy client could
        // present a signature that verifies against *something* but not against
        // the challenge this server issued.
        /// Builds a signature from the caller's keypair and the issued nonce.
        type SigBuilder = Box<dyn Fn(&Ed25519KeyPair, &[u8; 32]) -> String>;

        let cases: Vec<(&str, SigBuilder)> = vec![
            (
                "wrong nonce",
                Box::new(|kp: &Ed25519KeyPair, _n: &[u8; 32]| {
                    sign_claim(kp, &[9u8; 32], TEST_RECIPIENT, CLAIM_MESSAGE)
                }),
            ),
            (
                "wrong recipient",
                Box::new(|kp: &Ed25519KeyPair, n: &[u8; 32]| {
                    sign_claim(kp, n, "evil.example", CLAIM_MESSAGE)
                }),
            ),
            (
                // The account-enroll ceremony's message. A signature harvested
                // from that flow must not be replayable into this one.
                "wrong message",
                Box::new(|kp: &Ed25519KeyPair, n: &[u8; 32]| {
                    sign_claim(kp, n, TEST_RECIPIENT, "trace-commons-account-near-enroll")
                }),
            ),
            (
                "signed by a different key",
                Box::new(|_kp: &Ed25519KeyPair, n: &[u8; 32]| {
                    sign_claim(&keypair(), n, TEST_RECIPIENT, CLAIM_MESSAGE)
                }),
            ),
            (
                "garbage signature",
                Box::new(|_, _| "not-base64!".to_string()),
            ),
        ];

        for (label, make_sig) in cases {
            let h = harness();
            let kp = keypair();
            let (id, nonce) = start_challenge(&h.state, HOLDER).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/near-legion/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "accountId": HOLDER,
                    "publicKey": pubkey_string(&kp),
                    "signature": make_sig(&kp, &nonce),
                })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{label} should be refused");
            assert_eq!(error_of(&body), "SignatureInvalid", "{label}");
            assert!(
                h.sink.grants.lock().unwrap().is_empty(),
                "{label} must not write a grant"
            );
        }
    }

    /// A valid signature proves possession of a key. It does not prove that key
    /// belongs to the named account — that is what the FullAccess check is for.
    #[tokio::test]
    async fn a_valid_signature_from_a_key_not_on_the_account_is_refused() {
        let h = harness_with(100, Answer::No, Answer::Yes, FakeSink::default());
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "PublicKeyNotFullAccess");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_account_holding_no_legion_token_is_refused() {
        let h = harness_with(100, Answer::Yes, Answer::No, FakeSink::default());
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "AccountHoldsNoLegionToken");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    /// The treasury holds 2683 of 3333 tokens. Without the denylist it would
    /// pass every other check.
    #[tokio::test]
    async fn the_treasury_and_contract_accounts_cannot_claim() {
        for account in ["nearlegion.near", "intents.near"] {
            let h = harness();
            let kp = keypair();
            let (id, nonce) = start_challenge(&h.state, account).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/near-legion/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "accountId": account,
                    "publicKey": pubkey_string(&kp),
                    "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
                })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{account}");
            assert_eq!(error_of(&body), "AccountNotEligible", "{account}");
            assert!(h.sink.grants.lock().unwrap().is_empty(), "{account}");
        }
    }

    #[tokio::test]
    async fn one_account_cannot_hold_two_live_grants() {
        let h = harness();

        for expected in [StatusCode::CREATED, StatusCode::CONFLICT] {
            let kp = keypair();
            let (id, nonce) = start_challenge(&h.state, HOLDER).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/near-legion/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "accountId": HOLDER,
                    "publicKey": pubkey_string(&kp),
                    "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
                })),
            )
            .await;
            assert_eq!(status, expected, "{body}");
            if expected == StatusCode::CONFLICT {
                assert_eq!(error_of(&body), "InviteCredentialAlreadyBound");
            }
        }
        // A fresh challenge and a different key do not get around it: the bind
        // is on the account, not the session or the key.
        assert_eq!(h.sink.grants.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn claims_stop_at_the_cap() {
        // Cap of 1, with one grant already issued to somebody else.
        let mut sink = FakeSink::default();
        sink.grants
            .get_mut()
            .unwrap()
            .push(crate::db::InviteGrantWrite {
                invite_subject_hash: "sha256:".to_string() + &"a".repeat(64),
                policy_label: POLICY_LABEL.to_string(),
                tenant_mode: crate::trace_invite_registry::InviteTenantMode::Derived,
                fixed_tenant_id: None,
                tenant_template_id: Some("tpl-legion".to_string()),
                policy_version: "v1".to_string(),
                allowed_consent_scopes: Vec::new(),
                allowed_uses: Vec::new(),
                max_uses: 3,
                expires_at: None,
                issuance_source: ISSUANCE_SOURCE.to_string(),
                issued_by_label: None,
                credential_binding_hash: Some(credential_binding_hash("someone-else.near")),
                note_label: None,
            });
        let h = harness_with(1, Answer::Yes, Answer::Yes, sink);

        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error_of(&body), "NearLegionClaimCapReached");
        assert_eq!(h.sink.grants.lock().unwrap().len(), 1);
    }

    /// An RPC failure must be a retryable 503, never a 400 that tells a real
    /// holder they hold nothing.
    #[tokio::test]
    async fn rpc_failures_surface_as_503_with_no_partial_write() {
        for (key, token) in [(Answer::Fails, Answer::Yes), (Answer::Yes, Answer::Fails)] {
            let h = harness_with(100, key, token, FakeSink::default());
            let kp = keypair();
            let (id, nonce) = start_challenge(&h.state, HOLDER).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/near-legion/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "accountId": HOLDER,
                    "publicKey": pubkey_string(&kp),
                    "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
                })),
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(error_of(&body), "NearRpcUnavailable");
            assert!(h.sink.grants.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn a_backend_failure_never_returns_a_code() {
        let h = harness_with(
            100,
            Answer::Yes,
            Answer::Yes,
            FakeSink {
                fail_insert: true,
                ..Default::default()
            },
        );
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_of(&body), "ClaimBackendUnavailable");
        assert!(body.get("inviteCode").is_none());
    }

    #[tokio::test]
    async fn malformed_account_ids_are_refused_before_a_challenge_is_minted() {
        let h = harness();
        for bad in ["", "A.near", "alice near", &"a".repeat(65)] {
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/near-legion/challenge",
                Some(serde_json::json!({ "accountId": bad })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}");
            assert_eq!(error_of(&body), "AccountIdMalformed", "{bad:?}");
        }
    }

    #[tokio::test]
    async fn status_reports_remaining_capacity() {
        let h = harness_with(5, Answer::Yes, Answer::Yes, FakeSink::default());
        let (status, body) = call(&h.state, "GET", "/v1/onboard/near-legion/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["claimed"].as_u64(), Some(0));
        assert_eq!(body["cap"].as_u64(), Some(5));
        assert_eq!(body["remaining"].as_u64(), Some(5));
        assert_eq!(body["maxUses"].as_u64(), Some(3));

        // After a claim the counter moves, so the page shows real scarcity.
        let kp = keypair();
        let (id, nonce) = start_challenge(&h.state, HOLDER).await;
        call(
            &h.state,
            "POST",
            "/v1/onboard/near-legion/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "accountId": HOLDER,
                "publicKey": pubkey_string(&kp),
                "signature": sign_claim(&kp, &nonce, TEST_RECIPIENT, CLAIM_MESSAGE),
            })),
        )
        .await;
        let (_, body) = call(&h.state, "GET", "/v1/onboard/near-legion/status", None).await;
        assert_eq!(body["claimed"].as_u64(), Some(1));
        assert_eq!(body["remaining"].as_u64(), Some(4));
    }

    #[tokio::test]
    async fn status_never_reports_negative_remaining_when_the_cap_is_overshot() {
        // The cap is a soft bound: concurrent claims can overshoot it. The
        // public counter must degrade to zero rather than underflow.
        let mut sink = FakeSink::default();
        for i in 0..3u8 {
            sink.grants
                .get_mut()
                .unwrap()
                .push(crate::db::InviteGrantWrite {
                    invite_subject_hash: format!("sha256:{}{}", i, "b".repeat(63)),
                    policy_label: POLICY_LABEL.to_string(),
                    tenant_mode: crate::trace_invite_registry::InviteTenantMode::Derived,
                    fixed_tenant_id: None,
                    tenant_template_id: Some("tpl-legion".to_string()),
                    policy_version: "v1".to_string(),
                    allowed_consent_scopes: Vec::new(),
                    allowed_uses: Vec::new(),
                    max_uses: 3,
                    expires_at: None,
                    issuance_source: ISSUANCE_SOURCE.to_string(),
                    issued_by_label: None,
                    credential_binding_hash: Some(credential_binding_hash(&format!("a{i}.near"))),
                    note_label: None,
                });
        }
        let h = harness_with(1, Answer::Yes, Answer::Yes, sink);
        let (status, body) = call(&h.state, "GET", "/v1/onboard/near-legion/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["remaining"].as_u64(), Some(0));
    }
}
