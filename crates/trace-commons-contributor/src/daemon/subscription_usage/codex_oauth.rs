//! Read-only Codex OAuth usage, CodexBar's preferred Codex source.
//!
//! Follows CodexBar at commit 5f93dc3068b3bee3f8753f3772614cc2949c335d:
//! `docs/codex.md` ("OAuth API (preferred for the app)"),
//! `Sources/CodexBarCore/Providers/Codex/CodexOAuth/CodexOAuthCredentials.swift`
//! (auth.json parsing, `needsRefresh`), `.../CodexOAuth/CodexOAuthUsageFetcher.swift`
//! (request and response shape), `.../Codex/CodexProviderDescriptor.swift`
//! (`CodexOAuthFetchStrategy`: rereads and fallback rules) and
//! `.../Codex/CodexAdditionalRateLimitMapper.swift` (extra limits).
//! After a successful usage read it also asks, best effort, for the banked
//! reset inventory the way CodexBar's app does (see `reset_credit_count`).
//!
//! auth.json is only ever read. Codex owns that file and its refresh token, so
//! a token due for renewal is never redeemed here; the caller hands that case
//! to the native app-server, which may renew it and therefore still waits for
//! Codex to be closed. The bearer token stays in this module's memory and is
//! sent only in an Authorization header to the validated ChatGPT endpoint.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::OwnedSemaphorePermit;
use trace_commons_contributor_platform::external_claude_credentials::SecretBytes;

use super::providers::{
    CodexStore, HTTP_TIMEOUT, bounded_json, codex_completion_revision, codex_file_identity,
    failure, fingerprint, native_read, percent, request_error, unix_reset, usage_client,
};
use super::{Identity, LoginSpec, ProbeFailure, Reading, Window};

const MAX_BYTES: usize = 1024 * 1024;
/// CodexBar `fetchRateLimitResetCredits(timeout: TimeInterval = 4)`; it is
/// further capped by what is left of the usage request's budget.
const RESET_CREDITS_TIMEOUT: Duration = Duration::from_secs(4);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// CodexBar rereads a missing, unreadable, partially written or stale file
/// twice, 50 ms apart, in case Codex is replacing it.
const REREADS: usize = 2;
const REREAD_DELAY: Duration = Duration::from_millis(50);
/// `needsRefresh`: a recognized token expiry within five minutes is stale.
const EXPIRY_MARGIN_SECONDS: i64 = 5 * 60;
/// `needsRefresh`: otherwise the saved `last_refresh`, older than eight days.
const RENEWAL_AGE_DAYS: i64 = 8;
/// Internal: the provider rejected the token, so the native client may renew it.
const UNAUTHORIZED: &str = "unauthorized";
/// Internal: the file holds no refreshable OAuth sign-in for this path.
const OAUTH_UNAVAILABLE: &str = "oauth_unavailable";

/// Where the usage request goes. Production always uses the strict client and
/// the endpoint derived from the validated `chatgpt_base_url`.
pub(super) struct Target {
    client: reqwest::Client,
    /// Test-only endpoint override; `None` in production.
    base: Option<reqwest::Url>,
    reset_credits_timeout: Duration,
}

impl Target {
    pub(super) fn production() -> Result<Self, &'static str> {
        Ok(Self {
            client: usage_client()?,
            base: None,
            reset_credits_timeout: RESET_CREDITS_TIMEOUT,
        })
    }
}

struct Credential {
    token: SecretBytes,
    account_id: Option<String>,
    stale: bool,
}

struct Loaded {
    credential: Credential,
    /// The file identity the post-read witness must still observe.
    file_identity: (String, Option<String>),
}

// Deserialize only the fields CodexBar reads; unknown members are skipped.
// (The shared account witness, `codex_file_identity`, still parses the whole
// file as a Value in memory, as the native path always has.)
#[derive(Deserialize)]
struct AuthDocument<'a> {
    #[serde(rename = "OPENAI_API_KEY", borrow, default)]
    api_key: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    auth_mode: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    tokens: Option<Tokens<'a>>,
    #[serde(default)]
    last_refresh: Option<Value>,
}

#[derive(Deserialize)]
struct Tokens<'a> {
    #[serde(alias = "accessToken", borrow, default)]
    access_token: Option<Cow<'a, str>>,
    #[serde(alias = "refreshToken", borrow, default)]
    refresh_token: Option<Cow<'a, str>>,
    #[serde(alias = "idToken", borrow, default)]
    id_token: Option<Cow<'a, str>>,
    #[serde(alias = "accountId", borrow, default)]
    account_id: Option<Cow<'a, str>>,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn header_safe(value: &str) -> bool {
    value.len() <= 16 * 1024 && !value.chars().any(char::is_control)
}

fn jwt_payload(token: &str) -> Option<Value> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let bytes = engine.decode(parts[1]).ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
}

/// `accountIDFromJWT`: id token first, then access token.
pub(super) fn account_from_jwt(tokens: [Option<&str>; 2]) -> Option<String> {
    tokens.into_iter().flatten().find_map(|token| {
        let payload = jwt_payload(token)?;
        let text =
            |value: Option<&Value>| non_empty(value.and_then(Value::as_str)).map(str::to_owned);
        text(payload.get("chatgpt_account_id"))
            .or_else(|| text(payload.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")))
            .or_else(|| {
                payload
                    .get("organizations")
                    .and_then(Value::as_array)?
                    .iter()
                    .find_map(|organization| text(organization.get("id")))
            })
    })
}

/// `expirationFromJWT`: an integer `exp` in a well-formed three-part token.
fn expiry_from_jwt(token: &str) -> Option<DateTime<Utc>> {
    if token.split('.').any(str::is_empty) {
        return None;
    }
    jwt_payload(token)?
        .get("exp")
        .and_then(Value::as_i64)
        .and_then(|exp| DateTime::from_timestamp(exp, 0))
}

fn parse_credential(bytes: &[u8], now: DateTime<Utc>) -> Result<Credential, &'static str> {
    let document: AuthDocument = serde_json::from_slice(bytes).map_err(|_| "auth_missing")?;
    if non_empty(document.api_key.as_deref()).is_some()
        || matches!(document.auth_mode.as_deref(), Some("apikey" | "api_key"))
    {
        return Err("auth_missing");
    }
    let tokens = document.tokens.ok_or("auth_missing")?;
    let access = tokens
        .access_token
        .as_deref()
        .filter(|token| !token.is_empty() && header_safe(token))
        .ok_or("auth_missing")?;
    // CodexBar's OAuth credentials need both tokens; without a refresh token
    // only the native client can decide what this sign-in is.
    if tokens.refresh_token.as_deref().is_none_or(str::is_empty) {
        return Err(OAUTH_UNAVAILABLE);
    }
    let account_id = non_empty(tokens.account_id.as_deref())
        .map(str::to_owned)
        .or_else(|| account_from_jwt([tokens.id_token.as_deref(), Some(access)]))
        .filter(|account| account.len() <= 1024 && header_safe(account));
    let stale = match expiry_from_jwt(access) {
        Some(expiry) => (expiry - now).num_seconds() <= EXPIRY_MARGIN_SECONDS,
        None => document
            .last_refresh
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .is_none_or(|at| {
                now - at.with_timezone(&Utc) > chrono::Duration::days(RENEWAL_AGE_DAYS)
            }),
    };
    Ok(Credential {
        token: SecretBytes::new(access.as_bytes().to_vec()),
        account_id,
        stale,
    })
}

fn read_credential(path: &Path) -> Result<Loaded, &'static str> {
    let file =
        trace_commons_contributor_platform::local_files::open_import_file(path).map_err(|_| {
            if path.exists() {
                "unavailable"
            } else {
                "auth_missing"
            }
        })?;
    let mut bytes = Vec::new();
    let result = file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes);
    let bytes = SecretBytes::new(bytes);
    result.map_err(|_| "unavailable")?;
    if bytes.len() > MAX_BYTES {
        return Err("invalid_response");
    }
    let file_identity = codex_file_identity(&bytes)?;
    Ok(Loaded {
        credential: parse_credential(&bytes, Utc::now())?,
        file_identity,
    })
}

/// `None` means the native client should handle this sign-in.
async fn load(
    root: &Path,
    lease: &Arc<OwnedSemaphorePermit>,
) -> Result<Option<Loaded>, ProbeFailure> {
    let path = root.join("auth.json");
    let mut rereads = REREADS;
    loop {
        let attempt_path = path.clone();
        let attempt = tokio::time::timeout(
            READ_TIMEOUT,
            native_read(Arc::clone(lease), move || read_credential(&attempt_path)),
        )
        .await
        .map_err(|_| failure("timeout", None))?
        .map_err(|reason| failure(reason, None))?;
        let settled = attempt
            .as_ref()
            .is_ok_and(|loaded| !loaded.credential.stale);
        if settled || rereads == 0 {
            return match attempt {
                Ok(loaded) => Ok(Some(loaded)),
                Err(OAUTH_UNAVAILABLE) => Ok(None),
                Err(reason) => Err(failure(reason, None)),
            };
        }
        rereads -= 1;
        tokio::time::sleep(REREAD_DELAY).await;
    }
}

/// `normalizeChatGPTBaseURL`.
fn normalized_base(base: &str) -> String {
    let mut base = base.trim();
    if base.is_empty() {
        base = "https://chatgpt.com/backend-api/";
    }
    let mut url = base.trim_end_matches('/').to_owned();
    if (url.starts_with("https://chatgpt.com") || url.starts_with("https://chat.openai.com"))
        && !url.contains("/backend-api")
    {
        url.push_str("/backend-api");
    }
    url
}

/// `resolveUsageURL`.
fn usage_url(base: &str) -> Result<reqwest::Url, &'static str> {
    let mut url = normalized_base(base);
    url.push_str(if url.contains("/backend-api") {
        "/wham/usage"
    } else {
        "/api/codex/usage"
    });
    reqwest::Url::parse(&url).map_err(|_| "base_url_refused")
}

/// `resolveRateLimitResetCreditsURL`: the same base for every path shape.
fn reset_credits_url(base: &str) -> Result<reqwest::Url, &'static str> {
    reqwest::Url::parse(&(normalized_base(base) + "/wham/rate-limit-reset-credits"))
        .map_err(|_| "base_url_refused")
}

/// The count is authoritative, as on the native path (`rateLimitResetCredits.
/// availableCount`); per-credit details are not needed for the display.
fn parse_reset_credits(value: &Value) -> Option<u64> {
    value.get("available_count").and_then(Value::as_u64)
}

/// Best-effort banked-reset inventory, following CodexBar at 5f93dc3:
/// `CodexOAuthUsageFetcher.fetchRateLimitResetCredits` (CodexOAuthUsageFetcher.swift
/// lines 524-562: GET, Bearer, Accept, `OpenAI-Beta: codex-1`,
/// `originator: Codex Desktop`, `ChatGPT-Account-ID`, 4 s timeout) called from
/// `CodexOAuthFetchStrategy.fetchResetCreditsIfRequested`
/// (CodexProviderDescriptor.swift lines 800-819), which turns every failure
/// into "no inventory". Any failure here leaves banked resets unknown; it
/// never fails the usage reading or falls back. Only GET is ever sent; credits
/// are never redeemed or modified. Our User-Agent stays our own.
async fn reset_credit_count(
    client: &reqwest::Client,
    url: &reqwest::Url,
    token: &str,
    account: Option<&str>,
    budget: Duration,
) -> Option<u64> {
    if budget.is_zero() {
        return None;
    }
    tokio::time::timeout(budget, async {
        let response = request(client, url, token, account)
            .header("OpenAI-Beta", "codex-1")
            .header("originator", "Codex Desktop")
            .timeout(budget)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        parse_reset_credits(&bounded_json(response).await.ok()?)
    })
    .await
    .ok()
    .flatten()
}

fn request(
    client: &reqwest::Client,
    url: &reqwest::Url,
    token: &str,
    account: Option<&str>,
) -> reqwest::RequestBuilder {
    let request = client
        .get(url.clone())
        .bearer_auth(token)
        .header("Accept", "application/json");
    match account {
        Some(account) => request.header("ChatGPT-Account-Id", account),
        None => request,
    }
}

/// `CodexAuthenticatedHTTPTransport.perform`: only 401 is an authentication
/// failure; every other non-2xx status is a server error.
fn status(status: u16) -> Result<(), &'static str> {
    match status {
        200..=299 => Ok(()),
        401 => Err(UNAUTHORIZED),
        429 => Err("rate_limited"),
        _ => Err("unavailable"),
    }
}

fn window(value: Option<&Value>, slot: &str) -> Option<Window> {
    let window = value.filter(|value| value.is_object())?;
    let duration_mins = window
        .get("limit_window_seconds")
        .and_then(Value::as_u64)
        .map(|seconds| seconds / 60)
        .filter(|minutes| *minutes > 0 && *minutes <= i64::MAX as u64);
    let lane = match duration_mins {
        Some(300) => "session",
        Some(10080) => "weekly",
        _ => slot,
    };
    Some(Window {
        lane: lane.into(),
        label: None,
        duration_mins,
        used_permille: percent(window.get("used_percent")),
        resets_at: unix_reset(window.get("reset_at")),
    })
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    for c in value.to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

/// `CodexAdditionalRateLimitMapper`: named model-specific limits.
fn additional_windows(value: Option<&Value>) -> Vec<Window> {
    let mut used = BTreeSet::new();
    let mut windows = Vec::new();
    for entry in value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(16)
    {
        let text = |key: &str| non_empty(entry.get(key).and_then(Value::as_str));
        let (name, feature) = (text("limit_name"), text("metered_feature"));
        let limits = entry.get("rate_limit");
        let primary = limits.and_then(|limits| limits.get("primary_window"));
        let secondary = limits.and_then(|limits| limits.get("secondary_window"));
        let spark = [name, feature]
            .into_iter()
            .flatten()
            .any(|value| value.to_lowercase().contains("spark"));
        let mut named = |id: String, title: &str, value: Option<&Value>| {
            if let Some(mut window) = window(value, "model")
                && used.insert(id)
            {
                window.lane = "model".into();
                window.label = Some(title.to_owned());
                windows.push(window);
            }
        };
        if spark {
            for (value, fallback_weekly) in [(primary, false), (secondary, true)] {
                let minutes = value
                    .and_then(|v| v.get("limit_window_seconds"))
                    .and_then(Value::as_u64)
                    .map(|seconds| seconds / 60)
                    .unwrap_or(0);
                let weekly = if minutes > 0 && minutes <= 6 * 60 {
                    false
                } else if minutes >= 6 * 24 * 60 {
                    true
                } else {
                    fallback_weekly
                };
                let (id, title) = if weekly {
                    ("codex-spark-weekly", "Codex Spark Weekly")
                } else {
                    ("codex-spark", "Codex Spark 5-hour")
                };
                named(id.into(), title, value);
            }
        } else if let Some(source) = feature.or(name) {
            let id = slug(source);
            let chosen = primary.filter(|v| v.is_object()).or(secondary);
            if !id.is_empty() {
                named(
                    format!("codex-{id}"),
                    name.or(feature).unwrap_or(source),
                    chosen,
                );
            }
        }
    }
    windows
}

fn parse_usage(value: &Value, expected_account: Option<&str>) -> Result<Reading, &'static str> {
    if !value.is_object() {
        return Err("invalid_response");
    }
    let reported = non_empty(
        value
            .get("account_id")
            .or_else(|| value.get("accountId"))
            .and_then(Value::as_str),
    );
    if expected_account
        .zip(reported)
        .is_some_and(|(expected, reported)| expected != reported)
    {
        return Err("auth_changed");
    }
    let limits = value.get("rate_limit");
    let mut windows: Vec<_> = [
        ("primary_window", "primary"),
        ("secondary_window", "secondary"),
    ]
    .into_iter()
    .filter_map(|(key, slot)| window(limits.and_then(|limits| limits.get(key)), slot))
    .collect();
    windows.sort_by_key(|window| if window.lane == "session" { 0 } else { 1 });
    windows.extend(additional_windows(value.get("additional_rate_limits")));
    Ok(Reading {
        windows,
        // wham/usage carries no banked-reset inventory; `collect` asks for it
        // separately, best effort.
        banked_resets: None,
        identity: Identity {
            account: expected_account
                .or(reported)
                .map(|account| fingerprint(account.as_bytes())),
            email: None,
            organization: None,
        },
        native_revision: None,
        source: super::ReadingSource::Api,
    })
}

/// `Ok(None)` asks the caller to use the native app-server instead: the saved
/// sign-in needs renewal, the provider rejected it, or it has no refresh token.
pub(super) async fn collect(
    login: &LoginSpec,
    lease: &Arc<OwnedSemaphorePermit>,
    base_url: &str,
    target: &Target,
) -> Result<Option<Reading>, ProbeFailure> {
    let base = target.base.as_ref().map_or(base_url, reqwest::Url::as_str);
    let url = usage_url(base).map_err(|reason| failure(reason, None))?;
    let credits_url = reset_credits_url(base).map_err(|reason| failure(reason, None))?;
    let Some(loaded) = load(&login.root, lease).await? else {
        return Ok(None);
    };
    if loaded.credential.stale {
        return Ok(None);
    }
    let account = loaded.credential.account_id.as_deref();
    let identity = Identity {
        account: account.map(|account| fingerprint(account.as_bytes())),
        email: None,
        organization: None,
    };
    // One budget for both requests: the inventory gets only what the usage
    // request left of the usual HTTP timeout, so Refresh never waits longer.
    let deadline = tokio::time::Instant::now() + HTTP_TIMEOUT;
    let token =
        std::str::from_utf8(&loaded.credential.token).map_err(|_| failure("auth_missing", None))?;
    let result = async {
        let response = request(&target.client, &url, token, account)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(request_error)?;
        status(response.status().as_u16())?;
        parse_usage(&bounded_json(response).await?, account)
    }
    .await;
    let mut reading = match result {
        // CodexBar: an unauthorized OAuth read may fall back to the CLI.
        // Network, server and decode failures keep their own error.
        Err(UNAUTHORIZED) => return Ok(None),
        Err("auth_changed") => return Err(failure("auth_changed", None)),
        Err(reason) => return Err(failure(reason, Some(identity))),
        Ok(reading) => reading,
    };
    // Without account evidence from the file, its JWTs or the response this
    // reading cannot be attributed. The native app-server identifies the
    // account through `account/read`, so hand off rather than fail.
    if reading.identity.account.is_none() {
        return Ok(None);
    }
    let budget = target
        .reset_credits_timeout
        .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
    reading.banked_resets =
        reset_credit_count(&target.client, &credits_url, token, account, budget).await;
    // Same witness as the native path: storage selection and account are
    // unchanged, and the returned revision is what this reading belongs to.
    let revision = codex_completion_revision(
        &login.root,
        lease,
        &Some(loaded.file_identity),
        &(CodexStore::File, base_url.to_owned()),
    )
    .await?;
    reading.native_revision = Some(revision);
    Ok(Some(reading))
}

#[cfg(test)]
mod tests;
