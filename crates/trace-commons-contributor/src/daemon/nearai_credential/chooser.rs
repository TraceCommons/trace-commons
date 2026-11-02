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

use super::loopback_http::{self, Head};
use anyhow::{Result, anyhow, bail};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

/// Where the browser is first sent. The attempt's state follows as `?state=`.
pub const PAGE_PATH: &str = "/trace-commons/near-ai/sign-in";
/// Where the page's form posts the choice.
pub const CHOOSE_PATH: &str = "/trace-commons/near-ai/choose";

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
    fill(
        include_str!("choose.html"),
        offered,
        account_login,
        state,
        nonce,
    )
}

/// [`render`] on a given template. The placeholders are matched without
/// their line endings, so a template checked out with CRLF (Git on Windows)
/// fills exactly as an LF one does.
fn fill(
    template: &str,
    offered: &[&str],
    account_login: Option<&str>,
    state: &str,
    nonce: &str,
) -> Result<String> {
    use crate::private_inference_copy as copy;
    if !super::loopback::state_shaped(state)
        || !super::loopback::state_shaped(nonce)
        || offered.is_empty()
    {
        bail!("near_ai_credential_unavailable")
    }
    let marked = account_login.filter(|tag| offered.contains(tag));
    let mut options = String::new();
    if let Some(tag) = marked {
        options.push_str(&button(tag)?);
        options.push_str(&note(copy::NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN));
    }
    for tag in offered.iter().filter(|tag| Some(**tag) != marked) {
        options.push_str(&button(tag)?);
    }
    Ok(template
        .replace("__HEADING__", &escape(copy::NEAR_AI_SIGN_IN_PAGE_HEADING))
        .replace("__CHOOSE_PATH__", CHOOSE_PATH)
        .replace("__STATE__", state)
        .replace("__NONCE__", nonce)
        .replace("__OPTIONS__", options.trim_end()))
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

/// A choice that passed every check, with the connection that made it still
/// open so the caller can send the browser on once the next step exists.
pub(super) struct Chosen {
    pub provider: &'static str,
    stream: TcpStream,
}

impl Chosen {
    /// Send the browser to the chosen login. `location` is built by the
    /// ceremony, never read from the request. `false` when the redirect was
    /// not written in time: the browser was never sent on, and the listener
    /// has left the page, so the caller fails the attempt rather than wait.
    #[must_use]
    pub(super) async fn redirect(mut self, location: &str) -> bool {
        let response = format!(
            "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\n\
             Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\
             Connection: close\r\n\r\n"
        );
        loopback_http::write_all(&mut self.stream, response.as_bytes(), CONNECTION_TIMEOUT).await
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

    #[cfg(test)]
    pub(super) fn for_test(provider: &'static str, stream: TcpStream) -> Self {
        Self { provider, stream }
    }
}

/// This attempt's page coming back after the listener moved on to the first
/// login: the person went Back. A browser that kept the page in its
/// back/forward cache posts a second choice; one that did not (the page is
/// `no-store`) fetches the page's address again. The leg cannot switch, so
/// the listener answers with the page's existing refusal and ends the
/// attempt, and the app can start a new one at once instead of waiting out
/// the timeout.
///
/// Recognized only by this attempt's state: the page's exact address, or a
/// choice by the same rules as a first one (this listener's origin, a login
/// this build offers). A page elsewhere cannot end someone's sign-in.
pub(crate) struct StaleChoice {
    state: String,
}

/// What the listener answers a stale choice with: the chooser's own refusal.
pub(super) const STALE_CHOICE_BODY: &str = "Not accepted.";

/// The control name an attempt ended by a stale choice fails with.
pub(super) const STALE_CHOICE: &str = "near_ai_credential_choice_stale";

impl StaleChoice {
    pub(super) fn new(state: String) -> Self {
        Self { state }
    }

    /// Whether `head` and `body`, sent to `authority`, are this attempt's
    /// page fetched again or another choice from it.
    pub(super) fn matches(&self, head: &Head, body: &[u8], authority: &str) -> bool {
        if head.method == "GET" {
            return body.is_empty() && head.target == format!("{PAGE_PATH}?state={}", self.state);
        }
        head.method == "POST"
            && head.target == CHOOSE_PATH
            && matches!(
                choice(
                    head,
                    body,
                    authority,
                    &self.state,
                    crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
                ),
                Route::Choice(_)
            )
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
        let route = match tokio::time::timeout(
            CONNECTION_TIMEOUT,
            loopback_http::read_request(&mut stream, &authority, MAX_BODY_BYTES),
        )
        .await
        {
            Ok(Ok((head, body))) => route(&head, &body, &authority, state, offered),
            // Malformed, oversized, or for another name than this listener's.
            Ok(Err(_)) => Route::Refused,
            Err(_) => continue,
        };
        match route {
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

fn route(
    head: &Head,
    body: &[u8],
    authority: &str,
    state: &str,
    offered: &[&'static str],
) -> Route {
    match head.method.as_str() {
        "GET" if head.target == format!("{PAGE_PATH}?state={state}") && body.is_empty() => {
            Route::Page
        }
        "GET" => Route::NotFound,
        "POST" if head.target == CHOOSE_PATH => choice(head, body, authority, state, offered),
        _ => Route::NotFound,
    }
}

fn choice(
    head: &Head,
    body: &[u8],
    authority: &str,
    state: &str,
    offered: &[&'static str],
) -> Route {
    // A form post from this page carries this origin; one from anywhere else
    // carries its own, or `null`. Required, not merely checked when present.
    if head.origin.as_deref() != Some(format!("http://{authority}").as_str()) {
        return Route::Refused;
    }
    if head
        .fetch_site
        .as_deref()
        .is_some_and(|site| site != "same-origin")
    {
        return Route::Refused;
    }
    if !head.content_type.as_deref().is_some_and(|kind| {
        kind.split(';')
            .next()
            .is_some_and(|kind| kind.trim() == "application/x-www-form-urlencoded")
    }) {
        return Route::Refused;
    }
    // Decoded as a form body, not as a URL: a `#` is then part of a value,
    // never the start of a fragment that hides keys from the check below.
    let (mut seen_state, mut provider) = (None, None);
    for (key, value) in url::form_urlencoded::parse(body) {
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
    let _ = loopback_http::write_all(stream, response.as_bytes(), CONNECTION_TIMEOUT).await;
}
