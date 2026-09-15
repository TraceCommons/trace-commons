//! The loopback half of NEAR AI sign-in: a listener that survives the fact
//! that the tokens never reach it directly.
//!
//! cloud-api's OAuth finish is shaped for a single-page web app. It redirects
//! to the `frontend_callback` it was given with the credentials in the URL
//! *fragment* -- `#token=...&refresh_token=...` -- and a browser never sends a
//! fragment to a server. A listener that merely accepts the redirect therefore
//! receives a bare `GET /callback` and nothing else, forever. The standard
//! native-OAuth answer, and the one here, is a bounce: answer the redirect
//! with a tiny local page whose script reads `location.hash` and posts it back
//! to a second path on the same listener.
//!
//! Three constraints on the callback URL are the service's, confirmed against
//! the deployed build rather than read off its source, and all three are load-
//! bearing here:
//!
//! - The scheme must be `http` and the host must be the literal `127.0.0.1` or
//!   `localhost`. `https://127.0.0.1` is refused, and so is `[::1]` -- which is
//!   why this binds IPv4 explicitly and never `::1`.
//! - The URL may carry **no query and no fragment**. So the callback path is
//!   bare and nothing about this ceremony can be smuggled through it.
//! - A refusal is an HTTP 400 with an **empty body**. There is no error detail
//!   to relay, so a mis-formed callback gives a contributor nothing; getting
//!   the URL right locally is the only defence.
//!
//! Nothing in this module logs, formats or returns a token. [`SessionTokens`]
//! has a hand-written `Debug` that says only whether the fields are populated,
//! for the same reason the rest of this crate is hash-only: a credential that
//! reaches a log line has escaped.

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Where the service is told to send the browser. Bare -- no query, no
/// fragment -- because `validate_frontend_callback` refuses either.
pub const CALLBACK_PATH: &str = "/trace-commons/near-ai/callback";
/// The service refuses a longer agent than this outright, so a deposit
/// carrying one could never be refreshed and is refused at the door.
const MAX_USER_AGENT_BYTES: usize = 512;
/// Where the served page posts the fragment it read. This one is ours alone;
/// the service never sees it, so it may carry a body.
pub const DEPOSIT_PATH: &str = "/trace-commons/near-ai/deposit";

/// Header bytes accepted from one connection before it is abandoned.
const MAX_HEADER_BYTES: usize = 8192;
/// Body bytes accepted from one deposit. A JWT plus an `rt_` token is a few
/// hundred; this is generous and still bounded.
const MAX_BODY_BYTES: usize = 8192;
/// How long one connection may take to deliver its request.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

/// The session credentials a successful Cloud sign-in returns.
///
/// A session, never an inference credential: every management route on
/// cloud-api is `session_token`-only and every inference route is
/// `api_key`-only, with no fallback in either direction. The access token
/// authorizes management calls, including minting the inference key. The
/// refresh token is retained in OS storage for subsequent account reads.
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

/// A URL-safe random string, used as the ceremony's local state value.
pub fn random_state() -> Result<String> {
    let mut bytes = [0; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// The page served to the browser at [`CALLBACK_PATH`].
///
/// Deliberately minimal and entirely local: no stylesheet, no font, no image,
/// no script from anywhere. A page that fetched anything would tell a third
/// party the moment a contributor signed in, and would do it from a document
/// whose own URL fragment is a live session token.
///
/// `state` is interpolated into a JavaScript string literal. It is always this
/// module's own base64url output, so it cannot contain a quote or a backslash,
/// and [`bounce_page`] refuses anything else rather than trusting that.
fn bounce_page(state: &str) -> Result<String> {
    if state.is_empty()
        || state.len() > 64
        || !state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("near_ai_credential_callback_invalid")
    }
    Ok(format!(
        "<!doctype html>\n\
         <meta charset=\"utf-8\">\n\
         <title>Trace Commons</title>\n\
         <p id=\"m\">Finishing sign in.</p>\n\
         <script>\n\
         var m = document.getElementById(\"m\");\n\
         var h = location.hash.slice(1);\n\
         location.hash = \"\";\n\
         var u = encodeURIComponent(navigator.userAgent);\n\
         fetch(\"{DEPOSIT_PATH}\", {{ method: \"POST\", body: \"state={state}&ua=\" + u + \"&\" + h }})\n\
         .then(function (r) {{\n\
         m.textContent = r.ok\n\
         ? \"Signed in. Return to Trace Commons.\"\n\
         : \"Trace Commons could not finish sign-in. Return to the app and try again.\";\n\
         }})\n\
         .catch(function () {{ m.textContent = \"Trace Commons could not finish sign-in. Return to the app and try again.\"; }});\n\
         </script>\n"
    ))
}

/// One completed browser sign-in: the tokens, and the agent that earned them.
///
/// The agent is carried because cloud-api binds a refresh token to the
/// normalized User-Agent of the request that created the session, and for the
/// OAuth providers that request is the browser's, not ours. Without it the
/// stored session is refused on its first refresh and cannot be recovered by
/// signing in again.
pub struct BrowserSession {
    pub tokens: SessionTokens,
    pub user_agent: String,
}

/// Decode one deposit body into the session tokens it carries.
///
/// The `state` check is the whole of the local binding, and its limit should
/// be stated rather than implied: it stops a process that did not see the page
/// we served from posting a token of its own choosing, and it does not stop
/// one that did. Loopback is not a boundary between local processes on any
/// platform this ships to. What it buys is that the port is ephemeral, the
/// window is one ceremony long, and a blind poster is refused -- which is the
/// same bar the enrollment ceremony's callback holds itself to.
fn parse_deposit(body: &str, state: &str) -> Result<BrowserSession> {
    if body.len() > MAX_BODY_BYTES {
        bail!("near_ai_credential_callback_invalid")
    }
    // Reuse the URL parser rather than hand-rolling form decoding: the values
    // arrive percent-encoded and `query_pairs` is the same decoder the
    // enrollment callback already trusts.
    let url = reqwest::Url::parse(&format!("http://127.0.0.1/?{body}"))?;
    let mut seen_state = None;
    let mut access_token = None;
    let mut refresh_token = None;
    let mut user_agent = None;
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "state" => &mut seen_state,
            // Ours, not the service's: the page reports `navigator.userAgent`
            // so the refresh can present the agent the session was created
            // with. A fragment never carries this name, so it cannot collide.
            "ua" => &mut user_agent,
            // cloud-api names the access token `token` in the fragment and
            // the refresh token `refresh_token`. Not a typo, and not ours to
            // rename on the wire.
            "token" => &mut access_token,
            "refresh_token" => &mut refresh_token,
            _ => continue,
        };
        // A repeated key is a malformed deposit, not a last-one-wins.
        if slot.is_some() {
            bail!("near_ai_credential_callback_invalid")
        }
        *slot = Some(value.into_owned());
    }
    if seen_state.as_deref() != Some(state) {
        bail!("near_ai_credential_callback_invalid")
    }
    let access_token =
        access_token.ok_or_else(|| anyhow!("near_ai_credential_callback_invalid"))?;
    let refresh_token =
        refresh_token.ok_or_else(|| anyhow!("near_ai_credential_callback_invalid"))?;
    let user_agent = user_agent.unwrap_or_default();
    if access_token.is_empty() || refresh_token.is_empty() {
        bail!("near_ai_credential_callback_invalid")
    }
    // A browser that reported no agent cannot be refreshed later, and the
    // failure would land minutes afterwards on a screen that says nothing
    // about sign-in. Refuse it here, where the contributor is still looking
    // at the page that caused it.
    if user_agent.trim().is_empty() || user_agent.len() > MAX_USER_AGENT_BYTES {
        bail!("near_ai_credential_callback_invalid")
    }
    Ok(BrowserSession {
        tokens: SessionTokens {
            access_token,
            refresh_token,
        },
        user_agent,
    })
}

/// One HTTP request, far enough decoded to route it.
struct Incoming {
    method: String,
    path: String,
    body: String,
}

/// Read one request off a connection, tolerating a client that writes its
/// headers in more than one packet -- which a browser routinely does, and
/// which a single `read` would silently truncate.
async fn read_request(stream: &mut tokio::net::TcpStream) -> Result<Incoming> {
    let mut bytes = vec![0; MAX_HEADER_BYTES];
    let mut filled = 0;
    let header_end = loop {
        if filled == bytes.len() {
            bail!("near_ai_credential_callback_invalid")
        }
        let count = stream.read(&mut bytes[filled..]).await?;
        if count == 0 {
            bail!("near_ai_credential_callback_invalid")
        }
        filled += count;
        // Search from far enough back that a delimiter split across two reads
        // is still found.
        if let Some(at) = bytes[..filled]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let mut lines = head.lines();
    let parts: Vec<_> = lines.next().unwrap_or("").split_whitespace().collect();
    if parts.len() != 3 {
        bail!("near_ai_credential_callback_invalid")
    }
    let length: usize = lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);
    if length > MAX_BODY_BYTES {
        bail!("near_ai_credential_callback_invalid")
    }
    let mut body = bytes[header_end..filled].to_vec();
    body.truncate(length);
    while body.len() < length {
        let mut chunk = vec![0; length - body.len()];
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            bail!("near_ai_credential_callback_invalid")
        }
        body.extend_from_slice(&chunk[..count]);
    }
    Ok(Incoming {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, kind: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(CONNECTION_TIMEOUT, stream.write_all(response.as_bytes())).await;
}

/// Serve the bounce until one valid deposit arrives.
///
/// Unrelated local requests are answered and ignored rather than consuming the
/// listener: a browser that prefetches, a probe, or a stray `GET /favicon.ico`
/// must not end the ceremony. The caller bounds the total wait; this bounds
/// each connection. A valid deposit is acknowledged only after `finish` succeeds.
pub async fn receive_session<T, F, Fut>(
    listener: tokio::net::TcpListener,
    state: &str,
    finish: F,
) -> Result<T>
where
    F: FnOnce(BrowserSession) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let page = bounce_page(state)?;
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(Ok(request)) =
            tokio::time::timeout(CONNECTION_TIMEOUT, read_request(&mut stream)).await
        else {
            continue;
        };
        // The service appends the fragment to the callback URL; a browser
        // strips it before the request, so the path is exact.
        if request.method == "GET" && request.path == CALLBACK_PATH {
            respond(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                page.as_str(),
            )
            .await;
            continue;
        }
        if request.method != "POST" || request.path != DEPOSIT_PATH {
            respond(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                "Not found.",
            )
            .await;
            continue;
        }
        match parse_deposit(&request.body, state) {
            Ok(session) => {
                // Receiving tokens is not success: key minting and durable
                // credential replacement must finish before the browser is told
                // to return to a signed-in app. Never expose the service error.
                let outcome = finish(session).await;
                let status = if outcome.is_ok() {
                    "200 OK"
                } else {
                    "500 Internal Server Error"
                };
                respond(
                    &mut stream,
                    status,
                    "text/plain; charset=utf-8",
                    if outcome.is_ok() {
                        "Completed."
                    } else {
                        "Sign-in setup failed."
                    },
                )
                .await;
                return outcome;
            }
            Err(_) => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    "This callback was not accepted.",
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Post one deposit and read the status line back.
    async fn deposit(address: std::net::SocketAddr, body: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!(
                    "POST {DEPOSIT_PATH} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn a_deposit_is_accepted_only_with_this_ceremonys_state() {
        let tokens = parse_deposit(
            "state=abc&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x",
            "abc",
        )
        .unwrap();
        assert_eq!(tokens.tokens.access_token, "jwt");
        assert_eq!(tokens.tokens.refresh_token, "rt_x");
        assert_eq!(tokens.user_agent, "Mozilla/5.0 Test");
        // A different ceremony's state, or none at all, is refused: this is
        // the whole of the local binding and it must not be optional.
        assert!(
            parse_deposit(
                "state=other&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x",
                "abc"
            )
            .is_err()
        );
        assert!(
            parse_deposit("ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x", "abc").is_err()
        );
        // Half a credential is not a credential.
        assert!(parse_deposit("state=abc&ua=Mozilla/5.0%20Test&token=jwt", "abc").is_err());
        assert!(
            parse_deposit("state=abc&ua=Mozilla/5.0%20Test&refresh_token=rt_x", "abc").is_err()
        );
        assert!(
            parse_deposit(
                "state=abc&ua=Mozilla/5.0%20Test&token=&refresh_token=rt_x",
                "abc"
            )
            .is_err()
        );
        // A repeated key is malformed, never last-one-wins: a page that
        // appended a second `state=` must not be able to overwrite the first.
        assert!(
            parse_deposit(
                "state=abc&state=abc&ua=Mozilla/5.0%20Test&token=j&refresh_token=r",
                "abc"
            )
            .is_err()
        );
        assert!(
            parse_deposit(
                "state=abc&ua=Mozilla/5.0%20Test&token=j&token=j2&refresh_token=r",
                "abc"
            )
            .is_err()
        );
        // The agent is as load-bearing as the tokens: a deposit without one
        // produces a session that cannot ever be refreshed, and the refusal
        // has to land here rather than minutes later on an unrelated screen.
        assert!(parse_deposit("state=abc&token=jwt&refresh_token=rt_x", "abc").is_err());
        assert!(parse_deposit("state=abc&ua=&token=jwt&refresh_token=rt_x", "abc").is_err());
        assert!(parse_deposit("state=abc&ua=%20%20&token=jwt&refresh_token=rt_x", "abc").is_err());
        // And one the service would refuse outright is refused here instead.
        let oversized = "x".repeat(MAX_USER_AGENT_BYTES + 1);
        assert!(
            parse_deposit(
                &format!("state=abc&ua={oversized}&token=jwt&refresh_token=rt_x"),
                "abc"
            )
            .is_err()
        );
    }

    #[test]
    fn the_served_page_reaches_only_this_listener_and_quotes_nothing_it_was_given() {
        let state = random_state().unwrap();
        let page = bounce_page(&state).unwrap();
        assert!(page.contains(DEPOSIT_PATH));
        assert!(page.contains(&state));
        // No third party learns that a contributor signed in, and nothing is
        // fetched from a document whose own fragment is a live token.
        assert!(!page.contains("http://"), "{page}");
        assert!(!page.contains("https://"), "{page}");
        // A state that could break out of the JavaScript string literal is
        // refused rather than escaped.
        for hostile in ["\";alert(1)//", "a'b", "a\\b", "", &"a".repeat(65)] {
            assert!(bounce_page(hostile).is_err(), "{hostile}");
        }
    }

    #[test]
    fn a_session_never_prints_itself() {
        let rendered = format!(
            "{:?}",
            SessionTokens {
                access_token: "jwt-secret".into(),
                refresh_token: "rt_secret".into(),
            }
        );
        assert!(!rendered.contains("secret"), "{rendered}");
    }

    #[tokio::test]
    async fn the_redirect_is_answered_with_the_bounce_and_the_deposit_completes_it() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = random_state().unwrap();
        let task = tokio::spawn({
            let state = state.clone();
            async move { receive_session(listener, &state, |tokens| async { Ok(tokens) }).await }
        });

        // The browser's redirect arrives with the fragment already stripped.
        // Answering it must not end the ceremony -- if it did, the tokens
        // would never arrive at all.
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {CALLBACK_PATH} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(DEPOSIT_PATH), "{response}");

        // An unrelated local request is answered and ignored, not consumed.
        assert_eq!(
            deposit(address, "state=wrong&token=j&refresh_token=r").await,
            "HTTP/1.1 400 Bad Request"
        );
        assert_eq!(
            deposit(
                address,
                &format!("state={state}&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x")
            )
            .await,
            "HTTP/1.1 200 OK"
        );
        let tokens = task.await.unwrap().unwrap();
        assert_eq!(tokens.tokens.access_token, "jwt");
        assert_eq!(tokens.tokens.refresh_token, "rt_x");
        assert_eq!(tokens.user_agent, "Mozilla/5.0 Test");
    }

    #[tokio::test]
    async fn browser_success_waits_for_credential_setup_to_finish() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            receive_session(listener, "state", |tokens| async move {
                assert_eq!(tokens.tokens.refresh_token, "rt_x");
                started_tx.send(()).unwrap();
                finish_rx.await.unwrap();
                Ok(())
            })
            .await
        });
        let mut browser = tokio::spawn(async move {
            deposit(
                address,
                "state=state&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x",
            )
            .await
        });
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut browser)
                .await
                .is_err()
        );
        finish_tx.send(()).unwrap();
        assert_eq!(browser.await.unwrap(), "HTTP/1.1 200 OK");
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn valid_browser_login_does_not_report_success_when_setup_fails() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            receive_session(listener, "state", |_tokens| async {
                Err::<(), _>(anyhow!("synthetic credential storage failure"))
            })
            .await
        });
        assert_eq!(
            deposit(
                address,
                "state=state&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x"
            )
            .await,
            "HTTP/1.1 500 Internal Server Error"
        );
        assert!(server.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn a_deposit_split_across_packets_still_arrives_whole() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            receive_session(listener, "state", |tokens| async { Ok(tokens) }).await
        });
        let body = "state=state&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x";
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("POST {DEPOSIT_PATH} HTTP/1.1\r\nHost: local").as_bytes())
            .await
            .unwrap();
        tokio::task::yield_now().await;
        stream
            .write_all(format!("host\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        tokio::task::yield_now().await;
        // The body itself arrives after the headers, in its own packet.
        stream.write_all(body.as_bytes()).await.unwrap();
        assert_eq!(task.await.unwrap().unwrap().tokens.refresh_token, "rt_x");
    }
}
