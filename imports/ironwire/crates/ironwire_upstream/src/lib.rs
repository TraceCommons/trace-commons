//! Upstream backends.
//!
//! The native lane's contract (`docs/PROTOCOL.md` §2): for a request whose
//! inbound protocol matches the backend's, IronWire mutates the URL, the auth
//! headers, the hop-by-hop headers and — only when policy changed it — the
//! `model` key. Nothing else. The body is otherwise forwarded as the bytes the
//! client sent, and the response is forwarded back frame-for-frame.
//!
//! That is what makes fidelity 1.0 by construction, including for provider
//! features that did not exist when this shipped: unknown fields survive
//! because nothing here looks at them.
#![warn(missing_docs)]

pub mod anthropic;
pub mod backend;
pub mod breaker;
pub mod codex_version;
mod credential_host;
mod credential_read;
mod first_party_client;
pub mod headers;
pub mod observe;
pub mod openai_chat;
pub mod openai_responses;
pub mod receipt;
pub mod sse;

pub use backend::{Backend, BackendStatus, UpstreamError, UpstreamRequest, UpstreamResponse};

/// A model catalogue: each slug and the quality tier it satisfies, best first.
pub type Catalogue = Vec<(String, ironwire_core::protocol::ModelTier)>;
pub use observe::{Observation, RateLimitReading, UsageReading};

/// Join an OpenAI-family base URL to the path the client asked for.
///
/// An OpenAI-compatible `base_url` carries its own root, version segment and
/// all: `https://api.openai.com/v1` for the metered API,
/// `https://chatgpt.com/backend-api/codex` for the ChatGPT subscription — and
/// `http://127.0.0.1:8463/openai/v1` is what IronWire tells Codex to use. The
/// client's own path arrives versioned on top of that (`/v1/responses`), so
/// concatenating the two gives `…/v1/v1/responses` or `…/codex/v1/responses`.
/// Both are 404s, against the two endpoints that actually matter.
///
/// The mocks did not catch this because a mock is mounted at the server root,
/// where the two spellings coincide — the one shape no real provider has.
#[must_use]
pub fn endpoint_url(base_url: &str, client_path: &str) -> String {
    let suffix = client_path.strip_prefix("/v1").unwrap_or(client_path);
    format!("{}{}", base_url.trim_end_matches('/'), suffix)
}

#[cfg(test)]
mod endpoint_tests {
    use super::endpoint_url;

    #[test]
    fn the_two_endpoints_that_matter() {
        assert_eq!(
            endpoint_url("https://chatgpt.com/backend-api/codex", "/v1/responses"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            endpoint_url("https://api.openai.com/v1", "/v1/responses"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            endpoint_url("https://api.near.ai/v1", "/v1/chat/completions"),
            "https://api.near.ai/v1/chat/completions"
        );
    }

    /// A base URL that is only a host still gets a usable endpoint — this is
    /// the mock's shape, and the shape a self-hosted server may take.
    #[test]
    fn a_bare_host_base() {
        assert_eq!(
            endpoint_url("http://127.0.0.1:9000", "/v1/responses"),
            "http://127.0.0.1:9000/responses"
        );
    }
}
