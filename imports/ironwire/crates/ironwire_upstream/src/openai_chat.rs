//! OpenAI Chat Completions backends — NEAR AI, and any OpenAI-compatible
//! endpoint the user points us at.
//!
//! This is the target of the translated lane. It speaks a different family from
//! the Anthropic façade, so requests reach it only after
//! `ironwire_core::capability::eligible` has cleared a cross-family switch.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use futures_util::StreamExt;
use ironwire_core::capability::Capabilities;
use ironwire_core::protocol::{BackendId, BackendKind, ModelTier, Protocol, Wires};
use ironwire_core::quota::{Headroom, QuotaSnapshot};
use secrecy::{ExposeSecret, SecretString};

use crate::backend::{Backend, BackendStatus, UpstreamError, UpstreamRequest, UpstreamResponse};
use crate::headers::forward_response_header;
use crate::observe::{Observation, retry_after};

/// NEAR AI's OpenAI-compatible inference endpoint.
pub const NEARAI_DEFAULT_BASE_URL: &str = "https://cloud-api.near.ai/v1";

/// Ask NEAR AI's gateway to refuse an aliased model rather than substitute one.
///
/// **Off by default, and deliberately so.** This does not make the gateway
/// serve the model that was named; it makes the gateway answer 400 when the
/// name turns out to be an alias. Most requests through this proxy are somebody
/// getting work done, not somebody collecting evidence, and turning their
/// working inference call into a hard failure is not a trade IronWire gets to
/// make on their behalf. The default is to *observe* the substitution instead
/// ([`crate::observe::MODEL_ALIAS_RESOLVED_HEADER`]), which costs the caller
/// nothing and still records the fact.
///
/// What it is for: an operator who would rather have no answer than an
/// unverifiable one. An aliased response is served by a different model than
/// the request named, and cloud-api rewrites the body to add a `warning` field
/// -- stated on `inject_warning_field` in `crates/api/src/routes/common.rs`:
/// "rewriting the body means it no longer byte-matches what the backend TD
/// signed, so response-hash verification will not pass for aliased responses
/// ... strict clients can avoid it entirely with `x-no-aliasing`". For a caller
/// binding payloads to a model TD's signing key that is worth a 400.
///
/// Set `refuse_model_aliases = true` on the backend to turn it on.
pub const NO_ALIASING_HEADER: &str = "x-no-aliasing";

/// Capability profile for a modern OSS model served over Chat Completions.
///
/// `reasoning` and `prompt_cache` are false, and that is now only a *quality*
/// signal — neither blocks a route (`ironwire_core::capability`). `images` is
/// false because a text-only model genuinely cannot see them, which does.
#[must_use]
pub fn chat_capabilities(context_tokens: u32) -> Capabilities {
    Capabilities {
        wires: Wires::only(Protocol::OpenAiChat),
        tools: true,
        parallel_tool_calls: true,
        images: false,
        reasoning: false,
        prompt_cache: false,
        structured_output: false,
        context_tokens,
        unsupported_responses_tools: Vec::new(),
    }
}

/// Capability profile for NEAR AI.
///
/// Chat Completions and Responses at the same base URL, which is what the
/// provider actually serves. Chat stays *primary* because it is the only wire
/// `ironwire_translate` can produce today, so it is the one a Claude Code
/// fallback translates into; Responses is there so Codex arrives on the native
/// lane instead of being refused for a translation it never needed.
#[must_use]
pub fn nearai_capabilities() -> Capabilities {
    Capabilities {
        wires: Wires::new(Protocol::OpenAiChat, &[Protocol::OpenAiResponses]),
        // Verified against the live endpoint: a `namespace` entry returns
        // `unknown variant `namespace`, expected one of `function`,
        // `web_search`, `web_context_search`, `file_search`,
        // `code_interpreter`, `computer`, `mcp``. Every other type Codex
        // sends is on that list, so only this one is stripped.
        unsupported_responses_tools: vec!["namespace".to_string()],
        ..chat_capabilities(128_000)
    }
}

/// An OpenAI-compatible Chat Completions backend.
pub struct ChatCompletionsBackend {
    id: BackendId,
    name: String,
    kind: BackendKind,
    /// `None` for a local server that takes no auth, which is most of them.
    /// Absent means no `Authorization` header at all — not an empty one.
    api_key: Option<SecretString>,
    issuer_host: String,
    base_url: String,
    client: reqwest::Client,
    capabilities: Capabilities,
    /// Headers this backend adds to every request it sends, whatever the
    /// client sent. Provider-specific opt-ins, not credentials — a credential
    /// goes through [`Self::authorize`]. Empty for a generic OpenAI-compatible
    /// endpoint, which is somebody else's server and gets nothing invented.
    extra_headers: Vec<(String, String)>,
    /// Configured catalogue, used until a probe learns better.
    models: Vec<(String, ModelTier)>,
    /// What the endpoint itself reported.
    ///
    /// An OpenAI-compatible endpoint is somebody else's product with somebody
    /// else's release schedule; a list configured here is a guess that goes
    /// stale silently. Once we have asked, we use the answer.
    discovered: Arc<Mutex<Option<crate::Catalogue>>>,
    quota: Arc<Mutex<QuotaSnapshot>>,
}

impl ChatCompletionsBackend {
    /// Build a NEAR AI backend.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    /// The key is optional so the backend can be registered without one. A
    /// credits backend with no key reports `authenticated: false` and shows up
    /// in `status` as something to connect, which is the contract
    /// `build_registry` states: a backend nobody has a credential for is still
    /// a backend the user should be able to see IronWire knows about.
    pub fn nearai(
        api_key: Option<SecretString>,
        base_url: Option<String>,
        models: Vec<(String, ModelTier)>,
        timeout_secs: u64,
    ) -> reqwest::Result<Self> {
        Ok(Self::new(
            BackendId::from("nearai"),
            "NEAR AI",
            BackendKind::Credits,
            api_key,
            base_url.unwrap_or_else(|| NEARAI_DEFAULT_BASE_URL.to_string()),
            models,
            timeout_secs,
        )?
        .with_capabilities(nearai_capabilities()))
    }

    /// Build an arbitrary OpenAI-compatible backend.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    /// Build a backend for a model running on this machine or LAN.
    ///
    /// `BackendKind::Local` rather than `Credits`: free at the margin, not
    /// metered, and no consent gate. `api_key` is optional because most local
    /// servers take no auth — and `None` means no header, not an empty one.
    ///
    /// The base URL must include the OpenAI-compatible prefix, usually `/v1`.
    /// Ollama's native `/api/*` is a different protocol and is not supported.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    pub fn local(
        id: BackendId,
        name: &str,
        base_url: String,
        api_key: Option<SecretString>,
        models: Vec<(String, ModelTier)>,
        timeout_secs: u64,
    ) -> reqwest::Result<Self> {
        Self::new(
            id,
            name,
            BackendKind::Local,
            api_key,
            base_url,
            models,
            timeout_secs,
        )
    }

    /// Build an arbitrary OpenAI-compatible backend.
    ///
    /// # Errors
    ///
    /// Propagates a reqwest client build failure.
    pub fn new(
        id: BackendId,
        name: &str,
        kind: BackendKind,
        api_key: Option<SecretString>,
        base_url: String,
        models: Vec<(String, ModelTier)>,
        timeout_secs: u64,
    ) -> reqwest::Result<Self> {
        let base_url = base_url.trim_end_matches('/').to_string();
        // A credential is only ever attached to the host it was configured for
        // (`docs/TRUST.md` I2). For a user-supplied endpoint that host *is* the
        // configured base URL, so derive it rather than hardcoding one.
        let issuer_host = host_of(&base_url);
        Ok(Self {
            id,
            name: name.to_string(),
            kind,
            api_key,
            issuer_host,
            base_url,
            client: reqwest::Client::builder()
                .read_timeout(std::time::Duration::from_secs(timeout_secs))
                .connect_timeout(std::time::Duration::from_secs(15))
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .build()?,
            capabilities: chat_capabilities(128_000),
            extra_headers: Vec::new(),
            models,
            discovered: Arc::new(Mutex::new(None)),
            quota: Arc::new(Mutex::new(QuotaSnapshot::default())),
        })
    }

    /// Override the capability profile (context window, modalities).
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Set the headers this backend adds to every request.
    #[must_use]
    fn with_extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    /// Refuse an aliased model rather than accept a substituted one.
    ///
    /// See [`NO_ALIASING_HEADER`] for why this is off unless an operator asks.
    /// It covers both wires at once because `nearai_capabilities` speaks Chat
    /// Completions and Responses at the same base URL and both arrive at the
    /// same `send`; setting it per-wire would leave whichever agent landed on
    /// the other lane unprotected, which is a mistake this repo has already
    /// made once (`ironwire_core::policy`, the Codex fallback).
    #[must_use]
    pub fn refusing_model_aliases(self, refuse: bool) -> Self {
        if !refuse {
            return self;
        }
        self.with_extra_headers(vec![(NO_ALIASING_HEADER.to_string(), "true".to_string())])
    }

    /// Attach the credential, if there is one.
    ///
    /// A local server usually takes none, and sending `Authorization: Bearer `
    /// with nothing after it is not the same as sending no header — some
    /// servers reject it outright.
    fn account_scope(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        let key = self.api_key.as_ref()?.expose_secret();
        if key.is_empty() {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(b"ironwire:compatible-api-credential:v1\0");
        hash.update(self.base_url.as_bytes());
        hash.update([0]);
        hash.update(key.as_bytes());
        Some(format!("{:x}", hash.finalize()))
    }

    fn authorize(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        debug_assert_eq!(self.issuer_host, host_of(&self.base_url));
        match self.api_key.as_ref() {
            Some(key) => builder.bearer_auth(key.expose_secret()),
            None => builder,
        }
    }
}

fn host_of(base_url: &str) -> String {
    base_url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(base_url)
        .to_string()
}

#[async_trait::async_trait]
impl Backend for ChatCompletionsBackend {
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

    async fn status(&self) -> BackendStatus {
        // Local capacity needs no credential, so "no key" is its ordinary
        // state rather than a fault to report.
        let authenticated = self.kind == BackendKind::Local
            || self
                .api_key
                .as_ref()
                .is_some_and(|key| !key.expose_secret().is_empty());
        BackendStatus {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            authenticated,
            account_scope: self.account_scope(),
            detail: (!authenticated).then(|| "no API key configured".to_string()),
            quota: self.quota(),
            models: self.models(),
        }
    }

    async fn send(&self, request: UpstreamRequest) -> Result<UpstreamResponse, UpstreamError> {
        if request
            .expected_account_scope
            .as_ref()
            .is_some_and(|expected| self.account_scope().as_ref() != Some(expected))
        {
            return Err(UpstreamError::NeedsAuth {
                backend: self.id.clone(),
                detail:
                    "the credential changed; start a new session or restore the original API key"
                        .into(),
            });
        }

        let url = crate::endpoint_url(&self.base_url, &request.path);
        let mut builder = self
            .authorize(self.client.post(&url))
            .header("content-type", "application/json");
        for (name, value) in &request.headers {
            // The translated lane rebuilds the body, so inbound provider
            // headers describe a protocol this backend does not speak.
            if name.starts_with("anthropic-") || name == "content-type" {
                continue;
            }
            // Set below from this backend's own configuration. Forwarding the
            // client's copy would either duplicate the header or let a client
            // turn the guarantee off, and neither is its call to make.
            if self
                .extra_headers
                .iter()
                .any(|(ours, _)| ours.eq_ignore_ascii_case(name))
            {
                continue;
            }
            builder = builder.header(name, value);
        }
        for (name, value) in &self.extra_headers {
            builder = builder.header(name, value);
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

        let observation = Observation {
            retry_after_secs: retry_after(&headers, Utc::now()),
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
            quota.primary = Headroom::Exhausted {
                // Clamped at the parse site too (`observe::retry_after`); this
                // is belt and braces, because `chrono` panics rather than
                // saturating and the input is upstream-controlled.
                until: now
                    + chrono::Duration::seconds(
                        i64::try_from(secs.min(crate::observe::MAX_RETRY_AFTER_SECS))
                            .unwrap_or(86_400),
                    ),
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

    /// NEAR AI is the only Chat Completions provider that signs its answers.
    /// A generic OpenAI-compatible endpoint is somebody else's server, and
    /// asking it for a receipt would send the user's key to a path it never
    /// agreed to serve.
    fn offers_receipts(&self) -> bool {
        self.kind == BackendKind::Credits
    }

    async fn fetch_receipt(&self, upstream_id: &str, model: &str) -> crate::receipt::ReceiptFetch {
        use crate::receipt::{FETCH_TIMEOUT, ReceiptFetch, usable_id};
        if !self.offers_receipts() {
            return ReceiptFetch::NotOffered;
        }
        if !usable_id(upstream_id) {
            return ReceiptFetch::NotFound;
        }
        let request = self
            .authorize(
                self.client
                    .get(format!("{}/signature/{upstream_id}", self.base_url)),
            )
            .query(&[("model", model), ("signing_algo", "ed25519")])
            .timeout(FETCH_TIMEOUT);
        match request.send().await {
            Ok(response) => crate::receipt::read(response).await,
            Err(_) => ReceiptFetch::Unavailable,
        }
    }

    async fn probe(&self) -> Result<(), UpstreamError> {
        let response = self
            .authorize(self.client.get(format!("{}/models", self.base_url)))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| UpstreamError::Transport {
                backend: self.id.clone(),
                detail: e.to_string(),
            })?;
        let status = response.status();
        if status.is_success() {
            // The probe already has the answer in its hand; parsing it is what
            // turns a configured guess into what the endpoint actually serves.
            if let Ok(body) = response.bytes().await
                && let Some(models) = crate::openai_responses::parse_model_list(&body)
                && !models.is_empty()
            {
                // A discovered local model is `Fast`, whatever its name
                // suggests. `from_model_hint` resolves an unrecognised slug to
                // `Frontier` — right for a hosted catalogue, catastrophic here,
                // because local capacity also sorts cheapest and a
                // `qwen3-coder:30b` reading as frontier-tier would take work
                // meant for Opus. The user opts a local model up the ladder by
                // declaring a tier in config; discovery never does.
                let models: crate::Catalogue = if self.kind == BackendKind::Local {
                    models
                        .into_iter()
                        .map(|(name, _)| (name, ModelTier::Fast))
                        .collect()
                } else {
                    models
                };
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
            return Err(UpstreamError::NeedsAuth {
                backend: self.id.clone(),
                detail: "the configured API key was rejected".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(base: &str) -> ChatCompletionsBackend {
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some(base.to_string()),
            vec![("near-x".to_string(), ModelTier::Balanced)],
            60,
        )
        .expect("client builds")
    }

    #[tokio::test]
    async fn a_native_responses_replay_cannot_use_another_compatible_api_credential() {
        let original = backend("http://127.0.0.1:1/v1");
        let scope = original
            .status()
            .await
            .account_scope
            .expect("configured key identity");
        let replaced = ChatCompletionsBackend::nearai(
            Some(SecretString::from("another-inert-key")),
            Some("http://127.0.0.1:1/v1".into()),
            Vec::new(),
            60,
        )
        .expect("backend");
        let error = replaced
            .send(UpstreamRequest {
                path: "/v1/responses".into(),
                body: bytes::Bytes::from_static(b"{}"),
                headers: Vec::new(),
                stream: false,
                expected_account_scope: Some(scope),
            })
            .await
            .expect_err("changed credential refused before network");
        assert!(matches!(error, UpstreamError::NeedsAuth { .. }));
    }

    #[test]
    fn the_credential_is_bound_to_the_configured_endpoint() {
        // TRUST.md I2 for a user-supplied host: derived, not hardcoded.
        let b = backend("https://api.near.ai/v1");
        assert_eq!(b.issuer_host, "api.near.ai");
        assert!(b.api_key.is_some());
        let b = backend("http://127.0.0.1:9000/v1");
        assert_eq!(b.issuer_host, "127.0.0.1:9000");
        assert!(b.api_key.is_some());
    }

    #[test]
    fn near_ai_is_credit_capacity_and_needs_no_client_identity() {
        let b = backend(NEARAI_DEFAULT_BASE_URL);
        assert_eq!(b.kind(), BackendKind::Credits);
        assert_eq!(b.required_client_identity(), None);
        assert!(b.capabilities().wires.speaks(Protocol::OpenAiChat));
    }

    /// The wire NEAR AI serves that this backend used to hide.
    ///
    /// Codex speaks Responses, NEAR AI answers `/v1/responses` at the same base
    /// URL, and claiming Chat Completions alone meant the router refused the
    /// route for a translation that was never needed — the native lane was
    /// there the whole time.
    #[test]
    fn near_ai_serves_responses_as_well_as_chat_completions() {
        let b = backend(NEARAI_DEFAULT_BASE_URL);
        assert!(b.capabilities().wires.speaks(Protocol::OpenAiResponses));
        assert!(b.capabilities().wires.speaks(Protocol::OpenAiChat));
        // Chat stays the wire a translated route targets: it is the only one
        // `ironwire_translate` can produce.
        assert_eq!(b.capabilities().wires.primary(), Protocol::OpenAiChat);
    }

    /// A local server is not NEAR AI. Most of them serve Chat Completions and
    /// nothing else, and advertising a wire that 404s would turn a clean
    /// refusal into a failed request.
    #[test]
    fn a_local_backend_claims_only_chat_completions() {
        let b = ChatCompletionsBackend::local(
            BackendId::from("ollama"),
            "Ollama",
            "http://127.0.0.1:11434/v1".to_string(),
            None,
            Vec::new(),
            60,
        )
        .expect("client builds");
        assert!(!b.capabilities().wires.speaks(Protocol::OpenAiResponses));
    }

    #[test]
    fn a_chat_backend_is_a_different_family_from_the_anthropic_facade() {
        // Which is what makes it the translated lane's target.
        let b = backend(NEARAI_DEFAULT_BASE_URL);
        assert_ne!(
            b.capabilities().wires.primary().family(),
            Protocol::AnthropicMessages.family()
        );
    }

    #[test]
    fn trailing_slashes_do_not_double_up_in_the_url() {
        let b = backend("https://api.near.ai/v1/");
        assert_eq!(b.base_url, "https://api.near.ai/v1");
    }

    #[test]
    fn a_provider_stated_wait_marks_the_backend_exhausted() {
        let b = backend(NEARAI_DEFAULT_BASE_URL);
        b.record(&Observation {
            retry_after_secs: Some(60),
            ..Observation::default()
        });
        assert!(!b.quota().is_available(Utc::now()));
    }
}

#[cfg(test)]
mod local_tests {
    use super::*;

    fn local(models: Vec<(String, ModelTier)>) -> ChatCompletionsBackend {
        ChatCompletionsBackend::local(
            BackendId::from("ollama"),
            "ollama",
            "http://127.0.0.1:11434/v1".to_string(),
            None,
            models,
            60,
        )
        .expect("client builds")
    }

    /// Most local servers take no auth, and `Authorization: Bearer ` with
    /// nothing after it is not the same as sending no header — some reject it.
    #[test]
    fn a_local_backend_without_a_key_carries_no_credential() {
        let backend = local(Vec::new());
        assert!(backend.api_key.is_none());
        let request = backend
            .authorize(backend.client.get(&backend.base_url))
            .build()
            .expect("request builds");
        assert!(!request.headers().contains_key(http::header::AUTHORIZATION));
    }

    #[test]
    fn a_local_backend_with_a_key_is_bound_to_its_own_host() {
        let backend = ChatCompletionsBackend::local(
            BackendId::from("ollama"),
            "ollama",
            "http://127.0.0.1:11434/v1".to_string(),
            Some(SecretString::from("sk-local")),
            Vec::new(),
            60,
        )
        .expect("client builds");
        assert_eq!(backend.issuer_host, "127.0.0.1:11434");
        assert_eq!(
            backend
                .api_key
                .as_ref()
                .expect("a key was configured")
                .expose_secret(),
            "sk-local"
        );
        for _ in 0..1024 {
            let request = backend
                .authorize(backend.client.get(&backend.base_url))
                .build()
                .expect("request builds");
            assert_eq!(request.url().host_str(), Some("127.0.0.1"));
            assert_eq!(request.url().port(), Some(11434));
            assert_eq!(
                request.headers()[http::header::AUTHORIZATION],
                "Bearer sk-local"
            );
        }
    }

    /// Free at the margin, not metered, and no consent gate — the three things
    /// that make `Local` a different capacity class rather than a label.
    #[test]
    fn local_capacity_is_free_and_ungated() {
        let backend = local(Vec::new());
        assert_eq!(backend.kind(), BackendKind::Local);
        assert!(!backend.kind().is_metered());
        assert!(!backend.kind().requires_consent());
        assert_eq!(backend.required_client_identity(), None);
    }

    /// A local server that is not running is a backend `status` can explain,
    /// not a startup failure.
    #[tokio::test]
    async fn an_unreachable_local_server_still_reports_itself() {
        let status = local(Vec::new()).status().await;
        assert!(
            status.authenticated,
            "a local backend needs no credential, so it is never 'not logged in'"
        );
        assert_eq!(status.kind, BackendKind::Local);
    }
}
