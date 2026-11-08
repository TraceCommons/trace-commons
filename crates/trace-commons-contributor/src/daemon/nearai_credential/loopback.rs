//! Native OAuth callback: receive a one-time code bound to app-held PKCE.
//! Tokens and the verifier never pass through the browser. Host validation,
//! per-connection limits and the caller's ceremony deadline bound this listener.

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const CALLBACK_PATH: &str = "/trace-commons/near-ai/callback";
const MAX_BODY_BYTES: usize = 8192;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

pub struct SessionTokens {
    pub access_token: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for SessionTokens {
    /// Presence, never value. Derived `Debug` on a credential type is one
    /// `tracing::debug!` away from putting a session token in a log file.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionTokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .finish()
    }
}

/// Whether `value` has the shape of [`random_state`]'s output: non-empty, at
/// most 64 characters, base64url only. A value a page quotes inside a script
/// or an attribute is refused unless it has this shape, rather than escaped.
/// The one definition every loopback page checks against.
pub(super) fn state_shaped(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A URL-safe random string, used as the ceremony's local state value.
pub fn random_state() -> Result<String> {
    let mut bytes = [0; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// One attempt's secret. Deliberately not Debug or Clone: a verifier never
/// belongs in the browser, persisted configuration, or diagnostics.
pub struct Pkce {
    verifier: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Result<Self> {
        Ok(Self {
            verifier: random_state()?,
            state: random_state()?,
        })
    }

    pub fn challenge(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::digest::digest(&ring::digest::SHA256, self.verifier.as_bytes()).as_ref())
    }

    pub(super) fn verifier(&self) -> &str {
        &self.verifier
    }
}

pub(super) fn callback_url(port: u16) -> Result<String> {
    if port == 0 {
        bail!("near_ai_credential_callback_invalid")
    }
    Ok(format!("http://127.0.0.1:{port}{CALLBACK_PATH}"))
}

/// None is a matching-state provider refusal. Invalid or unsolicited requests
/// are ignored without consuming the attempt; no legacy token fields are taken.
fn parse_callback(target: &str, state: &str) -> Result<Option<String>> {
    if target.len() > 4096 || target.contains('#') {
        bail!("near_ai_credential_callback_invalid")
    }
    let query = target
        .strip_prefix(CALLBACK_PATH)
        .and_then(|suffix| suffix.strip_prefix('?'))
        .ok_or_else(|| anyhow!("near_ai_credential_callback_invalid"))?;
    let mut seen = std::collections::HashMap::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if !matches!(key.as_ref(), "code" | "state" | "error")
            || seen.insert(key.into_owned(), value.into_owned()).is_some()
        {
            bail!("near_ai_credential_callback_invalid")
        }
    }
    if seen.get("state").map(String::as_str) != Some(state) {
        bail!("near_ai_credential_callback_invalid")
    }
    match (seen.remove("code"), seen.remove("error")) {
        (Some(code), None)
            if !code.is_empty()
                && code.len() <= 1024
                && code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)) =>
        {
            Ok(Some(code))
        }
        (None, Some(error)) if !error.is_empty() && error.len() <= 128 => Ok(None),
        _ => bail!("near_ai_credential_callback_invalid"),
    }
}

/// Only fixed copy enters this page; callback fields and errors are never
/// reflected, and no script is needed to complete a verifier-bound exchange.
fn result_page(success: bool) -> String {
    include_str!("sign_in_result.html")
        .replace("__RESULT__", if success { "success" } else { "error" })
        .replace(
            "__HEADING__",
            if success {
                "You're signed in"
            } else {
                "Sign-in couldn't finish"
            },
        )
        .replace(
            "__MESSAGE__",
            if success {
                "Your NEAR AI account is connected. Return to __APP_NAME__ to continue."
            } else {
                "Return to __APP_NAME__ and start sign-in again."
            },
        )
        .replace("__APP_NAME__", crate::brand::APP_NAME)
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    respond_with_type(stream, status, "text/plain; charset=utf-8", body).await;
}

async fn respond_result(stream: &mut tokio::net::TcpStream, status: &str, success: bool) {
    respond_with_type(
        stream,
        status,
        "text/html; charset=utf-8",
        &result_page(success),
    )
    .await;
}

async fn respond_with_type(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    kind: &str,
    body: &str,
) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(CONNECTION_TIMEOUT, stream.write_all(response.as_bytes())).await;
}

pub(super) async fn receive_session_after<T, F, Fut>(
    listener: tokio::net::TcpListener,
    state: &str,
    stale: Option<&super::chooser::StaleChoice>,
    finish: F,
) -> Result<T>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    if !state_shaped(state) {
        bail!("near_ai_credential_callback_invalid")
    }
    let authority = format!("127.0.0.1:{}", listener.local_addr()?.port());
    loop {
        let (mut stream, _) = listener.accept().await?;
        let request = tokio::time::timeout(
            CONNECTION_TIMEOUT,
            super::loopback_http::read_request(&mut stream, &authority, MAX_BODY_BYTES),
        )
        .await;
        let Ok(Ok((head, body))) = request else {
            respond(
                &mut stream,
                "400 Bad Request",
                "This callback was not accepted.",
            )
            .await;
            continue;
        };
        if stale.is_some_and(|stale| stale.matches(&head, &body, &authority)) {
            respond(
                &mut stream,
                "400 Bad Request",
                super::chooser::STALE_CHOICE_BODY,
            )
            .await;
            bail!(super::chooser::STALE_CHOICE)
        }
        if head.method != "GET" || !body.is_empty() || head.speculative() {
            respond(
                &mut stream,
                "400 Bad Request",
                "This callback was not accepted.",
            )
            .await;
            continue;
        }
        match parse_callback(&head.target, state) {
            Ok(Some(code)) => {
                // Acknowledge only after exchange, mint and durable replacement.
                // Exactly one matching callback can invoke this FnOnce.
                let outcome = finish(code).await;
                respond_result(
                    &mut stream,
                    if outcome.is_ok() {
                        "200 OK"
                    } else {
                        "500 Internal Server Error"
                    },
                    outcome.is_ok(),
                )
                .await;
                return outcome;
            }
            Ok(None) => {
                respond_result(&mut stream, "400 Bad Request", false).await;
                bail!("near_ai_credential_refused")
            }
            Err(_) => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "This callback was not accepted.",
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn pkce_matches_rfc7636_and_attempts_are_independent() {
        let pkce = Pkce {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            state: "state".into(),
        };
        assert_eq!(
            pkce.challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let first = Pkce::new().unwrap();
        let second = Pkce::new().unwrap();
        assert_eq!(first.verifier().len(), 43);
        assert_ne!(first.verifier(), first.state);
        assert_ne!(first.verifier(), second.verifier());
        assert_ne!(first.state, second.state);
    }

    #[test]
    fn callbacks_require_unambiguous_code_and_matching_state() {
        assert_eq!(
            parse_callback(
                &format!("{CALLBACK_PATH}?code=one-time&state=state"),
                "state"
            )
            .unwrap(),
            Some("one-time".into())
        );
        assert!(
            parse_callback(
                &format!("{CALLBACK_PATH}?error=denied&state=state"),
                "state"
            )
            .unwrap()
            .is_none()
        );
        for suffix in [
            "",
            "?code=c",
            "?code=c&state=wrong",
            "?code=c&state=state&state=state",
            "?code=c&state=state&code=c",
            "?code=c&error=denied&state=state",
            "?token=jwt&refresh_token=rt&state=state",
            "?code=c&state=state&extra=x",
            "?error=denied",
            "?code=&state=state",
            "?code=c&state=state#token=jwt",
            "/other?code=c&state=state",
        ] {
            assert!(
                parse_callback(&format!("{CALLBACK_PATH}{suffix}"), "state").is_err(),
                "{suffix}"
            );
        }
    }

    async fn send(address: std::net::SocketAddr, method: &str, target: &str, host: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn unsolicited_and_legacy_callbacks_do_not_consume_the_attempt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            receive_session_after(listener, "state", None, |code| async move { Ok(code) }).await
        });
        for (method, suffix, host) in [
            ("GET", "", address.to_string()),
            ("POST", "?code=c&state=state", address.to_string()),
            ("GET", "?code=c&state=wrong", address.to_string()),
            (
                "GET",
                "?token=jwt&refresh_token=rt&state=state",
                address.to_string(),
            ),
            ("GET", "?code=c&state=state", "evil.example".into()),
        ] {
            let response = send(address, method, &format!("{CALLBACK_PATH}{suffix}"), &host).await;
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
            assert!(!task.is_finished());
        }
        let response = send(
            address,
            "GET",
            &format!("{CALLBACK_PATH}?code=secret-code&state=state"),
            &address.to_string(),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(!response.contains("secret-code"));
        assert!(!response.contains("state=state"));
        assert_eq!(task.await.unwrap().unwrap(), "secret-code");
    }

    #[tokio::test]
    async fn browser_success_waits_for_credential_setup_and_failures_are_fixed() {
        for succeeds in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (start_tx, start_rx) = tokio::sync::oneshot::channel();
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                receive_session_after(listener, "state", None, |_| async move {
                    start_tx.send(()).unwrap();
                    done_rx.await.unwrap();
                    if succeeds {
                        Ok(())
                    } else {
                        Err(anyhow!("synthetic-secret-error"))
                    }
                })
                .await
            });
            let mut browser = tokio::spawn(async move {
                send(
                    address,
                    "GET",
                    &format!("{CALLBACK_PATH}?code=c&state=state"),
                    &address.to_string(),
                )
                .await
            });
            start_rx.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(30), &mut browser)
                    .await
                    .is_err()
            );
            done_tx.send(()).unwrap();
            let response = browser.await.unwrap();
            assert!(response.starts_with(if succeeds {
                "HTTP/1.1 200"
            } else {
                "HTTP/1.1 500"
            }));
            assert!(!response.contains("synthetic-secret-error"));
            assert!(response.contains("Content-Type: text/html; charset=utf-8"));
            assert!(response.contains("Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'"));
            assert!(response.contains("Cache-Control: no-store"));
            assert!(response.contains("Referrer-Policy: no-referrer"));
            assert!(response.contains("aria-label=\"NEAR AI\""));
            assert!(response.contains(if succeeds {
                "You're signed in"
            } else {
                "Sign-in couldn't finish"
            }));
            assert!(response.contains("You can close this tab."));
            assert!(!response.contains("<script"));
            assert!(!response.contains("location.hash"));
            assert!(!response.contains("__APP_NAME__"));
            assert_eq!(task.await.unwrap().is_ok(), succeeds);
        }
    }
}
