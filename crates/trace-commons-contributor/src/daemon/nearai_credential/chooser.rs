//! The in-browser login chooser: the first page a sign-in that names no login
//! opens, listing this build's logins (`channel::NEAR_AI_SIGN_IN_PROVIDERS`)
//! as buttons.
//!
//! The person picks a login in the browser rather than in the app (owner,
//! 2026-11-02), so every shell has one Sign in button and the list lives in
//! one place. The page is data-driven: a login is a channel entry, a title
//! (`private_inference_copy::credential_provider_title`) and a ceremony arm.
//!
//! The page is served from the listener the ceremony already bound, and the
//! chosen login continues on that same listener: the OAuth callback and the
//! NEAR wallet page use its port. That keeps the redirect after a choice
//! same-origin for the wallet, and it keeps one port per sign-in, which is
//! what the CLI's remote sign-in tells the person to forward.
//!
//! What a choice must carry, and why:
//!
//! - **The attempt's state**, in the body of a `POST`. The page's address
//!   carries it too, and that address is handed to the shell once, by
//!   `begin`. A page elsewhere that never saw it cannot make a choice.
//! - **This listener's own origin** (`Origin: http://127.0.0.1:<port>`), and
//!   its own `Host`. A form posted from another site carries that site's
//!   origin and is refused even if it somehow had the state; a `Host` check
//!   refuses a DNS-rebound name pointing at this port.
//! - **A login this build offers.** Anything else is refused, never mapped to
//!   a default.
//!
//! Where the browser goes next is never taken from the request: it is the
//! service's authorize URL for this port, or the wallet page on this port.
//!
//! Nothing here logs. Responses carry fixed text only.

use anyhow::{Result, anyhow, bail};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Where the browser is first sent. The attempt's state follows as `?state=`.
pub const PAGE_PATH: &str = "/trace-commons/near-ai/sign-in";
/// Where the page's form posts the choice.
pub const CHOOSE_PATH: &str = "/trace-commons/near-ai/choose";

const MAX_HEADER_BYTES: usize = 8192;
/// `state=<43 chars>&provider=<tag>` is well under this.
const MAX_BODY_BYTES: usize = 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
#[path = "chooser_tests.rs"]
mod tests;

/// The address the shell opens: this listener's page, bound to `state`.
pub fn page_url(port: u16, state: &str) -> String {
    format!("http://127.0.0.1:{port}{PAGE_PATH}?state={state}")
}

/// A state or nonce is this crate's own base64url output. Anything else is
/// refused rather than escaped, as the OAuth bounce page does.
fn token_shaped(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// The page: one button per offered login, in the build's order, with the
/// wallet notice under the NEAR wallet button.
///
/// With `account_login` -- the login this device's account was last
/// accepted with (`login_record::commons_login`) -- that login comes first,
/// marked as connected to this app, and the others follow. A remembered
/// login this build does not offer is ignored and the page is drawn as if
/// none were remembered.
///
/// Fails closed on a login with no title: a button with no name, or with the
/// raw tag, is not something to show a person.
pub(super) fn render(
    offered: &[&str],
    account_login: Option<&str>,
    state: &str,
    nonce: &str,
) -> Result<String> {
    use crate::private_inference_copy as copy;
    if !token_shaped(state) || !token_shaped(nonce) || offered.is_empty() {
        bail!("near_ai_credential_unavailable")
    }
    let marked = account_login.filter(|tag| offered.contains(tag));
    let mut options = String::new();
    if let Some(tag) = marked {
        options.push_str(&button(tag)?);
        options.push_str(&note(copy::NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN));
        if let Some(path) = change_account_path() {
            options.push_str(&path);
        }
    }
    for tag in offered.iter().filter(|tag| Some(**tag) != marked) {
        options.push_str(&button(tag)?);
    }
    Ok(include_str!("choose.html")
        .replace("__HEADING__", &escape(copy::NEAR_AI_SIGN_IN_PAGE_HEADING))
        .replace("__CHOOSE_PATH__", CHOOSE_PATH)
        .replace("__STATE__", state)
        .replace("__NONCE__", nonce)
        .replace("__OPTIONS__\n", &options))
}

/// One login's button, and the wallet notice under the wallet's.
fn button(tag: &str) -> Result<String> {
    if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_lowercase()) {
        bail!("near_ai_credential_provider_unknown")
    }
    let title = crate::private_inference_copy::credential_provider_title(tag)
        .ok_or_else(|| anyhow!("near_ai_credential_provider_unknown"))?;
    let mut html = format!(
        "<button type=\"submit\" name=\"provider\" value=\"{tag}\">{}</button>\n",
        escape(title)
    );
    if tag == "near" {
        html.push_str(&note(
            crate::private_inference_copy::CREDENTIAL_WALLET_NOTICE,
        ));
    }
    Ok(html)
}

fn note(text: &str) -> String {
    format!("<p class=\"note\">{}</p>\n", escape(text))
}

/// Seam for changing which NEAR AI account the Trace Commons account uses.
/// That flow has its own spec (`change-nearai-account-design`); when it
/// exists, its way in from this page -- markup placed under the marked
/// login -- comes from here. Until then there is none, and the page offers
/// the other logins without promising they reach the same account.
fn change_account_path() -> Option<String> {
    None
}

/// A choice that passed every check, with the connection that made it still
/// open so the caller can send the browser on once the next step exists.
pub(super) struct Chosen {
    pub provider: &'static str,
    stream: TcpStream,
}

impl Chosen {
    /// Send the browser to the chosen login. `location` is built by the
    /// ceremony, never read from the request.
    pub(super) async fn redirect(mut self, location: &str) {
        let response = format!(
            "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\n\
             Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\
             Connection: close\r\n\r\n"
        );
        let _ = tokio::time::timeout(
            CONNECTION_TIMEOUT,
            self.stream.write_all(response.as_bytes()),
        )
        .await;
    }

    /// The chosen login could not be started. The same fixed text the OAuth
    /// listener answers a failed setup with.
    pub(super) async fn refuse(mut self) {
        respond(
            &mut self.stream,
            "500 Internal Server Error",
            "text/plain; charset=utf-8",
            "Sign-in setup failed.",
        )
        .await;
    }
}

/// Serve the page until a valid choice arrives.
///
/// Like the other loopback listeners, an unrelated or refused request is
/// answered and the listener keeps waiting: a prefetch, a probe, or a forged
/// post must not end the sign-in. The caller bounds the total wait.
pub(super) async fn receive_choice(
    listener: &TcpListener,
    state: &str,
    offered: &[&'static str],
    account_login: Option<&str>,
) -> Result<Chosen> {
    let port = listener.local_addr()?.port();
    let authority = format!("127.0.0.1:{port}");
    let page = render(
        offered,
        account_login,
        state,
        &super::loopback::random_state()?,
    )?;
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(Ok(request)) =
            tokio::time::timeout(CONNECTION_TIMEOUT, read_request(&mut stream)).await
        else {
            continue;
        };
        match route(&request, &authority, state, offered) {
            Route::Page => {
                respond(&mut stream, "200 OK", "text/html; charset=utf-8", &page).await;
            }
            Route::Choice(provider) => return Ok(Chosen { provider, stream }),
            Route::NotFound => {
                respond(
                    &mut stream,
                    "404 Not Found",
                    "text/plain; charset=utf-8",
                    "Not found.",
                )
                .await;
            }
            Route::Refused => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    "Not accepted.",
                )
                .await;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Page,
    Choice(&'static str),
    NotFound,
    Refused,
}

#[derive(Default)]
struct Request {
    method: String,
    target: String,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
    fetch_site: Option<String>,
    body: Vec<u8>,
}

fn route(request: &Request, authority: &str, state: &str, offered: &[&'static str]) -> Route {
    if request.host.as_deref() != Some(authority) {
        return Route::Refused;
    }
    match request.method.as_str() {
        "GET" if request.target == format!("{PAGE_PATH}?state={state}") => Route::Page,
        "GET" => Route::NotFound,
        "POST" if request.target == CHOOSE_PATH => choice(request, authority, state, offered),
        _ => Route::NotFound,
    }
}

fn choice(request: &Request, authority: &str, state: &str, offered: &[&'static str]) -> Route {
    // A form post from this page carries this origin; one from anywhere else
    // carries its own, or `null`. Required, not merely checked when present.
    if request.origin.as_deref() != Some(format!("http://{authority}").as_str()) {
        return Route::Refused;
    }
    if request
        .fetch_site
        .as_deref()
        .is_some_and(|site| site != "same-origin")
    {
        return Route::Refused;
    }
    if !request.content_type.as_deref().is_some_and(|kind| {
        kind.split(';')
            .next()
            .is_some_and(|kind| kind.trim() == "application/x-www-form-urlencoded")
    }) {
        return Route::Refused;
    }
    let Ok(body) = std::str::from_utf8(&request.body) else {
        return Route::Refused;
    };
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1/?{body}")) else {
        return Route::Refused;
    };
    let (mut seen_state, mut provider) = (None, None);
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "state" => &mut seen_state,
            "provider" => &mut provider,
            _ => return Route::Refused,
        };
        if slot.replace(value.into_owned()).is_some() {
            return Route::Refused;
        }
    }
    if seen_state.as_deref() != Some(state) {
        return Route::Refused;
    }
    match offered
        .iter()
        .find(|tag| Some(**tag) == provider.as_deref())
    {
        Some(tag) => Route::Choice(tag),
        None => Route::Refused,
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut bytes = Vec::with_capacity(1024);
    let mut chunk = [0; 1024];
    let end = loop {
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            bail!("near_ai_credential_callback_invalid")
        }
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            bail!("near_ai_credential_callback_invalid")
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = std::str::from_utf8(&bytes[..end - 4])
        .map_err(|_| anyhow!("near_ai_credential_callback_invalid"))?;
    let mut lines = head.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
    let [method, target, "HTTP/1.1"] = parts.as_slice() else {
        bail!("near_ai_credential_callback_invalid")
    };
    let mut request = Request {
        method: (*method).to_string(),
        target: (*target).to_string(),
        ..Request::default()
    };
    let mut length = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow!("near_ai_credential_callback_invalid"))?;
        let value = value.trim().to_string();
        let slot = match name.trim().to_ascii_lowercase().as_str() {
            "host" => &mut request.host,
            "origin" => &mut request.origin,
            "content-type" => &mut request.content_type,
            "sec-fetch-site" => &mut request.fetch_site,
            "content-length" => &mut length,
            "transfer-encoding" | "expect" => bail!("near_ai_credential_callback_invalid"),
            _ => continue,
        };
        // A repeated header is malformed, never last-one-wins.
        if slot.replace(value).is_some() {
            bail!("near_ai_credential_callback_invalid")
        }
    }
    let length: usize = match length {
        Some(value) => value
            .parse()
            .map_err(|_| anyhow!("near_ai_credential_callback_invalid"))?,
        None => 0,
    };
    if length > MAX_BODY_BYTES {
        bail!("near_ai_credential_callback_invalid")
    }
    let mut body = bytes[end..].to_vec();
    while body.len() < length {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            bail!("near_ai_credential_callback_invalid")
        }
        body.extend_from_slice(&chunk[..count]);
    }
    if body.len() != length {
        bail!("near_ai_credential_callback_invalid")
    }
    request.body = body;
    Ok(request)
}

async fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &str) {
    // The page's own policy is a meta tag; these are the ones a meta tag
    // cannot carry. No framing: the buttons are not to be clickjacked.
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: same-origin\r\n\
         X-Frame-Options: DENY\r\nContent-Security-Policy: frame-ancestors 'none'\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(CONNECTION_TIMEOUT, stream.write_all(response.as_bytes())).await;
}
