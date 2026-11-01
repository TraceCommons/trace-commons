// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Passkeys are removed (owner decision 2026-10-31). Every route that served
//! them is unregistered, so an older client that still calls one gets the
//! router's plain 404 and nothing else: no handler runs, no session is read,
//! nothing is written. These tests pin that, for the unauthenticated routes
//! and the authenticated ones alike, and pin that no passkey route is left on
//! the recorded account surface.

use super::*;
use axum::body::Body;
use tower::ServiceExt;

/// Every route the passkey surface served, as `(method, concrete path)`.
const REMOVED_ROUTES: &[(&str, &str)] = &[
    // Browser passkey sign-in and the step-up page.
    ("POST", "/account/passkey/login/start"),
    ("POST", "/account/passkey/login/finish"),
    ("GET", "/account/step-up"),
    ("GET", "/account/step-up?return=payout"),
    // Passkey management on a signed-in account.
    ("GET", "/v1/account/passkeys"),
    ("POST", "/v1/account/passkeys/register/start"),
    ("POST", "/v1/account/passkeys/register/finish"),
    ("POST", "/v1/account/passkeys/native/register/start"),
    ("POST", "/v1/account/passkeys/native/register/finish"),
    ("PATCH", "/v1/account/passkeys/AAAA"),
    ("DELETE", "/v1/account/passkeys/AAAA"),
    // Native passkey account creation and sign-in.
    ("POST", "/v1/account/native/passkey/create/start"),
    ("POST", "/v1/account/native/passkey/create/finish"),
    ("POST", "/v1/account/native/passkey/login/start"),
    ("POST", "/v1/account/native/passkey/login/finish"),
    // Binding state and binding a passkey account to NEAR AI.
    ("GET", "/v1/account/binding"),
    ("POST", "/v1/account/near-ai/provision/bind/start"),
    ("POST", "/v1/account/near-ai/provision/bind/finish"),
];

async fn status_of(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    auth: Option<&str>,
) -> StatusCode {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(cookie) = auth {
        builder = builder.header(axum::http::header::COOKIE, cookie);
    }
    let body = if method == "GET" || method == "DELETE" {
        Body::empty()
    } else {
        builder = builder.header(CONTENT_TYPE, "application/json");
        Body::from("{}")
    };
    app(state.clone())
        .oneshot(builder.body(body).expect("request builds"))
        .await
        .expect("router is infallible")
        .status()
}

/// An older client calling any removed route gets 404, with or without a
/// session credential, and the request reaches no handler: the in-memory
/// database records no account audit row.
#[tokio::test]
async fn every_removed_passkey_route_answers_404() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = Arc::new(NativeAuthTestDb::default());
    let state = test_state_with_db(temp.path().to_path_buf(), db.clone() as Arc<dyn Database>);
    let cookie = format!("{ACCOUNT_SESSION_COOKIE}=dGVuYW50LWE.not-a-session");
    for (method, uri) in REMOVED_ROUTES {
        for auth in [None, Some(cookie.as_str())] {
            assert_eq!(
                status_of(&state, method, uri, auth).await,
                StatusCode::NOT_FOUND,
                "{method} {uri} (credential: {})",
                auth.is_some()
            );
        }
    }
    assert!(
        db.account_audits.lock().unwrap().is_empty(),
        "a removed route reached a handler"
    );
}

/// A control for the test above: the same router still serves a route that
/// stays, so a 404 there is about the removed paths, not a broken router.
#[tokio::test]
async fn the_routes_that_stay_are_still_served() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = Arc::new(NativeAuthTestDb::default());
    let state = test_state_with_db(temp.path().to_path_buf(), db as Arc<dyn Database>);
    // Behind the account middleware: no credential is a 401, not a 404.
    assert_eq!(
        status_of(&state, "GET", "/v1/account/near-identities", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(&state, "POST", "/v1/account/logout", None).await,
        StatusCode::UNAUTHORIZED
    );
}

/// No passkey, binding or bind route is left on the recorded account surface.
#[test]
fn the_account_surface_holds_no_passkey_route() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let surface = authenticated_account_surface(state);
    let leftovers: Vec<_> = surface
        .registered()
        .iter()
        .filter(|route| {
            route.path.contains("passkey")
                || route.path == "/v1/account/binding"
                || route.path.contains("/provision/bind/")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(
        !surface.registered().is_empty(),
        "the account surface was recorded"
    );
}

/// The environment variables the passkey surface read are ignored, never a
/// startup failure: nothing in the service reads them any more, so a pilot or
/// staging env file that still sets them boots unchanged.
#[test]
fn the_retired_passkey_variables_are_read_nowhere() {
    let sources = [
        include_str!("../../trace-commons-ingest.rs"),
        include_str!("../near_provisioning.rs"),
        include_str!("../account_routes.rs"),
        include_str!("../../../../../trace-commons-server/src/config.rs"),
        include_str!("../../../../../trace-commons-server/src/account_ceremony.rs"),
        include_str!("../../../../../trace-commons-server/src/db/postgres.rs"),
    ];
    for retired in [
        "TRACE_COMMONS_WEBAUTHN_RP_ID",
        "TRACE_COMMONS_WEBAUTHN_RP_ORIGIN",
        "TRACE_COMMONS_WEBAUTHN_RP_NAME",
        "TRACE_COMMONS_UNBOUND_PASSKEY_ACCOUNT_CEILING",
        "TRACE_COMMONS_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY",
        "TRACE_COMMONS_UNBOUND_REAPER_",
    ] {
        for source in sources {
            assert!(!source.contains(retired), "{retired} is still read");
        }
    }
}
