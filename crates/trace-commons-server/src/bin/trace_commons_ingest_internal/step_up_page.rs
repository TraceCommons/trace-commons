// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The browser passkey step-up page, Z2 slice S7
//! (`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`,
//! "Step-up").
//!
//! A native session is weak: it can read and withdraw, but the Slice 3a gate
//! (`require_authenticator_change_allowed`) refuses it every authenticator
//! change and every payout change once the account holds a strong
//! authenticator. Those changes need a strong `client_kind='passkey'` cookie
//! session, which only a browser passkey sign-in mints. This page is where the
//! native app sends the person for one.
//!
//! `GET /account/step-up` serves a plain HTML page whose one inline script
//! drives the EXISTING routes and nothing else:
//!
//! - `POST /account/passkey/login/{start,finish}` for the sign-in, which sets
//!   the strong session cookie;
//! - `/v1/account/passkeys/register/{start,finish}`, `GET /v1/account/passkeys`
//!   and `DELETE /v1/account/passkeys/{credential_id}` to add or remove a
//!   passkey;
//! - `GET /v1/account/near-identities` and
//!   `PATCH /v1/account/near-identities/{public_key}/payout` to change the
//!   payout;
//! - `POST /v1/account/logout` to end the browser session afterwards.
//!
//! No new API, no new login mechanism, and nothing crosses back into the app:
//! the native token is never upgraded.
//!
//! **The `return` hint names a section of this page, never a URL.** It is
//! matched exactly against [`StepUpAction`]; anything else (a URL, a second
//! `return`, an unknown parameter, a malformed query) is refused with a
//! scriptless "not valid" page, and the refused value is never echoed or
//! logged. There is no redirect anywhere in this flow, so there is nothing to
//! open-redirect.
//!
//! **Fail closed.** When the relying party or the account database is not
//! configured, the page is a scriptless "unavailable" page with a 503. It
//! never renders the sign-in button against a server that cannot finish it.
//!
//! **Headers.** Every response carries [`CSP_WITH_SCRIPT`] or
//! [`CSP_WITHOUT_SCRIPT`] (no `'unsafe-inline'`: the one script and the one
//! style block are pinned by SHA-256), `frame-ancestors 'none'` plus
//! `X-Frame-Options: DENY`, `Cache-Control: no-store`,
//! `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.
//!
//! **Logs and audit.** Label-only log lines; no query value, cookie, IP or
//! account detail. The page itself is unauthenticated and has no tenant, so it
//! writes no audit row; the sign-in and every change it drives are audited by
//! the existing routes (`account_passkey_login`, `account_passkey_enrolled`,
//! `account_passkey_removed`, `account_payout_designated`).
//!
//! **Copy is PROPOSED** and awaits Zaki's approval. All of it lives in
//! [`STEP_UP_COPY`], the one table both the server-rendered HTML and the
//! script read from.

use std::sync::LazyLock;

use axum::extract::{Query, State, rejection::QueryRejection};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use sha2::{Digest, Sha256};

use super::AppState;
use std::sync::Arc;

/// The route this module serves. The URL contract for the native app is
/// documented in `docs/operator/native-step-up-page.md`.
pub(super) const STEP_UP_PATH: &str = "/account/step-up";

/// The only query parameter the page accepts.
pub(super) const RETURN_PARAM: &str = "return";

/// The page's one script, served inline and pinned by hash.
pub(super) const STEP_UP_SCRIPT: &str = include_str!("step_up_page.js");

/// The page's one style block, served inline and pinned by hash.
pub(super) const STEP_UP_STYLE: &str = "body{font:18px/1.5 system-ui,sans-serif;max-width:40rem;margin:8vh auto;padding:0 1.5rem;color:#1d2521;background:#f8faf7}button{font:inherit;padding:.6rem 1rem;margin:.25rem 0}input{font:inherit;padding:.4rem;width:100%;box-sizing:border-box}ul{padding-left:1.2rem}li{margin:.5rem 0}small{color:#4d5a53}#status{min-height:1.5em;font-weight:600}";

/// Every string the page shows. PROPOSED COPY, pending Zaki's approval: edit
/// it here and only here. Keys are referenced by the server-rendered HTML
/// below and by the script through `t('key')` / `say('key')`; a test fails if
/// the script names a key missing from this table.
pub(super) const STEP_UP_COPY: &[(&str, &str)] = &[
    ("page_title", "Confirm it's you - TraceCommons"),
    ("heading", "Confirm it's you"),
    (
        "intro",
        "Adding or removing a passkey, or changing where your credit is paid, needs a fresh passkey sign-in in this browser. The app cannot make these changes on its own.",
    ),
    (
        "noscript",
        "This page needs JavaScript to use your passkey.",
    ),
    ("sign_in_button", "Sign in with a passkey"),
    (
        "status_unsupported",
        "This browser cannot use passkeys. Open this page in a browser that supports them.",
    ),
    ("status_signing_in", "Waiting for your passkey..."),
    (
        "status_sign_in_failed",
        "Sign-in did not complete. Nothing was changed. You can try again.",
    ),
    ("status_signed_in", "Signed in."),
    ("status_waiting", "Waiting for your passkey..."),
    ("action_add_heading", "Add a passkey"),
    ("action_add_label", "Name for the new passkey (optional)"),
    ("action_add_button", "Add a passkey"),
    ("action_add_done", "Passkey added."),
    ("action_remove_heading", "Remove a passkey"),
    ("action_remove_button", "Remove"),
    (
        "action_remove_confirm",
        "Remove this passkey? Anything it is signed in to will be signed out.",
    ),
    ("action_remove_done", "Passkey removed."),
    ("action_remove_this_device", "(used for this sign-in)"),
    ("action_remove_unnamed", "Unnamed passkey"),
    ("action_payout_heading", "Choose where your credit is paid"),
    ("action_payout_button", "Pay credit here"),
    ("action_payout_current", "(credit is paid here)"),
    (
        "action_payout_none",
        "No NEAR account is linked to this account yet.",
    ),
    ("action_payout_done", "Payout account updated."),
    (
        "action_failed",
        "That did not work. Nothing was changed. You can try again.",
    ),
    (
        "action_refused",
        "This change needs a passkey sign-in in this browser. Sign in again and retry.",
    ),
    (
        "finish_hint",
        "When you are done, sign out of this browser and return to the app.",
    ),
    ("sign_out_button", "Sign out of this browser"),
    (
        "signed_out",
        "Signed out. You can close this window and return to the app.",
    ),
    ("unavailable_heading", "Not available"),
    (
        "unavailable_body",
        "Passkey sign-in is not available on this server right now. Nothing was changed. Return to the app.",
    ),
    ("invalid_heading", "This link is not valid"),
    (
        "invalid_body",
        "Return to the app and try again. Nothing was changed.",
    ),
];

/// Look up a copy string. A missing key is a programming error caught by the
/// tests; at runtime it renders as empty rather than panicking.
pub(super) fn copy(key: &str) -> &'static str {
    STEP_UP_COPY
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

/// The actions the native app may send the person here for. The `return`
/// hint must be exactly one of these labels; each names a section of the
/// page, never a destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StepUpAction {
    AddPasskey,
    RemovePasskey,
    ChangePayout,
}

impl StepUpAction {
    pub(super) const ALL: [Self; 3] = [Self::AddPasskey, Self::RemovePasskey, Self::ChangePayout];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::AddPasskey => "add-passkey",
            Self::RemovePasskey => "remove-passkey",
            Self::ChangePayout => "change-payout",
        }
    }

    /// Exact, case-sensitive match. No trimming, no decoding beyond the
    /// query's own, no prefix match.
    pub(super) fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.label() == raw)
    }
}

/// What a request's query asked for.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Requested {
    /// No `return`: every section, after sign-in.
    Menu,
    /// One allowlisted section.
    Action(StepUpAction),
    /// Anything else. The value is deliberately not kept.
    Refused,
}

/// Classify the query. Only `return`, at most once, with an allowlisted
/// value; everything else is refused.
pub(super) fn requested(pairs: &[(String, String)]) -> Requested {
    let mut action = None;
    for (key, value) in pairs {
        if key != RETURN_PARAM || action.is_some() {
            return Requested::Refused;
        }
        match StepUpAction::parse(value) {
            Some(parsed) => action = Some(parsed),
            None => return Requested::Refused,
        }
    }
    action.map_or(Requested::Menu, Requested::Action)
}

fn sha256_source(text: &str) -> String {
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(text.as_bytes()))
    )
}

/// The CSP for the page that runs the ceremony. `default-src 'none'` and only
/// the pinned script and style, same-origin fetches, no base, no forms, no
/// framing, and Trusted Types so no HTML sink can be reached.
pub(super) static CSP_WITH_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "default-src 'none'; script-src {}; style-src {}; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; require-trusted-types-for 'script'",
        sha256_source(STEP_UP_SCRIPT),
        sha256_source(STEP_UP_STYLE),
    )
});

/// The CSP for the scriptless unavailable and invalid pages: nothing runs and
/// nothing is fetched.
pub(super) static CSP_WITHOUT_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "default-src 'none'; style-src {}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        sha256_source(STEP_UP_STYLE),
    )
});

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn c(key: &str) -> String {
    escape_html(copy(key))
}

/// The copy table as JSON for the script, safe inside a `<script>` data
/// block: every `<` is escaped so no string can close the element.
fn copy_json() -> String {
    let map: serde_json::Map<String, serde_json::Value> = STEP_UP_COPY
        .iter()
        .map(|(k, v)| ((*k).to_string(), serde_json::Value::from(*v)))
        .collect();
    serde_json::Value::Object(map)
        .to_string()
        .replace('<', "\\u003c")
}

fn head(title_key: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><title>{}</title><style>{STEP_UP_STYLE}</style></head>",
        c(title_key)
    )
}

/// The ceremony page. `action` is `None` for the menu; the body carries the
/// allowlisted label, never request input.
pub(super) fn ceremony_html(action: Option<StepUpAction>) -> String {
    let action = action.map_or("", StepUpAction::label);
    format!(
        "{head}<body data-action=\"{action}\"><main>\
<h1>{heading}</h1><p>{intro}</p><noscript><p>{noscript}</p></noscript>\
<section id=\"sign-in-section\"><button id=\"sign-in\" type=\"button\" disabled>{sign_in}</button></section>\
<section id=\"add-passkey\" hidden><h2>{add_heading}</h2><label for=\"passkey-label\">{add_label}</label><input id=\"passkey-label\" maxlength=\"64\" autocomplete=\"off\"><button id=\"add-passkey-button\" type=\"button\">{add_button}</button></section>\
<section id=\"remove-passkey\" hidden><h2>{remove_heading}</h2><ul id=\"passkey-list\"></ul></section>\
<section id=\"change-payout\" hidden><h2>{payout_heading}</h2><ul id=\"payout-list\"></ul></section>\
<section id=\"finish\" hidden><p>{finish_hint}</p><button id=\"sign-out\" type=\"button\">{sign_out}</button></section>\
<p id=\"status\" role=\"status\" aria-live=\"polite\"></p>\
</main><script type=\"application/json\" id=\"tc-copy\">{copy}</script><script>{STEP_UP_SCRIPT}</script></body></html>",
        head = head("page_title"),
        heading = c("heading"),
        intro = c("intro"),
        noscript = c("noscript"),
        sign_in = c("sign_in_button"),
        add_heading = c("action_add_heading"),
        add_label = c("action_add_label"),
        add_button = c("action_add_button"),
        remove_heading = c("action_remove_heading"),
        payout_heading = c("action_payout_heading"),
        finish_hint = c("finish_hint"),
        sign_out = c("sign_out_button"),
        copy = copy_json(),
    )
}

/// A scriptless page with one heading and one paragraph.
pub(super) fn notice_html(heading_key: &str, body_key: &str) -> String {
    format!(
        "{head}<body><main><h1>{heading}</h1><p>{body}</p></main></body></html>",
        head = head(heading_key),
        heading = c(heading_key),
        body = c(body_key),
    )
}

fn page(status: StatusCode, html: String, csp: &str) -> Response {
    let mut response = (status, axum::response::Html(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    match HeaderValue::from_str(csp) {
        Ok(value) => {
            headers.insert(header::CONTENT_SECURITY_POLICY, value);
        }
        // The CSP is built from constants and base64, so this cannot happen;
        // if it ever did, serve nothing rather than a page without its policy.
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    response
}

/// Whether the page can do its job: a relying party to verify the sign-in and
/// an account database to hold the session. Either missing means the login
/// routes would refuse, so the page says so instead of offering a button.
fn available(state: &AppState) -> bool {
    state.account_webauthn.is_some() && state.db_mirror.is_some()
}

/// `GET /account/step-up` — the browser step-up page (Z2 S7).
pub(super) async fn step_up_page_handler(
    State(state): State<Arc<AppState>>,
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    if !available(state.as_ref()) {
        tracing::warn!(
            label = "account_step_up_unavailable",
            "step-up page served its unavailable state"
        );
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            notice_html("unavailable_heading", "unavailable_body"),
            &CSP_WITHOUT_SCRIPT,
        );
    }
    let requested = match query {
        Ok(Query(pairs)) => requested(&pairs),
        Err(_) => Requested::Refused,
    };
    let action = match requested {
        Requested::Menu => None,
        Requested::Action(action) => Some(action),
        Requested::Refused => {
            tracing::info!(
                label = "account_step_up_return_refused",
                "step-up page refused its query"
            );
            return page(
                StatusCode::BAD_REQUEST,
                notice_html("invalid_heading", "invalid_body"),
                &CSP_WITHOUT_SCRIPT,
            );
        }
    };
    tracing::info!(
        label = "account_step_up_page_served",
        action = action.map_or("menu", StepUpAction::label),
        "step-up page served"
    );
    page(StatusCode::OK, ceremony_html(action), &CSP_WITH_SCRIPT)
}
