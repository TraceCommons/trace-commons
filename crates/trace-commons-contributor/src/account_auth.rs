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
//! It is stored in the OS credential store. The 0700 state directory holds
//! only an opaque reference, replaced after readback verification. It appears in no log line and no error
//! string: every error below is a fixed label, like every other boundary in
//! this crate.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use trace_commons_operator_client::Client;

use crate::config::{ACCOUNT_SESSION_FILE, ConfigStore, ContributorConfig, config_allowlist};

#[cfg(feature = "test-credential-store")]
#[doc(hidden)]
pub mod wire;
#[cfg(not(feature = "test-credential-store"))]
mod wire;

pub use wire::NATIVE_REDIRECT_PATH;
use wire::{CODE_CHALLENGE_METHOD, LOOPBACK_WAIT, challenge_for_verifier, generate_verifier};

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

#[derive(Clone)]
pub(crate) struct LoadedAccountSession {
    pub session: AccountSession,
    pub snapshot: crate::daemon::commons_credentials::Snapshot,
    raw: Vec<u8>,
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
    try_load_token(store).ok().flatten()
}

/// Distinguish an unavailable OS store from a signed-out account. Callers may
/// ask the user to unlock the system store without starting a new login flow.
pub fn try_load_token(store: &ConfigStore) -> Result<Option<String>> {
    Ok(try_load_session_with_snapshot(store)?.map(|loaded| loaded.session.access_token))
}

pub(crate) fn try_load_session_with_snapshot(
    store: &ConfigStore,
) -> Result<Option<LoadedAccountSession>> {
    let Some((raw, snapshot)) = crate::daemon::commons_credentials::load_with_snapshot(
        store,
        crate::daemon::commons_credentials::Kind::Account,
    )?
    else {
        return Ok(None);
    };
    let Ok(session) = serde_json::from_slice::<AccountSession>(&raw) else {
        return Ok(None);
    };
    if !session_is_usable(&session) {
        return Ok(None);
    }
    Ok(Some(LoadedAccountSession {
        session,
        snapshot,
        raw,
    }))
}

/// Whether a session is still worth presenting: not expired, and not about
/// to be. The same test [`try_load_session_with_snapshot`] applies on load,
/// for a caller that holds a loaded session across time.
pub(crate) fn session_is_usable(session: &AccountSession) -> bool {
    session_is_usable_at(session, Utc::now())
}

fn session_is_usable_at(session: &AccountSession, now: DateTime<Utc>) -> bool {
    session.expires_at > now + EXPIRY_SKEW
}

/// A session rotation the server handed back: the new bearer token and, from
/// a server that extends a session when it rotates it, the session's new
/// expiry. `token` is a SECRET; like [`AccountSession`] this is deliberately
/// not `Debug`.
#[derive(Clone)]
pub struct RotatedToken {
    pub token: String,
    /// `None` from a server that does not send
    /// [`trace_commons_protocol::ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER`]:
    /// the stored expiry is kept, which is what such a server enforces.
    pub expires_at: Option<DateTime<Utc>>,
}

impl RotatedToken {
    /// Build from the two response headers. No token, no rotation. An expiry
    /// that is not RFC 3339 is dropped rather than refusing the token: the
    /// old token stops working shortly after a rotation, so losing the new
    /// one signs the contributor out, while losing the expiry only keeps the
    /// earlier one.
    pub fn from_headers(token: Option<String>, expires_at: Option<String>) -> Option<Self> {
        let token = token?;
        let expires_at = expires_at
            .and_then(|raw| DateTime::parse_from_rfc3339(raw.trim()).ok())
            .map(|at| at.with_timezone(&Utc));
        Some(Self { token, expires_at })
    }
}

/// A token alone: what a server that does not extend sessions hands back.
impl From<String> for RotatedToken {
    fn from(token: String) -> Self {
        Self {
            token,
            expires_at: None,
        }
    }
}

impl From<&str> for RotatedToken {
    fn from(token: &str) -> Self {
        token.to_string().into()
    }
}

pub(crate) fn store_rotated_token(
    store: &ConfigStore,
    loaded: &LoadedAccountSession,
    rotated: RotatedToken,
) -> Result<()> {
    let RotatedToken {
        token: rotated_token,
        expires_at,
    } = rotated;
    if rotated_token.trim().is_empty() || rotated_token.trim() != rotated_token {
        bail!("account_session_rotation_invalid");
    }
    // Preserve pre-enrollment origin/binding metadata and future extension
    // fields while rotating only the bearer secret and, when the server sent
    // one, the expiry it moved with it.
    let mut payload: serde_json::Value = serde_json::from_slice(&loaded.raw)?;
    payload["access_token"] = serde_json::Value::String(rotated_token);
    if let Some(expires_at) = expires_at {
        payload["expires_at"] = serde_json::to_value(expires_at)
            .context("serializing the rotated account session expiry")?;
    }
    let body = serde_json::to_vec(&payload).context("serializing the rotated account session")?;
    crate::daemon::commons_credentials::replace_account_rotation(store, &loaded.snapshot, &body)
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

/// Persist a token through the shared Commons credential lifecycle.
#[cfg(test)]
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
    .max_response_bytes(256 * 1024)
    .host_allowlist(config_allowlist(cfg))
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
        "<!doctype html><meta charset=\"utf-8\"><title>{app}</title><p>{message}</p>",
        app = crate::brand::APP_NAME
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
pub(crate) fn try_open_browser(url: &str) -> bool {
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
    let expected = crate::daemon::commons_credentials::account_snapshot(store, cfg)?;
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
    let browser_url = browser_url(&cfg.ingest_url, &login_path, &start.request_id)?;

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
    let owned_store = store.clone();
    tokio::task::spawn_blocking(move || {
        crate::daemon::commons_credentials::replace(
            &owned_store,
            &expected,
            &serde_json::to_vec(&session)?,
            None,
        )
    })
    .await
    .context("commons_credential_worker_unavailable")??;

    Ok(SignInOutcome {
        account_id: exchanged.account_id,
        expires_at,
    })
}

/// The URL the human opens: the server's root-relative login path on the
/// ingest ORIGIN, plus this flow's `native` request id.
///
/// Not `ingest_url + login_path`: `ingest_url` is the upload endpoint and
/// carries `/v1/traces`, which is how 0.12.6 printed a login URL that 404s.
fn browser_url(ingest_url: &str, login_path: &str, request_id: &str) -> Result<String> {
    let mut url = crate::config::ingest_origin_url(ingest_url, login_path)
        .context("building the sign-in URL")?;
    url.query_pairs_mut().append_pair("native", request_id);
    Ok(url.into())
}

/// Revoke the stored token server-side, then forget it locally.
///
/// The local file is removed even when the server call fails: a caller that
/// asked to sign out must not be left holding a live token on disk because the
/// network was down. The token expires on its own regardless.
pub async fn sign_out(store: &ConfigStore, cfg: &ContributorConfig) -> Result<()> {
    let expected = crate::daemon::commons_credentials::account_snapshot(store, cfg)?;
    crate::daemon::commons_credentials::clear_expected(store, &expected)?;
    let owned_store = store.clone();
    let token = tokio::task::spawn_blocking(move || {
        let raw = crate::daemon::commons_credentials::removed_payload(&owned_store, &expected)
            .ok()
            .flatten();
        let token = raw
            .and_then(|bytes| serde_json::from_slice::<AccountSession>(&bytes).ok())
            .map(|session| session.access_token);
        let _ = crate::daemon::commons_credentials::cleanup(&owned_store);
        token
    })
    .await
    .context("commons_credential_worker_unavailable")?;
    match token {
        Some(token) => {
            let client = Client::builder(
                &cfg.ingest_url,
                "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
            )
            .bearer_token(token)
            .host_allowlist(config_allowlist(cfg))
            .build()
            .context("building the ingest client for sign-out")?;
            client
                .call_raw::<()>(Method::POST, "/v1/account/logout", &[], None)
                .await
                .map(|_| ())
                .context("revoking the account session")
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ingest_url` is the upload endpoint and carries a path (`/v1/traces`),
    /// so the browser URL is built on its origin. 0.12.6 appended the login
    /// path to the whole string and printed `/v1/traces/account/login`, a 404.
    #[test]
    fn the_browser_url_is_built_on_the_ingest_origin_not_its_path() {
        for ingest in [
            "https://commons.example/v1/traces",
            "https://commons.example/v1/traces/",
            "https://commons.example/",
            "https://commons.example",
        ] {
            assert_eq!(
                browser_url(ingest, "/account/login?code=abc", "req-1").unwrap(),
                "https://commons.example/account/login?code=abc&native=req-1",
                "ingest_url = {ingest}"
            );
        }
    }

    #[test]
    fn the_browser_url_keeps_the_ingest_port_and_scheme() {
        for ingest in [
            "http://127.0.0.1:8443/v1/traces",
            "http://127.0.0.1:8443/v1/traces/",
            "http://127.0.0.1:8443",
        ] {
            assert_eq!(
                browser_url(ingest, "/account/login", "req-1").unwrap(),
                "http://127.0.0.1:8443/account/login?native=req-1",
                "ingest_url = {ingest}"
            );
        }
    }

    /// The login path comes from the server. Only a root-relative path is
    /// joined: anything that could name another host is refused rather than
    /// handed to the browser.
    #[test]
    fn a_login_path_that_could_leave_the_ingest_origin_is_refused() {
        for login_path in [
            "https://elsewhere.example/account/login?code=abc",
            "//elsewhere.example/account/login?code=abc",
            "/\\elsewhere.example/account/login?code=abc",
            "account/login?code=abc",
            "",
        ] {
            assert!(
                browser_url("https://commons.example/v1/traces", login_path, "req-1").is_err(),
                "login_path = {login_path:?}"
            );
        }
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

    fn live_session(token: &str) -> AccountSession {
        AccountSession {
            access_token: token.to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "synthetic-account".to_string(),
        }
    }

    #[test]
    fn a_server_rotation_replaces_the_os_stored_account_token() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        save_session(&store, &live_session("synthetic-token-before")).expect("save session");
        let loaded = try_load_session_with_snapshot(&store)
            .expect("load account session")
            .expect("account session exists");
        store_rotated_token(&store, &loaded, "synthetic-token-after".to_string().into())
            .expect("store rotated token");
        assert_eq!(
            try_load_token(&store).expect("reload token").as_deref(),
            Some("synthetic-token-after")
        );
    }

    /// A server that extends the session on rotation sends the new expiry
    /// beside the token; both are stored, so the session stays usable past
    /// the expiry it was signed in with.
    #[test]
    fn a_rotation_with_an_expiry_extends_the_stored_session() {
        use chrono::SubsecRound;
        let (_dir, store) = crate::config::tests_support::temp_store();
        let signed_in = Utc::now() + chrono::TimeDelta::hours(1);
        let mut session = live_session("synthetic-token-before");
        session.expires_at = signed_in;
        save_session(&store, &session).expect("save session");
        let loaded = try_load_session_with_snapshot(&store)
            .expect("load account session")
            .expect("account session exists");
        let extended = (Utc::now() + chrono::TimeDelta::hours(12)).trunc_subsecs(0);
        let rotated = RotatedToken::from_headers(
            Some("synthetic-token-after".to_string()),
            Some(extended.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        )
        .expect("a rotation");
        store_rotated_token(&store, &loaded, rotated).expect("store rotated token");
        let reloaded = try_load_session_with_snapshot(&store)
            .expect("reload")
            .expect("still signed in");
        assert_eq!(reloaded.session.access_token, "synthetic-token-after");
        assert_eq!(reloaded.session.expires_at, extended);
        let later = signed_in + chrono::TimeDelta::hours(2);
        assert!(!session_is_usable_at(&session, later));
        assert!(
            session_is_usable_at(&reloaded.session, later),
            "usable past the expiry it was signed in with"
        );
        // The rest of the record is kept.
        assert_eq!(reloaded.session.account_id, "synthetic-account");
    }

    /// An older server sends no expiry: the token rotates and the expiry is
    /// the one already stored, as before.
    #[test]
    fn a_rotation_without_an_expiry_keeps_the_stored_one() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let session = live_session("synthetic-token-before");
        save_session(&store, &session).expect("save session");
        let loaded = try_load_session_with_snapshot(&store)
            .expect("load account session")
            .expect("account session exists");
        let rotated = RotatedToken::from_headers(Some("synthetic-token-after".to_string()), None)
            .expect("a rotation");
        assert_eq!(rotated.expires_at, None);
        store_rotated_token(&store, &loaded, rotated).expect("store rotated token");
        let reloaded = try_load_session_with_snapshot(&store)
            .expect("reload")
            .expect("still signed in");
        assert_eq!(reloaded.session.access_token, "synthetic-token-after");
        assert_eq!(reloaded.session.expires_at, session.expires_at);
    }

    /// An expiry that does not parse is dropped and the token is still kept:
    /// losing the token would sign the contributor out, losing the expiry
    /// only means the stored one is used.
    #[test]
    fn an_unparseable_expiry_never_costs_the_token() {
        for raw in ["", "tomorrow", "2026-13-40T00:00:00Z", "1761264000"] {
            let rotated =
                RotatedToken::from_headers(Some("synthetic-token".to_string()), Some(raw.into()))
                    .expect("the token is kept");
            assert_eq!(rotated.token, "synthetic-token");
            assert_eq!(rotated.expires_at, None, "{raw:?}");
        }
        assert!(RotatedToken::from_headers(None, Some("2026-11-01T00:00:00Z".into())).is_none());
    }

    #[test]
    fn a_rotation_cannot_restore_a_concurrently_signed_out_session() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        save_session(&store, &live_session("synthetic-token-before")).expect("save session");
        let loaded = try_load_session_with_snapshot(&store)
            .expect("load account session")
            .expect("account session exists");
        clear_token(&store).expect("sign out locally");
        assert!(
            store_rotated_token(&store, &loaded, "synthetic-token-after".to_string().into())
                .is_err()
        );
        assert!(
            try_load_token(&store)
                .expect("reload signed-out state")
                .is_none()
        );
    }

    #[test]
    fn a_concurrent_rotation_keeps_the_first_newer_token() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        save_session(&store, &live_session("synthetic-token-before")).expect("save session");
        let first = try_load_session_with_snapshot(&store)
            .expect("load first account session")
            .expect("first account session exists");
        let second = try_load_session_with_snapshot(&store)
            .expect("load second account session")
            .expect("second account session exists");
        store_rotated_token(&store, &first, "synthetic-token-newer".to_string().into())
            .expect("store first rotation");
        store_rotated_token(&store, &second, "synthetic-token-late".to_string().into())
            .expect("accept concurrent rotation without rollback");
        assert_eq!(
            try_load_token(&store).expect("reload token").as_deref(),
            Some("synthetic-token-newer")
        );
    }
}
