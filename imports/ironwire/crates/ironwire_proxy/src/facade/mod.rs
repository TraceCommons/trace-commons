//! Provider façades: the native API surfaces IronWire presents on loopback.

pub mod anthropic;
pub mod error;
pub mod openai;

pub use error::FacadeError;

use ironwire_core::policy::ConversationKey;
use ironwire_core::protocol::BackendId;
use ironwire_creds::{SubscriptionLogin, SubscriptionProduct};
use ironwire_upstream::backend::Backend;

use crate::state::AppState;

/// The login a request at `/anthropic/account/{id}` or `/openai/account/{id}`
/// is bound to.
///
/// That URL is how a client says which login it is: Claude Code run with
/// `CLAUDE_CONFIG_DIR=~/.claude-work` is pointed at the `claude-work` URL, and
/// is then served by that login's subscription and no other (`docs/DESIGN.md`
/// §5). So an id that names nothing is refused rather than treated as unbound —
/// quietly serving it as the default login would put one account's request on
/// another account's token, the exact drift the binding exists to prevent.
///
/// # Errors
///
/// A message for the client — naming the logins that do exist and what to run
/// — when `account` is not a registered login of `product`.
pub(crate) fn bound_login(
    state: &AppState,
    account: &str,
    product: SubscriptionProduct,
) -> Result<BackendId, String> {
    let id = BackendId::from(account);
    let named = state.backends.get(&id);
    if named
        .and_then(|backend| backend.login())
        .is_some_and(|login| login.product == product)
    {
        return Ok(id);
    }

    let facade = match product {
        SubscriptionProduct::Claude => "anthropic",
        SubscriptionProduct::ChatGpt => "openai",
    };
    let name = product.name();
    // Client-supplied text, on its way to a terminal: anything that is not a
    // plain identifier is described rather than echoed.
    let shown = if !account.is_empty()
        && account.len() <= 64
        && account
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        format!("`{account}`")
    } else {
        "an account".to_string()
    };
    let what = match named.map(|backend| backend.login()) {
        Some(Some(login)) => format!(
            "This URL names {shown}, which is a {} login rather than a {name} one.",
            login.product.name()
        ),
        Some(None) => format!("This URL names {shown}, which is a backend but not a login."),
        None => format!("This URL names the {name} login {shown}, which IronWire does not have."),
    };
    let logins: Vec<String> = state
        .backends
        .all()
        .iter()
        .filter(|backend| {
            backend
                .login()
                .is_some_and(|login| login.product == product)
        })
        .map(|backend| format!("/{facade}/account/{}", backend.id()))
        .collect();
    let have = if logins.is_empty() {
        format!("It has no {name} login registered.")
    } else {
        format!("Its {name} logins are at {}.", logins.join(", "))
    };
    let connect = SubscriptionLogin::default_for(product).connect_command();
    let connect = if account == product.default_backend_id() {
        connect
    } else {
        format!("{connect} --config-dir DIR")
    };
    Err(format!(
        "{what} {have} To add one, run `{connect}`; `ironwire status` lists every backend."
    ))
}

/// The conversation key for a request on `client_login`.
///
/// A conversation on one login is not the same conversation on another, even
/// when the two sessions share a system prompt and tool set — which every
/// session of one client version does. Sharing a key would make them share one
/// sticky route, so each login's turns would keep evicting the other's. A
/// request at the plain façade keeps the key it always had.
pub(crate) fn key_for_login(
    key: ConversationKey,
    client_login: Option<&BackendId>,
) -> ConversationKey {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let Some(login) = client_login else {
        return key;
    };
    let mut hasher = DefaultHasher::new();
    key.0.hash(&mut hasher);
    login.as_str().hash(&mut hasher);
    ConversationKey(hasher.finish())
}

/// What to run to connect the login `id` reads, for a refusal that names it.
pub(crate) fn connect_command(state: &AppState, id: &BackendId) -> Option<String> {
    state
        .backends
        .get(id)
        .and_then(|backend| backend.login())
        .map(|login| login.connect_command())
}

/// Whether a model list may offer what `backend` serves to a client on
/// `client_login` (`None` at the plain façade, which belongs to the default
/// login in its default place).
///
/// The router's own-login rule (`ironwire_core::policy`), asked again for the
/// lists that offer models: a picker must not offer what the request would
/// then be refused, and another login's subscription is exactly that unless
/// the user turned on cross-account fallback. Everything that is not a login
/// is untouched by it.
pub(crate) fn offered_to(
    backend: &dyn Backend,
    client_login: Option<&BackendId>,
    cross_account_fallback: bool,
) -> bool {
    let Some(login) = backend.login() else {
        return true;
    };
    cross_account_fallback
        || match client_login {
            Some(id) => backend.id() == id,
            // `Candidate::default_login`'s rule: a relocated default is
            // reached only through its bound URL.
            None => login.default && login.location.is_none(),
        }
}

/// The account segment of a login-bound URL, or `""` when axum could not
/// extract one.
///
/// A segment that is not UTF-8 once percent-decoded is rejected by `Path`
/// before the handler runs, with axum's own plain-text 400. Taking the
/// rejection and asking [`bound_login`] about an empty account gives that
/// client the façade's 404 instead — the same envelope, list of logins and
/// command as any other URL naming no login.
pub(crate) fn account_segment(
    account: Result<axum::extract::Path<String>, axum::extract::rejection::PathRejection>,
) -> String {
    account
        .map(|axum::extract::Path(account)| account)
        .unwrap_or_default()
}

/// How Codex spelled its session header before `session-id`.
///
/// Read as well because the session id keys which ChatGPT account a session's
/// reasoning lives on, not only the ledger row: an older Codex whose sessions
/// all fell back to the shared conversation key could have one session's move
/// to another login taken as every session's.
const LEGACY_CODEX_SESSION_HEADER: &str = "session_id";

/// The client's own session id, when it sent one.
///
/// Read from `x-ironwire-session-id` when the client sent it, and otherwise
/// from the native header for the facade's protocol — on the Responses wire,
/// `session-id`, else the older `session_id`. The neutral header is
/// the one the client chose to address to IronWire, so it takes precedence
/// whenever both are present.
///
/// Bounded and sanitised before it reaches the ledger: this is client-supplied
/// text, and it is written to a local database that `ironwire log` renders to a
/// terminal. Anything that is not a plain identifier is dropped rather than
/// stored -- a row addressed by nothing is strictly better than a row that can
/// smuggle control characters onto a user's screen.
pub(crate) fn client_session_id(
    headers: &axum::http::HeaderMap,
    protocol: ironwire_core::protocol::Protocol,
) -> Option<String> {
    // The header addressed to us wins; the native one is the fallback. A
    // neutral header that fails validation is a client that sent us garbage,
    // and the answer is nothing -- not its native id filed instead.
    let native = ironwire_upstream::headers::client_session_header(protocol);
    let name = if headers.contains_key(ironwire_upstream::headers::NEUTRAL_SESSION_HEADER) {
        ironwire_upstream::headers::NEUTRAL_SESSION_HEADER
    } else if protocol == ironwire_core::protocol::Protocol::OpenAiResponses
        && !headers.contains_key(native)
        && headers.contains_key(LEGACY_CODEX_SESSION_HEADER)
    {
        LEGACY_CODEX_SESSION_HEADER
    } else {
        native
    };
    let raw = headers.get(name)?.to_str().ok()?;
    let ok = !raw.is_empty()
        && raw.len() <= 200
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    ok.then(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use ironwire_core::protocol::Protocol;

    fn with(name: &'static str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(v) = axum::http::HeaderValue::from_str(value) {
            headers.insert(name, v);
        }
        headers
    }

    #[test]
    fn a_real_session_id_is_recorded() {
        let headers = with(
            "x-claude-code-session-id",
            "5db811ed-ce4a-45a7-ab00-56890e111668",
        );
        assert_eq!(
            client_session_id(&headers, Protocol::AnthropicMessages).as_deref(),
            Some("5db811ed-ce4a-45a7-ab00-56890e111668")
        );
        let headers = with("session-id", "01a05a5d-243a-7ac0-bdcd-ba06b4309c36");
        assert_eq!(
            client_session_id(&headers, Protocol::OpenAiResponses).as_deref(),
            Some("01a05a5d-243a-7ac0-bdcd-ba06b4309c36")
        );
    }

    #[test]
    fn an_absent_header_records_nothing_rather_than_an_empty_string() {
        assert!(client_session_id(&HeaderMap::new(), Protocol::AnthropicMessages).is_none());
        let headers = with("x-claude-code-session-id", "");
        assert!(client_session_id(&headers, Protocol::AnthropicMessages).is_none());
    }

    #[test]
    fn one_conversation_on_two_logins_is_two_conversations() {
        let key = ConversationKey::derive(Protocol::AnthropicMessages, "You are Claude Code", &[]);
        let work = BackendId::from("claude-work");
        let play = BackendId::from("claude-play");
        assert_eq!(
            key_for_login(key.clone(), None),
            key,
            "the plain façade keeps the key it always had"
        );
        assert_ne!(key_for_login(key.clone(), Some(&work)), key);
        assert_ne!(
            key_for_login(key.clone(), Some(&work)),
            key_for_login(key.clone(), Some(&play))
        );
        assert_eq!(
            key_for_login(key.clone(), Some(&work)),
            key_for_login(key, Some(&work)),
            "and stable, or it is not affinity"
        );
    }

    #[test]
    fn a_session_id_that_is_not_an_identifier_is_dropped() {
        // Client-supplied text that `ironwire log` renders to a terminal. A row
        // addressed by nothing beats a row carrying an escape sequence.
        for hostile in [
            "not a session id",
            "\u{1b}[2Jcleared",
            "a/../../etc/passwd",
            "<script>",
        ] {
            let headers = with("x-claude-code-session-id", hostile);
            assert!(
                client_session_id(&headers, Protocol::AnthropicMessages).is_none(),
                "should have dropped {hostile:?}"
            );
        }
        let headers = with("x-claude-code-session-id", &"a".repeat(201));
        assert!(client_session_id(&headers, Protocol::AnthropicMessages).is_none());
    }
}

#[cfg(test)]
mod join_contract {
    use super::*;
    use axum::http::HeaderMap;
    use ironwire_core::protocol::Protocol;

    fn with(name: &'static str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            name,
            axum::http::HeaderValue::from_str(value).expect("header"),
        );
        headers
    }

    /// A consumer joins on this, so the header a façade reads is part of the
    /// contract rather than an implementation detail. `docs/PROTOCOL.md` §3
    /// states these two; a change here is a change there.
    #[test]
    fn each_facade_reads_the_header_protocol_md_names() {
        assert_eq!(
            ironwire_upstream::headers::client_session_header(Protocol::AnthropicMessages),
            "x-claude-code-session-id"
        );
        assert_eq!(
            ironwire_upstream::headers::client_session_header(Protocol::OpenAiResponses),
            "session-id"
        );
        assert_eq!(
            ironwire_upstream::headers::client_session_header(Protocol::OpenAiChat),
            "session-id"
        );
    }

    /// Verbatim, and that is the whole contract: a consumer holding the same
    /// session under a different spelling must map it itself, because
    /// IronWire never sees the other spelling and cannot.
    ///
    /// The values here are the two real shapes we have seen: Claude Code
    /// sends a bare UUID, and a client whose own identifier carries a prefix
    /// sends it with the prefix intact.
    #[test]
    fn the_session_id_is_stored_verbatim() {
        let bare = "5db811ed-ce4a-45a7-ab00-56890e111668";
        assert_eq!(
            client_session_id(
                &with("x-claude-code-session-id", bare),
                Protocol::AnthropicMessages
            )
            .as_deref(),
            Some(bare),
            "no normalisation, no case folding"
        );

        let prefixed = "rollout-2026-09-02T10-14-22-5db811ed-ce4a-45a7-ab00-56890e111668";
        assert_eq!(
            client_session_id(&with("session-id", prefixed), Protocol::OpenAiChat).as_deref(),
            Some(prefixed),
            "a prefix is not stripped -- a consumer expecting a bare UUID here \
             would join nothing, and silently"
        );
    }

    /// Older Codex — and fixtures copied from it — spell the header
    /// `session_id`. It is read when `session-id` is absent, under the same
    /// rules, and only on the wire Codex speaks.
    #[test]
    fn codexs_older_session_header_is_read_after_the_current_one() {
        let id = "01998f6e-0000-7000-8000-000000000000";
        assert_eq!(
            client_session_id(&with("session_id", id), Protocol::OpenAiResponses).as_deref(),
            Some(id)
        );

        let mut both = with("session-id", "current-id");
        both.insert(
            "session_id",
            axum::http::HeaderValue::from_static("older-id"),
        );
        assert_eq!(
            client_session_id(&both, Protocol::OpenAiResponses).as_deref(),
            Some("current-id")
        );

        assert_eq!(
            client_session_id(
                &with("session_id", "not a session id"),
                Protocol::OpenAiResponses
            ),
            None
        );
        assert_eq!(
            client_session_id(&with("session_id", id), Protocol::AnthropicMessages),
            None
        );
    }

    /// A client that names no session yields no attribution, rather than a
    /// fabricated one. Same rule as usage and capacity.
    #[test]
    fn no_header_is_none_not_a_placeholder() {
        assert_eq!(
            client_session_id(&HeaderMap::new(), Protocol::AnthropicMessages),
            None
        );
    }

    /// A client with no native session header can still name its session.
    #[test]
    fn the_neutral_header_is_read_on_every_facade() {
        let id = "aider-2026-09-03-0001";
        for protocol in [
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
            Protocol::OpenAiChat,
        ] {
            assert_eq!(
                client_session_id(&with("x-ironwire-session-id", id), protocol).as_deref(),
                Some(id),
                "{protocol:?} must read the neutral header"
            );
        }
    }

    /// The header the client addressed to us wins over the one it addresses
    /// to the provider. A wrapper that sets ours has said which id it wants
    /// the row filed under.
    #[test]
    fn the_neutral_header_takes_precedence_over_the_native_one() {
        let mut headers = with("x-claude-code-session-id", "native-id");
        headers.insert(
            "x-ironwire-session-id",
            axum::http::HeaderValue::from_static("chosen-id"),
        );
        assert_eq!(
            client_session_id(&headers, Protocol::AnthropicMessages).as_deref(),
            Some("chosen-id")
        );
    }

    /// The same validation applies: a hostile neutral header is dropped, and
    /// does not fall through to the native one either.
    #[test]
    fn a_hostile_neutral_header_is_dropped_not_bypassed() {
        let mut headers = with("x-claude-code-session-id", "native-id");
        headers.insert(
            "x-ironwire-session-id",
            axum::http::HeaderValue::from_static("not a session id"),
        );
        assert_eq!(
            client_session_id(&headers, Protocol::AnthropicMessages),
            None,
            "a client that sent us garbage does not get its native id filed instead"
        );
    }
}
