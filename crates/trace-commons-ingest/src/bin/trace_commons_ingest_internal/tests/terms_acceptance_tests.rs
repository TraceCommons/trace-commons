// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/account/terms-acceptance`: authentication, the same-origin rule
//! for cookie sessions, validation labels, and the PostgreSQL-backed record.

use super::*;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_protocol::terms::{
    TERMS_ACCEPTANCE_PATH, TermsAcceptanceRequest, TermsAcceptanceResponse,
};

fn session_ctx(method: AccountAuthMethod) -> AccountCtx {
    let account = Uuid::new_v4();
    AccountCtx {
        account_id: trace_commons_server::account_session::AccountId::from_uuid(account),
        principal_set:
            trace_commons_server::account_session::AccountPrincipalSet::from_iter_for_test_only(
                Vec::<String>::new(),
            ),
        auth_method: method,
        tenant_id: "tenant-a".to_string(),
        actor_ref: format!("account-actor:{account}"),
        auth_credential_id: None,
        client_kind: "near".into(),
        session_token_hash: None,
    }
}

fn request(accepted_at: chrono::DateTime<chrono::Utc>) -> TermsAcceptanceRequest {
    TermsAcceptanceRequest {
        terms_version: "2026-08-22-draft".into(),
        content_sha256: "ab".repeat(32),
        accepted_at,
    }
}

async fn post(
    state: &Arc<AppState>,
    headers: HeaderMap,
    body: serde_json::Value,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method("POST")
        .uri(TERMS_ACCEPTANCE_PATH)
        .header(CONTENT_TYPE, "application/json");
    for (name, value) in headers.iter() {
        builder = builder.header(name, value);
    }
    app(state.clone())
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .expect("router is infallible")
}

#[tokio::test]
async fn terms_acceptance_requires_a_session() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let body = serde_json::to_value(request(chrono::Utc::now())).unwrap();
    let response = post(&state, HeaderMap::new(), body.clone()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    // A device bearer is not an account session on this surface either.
    let mut bearer = HeaderMap::new();
    bearer.insert(AUTHORIZATION, HeaderValue::from_static("Bearer token-a"));
    let response = post(&state, bearer, body).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_cross_site_cookie() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let mut headers = HeaderMap::new();
    headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
    let err = terms_acceptance::handler(
        State(state),
        Extension(session_ctx(AccountAuthMethod::SessionCookie)),
        headers,
        Json(request(chrono::Utc::now())),
    )
    .await
    .expect_err("cross-site cookie refused");
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(err.1.0.error, "cross-origin account mutation");
}

#[tokio::test]
async fn rejects_bad_hash_and_future_time() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let mut bad_hash = request(chrono::Utc::now());
    bad_hash.content_sha256 = "NOT-A-HASH".into();
    let err = terms_acceptance::handler(
        State(state.clone()),
        Extension(session_ctx(AccountAuthMethod::NativeToken)),
        HeaderMap::new(),
        Json(bad_hash),
    )
    .await
    .expect_err("bad hash refused");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1.0.error, "terms-hash-invalid");

    let future = request(chrono::Utc::now() + chrono::Duration::minutes(6));
    let err = terms_acceptance::handler(
        State(state),
        Extension(session_ctx(AccountAuthMethod::NativeToken)),
        HeaderMap::new(),
        Json(future),
    )
    .await
    .expect_err("future time refused");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1.0.error, "terms-accepted-at-future");
}

#[tokio::test]
async fn rejects_unknown_fields() {
    // The body type refuses unknown fields; the handler never sees one.
    let with_extra = serde_json::json!({
        "terms_version": "v1",
        "content_sha256": "ab".repeat(32),
        "accepted_at": "2026-11-09T00:00:00Z",
        "account_id": "00000000-0000-0000-0000-000000000000"
    });
    assert!(serde_json::from_value::<TermsAcceptanceRequest>(with_extra.clone()).is_err());

    // Through the router, behind a real session when PostgreSQL is available.
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = backend.clone();
    let state = test_state_with_db(temp.path().to_path_buf(), db);
    let headers = account_session_headers(&state, "token-a").await;
    let response = post(&state, headers, with_extra).await;
    assert!(
        !response.status().is_success(),
        "unknown field must not be accepted, got {}",
        response.status()
    );
}

#[tokio::test]
async fn records_once_and_repeats_idempotently() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = backend.clone();
    let state = test_state_with_db(temp.path().to_path_buf(), db);
    let headers = account_session_headers(&state, "token-a").await;
    let body = serde_json::to_value(request(chrono::Utc::now())).unwrap();

    let mut recorded = Vec::new();
    for _ in 0..2 {
        let response = post(&state, headers.clone(), body.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body reads");
        let parsed: TermsAcceptanceResponse = serde_json::from_slice(&bytes).expect("response");
        recorded.push(parsed.recorded_at);
    }
    assert_eq!(recorded[0], recorded[1], "a repeat returns the first time");
}

#[test]
fn route_is_registered_as_post() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let surface = authenticated_account_surface(state);
    let methods: Vec<&str> = surface
        .registered()
        .iter()
        .filter(|route| route.path == TERMS_ACCEPTANCE_PATH)
        .map(|route| route.method)
        .collect();
    assert_eq!(methods, vec!["POST"]);
}
