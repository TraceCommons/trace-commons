//! The `Backend` trait and its request/response shapes.
//!
//! Note what these types are *not*: there is no `CompletionRequest`, no
//! `ChatMessage`, no `ToolCall`. `ironclaw_llm` has all of those and they are
//! right for an agent that owns its prompts — but they are a
//! chat-completions-shaped common denominator, and re-encoding through one is
//! exactly the fidelity loss the native lane exists to avoid
//! (`docs/DESIGN.md` §7). A backend here moves bytes.

use bytes::Bytes;
use futures_util::stream::BoxStream;
use ironwire_core::capability::Capabilities;
use ironwire_core::protocol::{BackendId, BackendKind, ClientIdentity, ModelTier};
use ironwire_core::quota::QuotaSnapshot;
use ironwire_creds::{CredentialError, SubscriptionLogin};

use crate::observe::Observation;

/// A request as it leaves IronWire.
#[derive(Debug, Clone)]
pub struct UpstreamRequest {
    /// Path suffix beneath the backend's base URL, e.g. `/v1/messages`.
    pub path: String,
    /// Body bytes, already carrying any policy-driven `model` edit.
    pub body: Bytes,
    /// Inbound headers worth forwarding, already stripped of hop-by-hop and
    /// auth headers by [`crate::headers`].
    pub headers: Vec<(String, String)>,
    /// Whether the client asked for SSE.
    pub stream: bool,
    /// Account or API credential scope selected by routing, if known. Native
    /// Responses backends recheck it against the credential actually sent.
    pub expected_account_scope: Option<String>,
}

/// A response on its way back to the client.
pub struct UpstreamResponse {
    /// Upstream status.
    pub status: http::StatusCode,
    /// Response headers, stripped of hop-by-hop headers.
    pub headers: Vec<(String, String)>,
    /// Body, streamed. Forwarded verbatim.
    pub body: BoxStream<'static, Result<Bytes, UpstreamError>>,
}

impl std::fmt::Debug for UpstreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamResponse")
            .field("status", &self.status)
            .field("headers", &self.headers.len())
            .finish_non_exhaustive()
    }
}

/// Failure talking to a backend.
///
/// The distinction that matters for routing is [`UpstreamError::is_retryable`]:
/// a retryable error before the first byte descends the ladder, and after the
/// first byte nothing is retryable at all (`docs/PROTOCOL.md` §5).
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// The backend has no usable credential.
    #[error("{backend} is not authenticated: {detail}")]
    NeedsAuth {
        /// Which backend.
        backend: BackendId,
        /// What went wrong.
        detail: String,
    },

    /// Rate limited. Carries the provider's own `retry-after` where given —
    /// never a value of our own invention.
    #[error("{backend} is rate limited")]
    RateLimited {
        /// Which backend.
        backend: BackendId,
        /// Seconds the provider asked us to wait.
        retry_after_secs: Option<u64>,
    },

    /// Transport failure: connect, TLS, timeout, reset.
    #[error("{backend} transport failure: {detail}")]
    Transport {
        /// Which backend.
        backend: BackendId,
        /// What went wrong.
        detail: String,
    },

    /// The upstream returned a non-success status we pass through unchanged.
    #[error("{backend} returned {status}: {}", reason(body))]
    Upstream {
        /// Which backend.
        backend: BackendId,
        /// Status returned.
        status: http::StatusCode,
        /// Body, for the client.
        body: Bytes,
    },

    /// A credential was about to be attached to a host that did not issue it.
    /// This is a bug, and `docs/TRUST.md` I2 says it must be impossible rather
    /// than merely unlikely.
    #[error("refusing to send a {issuer} credential to {attempted}")]
    CredentialHostMismatch {
        /// Host that issued the credential.
        issuer: &'static str,
        /// Host we were about to send it to.
        attempted: String,
    },
}

impl UpstreamError {
    /// Whether trying a different backend is worthwhile.
    ///
    /// Only meaningful before the first response byte has reached the client.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Transport { .. } | Self::NeedsAuth { .. } => true,
            Self::Upstream { status, .. } => status.is_server_error(),
            // A host mismatch is our bug. Retrying elsewhere would hide it.
            Self::CredentialHostMismatch { .. } => false,
        }
    }

    /// Whether this failure says the *backend* is unhealthy.
    ///
    /// Distinct from [`Self::is_retryable`], which asks whether some other
    /// backend should be tried. The two differ where they must:
    ///
    /// - A missing or rejected credential is worth failing over, but it is a
    ///   configuration problem, not an outage. Counting it would open a circuit
    ///   and replace "re-run `claude login`" with "temporarily unavailable".
    /// - A rate limit means the backend is working exactly as designed, and its
    ///   own quota accounting already steers us away (`docs/CRITIQUE.md` §4).
    ///   Counting it too would take a backend out twice for one event.
    #[must_use]
    pub fn indicates_unhealthy_backend(&self) -> bool {
        match self {
            Self::Transport { .. } => true,
            Self::Upstream { status, .. } => status.is_server_error(),
            Self::RateLimited { .. }
            | Self::NeedsAuth { .. }
            | Self::CredentialHostMismatch { .. } => false,
        }
    }

    /// Which backend this came from, where one is known.
    #[must_use]
    pub fn backend_id(&self) -> Option<&BackendId> {
        match self {
            Self::NeedsAuth { backend, .. }
            | Self::RateLimited { backend, .. }
            | Self::Transport { backend, .. }
            | Self::Upstream { backend, .. } => Some(backend),
            Self::CredentialHostMismatch { .. } => None,
        }
    }

    /// What the upstream said, where it said anything.
    ///
    /// Only meaningful on [`Self::Upstream`]; every other variant carries its
    /// detail in the variant itself.
    #[must_use]
    pub fn upstream_reason(&self) -> Option<String> {
        match self {
            Self::Upstream { body, .. } => Some(reason(body)),
            _ => None,
        }
    }

    /// Provider-supplied retry delay, where there is one.
    #[must_use]
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_secs, ..
            } => *retry_after_secs,
            _ => None,
        }
    }
}

/// The provider's own explanation, pulled out of an error body.
///
/// A refusal the user cannot read is a refusal they cannot act on: the whole
/// symptom this exists for was a Codex session told only that "the upstream
/// closed without producing a response" when the provider had in fact named
/// the offending field. Providers overwhelmingly answer with either
/// `{"error": {"message": ...}}` or `{"error": "..."}`; anything else is
/// shown raw. Bounded, because an error body is upstream-controlled and ends
/// up in a log line and an error message.
///
/// This is not a hash-only surface. The detail is the point.
#[must_use]
pub fn reason(body: &Bytes) -> String {
    const MAX: usize = 600;

    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        return "no body".to_string();
    }

    let extracted = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| match value.get("error") {
            Some(serde_json::Value::String(message)) => Some(message.clone()),
            Some(error) => error
                .get("message")
                .and_then(|m| m.as_str())
                .map(ToString::to_string),
            None => value
                .get("message")
                .and_then(|m| m.as_str())
                .map(ToString::to_string),
        })
        .filter(|message| !message.trim().is_empty());

    let message = extracted.unwrap_or_else(|| text.to_string());
    if message.chars().count() > MAX {
        message.chars().take(MAX).collect::<String>() + "…"
    } else {
        message
    }
}

/// Why a subscription's credential cannot be used, ending in the command that
/// logs that login in.
///
/// Most reader errors already end that way; the ones that do not (an I/O
/// failure, an explicit fixture file) get it added. With several logins per
/// product, "run `claude`" is the wrong fix for all but one of them, so the
/// command has to be the login's own.
pub(crate) fn unusable_login(err: &CredentialError, login_command: &str) -> String {
    let says_what_to_run = matches!(
        err,
        CredentialError::NotLoggedIn { .. }
            | CredentialError::Expired { .. }
            | CredentialError::Malformed {
                command: Some(_),
                ..
            }
    );
    if says_what_to_run {
        err.to_string()
    } else {
        format!("{err}; run `{login_command}` to log in")
    }
}

/// Current state of a backend, as `ironwire status` shows it.
#[derive(Debug, Clone)]
pub struct BackendStatus {
    /// Stable id.
    pub id: BackendId,
    /// Display name.
    pub name: String,
    /// Capacity kind.
    pub kind: BackendKind,
    /// Whether a credential was found and looks usable.
    pub authenticated: bool,
    /// Opaque replay identity: a subscription's provider-observed account id
    /// (stable across token refresh), or an API key's exact credential hash.
    /// The latter identifies only the key, never an inferred organization.
    /// Missing subscription account identity remains `None`.
    pub account_scope: Option<String>,
    /// Why not, when `authenticated` is false.
    pub detail: Option<String>,
    /// Observed capacity. Never estimated (`docs/CRITIQUE.md` §4).
    pub quota: QuotaSnapshot,
    /// Models offered, best first.
    pub models: Vec<(String, ModelTier)>,
}

/// A place requests can go.
///
/// Async implementations must offload blocking filesystem, process, or
/// credential-store I/O themselves (for example with `spawn_blocking`). The
/// registry awaits them under bounded semaphores and never occupies a blocking
/// worker while waiting for another blocking job.
#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    /// Stable id.
    fn id(&self) -> &BackendId;

    /// Display name for `ironwire status`.
    fn name(&self) -> &str;

    /// Capacity kind.
    fn kind(&self) -> BackendKind;

    /// What this backend can preserve.
    fn capabilities(&self) -> &Capabilities;

    /// Models offered, best first.
    ///
    /// Owned rather than borrowed, because for some backends this is not a
    /// fixed list: the ChatGPT backend gates models behind the client version
    /// it is told about, so the real catalogue is something we *ask* for and
    /// then remember (`crate::codex_version`).
    fn models(&self) -> Vec<(String, ModelTier)>;

    /// Whether [`Self::models`] is what the provider said, rather than a list
    /// compiled into this binary.
    ///
    /// Routing needs the difference: see
    /// [`ironwire_core::policy::Candidate::catalogue_from_provider`]. Default
    /// `false`, because a backend that has never asked has only a guess.
    fn catalogue_from_provider(&self) -> bool {
        false
    }

    /// The client identity this backend will only serve, if it has one
    /// (`docs/TRANSLATION.md` aside, this is `docs/TRUST.md` §3).
    ///
    /// A *specific* product, not "some identity": a subscription is served only
    /// for the client it belongs to, and Claude Code's name must never unlock
    /// ChatGPT's subscription. `None` means the backend serves anyone, which is
    /// every metered key.
    fn required_client_identity(&self) -> Option<ClientIdentity> {
        None
    }

    /// The login this backend reads its credential from, if it is a
    /// subscription.
    ///
    /// One product can have several logins (Claude Code under several config
    /// dirs, Codex under several homes), each its own backend. This is how the
    /// router tells a product's default login from the others, and how the
    /// control API, CLI and consent prompt name the one they mean. `None` for
    /// everything that is not a subscription.
    fn login(&self) -> Option<SubscriptionLogin> {
        None
    }

    /// Current status, including a fresh credential check.
    async fn status(&self) -> BackendStatus;

    /// Send a request and return the response, streaming.
    ///
    /// # Errors
    ///
    /// Any [`UpstreamError`]. Implementations must not retry internally: retry
    /// and failover are the router's decision, because only it knows whether a
    /// byte has already reached the client.
    async fn send(&self, request: UpstreamRequest) -> Result<UpstreamResponse, UpstreamError>;

    /// Fold an observation drawn off the wire into this backend's quota state.
    fn record(&self, observation: &Observation);

    /// Latest observed quota.
    fn quota(&self) -> QuotaSnapshot;

    /// Seed observed quota from a previous run (`ironwire_core::quota_store`).
    ///
    /// Default: ignore, so a backend that tracks no quota needs no change. The
    /// caller has already applied the staleness rules — what arrives here is
    /// what is still true, with its original observation times intact.
    fn restore_quota(&self, _snapshot: QuotaSnapshot) {}

    /// The provider's own models document, verbatim, if this backend has one.
    ///
    /// `/v1/models` is not one schema. The public OpenAI endpoint answers with
    /// `{"object":"list","data":[…]}`, while the endpoint Codex actually asks —
    /// `chatgpt.com/backend-api/codex/models` — answers with `{"models":[…]}`,
    /// where each entry carries the client configuration Codex runs on:
    /// context window, truncation policy, reasoning levels, even the
    /// instructions template. Synthesizing a list in the other product's shape
    /// throws all of that away and leaves Codex on its compiled-in defaults.
    ///
    /// So this is the same rule as the native lane, applied to a second
    /// endpoint: forward the provider's bytes rather than a summary of them.
    /// `None` for backends that have no such document to forward.
    async fn models_document(&self) -> Option<Vec<u8>> {
        None
    }

    /// Whether this backend signs its answers, so that an exchange it served
    /// can later be checked against a receipt ([`Self::fetch_receipt`]).
    ///
    /// Decides whether a ledger row starts `pending` or `outside`. Default
    /// `false`.
    fn offers_receipts(&self) -> bool {
        false
    }

    /// Fetch the provider's receipt for the response it identified as
    /// `upstream_id`, served by `model`.
    ///
    /// Default [`crate::receipt::ReceiptFetch::NotOffered`]: only a provider
    /// that signs its answers has anything to fetch. Never called on the
    /// response path -- a receipt is looked up after the exchange is recorded
    /// (`ironwire_proxy::proof`) -- and the credential goes only to this
    /// backend's own host, exactly as for [`Self::send`].
    async fn fetch_receipt(
        &self,
        _upstream_id: &str,
        _model: &str,
    ) -> crate::receipt::ReceiptFetch {
        crate::receipt::ReceiptFetch::NotOffered
    }

    /// Verify this backend actually works, right now, over the network.
    ///
    /// `ironwire doctor` calls this because a config that parses and a
    /// credential that exists prove nothing: the failures that matter — an
    /// expired token, a beta flag the provider stopped honouring, an account
    /// not entitled to a model — only appear on the wire.
    ///
    /// Implementations must probe **without** claiming another product's
    /// identity (`docs/TRUST.md` §3). For subscription backends that rules out
    /// a synthetic inference call and leaves an auth-only check.
    ///
    /// # Errors
    ///
    /// Any [`UpstreamError`] the probe surfaces.
    async fn probe(&self) -> Result<(), UpstreamError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> BackendId {
        BackendId::from("test")
    }

    #[test]
    fn transient_failures_are_worth_another_backend() {
        assert!(
            UpstreamError::RateLimited {
                backend: id(),
                retry_after_secs: Some(30)
            }
            .is_retryable()
        );
        assert!(
            UpstreamError::Transport {
                backend: id(),
                detail: "reset".into()
            }
            .is_retryable()
        );
    }

    #[test]
    fn client_errors_are_not_retried_elsewhere() {
        // A 400 is the client's request being wrong; another backend will
        // reject it identically, and hiding that helps nobody.
        let err = UpstreamError::Upstream {
            backend: id(),
            status: http::StatusCode::BAD_REQUEST,
            body: Bytes::new(),
        };
        assert!(!err.is_retryable());

        let err = UpstreamError::Upstream {
            backend: id(),
            status: http::StatusCode::BAD_GATEWAY,
            body: Bytes::new(),
        };
        assert!(err.is_retryable());
    }

    #[test]
    fn a_refusal_carries_the_provider_own_words() {
        // The shape every OpenAI-compatible server uses.
        let err = UpstreamError::Upstream {
            backend: id(),
            status: http::StatusCode::BAD_REQUEST,
            body: Bytes::from_static(
                br#"{"error":{"message":"tools[22]: 'name' is a required property"}}"#,
            ),
        };
        assert!(
            err.to_string().contains("'name' is a required property"),
            "{err}"
        );

        // Anthropic's, and the bare-string variant some gateways send.
        assert_eq!(
            reason(&Bytes::from_static(br#"{"error":"model not found"}"#)),
            "model not found"
        );

        // Not JSON at all: shown rather than discarded, which is the whole
        // point — an unreadable refusal is one the user cannot act on.
        assert_eq!(
            reason(&Bytes::from_static(b"upstream connect error")),
            "upstream connect error"
        );
        assert_eq!(reason(&Bytes::new()), "no body");
    }

    #[test]
    fn a_refusal_is_bounded_because_the_body_is_not_ours() {
        let body = Bytes::from("x".repeat(10_000));
        let printed = reason(&body);
        assert!(printed.chars().count() <= 601, "{}", printed.len());
        assert!(printed.ends_with('…'));
    }

    #[test]
    fn a_credential_host_mismatch_is_never_retried_away() {
        // TRUST.md I2 — this is our bug, and failing loudly is the point.
        let err = UpstreamError::CredentialHostMismatch {
            issuer: "api.anthropic.com",
            attempted: "evil.example".into(),
        };
        assert!(!err.is_retryable());
    }

    #[test]
    fn retry_after_comes_from_the_provider_or_not_at_all() {
        assert_eq!(
            UpstreamError::RateLimited {
                backend: id(),
                retry_after_secs: Some(42)
            }
            .retry_after_secs(),
            Some(42)
        );
        assert_eq!(
            UpstreamError::RateLimited {
                backend: id(),
                retry_after_secs: None
            }
            .retry_after_secs(),
            None,
            "we must not invent a delay the provider did not give"
        );
    }
}
