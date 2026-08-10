//! Loopback (native-app) account sign-in: PKCE binding, exact loopback
//! redirect matching, and the bearer-token encoding for a native session.
//!
//! # Why this exists
//!
//! `/v1/account/*` is guarded by a browser session cookie (Secure, HttpOnly,
//! SameSite=Strict, rotated per request). A native application — the
//! contributor daemon, the macOS app — cannot hold that cookie, and must not
//! be handed a way to make the browser session reachable from a native
//! process. So a native client instead completes the EXISTING browser
//! login-link flow and receives, on a loopback redirect, a one-time
//! authorization code it exchanges for a short-lived bearer token bound to a
//! real `trace_sessions` row (`client_kind = 'native'`).
//!
//! Because the token IS a session row, everything that already governs
//! sessions governs it for free: expiry, idle cap, rotation-on-use, and
//! `POST /v1/account/sessions/revoke-all`.
//!
//! # The threat this module is written against
//!
//! A loopback redirect is reachable by ANY local process. Another process on
//! the same machine can race the listener, or read the code out of a browser
//! history / a shell scrollback. The code alone is therefore NOT sufficient
//! authority:
//!
//! - **PKCE (S256).** The app generates a high-entropy verifier, sends only
//!   `sha256(verifier)` when it starts the flow, and must present the verifier
//!   at exchange. A code intercepted by another local process is useless.
//! - **One-time, short-lived codes.** Both the pending request and the issued
//!   code live in a single-use, TTL-bounded store; the first successful `take`
//!   removes them, so a replay finds nothing.
//! - **Exact redirect matching.** Only `http://127.0.0.1:{port}` with the one
//!   fixed path below. No wildcards, no `localhost` (which resolves through
//!   the resolver and can be pointed elsewhere), no `[::1]`, no user-supplied
//!   host, no query or fragment.
//!
//! # Single-instance limitation
//!
//! The pending-request and issued-code stores are the in-process
//! [`crate::account_passkey::CeremonyStore`], with the same caveat: a flow
//! started on one process cannot be finished on another, and state is lost on
//! restart. Both TTLs are minutes, and the pilot runs a single ingest process.
//! A horizontally-scaled deployment MUST move these to a shared short-TTL
//! store.

use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha256};

/// TTL for a pending authorization request (created by the app, consumed by
/// the browser when the user activates the login link). Minutes, not hours:
/// this covers a human switching to their browser and clicking one button.
pub const NATIVE_AUTH_REQUEST_TTL: Duration = Duration::from_secs(5 * 60);

/// TTL for an issued authorization code (created by the browser redeem,
/// consumed by the app's exchange). Deliberately much shorter than the
/// request TTL: the app is already listening when the code is minted, so the
/// only thing this window has to cover is one loopback round trip.
pub const NATIVE_AUTH_CODE_TTL: Duration = Duration::from_secs(2 * 60);

/// Lifetime of the bearer token minted at exchange. Short-lived by
/// requirement: a native client re-runs the browser flow rather than holding a
/// week-long credential the way a browser cookie does.
pub const NATIVE_SESSION_TTL_HOURS: i64 = 12;

/// `client_kind` recorded on the session row. A public label. It is NOT in
/// [`crate::account_session::AccountCtx::is_strong_session`]'s strong set, so
/// a native token is a WEAK authenticator exactly like a device-link web
/// session: it can read and withdraw, and it can never change authenticators
/// or redirect payouts.
pub const NATIVE_SESSION_CLIENT_KIND: &str = "native";

/// The ONLY loopback path a native client may register. Fixed server-side so
/// "exact redirect matching" is a string comparison against a constant rather
/// than a policy about client-supplied paths.
pub const NATIVE_REDIRECT_PATH: &str = "/trace-commons/native-auth/callback";

/// The only loopback host accepted. Literal IPv4 loopback: never `localhost`
/// (a name, resolvable to anything by a hostile resolver or hosts file) and
/// never `[::1]` (a second spelling that would have to be validated twice).
const NATIVE_REDIRECT_HOST: &str = "http://127.0.0.1:";

/// Lowest port a loopback redirect may bind. Below 1024 is privileged; a real
/// ephemeral port is far above this.
const NATIVE_REDIRECT_MIN_PORT: u16 = 1024;

/// Prefix marking a bearer token as an ACCOUNT native-session token rather
/// than a device upload claim. The two arrive in the same `Authorization:
/// Bearer` header and must never be confused: the resolver dispatches on this
/// prefix, so a device claim can never be mistaken for a session and a session
/// token can never be mistaken for a device claim.
pub const NATIVE_TOKEN_PREFIX: &str = "tcn1_";

/// The PKCE method this server accepts. `plain` is deliberately NOT accepted:
/// with `plain` the challenge IS the verifier, so an intercepted start request
/// hands over everything.
pub const NATIVE_CODE_CHALLENGE_METHOD: &str = "S256";

/// Minimum / maximum verifier length, per RFC 7636 section 4.1.
const VERIFIER_MIN_LEN: usize = 43;
const VERIFIER_MAX_LEN: usize = 128;

/// A base64url(sha256) challenge is always exactly 43 characters unpadded.
const CHALLENGE_LEN: usize = 43;

/// A validated loopback redirect URI. Constructible only through
/// [`validate_loopback_redirect_uri`], so a handler cannot accidentally hold an
/// unvalidated one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopbackRedirect {
    uri: String,
    port: u16,
}

impl LoopbackRedirect {
    /// The exact URI string, safe to use as a `Location` header.
    pub fn as_str(&self) -> &str {
        &self.uri
    }

    /// The loopback port the app is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Accept `http://127.0.0.1:{port}{NATIVE_REDIRECT_PATH}` and nothing else.
///
/// Everything is compared literally: scheme+host prefix, then a decimal port in
/// `[1024, 65535]`, then the fixed path, then end-of-string. There is no URL
/// parser in the path, so there is no parser-differential to exploit — a
/// userinfo section, an added query or fragment, an alternate host spelling,
/// a trailing slash, or an uppercase scheme all simply fail to match.
pub fn validate_loopback_redirect_uri(uri: &str) -> Option<LoopbackRedirect> {
    let rest = uri.strip_prefix(NATIVE_REDIRECT_HOST)?;
    let (port_str, path) = rest.split_once('/')?;
    // `split_once('/')` ate the leading '/', so put it back before comparing
    // against the fixed path constant.
    if format!("/{path}") != NATIVE_REDIRECT_PATH {
        return None;
    }
    // Reject a port with a leading '+', '-', leading zero, or any non-digit:
    // `u16::from_str` already rejects sign and non-digits, and a leading zero
    // would give two spellings of one port.
    if port_str.is_empty() || (port_str.len() > 1 && port_str.starts_with('0')) {
        return None;
    }
    if !port_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    if port < NATIVE_REDIRECT_MIN_PORT {
        return None;
    }
    Some(LoopbackRedirect {
        uri: uri.to_string(),
        port,
    })
}

/// Whether a PKCE verifier is well-formed: RFC 7636's 43..=128 characters from
/// the unreserved set. Checked before hashing so a caller cannot smuggle
/// arbitrary bytes (or a trivially short verifier) into the binding.
pub fn verifier_is_wellformed(verifier: &str) -> bool {
    let len = verifier.len();
    if !(VERIFIER_MIN_LEN..=VERIFIER_MAX_LEN).contains(&len) {
        return false;
    }
    verifier
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// Whether a submitted `code_challenge` has the exact shape of an unpadded
/// base64url sha256 digest. A malformed challenge is rejected at start rather
/// than silently never matching at exchange.
pub fn challenge_is_wellformed(challenge: &str) -> bool {
    challenge.len() == CHALLENGE_LEN
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// `base64url_nopad(sha256(verifier))` — the S256 challenge derivation, shared
/// by the client and by the exchange handler so the two cannot drift.
pub fn challenge_for_verifier(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Constant-time equality for the PKCE and code comparisons.
///
/// Length inequality is unavoidably observable (and both operands here are
/// fixed-length in the honest case), but the byte comparison itself must not
/// short-circuit: these compare a client-supplied value against a stored
/// secret-derived one.
pub fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // Compare every overlapping byte without short-circuiting, then fold in the
    // length check at the end. `black_box` on the accumulator keeps the
    // optimizer from reintroducing an early exit. Length inequality IS
    // observable, which is fine: every operand compared here is fixed-length in
    // the honest case (a 43-char challenge, a 27-char code handle), so length
    // leaks nothing about the secret's value.
    // (`ring::constant_time` is deprecated upstream and `subtle` is not in this
    // tree; this is a few lines rather than a new dependency.)
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
        diff = std::hint::black_box(diff);
    }
    a.len() == b.len() && diff == 0
}

/// State held between the app's authorize-start and the browser's redeem.
///
/// Note what is NOT here: no account, no tenant, no principal. Starting a flow
/// is unauthenticated and confers nothing; the account is attached only when a
/// human completes the browser login.
#[derive(Debug, Clone)]
pub struct PendingNativeAuth {
    pub code_challenge: String,
    pub redirect: LoopbackRedirect,
}

/// State held between the browser's redeem and the app's token exchange.
///
/// `code_challenge` is carried forward from the pending request so the
/// exchange verifies the verifier against the challenge that was registered
/// BEFORE the browser step — the whole point of PKCE.
#[derive(Debug, Clone)]
pub struct IssuedNativeCode {
    pub request_id: String,
    pub code_challenge: String,
    pub tenant_id: String,
    pub account_id: uuid::Uuid,
}

/// Encode a native bearer token: `tcn1_{b64url(tenant_id)}.{secret}`.
///
/// The tenant travels with the token for exactly the reason it travels in the
/// session cookie — to bootstrap an RLS-scoped lookup from a request that
/// carries no tenant context — and it is equally safe: only `sha256(secret)`
/// is stored, and that hash is globally unique, so a forged tenant scopes the
/// lookup to a tenant where the hash does not exist and finds no row.
pub fn native_token_value(tenant_id: &str, secret: &str) -> String {
    format!(
        "{NATIVE_TOKEN_PREFIX}{}.{secret}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tenant_id.as_bytes()),
    )
}

/// Inverse of [`native_token_value`]: `(tenant_id, sha256(secret))`, or `None`
/// for anything malformed. Returns the HASH, never the secret, so no caller can
/// accidentally hold the raw secret after parsing.
pub fn native_token_parts(token: &str) -> Option<(String, String)> {
    let body = token.strip_prefix(NATIVE_TOKEN_PREFIX)?;
    let (b64_tenant, secret) = body.split_once('.')?;
    if secret.is_empty() {
        return None;
    }
    let tenant_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64_tenant)
        .ok()?;
    let tenant_id = String::from_utf8(tenant_bytes).ok()?;
    Some((tenant_id, crate::account_session::hash_secret(secret)))
}

/// Whether a bearer value is a native account token (and so must be resolved
/// as a session, not as a device upload claim).
pub fn is_native_token(bearer: &str) -> bool {
    bearer.starts_with(NATIVE_TOKEN_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_exact_loopback_redirect() {
        let uri = format!("http://127.0.0.1:52341{NATIVE_REDIRECT_PATH}");
        let redirect = validate_loopback_redirect_uri(&uri).expect("exact loopback is accepted");
        assert_eq!(redirect.port(), 52341);
        assert_eq!(redirect.as_str(), uri);
    }

    #[test]
    fn rejects_every_non_loopback_spelling() {
        // Each of these is a way an attacker (or a careless client) could try to
        // get the authorization code delivered somewhere other than a listener
        // on this machine's loopback interface.
        for uri in [
            // A name, not an address: resolvable elsewhere.
            "http://localhost:52341/trace-commons/native-auth/callback",
            // The other loopback spelling; one canonical form only.
            "http://[::1]:52341/trace-commons/native-auth/callback",
            // Any non-loopback host.
            "http://127.0.0.2:52341/trace-commons/native-auth/callback",
            "http://192.168.1.5:52341/trace-commons/native-auth/callback",
            "http://evil.example.com:52341/trace-commons/native-auth/callback",
            // userinfo trick: the real host is evil.example.com.
            "http://127.0.0.1:52341@evil.example.com/trace-commons/native-auth/callback",
            // Wrong scheme.
            "https://127.0.0.1:52341/trace-commons/native-auth/callback",
            "HTTP://127.0.0.1:52341/trace-commons/native-auth/callback",
            // Wrong or wildcarded path.
            "http://127.0.0.1:52341/",
            "http://127.0.0.1:52341/trace-commons/native-auth/callback/",
            "http://127.0.0.1:52341/anything-else",
            "http://127.0.0.1:52341/trace-commons/native-auth/callback/../../x",
            // Extra query or fragment the app did not register.
            "http://127.0.0.1:52341/trace-commons/native-auth/callback?x=1",
            "http://127.0.0.1:52341/trace-commons/native-auth/callback#x",
            // Missing, privileged, non-numeric, or ambiguous ports.
            "http://127.0.0.1/trace-commons/native-auth/callback",
            "http://127.0.0.1:80/trace-commons/native-auth/callback",
            "http://127.0.0.1:052341/trace-commons/native-auth/callback",
            "http://127.0.0.1:+52341/trace-commons/native-auth/callback",
            "http://127.0.0.1:99999/trace-commons/native-auth/callback",
            "http://127.0.0.1:abc/trace-commons/native-auth/callback",
            "",
        ] {
            assert!(
                validate_loopback_redirect_uri(uri).is_none(),
                "must reject {uri}"
            );
        }
    }

    #[test]
    fn verifier_shape_follows_rfc7636() {
        assert!(verifier_is_wellformed(&"a".repeat(43)));
        assert!(verifier_is_wellformed(&"a".repeat(128)));
        assert!(verifier_is_wellformed(&format!("{}-._~", "a".repeat(39))));
        assert!(!verifier_is_wellformed(&"a".repeat(42)));
        assert!(!verifier_is_wellformed(&"a".repeat(129)));
        assert!(!verifier_is_wellformed(&format!("{}/+=", "a".repeat(40))));
        assert!(!verifier_is_wellformed(""));
    }

    #[test]
    fn challenge_shape_is_pinned_to_a_base64url_sha256() {
        let challenge = challenge_for_verifier(&"a".repeat(43));
        assert_eq!(challenge.len(), CHALLENGE_LEN);
        assert!(challenge_is_wellformed(&challenge));
        assert!(!challenge_is_wellformed("short"));
        assert!(!challenge_is_wellformed(&"a".repeat(44)));
        // Padded / standard-alphabet base64 is not the encoding we pinned.
        assert!(!challenge_is_wellformed(&format!("{}+", "a".repeat(42))));
    }

    #[test]
    fn challenge_derivation_matches_rfc7636_s256() {
        // The RFC 7636 appendix B worked example, so this cannot drift into
        // "whatever our own code happens to compute".
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_for_verifier(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_different_verifier_never_matches_the_challenge() {
        let challenge = challenge_for_verifier(&"a".repeat(43));
        assert!(!secret_eq(
            &challenge,
            &challenge_for_verifier(&"b".repeat(43))
        ));
        assert!(secret_eq(
            &challenge,
            &challenge_for_verifier(&"a".repeat(43))
        ));
    }

    #[test]
    fn native_token_round_trips_and_rejects_garbage() {
        let token = native_token_value("tenant-abc", "s3cret");
        assert!(is_native_token(&token));
        let (tenant, hash) = native_token_parts(&token).expect("round trip");
        assert_eq!(tenant, "tenant-abc");
        assert_eq!(hash, crate::account_session::hash_secret("s3cret"));
        // The raw secret must not survive parsing.
        assert!(!hash.contains("s3cret"));

        // A device upload claim is not a native token and must not parse as one.
        assert!(!is_native_token("eyJhbGciOiJFZERTQSJ9.x.y"));
        assert!(native_token_parts("eyJhbGciOiJFZERTQSJ9.x.y").is_none());
        // Malformed native tokens fail closed.
        assert!(native_token_parts("tcn1_nodot").is_none());
        assert!(native_token_parts("tcn1_dGVuYW50.").is_none());
        assert!(native_token_parts("tcn1_!!!.secret").is_none());
    }
}
