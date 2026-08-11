//! Account sign-in for a native client: the loopback browser flow, and the
//! short-lived bearer token it yields.
//!
//! # Why the browser
//!
//! `/v1/account/*` — trace read-back and, the reason this exists, withdrawal —
//! is guarded by a browser session. This daemon holds a device key, and the
//! device key deliberately gains NO new authority here: it can already upload,
//! and it is not turned into a way to withdraw traces or read account history.
//! Instead the human completes the SAME login-link flow they would complete in
//! a browser anyway, and the browser hands this process a one-time code on a
//! loopback redirect, which it exchanges for a token of its own.
//!
//! # Why the loopback redirect is not enough on its own
//!
//! Any local process can bind a loopback port, or read a code out of browser
//! history. So the code is bound to THIS process by PKCE: a high-entropy
//! verifier is generated here, only its sha256 is sent when the flow starts,
//! and the verifier must be presented at exchange. A code intercepted by
//! another local process is useless without it. The listener binds 127.0.0.1
//! only (never 0.0.0.0), serves exactly one request, and is dropped
//! immediately afterwards.
//!
//! # The token is a secret at rest
//!
//! It is written to the same 0700 state directory as the device key, at 0600,
//! through the same atomic writer. It appears in no log line and no error
//! string: every error below is a fixed label, like every other boundary in
//! this crate.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use trace_commons_operator_client::Client;

use crate::config::{ACCOUNT_SESSION_FILE, ConfigStore, ContributorConfig, allowlist_for};

/// The loopback path the server accepts, and the ONLY one it accepts. Pinned
/// here and cross-checked against the server's own constant by
/// `constants_match_the_server` below.
pub const NATIVE_REDIRECT_PATH: &str = "/trace-commons/native-auth/callback";

/// PKCE method. The server refuses `plain`.
const CODE_CHALLENGE_METHOD: &str = "S256";

/// How long to wait for the human to finish in their browser. Matches the
/// server's pending-request TTL, so the listener never outlives the flow it is
/// serving.
const LOOPBACK_WAIT: Duration = Duration::from_secs(5 * 60);

/// Cap on the bytes read from the loopback connection before giving up. A
/// browser's GET of a short callback URL is well under this; the cap stops a
/// hostile local process from streaming forever into this process.
const LOOPBACK_MAX_REQUEST_BYTES: usize = 8 * 1024;

/// Refresh the token when it is within this of expiry, rather than letting a
/// long operation start with a token that dies mid-flight.
const EXPIRY_SKEW: chrono::TimeDelta = chrono::TimeDelta::minutes(2);

/// The stored account session. `access_token` is a SECRET; this struct is
/// deliberately not `Debug`, so it cannot be accidentally formatted into a log
/// line or an error.
#[derive(Clone, Serialize, Deserialize)]
pub struct AccountSession {
    pub access_token: String,
    pub expires_at: DateTime<Utc>,
    pub account_id: String,
}

/// What a completed sign-in reports to a caller. Carries no token.
#[derive(Debug, Clone)]
pub struct SignInOutcome {
    pub account_id: String,
    pub expires_at: DateTime<Utc>,
}

/// Load the stored token, if there is one and it is still usable.
///
/// Returns `None` — never a stale token and never an error — when the file is
/// absent, unparseable, or expired (or about to be). Fail closed: the caller
/// then reports "sign in again" rather than making a call that will 401.
pub fn load_token(store: &ConfigStore) -> Option<String> {
    let raw = store
        .read_daemon_file(ACCOUNT_SESSION_FILE)
        .ok()
        .flatten()?;
    let session: AccountSession = serde_json::from_slice(&raw).ok()?;
    if session.expires_at <= Utc::now() + EXPIRY_SKEW {
        return None;
    }
    Some(session.access_token)
}

/// Whether a usable token is stored, without handing the token out. For status
/// output, which must never print the secret.
pub fn session_status(store: &ConfigStore) -> Option<DateTime<Utc>> {
    let raw = store
        .read_daemon_file(ACCOUNT_SESSION_FILE)
        .ok()
        .flatten()?;
    let session: AccountSession = serde_json::from_slice(&raw).ok()?;
    (session.expires_at > Utc::now()).then_some(session.expires_at)
}

/// Persist a token at 0600 inside the 0700 state directory.
fn save_session(store: &ConfigStore, session: &AccountSession) -> Result<()> {
    let body = serde_json::to_vec(session).context("serializing the account session")?;
    store.write_daemon_file(ACCOUNT_SESSION_FILE, &body)
}

/// Forget the stored token. Local only: use `sign_out` to also revoke it
/// server-side.
pub fn clear_token(store: &ConfigStore) -> Result<()> {
    store.remove_daemon_file(ACCOUNT_SESSION_FILE)
}

#[derive(Serialize)]
struct AuthorizeStartRequest<'a> {
    code_challenge: &'a str,
    code_challenge_method: &'a str,
    redirect_uri: &'a str,
}

#[derive(Deserialize)]
struct AuthorizeStartResponse {
    request_id: String,
}

#[derive(Serialize)]
struct TokenExchangeRequest<'a> {
    request_id: &'a str,
    code: &'a str,
    code_verifier: &'a str,
}

#[derive(Deserialize)]
struct TokenExchangeResponse {
    access_token: String,
    expires_in_secs: i64,
    account_id: String,
}

/// A 43-character PKCE verifier: 32 CSPRNG bytes, unpadded base64url, which is
/// exactly RFC 7636's minimum length and its unreserved alphabet.
fn generate_verifier() -> Result<String> {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow::anyhow!("could not generate a sign-in verifier"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn challenge_for_verifier(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The two native endpoints take NO credential — one parks a PKCE challenge,
/// the other creates the session — so there is no token to present. The
/// operator client always attaches an `Authorization` header, so this fixed,
/// non-secret placeholder rides along and the server ignores it. Using the
/// shared client is worth that: it brings the host allowlist (which is what
/// keeps this flow pointed at the configured ingest host), the timeout, and
/// the typed error labels.
const UNAUTHENTICATED_PLACEHOLDER: &str = "unauthenticated";

fn native_client(cfg: &ContributorConfig) -> Result<Client> {
    Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(UNAUTHENTICATED_PLACEHOLDER)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .build()
    .context("building the ingest client for native sign-in")
}

/// Bind a loopback listener on an ephemeral port.
///
/// `127.0.0.1` explicitly, NEVER `0.0.0.0`: the redirect must be reachable
/// only from this machine, and the server will refuse any other host anyway.
async fn bind_loopback() -> Result<(tokio::net::TcpListener, u16)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("binding the loopback sign-in listener")?;
    let port = listener
        .local_addr()
        .context("reading the loopback listener address")?
        .port();
    Ok((listener, port))
}

/// Serve exactly one request, extract the `code` query parameter, answer with a
/// small page, and return. The listener is dropped by the caller immediately
/// afterwards, so the port does not stay open.
async fn await_loopback_code(listener: tokio::net::TcpListener) -> Result<String> {
    let accept = tokio::time::timeout(LOOPBACK_WAIT, listener.accept())
        .await
        .map_err(|_| anyhow::anyhow!("sign-in timed out waiting for the browser"))?;
    let (mut stream, _peer) = accept.context("accepting the loopback sign-in callback")?;

    // Read until the end of the request head, or the byte cap.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .context("reading the loopback sign-in callback")?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= LOOPBACK_MAX_REQUEST_BYTES {
            break;
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let target = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string();

    let code = target
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| pair.strip_prefix("code="))
        .map(str::to_string);

    // Answer either way, so the human sees something other than a dead tab.
    let (status, message) = match code.as_deref() {
        Some(_) => ("200 OK", "Signed in. You can close this tab."),
        None => ("400 Bad Request", "Sign-in did not complete."),
    };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Trace Commons</title><p>{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;

    // The path must be the one we registered. The server only ever redirects
    // there, so a callback on any other path is some other local process
    // knocking on this port; refuse it rather than trust its query string.
    let path = target.split('?').next().unwrap_or_default();
    if path != NATIVE_REDIRECT_PATH {
        bail!("sign-in callback arrived on an unexpected path");
    }
    code.ok_or_else(|| anyhow::anyhow!("sign-in callback carried no code"))
}

/// Best-effort: hand the URL to the platform's browser opener. A failure here
/// is not fatal — the caller prints the URL, which is the headless path.
fn try_open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let attempt = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let attempt = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let attempt = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let attempt: std::io::Result<std::process::ExitStatus> =
        Err(std::io::Error::other("no opener"));

    matches!(attempt, Ok(status) if status.success())
}

/// Run the whole loopback sign-in and persist the resulting token.
///
/// `announce` is handed the browser URL. A terminal caller prints it (so a
/// headless machine can complete the flow by pasting it into a browser
/// elsewhere on the same network path); a GUI caller can show it too. The URL
/// carries a single-use login code, so it must be treated as a secret and
/// never logged.
pub async fn sign_in<F>(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    open_browser: bool,
    announce: F,
) -> Result<SignInOutcome>
where
    F: FnOnce(&str),
{
    // Bind BEFORE registering the redirect, so the port we register is the port
    // we are actually listening on.
    let (listener, port) = bind_loopback().await?;
    let redirect_uri = format!("http://127.0.0.1:{port}{NATIVE_REDIRECT_PATH}");

    let verifier = generate_verifier()?;
    let challenge = challenge_for_verifier(&verifier);

    let client = native_client(cfg)?;
    let start: AuthorizeStartResponse = client
        .call_json(
            Method::POST,
            "/v1/account/native/authorize",
            &[],
            Some(&AuthorizeStartRequest {
                code_challenge: &challenge,
                code_challenge_method: CODE_CHALLENGE_METHOD,
                redirect_uri: &redirect_uri,
            }),
        )
        .await
        .context("starting the native sign-in flow")?;

    // The login link itself is minted by the existing device-authenticated
    // endpoint. This is an authority the device key ALREADY has; the flow adds
    // none.
    let login_path = crate::submit::mint_account_login_link(store, cfg).await?;
    let separator = if login_path.contains('?') { '&' } else { '?' };
    let browser_url = format!(
        "{}{login_path}{separator}native={}",
        cfg.ingest_url.trim_end_matches('/'),
        start.request_id
    );

    if open_browser {
        try_open_browser(&browser_url);
    }
    announce(&browser_url);

    let code = await_loopback_code(listener).await?;

    let exchanged: TokenExchangeResponse = client
        .call_json(
            Method::POST,
            "/v1/account/native/token",
            &[],
            Some(&TokenExchangeRequest {
                request_id: &start.request_id,
                code: &code,
                code_verifier: &verifier,
            }),
        )
        .await
        .context("exchanging the sign-in code")?;

    let expires_at = Utc::now() + chrono::TimeDelta::seconds(exchanged.expires_in_secs);
    let session = AccountSession {
        access_token: exchanged.access_token,
        expires_at,
        account_id: exchanged.account_id.clone(),
    };
    save_session(store, &session)?;

    Ok(SignInOutcome {
        account_id: exchanged.account_id,
        expires_at,
    })
}

/// Revoke the stored token server-side, then forget it locally.
///
/// The local file is removed even when the server call fails: a caller that
/// asked to sign out must not be left holding a live token on disk because the
/// network was down. The token expires on its own regardless.
pub async fn sign_out(store: &ConfigStore, cfg: &ContributorConfig) -> Result<()> {
    let token = load_token(store);
    let result = match token {
        Some(token) => {
            let client = Client::builder(
                &cfg.ingest_url,
                "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
            )
            .bearer_token(token)
            .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
            .build()
            .context("building the ingest client for sign-out")?;
            client
                .call_raw::<()>(Method::POST, "/v1/account/logout", &[], None)
                .await
                .map(|_| ())
                .context("revoking the account session")
        }
        None => Ok(()),
    };
    clear_token(store)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client's copy of the wire constants must equal the server's. Both
    /// are in this workspace, and `trace-commons-server` is a dev-dependency
    /// here, so this is a real cross-check rather than two hand-maintained
    /// copies that can drift.
    #[test]
    fn constants_match_the_server() {
        use trace_commons_server::account_native_auth as server;
        assert_eq!(NATIVE_REDIRECT_PATH, server::NATIVE_REDIRECT_PATH);
        assert_eq!(CODE_CHALLENGE_METHOD, server::NATIVE_CODE_CHALLENGE_METHOD);
        assert_eq!(
            LOOPBACK_WAIT.as_secs(),
            server::NATIVE_AUTH_REQUEST_TTL.as_secs(),
            "the listener must not outlive the server's pending request"
        );
    }

    /// The verifier this client generates must satisfy the server's own
    /// well-formedness check, and the challenge must be the one the server
    /// derives.
    #[test]
    fn generated_pkce_satisfies_the_server() {
        use trace_commons_server::account_native_auth as server;
        let verifier = generate_verifier().expect("verifier");
        assert!(server::verifier_is_wellformed(&verifier));
        let challenge = challenge_for_verifier(&verifier);
        assert!(server::challenge_is_wellformed(&challenge));
        assert_eq!(challenge, server::challenge_for_verifier(&verifier));
    }

    /// The redirect this client registers must be one the server accepts.
    #[test]
    fn the_registered_redirect_is_one_the_server_accepts() {
        use trace_commons_server::account_native_auth as server;
        let uri = format!("http://127.0.0.1:49152{NATIVE_REDIRECT_PATH}");
        assert!(server::validate_loopback_redirect_uri(&uri).is_some());
    }

    #[tokio::test]
    async fn the_loopback_listener_binds_only_to_127_0_0_1() {
        let (listener, port) = bind_loopback().await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        assert_eq!(
            addr.ip().to_string(),
            "127.0.0.1",
            "the listener must never be reachable off this machine"
        );
        assert!(port >= 1024);
    }

    #[tokio::test]
    async fn the_listener_serves_one_request_and_then_the_port_is_closed() {
        let (listener, port) = bind_loopback().await.expect("bind");
        let handle = tokio::spawn(await_loopback_code(listener));

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream
            .write_all(
                format!("GET {NATIVE_REDIRECT_PATH}?code=abc123&request_id=r HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("read");
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));

        let code = handle.await.expect("join").expect("code");
        assert_eq!(code, "abc123");

        // The listener was dropped when `await_loopback_code` returned, so the
        // port must no longer accept.
        let second = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
        assert!(
            second.is_err(),
            "the loopback port must not stay open after the exchange"
        );
    }

    #[tokio::test]
    async fn a_callback_on_another_path_is_refused() {
        // Some other local process knocking on the port must not be able to
        // feed this client a code.
        let (listener, port) = bind_loopback().await.expect("bind");
        let handle = tokio::spawn(await_loopback_code(listener));

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream
            .write_all(b"GET /something-else?code=attacker HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .expect("write");
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;

        let result = handle.await.expect("join");
        assert!(
            result.is_err(),
            "a callback on another path must be refused"
        );
    }

    #[test]
    fn an_expired_stored_token_is_not_offered() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let session = AccountSession {
            access_token: "tcn1_dGVuYW50.secret".to_string(),
            expires_at: Utc::now() - chrono::TimeDelta::minutes(1),
            account_id: "acct".to_string(),
        };
        save_session(&store, &session).expect("save");
        assert!(
            load_token(&store).is_none(),
            "an expired token must fail closed, not be presented and 401"
        );
        assert!(session_status(&store).is_none());
    }

    #[test]
    fn a_token_about_to_expire_is_not_offered() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let session = AccountSession {
            access_token: "tcn1_dGVuYW50.secret".to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::seconds(30),
            account_id: "acct".to_string(),
        };
        save_session(&store, &session).expect("save");
        assert!(load_token(&store).is_none());
    }

    #[test]
    fn a_live_token_is_offered_and_stored_at_0600() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let session = AccountSession {
            access_token: "tcn1_dGVuYW50.secret".to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        save_session(&store, &session).expect("save");
        assert_eq!(load_token(&store).as_deref(), Some("tcn1_dGVuYW50.secret"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.daemon_path(ACCOUNT_SESSION_FILE))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the token is a secret at rest");
        }

        clear_token(&store).expect("clear");
        assert!(load_token(&store).is_none());
    }
}
