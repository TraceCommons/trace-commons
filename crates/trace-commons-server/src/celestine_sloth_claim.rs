//! Self-serve invite claims for Celestine Sloth Society holders.
//!
//! A holder proves control of a Cosmos account with an ADR-036 `signArbitrary`
//! signature, and receives one invite code redeemable a fixed number of times.
//! This is the Cosmos twin of [`crate::near_legion_claim`]: same wire shape,
//! same refusal discipline, same grant shape, different chain.
//!
//! It is a parallel module rather than a generalisation of the Legion path on
//! purpose. The chains differ, the signature schemes differ, the address formats
//! differ, and the ownership queries differ; a shared abstraction would have been
//! a slug plus a chain-shaped enum, which is a switch statement with extra
//! indirection rather than reuse. [`InviteGrantSink`] is the one genuinely shared
//! piece — it is already generic over `policy_label`, so it is imported.
//!
//! Two properties are load-bearing and deliberately not application logic:
//!
//! - **One live grant per address** is the V42 partial unique index on
//!   `(policy_label, credential_binding_hash) WHERE revoked_at IS NULL`. A
//!   concurrent double-claim loses to Postgres, not to a check-then-act race.
//! - **The raw bech32 address is never stored.** It is recorded only as
//!   [`credential_binding_hash`], matching the site's stated privacy posture.
//!
//! The substantive improvement over the Legion path is the binding check. A
//! secp256k1 public key *is* the address:
//!
//! ```text
//! addr = bech32(hrp, ripemd160(sha256(compressed_pubkey)))
//! ```
//!
//! so proving possession of the key and proving control of the address are the
//! same act. There is no RPC that can fail closed here — only
//! [`ClaimError::PublicKeyAddressMismatch`], computed locally.
//!
//! The global cap is a soft bound: the count and the insert are not one
//! transaction, so concurrent claims can overshoot by the in-flight count.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router};
use serde::Deserialize;

use crate::near_legion_claim::InviteGrantSink;
use crate::trace_invite_registry::InviteRegistry as _;

/// Invite pool these grants live in. Distinct from both the operator pool and
/// the Legion pool, so the unique index scopes per-address claims to this cohort
/// and an operator can list or revoke the cohort as a unit.
pub const POLICY_LABEL: &str = "celestine-sloths";

/// Recorded on every grant so audit can separate self-serve claims from
/// operator-minted invites, and this cohort from the Legion cohort.
pub const ISSUANCE_SOURCE: &str = "celestine-sloths-cw721";

/// Operator free text; never returned to non-admin callers.
pub const ISSUED_BY_LABEL: &str = "celestine-sloth-claim";

/// The human-readable message the wallet displays and signs. Distinct from both
/// the Legion claim message and the account-enroll message, so a signature
/// captured from one ceremony can never be replayed into another.
pub const CLAIM_MESSAGE: &str = "Claim Trace Commons invite codes for this Cosmos account. This does not authorize a transaction.";

/// Cosmos Hub default. Overridable because the collection has moved chains once
/// already and a redeploy elsewhere would change only the prefix.
pub const DEFAULT_BECH32_HRP: &str = "cosmos";

pub const DEFAULT_CAP: u32 = 100;
pub const DEFAULT_MAX_USES: u32 = 3;
pub const DEFAULT_GRANT_TTL_DAYS: i64 = 30;

/// A bech32 payload of exactly 20 bytes — the `ripemd160(sha256(pubkey))`
/// digest every Cosmos account address is built from.
const ADDRESS_PAYLOAD_LEN: usize = 20;

/// Resolved configuration for the claim surface. Constructed only when the
/// feature is explicitly enabled AND every required value is present; a partial
/// set fails closed so the routes stay 404 rather than half-working.
///
/// There is deliberately no default contract. The Legion module hardcodes
/// `nearlegion.nfts.tg` because that address is known and verified; the Sloth
/// contract address is not yet confirmed, and a wrong default that silently
/// queries a nonexistent contract is worse than an unmounted route.
#[derive(Debug, Clone)]
pub struct CelestineSlothConfig {
    pub contract: String,
    pub lcd_url: String,
    pub bech32_hrp: String,
    pub denylist: Vec<String>,
    pub cap: u32,
    pub tenant_template_id: String,
    pub max_uses: u32,
    pub grant_ttl_days: i64,
}

/// Every way a claim can be refused. Each maps to one public label and one
/// status; the labels are stable wire values the `/sloths` page switches on to
/// render distinct copy, so a cap-reached refusal never reads as a signature
/// failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimError {
    AddressMalformed,
    ChallengeNonceInvalid,
    SignatureInvalid,
    PublicKeyAddressMismatch,
    AccountHoldsNoSlothToken,
    AccountNotEligible,
    InviteCredentialAlreadyBound,
    CelestineSlothClaimCapReached,
    ChainRpcUnavailable,
    ClaimBackendUnavailable,
}

impl ClaimError {
    /// Stable public wire label. Never includes internal detail.
    pub fn public_label(self) -> &'static str {
        match self {
            Self::AddressMalformed => "AddressMalformed",
            Self::ChallengeNonceInvalid => "ChallengeNonceInvalid",
            Self::SignatureInvalid => "SignatureInvalid",
            Self::PublicKeyAddressMismatch => "PublicKeyAddressMismatch",
            Self::AccountHoldsNoSlothToken => "AccountHoldsNoSlothToken",
            Self::AccountNotEligible => "AccountNotEligible",
            Self::InviteCredentialAlreadyBound => "InviteCredentialAlreadyBound",
            Self::CelestineSlothClaimCapReached => "CelestineSlothClaimCapReached",
            Self::ChainRpcUnavailable => "ChainRpcUnavailable",
            Self::ClaimBackendUnavailable => "ClaimBackendUnavailable",
        }
    }

    /// HTTP status for this refusal.
    pub fn status(self) -> u16 {
        match self {
            Self::AddressMalformed
            | Self::ChallengeNonceInvalid
            | Self::SignatureInvalid
            | Self::PublicKeyAddressMismatch
            | Self::AccountHoldsNoSlothToken
            | Self::AccountNotEligible => 400,
            Self::InviteCredentialAlreadyBound | Self::CelestineSlothClaimCapReached => 409,
            Self::ChainRpcUnavailable | Self::ClaimBackendUnavailable => 503,
        }
    }
}

impl CelestineSlothConfig {
    /// Load from the environment. Returns `None` unless
    /// `TRACE_COMMONS_CELESTINE_SLOTHS_ENABLED` is truthy and non-blank
    /// `CONTRACT`, `LCD_URL` and `TENANT_TEMPLATE` are all present. Malformed
    /// numeric values fall back to the defaults rather than failing the process;
    /// an unparseable cap must not open the surface wider than intended.
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("TRACE_COMMONS_CELESTINE_SLOTHS_ENABLED")
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                v == "1" || v == "true" || v == "yes"
            })
            .unwrap_or(false);
        if !enabled {
            return None;
        }

        // All three are required with no default: a half-configured deployment
        // must 404 rather than half-work.
        let contract = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_CONTRACT")?;
        let lcd_url = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_LCD_URL")?;
        let tenant_template_id = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_TENANT_TEMPLATE")?;

        let bech32_hrp = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_BECH32_HRP")
            .unwrap_or_else(|| DEFAULT_BECH32_HRP.to_string());

        let denylist = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_DENYLIST")
            .map(|raw| parse_denylist(&raw))
            .unwrap_or_default();

        let cap = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_CAP")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|c| *c > 0)
            .unwrap_or(DEFAULT_CAP);

        let max_uses = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_MAX_USES")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|m| *m > 0)
            .unwrap_or(DEFAULT_MAX_USES);

        let grant_ttl_days = non_blank_env("TRACE_COMMONS_CELESTINE_SLOTHS_GRANT_TTL_DAYS")
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|d| *d > 0 && *d <= 3650)
            .unwrap_or(DEFAULT_GRANT_TTL_DAYS);

        Some(Self {
            contract,
            lcd_url,
            bech32_hrp,
            denylist,
            cap,
            tenant_template_id,
            max_uses,
            grant_ttl_days,
        })
    }

    /// True iff this address is excluded from claiming regardless of holdings.
    /// bech32 addresses are canonically lowercase; the comparison is
    /// case-insensitive so a mixed-case denylist entry still bites.
    pub fn is_denylisted(&self, address: &str) -> bool {
        let candidate = address.trim();
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

/// Hash a Cosmos address for storage as `credential_binding_hash`.
///
/// The `cosmos-account:` prefix is domain separation: it guarantees this digest
/// can never collide with a `near-account:` digest, an invite-code digest
/// (prefixed `invite:`), or a bare SHA-256 of the address computed elsewhere.
/// The raw address is never persisted.
pub fn credential_binding_hash(address: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"cosmos-account:");
    hasher.update(address.trim().as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Shape check on a bech32 account address before it reaches an LCD call.
///
/// Requires a well-formed bech32 string (checksum included), the configured
/// human-readable prefix, and a 20-byte payload — the `ripemd160(sha256(pk))`
/// digest every Cosmos account address is built from. This is a cheap reject for
/// junk input, not a claim that the account exists on chain.
pub fn is_valid_cosmos_address(address: &str, expected_hrp: &str) -> bool {
    match bech32::decode(address) {
        Ok((hrp, data)) => {
            hrp.as_str().eq_ignore_ascii_case(expected_hrp) && data.len() == ADDRESS_PAYLOAD_LEN
        }
        Err(_) => false,
    }
}

/// Derive the bech32 account address a secp256k1 public key controls.
///
/// `addr = bech32(hrp, ripemd160(sha256(compressed_pubkey)))`. Both compressed
/// (33-byte) and uncompressed (65-byte) SEC1 encodings are accepted on input;
/// the digest is always taken over the *compressed* form, because that is what
/// Cosmos hashes and an uncompressed key would otherwise derive a different —
/// and wrong — address.
pub fn address_from_public_key(public_key: &[u8], hrp: &str) -> Result<String> {
    use k256::ecdsa::VerifyingKey;
    use ripemd::Ripemd160;
    use sha2::{Digest, Sha256};

    let verifying_key = VerifyingKey::from_sec1_bytes(public_key)
        .context("public key was not a secp256k1 point")?;
    let compressed = verifying_key.to_encoded_point(true);
    let sha = Sha256::digest(compressed.as_bytes());
    let ripe = Ripemd160::digest(sha);

    let hrp = bech32::Hrp::parse(hrp).context("bech32 prefix was not valid")?;
    bech32::encode::<bech32::Bech32>(hrp, ripe.as_slice()).context("bech32 encoding failed")
}

/// The exact string a wallet is asked to sign.
///
/// The nonce is inside the signed text, not merely alongside it, so a signature
/// is bound to the server-issued challenge and cannot be lifted from one claim
/// attempt into another.
pub fn claim_sign_message(nonce_hex: &str) -> String {
    format!("{CLAIM_MESSAGE}\n\nNonce: {nonce_hex}")
}

/// Build the ADR-036 `sign/MsgSignData` document, byte-exact.
///
/// This is what the signature is computed over, so the layout is a wire
/// contract, not a formatting preference. Amino-JSON canonicalisation means no
/// whitespace and lexicographically sorted keys at every level; the literal is
/// written out rather than serialised through `serde_json` so that a map
/// implementation change can never silently reorder it and break every wallet
/// signature at once.
pub fn adr036_sign_doc(address: &str, message: &str) -> String {
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(message.as_bytes());
    format!(
        concat!(
            r#"{{"account_number":"0","chain_id":"","fee":{{"amount":[],"gas":"0"}},"#,
            r#""memo":"","msgs":[{{"type":"sign/MsgSignData","value":"#,
            r#"{{"data":"{data}","signer":"{signer}"}}}}],"sequence":"0"}}"#
        ),
        data = data,
        signer = address,
    )
}

/// Verify an ADR-036 signature and confirm the signing key controls the address.
///
/// Returns `Ok(())` only when the signature verifies AND the public key derives
/// exactly `address`. The two failures are distinct on the wire:
/// [`ClaimError::SignatureInvalid`] means the bytes do not check out;
/// [`ClaimError::PublicKeyAddressMismatch`] means they do, but for somebody
/// else's account.
///
/// `public_key` and `signature` are base64, matching what Keplr's and Leap's
/// `signArbitrary` return. The signature is 64-byte compact `r || s`; a
/// high-`s` signature is refused rather than normalised, because Cosmos
/// consensus requires low-`s` and accepting both halves of a malleable pair
/// would let one authorisation be presented as two distinct ones.
///
/// `k256`'s own `verify` already rejects high-`s`, so this check is redundant
/// today and a mutation test confirms removing it changes no observable
/// behaviour. It is kept as an explicit guard so the malleability property is
/// stated at the call site rather than inherited silently from an upstream
/// default that a version bump could relax.
pub fn verify_adr036(
    address: &str,
    hrp: &str,
    public_key_b64: &str,
    signature_b64: &str,
    message: &str,
) -> std::result::Result<(), ClaimError> {
    use base64::Engine as _;
    use k256::ecdsa::signature::Verifier as _;
    use k256::ecdsa::{Signature, VerifyingKey};

    let engine = base64::engine::general_purpose::STANDARD;
    let public_key = engine
        .decode(public_key_b64.trim())
        .map_err(|_| ClaimError::SignatureInvalid)?;
    let signature_bytes = engine
        .decode(signature_b64.trim())
        .map_err(|_| ClaimError::SignatureInvalid)?;

    let verifying_key =
        VerifyingKey::from_sec1_bytes(&public_key).map_err(|_| ClaimError::SignatureInvalid)?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|_| ClaimError::SignatureInvalid)?;
    // `normalize_s` returns `Some` only when the input was high-`s`.
    if signature.normalize_s().is_some() {
        return Err(ClaimError::SignatureInvalid);
    }

    let sign_doc = adr036_sign_doc(address, message);
    verifying_key
        .verify(sign_doc.as_bytes(), &signature)
        .map_err(|_| ClaimError::SignatureInvalid)?;

    // BINDING CHECK. The signature proves possession of a key; this proves the
    // key controls the claimed address. Purely local — it can never return
    // "unavailable", unlike the Legion path's FullAccess RPC.
    let derived =
        address_from_public_key(&public_key, hrp).map_err(|_| ClaimError::SignatureInvalid)?;
    if derived != address.trim() {
        return Err(ClaimError::PublicKeyAddressMismatch);
    }
    Ok(())
}

/// Extract "holds at least one token" from a `cw721` `tokens` smart-query
/// response.
///
/// The LCD wraps the contract's answer as `{"data":{"tokens":[...]}}`. A
/// non-empty array means the address holds at least one token; the count is
/// irrelevant, which is why the query passes `limit: 1`.
///
/// A contract-level or gateway-level error (`code`/`message`, a missing
/// `data`, a malformed body) yields `Err`, which the caller fails closed on
/// rather than reading as zero.
pub fn parse_cw721_tokens_response(response: &serde_json::Value) -> Result<bool> {
    // The LCD reports contract errors as a gRPC-gateway envelope with a numeric
    // `code`, alongside a `message`. Treating that as "no tokens" would tell a
    // genuine holder to go buy an NFT they already own.
    if response.get("code").is_some() {
        return Err(anyhow!("LCD returned a gateway error"));
    }
    let data = response
        .get("data")
        .ok_or_else(|| anyhow!("LCD response had no data"))?;
    let tokens = data
        .get("tokens")
        .and_then(|t| t.as_array())
        .ok_or_else(|| anyhow!("cw721 response had no tokens array"))?;
    Ok(!tokens.is_empty())
}

/// Test seam for the token-ownership check, mirroring
/// [`crate::near_legion_claim::NearLegionTokenChecker`].
///
/// The live implementation is a network call that cannot run in a hermetic
/// test. The handler consults an optional override first and falls back to
/// [`cosmos_account_holds_sloth_token`] when none is installed. The production
/// path installs no override.
#[async_trait::async_trait]
pub trait CelestineSlothTokenChecker: Send + Sync {
    /// Return `true` iff `address` holds at least one token of the configured
    /// collection. Any error is fail-closed by the caller.
    async fn holds_sloth_token(&self, cfg: &CelestineSlothConfig, address: &str) -> Result<bool>;
}

/// Build the CosmWasm smart-query URL for an ownership check.
///
/// The query argument travels base64-encoded in the path, which is the LCD's
/// contract for `/cosmwasm/wasm/v1/contract/{addr}/smart/{query}`. `limit: 1`
/// keeps the response bounded.
pub fn cw721_tokens_query_url(lcd_url: &str, contract: &str, owner: &str) -> String {
    use base64::Engine as _;
    let query = serde_json::json!({ "tokens": { "owner": owner, "limit": 1 } }).to_string();
    let encoded = base64::engine::general_purpose::STANDARD.encode(query.as_bytes());
    format!(
        "{}/cosmwasm/wasm/v1/contract/{}/smart/{}",
        lcd_url.trim_end_matches('/'),
        contract,
        // The base64 alphabet includes `+` and `/`, both of which change meaning
        // inside a URL path segment.
        urlencode_path_segment(&encoded),
    )
}

/// Percent-encode the characters that are unsafe in a URL path segment.
///
/// Standard base64 emits `+`, `/` and `=`; `/` would split the path and `+`
/// is ambiguous in some gateways. Hand-rolled rather than pulling a
/// percent-encoding crate for three characters.
fn urlencode_path_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Live `cw721 tokens{owner, limit: 1}` smart query against the configured
/// collection.
///
/// Bounded timeout: a hanging LCD must surface as `ChainRpcUnavailable` rather
/// than holding the request open. Both a transport failure and a contract-level
/// error are `Err`; the caller fails closed on `Err`.
pub async fn cosmos_account_holds_sloth_token(
    cfg: &CelestineSlothConfig,
    address: &str,
) -> Result<bool> {
    let url = cw721_tokens_query_url(&cfg.lcd_url, &cfg.contract, address);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("LCD client build failed")?;
    let resp = client
        .get(&url)
        .send()
        .await
        .context("LCD request failed")?
        .error_for_status()
        .context("LCD returned error status")?;
    let json: serde_json::Value = resp
        .json()
        .await
        .context("LCD response was not valid JSON")?;

    parse_cw721_tokens_response(&json)
}

/// A pending claim challenge. Held single-use and TTL-bounded in a
/// [`crate::account_passkey::CeremonyStore`].
///
/// `address` is bound at issue time so a challenge minted for one address
/// cannot be redeemed against another. The signature check alone would already
/// catch that, but binding lets the mismatch be refused before any work.
#[derive(Debug, Clone)]
pub struct CelestineSlothChallenge {
    pub nonce: [u8; 32],
    pub address: String,
}

/// State for the claim sub-router. Cheap to clone (all `Arc`).
///
/// `token_checker_override` is a test seam. Production constructs this with
/// `None` and takes the live LCD path; it is an ordinary `Option` field rather
/// than a `#[cfg(test)]` one so the issuer's construction site does not need a
/// second cfg-gated shape.
#[derive(Clone)]
pub struct CelestineSlothClaimState {
    pub sink: Arc<dyn InviteGrantSink>,
    pub registry: Option<Arc<crate::trace_invite_registry::DbInviteRegistry>>,
    pub sloths: Arc<CelestineSlothConfig>,
    pub challenges: Arc<crate::account_passkey::CeremonyStore<CelestineSlothChallenge>>,
    pub token_checker_override: Option<Arc<dyn CelestineSlothTokenChecker>>,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeRequest {
    pub address: String,
}

#[derive(Debug, Deserialize)]
pub struct ClaimRequest {
    #[serde(rename = "challengeId")]
    pub challenge_id: String,
    pub address: String,
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
/// The `/sloths` page is served from the community site, a different origin from
/// this issuer, so these routes need CORS of their own rather than relying on a
/// reverse proxy to add it. Defaults match the community surface's own default
/// origin list.
fn celestine_sloth_cors_layer() -> tower_http::cors::CorsLayer {
    use axum::http::HeaderValue;
    use axum::http::header::{ACCEPT, CONTENT_TYPE};

    let configured =
        std::env::var("TRACE_COMMONS_CELESTINE_SLOTHS_CORS_ORIGINS").unwrap_or_else(|_| {
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
pub fn celestine_sloth_claim_router(state: CelestineSlothClaimState) -> Router {
    Router::new()
        .route(
            "/v1/onboard/celestine-sloths/challenge",
            axum::routing::post(challenge_handler),
        )
        .route(
            "/v1/onboard/celestine-sloths/claim",
            axum::routing::post(claim_handler),
        )
        .route(
            "/v1/onboard/celestine-sloths/status",
            axum::routing::get(status_handler),
        )
        .layer(celestine_sloth_cors_layer())
        .with_state(state)
}

impl CelestineSlothClaimState {
    /// Assemble from environment configuration and an already-connected invite
    /// backend. Returns `None` — leaving the routes unmounted — unless the
    /// feature is fully configured and an invite backend exists. A
    /// half-configured deployment 404s rather than half-working.
    ///
    /// Note there is no `NearConfig` equivalent to require: this path needs no
    /// chain sign-in configuration, only the collection and the LCD.
    pub fn from_env(
        backend: Option<Arc<crate::db::postgres::PgBackend>>,
        registry: Option<Arc<crate::trace_invite_registry::DbInviteRegistry>>,
    ) -> Option<Self> {
        let sloths = CelestineSlothConfig::from_env()?;
        let backend = backend?;
        Some(Self {
            sink: backend,
            registry,
            sloths: Arc::new(sloths),
            challenges: Arc::new(crate::account_passkey::CeremonyStore::new()),
            token_checker_override: None,
        })
    }
}

/// `POST /v1/onboard/celestine-sloths/challenge` — issue an ADR-036 challenge.
///
/// Returns the challenge id in the RESPONSE BODY rather than a cookie. The
/// `/sloths` page is served from a different origin than this issuer, and a
/// `SameSite=Strict` cookie would simply not be sent on that cross-site
/// request. The id is high-entropy, single-use and TTL-bounded, so it is a
/// capability handle in its own right.
///
/// `signDoc` is returned in full so the page signs exactly what the server will
/// verify, rather than reconstructing the document and drifting from it.
async fn challenge_handler(
    State(state): State<CelestineSlothClaimState>,
    Json(request): Json<ChallengeRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    use rand::RngCore as _;

    let address = request.address.trim().to_string();
    if !is_valid_cosmos_address(&address, &state.sloths.bech32_hrp) {
        return claim_refusal(ClaimError::AddressMalformed);
    }

    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let nonce_hex = hex::encode(nonce);
    let challenge_id = crate::account_passkey::new_ceremony_id();
    let message = claim_sign_message(&nonce_hex);
    let sign_doc = adr036_sign_doc(&address, &message);

    state.challenges.put(
        challenge_id.clone(),
        CelestineSlothChallenge {
            nonce,
            address: address.clone(),
        },
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challengeId": challenge_id,
            "message": message,
            // Hex on the wire; the stored bytes remain the verification truth.
            "nonce": nonce_hex,
            "signDoc": sign_doc,
        })),
    )
}

/// `POST /v1/onboard/celestine-sloths/claim` — verify and mint.
///
/// Check order is deliberate. The signature comes first so nothing downstream is
/// observable without proving control of a key; the address binding rides along
/// with it, being a local computation over the same key. Then the cap, a cheap
/// local read — a full pool should not spend a network round-trip to refuse.
/// Then the denylist. Ownership, the only network call, comes last before the
/// write.
async fn claim_handler(
    State(state): State<CelestineSlothClaimState>,
    Json(request): Json<ClaimRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let address = request.address.trim().to_string();
    if !is_valid_cosmos_address(&address, &state.sloths.bech32_hrp) {
        return claim_refusal(ClaimError::AddressMalformed);
    }

    // Single-use `take`: a replayed challenge id is already gone.
    let Some(challenge) = state.challenges.take(&request.challenge_id) else {
        return claim_refusal(ClaimError::ChallengeNonceInvalid);
    };
    if challenge.address != address {
        return claim_refusal(ClaimError::ChallengeNonceInvalid);
    }

    // Proof of key possession over the server-issued nonce, plus the local
    // proof that the key controls this address.
    let message = claim_sign_message(&hex::encode(challenge.nonce));
    if let Err(error) = verify_adr036(
        &address,
        &state.sloths.bech32_hrp,
        &request.public_key,
        &request.signature,
        &message,
    ) {
        return claim_refusal(error);
    }

    // Cheap local bound before any network call.
    let live = match state.sink.count_live(POLICY_LABEL).await {
        Ok(n) => n,
        Err(_) => return claim_refusal(ClaimError::ClaimBackendUnavailable),
    };
    if live >= state.sloths.cap {
        return claim_refusal(ClaimError::CelestineSlothClaimCapReached);
    }

    if state.sloths.is_denylisted(&address) {
        return claim_refusal(ClaimError::AccountNotEligible);
    }

    let holds = match &state.token_checker_override {
        Some(checker) => checker.holds_sloth_token(&state.sloths, &address).await,
        None => cosmos_account_holds_sloth_token(&state.sloths, &address).await,
    };
    match holds {
        Ok(true) => {}
        Ok(false) => return claim_refusal(ClaimError::AccountHoldsNoSlothToken),
        // An LCD failure must not read as "holds nothing": that would be
        // indistinguishable from a genuine non-holder and would tell the user
        // to go buy an NFT they may already own.
        Err(_) => return claim_refusal(ClaimError::ChainRpcUnavailable),
    }

    // The raw code exists here and in exactly one response body. It is never
    // stored, logged, or retrievable afterward.
    let code = crate::trace_invite_registry::generate_invite_code();
    let invite_subject_hash = crate::trace_upload_claim_allowlist::hash_invite_code(&code);
    let expires_at = chrono::Duration::try_days(state.sloths.grant_ttl_days)
        .and_then(|d| chrono::Utc::now().checked_add_signed(d));

    let write = crate::db::InviteGrantWrite {
        invite_subject_hash: invite_subject_hash.clone(),
        policy_label: POLICY_LABEL.to_string(),
        tenant_mode: crate::trace_invite_registry::InviteTenantMode::Derived,
        fixed_tenant_id: None,
        tenant_template_id: Some(state.sloths.tenant_template_id.clone()),
        policy_version: "v1".to_string(),
        allowed_consent_scopes: Vec::new(),
        allowed_uses: Vec::new(),
        max_uses: state.sloths.max_uses,
        expires_at,
        issuance_source: ISSUANCE_SOURCE.to_string(),
        issued_by_label: Some(ISSUED_BY_LABEL.to_string()),
        // The only record of which address claimed. The raw address is not stored.
        credential_binding_hash: Some(credential_binding_hash(&address)),
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
                    tenant_template_id: Some(state.sloths.tenant_template_id.clone()),
                    policy_version: "v1".to_string(),
                    allowed_consent_scopes: Vec::new(),
                    allowed_uses: Vec::new(),
                    max_uses: state.sloths.max_uses,
                    expires_at,
                    issuance_source: ISSUANCE_SOURCE.to_string(),
                    issued_by_label: Some(ISSUED_BY_LABEL.to_string()),
                    credential_binding_hash: Some(credential_binding_hash(&address)),
                    note_label: None,
                    revoked_at: None,
                });
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "inviteCode": code,
                    "maxUses": state.sloths.max_uses,
                    "expiresAt": expires_at,
                })),
            )
        }
        // The V42 partial unique index refused a second live grant for this
        // address. This is the one-claim-per-address rule firing.
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

/// `GET /v1/onboard/celestine-sloths/status` — public remaining-count.
///
/// Counts only. Lets the `/sloths` page show scarcity up front instead of
/// surfacing it as a surprise 409 after a user has already signed.
async fn status_handler(
    State(state): State<CelestineSlothClaimState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let claimed = match state.sink.count_live(POLICY_LABEL).await {
        Ok(n) => n,
        Err(_) => return claim_refusal(ClaimError::ClaimBackendUnavailable),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "claimed": claimed,
            "cap": state.sloths.cap,
            "remaining": state.sloths.cap.saturating_sub(claimed),
            "maxUses": state.sloths.max_uses,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known secp256k1 test vector. This is the private key `1`, whose public
    /// key is the secp256k1 generator point — the most widely published pair
    /// there is, so the derived address can be checked against an independent
    /// source rather than against our own implementation.
    const GENERATOR_COMPRESSED: &str =
        "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798";
    const GENERATOR_UNCOMPRESSED: &str = concat!(
        "0479BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798",
        "483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8"
    );

    fn cfg() -> CelestineSlothConfig {
        CelestineSlothConfig {
            contract: "cosmos1contract".to_string(),
            lcd_url: "https://lcd.example".to_string(),
            bech32_hrp: DEFAULT_BECH32_HRP.to_string(),
            denylist: vec!["cosmos1treasury".to_string()],
            cap: DEFAULT_CAP,
            tenant_template_id: "tpl".to_string(),
            max_uses: DEFAULT_MAX_USES,
            grant_ttl_days: DEFAULT_GRANT_TTL_DAYS,
        }
    }

    /// The address the generator public key controls, computed once and pinned.
    fn generator_address() -> String {
        address_from_public_key(&hex::decode(GENERATOR_COMPRESSED).unwrap(), "cosmos").unwrap()
    }

    #[test]
    fn address_derivation_is_the_documented_pipeline() {
        // Verify against a hand-computed ripemd160(sha256(pk)) rather than
        // against `address_from_public_key` itself, so the test cannot pass by
        // agreeing with a bug in the function under test.
        use ripemd::Ripemd160;
        use sha2::{Digest, Sha256};
        let pk = hex::decode(GENERATOR_COMPRESSED).unwrap();
        let expected_payload = Ripemd160::digest(Sha256::digest(&pk));
        let derived = address_from_public_key(&pk, "cosmos").unwrap();

        let (hrp, data) = bech32::decode(&derived).unwrap();
        assert_eq!(hrp.as_str(), "cosmos");
        assert_eq!(data.as_slice(), expected_payload.as_slice());
        assert_eq!(data.len(), ADDRESS_PAYLOAD_LEN);
        assert!(derived.starts_with("cosmos1"));
    }

    #[test]
    fn uncompressed_and_compressed_keys_derive_the_same_address() {
        // Cosmos hashes the compressed form. Accepting an uncompressed key but
        // hashing it as-is would derive a different — and wrong — address, so a
        // holder presenting an uncompressed key would be told their key does not
        // control their own account.
        let compressed = hex::decode(GENERATOR_COMPRESSED).unwrap();
        let uncompressed = hex::decode(GENERATOR_UNCOMPRESSED).unwrap();
        assert_eq!(
            address_from_public_key(&compressed, "cosmos").unwrap(),
            address_from_public_key(&uncompressed, "cosmos").unwrap()
        );
    }

    #[test]
    fn a_non_default_hrp_changes_only_the_prefix() {
        // The collection has moved chains once already; the payload must not
        // depend on the prefix.
        let pk = hex::decode(GENERATOR_COMPRESSED).unwrap();
        let cosmos = address_from_public_key(&pk, "cosmos").unwrap();
        let stars = address_from_public_key(&pk, "stars").unwrap();
        assert!(stars.starts_with("stars1"));
        assert_ne!(cosmos, stars);
        assert_eq!(
            bech32::decode(&cosmos).unwrap().1,
            bech32::decode(&stars).unwrap().1
        );
    }

    #[test]
    fn address_derivation_rejects_junk_keys() {
        for bad in [vec![], vec![0u8; 33], vec![2u8; 10], vec![9u8; 65]] {
            assert!(
                address_from_public_key(&bad, "cosmos").is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn address_validation_accepts_real_addresses_and_rejects_junk() {
        let good = generator_address();
        assert!(is_valid_cosmos_address(&good, "cosmos"));
        // Wrong chain prefix: a Stargaze address must not claim on a Hub pool.
        assert!(!is_valid_cosmos_address(&good, "stars"));

        for bad in [
            "",
            "cosmos1",
            "not-an-address",
            // Correct shape, corrupted checksum.
            &format!("{}x", &good[..good.len() - 1]),
            // A valid bech32 string whose payload is not 20 bytes.
            &bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("cosmos").unwrap(), &[0u8; 32])
                .unwrap(),
        ] {
            assert!(
                !is_valid_cosmos_address(bad, "cosmos"),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn credential_binding_hash_is_stable_namespaced_and_collision_free() {
        let addr = generator_address();
        let a = credential_binding_hash(&addr);
        assert_eq!(a, credential_binding_hash(&addr));
        assert!(a.starts_with("sha256:"));
        assert_eq!(a.len(), "sha256:".len() + 64);
        assert_ne!(a, credential_binding_hash("cosmos1someoneelse"));
        // Surrounding whitespace must not mint a second grant for one address.
        assert_eq!(a, credential_binding_hash(&format!("  {addr}  ")));

        // Domain separation: this digest can never collide with the Legion
        // module's `near-account:` digest over the same string, with an
        // invite-code digest, or with a bare sha256.
        assert_ne!(a, crate::near_legion_claim::credential_binding_hash(&addr));
        assert_ne!(
            a,
            crate::trace_upload_claim_allowlist::hash_invite_code(&addr)
        );
        let bare = {
            use sha2::{Digest, Sha256};
            format!("sha256:{}", hex::encode(Sha256::digest(addr.as_bytes())))
        };
        assert_ne!(a, bare);
    }

    #[test]
    fn credential_binding_hash_matches_the_v42_column_shape() {
        // The column CHECK is `^sha256:[0-9a-f]{64}$`. A hash that does not
        // match is rejected at insert time, not at claim time.
        let h = credential_binding_hash(&generator_address());
        let hex_part = h.strip_prefix("sha256:").expect("sha256 prefix");
        assert_eq!(hex_part.len(), 64);
        assert!(
            hex_part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    /// The sign doc is what signatures are computed over, so a formatting drift
    /// is a silent authentication break rather than a visible failure. This
    /// pins it byte-for-byte.
    #[test]
    fn adr036_sign_doc_is_byte_exact() {
        let doc = adr036_sign_doc("cosmos1abc", "hello");
        assert_eq!(
            doc,
            concat!(
                r#"{"account_number":"0","chain_id":"","fee":{"amount":[],"gas":"0"},"#,
                r#""memo":"","msgs":[{"type":"sign/MsgSignData","value":"#,
                r#"{"data":"aGVsbG8=","signer":"cosmos1abc"}}],"sequence":"0"}"#
            )
        );
        // Amino JSON is canonical: no whitespace, and keys sorted at every
        // level. Both are properties wallets rely on.
        assert!(!doc.contains(' '));
        assert!(!doc.contains('\n'));
        for (earlier, later) in [
            ("account_number", "chain_id"),
            ("chain_id", "fee"),
            ("\"amount\"", "\"gas\""),
            ("fee", "memo"),
            ("memo", "msgs"),
            ("\"type\"", "\"value\""),
            ("\"data\"", "\"signer\""),
            ("msgs", "sequence"),
        ] {
            assert!(
                doc.find(earlier).unwrap() < doc.find(later).unwrap(),
                "{earlier} must sort before {later}"
            );
        }
        // Round-trips as JSON, so a wallet parsing it sees the same document.
        let parsed: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(parsed["msgs"][0]["value"]["signer"], "cosmos1abc");
    }

    #[test]
    fn the_sign_message_binds_the_nonce() {
        let a = claim_sign_message("aa");
        let b = claim_sign_message("bb");
        assert_ne!(a, b);
        assert!(a.contains(CLAIM_MESSAGE));
        assert!(a.contains("aa"));
    }

    #[test]
    fn claim_message_is_distinct_from_the_other_ceremonies() {
        // A signature captured from one ceremony must not verify in another.
        assert_ne!(CLAIM_MESSAGE, crate::near_legion_claim::CLAIM_MESSAGE);
        assert_ne!(CLAIM_MESSAGE, "trace-commons-account-near-enroll");
        assert!(!CLAIM_MESSAGE.is_empty());
    }

    #[test]
    fn policy_identifiers_are_distinct_from_the_legion_cohort() {
        // The V42 unique index is scoped by policy_label; sharing one would
        // make holding both NFTs yield one grant instead of two.
        assert_ne!(POLICY_LABEL, crate::near_legion_claim::POLICY_LABEL);
        assert_ne!(ISSUANCE_SOURCE, crate::near_legion_claim::ISSUANCE_SOURCE);
        assert_ne!(ISSUED_BY_LABEL, crate::near_legion_claim::ISSUED_BY_LABEL);
    }

    #[test]
    fn cw721_response_parsing_distinguishes_holders_from_non_holders() {
        assert!(
            !parse_cw721_tokens_response(&serde_json::json!({ "data": { "tokens": [] } })).unwrap()
        );
        assert!(
            parse_cw721_tokens_response(&serde_json::json!({ "data": { "tokens": ["1"] } }))
                .unwrap()
        );
        // `limit: 1` bounds the response, but more than one must still read as
        // "holds something" rather than as a surprise.
        assert!(
            parse_cw721_tokens_response(&serde_json::json!({ "data": { "tokens": ["1", "2"] } }))
                .unwrap()
        );
    }

    #[test]
    fn cw721_response_parsing_fails_closed_on_every_error_shape() {
        // A gateway error must never read as "holds nothing" — that would be
        // indistinguishable from a real non-holder, so the caller needs an Err
        // to turn into a 503 rather than a 400.
        let gateway_error = serde_json::json!({
            "code": 2,
            "message": "contract: not found",
            "details": [],
        });
        assert!(parse_cw721_tokens_response(&gateway_error).is_err());

        for malformed in [
            serde_json::json!({}),
            serde_json::json!({ "data": {} }),
            serde_json::json!({ "data": { "tokens": "not-an-array" } }),
            serde_json::json!({ "data": null }),
            serde_json::json!("a bare string"),
        ] {
            assert!(
                parse_cw721_tokens_response(&malformed).is_err(),
                "should reject {malformed}"
            );
        }
    }

    #[test]
    fn the_smart_query_url_is_path_safe_and_carries_the_bounded_query() {
        use base64::Engine as _;
        let url = cw721_tokens_query_url("https://lcd.example/", "cosmos1contract", "cosmos1owner");
        assert!(
            url.starts_with("https://lcd.example/cosmwasm/wasm/v1/contract/cosmos1contract/smart/")
        );
        // A trailing slash on the configured LCD must not double up.
        assert!(!url.contains("//cosmwasm"));

        let encoded = url.rsplit('/').next().unwrap();
        // Base64 emits `+`, `/` and `=`; `/` in particular would split the path.
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('='));

        let decoded_pct = percent_decode(encoded);
        let query = base64::engine::general_purpose::STANDARD
            .decode(decoded_pct)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&query).unwrap();
        assert_eq!(parsed["tokens"]["owner"], "cosmos1owner");
        assert_eq!(parsed["tokens"]["limit"], 1);
    }

    /// Minimal percent-decoder, test-only, so the URL assertion checks the real
    /// encoding rather than trusting the encoder.
    fn percent_decode(raw: &str) -> Vec<u8> {
        let bytes = raw.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                let hi = (bytes[i + 1] as char).to_digit(16).unwrap() as u8;
                let lo = (bytes[i + 2] as char).to_digit(16).unwrap() as u8;
                out.push(hi << 4 | lo);
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        out
    }

    #[test]
    fn denylist_matching_trims_and_ignores_case() {
        let c = cfg();
        assert!(c.is_denylisted("cosmos1treasury"));
        assert!(c.is_denylisted("  cosmos1treasury  "));
        assert!(c.is_denylisted("COSMOS1TREASURY"));
        assert!(!c.is_denylisted("cosmos1alice"));
        // A near-miss must not be swept up.
        assert!(!c.is_denylisted("cosmos1treasury2"));
    }

    #[test]
    fn denylist_parsing_trims_and_drops_empties() {
        assert_eq!(
            parse_denylist("cosmos1a, cosmos1b ,,  cosmos1c  "),
            vec!["cosmos1a", "cosmos1b", "cosmos1c"]
        );
        assert!(parse_denylist("  ,, ").is_empty());
    }

    #[test]
    fn error_labels_and_statuses_are_distinct_and_correctly_classed() {
        use ClaimError::*;
        let all = [
            AddressMalformed,
            ChallengeNonceInvalid,
            SignatureInvalid,
            PublicKeyAddressMismatch,
            AccountHoldsNoSlothToken,
            AccountNotEligible,
            InviteCredentialAlreadyBound,
            CelestineSlothClaimCapReached,
            ChainRpcUnavailable,
            ClaimBackendUnavailable,
        ];
        // The /sloths page switches on these to render distinct copy, so a
        // duplicate label would silently collapse two situations into one.
        let mut labels: Vec<&str> = all.iter().map(|e| e.public_label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "duplicate public label");

        // The full table from the design, asserted so a rename cannot silently
        // change the wire contract.
        for (error, label, status) in [
            (AddressMalformed, "AddressMalformed", 400),
            (ChallengeNonceInvalid, "ChallengeNonceInvalid", 400),
            (SignatureInvalid, "SignatureInvalid", 400),
            (PublicKeyAddressMismatch, "PublicKeyAddressMismatch", 400),
            (AccountHoldsNoSlothToken, "AccountHoldsNoSlothToken", 400),
            (AccountNotEligible, "AccountNotEligible", 400),
            (
                InviteCredentialAlreadyBound,
                "InviteCredentialAlreadyBound",
                409,
            ),
            (
                CelestineSlothClaimCapReached,
                "CelestineSlothClaimCapReached",
                409,
            ),
            (ChainRpcUnavailable, "ChainRpcUnavailable", 503),
            (ClaimBackendUnavailable, "ClaimBackendUnavailable", 503),
        ] {
            assert_eq!(error.public_label(), label);
            assert_eq!(error.status(), status, "{label}");
        }
    }
}

/// End-to-end tests over the claim router, driven with real secp256k1
/// signatures.
///
/// The grant store and the token check are both behind traits, so every refusal
/// path is exercised hermetically — no database and no network.
#[cfg(test)]
mod router_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use k256::ecdsa::signature::Signer as _;
    use k256::ecdsa::{Signature, SigningKey};
    use std::sync::Mutex;
    use tower::ServiceExt as _;

    fn signing_key(seed: u8) -> SigningKey {
        // Deterministic, non-zero scalars well inside the curve order.
        let mut bytes = [0u8; 32];
        bytes[31] = seed;
        bytes[0] = 1;
        SigningKey::from_slice(&bytes).expect("valid scalar")
    }

    fn pubkey_b64(sk: &SigningKey) -> String {
        use base64::Engine as _;
        let point = sk.verifying_key().to_encoded_point(true);
        base64::engine::general_purpose::STANDARD.encode(point.as_bytes())
    }

    fn address_of(sk: &SigningKey) -> String {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(pubkey_b64(sk))
            .unwrap();
        address_from_public_key(&raw, DEFAULT_BECH32_HRP).unwrap()
    }

    /// Sign the ADR-036 document for `address` over `message`, as a wallet would.
    fn sign_doc_with(sk: &SigningKey, address: &str, message: &str) -> String {
        use base64::Engine as _;
        let doc = adr036_sign_doc(address, message);
        let sig: Signature = sk.sign(doc.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
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

    struct FakeTokenChecker(Answer);

    #[async_trait::async_trait]
    impl CelestineSlothTokenChecker for FakeTokenChecker {
        async fn holds_sloth_token(
            &self,
            _cfg: &CelestineSlothConfig,
            _address: &str,
        ) -> Result<bool> {
            match self.0 {
                Answer::Yes => Ok(true),
                Answer::No => Ok(false),
                Answer::Fails => Err(anyhow!("lcd down")),
            }
        }
    }

    struct Harness {
        state: CelestineSlothClaimState,
        sink: Arc<FakeSink>,
    }

    fn harness_with(cap: u32, token: Answer, denylist: Vec<String>, sink: FakeSink) -> Harness {
        let sink = Arc::new(sink);
        let state = CelestineSlothClaimState {
            sink: sink.clone(),
            registry: None,
            sloths: Arc::new(CelestineSlothConfig {
                contract: "cosmos1contract".to_string(),
                lcd_url: "http://127.0.0.1:1/unused".to_string(),
                bech32_hrp: DEFAULT_BECH32_HRP.to_string(),
                denylist,
                cap,
                tenant_template_id: "tpl-sloths".to_string(),
                max_uses: 3,
                grant_ttl_days: 30,
            }),
            challenges: Arc::new(crate::account_passkey::CeremonyStore::new()),
            token_checker_override: Some(Arc::new(FakeTokenChecker(token))),
        };
        Harness { state, sink }
    }

    fn harness() -> Harness {
        harness_with(100, Answer::Yes, Vec::new(), FakeSink::default())
    }

    async fn call(
        state: &CelestineSlothClaimState,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let app = celestine_sloth_claim_router(state.clone());
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

    /// Issue a challenge and return `(challenge_id, message)`.
    async fn start_challenge(state: &CelestineSlothClaimState, address: &str) -> (String, String) {
        let (status, body) = call(
            state,
            "POST",
            "/v1/onboard/celestine-sloths/challenge",
            Some(serde_json::json!({ "address": address })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "challenge failed: {body}");
        let id = body["challengeId"].as_str().unwrap().to_string();
        let message = body["message"].as_str().unwrap().to_string();
        // The page signs `signDoc` verbatim; it must be exactly what the server
        // will rebuild from the message, or every wallet signature fails.
        assert_eq!(
            body["signDoc"].as_str().unwrap(),
            adr036_sign_doc(address, &message)
        );
        (id, message)
    }

    fn error_of(body: &serde_json::Value) -> &str {
        body["error"].as_str().unwrap_or("<no error field>")
    }

    #[tokio::test]
    async fn a_holder_claims_a_code_and_the_grant_has_the_specified_shape() {
        let h = harness();
        let sk = signing_key(7);
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;

        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
            })),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED, "claim failed: {body}");
        let code = body["inviteCode"].as_str().expect("invite code returned");
        assert!(!code.is_empty());
        assert_eq!(body["maxUses"].as_u64(), Some(3));

        // The grant is shaped as the design specifies, and — the point of the
        // whole privacy posture — the raw address is nowhere in it.
        let grants = h.sink.grants.lock().unwrap();
        assert_eq!(grants.len(), 1);
        let g = &grants[0];
        assert_eq!(g.policy_label, POLICY_LABEL);
        assert_eq!(g.issuance_source, ISSUANCE_SOURCE);
        assert_eq!(g.issued_by_label.as_deref(), Some(ISSUED_BY_LABEL));
        assert_eq!(g.max_uses, 3);
        assert_eq!(
            g.tenant_mode,
            crate::trace_invite_registry::InviteTenantMode::Derived
        );
        assert_eq!(g.tenant_template_id.as_deref(), Some("tpl-sloths"));
        assert!(g.fixed_tenant_id.is_none());
        assert_eq!(
            g.credential_binding_hash.as_deref(),
            Some(credential_binding_hash(&addr).as_str())
        );
        assert!(g.expires_at.is_some());
        // The stored hash must not contain the address.
        assert!(!format!("{g:?}").contains(&addr));
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
        let sk = signing_key(11);
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;
        let body = serde_json::json!({
            "challengeId": id,
            "address": addr,
            "publicKey": pubkey_b64(&sk),
            "signature": sign_doc_with(&sk, &addr, &message),
        });

        let (first, _) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(body.clone()),
        )
        .await;
        assert_eq!(first, StatusCode::CREATED);

        // Replaying the exact same signed request must not mint a second code.
        let (second, second_body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(body),
        )
        .await;
        assert_eq!(second, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&second_body), "ChallengeNonceInvalid");
        assert_eq!(h.sink.grants.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_challenge_issued_for_one_address_cannot_be_claimed_for_another() {
        let h = harness();
        let sk = signing_key(13);
        let other = address_of(&signing_key(14));
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;

        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": other,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
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
        let sk = signing_key(17);
        let addr = address_of(&sk);
        let message = claim_sign_message(&hex::encode([7u8; 32]));
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": "not-a-real-ceremony-id",
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "ChallengeNonceInvalid");
    }

    /// The case that matters most: a signature that verifies perfectly, against
    /// a key that controls a *different* address. Accepting it would let anyone
    /// with any Cosmos key claim on behalf of any holder.
    #[tokio::test]
    async fn a_valid_signature_from_a_key_for_another_address_is_refused() {
        let h = harness();
        let holder = signing_key(21);
        let attacker = signing_key(22);
        let addr = address_of(&holder);
        assert_ne!(addr, address_of(&attacker));

        let (id, message) = start_challenge(&h.state, &addr).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                // The attacker's key, signing the holder's document. The
                // signature itself is perfectly valid.
                "publicKey": pubkey_b64(&attacker),
                "signature": sign_doc_with(&attacker, &addr, &message),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "PublicKeyAddressMismatch");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn signatures_over_the_wrong_nonce_address_or_bytes_are_refused() {
        // Each case is a distinct way an attacker or a buggy client could
        // present a signature that verifies against *something* but not against
        // the challenge this server issued.
        type SigBuilder = Box<dyn Fn(&SigningKey, &str, &str) -> String>;

        let cases: Vec<(&str, SigBuilder)> = vec![
            (
                "wrong nonce",
                Box::new(|sk: &SigningKey, addr: &str, _m: &str| {
                    sign_doc_with(sk, addr, &claim_sign_message(&hex::encode([9u8; 32])))
                }),
            ),
            (
                // A document naming a different signer. The wallet would have
                // shown the user somebody else's address.
                "wrong signer in the doc",
                Box::new(|sk: &SigningKey, _addr: &str, m: &str| {
                    sign_doc_with(sk, "cosmos1elsewhere", m)
                }),
            ),
            (
                // The Legion ceremony's message. A signature harvested from
                // that flow must not be replayable into this one.
                "wrong message",
                Box::new(|sk: &SigningKey, addr: &str, _m: &str| {
                    sign_doc_with(sk, addr, crate::near_legion_claim::CLAIM_MESSAGE)
                }),
            ),
            (
                "garbage signature",
                Box::new(|_, _, _| "not-base64!".to_string()),
            ),
            (
                "well-formed base64 that is not a signature",
                Box::new(|_, _, _| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.encode([0u8; 64])
                }),
            ),
        ];

        for (label, make_sig) in cases {
            let h = harness();
            let sk = signing_key(31);
            let addr = address_of(&sk);
            let (id, message) = start_challenge(&h.state, &addr).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/celestine-sloths/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "address": addr,
                    "publicKey": pubkey_b64(&sk),
                    "signature": make_sig(&sk, &addr, &message),
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

    /// ECDSA signatures are malleable: `(r, s)` and `(r, n - s)` are both valid
    /// for the same message. Cosmos requires low-`s`, and accepting both would
    /// let one authorisation be presented as two distinct byte strings.
    ///
    /// This pins the observable behaviour, not a specific line: `k256`'s
    /// `verify` refuses high-`s` on its own, so the test still passes with the
    /// explicit guard in `verify_adr036` removed. That is the point — the
    /// property must hold however it is enforced.
    #[tokio::test]
    async fn a_high_s_signature_is_refused() {
        use base64::Engine as _;
        let h = harness();
        let sk = signing_key(41);
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;

        let doc = adr036_sign_doc(&addr, &message);
        let sig: Signature = sk.sign(doc.as_bytes());
        // k256 always emits low-`s`; flip it to the equivalent high-`s` form.
        let high_s = {
            let r = sig.r();
            let s = -*sig.s();
            Signature::from_scalars(*r.as_ref(), s).unwrap()
        };
        assert!(high_s.normalize_s().is_some(), "test built a low-s value");

        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": base64::engine::general_purpose::STANDARD.encode(high_s.to_bytes()),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "SignatureInvalid");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_address_holding_no_sloth_token_is_refused() {
        let h = harness_with(100, Answer::No, Vec::new(), FakeSink::default());
        let sk = signing_key(51);
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "AccountHoldsNoSlothToken");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_denylisted_address_cannot_claim() {
        let sk = signing_key(61);
        let addr = address_of(&sk);
        let h = harness_with(100, Answer::Yes, vec![addr.clone()], FakeSink::default());
        let (id, message) = start_challenge(&h.state, &addr).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_of(&body), "AccountNotEligible");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_address_cannot_hold_two_live_grants() {
        let h = harness();
        let sk = signing_key(71);
        let addr = address_of(&sk);

        for expected in [StatusCode::CREATED, StatusCode::CONFLICT] {
            let (id, message) = start_challenge(&h.state, &addr).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/celestine-sloths/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "address": addr,
                    "publicKey": pubkey_b64(&sk),
                    "signature": sign_doc_with(&sk, &addr, &message),
                })),
            )
            .await;
            assert_eq!(status, expected, "{body}");
            if expected == StatusCode::CONFLICT {
                assert_eq!(error_of(&body), "InviteCredentialAlreadyBound");
            }
        }
        // A fresh challenge does not get around it: the bind is on the address,
        // not the session.
        assert_eq!(h.sink.grants.lock().unwrap().len(), 1);
    }

    /// Cap boundary: one below the cap succeeds, at the cap refuses.
    #[tokio::test]
    async fn claims_stop_at_the_cap() {
        for (existing, cap, expected, label) in [
            (1u32, 2u32, StatusCode::CREATED, "one below the cap"),
            (1, 1, StatusCode::CONFLICT, "at the cap"),
        ] {
            let mut sink = FakeSink::default();
            for i in 0..existing {
                sink.grants
                    .get_mut()
                    .unwrap()
                    .push(crate::db::InviteGrantWrite {
                        invite_subject_hash: format!("sha256:{i}{}", "a".repeat(63)),
                        policy_label: POLICY_LABEL.to_string(),
                        tenant_mode: crate::trace_invite_registry::InviteTenantMode::Derived,
                        fixed_tenant_id: None,
                        tenant_template_id: Some("tpl-sloths".to_string()),
                        policy_version: "v1".to_string(),
                        allowed_consent_scopes: Vec::new(),
                        allowed_uses: Vec::new(),
                        max_uses: 3,
                        expires_at: None,
                        issuance_source: ISSUANCE_SOURCE.to_string(),
                        issued_by_label: None,
                        credential_binding_hash: Some(credential_binding_hash(&format!(
                            "cosmos1other{i}"
                        ))),
                        note_label: None,
                    });
            }
            let h = harness_with(cap, Answer::Yes, Vec::new(), sink);
            let sk = signing_key(81);
            let addr = address_of(&sk);
            let (id, message) = start_challenge(&h.state, &addr).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/celestine-sloths/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "address": addr,
                    "publicKey": pubkey_b64(&sk),
                    "signature": sign_doc_with(&sk, &addr, &message),
                })),
            )
            .await;
            assert_eq!(status, expected, "{label}: {body}");
            if expected == StatusCode::CONFLICT {
                assert_eq!(error_of(&body), "CelestineSlothClaimCapReached", "{label}");
            }
            assert_eq!(
                h.sink.grants.lock().unwrap().len() as u32,
                if expected == StatusCode::CREATED {
                    existing + 1
                } else {
                    existing
                },
                "{label}"
            );
        }
    }

    /// An LCD failure must be a retryable 503, never a 400 that tells a real
    /// holder they hold nothing.
    #[tokio::test]
    async fn an_lcd_failure_surfaces_as_503_with_no_partial_write() {
        let h = harness_with(100, Answer::Fails, Vec::new(), FakeSink::default());
        let sk = signing_key(91);
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;
        let (status, body) = call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_of(&body), "ChainRpcUnavailable");
        assert!(h.sink.grants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_backend_failure_never_returns_a_code() {
        for sink in [
            FakeSink {
                fail_insert: true,
                ..Default::default()
            },
            FakeSink {
                fail_count: true,
                ..Default::default()
            },
        ] {
            let h = harness_with(100, Answer::Yes, Vec::new(), sink);
            let sk = signing_key(101);
            let addr = address_of(&sk);
            let (id, message) = start_challenge(&h.state, &addr).await;
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/celestine-sloths/claim",
                Some(serde_json::json!({
                    "challengeId": id,
                    "address": addr,
                    "publicKey": pubkey_b64(&sk),
                    "signature": sign_doc_with(&sk, &addr, &message),
                })),
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(error_of(&body), "ClaimBackendUnavailable");
            assert!(body.get("inviteCode").is_none());
        }
    }

    #[tokio::test]
    async fn malformed_addresses_are_refused_before_a_challenge_is_minted() {
        let h = harness();
        let good = address_of(&signing_key(111));
        for bad in [
            "",
            "cosmos1",
            "not-an-address",
            // A valid Stargaze-prefixed address on a Hub-configured pool.
            &address_from_public_key(
                &hex::decode("0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798")
                    .unwrap(),
                "stars",
            )
            .unwrap(),
            // Correct shape, corrupted checksum.
            &format!("{}x", &good[..good.len() - 1]),
        ] {
            let (status, body) = call(
                &h.state,
                "POST",
                "/v1/onboard/celestine-sloths/challenge",
                Some(serde_json::json!({ "address": bad })),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}");
            assert_eq!(error_of(&body), "AddressMalformed", "{bad:?}");
        }
    }

    #[tokio::test]
    async fn status_reports_remaining_capacity() {
        let h = harness_with(5, Answer::Yes, Vec::new(), FakeSink::default());
        let (status, body) =
            call(&h.state, "GET", "/v1/onboard/celestine-sloths/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["claimed"].as_u64(), Some(0));
        assert_eq!(body["cap"].as_u64(), Some(5));
        assert_eq!(body["remaining"].as_u64(), Some(5));
        assert_eq!(body["maxUses"].as_u64(), Some(3));

        // After a claim the counter moves, so the page shows real scarcity.
        let sk = signing_key(121);
        let addr = address_of(&sk);
        let (id, message) = start_challenge(&h.state, &addr).await;
        call(
            &h.state,
            "POST",
            "/v1/onboard/celestine-sloths/claim",
            Some(serde_json::json!({
                "challengeId": id,
                "address": addr,
                "publicKey": pubkey_b64(&sk),
                "signature": sign_doc_with(&sk, &addr, &message),
            })),
        )
        .await;
        let (_, body) = call(&h.state, "GET", "/v1/onboard/celestine-sloths/status", None).await;
        assert_eq!(body["claimed"].as_u64(), Some(1));
        assert_eq!(body["remaining"].as_u64(), Some(4));
    }

    #[tokio::test]
    async fn status_fails_closed_when_the_backend_is_down() {
        let h = harness_with(
            5,
            Answer::Yes,
            Vec::new(),
            FakeSink {
                fail_count: true,
                ..Default::default()
            },
        );
        let (status, body) =
            call(&h.state, "GET", "/v1/onboard/celestine-sloths/status", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_of(&body), "ClaimBackendUnavailable");
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
                    tenant_template_id: Some("tpl-sloths".to_string()),
                    policy_version: "v1".to_string(),
                    allowed_consent_scopes: Vec::new(),
                    allowed_uses: Vec::new(),
                    max_uses: 3,
                    expires_at: None,
                    issuance_source: ISSUANCE_SOURCE.to_string(),
                    issued_by_label: None,
                    credential_binding_hash: Some(credential_binding_hash(&format!("cosmos1a{i}"))),
                    note_label: None,
                });
        }
        let h = harness_with(1, Answer::Yes, Vec::new(), sink);
        let (status, body) =
            call(&h.state, "GET", "/v1/onboard/celestine-sloths/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["remaining"].as_u64(), Some(0));
    }
}
