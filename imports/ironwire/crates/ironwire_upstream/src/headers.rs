//! Header handling for the native lane.
//!
//! The exact mutation set is enumerated in `docs/PROTOCOL.md` §2 and tested
//! here. Anything not named is forwarded untouched — including provider headers
//! we have never heard of, which is how a new `anthropic-beta` value keeps
//! working without an IronWire release.

use ironwire_core::protocol::Protocol;

/// Headers that describe *this* hop and must never be forwarded to the next
/// one. Forwarding `transfer-encoding` in particular corrupts the response.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Headers carrying the *client's* credential. Stripped so a stray key in a
/// client config can never reach a provider under IronWire's routing, and
/// replaced with the credential the router actually chose.
const AUTH: &[&str] = &["authorization", "x-api-key", "api-key"];

/// Headers naming the account a bearer belongs to. Stripped with the bearer,
/// because they are only true of the credential they came with: a client's
/// copy beside the credential the router chose — another of the user's logins,
/// or another provider altogether — names an account that credential is not.
/// A backend that needs one sets it from the token it is presenting
/// (`docs/PROTOCOL.md` §2 item 2).
pub(crate) const ACCOUNT_BOUND: &[&str] = &["chatgpt-account-id", "x-openai-fedramp"];

/// Headers we replace because they describe the connection to IronWire rather
/// than the connection to the provider.
const REWRITTEN: &[&str] = &["host", "content-length", "accept-encoding"];

/// Headers addressed to IronWire itself. Read here, never forwarded: a
/// provider did not ask for them, and forwarding one would tell it there is a
/// proxy in the path.
const ADDRESSED_TO_US: &[&str] = &[NEUTRAL_SESSION_HEADER];

/// The one header any client can use to name its session to IronWire.
///
/// The two native headers below are what Claude Code and Codex already send.
/// Everything else that speaks Chat Completions -- Aider, Cline, Roo -- sends
/// no session header at all, so their rows can never be attributed. A client
/// or wrapper that can add a request header sets this one and becomes
/// attributable without IronWire having to know it exists. It takes
/// precedence over the native header when both are present, because it is
/// the one the client chose to address to us.
pub const NEUTRAL_SESSION_HEADER: &str = "x-ironwire-session-id";

/// Whether an inbound request header should be forwarded upstream.
#[must_use]
pub fn forward_request_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !HOP_BY_HOP.contains(&lower.as_str())
        && !AUTH.contains(&lower.as_str())
        && !ACCOUNT_BOUND.contains(&lower.as_str())
        && !REWRITTEN.contains(&lower.as_str())
        && !ADDRESSED_TO_US.contains(&lower.as_str())
}

/// The header a coding agent uses to name its own session, per inbound wire.
///
/// Both values are measured, not guessed: Claude Code 2.1.252 sends
/// `x-claude-code-session-id` and Codex 0.151.0 sends `session-id`, on every
/// request. Neither is on any list above, so both already reach the provider
/// untouched -- this only says which one is worth *recording*.
///
/// Worth recording because `Exchange::conversation` cannot answer "which
/// session was this". That key is a routing-affinity hash, stable across a
/// whole session by design and therefore stable across two sessions that share
/// a tool list. This is the client's own identifier, and it is what makes
/// `ironwire log` line up with the session a user is actually looking at.
#[must_use]
pub fn client_session_header(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::AnthropicMessages => "x-claude-code-session-id",
        Protocol::OpenAiResponses | Protocol::OpenAiChat => "session-id",
    }
}

/// Whether an upstream response header should be forwarded to the client.
///
/// Content encoding is included deliberately: we do not decompress, so the
/// client must be told what it is receiving.
#[must_use]
pub fn forward_response_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !HOP_BY_HOP.contains(&lower.as_str()) && lower != "content-length"
}

/// Select the inbound headers to forward.
#[must_use]
pub fn select_forwarded<'a, I>(headers: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    headers
        .into_iter()
        .filter(|(name, _)| forward_request_header(name))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_agent_names_its_session_in_its_own_header() {
        // Measured against the real clients, not inferred from their docs:
        // Claude Code 2.1.252 and Codex 0.151.0. If an agent renames its
        // header, this is the test that says so rather than a ledger column
        // that quietly goes empty.
        assert_eq!(
            client_session_header(Protocol::AnthropicMessages),
            "x-claude-code-session-id"
        );
        assert_eq!(
            client_session_header(Protocol::OpenAiResponses),
            "session-id"
        );
        assert_eq!(client_session_header(Protocol::OpenAiChat), "session-id");
    }

    #[test]
    fn reading_a_session_header_does_not_stop_it_reaching_the_provider() {
        // Recording is not interception. Both headers must still forward
        // untouched, or the client's own correlation breaks upstream.
        for protocol in [
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
            Protocol::OpenAiChat,
        ] {
            assert!(forward_request_header(client_session_header(protocol)));
        }
    }

    #[test]
    fn the_neutral_session_header_is_addressed_to_us_and_stops_here() {
        // A client that names its session to IronWire is talking to this hop,
        // not to the provider. Forwarding it would hand a provider a header
        // it never asked for, and would leak that a proxy is in the path.
        assert_eq!(NEUTRAL_SESSION_HEADER, "x-ironwire-session-id");
        assert!(!forward_request_header(NEUTRAL_SESSION_HEADER));
        assert!(!forward_request_header("X-IronWire-Session-Id"));
    }

    #[test]
    fn the_native_session_headers_still_reach_the_provider() {
        // Adding a header of our own must not change what happens to theirs.
        for protocol in [
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
            Protocol::OpenAiChat,
        ] {
            assert!(forward_request_header(client_session_header(protocol)));
        }
    }

    #[test]
    fn provider_headers_we_have_never_seen_are_forwarded() {
        // This is the whole point of the native lane: a beta flag shipped
        // after us must keep working without an IronWire release.
        assert!(forward_request_header("anthropic-beta"));
        assert!(forward_request_header("anthropic-version"));
        assert!(forward_request_header("anthropic-some-future-thing"));
        assert!(forward_request_header("x-stainless-lang"));
    }

    #[test]
    fn the_clients_own_credentials_never_ride_along() {
        assert!(!forward_request_header("authorization"));
        assert!(!forward_request_header("Authorization"));
        assert!(!forward_request_header("x-api-key"));
        assert!(!forward_request_header("X-Api-Key"));
    }

    #[test]
    fn an_account_the_client_names_never_rides_along_either() {
        // The bearer and the account it belongs to are one fact. Stripping the
        // first while forwarding the second would pair the client's account
        // with whichever credential the router chose.
        for name in [
            "chatgpt-account-id",
            "ChatGPT-Account-ID",
            "x-openai-fedramp",
            "X-OpenAI-Fedramp",
        ] {
            assert!(!forward_request_header(name), "{name} leaked upstream");
        }
        // Codex's per-session headers are not account-bound and still pass.
        assert!(forward_request_header("x-codex-installation-id"));
        assert!(forward_request_header("originator"));
    }

    #[test]
    fn hop_by_hop_headers_are_dropped_in_both_directions() {
        for name in ["connection", "Transfer-Encoding", "keep-alive", "upgrade"] {
            assert!(!forward_request_header(name), "{name} leaked upstream");
            assert!(!forward_response_header(name), "{name} leaked downstream");
        }
    }

    #[test]
    fn rate_limit_headers_reach_us_intact() {
        // Observation depends on these surviving the response path.
        assert!(forward_response_header(
            "anthropic-ratelimit-unified-status"
        ));
        assert!(forward_response_header("retry-after"));
    }

    #[test]
    fn content_length_is_recomputed_not_forwarded() {
        // A policy-driven model edit changes the body length; forwarding the
        // original would truncate the request.
        assert!(!forward_request_header("content-length"));
        assert!(!forward_response_header("content-length"));
    }

    #[test]
    fn selection_normalises_names_and_keeps_order() {
        let selected = select_forwarded([
            ("Anthropic-Version", "2023-06-01"),
            ("Authorization", "Bearer secret"),
            ("Content-Type", "application/json"),
        ]);
        assert_eq!(
            selected,
            vec![
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ]
        );
    }
}
