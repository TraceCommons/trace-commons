// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The browser passkey step-up page (Z2 native passkey identity, slice S7).
//!
//! The in-memory tests run everywhere. The PostgreSQL test self-skips without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL` and needs the login-resolver URL too,
//! because the page's sign-in resolves the credential's tenant through the
//! resolver role. Its ceremonies are real WebAuthn ceremonies driven by the
//! dev-only SoftPasskey authenticator, from the origins a browser would report.

use super::*;
use crate::step_up_page::{
    CSP_WITH_SCRIPT, CSP_WITHOUT_SCRIPT, STEP_UP_COPY, STEP_UP_PATH, STEP_UP_SCRIPT, STEP_UP_STYLE,
    StepUpAction,
};
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_server::account_native_passkey::UnboundAccountCeiling;

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes).unwrap_or_default()
    }

    fn header(&self, name: impl axum::http::header::AsHeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The `name=value` pair of the first `Set-Cookie` for `name`.
    fn cookie_pair(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(axum::http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|c| c.starts_with(&format!("{name}=")))
            .and_then(|c| c.split(';').next())
            .map(str::to_string)
    }
}

async fn send(state: &Arc<AppState>, request: axum::http::Request<Body>) -> Reply {
    let response = app(state.clone())
        .oneshot(request)
        .await
        .expect("router is infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    Reply {
        status,
        headers,
        bytes,
    }
}

fn page_request(query: &str) -> axum::http::Request<Body> {
    let uri = if query.is_empty() {
        STEP_UP_PATH.to_string()
    } else {
        format!("{STEP_UP_PATH}?{query}")
    };
    axum::http::Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// A state with a relying party and an in-memory account database: the page
/// is available.
fn available_state() -> (Arc<AppState>, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = Arc::new(NativeAuthTestDb::default());
    let state = test_state_with_webauthn(root.path().to_path_buf(), Some(db));
    (state, root)
}

/// The executable scripts and the style blocks of a served page.
fn inline_elements(html: &str, open: &str, close: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(open) {
        let after = &rest[at + open.len()..];
        let end = after.find(close).expect("element closes");
        found.push(after[..end].to_string());
        rest = &after[end + close.len()..];
    }
    found
}

fn sha256_source(text: &str) -> String {
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(text.as_bytes()))
    )
}

/// Every single-quoted string literal in the script, skipping whole-line
/// comments (their apostrophes are prose, not quotes).
fn quoted_literals(script: &str) -> Vec<String> {
    let code: String = script
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let script = code.as_str();
    let mut found = Vec::new();
    let mut chars = script.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c != '\'' {
            continue;
        }
        let mut end = None;
        while let Some((i, d)) = chars.next() {
            if d == '\\' {
                chars.next();
            } else if d == '\'' {
                end = Some(i);
                break;
            }
        }
        if let Some(end) = end {
            found.push(script[start + 1..end].to_string());
        }
    }
    found
}

/// The headers every step-up response carries, whatever it renders.
fn assert_strict_headers(reply: &Reply, csp: &str, what: &str) {
    assert_eq!(
        reply.header(axum::http::header::CONTENT_SECURITY_POLICY),
        Some(csp),
        "{what}: CSP"
    );
    assert_eq!(
        reply.header(axum::http::header::CACHE_CONTROL),
        Some("no-store"),
        "{what}: no-store"
    );
    assert_eq!(
        reply.header(axum::http::header::REFERRER_POLICY),
        Some("no-referrer"),
        "{what}: referrer"
    );
    assert_eq!(
        reply.header(axum::http::header::X_CONTENT_TYPE_OPTIONS),
        Some("nosniff"),
        "{what}: nosniff"
    );
    assert_eq!(
        reply.header(axum::http::header::X_FRAME_OPTIONS),
        Some("DENY"),
        "{what}: frame options"
    );
    // A page that opened this one in a popup keeps no handle to it, so it
    // cannot navigate the window once the person has signed in.
    assert_eq!(
        reply.header("cross-origin-opener-policy"),
        Some("same-origin"),
        "{what}: opener policy"
    );
    assert!(
        reply
            .header(axum::http::header::CONTENT_TYPE)
            .is_some_and(|v| v.starts_with("text/html")),
        "{what}: html"
    );
    // Whichever policy, it is closed by default, never framed, and never
    // allows inline code by blanket permission or any remote source.
    let directives: Vec<&str> = csp.split(';').map(str::trim).collect();
    for required in [
        "default-src 'none'",
        "frame-ancestors 'none'",
        "base-uri 'none'",
        "form-action 'none'",
    ] {
        assert!(directives.contains(&required), "{what}: {required}");
    }
    for forbidden in [
        "'unsafe-inline'",
        "'unsafe-eval'",
        "'unsafe-hashes'",
        "http:",
        "https:",
        "data:",
        "*",
    ] {
        assert!(!csp.contains(forbidden), "{what}: {forbidden} in {csp}");
    }
}

// --- In-memory -------------------------------------------------------------

/// The ceremony page: the strict headers, and a CSP that admits exactly the
/// one script and the one style block the page carries, by hash.
#[tokio::test]
async fn the_page_is_served_with_strict_headers_and_a_pinned_script() {
    let (state, _root) = available_state();
    let reply = send(&state, page_request("")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_strict_headers(&reply, &CSP_WITH_SCRIPT, "ceremony page");

    let html = reply.text();
    // One executable script, inline; the only other script element is the
    // copy table's data block, which is never executed.
    let scripts = inline_elements(&html, "<script>", "</script>");
    assert_eq!(scripts, vec![STEP_UP_SCRIPT.to_string()]);
    assert_eq!(html.matches("<script").count(), 2);
    assert_eq!(
        html.matches("<script type=\"application/json\" id=\"tc-copy\">")
            .count(),
        1
    );
    // No third-party (or any external) script, style, frame or image.
    for forbidden in ["src=", "href=", "<iframe", "<link", "<img", "<form"] {
        assert!(!html.contains(forbidden), "{forbidden} in the page");
    }
    let styles = inline_elements(&html, "<style>", "</style>");
    assert_eq!(styles, vec![STEP_UP_STYLE.to_string()]);

    // The policy names exactly those two hashes and fetches only same-origin.
    let csp = CSP_WITH_SCRIPT.as_str();
    let directives: Vec<&str> = csp.split(';').map(str::trim).collect();
    assert!(directives.contains(&format!("script-src {}", sha256_source(&scripts[0])).as_str()));
    assert!(directives.contains(&format!("style-src {}", sha256_source(&styles[0])).as_str()));
    assert!(directives.contains(&"connect-src 'self'"));
    assert!(directives.contains(&"require-trusted-types-for 'script'"));
    // The script cannot close its own element.
    assert!(!STEP_UP_SCRIPT.to_ascii_lowercase().contains("</script"));
}

/// Each allowlisted `return` selects its section, and the page carries the
/// allowlist's own label, not the request's bytes. No `return` is the menu.
#[tokio::test]
async fn each_allowlisted_return_selects_its_section() {
    let (state, _root) = available_state();
    let menu = send(&state, page_request("")).await;
    assert_eq!(menu.status, StatusCode::OK);
    assert!(menu.text().contains("<body data-action=\"\">"));

    for action in StepUpAction::ALL {
        let reply = send(&state, page_request(&format!("return={}", action.label()))).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", action.label());
        assert_strict_headers(&reply, &CSP_WITH_SCRIPT, action.label());
        assert!(
            reply
                .text()
                .contains(&format!("<body data-action=\"{}\">", action.label())),
            "{}",
            action.label()
        );
        // The label names a section the page has.
        assert!(
            reply
                .text()
                .contains(&format!("<section id=\"{}\" hidden>", action.label()))
        );
    }
    assert_eq!(
        StepUpAction::ALL.map(StepUpAction::label),
        ["add-passkey", "remove-passkey", "change-payout"]
    );
}

/// A `return` that is not exactly an allowlisted action is refused: a URL, a
/// path, a scheme, a near miss, a duplicate, or any other parameter. The
/// refusal runs no script and never echoes what it was given.
#[tokio::test]
async fn a_return_outside_the_allowlist_is_refused() {
    let (state, _root) = available_state();
    for query in [
        "return=https%3A%2F%2Fevil.example%2Fadd-passkey",
        "return=https://evil.example",
        "return=%2F%2Fevil.example",
        "return=%2Fv1%2Faccount%2Flogout",
        "return=javascript%3Aalert(1)",
        "return=ADD-PASSKEY",
        "return=add-passkey%20",
        "return=%20add-passkey",
        "return=add-passkeyevil",
        "return=add",
        "return=",
        "return",
        "return=add-passkey&return=change-payout",
        "return=add-passkey&return=add-passkey",
        "next=https%3A%2F%2Fevil.example",
        "return=add-passkey&next=https%3A%2F%2Fevil.example",
        "redirect_uri=https%3A%2F%2Fevil.example",
    ] {
        let reply = send(&state, page_request(query)).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        assert_strict_headers(&reply, &CSP_WITHOUT_SCRIPT, query);
        let html = reply.text();
        assert!(!html.contains("<script"), "{query}: runs a script");
        assert!(!html.contains("evil"), "{query}: echoed");
        assert!(!html.contains("data-action"), "{query}: selected an action");
        assert!(
            html.contains(crate::step_up_page::copy("invalid_heading")),
            "{query}"
        );
        assert!(reply.header(axum::http::header::LOCATION).is_none());
    }
}

/// Without a relying party, or without the account database, the page is a
/// plain unavailable notice: no script, no sign-in button, and a 503.
#[tokio::test]
async fn without_a_relying_party_or_database_the_page_is_unavailable() {
    let root = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = Arc::new(NativeAuthTestDb::default());
    let no_rp = test_state_with_options(
        root.path().to_path_buf(),
        Some(db),
        None,
        false,
        false,
        false,
        false,
    );
    let no_db = test_state_with_webauthn(root.path().to_path_buf(), None);
    for (what, state) in [("no relying party", no_rp), ("no database", no_db)] {
        for query in [
            "",
            "return=add-passkey",
            "return=https%3A%2F%2Fevil.example",
        ] {
            let reply = send(&state, page_request(query)).await;
            assert_eq!(
                reply.status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{what} {query}"
            );
            assert_strict_headers(&reply, &CSP_WITHOUT_SCRIPT, what);
            let html = reply.text();
            assert!(!html.contains("<script"), "{what}: runs a script");
            assert!(!html.contains("id=\"sign-in\""), "{what}: offers a sign-in");
            assert!(html.contains(crate::step_up_page::copy("unavailable_heading")));
            assert!(html.contains(crate::step_up_page::copy("unavailable_body")));
        }
    }
}

/// The page's sign-in start: the browser passkey login, asked for a
/// short-lived step-up session.
const STEP_UP_LOGIN_START: &str = "/account/passkey/login/start?purpose=step_up";

/// The routes the script calls, with the method it calls each with. This is
/// the whole of the page's API surface; every one already exists.
const SCRIPT_ROUTES: &[(&str, &str, &str)] = &[
    ("POST", "loginStart", STEP_UP_LOGIN_START),
    ("POST", "loginFinish", "/account/passkey/login/finish"),
    ("GET", "passkeys", "/v1/account/passkeys"),
    ("DELETE", "passkey", "/v1/account/passkeys/{credential_id}"),
    (
        "POST",
        "registerStart",
        "/v1/account/passkeys/register/start",
    ),
    (
        "POST",
        "registerFinish",
        "/v1/account/passkeys/register/finish",
    ),
    ("GET", "nearIdentities", "/v1/account/near-identities"),
    (
        "PATCH",
        "payout",
        "/v1/account/near-identities/{public_key}/payout",
    ),
    ("POST", "logout", "/v1/account/logout"),
];

/// The script names exactly the routes above, calls each with its method, and
/// every one is a route the router already serves. Nothing absolute, nothing
/// that writes HTML.
#[tokio::test]
async fn the_script_calls_only_existing_routes() {
    let paths: BTreeSet<String> = quoted_literals(STEP_UP_SCRIPT)
        .into_iter()
        .filter(|s| s.starts_with('/') && s[1..].starts_with(|c: char| c.is_ascii_alphabetic()))
        .collect();
    let expected: BTreeSet<String> = SCRIPT_ROUTES
        .iter()
        .map(|(_, _, path)| path.to_string())
        .collect();
    assert_eq!(paths, expected);

    for (method, key, _) in SCRIPT_ROUTES {
        let direct = format!("send('{method}', ROUTES.{key})");
        let direct_with_body = format!("send('{method}', ROUTES.{key},");
        let templated = format!("send('{method}', withId(ROUTES.{key},");
        let called = STEP_UP_SCRIPT.contains(&direct)
            || STEP_UP_SCRIPT.contains(&direct_with_body)
            || STEP_UP_SCRIPT.contains(&templated)
            || STEP_UP_SCRIPT.contains(&format!("'{method}',\n        ROUTES.{key},"));
        assert!(called, "{method} {key} is not called that way");
    }
    // Every fetch goes through `send`, and nothing reaches off-origin.
    assert_eq!(STEP_UP_SCRIPT.matches("fetch(").count(), 1);
    for forbidden in [
        "http:",
        "https:",
        "'//",
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "new Function",
        "location.",
        "window.open",
        "postMessage",
        "localStorage",
        "sessionStorage",
    ] {
        assert!(
            !STEP_UP_SCRIPT.contains(forbidden),
            "the script uses {forbidden}"
        );
    }

    // Each route exists on the router: an unauthenticated call is refused by
    // the route itself, never answered 404 or 405.
    let (state, _root) = available_state();
    for (method, _, path) in SCRIPT_ROUTES {
        let uri = path
            .replace("{credential_id}", "AAAA")
            .replace("{public_key}", "ed25519:abc");
        let reply = send(
            &state,
            axum::http::Request::builder()
                .method(*method)
                .uri(&uri)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert!(
            reply.status != StatusCode::NOT_FOUND && reply.status != StatusCode::METHOD_NOT_ALLOWED,
            "{method} {uri}: {}",
            reply.status
        );
    }
    // The check can fail: a route that does not exist is a 404 here.
    let missing = send(
        &state,
        axum::http::Request::builder()
            .method("POST")
            .uri("/account/step-up/finish")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(matches!(
        missing.status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ));
}

/// All copy lives in the one table, and the script names no key it lacks.
#[test]
fn every_copy_key_the_script_uses_is_in_the_one_table() {
    let keys: BTreeSet<&str> = STEP_UP_COPY.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys.len(), STEP_UP_COPY.len(), "duplicate copy key");
    let snake = |s: &str| {
        let mut parts = s.split('_');
        s.contains('_') && parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase()))
    };
    let used: BTreeSet<String> = quoted_literals(STEP_UP_SCRIPT)
        .into_iter()
        .filter(|s| snake(s))
        .collect();
    assert!(!used.is_empty());
    for key in &used {
        assert!(keys.contains(key.as_str()), "{key} is not in STEP_UP_COPY");
    }
    for (_, text) in STEP_UP_COPY {
        assert!(!text.is_empty());
        assert!(!text.contains('<'), "copy is text, not markup");
    }
}

// --- PostgreSQL --------------------------------------------------------------

const RP_ID: &str = "tracecommons.ai";
const NATIVE_ORIGIN: &str = "https://tracecommons.ai";
const INGEST_ORIGIN: &str = "https://ingest.tracecommons.ai";

type SoftAuthenticator = webauthn_authenticator_rs::WebauthnAuthenticator<
    webauthn_authenticator_rs::softpasskey::SoftPasskey,
>;

fn url(origin: &str) -> webauthn_rs::prelude::Url {
    webauthn_rs::prelude::Url::parse(origin).expect("origin")
}

/// The pilot's relying party with the ingest host on the origin list, as the
/// page needs it configured.
fn pilot_shaped_state(db: Arc<dyn Database>, ceiling: i64) -> (Arc<AppState>, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("temp dir");
    let mut state = test_state_with_options(
        root.path().to_path_buf(),
        Some(db),
        None,
        false,
        false,
        false,
        false,
    );
    let webauthn = build_webauthn(&WebauthnConfig {
        rp_id: RP_ID.to_string(),
        rp_origin: format!("{NATIVE_ORIGIN},{INGEST_ORIGIN}"),
        rp_name: "TraceCommons Test".to_string(),
    })
    .expect("relying party builds");
    let fresh = Arc::get_mut(&mut state).expect("fresh state is uniquely owned");
    fresh.account_webauthn = Some(Arc::new(webauthn));
    fresh.account_unbound_ceiling = Arc::new(UnboundAccountCeiling::with_limit(ceiling));
    (state, root)
}

/// A request the page's script would make: same-origin, from the ingest host.
fn page_fetch(
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<&serde_json::Value>,
) -> axum::http::Request<Body> {
    let mut builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header(axum::http::header::ORIGIN, INGEST_ORIGIN)
        .header("sec-fetch-site", "same-origin");
    if let Some(cookie) = cookie {
        builder = builder.header(axum::http::header::COOKIE, cookie);
    }
    match body {
        Some(body) => builder
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).expect("json")))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    }
}

fn bearer(method: &str, uri: &str, token: &str, body: Option<&str>) -> axum::http::Request<Body> {
    let builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {token}"));
    match body {
        Some(body) => builder
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    }
}

/// A discoverable assertion from `origin`. The SoftPasskey cannot discover a
/// credential or return a user handle, so both are supplied the way a real
/// resident-key authenticator would supply them (see
/// `softpasskey_authenticate_discoverable`); the signature, RP ID hash and
/// client data origin are the authenticator's own.
fn assert_from(
    authenticator: &mut SoftAuthenticator,
    challenge: &serde_json::Value,
    credential_id: &str,
    account_id: Uuid,
    origin: &str,
) -> serde_json::Value {
    let mut patched = challenge.clone();
    patched["publicKey"]["allowCredentials"] =
        serde_json::json!([{ "type": "public-key", "id": credential_id }]);
    let options: webauthn_rs::prelude::RequestChallengeResponse =
        serde_json::from_value(patched).expect("request challenge");
    let assertion = authenticator
        .do_authentication(url(origin), options)
        .expect("software authenticator asserts");
    let mut json = serde_json::to_value(&assertion).expect("assertion json");
    json["response"]["userHandle"] = serde_json::json!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(account_id.as_bytes())
    );
    json
}

/// The page's sign-in, as its script performs it, from `origin`. Returns the
/// finish reply.
async fn page_sign_in(
    state: &Arc<AppState>,
    authenticator: &mut SoftAuthenticator,
    credential_id: &str,
    account_id: Uuid,
    origin: &str,
) -> Reply {
    sign_in_through(
        state,
        STEP_UP_LOGIN_START,
        authenticator,
        credential_id,
        account_id,
        origin,
    )
    .await
}

/// A browser passkey sign-in that starts at `start_uri`, from `origin`.
async fn sign_in_through(
    state: &Arc<AppState>,
    start_uri: &str,
    authenticator: &mut SoftAuthenticator,
    credential_id: &str,
    account_id: Uuid,
    origin: &str,
) -> Reply {
    let start = send(state, page_fetch("POST", start_uri, None, None)).await;
    assert_eq!(start.status, StatusCode::OK, "login/start");
    assert_step_up_cookies_are_host_bound(
        &start,
        &[ACCOUNT_PASSKEY_CEREMONY_COOKIE],
        "login/start",
    );
    let ceremony = start
        .cookie_pair(ACCOUNT_PASSKEY_CEREMONY_COOKIE)
        .expect("ceremony cookie");
    let assertion = assert_from(
        authenticator,
        &start.json(),
        credential_id,
        account_id,
        origin,
    );
    send(
        state,
        page_fetch(
            "POST",
            "/account/passkey/login/finish",
            Some(&ceremony),
            Some(&assertion),
        ),
    )
    .await
}

/// Every `Set-Cookie` on `reply` is host-bound, and each of `expected` is
/// among them: `Secure`, `Path=/`, no `Domain`, `HttpOnly` and
/// `SameSite=Strict` -- the attributes a browser demands before it accepts a
/// `__Host-` name, plus the two the account cookies always carry. Names are
/// the constants, never a literal, so this holds whatever prefix they carry.
/// It is the step-up flow's copy of the shared host-bound check that #1138
/// adds, and folds into it once both land.
fn assert_step_up_cookies_are_host_bound(reply: &Reply, expected: &[&str], what: &str) {
    let values: Vec<&str> = reply
        .headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().expect("ascii Set-Cookie"))
        .collect();
    for name in expected {
        assert!(
            values
                .iter()
                .any(|raw| raw.starts_with(&format!("{name}="))),
            "{what}: no Set-Cookie for {name}"
        );
    }
    for raw in values {
        let parsed = cookie::Cookie::parse(raw.to_string()).expect("Set-Cookie parses");
        // The name only: the value is a live secret and stays out of output.
        let name = parsed.name();
        assert!(
            [ACCOUNT_SESSION_COOKIE, ACCOUNT_PASSKEY_CEREMONY_COOKIE].contains(&parsed.name()),
            "{what}: unexpected cookie {}",
            parsed.name()
        );
        assert_eq!(
            parsed.secure(),
            Some(true),
            "{what}: must be Secure: {name}"
        );
        assert_eq!(parsed.path(), Some("/"), "{what}: must be Path=/: {name}");
        assert_eq!(
            parsed.domain(),
            None,
            "{what}: must not carry a Domain: {name}"
        );
        assert!(
            !raw.to_ascii_lowercase().contains("domain="),
            "{what}: must not carry a Domain: {name}"
        );
        assert_eq!(
            parsed.http_only(),
            Some(true),
            "{what}: must be HttpOnly: {name}"
        );
        assert_eq!(
            parsed.same_site(),
            Some(cookie::SameSite::Strict),
            "{what}: must be SameSite=Strict: {name}"
        );
    }
}

/// The S7 end-to-end: the session the page's sign-in mints, from the ingest
/// origin on the origin list, is strong and can change an authenticator; the
/// account's native session cannot (the S2 gate), and a sign-in from an origin
/// that is not listed mints nothing.
#[tokio::test]
async fn pg_the_page_sign_in_is_strong_and_a_native_session_is_not() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    reset_account_rate_limiter_for_test();
    let current = backend
        .count_unbound_passkey_accounts()
        .await
        .expect("unbound count");
    let db: Arc<dyn Database> = backend.clone();
    let (state, _root) = pilot_shaped_state(db, current + 5);
    let mut admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");

    // A passkey-origin account, created natively from the RP origin, then
    // bound so only session strength decides what follows.
    let start = send(
        &state,
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/account/native/passkey/create/start")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"label":"Laptop"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.text());
    let start = start.json();
    let mut authenticator: SoftAuthenticator = new_software_authenticator();
    let options: webauthn_rs::prelude::CreationChallengeResponse =
        serde_json::from_value(start["public_key"].clone()).expect("creation challenge");
    let credential = authenticator
        .do_registration(url(NATIVE_ORIGIN), options)
        .expect("software authenticator registers");
    let created = send(
        &state,
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/account/native/passkey/create/finish")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "ceremony_id": start["ceremony_id"],
                    "credential": credential,
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text());
    let created = created.json();
    let native = created["access_token"].as_str().expect("token").to_string();
    let (tenant, _) = native_token_parts(&native).expect("tcn1_ token");
    let account_id = Uuid::parse_str(created["account_id"].as_str().unwrap()).unwrap();
    let credential_id = credential_id_to_string(&webauthn_rs::prelude::CredentialID::from(
        credential.raw_id.as_ref(),
    ));
    admin
        .execute(
            "UPDATE trace_account_bindings SET state = 'bound', bound_at = now()
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &account_id],
        )
        .await
        .expect("bind");

    // The native session is weak: every change the page exists for is refused.
    for (method, uri, body) in [
        (
            "POST",
            "/v1/account/passkeys/register/start".to_string(),
            None,
        ),
        (
            "DELETE",
            format!("/v1/account/passkeys/{credential_id}"),
            None,
        ),
        (
            "PATCH",
            "/v1/account/near-identities/ed25519:abc/payout".to_string(),
            Some(r#"{"payout":true}"#),
        ),
    ] {
        let reply = send(&state, bearer(method, &uri, &native, body)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "native {method} {uri}");
        assert_eq!(
            reply.json()["error"],
            "a passkey or NEAR sign-in is required to change authenticators"
        );
    }

    // The page is served for the action the app sent the person for.
    let page = send(&state, page_request("return=add-passkey")).await;
    assert_eq!(page.status, StatusCode::OK);

    // A sign-in whose client data names an origin not on the list mints
    // nothing, even though its RP ID is right.
    let stray = page_sign_in(
        &state,
        &mut authenticator,
        &credential_id,
        account_id,
        "https://other.tracecommons.ai",
    )
    .await;
    assert_eq!(stray.status, StatusCode::BAD_REQUEST);
    assert!(stray.cookie_pair(ACCOUNT_SESSION_COOKIE).is_none());

    // The page's sign-in from the ingest origin: a 303 carrying a strong
    // `passkey` session bound to the credential that asserted.
    let signed_in = page_sign_in(
        &state,
        &mut authenticator,
        &credential_id,
        account_id,
        INGEST_ORIGIN,
    )
    .await;
    assert_eq!(
        signed_in.status,
        StatusCode::SEE_OTHER,
        "{}",
        signed_in.text()
    );
    assert_step_up_cookies_are_host_bound(&signed_in, &[ACCOUNT_SESSION_COOKIE], "login/finish");
    let session = signed_in
        .cookie_pair(ACCOUNT_SESSION_COOKIE)
        .expect("session cookie");
    let cookie_value = session.split_once('=').unwrap().1.to_string();
    let (cookie_tenant, token_hash) =
        account_session_cookie_parts(&cookie_value).expect("session cookie parses");
    assert_eq!(cookie_tenant, tenant);
    assert_eq!(
        session_kind_and_credential(backend.as_ref(), &tenant, &token_hash).await,
        Some(("passkey".to_string(), Some(credential_id.clone())))
    );

    // That session adds a second passkey, from the ingest origin, through the
    // existing registration routes the page drives.
    let register = send(
        &state,
        page_fetch(
            "POST",
            "/v1/account/passkeys/register/start",
            Some(&session),
            None,
        ),
    )
    .await;
    assert_eq!(register.status, StatusCode::OK, "{}", register.text());
    assert_step_up_cookies_are_host_bound(
        &register,
        &[ACCOUNT_PASSKEY_CEREMONY_COOKIE],
        "register/start",
    );
    let ceremony = register
        .cookie_pair(ACCOUNT_PASSKEY_CEREMONY_COOKIE)
        .expect("ceremony cookie");
    let options: webauthn_rs::prelude::CreationChallengeResponse =
        serde_json::from_value(register.json()).expect("creation challenge");
    let mut second: SoftAuthenticator = new_software_authenticator();
    let attestation = second
        .do_registration(url(INGEST_ORIGIN), options)
        .expect("software authenticator registers");
    let mut finish_body = serde_json::to_value(&attestation).expect("attestation json");
    finish_body["label"] = serde_json::json!("Phone");
    let finished = send(
        &state,
        page_fetch(
            "POST",
            "/v1/account/passkeys/register/finish",
            Some(&format!("{session}; {ceremony}")),
            Some(&finish_body),
        ),
    )
    .await;
    assert_eq!(finished.status, StatusCode::OK, "{}", finished.text());
    assert_step_up_cookies_are_host_bound(&finished, &[], "register/finish");
    let second_id = finished.json()["credential_id"]
        .as_str()
        .expect("credential id")
        .to_string();

    let listed = send(
        &state,
        page_fetch("GET", "/v1/account/passkeys", Some(&session), None),
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK);
    let passkeys = listed.json()["passkeys"].as_array().cloned().unwrap();
    assert_eq!(passkeys.len(), 2);
    assert!(
        passkeys
            .iter()
            .any(|p| p["credential_id"] == credential_id.as_str() && p["this_device"] == true)
    );

    // The payout change passes the gate (the 404 is the uniform "no such
    // identity", reached only after the strength check).
    let payout = send(
        &state,
        page_fetch(
            "PATCH",
            "/v1/account/near-identities/ed25519:abc/payout",
            Some(&session),
            Some(&serde_json::json!({ "payout": true })),
        ),
    )
    .await;
    assert_eq!(payout.status, StatusCode::NOT_FOUND, "{}", payout.text());

    // And it removes the passkey it added.
    let removed = send(
        &state,
        page_fetch(
            "DELETE",
            &format!("/v1/account/passkeys/{second_id}"),
            Some(&session),
            None,
        ),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.text());

    // The native token is still weak afterwards: nothing crossed from the
    // browser session into it.
    let still_weak = send(
        &state,
        bearer("POST", "/v1/account/passkeys/register/start", &native, None),
    )
    .await;
    assert_eq!(still_weak.status, StatusCode::FORBIDDEN);

    // Four gate refusals, all from the native session, label-only.
    let tx = admin.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant ctx");
    let denied: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_account_audit
              WHERE tenant_id = $1 AND action = 'account_authenticator_gate_denied'
                AND safe_metadata = '{}'::jsonb",
            &[&tenant],
        )
        .await
        .expect("count")
        .get(0);
    tx.commit().await.expect("commit");
    assert_eq!(denied, 4);

    admin
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("cleanup");
}

// --- Step-up session lifetime ---------------------------------------------

/// `purpose` on the browser login start is an allowlist of one. Anything else
/// is the uniform passkey-login deny, byte for byte; no purpose is today's
/// sign-in.
#[tokio::test]
async fn a_login_purpose_outside_the_allowlist_is_refused() {
    reset_account_rate_limiter_for_test();
    let (state, _root) = available_state();
    let deny = passkey_login_generic_deny();
    let deny_status = deny.status();
    let deny_bytes = axum::body::to_bytes(deny.into_body(), 1024)
        .await
        .expect("body")
        .to_vec();
    for query in [
        "purpose=other",
        "purpose=STEP_UP",
        "purpose=",
        "purpose=step_up&purpose=step_up",
        "purpose=step_up%00",
    ] {
        let reply = send(
            &state,
            page_fetch(
                "POST",
                &format!("/account/passkey/login/start?{query}"),
                None,
                None,
            ),
        )
        .await;
        assert_eq!(reply.status, deny_status, "{query}");
        assert_eq!(reply.bytes, deny_bytes, "{query}");
        assert!(reply.cookie_pair(ACCOUNT_PASSKEY_CEREMONY_COOKIE).is_none());
    }
    for uri in ["/account/passkey/login/start", STEP_UP_LOGIN_START] {
        let reply = send(&state, page_fetch("POST", uri, None, None)).await;
        assert_eq!(reply.status, StatusCode::OK, "{uri}");
        assert!(reply.cookie_pair(ACCOUNT_PASSKEY_CEREMONY_COOKIE).is_some());
    }
    reset_account_rate_limiter_for_test();
}

/// A bound passkey-origin account, created natively, and what a browser
/// sign-in with its passkey needs.
struct StepUpAccount {
    tenant: String,
    account_id: Uuid,
    credential_id: String,
    authenticator: SoftAuthenticator,
}

async fn step_up_account(
    state: &Arc<AppState>,
    admin: &deadpool_postgres::Object,
) -> StepUpAccount {
    let start = send(
        state,
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/account/native/passkey/create/start")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.text());
    let start = start.json();
    let mut authenticator: SoftAuthenticator = new_software_authenticator();
    let options: webauthn_rs::prelude::CreationChallengeResponse =
        serde_json::from_value(start["public_key"].clone()).expect("creation challenge");
    let credential = authenticator
        .do_registration(url(NATIVE_ORIGIN), options)
        .expect("software authenticator registers");
    let created = send(
        state,
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/account/native/passkey/create/finish")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "ceremony_id": start["ceremony_id"],
                    "credential": credential,
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text());
    let created = created.json();
    let (tenant, _) =
        native_token_parts(created["access_token"].as_str().expect("token")).expect("tcn1_");
    let account_id = Uuid::parse_str(created["account_id"].as_str().unwrap()).unwrap();
    admin
        .execute(
            "UPDATE trace_account_bindings SET state = 'bound', bound_at = now()
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &account_id],
        )
        .await
        .expect("bind");
    StepUpAccount {
        tenant,
        account_id,
        credential_id: credential_id_to_string(&webauthn_rs::prelude::CredentialID::from(
            credential.raw_id.as_ref(),
        )),
        authenticator,
    }
}

/// Sign in from the ingest origin through `start_uri`; returns the whole
/// `Set-Cookie` line of the session cookie and its `name=value` pair.
async fn browser_session(
    state: &Arc<AppState>,
    account: &mut StepUpAccount,
    start_uri: &str,
) -> (String, String) {
    let reply = sign_in_through(
        state,
        start_uri,
        &mut account.authenticator,
        &account.credential_id,
        account.account_id,
        INGEST_ORIGIN,
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    session_set_cookie(&reply).expect("session cookie")
}

/// The session cookie's full `Set-Cookie` line and its `name=value` pair.
fn session_set_cookie(reply: &Reply) -> Option<(String, String)> {
    let line = reply
        .headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(&format!("{ACCOUNT_SESSION_COOKIE}=")))?
        .to_string();
    let pair = line.split(';').next()?.to_string();
    Some((line, pair))
}

fn max_age(set_cookie: &str) -> i64 {
    set_cookie
        .split(';')
        .find_map(|part| part.trim().strip_prefix("Max-Age="))
        .and_then(|v| v.parse().ok())
        .expect("Max-Age")
}

/// `(expires_at - created_at, expires_at)` of the account's one session row,
/// the lifetime in whole seconds.
async fn session_lifetime(
    admin: &mut deadpool_postgres::Object,
    tenant: &str,
) -> (i64, chrono::DateTime<Utc>) {
    let tx = admin.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant ctx");
    let row = tx
        .query_one(
            "SELECT extract(epoch FROM expires_at - created_at)::BIGINT, expires_at
               FROM trace_sessions
              WHERE tenant_id = $1 AND client_kind = 'passkey'",
            &[&tenant],
        )
        .await
        .expect("one browser session");
    tx.commit().await.expect("commit");
    (row.get(0), row.get(1))
}

/// Move the clock forward by `secs` for every session in `tenant`: each of the
/// row's timestamps moves back by the same amount, which is what the server
/// sees when that much time has passed. The same device the other session
/// tests use (they backdate `last_seen_at` or `token_issued_at`).
async fn advance_session_clock(admin: &mut deadpool_postgres::Object, tenant: &str, secs: i64) {
    let tx = admin.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant ctx");
    tx.execute(
        "UPDATE trace_sessions
            SET created_at = created_at - make_interval(secs => $2),
                expires_at = expires_at - make_interval(secs => $2),
                last_seen_at = last_seen_at - make_interval(secs => $2),
                token_issued_at = token_issued_at - make_interval(secs => $2),
                prev_token_valid_until = prev_token_valid_until - make_interval(secs => $2)
          WHERE tenant_id = $1",
        &[&tenant, &(secs as f64)],
    )
    .await
    .expect("advance");
    tx.commit().await.expect("commit");
}

async fn passkeys_with(state: &Arc<AppState>, cookie: &str) -> Reply {
    send(
        state,
        page_fetch("GET", "/v1/account/passkeys", Some(cookie), None),
    )
    .await
}

async fn pg_step_up_state() -> Option<(
    Arc<PgBackend>,
    Arc<AppState>,
    tempfile::TempDir,
    deadpool_postgres::Object,
)> {
    let backend = postgres_backend_for_ingest_test().await?;
    reset_account_rate_limiter_for_test();
    let current = backend
        .count_unbound_passkey_accounts()
        .await
        .expect("unbound count");
    let db: Arc<dyn Database> = backend.clone();
    let (state, root) = pilot_shaped_state(db, current + 5);
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    Some((backend, state, root, admin))
}

const STEP_UP_TTL_SECS: i64 = STEP_UP_SESSION_TTL_MINUTES * 60;

/// The page's sign-in mints a session that lasts the step-up TTL: the cookie
/// says so, the row says so, and the session is refused once that much time
/// has passed, not before.
#[tokio::test]
async fn pg_a_step_up_session_expires_after_its_ttl() {
    let Some((_backend, state, _root, mut admin)) = pg_step_up_state().await else {
        return;
    };
    assert_eq!(STEP_UP_TTL_SECS, 15 * 60);
    let mut account = step_up_account(&state, &admin).await;
    let (set_cookie, session) = browser_session(&state, &mut account, STEP_UP_LOGIN_START).await;
    assert_eq!(max_age(&set_cookie), STEP_UP_TTL_SECS, "{set_cookie}");
    let (lifetime, _) = session_lifetime(&mut admin, &account.tenant).await;
    assert!(
        (STEP_UP_TTL_SECS - 5..=STEP_UP_TTL_SECS).contains(&lifetime),
        "row lifetime {lifetime}s"
    );

    assert_eq!(passkeys_with(&state, &session).await.status, StatusCode::OK);
    advance_session_clock(&mut admin, &account.tenant, STEP_UP_TTL_SECS - 60).await;
    assert_eq!(
        passkeys_with(&state, &session).await.status,
        StatusCode::OK,
        "a minute before the TTL"
    );
    advance_session_clock(&mut admin, &account.tenant, 61).await;
    assert_eq!(
        passkeys_with(&state, &session).await.status,
        StatusCode::UNAUTHORIZED,
        "past the TTL"
    );

    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .expect("cleanup");
}

/// A browser passkey sign-in that does not ask for step-up keeps today's
/// seven days, in the cookie and the row, and outlives the step-up TTL.
#[tokio::test]
async fn pg_a_sign_in_without_the_step_up_purpose_is_unchanged() {
    let Some((_backend, state, _root, mut admin)) = pg_step_up_state().await else {
        return;
    };
    let mut account = step_up_account(&state, &admin).await;
    let (set_cookie, session) =
        browser_session(&state, &mut account, "/account/passkey/login/start").await;
    let seven_days = ACCOUNT_SESSION_TTL_DAYS * 24 * 60 * 60;
    assert_eq!(max_age(&set_cookie), seven_days, "{set_cookie}");
    let (lifetime, _) = session_lifetime(&mut admin, &account.tenant).await;
    assert!(
        (seven_days - 5..=seven_days).contains(&lifetime),
        "row lifetime {lifetime}s"
    );
    advance_session_clock(&mut admin, &account.tenant, STEP_UP_TTL_SECS + 60).await;
    assert_eq!(
        passkeys_with(&state, &session).await.status,
        StatusCode::OK,
        "an ordinary session outlives the step-up TTL"
    );

    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .expect("cleanup");
}

/// Rotation-on-use cannot extend a step-up session: the rotated cookie's
/// Max-Age is capped at what is left of the session, the row's absolute
/// expiry does not move, and the rotated secret is refused once the TTL has
/// passed. Activity (the idle window) does not move it either.
#[tokio::test]
async fn pg_rotation_cannot_extend_a_step_up_session() {
    let Some((_backend, state, _root, mut admin)) = pg_step_up_state().await else {
        return;
    };
    let mut account = step_up_account(&state, &admin).await;
    let (_, session) = browser_session(&state, &mut account, STEP_UP_LOGIN_START).await;
    let (_, expires_before) = session_lifetime(&mut admin, &account.tenant).await;

    // Five minutes in, the token has aged past the rotation interval (the way
    // the rotation tests force it), so the next request rotates.
    advance_session_clock(&mut admin, &account.tenant, 5 * 60).await;
    admin
        .execute(
            "UPDATE trace_sessions SET token_issued_at = now() - interval '13 hours'
              WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .expect("age the token");
    let rotated = passkeys_with(&state, &session).await;
    assert_eq!(rotated.status, StatusCode::OK);
    assert_step_up_cookies_are_host_bound(&rotated, &[ACCOUNT_SESSION_COOKIE], "rotation");
    let (rotated_line, rotated_session) =
        session_set_cookie(&rotated).expect("rotation set a new session cookie");
    assert_ne!(rotated_session, session, "the secret rotated");
    let left = STEP_UP_TTL_SECS - 5 * 60;
    let rotated_max_age = max_age(&rotated_line);
    assert!(
        (left - 5..=left).contains(&rotated_max_age),
        "rotated cookie Max-Age {rotated_max_age}s, {left}s left"
    );
    let (_, expires_after) = session_lifetime(&mut admin, &account.tenant).await;
    assert_eq!(
        expires_after,
        expires_before - chrono::Duration::seconds(5 * 60),
        "rotation leaves the absolute expiry where the clock put it"
    );

    // More activity, then past the TTL: refused, old secret and new.
    assert_eq!(
        passkeys_with(&state, &rotated_session).await.status,
        StatusCode::OK
    );
    advance_session_clock(&mut admin, &account.tenant, left + 1).await;
    assert_eq!(
        passkeys_with(&state, &rotated_session).await.status,
        StatusCode::UNAUTHORIZED,
        "the rotated secret past the TTL"
    );
    assert_eq!(
        passkeys_with(&state, &session).await.status,
        StatusCode::UNAUTHORIZED,
        "the original secret past the TTL"
    );

    admin
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .expect("cleanup");
}

// --- The script's hand-built finish bodies ----------------------------------

/// The finish bodies exactly as `step_up_page.js` builds them by hand: the
/// literal text, not a value serialized from webauthn-rs. The PostgreSQL tests
/// above build theirs with serde, which always emits every field the server
/// wants, so they cannot notice the script drifting from it.
/// `scripts/ci/test-step-up-page.cjs` checks from the other side that the
/// script builds exactly these keys, so the one file pins both ends.
const SCRIPT_FINISH_BODIES: &str = include_str!("step_up_finish_bodies.json");

fn script_finish_body(name: &str) -> String {
    let all: serde_json::Value =
        serde_json::from_str(SCRIPT_FINISH_BODIES).expect("finish-body fixture is JSON");
    let body = all.get(name).unwrap_or_else(|| panic!("no fixture {name}"));
    body.to_string()
}

/// The login finish body the script builds is a `PublicKeyCredential`, the
/// type the `login/finish` extractor reads, with or without a user handle.
#[test]
fn the_script_login_finish_body_deserializes() {
    let with: webauthn_rs::prelude::PublicKeyCredential =
        serde_json::from_str(&script_finish_body("login_finish")).expect("login/finish body");
    assert_eq!(with.id, "AQIDBA");
    assert_eq!(with.raw_id.as_ref(), &[1, 2, 3, 4]);
    assert_eq!(with.type_, "public-key");
    assert_eq!(with.response.authenticator_data.as_ref(), &[0, 1, 2]);
    assert_eq!(with.response.client_data_json.as_ref(), b"{}");
    let handle: Vec<u8> = (0u8..16).collect();
    assert_eq!(with.get_user_unique_id(), Some(handle.as_slice()));

    let without: webauthn_rs::prelude::PublicKeyCredential =
        serde_json::from_str(&script_finish_body("login_finish_without_user_handle"))
            .expect("login/finish body with a null userHandle");
    assert_eq!(without.get_user_unique_id(), None);
}

/// The register finish body the script builds is the
/// `AccountPasskeyRegisterFinishBody` the `register/finish` route reads, with
/// the optional label beside the flattened credential.
#[test]
fn the_script_register_finish_body_deserializes() {
    let labelled: AccountPasskeyRegisterFinishBody =
        serde_json::from_str(&script_finish_body("register_finish")).expect("register/finish body");
    assert_eq!(labelled.label.as_deref(), Some("Phone"));
    assert_eq!(labelled.credential.id, "AQIDBA");
    assert_eq!(labelled.credential.raw_id.as_ref(), &[1, 2, 3, 4]);
    assert_eq!(labelled.credential.type_, "public-key");
    assert_eq!(
        labelled.credential.response.client_data_json.as_ref(),
        b"{}"
    );
    assert!(!labelled.credential.response.attestation_object.is_empty());

    let unlabelled: AccountPasskeyRegisterFinishBody =
        serde_json::from_str(&script_finish_body("register_finish_without_label"))
            .expect("register/finish body without a label");
    assert_eq!(unlabelled.label, None);
}
