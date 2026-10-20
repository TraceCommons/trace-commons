//! Anthropic backends: the Claude subscription and the metered API key.
//!
//! Both speak the same wire, which is what makes them rungs 0–2 of the same
//! ladder: falling from the subscription to an API key costs a cold cache and
//! nothing else — no translation, no dropped reasoning (`docs/DESIGN.md` §3).

use std::sync::{Arc, Mutex};

use chrono::Utc;
use futures_util::StreamExt;
use ironwire_catalog::AnthropicCatalog;
use ironwire_core::capability::Capabilities;
use ironwire_core::protocol::{BackendId, BackendKind, ClientIdentity, ModelTier, Protocol, Wires};
use ironwire_core::quota::{Headroom, QuotaSnapshot};
use ironwire_creds::claude::{ANTHROPIC_HOST, ClaudeLogin};
use ironwire_creds::{Bearer, CredentialError, SubscriptionLogin, SubscriptionProduct};
use secrecy::{ExposeSecret, SecretString};

use crate::backend::{
    Backend, BackendStatus, UpstreamError, UpstreamRequest, UpstreamResponse, unusable_login,
};
use crate::headers::forward_response_header;
use crate::observe::{Observation, anthropic_rate_limit, retry_after};

/// Default base URL for the Anthropic API.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// How a given Anthropic backend authenticates.
#[derive(Clone)]
pub enum AnthropicAuth {
    /// One Claude Code login's subscription token, re-read from that login's
    /// store on every request so a background refresh by Claude Code is
    /// picked up.
    Subscription(ClaudeLogin),
    /// A metered API key.
    ApiKey(SecretString),
}

/// Capability profile shared by every current Anthropic model.
#[must_use]
pub fn anthropic_capabilities() -> Capabilities {
    Capabilities {
        wires: Wires::only(Protocol::AnthropicMessages),
        tools: true,
        parallel_tool_calls: true,
        images: true,
        reasoning: true,
        prompt_cache: true,
        structured_output: true,
        context_tokens: 200_000,
        // This backend never sees a Responses body.
        unsupported_responses_tools: Vec::new(),
    }
}

/// Models this backend offers, best first.
#[must_use]
pub fn anthropic_models() -> Vec<(String, ModelTier)> {
    vec![
        ("claude-opus-4-6".to_string(), ModelTier::Frontier),
        ("claude-sonnet-4-6".to_string(), ModelTier::Balanced),
        ("claude-haiku-4-5".to_string(), ModelTier::Fast),
    ]
}

/// An Anthropic-family backend.
pub struct AnthropicBackend {
    id: BackendId,
    name: String,
    kind: BackendKind,
    auth: AnthropicAuth,
    base_url: String,
    client: reqwest::Client,
    capabilities: Capabilities,
    /// Compiled-in catalogue, used until a probe learns better.
    models: Vec<(String, ModelTier)>,
    /// What `GET /v1/models` actually reported for this account.
    ///
    /// The probe already fetches this list to check the credential; not reading
    /// it left `ironwire status` naming models from whenever the binary was
    /// built, months after the account gained newer ones.
    discovered: Arc<Mutex<Option<crate::Catalogue>>>,
    quota: Arc<Mutex<QuotaSnapshot>>,
    /// Protocol constants. These are the values most likely to change under us,
    /// so they come from the signed catalog channel rather than a release
    /// (`docs/UPDATES.md`). The *host* deliberately does not.
    catalog: AnthropicCatalog,
}

impl AnthropicBackend {
    /// Replace the protocol constants with a newer signed set.
    #[must_use]
    pub fn with_catalog(mut self, catalog: AnthropicCatalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// Build the subscription backend for the default Claude Code login, as
    /// `claude-sub`.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    pub fn subscription(base_url: Option<String>, timeout_secs: u64) -> reqwest::Result<Self> {
        Self::subscription_for(
            BackendId::from(SubscriptionProduct::Claude.default_backend_id()),
            "Claude subscription".to_string(),
            ClaudeLogin::default_login(),
            base_url,
            timeout_secs,
        )
    }

    /// Build the subscription backend for one Claude Code login.
    ///
    /// `id` is the backend's identity everywhere — consent, quota, ledger — so
    /// it must stay attached to this login across restarts: `claude-sub` for
    /// the default login (relocated or not), the config entry's id otherwise.
    /// `name` is what `ironwire status` shows; for a login other than the
    /// default that is normally its [`SubscriptionLogin::label`], so two
    /// logins never share a heading.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    pub fn subscription_for(
        id: BackendId,
        name: String,
        login: ClaudeLogin,
        base_url: Option<String>,
        timeout_secs: u64,
    ) -> reqwest::Result<Self> {
        Ok(Self {
            id,
            name,
            kind: BackendKind::Subscription,
            auth: AnthropicAuth::Subscription(login),
            base_url: normalize_base(base_url),
            client: crate::first_party_client::build(timeout_secs)?,
            capabilities: anthropic_capabilities(),
            models: anthropic_models(),
            discovered: Arc::new(Mutex::new(None)),
            quota: Arc::new(Mutex::new(QuotaSnapshot::default())),
            catalog: AnthropicCatalog::default(),
        })
    }

    /// Build an API-key-backed Anthropic backend.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    pub fn api_key(
        key: SecretString,
        base_url: Option<String>,
        timeout_secs: u64,
    ) -> reqwest::Result<Self> {
        Ok(Self {
            id: BackendId::from("anthropic-key"),
            name: "Anthropic API".to_string(),
            kind: BackendKind::ApiKey,
            auth: AnthropicAuth::ApiKey(key),
            base_url: normalize_base(base_url),
            client: crate::first_party_client::build(timeout_secs)?,
            capabilities: anthropic_capabilities(),
            models: anthropic_models(),
            discovered: Arc::new(Mutex::new(None)),
            quota: Arc::new(Mutex::new(QuotaSnapshot::default())),
            catalog: AnthropicCatalog::default(),
        })
    }

    /// Resolve the credential for this request.
    ///
    /// The subscription token is re-read every time rather than cached: Claude
    /// Code refreshes it in the background, and a cached copy would go stale in
    /// a way that looks like a rate limit.
    fn credential(auth: &AnthropicAuth) -> Result<Bearer, CredentialError> {
        match auth {
            AnthropicAuth::Subscription(login) => {
                let creds = login.read()?;
                // A token past its stated expiry is refused here rather than
                // sent to earn a 401. IronWire never refreshes it (see `send`),
                // so a login whose Claude Code has not run for a while stays
                // stale until it does — the usual state of a second login, and
                // one command fixes it.
                if creds.is_expired(Utc::now()) {
                    return Err(CredentialError::Expired {
                        product: "Claude Code",
                        command: login.login_command(),
                    });
                }
                Ok(creds.bearer())
            }
            AnthropicAuth::ApiKey(key) => Ok(Bearer {
                token: key.clone(),
                issuer_host: ANTHROPIC_HOST,
            }),
        }
    }

    /// The credential, or the reason it cannot be used ending in what to run.
    async fn credential_or_detail(&self) -> Result<Bearer, String> {
        let auth = self.auth.clone();
        crate::credential_read::read(move || {
            Self::credential(&auth).map_err(|err| match &auth {
                AnthropicAuth::Subscription(login) => unusable_login(&err, &login.login_command()),
                AnthropicAuth::ApiKey(_) => err.to_string(),
            })
        })
        .await
    }

    /// Assert the credential is going to the host that issued it
    /// (`docs/TRUST.md` I2). This is a guard against our own bugs, not against
    /// the user.
    fn check_host(&self, bearer: &Bearer) -> Result<(), UpstreamError> {
        crate::credential_host::check(&self.base_url, bearer.issuer_host)
    }
}

#[async_trait::async_trait]
impl Backend for AnthropicBackend {
    fn id(&self) -> &BackendId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn models(&self) -> Vec<(String, ModelTier)> {
        match self.discovered.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
        .unwrap_or_else(|| self.models.clone())
    }

    fn catalogue_from_provider(&self) -> bool {
        match self.discovered.lock() {
            Ok(guard) => guard.is_some(),
            Err(poisoned) => poisoned.into_inner().is_some(),
        }
    }

    fn required_client_identity(&self) -> Option<ClientIdentity> {
        // TRUST.md §3: the subscription serves the client it belongs to — and
        // only that one. The metered key serves anyone.
        matches!(self.auth, AnthropicAuth::Subscription(_)).then_some(ClientIdentity::ClaudeCode)
    }

    fn login(&self) -> Option<SubscriptionLogin> {
        match &self.auth {
            // The default backend id is the default login even when relocated
            // to a directory: everything recorded under `claude-sub` is about
            // it.
            AnthropicAuth::Subscription(login) => Some(
                login
                    .describe(self.id.as_str() == SubscriptionProduct::Claude.default_backend_id()),
            ),
            AnthropicAuth::ApiKey(_) => None,
        }
    }

    async fn status(&self) -> BackendStatus {
        let (authenticated, detail) = match self.credential_or_detail().await {
            Ok(_) => (true, None),
            Err(detail) => (false, Some(detail)),
        };
        BackendStatus {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            authenticated,
            account_scope: None,
            detail,
            quota: self.quota(),
            models: self.models(),
        }
    }

    async fn send(&self, request: UpstreamRequest) -> Result<UpstreamResponse, UpstreamError> {
        let bearer =
            self.credential_or_detail()
                .await
                .map_err(|detail| UpstreamError::NeedsAuth {
                    backend: self.id.clone(),
                    detail,
                })?;
        self.check_host(&bearer)?;

        match (self.send_once(request.clone(), &bearer).await, &self.auth) {
            // A 401 on a subscription usually means Claude Code rotated the
            // token between our read and our send. Re-read and retry once.
            //
            // We deliberately do *not* drive a refresh ourselves. Doing so
            // means writing to another product's credential store, and racing
            // its own rotation could invalidate the user's Claude Code session
            // — breaking the thing they actually care about in order to fix
            // something they did not ask us to fix. Re-reading is enough,
            // because Claude Code refreshes in the background anyway.
            (Err(UpstreamError::NeedsAuth { .. }), AnthropicAuth::Subscription(login)) => {
                let refreshed = self.credential_or_detail().await.map_err(|detail| {
                    UpstreamError::NeedsAuth {
                        backend: self.id.clone(),
                        detail,
                    }
                })?;
                if refreshed.token.expose_secret() == bearer.token.expose_secret() {
                    // Same token, same answer. Retrying would just burn a
                    // request and delay telling the user to re-authenticate.
                    return Err(UpstreamError::NeedsAuth {
                        backend: self.id.clone(),
                        detail: rejected_token_detail(login),
                    });
                }
                tracing::debug!(
                    backend = %self.id,
                    "credential rotated under us; retrying once with the fresh token"
                );
                self.send_once(request, &refreshed).await
            }
            (other, _) => other,
        }
    }

    fn record(&self, observation: &Observation) {
        let now = Utc::now();
        let mut quota = match self.quota.lock() {
            Ok(q) => q,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(reading) = observation.primary.clone() {
            quota.primary = reading.into_headroom(now);
        }
        if let Some(secs) = observation.retry_after_secs {
            // A provider-stated wait always wins over a percentage: it is the
            // more specific fact, and it is the one that is actionable.
            quota.primary = Headroom::Exhausted {
                until: now + chrono::Duration::seconds(i64::try_from(secs).unwrap_or(i64::MAX)),
            };
        }
    }

    fn restore_quota(&self, snapshot: QuotaSnapshot) {
        match self.quota.lock() {
            Ok(mut guard) => *guard = snapshot,
            Err(poisoned) => *poisoned.into_inner() = snapshot,
        }
    }

    fn quota(&self) -> QuotaSnapshot {
        match self.quota.lock() {
            Ok(q) => q.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    async fn probe(&self) -> Result<(), UpstreamError> {
        let bearer =
            self.credential_or_detail()
                .await
                .map_err(|detail| UpstreamError::NeedsAuth {
                    backend: self.id.clone(),
                    detail,
                })?;
        self.check_host(&bearer)?;

        // `GET /v1/models` validates the credential without an inference call.
        //
        // For the subscription backend this is the *only* honest probe: a real
        // message would have to carry Claude Code's identity to be accepted,
        // and synthesising that identity is exactly what `docs/TRUST.md` §3
        // forbids. Checking auth without pretending to be another product is
        // both cheaper and the only version we are willing to ship.
        let mut builder = self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .header("anthropic-version", &self.catalog.api_version)
            .timeout(std::time::Duration::from_secs(15));
        builder = match &self.auth {
            AnthropicAuth::Subscription(_) => builder
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", bearer.token.expose_secret()),
                )
                .header("anthropic-beta", &self.catalog.oauth_beta),
            AnthropicAuth::ApiKey(_) => builder.header("x-api-key", bearer.token.expose_secret()),
        };

        let response = builder.send().await.map_err(|e| UpstreamError::Transport {
            backend: self.id.clone(),
            detail: e.to_string(),
        })?;
        let status = response.status();
        if status.is_success() {
            // The list is already in our hands from the auth check; reading it
            // is what keeps `status` from naming a catalogue frozen at build
            // time. Ordered as the provider ordered it — newest first.
            if let Ok(body) = response.bytes().await
                && let Some(models) = crate::openai_responses::parse_model_list(&body)
                && !models.is_empty()
            {
                tracing::info!(
                    backend = %self.id,
                    count = models.len(),
                    "learned the model catalogue from the provider"
                );
                match self.discovered.lock() {
                    Ok(mut guard) => *guard = Some(models),
                    Err(poisoned) => *poisoned.into_inner() = Some(models),
                }
            }
            return Ok(());
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = response.bytes().await.unwrap_or_default();
            return Err(UpstreamError::NeedsAuth {
                backend: self.id.clone(),
                detail: String::from_utf8_lossy(&body).chars().take(200).collect(),
            });
        }
        Err(UpstreamError::Upstream {
            backend: self.id.clone(),
            status: http::StatusCode::from_u16(status.as_u16())
                .unwrap_or(http::StatusCode::BAD_GATEWAY),
            body: response.bytes().await.unwrap_or_default(),
        })
    }
}

impl AnthropicBackend {
    /// One attempt with a specific credential. No retry, no failover — both are
    /// the router's decision, because only it knows whether a byte has already
    /// reached the client (`docs/PROTOCOL.md` §5).
    async fn send_once(
        &self,
        request: UpstreamRequest,
        bearer: &Bearer,
    ) -> Result<UpstreamResponse, UpstreamError> {
        let url = format!("{}{}", self.base_url, request.path);
        let mut builder = self.client.post(&url);

        // Forward everything the client sent that survived the header filter.
        // Track whether the client supplied the protocol headers so we only
        // add defaults when it did not — overriding a client's own
        // `anthropic-version` would be a mutation we did not enumerate.
        let mut saw_version = false;
        let mut saw_beta: Option<String> = None;
        for (name, value) in &request.headers {
            if name == "anthropic-version" {
                saw_version = true;
            }
            if name == "anthropic-beta" {
                saw_beta = Some(value.clone());
                continue; // re-added below, possibly extended
            }
            builder = builder.header(name, value);
        }

        match &self.auth {
            AnthropicAuth::Subscription(_) => {
                builder = builder.header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", bearer.token.expose_secret()),
                );
                // OAuth bearer auth is gated behind this beta flag; without it
                // the API answers 401. Append rather than replace so the
                // client's own beta flags survive.
                let beta = match saw_beta {
                    Some(existing) if existing.contains(&self.catalog.oauth_beta) => existing,
                    Some(existing) => format!("{existing},{}", self.catalog.oauth_beta),
                    None => self.catalog.oauth_beta.to_string(),
                };
                builder = builder.header("anthropic-beta", beta);
                if !saw_version {
                    builder = builder.header("anthropic-version", &self.catalog.api_version);
                }
            }
            AnthropicAuth::ApiKey(_) => {
                builder = builder.header("x-api-key", bearer.token.expose_secret());
                if let Some(beta) = saw_beta {
                    builder = builder.header("anthropic-beta", beta);
                }
                if !saw_version {
                    builder = builder.header("anthropic-version", &self.catalog.api_version);
                }
            }
        }

        let response =
            builder
                .body(request.body)
                .send()
                .await
                .map_err(|e| UpstreamError::Transport {
                    backend: self.id.clone(),
                    detail: e.to_string(),
                })?;

        let status = response.status();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter(|(name, _)| forward_response_header(name.as_str()))
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_string(), v.to_string()))
            })
            .collect();

        // Fold rate-limit state in before deciding what to do with the status:
        // a 429 tells us at least as much as a 200.
        let now = Utc::now();
        let observation = Observation {
            primary: anthropic_rate_limit(&headers),
            retry_after_secs: retry_after(&headers, now),
            ..Observation::default()
        };
        if !observation.is_empty() {
            self.record(&observation);
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(UpstreamError::RateLimited {
                backend: self.id.clone(),
                retry_after_secs: observation.retry_after_secs,
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = response.bytes().await.unwrap_or_default();
            return Err(UpstreamError::NeedsAuth {
                backend: self.id.clone(),
                detail: String::from_utf8_lossy(&body).chars().take(400).collect(),
            });
        }
        // Any refusal, not only a server-side one. A 4xx used to fall through
        // to the streaming path below, where a JSON error body reached the SSE
        // guard, produced no frames, and was reported to the client as "the
        // upstream closed without producing a response" — with the provider's
        // actual complaint discarded unread. See `tests/codex_real_turn.rs`.
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            return Err(UpstreamError::Upstream {
                backend: self.id.clone(),
                status: http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(http::StatusCode::BAD_GATEWAY),
                body,
            });
        }

        // Client errors (4xx other than the two above) are the client's own
        // request being wrong. Pass them through as a normal response so the
        // agent sees exactly what the provider said.
        let backend = self.id.clone();
        let body = response
            .bytes_stream()
            .map(move |chunk| {
                chunk.map_err(|e| UpstreamError::Transport {
                    backend: backend.clone(),
                    detail: e.to_string(),
                })
            })
            .boxed();

        Ok(UpstreamResponse {
            status: http::StatusCode::from_u16(status.as_u16())
                .unwrap_or(http::StatusCode::BAD_GATEWAY),
            headers,
            body,
        })
    }
}

/// What to say when the provider rejected a token that re-reading did not
/// change: which login it was, and the command that logs that one in.
fn rejected_token_detail(login: &ClaudeLogin) -> String {
    let whose = match login.describe(false).location {
        None => String::new(),
        Some(location) => format!(" for {location}"),
    };
    format!(
        "the stored Claude Code token{whose} was rejected; re-authenticate by running `{}` \
         and logging in",
        login.login_command()
    )
}

fn normalize_base(base: Option<String>) -> String {
    base.unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subscription_backend(base: &str) -> AnthropicBackend {
        AnthropicBackend::subscription(Some(base.to_string()), 60).expect("client builds")
    }

    #[test]
    fn only_the_subscription_demands_a_matching_client_identity() {
        let sub = subscription_backend(DEFAULT_BASE_URL);
        assert_eq!(
            sub.required_client_identity(),
            Some(ClientIdentity::ClaudeCode)
        );

        let key = AnthropicBackend::api_key(SecretString::from("sk-ant-x"), None, 60)
            .expect("client builds");
        assert_eq!(key.required_client_identity(), None);
    }

    #[test]
    fn a_credential_is_refused_for_a_host_that_did_not_issue_it() {
        // TRUST.md I2 — a misconfigured base_url must not exfiltrate a token.
        let backend = subscription_backend("https://evil.example");
        let bearer = Bearer {
            token: SecretString::from("sk-ant-oat01-x"),
            issuer_host: ANTHROPIC_HOST,
        };
        let err = backend
            .check_host(&bearer)
            .expect_err("must refuse a foreign host");
        assert!(matches!(err, UpstreamError::CredentialHostMismatch { .. }));
    }

    #[test]
    fn the_issuers_own_host_is_accepted() {
        let backend = subscription_backend(DEFAULT_BASE_URL);
        let bearer = Bearer {
            token: SecretString::from("sk-ant-oat01-x"),
            issuer_host: ANTHROPIC_HOST,
        };
        assert!(backend.check_host(&bearer).is_ok());
    }

    #[test]
    fn loopback_is_allowed_so_the_conformance_harness_can_run() {
        let backend = subscription_backend("http://127.0.0.1:9999");
        let bearer = Bearer {
            token: SecretString::from("sk-ant-oat01-x"),
            issuer_host: ANTHROPIC_HOST,
        };
        assert!(backend.check_host(&bearer).is_ok());
    }

    /// Whether this backend would attach its Claude token to `base`.
    fn sends_the_token_to(base: &str) -> bool {
        let bearer = Bearer {
            token: SecretString::from("sk-ant-oat01-x"),
            issuer_host: ANTHROPIC_HOST,
        };
        subscription_backend(base).check_host(&bearer).is_ok()
    }

    /// TRUST.md I2. Everything before the `@` is userinfo; the request goes to
    /// what follows it.
    #[test]
    fn userinfo_cannot_dress_a_foreign_host_as_loopback_or_the_issuer() {
        for base in [
            "https://127.0.0.1@evil.example",
            "https://localhost:443@evil.example",
            "https://api.anthropic.com@evil.example",
            "https://user:pw@api.anthropic.com",
        ] {
            assert!(!sends_the_token_to(base), "{base} must be refused");
        }
    }

    #[test]
    fn a_host_that_merely_begins_like_loopback_or_the_issuer_is_neither() {
        for base in [
            "https://localhost.evil.example",
            "https://127.0.0.1.evil.example",
            "https://api.anthropic.com.evil.example",
            "https://api.anthropic.com.",
        ] {
            assert!(!sends_the_token_to(base), "{base} must be refused");
        }
    }

    #[test]
    fn the_real_provider_is_never_sent_the_token_in_cleartext() {
        assert!(!sends_the_token_to("http://api.anthropic.com"));
    }

    #[test]
    fn the_issuers_host_is_recognised_however_it_is_spelled() {
        for base in [
            "https://API.Anthropic.com",
            "https://api.anthropic.com:443/",
            "https://api.anthropic.com/",
        ] {
            assert!(sends_the_token_to(base), "{base} is the issuer");
        }
    }

    #[test]
    fn every_spelling_of_this_machine_is_loopback() {
        for base in [
            "http://127.0.0.1:1234",
            "http://[::1]:1234",
            "http://localhost:9",
            "http://127.0.0.2",
        ] {
            assert!(sends_the_token_to(base), "{base} is loopback");
        }
    }

    #[test]
    fn base_urls_normalise_to_one_form() {
        assert_eq!(normalize_base(None), DEFAULT_BASE_URL);
        assert_eq!(
            normalize_base(Some("https://example.test/".to_string())),
            "https://example.test"
        );
    }

    #[test]
    fn a_provider_stated_wait_overrides_a_percentage() {
        let backend = subscription_backend(DEFAULT_BASE_URL);
        backend.record(&Observation {
            primary: Some(crate::observe::RateLimitReading {
                used_pct: 50.0,
                resets_at: None,
            }),
            retry_after_secs: Some(120),
            ..Observation::default()
        });
        let quota = backend.quota();
        assert!(
            matches!(quota.primary, Headroom::Exhausted { .. }),
            "a concrete retry-after must win over a percentage"
        );
        assert!(!quota.is_available(Utc::now()));
    }

    #[test]
    fn quota_starts_unknown_rather_than_optimistic() {
        let backend = subscription_backend(DEFAULT_BASE_URL);
        assert_eq!(backend.quota().primary, Headroom::Unknown);
        assert!(backend.quota().is_available(Utc::now()));
    }
}
