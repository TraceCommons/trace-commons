// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::json;

#[test]
fn tenant_access_grant_env_decouples_read_from_require() {
    // Neither env: no DB attached, no enforcement (unchanged default).
    assert_eq!(
        super::tenant_access_grant_env_decision(false, false),
        (false, false)
    );
    // REQUIRE alone: attach DB and enforce (legacy behavior preserved).
    assert_eq!(
        super::tenant_access_grant_env_decision(true, false),
        (true, true)
    );
    // GRANT_DB alone: attach DB for reading, do NOT enforce (the new path).
    assert_eq!(
        super::tenant_access_grant_env_decision(false, true),
        (true, false)
    );
    // Both: attach and enforce.
    assert_eq!(
        super::tenant_access_grant_env_decision(true, true),
        (true, true)
    );
}
use std::collections::BTreeMap;
use tower::ServiceExt;
use uuid::Uuid;

use super::*;
use crate::trace_corpus_storage::{
    TraceTenantAccessGrantRecord, TraceTenantAccessGrantRole, TraceTenantAccessGrantStatus,
};
use trace_commons_protocol::onboarding::TRACE_ONBOARD_REQUEST_SCHEMA_VERSION;
use trace_commons_protocol::trace_contribution::{ConsentScope, TraceAllowedUse};

#[test]
fn intersect_empty_request_grants_full_ceiling() {
    let ceiling = vec![
        ConsentScope::DebuggingEvaluation,
        ConsentScope::ModelTraining,
    ];
    let got =
        intersect_requested_with_ceiling(&[], &ceiling, "consent scopes not permitted").unwrap();
    assert_eq!(got, ceiling);
}

#[test]
fn intersect_clips_to_ceiling_and_rejects_empty() {
    let ceiling = vec![
        ConsentScope::DebuggingEvaluation,
        ConsentScope::PublicAttribution,
    ];
    let got = intersect_requested_with_ceiling(
        &[
            ConsentScope::ModelTraining,
            ConsentScope::DebuggingEvaluation,
        ],
        &ceiling,
        "consent scopes not permitted",
    )
    .unwrap();
    assert_eq!(got, vec![ConsentScope::DebuggingEvaluation]);
    let err = intersect_requested_with_ceiling(
        &[ConsentScope::ModelTraining],
        &ceiling,
        "consent scopes not permitted",
    )
    .unwrap_err();
    // IssuerError renders {"error": label}; assert the label text.
    assert!(format!("{err:?}").contains("consent scopes not permitted"));
}

#[test]
fn uses_implied_by_scopes_maps_each_scope_and_always_appends_aggregate_analytics() {
    assert_eq!(
        uses_implied_by_scopes(&[ConsentScope::DebuggingEvaluation]),
        vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::AggregateAnalytics
        ]
    );
    assert_eq!(
        uses_implied_by_scopes(&[ConsentScope::BenchmarkOnly]),
        vec![
            TraceAllowedUse::BenchmarkGeneration,
            TraceAllowedUse::AggregateAnalytics
        ]
    );
    assert_eq!(
        uses_implied_by_scopes(&[ConsentScope::RankingTraining]),
        vec![
            TraceAllowedUse::RankingModelTraining,
            TraceAllowedUse::AggregateAnalytics
        ]
    );
    assert_eq!(
        uses_implied_by_scopes(&[ConsentScope::ModelTraining]),
        vec![
            TraceAllowedUse::ModelTraining,
            TraceAllowedUse::AggregateAnalytics
        ]
    );
    assert_eq!(
        uses_implied_by_scopes(&[ConsentScope::PublicAttribution]),
        vec![TraceAllowedUse::AggregateAnalytics]
    );
}

#[test]
fn resolve_granted_uses_caps_empty_request_to_scope_implied_uses() {
    // Ceiling includes model_training, but the granted scopes only cover
    // debugging_evaluation — an empty allowed_uses request must not walk
    // away with model_training just because the ceiling allows it.
    let ceiling = vec![
        TraceAllowedUse::Debugging,
        TraceAllowedUse::Evaluation,
        TraceAllowedUse::ModelTraining,
        TraceAllowedUse::AggregateAnalytics,
    ];
    let granted_scopes = vec![ConsentScope::DebuggingEvaluation];
    let got = resolve_granted_uses(&[], &granted_scopes, &ceiling).unwrap();
    assert_eq!(
        got,
        vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::AggregateAnalytics
        ]
    );
    assert!(!got.contains(&TraceAllowedUse::ModelTraining));
}

#[test]
fn resolve_granted_uses_keeps_intersection_behavior_for_non_empty_request() {
    let ceiling = vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation];
    let granted_scopes = vec![ConsentScope::DebuggingEvaluation];
    let got =
        resolve_granted_uses(&[TraceAllowedUse::Debugging], &granted_scopes, &ceiling).unwrap();
    assert_eq!(got, vec![TraceAllowedUse::Debugging]);
}

#[test]
fn normalize_subject_accepts_pseudonymous_token() {
    let s = normalize_subject("  tenant_sha256:ab12CD_-  ").expect("valid");
    assert_eq!(s, "tenant_sha256:ab12CD_-");
}

#[test]
fn normalize_subject_rejects_empty_and_oversized_and_bad_chars() {
    assert!(normalize_subject("   ").is_err());
    assert!(normalize_subject(&"a".repeat(129)).is_err());
    assert!(normalize_subject("has space").is_err());
    assert!(normalize_subject("bad/slash").is_err());
}

const TEST_EDDSA_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIAGfN68ko7YyCGJMb3lHVwTn5aiUtbIsAclIx/lX0p2R\n-----END PRIVATE KEY-----\n";
const TEST_EDDSA_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAMnniSMeHZrdoe3gkL7ZeHmG7vAg65c5TqaBd71B2qDw=\n-----END PUBLIC KEY-----\n";
const WORKLOAD_EDDSA_PRIVATE_KEY_PEM: &str = TEST_EDDSA_PRIVATE_KEY_PEM;
const WORKLOAD_EDDSA_PUBLIC_KEY_PEM: &str = TEST_EDDSA_PUBLIC_KEY_PEM;

fn test_config() -> TraceUploadClaimIssuerConfig {
    TraceUploadClaimIssuerConfig {
        bind: "127.0.0.1:0".parse().expect("bind parses"),
        signing_private_key_pem: TEST_EDDSA_PRIVATE_KEY_PEM.to_string(),
        signing_public_key_pem: TEST_EDDSA_PUBLIC_KEY_PEM.to_string(),
        signing_kid: "issuer-key-1".to_string(),
        issuer: "trace-commons-upload-issuer".to_string(),
        audience: "trace-commons-upload".to_string(),
        max_ttl_seconds: 300,
        workload_public_key_pem: WORKLOAD_EDDSA_PUBLIC_KEY_PEM.to_string(),
        workload_issuer: Some("workload-issuer".to_string()),
        workload_audience: Some("trace-claim-issuer".to_string()),
        tenant_access_grant_db: None,
        require_tenant_access_grants: false,
        shutdown_grace_seconds: DEFAULT_SHUTDOWN_GRACE_SECONDS,
        request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        allowlist_source: None,
        allowlist_refresh_interval_seconds: DEFAULT_ALLOWLIST_REFRESH_INTERVAL_SECONDS,
        allowlist_max_stale_seconds: DEFAULT_ALLOWLIST_MAX_STALE_SECONDS,
        onboarding_device_key_db: None,
        onboarding_ingest_url: Some("https://ingest.tracecommons.ai".to_string()),
        onboarding_community_url: Some("https://tracecommons.ai".to_string()),
        onboarding_profile_url: Some("https://tracecommons.ai/profile".to_string()),
        onboarding_leaderboard_url: Some("https://tracecommons.ai/leaderboard".to_string()),
        admin_bind: None,
        invite_admin_backend: None,
        invite_admin_registry: None,
        invite_registry_authoritative: false,
        near_ai_signup_policy_template: None,
    }
}

/// A sign-up template without the grant database would never be read --
/// the ceiling only comes from a grant when the issuer can read grants --
/// so NEAR AI devices would stay on the floor while the configuration
/// claims otherwise. The issuer refuses to start in that state.
#[test]
fn a_sign_up_template_without_the_grant_database_refuses_to_start() {
    let template = crate::near_ai_signup_policy::NearAiSignupPolicyTemplate::parse(
            r#"{"policy_version":"v","allowed_consent_scopes":["model_training"],"allowed_uses":["model_training"]}"#,
        )
        .unwrap();
    let mut config = test_config();
    config.near_ai_signup_policy_template = Some(template);
    let error = config
        .build_state()
        .err()
        .expect("template without grant database must refuse")
        .to_string();
    assert!(
        error.contains(crate::near_ai_signup_policy::NEAR_AI_SIGNUP_POLICY_TEMPLATE_ENV)
            && error.contains(TRACE_UPLOAD_CLAIM_ISSUER_TENANT_ACCESS_GRANT_DB_ENV),
        "{error}"
    );
    assert!(
        test_config().build_state().is_ok(),
        "no template: unchanged"
    );
}

fn workload_token(issuer: &str, audience: &str) -> String {
    let now = Utc::now();
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("workload-key-1".to_string());
    jsonwebtoken::encode(
        &header,
        &json!({
            "sub": "principal:agent-1",
            "principal_ref": "principal:agent-1",
            "tenant_id": "tenant-a",
            "iss": issuer,
            "aud": audience,
            "iat": now.timestamp(),
            "exp": (now + Duration::minutes(5)).timestamp(),
            "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
            "allowed_uses": ["debugging", "evaluation"],
        }),
        &EncodingKey::from_ed_pem(WORKLOAD_EDDSA_PRIVATE_KEY_PEM.as_bytes())
            .expect("workload key parses"),
    )
    .expect("workload token signs")
}

fn claim_request() -> serde_json::Value {
    json!({
        "schema_version": TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION,
        "tenant_id": "tenant-a",
        "audience": "trace-commons-upload",
        "trace_id": Uuid::new_v4(),
        "submission_id": Uuid::new_v4(),
        "consent_scopes": ["debugging_evaluation"],
        "allowed_uses": ["debugging"],
        "requested_at": Utc::now(),
    })
}

async fn post_claim(
    config: TraceUploadClaimIssuerConfig,
    token: String,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let router = trace_upload_claim_issuer_router(config).expect("router builds");
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/trace-upload-claim")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body reads");
    let json = serde_json::from_slice(&body).expect("json response");
    (status, json)
}

async fn post_device_claim(
    config: TraceUploadClaimIssuerConfig,
    device_key_id: &str,
    signature: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let router = trace_upload_claim_issuer_router(config).expect("router builds");
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/trace-upload-claim")
                .header(header::CONTENT_TYPE, "application/json")
                .header(TRACE_DEVICE_KEY_ID_HEADER, device_key_id)
                .header(TRACE_DEVICE_SIGNATURE_HEADER, signature)
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body reads");
    let json = serde_json::from_slice(&body).expect("json response");
    (status, json)
}

fn onboard_request(invite_code: &str) -> serde_json::Value {
    json!({
        "schema_version": TRACE_ONBOARD_REQUEST_SCHEMA_VERSION,
        "invite_code": invite_code,
        "device_public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "client_info": {
            "agent": "ironclaw",
            "version": "0.x.y"
        }
    })
}

async fn post_onboard(
    config: TraceUploadClaimIssuerConfig,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let router = trace_upload_claim_issuer_router(config).expect("router builds");
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/onboard")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body reads");
    let json = serde_json::from_slice(&body).expect("json response");
    (status, json)
}

async fn get_text(config: TraceUploadClaimIssuerConfig, uri: &str) -> (StatusCode, String) {
    let router = trace_upload_claim_issuer_router(config).expect("router builds");
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body reads");
    (
        status,
        String::from_utf8(body.to_vec()).expect("body is utf8"),
    )
}

#[tokio::test]
async fn invite_landing_route_explains_agent_onboarding() {
    let (status, body) = get_text(test_config(), "/onboard#INV9K3RT5FBQ72JX").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Trace Commons invite link"));
    assert!(body.contains("/v1/onboard"));
    assert!(body.contains("trace_commons.onboard_request.v1"));
    assert!(body.contains("invite_code"));
    assert!(body.contains("device_public_key"));

    // The page points at the supported CLI entry point rather than
    // leaving an agent to hand-roll the POST.
    assert!(body.contains("login --invite"));

    // Each of these is a step a real contributor got wrong by reading the
    // source instead of this page. Losing any of them regresses the
    // onboarding experience even though the route still returns 200.
    assert!(
        body.contains("NOT IDEMPOTENT"),
        "must warn that a repeat POST spends another invite use"
    );
    assert!(
        body.contains("sha256:"),
        "must distinguish the raw public key from the device_key_id digest"
    );
    assert!(
        body.contains("contributor.json"),
        "must say where the response has to be persisted"
    );
    assert!(
        body.contains("--dry-run"),
        "must steer a first submit through a dry run"
    );

    // The landing page never echoes the invite code: the fragment is not
    // sent to the server, and a code in a response body would be a
    // credential in a log.
    assert!(!body.contains("INV9K3RT5FBQ72JX"));
}

#[tokio::test]
async fn eddsa_only_issue_success_returns_bounded_upload_claim() {
    let (status, body) = post_claim(
        test_config(),
        workload_token("workload-issuer", "trace-claim-issuer"),
        claim_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert!(body["expires_in"].as_i64().expect("expires_in") <= 300);

    let token = body["access_token"].as_str().expect("access token");
    let header = jsonwebtoken::decode_header(token).expect("issuer token header");
    assert_eq!(header.alg, Algorithm::EdDSA);
    assert_eq!(header.kid.as_deref(), Some("issuer-key-1"));

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&["trace-commons-upload-issuer"]);
    validation.set_audience(&["trace-commons-upload"]);
    let claims = jsonwebtoken::decode::<serde_json::Value>(
        token,
        &DecodingKey::from_ed_pem(TEST_EDDSA_PUBLIC_KEY_PEM.as_bytes())
            .expect("issuer public key parses"),
        &validation,
    )
    .expect("issuer token verifies")
    .claims;
    assert_eq!(claims["tenant_id"], "tenant-a");
    assert_eq!(claims["role"], "contributor");
    assert_eq!(claims["sub"], "principal:agent-1");
    assert_eq!(claims["principal_ref"], "principal:agent-1");
    assert_eq!(
        claims["allowed_consent_scopes"],
        json!(["debugging_evaluation"])
    );
    assert_eq!(claims["allowed_uses"], json!(["debugging"]));
    assert!(claims["jti"].as_str().is_some_and(|jti| !jti.is_empty()));
}

#[tokio::test]
async fn claim_response_echoes_granted_scopes() {
    let (status, body) = post_claim(
        test_config(),
        workload_token("workload-issuer", "trace-claim-issuer"),
        claim_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let token = body["access_token"].as_str().expect("access token");
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&["trace-commons-upload-issuer"]);
    validation.set_audience(&["trace-commons-upload"]);
    let claims = jsonwebtoken::decode::<serde_json::Value>(
        token,
        &DecodingKey::from_ed_pem(TEST_EDDSA_PUBLIC_KEY_PEM.as_bytes())
            .expect("issuer public key parses"),
        &validation,
    )
    .expect("issuer token verifies")
    .claims;

    assert_eq!(body["consent_scopes"], claims["allowed_consent_scopes"]);
    assert_eq!(body["allowed_uses"], claims["allowed_uses"]);
    assert_eq!(body["consent_scopes"], json!(["debugging_evaluation"]));
    assert_eq!(body["allowed_uses"], json!(["debugging"]));
}

#[tokio::test]
async fn wrong_workload_audience_or_issuer_is_rejected() {
    let (status, _) = post_claim(
        test_config(),
        workload_token("wrong-issuer", "trace-claim-issuer"),
        claim_request(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = post_claim(
        test_config(),
        workload_token("workload-issuer", "wrong-audience"),
        claim_request(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn no_rsa_or_generic_jwks_material_is_accepted_or_exposed() {
    assert!(
        TraceUploadClaimIssuerConfig {
            signing_private_key_pem:
                "-----BEGIN RSA PRIVATE KEY-----\nredacted\n-----END RSA PRIVATE KEY-----"
                    .to_string(),
            ..test_config()
        }
        .build_state()
        .is_err()
    );

    let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/.well-known/trace-commons-ed25519-keyset.json")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body reads");
    let text = String::from_utf8(body.to_vec()).expect("utf8");
    assert!(text.contains("\"public_key_pem\""));
    assert!(text.contains("BEGIN PUBLIC KEY"));
    assert!(!text.contains("\"kty\""));
    assert!(!text.contains("\"crv\""));
    assert!(!text.contains("\"x\""));
    assert!(!text.contains("RSA"));
    assert!(!text.contains("BEGIN PRIVATE KEY"));
}

#[test]
fn tenant_access_grant_env_flag_uses_explicit_truthy_values() {
    for value in ["1", "true", "TRUE", "yes", "on", " on "] {
        assert!(parse_truthy_env_value(value), "{value} should be truthy");
    }
    for value in ["", "0", "false", "off", "no", "tenant-a"] {
        assert!(!parse_truthy_env_value(value), "{value} should be falsey");
    }
}

#[tokio::test]
async fn issues_claim_with_public_attribution_scope() {
    // public_attribution is a profile-management consent. A workload
    // token that allows it can mint an upload claim scoped to ONLY
    // public_attribution with no allowed_uses — the claim is used to
    // authenticate /v1/community/profile, not to submit traces.
    let state = test_config().build_state().expect("state builds");
    let workload = WorkloadClaims {
        sub: Some("principal:agent-1".to_string()),
        principal_ref: None,
        tenant_id: Some("tenant-a".to_string()),
        iss: None,
        aud: None,
        exp: Utc::now().timestamp() + 60,
        iat: Some(Utc::now().timestamp()),
        allowed_consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        invite_code: None,
    };
    let request = TraceUploadClaimRequest {
        schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
        tenant_id: Some("tenant-a".to_string()),
        audience: Some("trace-commons-upload".to_string()),
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        subject: None,
        requested_at: Utc::now(),
        consent_basis: None,
        policy_digest: None,
        send_mode: None,
    };
    let response = state
        .issue_claim(&workload, request)
        .await
        .expect("issue succeeds");
    let token_parts: Vec<&str> = response.access_token.split('.').collect();
    assert_eq!(token_parts.len(), 3, "JWT shape");
    let payload = base64_url_decode(token_parts[1]);
    assert!(
        payload.contains("public_attribution"),
        "minted claim carries the requested scope: {payload}"
    );
}

/// Ingest resolves a device's account from a device-shaped actor, so the
/// workload path, which verifies no device key, may not mint one -- in
/// either `sub` or `principal_ref`, bare or `instance:` form.
#[tokio::test]
async fn workload_path_refuses_a_device_shaped_actor() {
    let state = test_config().build_state().expect("state builds");
    let key = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let instance = crate::account_trust::device_claim_actor("tenant-a", key, Some("u"));
    for (sub, principal_ref) in [
        (Some(key), None),
        (Some(instance.as_str()), None),
        (Some("principal:agent-1"), Some(instance.as_str())),
        (None, Some(key)),
    ] {
        let workload = WorkloadClaims {
            sub: sub.map(str::to_string),
            principal_ref: principal_ref.map(str::to_string),
            tenant_id: Some("tenant-a".to_string()),
            iss: None,
            aud: None,
            exp: Utc::now().timestamp() + 60,
            iat: Some(Utc::now().timestamp()),
            allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: vec![TraceAllowedUse::Debugging],
            invite_code: None,
        };
        let request = TraceUploadClaimRequest {
            schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: Some("tenant-a".to_string()),
            audience: Some("trace-commons-upload".to_string()),
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: vec![TraceAllowedUse::Debugging],
            subject: None,
            requested_at: Utc::now(),
            consent_basis: None,
            policy_digest: None,
            send_mode: None,
        };
        let error = state
            .issue_claim(&workload, request)
            .await
            .expect_err("device-shaped workload actor refused");
        assert_eq!(
            error.status,
            StatusCode::FORBIDDEN,
            "{sub:?} {principal_ref:?}"
        );
        assert_eq!(error.message, "workload subject is reserved");
    }
}

#[tokio::test]
async fn rejects_public_attribution_request_when_workload_lacks_it() {
    let state = test_config().build_state().expect("state builds");
    let workload = WorkloadClaims {
        sub: Some("principal:agent-1".to_string()),
        principal_ref: None,
        tenant_id: Some("tenant-a".to_string()),
        iss: None,
        aud: None,
        exp: Utc::now().timestamp() + 60,
        iat: Some(Utc::now().timestamp()),
        allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: vec![TraceAllowedUse::Debugging],
        invite_code: None,
    };
    let request = TraceUploadClaimRequest {
        schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
        tenant_id: Some("tenant-a".to_string()),
        audience: Some("trace-commons-upload".to_string()),
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        subject: None,
        requested_at: Utc::now(),
        consent_basis: None,
        policy_digest: None,
        send_mode: None,
    };
    assert!(
        state.issue_claim(&workload, request).await.is_err(),
        "workload scope allowlist gates public_attribution requests",
    );
}

#[tokio::test]
async fn issues_claim_with_mixed_public_attribution_and_trace_scopes() {
    // Common pilot case: contributor has a workload token that
    // grants both submitting traces and managing their profile.
    let state = test_config().build_state().expect("state builds");
    let workload = WorkloadClaims {
        sub: Some("principal:agent-1".to_string()),
        principal_ref: None,
        tenant_id: Some("tenant-a".to_string()),
        iss: None,
        aud: None,
        exp: Utc::now().timestamp() + 60,
        iat: Some(Utc::now().timestamp()),
        allowed_consent_scopes: vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::PublicAttribution,
        ],
        allowed_uses: vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::AggregateAnalytics,
        ],
        invite_code: None,
    };
    let request = TraceUploadClaimRequest {
        schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
        tenant_id: Some("tenant-a".to_string()),
        audience: Some("trace-commons-upload".to_string()),
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::PublicAttribution,
        ],
        allowed_uses: vec![TraceAllowedUse::Debugging],
        subject: None,
        requested_at: Utc::now(),
        consent_basis: None,
        policy_digest: None,
        send_mode: None,
    };
    let response = state
        .issue_claim(&workload, request)
        .await
        .expect("issue succeeds");
    let token_parts: Vec<&str> = response.access_token.split('.').collect();
    let payload = base64_url_decode(token_parts[1]);
    assert!(payload.contains("debugging_evaluation"));
    assert!(payload.contains("public_attribution"));
}

fn base64_url_decode(input: &str) -> String {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .expect("base64");
    String::from_utf8(bytes).expect("utf8")
}

#[tokio::test]
async fn rejects_requests_exceeding_workload_allowances() {
    let state = test_config().build_state().expect("state builds");
    let workload = WorkloadClaims {
        sub: Some("principal:agent-1".to_string()),
        principal_ref: None,
        tenant_id: Some("tenant-a".to_string()),
        iss: None,
        aud: None,
        exp: Utc::now().timestamp() + 60,
        iat: Some(Utc::now().timestamp()),
        allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: vec![TraceAllowedUse::Debugging],
        invite_code: None,
    };
    let request = TraceUploadClaimRequest {
        schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
        tenant_id: Some("tenant-a".to_string()),
        audience: Some("trace-commons-upload".to_string()),
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::ModelTraining],
        allowed_uses: vec![TraceAllowedUse::ModelTraining],
        subject: None,
        requested_at: Utc::now(),
        consent_basis: None,
        policy_digest: None,
        send_mode: None,
    };
    assert!(state.issue_claim(&workload, request).await.is_err());
}

/// Builds a workload token whose only interesting property is its
/// `tenant_id`. Scopes are wide enough that the request below is refused
/// for the tenant binding and nothing else.
fn workload_with_tenant(tenant_id: Option<&str>) -> WorkloadClaims {
    WorkloadClaims {
        sub: Some("principal:agent-1".to_string()),
        principal_ref: None,
        tenant_id: tenant_id.map(str::to_string),
        iss: None,
        aud: None,
        exp: Utc::now().timestamp() + 60,
        iat: Some(Utc::now().timestamp()),
        allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: vec![TraceAllowedUse::Debugging],
        invite_code: None,
    }
}

fn upload_claim_request(tenant_id: Option<&str>) -> TraceUploadClaimRequest {
    TraceUploadClaimRequest {
        schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
        tenant_id: tenant_id.map(str::to_string),
        audience: Some("trace-commons-upload".to_string()),
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: vec![TraceAllowedUse::Debugging],
        subject: None,
        requested_at: Utc::now(),
        consent_basis: None,
        policy_digest: None,
        send_mode: None,
    }
}

/// A workload token that carries no `tenant_id` must not be able to mint
/// a claim for a tenant named only in the request body.
///
/// `tenant_id` is `Option<String>` on `WorkloadClaims`, token validation
/// never requires it, and the default config here has no allowlist
/// source — so nothing upstream supplies the binding. Before the tenant
/// became authoritative this request succeeded and minted a claim for
/// whatever tenant the caller asked for.
#[tokio::test]
async fn refuses_upload_claim_when_workload_token_carries_no_tenant() {
    let state = test_config().build_state().expect("state builds");
    let error = state
        .issue_claim(
            &workload_with_tenant(None),
            upload_claim_request(Some("tenant-not-ours")),
        )
        .await
        .expect_err("a token with no tenant authorises no tenant");
    assert_eq!(error.message, "WorkloadTenantMissing");
    assert_eq!(error.status, StatusCode::FORBIDDEN);
}

/// Same refusal for a present-but-blank tenant: whitespace is not a
/// tenant, and trimming it must not leave an empty binding that compares
/// equal to nothing.
#[tokio::test]
async fn refuses_upload_claim_when_workload_tenant_is_blank() {
    let state = test_config().build_state().expect("state builds");
    for blank in ["", "   ", "\t"] {
        let error = state
            .issue_claim(
                &workload_with_tenant(Some(blank)),
                upload_claim_request(Some("tenant-not-ours")),
            )
            .await
            .expect_err("blank workload tenant is refused");
        assert_eq!(
            error.message, "WorkloadTenantMissing",
            "blank tenant {blank:?} must fail closed"
        );
    }
}

/// The token's tenant is authoritative, so a request that omits the
/// field is still bound to the token's tenant rather than being refused.
#[tokio::test]
async fn workload_tenant_is_authoritative_when_request_omits_tenant() {
    let state = test_config().build_state().expect("state builds");
    let response = state
        .issue_claim(
            &workload_with_tenant(Some("tenant-a")),
            upload_claim_request(None),
        )
        .await
        .expect("issue succeeds on the token's own tenant");
    let payload = base64_url_decode(response.access_token.split('.').collect::<Vec<_>>()[1]);
    assert!(
        payload.contains("tenant-a"),
        "claim is minted for the token's tenant: {payload}"
    );
}

/// A request naming a different tenant than the token is still refused,
/// and now unconditionally rather than only when the token happened to
/// carry a tenant.
#[tokio::test]
async fn refuses_upload_claim_when_request_tenant_differs_from_workload() {
    let state = test_config().build_state().expect("state builds");
    let error = state
        .issue_claim(
            &workload_with_tenant(Some("tenant-a")),
            upload_claim_request(Some("tenant-b")),
        )
        .await
        .expect_err("cross-tenant request is refused");
    assert_eq!(error.status, StatusCode::FORBIDDEN);
}

#[test]
fn tenant_grant_principal_ref_matches_ingest_signed_actor_shape() {
    assert_eq!(
        signed_claim_auth_principal_ref("tenant-a", "actor-123"),
        method_bound_principal_ref("signed_claim", "signed:tenant-a:actor-123")
    );
    assert_eq!(
        legacy_signed_claim_principal_ref("tenant-a", "actor-123"),
        principal_storage_ref("signed:tenant-a:actor-123")
    );
    assert_ne!(
        signed_claim_auth_principal_ref("tenant-a", "actor-123"),
        legacy_signed_claim_principal_ref("tenant-a", "actor-123")
    );
}

#[test]
fn tenant_grant_authorization_requires_contributor_binding_and_intersects_allowlists() {
    let now = Utc::now();
    let mut consent_scopes = vec![
        ConsentScope::DebuggingEvaluation,
        ConsentScope::BenchmarkOnly,
    ];
    let mut allowed_uses = vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation];
    let grant = test_tenant_access_grant(
        now,
        TraceTenantAccessGrantRole::Contributor,
        vec!["debugging_evaluation"],
        vec!["debugging"],
        Some("trace-commons-upload-issuer"),
        Some("trace-commons-upload"),
        Some("actor-123"),
    );

    authorize_upload_claim_from_tenant_grants(
        &[grant],
        "trace-commons-upload-issuer",
        "trace-commons-upload",
        "actor-123",
        &mut consent_scopes,
        &mut allowed_uses,
    )
    .expect("grant authorizes claim");

    assert_eq!(consent_scopes, vec![ConsentScope::DebuggingEvaluation]);
    assert_eq!(allowed_uses, vec![TraceAllowedUse::Debugging]);

    let mut consent_scopes = Vec::new();
    let mut allowed_uses = Vec::new();
    let default_grant = test_tenant_access_grant(
        now,
        TraceTenantAccessGrantRole::Contributor,
        vec!["benchmark_only"],
        vec!["evaluation"],
        Some("trace-commons-upload-issuer"),
        Some("trace-commons-upload"),
        Some("actor-123"),
    );
    authorize_upload_claim_from_tenant_grants(
        &[default_grant],
        "trace-commons-upload-issuer",
        "trace-commons-upload",
        "actor-123",
        &mut consent_scopes,
        &mut allowed_uses,
    )
    .expect("empty request allowlists inherit the grant constraints");
    assert_eq!(consent_scopes, vec![ConsentScope::BenchmarkOnly]);
    assert_eq!(allowed_uses, vec![TraceAllowedUse::Evaluation]);

    let mut consent_scopes = vec![ConsentScope::DebuggingEvaluation];
    let mut allowed_uses = vec![TraceAllowedUse::Debugging];
    let reviewer_grant = test_tenant_access_grant(
        now,
        TraceTenantAccessGrantRole::Reviewer,
        vec![],
        vec![],
        None,
        None,
        None,
    );
    assert!(
        authorize_upload_claim_from_tenant_grants(
            &[reviewer_grant],
            "trace-commons-upload-issuer",
            "trace-commons-upload",
            "actor-123",
            &mut consent_scopes,
            &mut allowed_uses,
        )
        .is_err()
    );
}

#[test]
fn generate_keypair_produces_parseable_ed25519_material() {
    let keypair = generate_upload_claim_keypair().expect("keygen succeeds");
    assert!(
        keypair
            .private_key_pem
            .starts_with("-----BEGIN PRIVATE KEY-----")
    );
    assert!(
        keypair
            .private_key_pem
            .contains("-----END PRIVATE KEY-----")
    );
    assert!(
        keypair
            .public_key_pem
            .starts_with("-----BEGIN PUBLIC KEY-----")
    );
    assert!(keypair.public_key_pem.contains("-----END PUBLIC KEY-----"));
    assert!(Uuid::parse_str(&keypair.suggested_kid).is_ok());
    // Round-trip through the same code path the server uses at startup.
    EncodingKey::from_ed_pem(keypair.private_key_pem.as_bytes())
        .expect("generated private key parses as EdDSA");
    DecodingKey::from_ed_pem(keypair.public_key_pem.as_bytes())
        .expect("generated public key parses as EdDSA");
    // Sign and verify a round-trip to confirm public matches private.
    let signing_key = EncodingKey::from_ed_pem(keypair.private_key_pem.as_bytes()).unwrap();
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(keypair.suggested_kid.clone());
    let token = jsonwebtoken::encode(
        &header,
        &json!({"sub": "probe", "exp": Utc::now().timestamp() + 60}),
        &signing_key,
    )
    .expect("signs");
    let verifying_key = DecodingKey::from_ed_pem(keypair.public_key_pem.as_bytes()).unwrap();
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_aud = false;
    jsonwebtoken::decode::<serde_json::Value>(&token, &verifying_key, &validation)
        .expect("generated keypair round-trips through JWT sign+verify");
}

#[test]
fn health_check_reports_signing_and_workload_status() {
    let state = test_config().build_state().expect("state builds");
    let checks = state.run_health_checks();
    assert_eq!(checks["signing_key"], json!("ok"));
    assert_eq!(checks["workload_public_key"], json!("ok"));
}

#[tokio::test]
async fn health_endpoint_returns_503_when_workload_pem_is_malformed() {
    let mut state = (*test_config().build_state().expect("state builds")).clone_for_test();
    state.workload_public_key_pem =
        "-----BEGIN PUBLIC KEY-----\nnot-base64\n-----END PUBLIC KEY-----\n".to_string();
    let router = Router::new()
        .route("/health", get(health_handler))
        .with_state(Arc::new(state));
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["checks"]["workload_public_key"], "fail");
}

#[tokio::test]
async fn health_endpoint_returns_200_when_keys_are_loadable() {
    let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["checks"]["signing_key"], "ok");
    assert_eq!(json["checks"]["workload_public_key"], "ok");
}

#[tokio::test]
async fn health_endpoint_reports_build_identity_on_both_outcomes() {
    // The point of the identity is that it survives a bad deploy, so it has
    // to be on the degraded response as much as the healthy one.
    for (config_state, expected_status) in [
        (test_config().build_state().expect("state builds"), "ok"),
        (
            {
                let mut state =
                    (*test_config().build_state().expect("state builds")).clone_for_test();
                state.workload_public_key_pem =
                    "-----BEGIN PUBLIC KEY-----\nnot-base64\n-----END PUBLIC KEY-----\n"
                        .to_string();
                Arc::new(state)
            },
            "degraded",
        ),
    ] {
        let router = Router::new()
            .route("/health", get(health_handler))
            .with_state(config_state);
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], expected_status);
        assert_eq!(json["build_commit"], trace_commons_build_info::COMMIT);
        assert_eq!(json["build_time"], trace_commons_build_info::BUILD_TIME);
        assert_eq!(json["build_version"], env!("CARGO_PKG_VERSION"));
        // Additive only: the fields existing consumers read are untouched.
        assert!(json["checks"].is_object());
    }
}

#[tokio::test]
async fn graceful_shutdown_completes_within_grace_window() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
    let (signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
    let signal = async move {
        let _ = signal_rx.await;
    };
    let serve = tokio::spawn(serve_router_with_graceful_shutdown(
        listener, router, 2, signal,
    ));
    // Give the server a moment to start accepting.
    tokio::time::sleep(StdDuration::from_millis(50)).await;
    signal_tx.send(()).unwrap();
    let outcome = tokio::time::timeout(StdDuration::from_secs(5), serve)
        .await
        .expect("serve returns within timeout")
        .expect("task joins");
    outcome.expect("serve returns Ok after graceful shutdown");
}

#[test]
fn onboarding_ingest_url_origin_normalizes_to_submit_endpoint() {
    assert_eq!(
        normalize_onboarding_ingest_url(Some("https://ingest.tracecommons.ai".to_string()))
            .expect("origin normalizes")
            .as_deref(),
        Some("https://ingest.tracecommons.ai/v1/traces")
    );
    assert_eq!(
        normalize_onboarding_ingest_url(Some(
            "https://ingest.tracecommons.ai/v1/traces".to_string()
        ))
        .expect("endpoint is preserved")
        .as_deref(),
        Some("https://ingest.tracecommons.ai/v1/traces")
    );
    assert_eq!(
        normalize_onboarding_ingest_url(Some("   ".to_string())).expect("blank is absent"),
        None
    );
    assert!(
        normalize_onboarding_ingest_url(Some(
            "https://ingest.tracecommons.ai/v1/traces#secret".to_string()
        ))
        .is_err(),
        "onboarding ingest URL must not carry fragments"
    );
}

impl TraceUploadClaimIssuerState {
    fn clone_for_test(&self) -> TraceUploadClaimIssuerState {
        TraceUploadClaimIssuerState {
            signing_key: EncodingKey::from_ed_pem(TEST_EDDSA_PRIVATE_KEY_PEM.as_bytes()).unwrap(),
            signing_kid: self.signing_kid.clone(),
            issuer: self.issuer.clone(),
            audience: self.audience.clone(),
            max_ttl_seconds: self.max_ttl_seconds,
            workload_decoding_key: DecodingKey::from_ed_pem(
                WORKLOAD_EDDSA_PUBLIC_KEY_PEM.as_bytes(),
            )
            .unwrap(),
            workload_issuer: self.workload_issuer.clone(),
            workload_audience: self.workload_audience.clone(),
            signing_public_key_pem: self.signing_public_key_pem.clone(),
            workload_public_key_pem: self.workload_public_key_pem.clone(),
            tenant_access_grant_db: self.tenant_access_grant_db.clone(),
            require_tenant_access_grants: self.require_tenant_access_grants,
            allowlist_source: self.allowlist_source.clone(),
            allowlist_max_stale: self.allowlist_max_stale,
            onboarding_device_key_db: self.onboarding_device_key_db.clone(),
            onboarding_ingest_url: self.onboarding_ingest_url.clone(),
            onboarding_community_url: self.onboarding_community_url.clone(),
            onboarding_profile_url: self.onboarding_profile_url.clone(),
            onboarding_leaderboard_url: self.onboarding_leaderboard_url.clone(),
            denial_counter: Arc::clone(&self.denial_counter),
            instance_replay_cache: Arc::clone(&self.instance_replay_cache),
            instance_rate_limiter: Arc::clone(&self.instance_rate_limiter),
            instance_enroll_default_rate_per_min: self.instance_enroll_default_rate_per_min,
            invite_lookup_rate_per_min: self.invite_lookup_rate_per_min,
            invite_lookup_client_ip_header: self.invite_lookup_client_ip_header.clone(),
            invite_lookup_per_client_rate_per_min: self.invite_lookup_per_client_rate_per_min,
            invite_lookup_client_limiter: Arc::clone(&self.invite_lookup_client_limiter),
            invite_admin_backend: self.invite_admin_backend.clone(),
            invite_admin_registry: self.invite_admin_registry.clone(),
            invite_registry_authoritative: self.invite_registry_authoritative,
            near_ai_signup_policy_template: self.near_ai_signup_policy_template.clone(),
            // The self-serve claim surfaces have their own hermetic tests in
            // `near_legion_claim` and `celestine_sloth_claim`; these issuer
            // tests build state without either.
            near_legion_claim: None,
            celestine_sloth_claim: None,
        }
    }
}

fn test_tenant_access_grant(
    now: DateTime<Utc>,
    role: TraceTenantAccessGrantRole,
    allowed_consent_scopes: Vec<&str>,
    allowed_uses: Vec<&str>,
    issuer: Option<&str>,
    audience: Option<&str>,
    subject: Option<&str>,
) -> TraceTenantAccessGrantRecord {
    TraceTenantAccessGrantRecord {
        tenant_id: "tenant-a".to_string(),
        grant_id: Uuid::new_v4(),
        principal_ref: principal_storage_ref("signed:tenant-a:actor-123"),
        role,
        status: TraceTenantAccessGrantStatus::Active,
        allowed_consent_scopes: allowed_consent_scopes
            .into_iter()
            .map(str::to_string)
            .collect(),
        allowed_uses: allowed_uses.into_iter().map(str::to_string).collect(),
        issuer: issuer.map(str::to_string),
        audience: audience.map(str::to_string),
        subject: subject.map(str::to_string),
        issued_at: now,
        expires_at: None,
        revoked_at: None,
        created_by_principal_ref: None,
        revoked_by_principal_ref: None,
        reason: None,
        metadata: BTreeMap::new(),
        created_at: now,
        updated_at: now,
    }
}

// ----- Pilot allowlist integration cases -------------------------------

use crate::trace_upload_claim_allowlist::{AllowlistSourceSpec, hash_invite_code};

fn workload_token_with_invite(issuer: &str, audience: &str, invite_code: &str) -> String {
    let now = Utc::now();
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("workload-key-1".to_string());
    jsonwebtoken::encode(
        &header,
        &json!({
            "sub": "principal:agent-1",
            "principal_ref": "principal:agent-1",
            "tenant_id": "tenant-a",
            "iss": issuer,
            "aud": audience,
            "iat": now.timestamp(),
            "exp": (now + Duration::minutes(5)).timestamp(),
            "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
            "allowed_uses": ["debugging", "evaluation"],
            "invite_code": invite_code,
        }),
        &EncodingKey::from_ed_pem(WORKLOAD_EDDSA_PRIVATE_KEY_PEM.as_bytes())
            .expect("workload key parses"),
    )
    .expect("workload token signs")
}

fn write_allowlist_file(path: &std::path::Path, policy_label: &str, codes: &[&str]) {
    use std::io::Write;
    let entries: Vec<String> = codes
        .iter()
        .map(|c| {
            format!(
                "{{\"subject_hash\":\"{}\",\"tenant_id\":\"tenant-a\"}}",
                hash_invite_code(c)
            )
        })
        .collect();
    let body = format!(
        "{{\"version\":1,\"generated_at\":\"2026-05-17T00:00:00Z\",\"policy_label\":\"{policy_label}\",\"entries\":[{}]}}",
        entries.join(",")
    );
    let mut f = std::fs::File::create(path).expect("create allowlist file");
    f.write_all(body.as_bytes()).expect("write allowlist file");
}

fn config_with_file_allowlist(path: std::path::PathBuf) -> TraceUploadClaimIssuerConfig {
    TraceUploadClaimIssuerConfig {
        allowlist_source: Some(AllowlistSourceSpec::File(path)),
        ..test_config()
    }
}

#[tokio::test]
async fn allowlist_admits_listed_invite_and_embeds_policy_label() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
    let config = config_with_file_allowlist(path);
    let token = workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-OK-001");
    let (status, body) = post_claim(config, token, claim_request()).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .expect("access_token present");

    // Decode the minted JWT body and confirm policy_label is embedded.
    let parts: Vec<&str> = access_token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT has three parts");
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("payload decodes");
    let parsed: serde_json::Value = serde_json::from_slice(&payload).expect("payload is json");
    assert_eq!(
        parsed.get("policy_label").and_then(|v| v.as_str()),
        Some("pilot-2026-05"),
        "minted JWT carries policy_label"
    );
}

#[tokio::test]
async fn onboard_requires_registry_db_after_allowlist_match() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INVOK001INVOK001"]);
    let config = config_with_file_allowlist(path);
    let (status, body) = post_onboard(config, onboard_request("INVOK001INVOK001")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("OnboardRegistryNotConfigured")
    );
}

#[tokio::test]
async fn onboard_requires_allowlist_source() {
    let (status, body) = post_onboard(test_config(), onboard_request("INVOK001INVOK001")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("OnboardAllowlistNotConfigured")
    );
}

#[tokio::test]
async fn onboard_refuses_malformed_device_public_key() {
    let mut body = onboard_request("INVOK001INVOK001");
    body["device_public_key"] = json!("not-base64");
    let (status, body) = post_onboard(test_config(), body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("DeviceKeyMalformed")
    );
}

#[tokio::test]
async fn onboard_refuses_unlisted_invite_with_named_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INVOK001INVOK001"]);
    let config = config_with_file_allowlist(path);
    let (status, body) = post_onboard(config, onboard_request("MISS0001MISS0001")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("InviteNotValid")
    );
}

#[tokio::test]
async fn onboard_fails_closed_when_authoritative_with_no_registry() {
    // Authoritative mode with no registry must refuse, NOT fall back to
    // the file allowlist. Silent fallback would let a revoked invite
    // redeem again after a config mistake.
    let config = TraceUploadClaimIssuerConfig {
        invite_registry_authoritative: true,
        near_ai_signup_policy_template: None,
        ..test_config()
    };
    let (status, body) = post_onboard(config, onboard_request("INVOK001INVOK001")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("InviteRegistryNotConfigured")
    );
}

#[tokio::test]
async fn device_key_claim_requires_well_formed_device_key_id() {
    let (status, body) =
        post_device_claim(test_config(), "not-a-key-id", "AA==", claim_request()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("invalid device key id")
    );
}

#[tokio::test]
async fn device_key_claim_requires_base64_signature() {
    let (status, body) = post_device_claim(
        test_config(),
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "not-base64",
        claim_request(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("invalid device key signature")
    );
}

#[tokio::test]
async fn device_key_claim_uses_registry_gate_instead_of_workload_token() {
    let (status, body) = post_device_claim(
        test_config(),
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        claim_request(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("device key registry is not configured")
    );
}

#[test]
fn device_key_signature_verifies_exact_claim_body() {
    use ring::signature::KeyPair;

    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
    let keypair =
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
    let public_key = keypair.public_key().as_ref();
    let device_key_id = device_key_id_from_public_key_bytes(public_key);
    let public_key_wire = base64::engine::general_purpose::STANDARD.encode(public_key);
    let stored_public_key =
        device_public_key_bytes(&public_key_wire, &device_key_id).expect("stored key parses");

    let body = claim_request().to_string();
    let signature = keypair.sign(body.as_bytes());
    verify_device_claim_signature(&stored_public_key, body.as_bytes(), signature.as_ref())
        .expect("signature verifies over exact body");
    assert!(
        verify_device_claim_signature(&stored_public_key, b"{}", signature.as_ref()).is_err(),
        "signature must bind the exact serialized body"
    );
}

#[test]
fn device_key_workload_jwt_verifies_registered_public_key() {
    use ring::signature::KeyPair;

    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
    let keypair =
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
    let public_key = keypair.public_key().as_ref();
    let device_key_id = device_key_id_from_public_key_bytes(public_key);
    let now = Utc::now();
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(device_key_id);
    let token = jsonwebtoken::encode(
        &header,
        &json!({
            "tenant_id": "tenant-a",
            "aud": "trace-commons-ingest",
            "iat": now.timestamp(),
            "exp": (now + Duration::minutes(5)).timestamp(),
        }),
        &EncodingKey::from_ed_der(pkcs8.as_ref()),
    )
    .expect("device token signs");

    let claims = verify_device_workload_jwt(&token, public_key, Some("trace-commons-ingest"))
        .expect("registered public key verifies device token");
    validate_device_workload_claims(&claims, "tenant-a").expect("tenant matches");
    assert_eq!(claims.tenant_id, "tenant-a");
    assert!(
        validate_device_workload_claims(&claims, "tenant-b").is_err(),
        "device JWT must not authorize another tenant"
    );
    assert!(
        verify_device_workload_jwt(&token, public_key, Some("wrong-audience")).is_err(),
        "device JWT audience must be enforced"
    );
}

#[tokio::test]
async fn allowlist_refuses_unlisted_invite_with_named_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
    let config = config_with_file_allowlist(path);
    let token =
        workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-NOT-LISTED");
    let (status, body) = post_claim(config, token, claim_request()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("PilotAllowlistNotMatched")
    );
}

#[tokio::test]
async fn allowlist_refuses_missing_invite_with_named_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
    let config = config_with_file_allowlist(path);
    // Use the base workload_token helper (no invite_code field).
    let token = workload_token("workload-issuer", "trace-claim-issuer");
    let (status, body) = post_claim(config, token, claim_request()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("PilotAllowlistInviteCodeMissing")
    );
}

#[tokio::test]
async fn allowlist_off_by_default_preserves_legacy_behavior() {
    // No invite_code field; no allowlist configured. Issuance must
    // succeed exactly as before this slice.
    let config = test_config();
    let token = workload_token("workload-issuer", "trace-claim-issuer");
    let (status, _) = post_claim(config, token, claim_request()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn allowlist_stale_snapshot_refuses_with_named_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INV-OK-001"]);
    let mut config = config_with_file_allowlist(path);
    // Force the snapshot to be "stale" by setting max-stale to 0 and
    // sleeping enough that the cached snapshot's age > 0.
    config.allowlist_max_stale_seconds = 0;
    let token = workload_token_with_invite("workload-issuer", "trace-claim-issuer", "INV-OK-001");
    // Wait a brief moment so loaded_at.elapsed() > Duration::from_secs(0).
    // Duration::from_secs(0) means "any elapsed time is stale", but our
    // comparison is `snapshot_age > max_stale`, so the snapshot has to
    // be older than zero — any non-zero elapsed wins.
    tokio::time::sleep(StdDuration::from_millis(5)).await;
    let (status, body) = post_claim(config, token, claim_request()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("PilotAllowlistStale")
    );
}

fn enrollment_keypair() -> ring::signature::Ed25519KeyPair {
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .expect("generate enrollment keypair");
    ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse enrollment keypair")
}

struct EnrollmentFixture {
    _allowlist_dir: tempfile::TempDir,
    state: Arc<TraceUploadClaimIssuerState>,
    instance_key: ring::signature::Ed25519KeyPair,
    instance_public: Vec<u8>,
    device_public: Vec<u8>,
}

impl EnrollmentFixture {
    fn new() -> Self {
        use ring::signature::KeyPair;
        let instance_key = enrollment_keypair();
        let instance_public = instance_key.public_key().as_ref().to_vec();
        let device_public = enrollment_keypair().public_key().as_ref().to_vec();
        let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_public);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        std::fs::write(&path, format!(
                r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"ironclaw-test","instance_public_key":"{instance_pk_b64}","max_enrollments":100,"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
            )).expect("write allowlist");
        let state = TraceUploadClaimIssuerConfig {
            allowlist_source: Some(AllowlistSourceSpec::File(path)),
            onboarding_device_key_db: None,
            ..test_config()
        }
        .build_state()
        .expect("state builds");
        Self {
            _allowlist_dir: dir,
            state,
            instance_key,
            instance_public,
            device_public,
        }
    }

    fn request(
        &self,
        user: &str,
    ) -> trace_commons_protocol::onboarding::TraceInstanceEnrollRequest {
        make_enroll_request(
            &self.instance_key,
            &self.instance_public,
            &self.device_public,
            "trace-commons-upload",
            "ironclaw-test",
            user,
            &Uuid::new_v4().to_string(),
        )
    }

    fn sign(&self, request: &mut trace_commons_protocol::onboarding::TraceInstanceEnrollRequest) {
        let bytes = trace_commons_protocol::onboarding::instance_enroll_attestation_signing_bytes(
            &request.attestation,
        );
        request.attestation_sig = base64::engine::general_purpose::STANDARD
            .encode(self.instance_key.sign(&bytes).as_ref());
    }
}

#[tokio::test]
async fn enroll_rejects_bad_signature_uniformly() {
    let fixture = EnrollmentFixture::new();
    let mut request = fixture.request("user-enroll-bad-sig-test");
    request.attestation_sig = base64::engine::general_purpose::STANDARD.encode([0u8; 64]);
    let err = fixture
        .state
        .enroll(request)
        .await
        .expect_err("bad sig must be rejected");
    assert_eq!(
        err.status,
        StatusCode::FORBIDDEN,
        "must map to 403 FORBIDDEN"
    );
}

#[tokio::test]
async fn enroll_happy_path_provisions_user_tenant() {
    use crate::config::SslMode;
    use crate::trace_upload_claim_allowlist::hash_instance_subject;
    use ring::signature::KeyPair;
    use secrecy::SecretString;
    use trace_commons_protocol::onboarding::{
        TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
        TraceInstanceEnrollRequest, TraceOnboardClientInfo, derive_user_tenant_id,
        device_key_id_from_public_key_bytes, instance_enroll_attestation_signing_bytes,
    };

    // Skip if no DB available.
    let pg_url = match std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
    {
        Ok(u) => u,
        Err(_) => {
            eprintln!("skipping enroll_happy_path_provisions_user_tenant: no DB configured");
            return;
        }
    };
    let db_config = crate::config::DatabaseConfig {
        url: SecretString::from(pg_url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let pg = match crate::db::postgres::PgBackend::new(&db_config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping: database unavailable ({e})");
            return;
        }
    };
    let pool = pg.raw_pool_for_tests_and_diagnostics();
    let db: std::sync::Arc<dyn crate::db::Database> = std::sync::Arc::new(pg);

    // Generate instance keypair.
    let rng = ring::rand::SystemRandom::new();
    let instance_pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
    let instance_kp =
        ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
    let instance_pk = instance_kp.public_key().as_ref().to_vec();
    let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);

    // Generate device keypair.
    let device_pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
    let device_kp =
        ring::signature::Ed25519KeyPair::from_pkcs8(device_pkcs8.as_ref()).expect("parse");
    let device_pk = device_kp.public_key().as_ref().to_vec();
    let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&device_pk);
    let device_key_id = device_key_id_from_public_key_bytes(&device_pk);

    let instance_id = format!("test-instance-enroll-happy-{}", uuid::Uuid::new_v4());
    let user_subject = format!("test-user-enroll-happy-{}", uuid::Uuid::new_v4());

    // Build allowlist with the generated instance key.
    let instance_subject_hash = hash_instance_subject(&instance_pk);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    {
        use std::io::Write;
        let body = format!(
            r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"{instance_id}","instance_public_key":"{instance_pk_b64}","max_enrollments":100,"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
        );
        let mut f = std::fs::File::create(&path).expect("create allowlist");
        f.write_all(body.as_bytes()).expect("write allowlist");
    }

    let config = TraceUploadClaimIssuerConfig {
        allowlist_source: Some(AllowlistSourceSpec::File(path)),
        onboarding_device_key_db: Some(db.clone()),
        ..test_config()
    };

    let state = config.build_state().expect("state builds");

    let now_ts = chrono::Utc::now().timestamp();
    let attestation = TraceInstanceEnrollAttestation {
        device_key_id: device_key_id.clone(),
        aud: "trace-commons-upload".to_string(),
        instance_id: instance_id.clone(),
        user_subject: user_subject.clone(),
        nonce: uuid::Uuid::new_v4().to_string(),
        exp: now_ts + 240,
    };
    let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
    let sig = instance_kp.sign(&signing_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());

    let request = TraceInstanceEnrollRequest {
        schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
        instance_public_key: instance_pk_b64.clone(),
        device_public_key: device_pk_b64.clone(),
        attestation,
        attestation_sig: sig_b64,
        client_info: TraceOnboardClientInfo {
            agent: "ironclaw".to_string(),
            version: "0.x.y".to_string(),
        },
    };

    let resp = state.enroll(request).await.expect("enroll succeeds");
    let expected_tenant_id = derive_user_tenant_id(&instance_id, &user_subject);
    let expected_device_key_id = device_key_id_from_public_key_bytes(&device_pk);
    assert_eq!(
        resp.tenant_id, expected_tenant_id,
        "tenant_id matches derived value"
    );
    assert_eq!(
        resp.device_key_id, expected_device_key_id,
        "device_key_id matches"
    );

    // Cleanup: remove test rows so reruns don't accumulate.
    let client = pool.get().await.expect("pool get for cleanup");
    client
        .execute(
            "DELETE FROM trace_instance_enrollments WHERE instance_subject_hash = $1",
            &[&instance_subject_hash],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&expected_tenant_id],
        )
        .await
        .ok();
}

#[tokio::test]
async fn enroll_rejects_future_exp_uniformly() {
    let fixture = EnrollmentFixture::new();
    let mut request = fixture.request("user-future-exp-test");
    // A valid signature with expiry beyond the 5-minute upper bound.
    request.attestation.exp = Utc::now().timestamp() + 3600;
    fixture.sign(&mut request);
    let err = fixture
        .state
        .enroll(request)
        .await
        .expect_err("far-future exp must be rejected");
    assert_eq!(
        err.status,
        StatusCode::FORBIDDEN,
        "future exp must map to uniform 403 FORBIDDEN, not 400"
    );
}

#[tokio::test]
async fn enroll_device_key_mismatch_is_uniform_403() {
    use ring::signature::KeyPair;
    use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;
    let fixture = EnrollmentFixture::new();
    let mut request = fixture.request("user-device-mismatch-test");
    // Sign a different key's ID while the request still carries the real device key.
    request.attestation.device_key_id =
        device_key_id_from_public_key_bytes(enrollment_keypair().public_key().as_ref());
    fixture.sign(&mut request);
    let err = fixture
        .state
        .enroll(request)
        .await
        .expect_err("device_key_id mismatch must be rejected");
    assert_eq!(
        err.status,
        StatusCode::FORBIDDEN,
        "device_key_id mismatch must return uniform 403 FORBIDDEN, not 400 EnrollMalformed"
    );
}

// ── Task-8 shared helpers ────────────────────────────────────────────────

/// Build a DB-backed issuer state for a fresh instance entry.
///
/// Returns `(state, instance_kp, instance_pk_bytes, instance_subject_hash, pool)`
/// on success, or `None` when the DB is not available (caller must skip).
///
/// `tag` is embedded in the `instance_id` so each test uses a unique entry.
/// `max_enrollments` caps how many distinct users may enroll against this entry.
async fn build_pg_state_for_enroll_test(
    tag: &str,
    max_enrollments: u32,
) -> Option<(
    std::sync::Arc<TraceUploadClaimIssuerState>,
    ring::signature::Ed25519KeyPair,
    Vec<u8>,
    String,
    deadpool_postgres::Pool,
)> {
    use crate::config::SslMode;
    use crate::trace_upload_claim_allowlist::hash_instance_subject;
    use ring::signature::KeyPair;
    use secrecy::SecretString;

    let pg_url = match std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
    {
        Ok(u) => u,
        Err(_) => return None,
    };
    let db_config = crate::config::DatabaseConfig {
        url: SecretString::from(pg_url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let pg = match crate::db::postgres::PgBackend::new(&db_config).await {
        Ok(b) => b,
        Err(_) => return None,
    };
    let pool = pg.raw_pool_for_tests_and_diagnostics();
    let db: std::sync::Arc<dyn crate::db::Database> = std::sync::Arc::new(pg);

    let rng = ring::rand::SystemRandom::new();
    let instance_pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("instance keypair");
    let instance_kp =
        ring::signature::Ed25519KeyPair::from_pkcs8(instance_pkcs8.as_ref()).expect("parse");
    let instance_pk = instance_kp.public_key().as_ref().to_vec();
    let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(&instance_pk);
    let instance_id = format!("test-enroll-{tag}-{}", uuid::Uuid::new_v4());
    let instance_subject_hash = hash_instance_subject(&instance_pk);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    {
        use std::io::Write;
        let body = format!(
            r#"{{"version":1,"generated_at":"2026-01-01T00:00:00Z","policy_label":"test","entries":[{{"kind":"instance","instance_id":"{instance_id}","instance_public_key":"{instance_pk_b64}","max_enrollments":{max_enrollments},"policy_template":{{"policy_version":"v1","allowed_consent_scopes":["debugging_evaluation"],"allowed_uses":["debugging"]}}}}]}}"#
        );
        let mut f = std::fs::File::create(&path).expect("create allowlist");
        f.write_all(body.as_bytes()).expect("write allowlist");
    }
    // Keep `dir` alive by leaking it — the state needs the file to exist
    // for the duration of the test.  `tempdir` cleans up on drop; Box::leak
    // prevents that so the path remains valid.
    std::mem::forget(dir);

    let config = TraceUploadClaimIssuerConfig {
        allowlist_source: Some(AllowlistSourceSpec::File(path)),
        onboarding_device_key_db: Some(db),
        ..test_config()
    };
    let state = config.build_state().expect("state builds");
    Some((state, instance_kp, instance_pk, instance_subject_hash, pool))
}

/// Build a `TraceInstanceEnrollRequest` for the given parameters.
///
/// `instance_kp` is the instance signing keypair whose public key is in
/// the allowlist.  `device_pk_bytes` must be exactly 32 bytes (Ed25519).
/// `nonce` is used verbatim so callers can control replay.
fn make_enroll_request(
    instance_kp: &ring::signature::Ed25519KeyPair,
    instance_pk: &[u8],
    device_pk_bytes: &[u8],
    audience: &str,
    instance_id: &str,
    user_subject: &str,
    nonce: &str,
) -> trace_commons_protocol::onboarding::TraceInstanceEnrollRequest {
    use trace_commons_protocol::onboarding::{
        TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
        TraceInstanceEnrollRequest, TraceOnboardClientInfo, device_key_id_from_public_key_bytes,
        instance_enroll_attestation_signing_bytes,
    };

    let device_pk_b64 = base64::engine::general_purpose::STANDARD.encode(device_pk_bytes);
    let device_key_id = device_key_id_from_public_key_bytes(device_pk_bytes);
    let instance_pk_b64 = base64::engine::general_purpose::STANDARD.encode(instance_pk);
    let now_ts = chrono::Utc::now().timestamp();
    let attestation = TraceInstanceEnrollAttestation {
        device_key_id,
        aud: audience.to_string(),
        instance_id: instance_id.to_string(),
        user_subject: user_subject.to_string(),
        nonce: nonce.to_string(),
        exp: now_ts + 240,
    };
    let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
    let sig = instance_kp.sign(&signing_bytes);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());
    TraceInstanceEnrollRequest {
        schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
        instance_public_key: instance_pk_b64,
        device_public_key: device_pk_b64,
        attestation,
        attestation_sig: sig_b64,
        client_info: TraceOnboardClientInfo {
            agent: "ironclaw".to_string(),
            version: "0.x.y".to_string(),
        },
    }
}

// ── Task-8 tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn enroll_second_device_same_user_reuses_tenant_and_does_not_consume_cap() {
    use trace_commons_protocol::onboarding::derive_user_tenant_id;

    let Some((state, instance_kp, instance_pk, instance_subject_hash, pool)) =
        build_pg_state_for_enroll_test(
            "multi-device",
            1, // cap = 1 user slot
        )
        .await
    else {
        eprintln!(
            "skipping enroll_second_device_same_user_reuses_tenant_and_does_not_consume_cap: \
                 no DB configured"
        );
        return;
    };

    // Extract the instance_id from state's allowlist snapshot so we can
    // use it when building requests.
    let snapshot = state.onboard_allowlist_snapshot().expect("snapshot");
    use crate::trace_upload_claim_allowlist::hash_instance_subject;
    let hash = hash_instance_subject(&instance_pk);
    let entry = snapshot.instance_entry(&hash).expect("entry in snapshot");
    let instance_id = entry.instance_id.clone();
    let audience = state.audience.clone();

    let user1 = format!("multi-device-user-1-{}", uuid::Uuid::new_v4());
    let user2 = format!("multi-device-user-2-{}", uuid::Uuid::new_v4());

    // Device A bytes (32 bytes).
    let rng = ring::rand::SystemRandom::new();
    let dev_a_pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("dev A keypair");
    let dev_a =
        ring::signature::Ed25519KeyPair::from_pkcs8(dev_a_pkcs8.as_ref()).expect("parse dev A");
    use ring::signature::KeyPair;
    let dev_a_pk = dev_a.public_key().as_ref().to_vec();

    let dev_b_pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("dev B keypair");
    let dev_b =
        ring::signature::Ed25519KeyPair::from_pkcs8(dev_b_pkcs8.as_ref()).expect("parse dev B");
    let dev_b_pk = dev_b.public_key().as_ref().to_vec();

    let dev_c_pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("dev C keypair");
    let dev_c =
        ring::signature::Ed25519KeyPair::from_pkcs8(dev_c_pkcs8.as_ref()).expect("parse dev C");
    let dev_c_pk = dev_c.public_key().as_ref().to_vec();

    // Enroll user-1 with device A.
    let req_a = make_enroll_request(
        &instance_kp,
        &instance_pk,
        &dev_a_pk,
        &audience,
        &instance_id,
        &user1,
        &uuid::Uuid::new_v4().to_string(),
    );
    let resp_a = state.enroll(req_a).await.expect("user-1 device-A enrolls");

    // Enroll user-1 with device B (same user, different device).
    let req_b = make_enroll_request(
        &instance_kp,
        &instance_pk,
        &dev_b_pk,
        &audience,
        &instance_id,
        &user1,
        &uuid::Uuid::new_v4().to_string(),
    );
    let resp_b = state.enroll(req_b).await.expect("user-1 device-B enrolls");

    assert_eq!(
        resp_a.tenant_id, resp_b.tenant_id,
        "same user-subject must yield same tenant_id regardless of device"
    );

    // Enroll user-2 with device C — must be refused: cap is 1 user and
    // user-1 already consumed it.
    let req_c = make_enroll_request(
        &instance_kp,
        &instance_pk,
        &dev_c_pk,
        &audience,
        &instance_id,
        &user2,
        &uuid::Uuid::new_v4().to_string(),
    );
    let err_c = state
        .enroll(req_c)
        .await
        .expect_err("user-2 must be refused when cap = 1 is consumed");
    assert_eq!(
        err_c.status,
        StatusCode::FORBIDDEN,
        "cap exceeded must return 403 FORBIDDEN"
    );

    // Cleanup.
    let tenant1 = derive_user_tenant_id(&instance_id, &user1);
    let tenant2 = derive_user_tenant_id(&instance_id, &user2);
    let client = pool.get().await.expect("pool get for cleanup");
    client
        .execute(
            "DELETE FROM trace_instance_enrollments WHERE instance_subject_hash = $1",
            &[&instance_subject_hash],
        )
        .await
        .ok();
    for tid in [&tenant1, &tenant2] {
        client
            .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[tid])
            .await
            .ok();
    }
}

#[tokio::test]
async fn enroll_replayed_nonce_is_refused() {
    let Some((state, instance_kp, instance_pk, instance_subject_hash, pool)) =
        build_pg_state_for_enroll_test("replay", 100).await
    else {
        eprintln!("skipping enroll_replayed_nonce_is_refused: no DB configured");
        return;
    };

    let snapshot = state.onboard_allowlist_snapshot().expect("snapshot");
    use crate::trace_upload_claim_allowlist::hash_instance_subject;
    let hash = hash_instance_subject(&instance_pk);
    let entry = snapshot.instance_entry(&hash).expect("entry in snapshot");
    let instance_id = entry.instance_id.clone();
    let audience = state.audience.clone();

    let rng = ring::rand::SystemRandom::new();
    let dev_pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("device keypair");
    let dev_kp = ring::signature::Ed25519KeyPair::from_pkcs8(dev_pkcs8.as_ref()).expect("parse");
    use ring::signature::KeyPair;
    let dev_pk = dev_kp.public_key().as_ref().to_vec();

    let user = format!("replay-user-{}", uuid::Uuid::new_v4());
    let nonce = format!("dup-nonce-{}", uuid::Uuid::new_v4());

    let req1 = make_enroll_request(
        &instance_kp,
        &instance_pk,
        &dev_pk,
        &audience,
        &instance_id,
        &user,
        &nonce,
    );
    // First use must succeed and record the nonce.
    state.enroll(req1).await.expect("first enroll succeeds");

    // Second request with the SAME nonce — the replay cache recorded it
    // after the first success, so the pre-check must now reject it.
    let req2 = make_enroll_request(
        &instance_kp,
        &instance_pk,
        &dev_pk,
        &audience,
        &instance_id,
        &user,
        &nonce,
    );
    let err = state
        .enroll(req2)
        .await
        .expect_err("replayed nonce must be refused");
    assert_eq!(
        err.status,
        StatusCode::FORBIDDEN,
        "replay must return 403 FORBIDDEN"
    );

    // Cleanup.
    use trace_commons_protocol::onboarding::derive_user_tenant_id;
    let tenant_id = derive_user_tenant_id(&instance_id, &user);
    let client = pool.get().await.expect("pool get for cleanup");
    client
        .execute(
            "DELETE FROM trace_instance_enrollments WHERE instance_subject_hash = $1",
            &[&instance_subject_hash],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .ok();
}

#[tokio::test]
async fn onboard_invite_path_unaffected_by_enroll() {
    // Regression guard: the enroll path shares no mutable state that
    // breaks the existing invite onboard flow. Verify the invite-not-valid
    // error still surfaces correctly after the enroll wiring.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    write_allowlist_file(&path, "pilot-2026-05", &["INVOK001INVOK001"]);
    let config = config_with_file_allowlist(path);
    // An invite that is NOT in the allowlist must still return InviteNotValid.
    let (status, body) = post_onboard(config, onboard_request("MISS0001MISS0001")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "unlisted invite must still return 403 after enroll wiring"
    );
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("InviteNotValid"),
        "error code must be InviteNotValid"
    );
}

// ---- Task 2: per-user subject tests ----

use crate::db::{DeviceKeyRecord, InstanceEnrollmentOutcome, InstanceUserProvision};
use crate::error::DatabaseError;

struct StubDeviceKeyDb {
    refuse_upload_binding: bool,
    device_keys:
        std::sync::RwLock<std::collections::HashMap<(String, String), crate::db::DeviceKeyRecord>>,
}

impl StubDeviceKeyDb {
    fn new() -> Self {
        Self {
            refuse_upload_binding: false,
            device_keys: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    fn insert_test_device_key(
        &self,
        tenant_id: &str,
        device_key_id: &str,
        record: crate::db::DeviceKeyRecord,
    ) {
        self.device_keys
            .write()
            .unwrap()
            .insert((tenant_id.to_string(), device_key_id.to_string()), record);
    }
}

impl crate::trace_corpus_storage::TraceCorpusStore for StubDeviceKeyDb {}

#[async_trait::async_trait]
impl crate::db::Database for StubDeviceKeyDb {
    async fn bind_device_upload_principal(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
        _actor_ref: &str,
    ) -> Result<(), DatabaseError> {
        if self.refuse_upload_binding {
            Err(DatabaseError::Pool("upload ownership unavailable".into()))
        } else {
            Ok(())
        }
    }

    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        Ok(())
    }

    async fn enroll_instance_user(&self, _p: InstanceUserProvision) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool("stub".into()))
    }

    async fn reserve_instance_enrollment(
        &self,
        _instance_subject_hash: &str,
        _user_subject_hash: &str,
        _tenant_id: &str,
        _max_enrollments: i64,
    ) -> Result<InstanceEnrollmentOutcome, DatabaseError> {
        Err(DatabaseError::Pool("stub".into()))
    }

    async fn instance_ledger_rls_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }

    async fn get_device_key(
        &self,
        tenant_id: &str,
        device_key_id: &str,
    ) -> Result<Option<DeviceKeyRecord>, DatabaseError> {
        Ok(self
            .device_keys
            .read()
            .unwrap()
            .get(&(tenant_id.to_string(), device_key_id.to_string()))
            .cloned())
    }
}

fn device_claim_request_body(tenant_id: &str, subject: Option<&str>) -> serde_json::Value {
    let mut body = json!({
        "schema_version": TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION,
        "tenant_id": tenant_id,
        "audience": "trace-commons-upload",
        "trace_id": Uuid::new_v4(),
        "submission_id": Uuid::new_v4(),
        "consent_scopes": ["debugging_evaluation"],
        "allowed_uses": ["debugging"],
        "requested_at": Utc::now(),
    });
    if let Some(s) = subject {
        body["subject"] = serde_json::Value::String(s.to_string());
    }
    body
}

async fn post_signed_device_claim_for_tenant(
    tenant_id: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value, String) {
    post_signed_device_claim_with_binding(tenant_id, body, false, false).await
}

async fn post_signed_device_claim_with_binding(
    tenant_id: &str,
    body: serde_json::Value,
    refuse_binding: bool,
    device_jwt: bool,
) -> (StatusCode, serde_json::Value, String) {
    use ring::signature::KeyPair;
    use std::sync::Arc;
    use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;

    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
    let kp = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
    let pk = kp.public_key().as_ref();
    let device_key_id = device_key_id_from_public_key_bytes(pk);
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode(pk);

    let mut db = StubDeviceKeyDb::new();
    db.refuse_upload_binding = refuse_binding;
    let stub = Arc::new(db);
    stub.insert_test_device_key(
        tenant_id,
        &device_key_id,
        DeviceKeyRecord {
            device_key_id: device_key_id.clone(),
            tenant_id: tenant_id.to_string(),
            public_key: pk_b64,
            invite_subject_hash: Some("sha256:stub".to_string()),
            client_info: serde_json::json!({}),
            created_at: Utc::now(),
            revoked_at: None,
        },
    );

    let config = TraceUploadClaimIssuerConfig {
        onboarding_device_key_db: Some(stub as Arc<dyn crate::db::Database>),
        ..test_config()
    };

    let body_str = body.to_string();
    let sig = kp.sign(body_str.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.as_ref());

    let (status, response) = if device_jwt {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(device_key_id.clone());
        let now = Utc::now();
        let token = jsonwebtoken::encode(
            &header,
            &json!({"tenant_id": tenant_id, "aud": "trace-commons-upload",
                    "iat": now.timestamp(), "exp": (now + Duration::minutes(5)).timestamp()}),
            &EncodingKey::from_ed_der(pkcs8.as_ref()),
        )
        .unwrap();
        let router = trace_upload_claim_issuer_router(config).unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/trace-upload-claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(body_str))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    } else {
        post_device_claim(config, &device_key_id, &sig_b64, body).await
    };
    (status, response, device_key_id)
}

#[tokio::test]
async fn device_claim_refuses_token_when_upload_ownership_cannot_be_recorded() {
    let tenant = "test-device-tenant";
    for subject in [None, Some("sha256:subject")] {
        let (status, body, _) = post_signed_device_claim_with_binding(
            tenant,
            device_claim_request_body(tenant, subject),
            true,
            false,
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.get("access_token").is_none());
    }
}

#[tokio::test]
async fn device_jwt_claim_refuses_token_when_upload_ownership_cannot_be_recorded() {
    let tenant = "test-device-tenant";
    let (status, body, _) = post_signed_device_claim_with_binding(
        tenant,
        device_claim_request_body(tenant, None),
        true,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.get("access_token").is_none());
}

#[tokio::test]
async fn device_jwt_claim_succeeds_when_upload_ownership_is_recorded() {
    let tenant = "test-device-tenant";
    let (status, body, device) = post_signed_device_claim_with_binding(
        tenant,
        device_claim_request_body(tenant, None),
        false,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decode_issued_claims(&body)["principal_ref"], device);
}

fn decode_issued_claims(body: &serde_json::Value) -> serde_json::Value {
    let token = body["access_token"].as_str().expect("access_token field");
    let payload = token.split('.').nth(1).expect("jwt has 3 parts");
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("payload is base64url");
    serde_json::from_slice(&decoded).expect("payload is json")
}

#[tokio::test]
async fn device_claim_without_subject_uses_raw_device_key_id() {
    let tenant_id = "test-device-tenant";
    let body = device_claim_request_body(tenant_id, None);
    let (status, response, device_key_id) =
        post_signed_device_claim_for_tenant(tenant_id, body).await;
    assert_eq!(status, StatusCode::OK, "claim must succeed: {:?}", response);
    let claims = decode_issued_claims(&response);
    assert_eq!(
        claims["sub"].as_str(),
        Some(device_key_id.as_str()),
        "sub must be raw device_key_id when no subject"
    );
    assert_eq!(
        claims["principal_ref"].as_str(),
        Some(device_key_id.as_str()),
        "principal_ref must be raw device_key_id when no subject"
    );
}

#[tokio::test]
async fn device_claim_with_subject_yields_namespaced_principal() {
    let tenant_id = "test-device-tenant";
    let subject = "user-abc123";
    let body = device_claim_request_body(tenant_id, Some(subject));
    let (status, response, device_key_id) =
        post_signed_device_claim_for_tenant(tenant_id, body).await;
    assert_eq!(status, StatusCode::OK, "claim must succeed: {:?}", response);
    let claims = decode_issued_claims(&response);
    let expected = format!("instance:{tenant_id}:{device_key_id}:user:{subject}");
    assert_eq!(
        claims["sub"].as_str(),
        Some(expected.as_str()),
        "sub must be namespaced with subject"
    );
    assert_eq!(
        claims["principal_ref"].as_str(),
        Some(expected.as_str()),
        "principal_ref must be namespaced with subject"
    );
}

#[tokio::test]
async fn empty_request_on_hardcoded_floor_grants_exactly_implied_uses() {
    // Regression pin: a device-key claim with no registered tenant
    // access grant falls back to the hardcoded pilot floor
    // (debugging_evaluation + public_attribution scopes). An empty
    // consent_scopes/allowed_uses request must resolve to exactly the
    // uses those floor scopes imply — [debugging, evaluation,
    // aggregate_analytics] — never the full uses ceiling by coincidence.
    let tenant_id = "test-device-tenant-floor";
    let mut body = device_claim_request_body(tenant_id, None);
    body["consent_scopes"] = json!([]);
    body["allowed_uses"] = json!([]);
    let (status, response, _device_key_id) =
        post_signed_device_claim_for_tenant(tenant_id, body).await;
    assert_eq!(status, StatusCode::OK, "claim must succeed: {:?}", response);
    let claims = decode_issued_claims(&response);
    assert_eq!(
        claims["allowed_consent_scopes"],
        json!(["debugging_evaluation", "public_attribution"]),
        "a grant-free device upload claim must retain a nonempty scope ceiling"
    );
    assert_eq!(
        claims["allowed_uses"],
        json!(["debugging", "evaluation", "aggregate_analytics"]),
        "empty request on hardcoded floor must grant exactly the implied uses"
    );
}

#[tokio::test]
async fn distinct_subjects_yield_distinct_device_key_principals() {
    let tenant_id = "test-device-tenant-distinct";

    let subject_alice = "user-alice-hash";
    let body_alice = device_claim_request_body(tenant_id, Some(subject_alice));
    let (status_alice, response_alice, device_key_id_alice) =
        post_signed_device_claim_for_tenant(tenant_id, body_alice).await;
    assert_eq!(
        status_alice,
        StatusCode::OK,
        "alice claim must succeed: {:?}",
        response_alice
    );
    let claims_alice = decode_issued_claims(&response_alice);
    let expected_alice = format!("instance:{tenant_id}:{device_key_id_alice}:user:{subject_alice}");
    assert_eq!(
        claims_alice["principal_ref"].as_str(),
        Some(expected_alice.as_str()),
        "alice principal_ref must be namespaced with subject"
    );
    let p1 = claims_alice["principal_ref"]
        .as_str()
        .expect("alice principal_ref");

    let subject_bob = "user-bob-hash";
    let body_bob = device_claim_request_body(tenant_id, Some(subject_bob));
    let (status_bob, response_bob, _device_key_id_bob) =
        post_signed_device_claim_for_tenant(tenant_id, body_bob).await;
    assert_eq!(
        status_bob,
        StatusCode::OK,
        "bob claim must succeed: {:?}",
        response_bob
    );
    let claims_bob = decode_issued_claims(&response_bob);
    let p2 = claims_bob["principal_ref"]
        .as_str()
        .expect("bob principal_ref");

    assert_ne!(p1, p2, "distinct subjects must yield distinct principals");
}

/// Generate real Ed25519 keypairs until one's base64 body happens to
/// contain the literal substring "RSA" (roughly 1-in-a-few-hundred keys),
/// so the validators must not reject it on that basis alone.
fn generate_ed25519_keypair_with_rsa_in_body() -> GeneratedUploadClaimKeypair {
    for _ in 0..100_000 {
        let keypair = generate_upload_claim_keypair().expect("keypair generates");
        let private_body: String = keypair
            .private_key_pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let public_body: String = keypair
            .public_key_pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        if private_body.contains("RSA") || public_body.contains("RSA") {
            return keypair;
        }
    }
    panic!("did not find an Ed25519 key with \"RSA\" in its base64 body after 100000 tries");
}

#[test]
fn validate_eddsa_private_key_pem_accepts_ed25519_key_with_rsa_in_body() {
    let keypair = generate_ed25519_keypair_with_rsa_in_body();
    let normalized = validate_eddsa_private_key_pem(&keypair.private_key_pem).expect(
        "a valid Ed25519 PKCS#8 key must be accepted even if its base64 body contains \"RSA\"",
    );
    // Downstream acceptance: the normalized PEM still parses as a signing key.
    EncodingKey::from_ed_pem(normalized.as_bytes()).expect("normalized key still parses");
}

#[test]
fn validate_eddsa_public_key_pem_accepts_ed25519_key_with_rsa_in_body() {
    let keypair = generate_ed25519_keypair_with_rsa_in_body();
    let normalized = validate_eddsa_public_key_pem(&keypair.public_key_pem).expect(
        "a valid Ed25519 SPKI key must be accepted even if its base64 body contains \"RSA\"",
    );
    DecodingKey::from_ed_pem(normalized.as_bytes()).expect("normalized key still parses");
}

#[test]
fn validate_eddsa_private_key_pem_still_rejects_real_rsa_header() {
    let rsa_pem =
        "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAK\n-----END RSA PRIVATE KEY-----\n";
    let err = validate_eddsa_private_key_pem(rsa_pem)
        .expect_err("an actual RSA-headered key must still be rejected");
    assert!(err.to_string().contains("RSA"));
}

#[test]
fn validate_eddsa_public_key_pem_still_rejects_real_rsa_header() {
    let rsa_pem = "-----BEGIN RSA PUBLIC KEY-----\nMIIBOgIBAAJBAK\n-----END RSA PUBLIC KEY-----\n";
    let err = validate_eddsa_public_key_pem(rsa_pem)
        .expect_err("an actual RSA-headered key must still be rejected");
    assert!(err.to_string().contains("RSA"));
}

// ---- A device reading its own invite subject hash ----

mod device_invite_subject {
    use super::*;
    use ring::signature::KeyPair;
    use std::sync::Arc;
    use trace_commons_protocol::device_invite_subject::{
        DEVICE_INVITE_SUBJECT_PATH, DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION,
    };
    use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;

    const TENANT: &str = "tenant-invitee";
    const INVITE: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    struct Device {
        key: ring::signature::Ed25519KeyPair,
        id: String,
        public_key: String,
    }

    fn device() -> Device {
        let pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap();
        let key = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let id = device_key_id_from_public_key_bytes(key.public_key().as_ref());
        let public_key =
            base64::engine::general_purpose::STANDARD.encode(key.public_key().as_ref());
        Device {
            key,
            id,
            public_key,
        }
    }

    fn record(device: &Device, invite: Option<&str>, revoked: bool) -> DeviceKeyRecord {
        DeviceKeyRecord {
            device_key_id: device.id.clone(),
            tenant_id: TENANT.to_string(),
            public_key: device.public_key.clone(),
            invite_subject_hash: invite.map(str::to_string),
            client_info: json!({}),
            created_at: Utc::now(),
            revoked_at: revoked.then(Utc::now),
        }
    }

    fn config_with(records: Vec<DeviceKeyRecord>) -> TraceUploadClaimIssuerConfig {
        let stub = Arc::new(StubDeviceKeyDb::new());
        for record in records {
            stub.insert_test_device_key(
                &record.tenant_id.clone(),
                &record.device_key_id.clone(),
                record,
            );
        }
        TraceUploadClaimIssuerConfig {
            onboarding_device_key_db: Some(stub as Arc<dyn crate::db::Database>),
            ..test_config()
        }
    }

    fn body(device_key_id: &str, issued_at: i64) -> String {
        json!({
            "schema_version": DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION,
            "tenant_id": TENANT,
            "device_key_id": device_key_id,
            "issued_at": issued_at,
        })
        .to_string()
    }

    fn sign(device: &Device, body: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(device.key.sign(body.as_bytes()).as_ref())
    }

    async fn post(
        config: TraceUploadClaimIssuerConfig,
        headers: &[(&str, &str)],
        body: String,
    ) -> (StatusCode, serde_json::Value) {
        let router = trace_upload_claim_issuer_router(config).expect("router builds");
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(DEVICE_INVITE_SUBJECT_PATH)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = router
            .oneshot(request.body(Body::from(body)).expect("request builds"))
            .await
            .expect("request completes");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).expect("json response"),
        )
    }

    async fn signed(
        config: TraceUploadClaimIssuerConfig,
        device: &Device,
        body: String,
    ) -> (StatusCode, serde_json::Value) {
        let signature = sign(device, &body);
        post(
            config,
            &[
                (TRACE_DEVICE_KEY_ID_HEADER, device.id.as_str()),
                (TRACE_DEVICE_SIGNATURE_HEADER, signature.as_str()),
            ],
            body,
        )
        .await
    }

    fn error(body: &serde_json::Value) -> Option<&str> {
        body.get("error").and_then(|v| v.as_str())
    }

    #[tokio::test]
    async fn a_device_reads_its_own_invite_subject_hash() {
        let d = device();
        let (status, response) = signed(
            config_with(vec![record(&d, Some(INVITE), false)]),
            &d,
            body(&d.id, Utc::now().timestamp()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response, json!({ "invite_subject_hash": INVITE }));
    }

    #[tokio::test]
    async fn an_unsigned_request_is_refused() {
        let d = device();
        let (status, response) = post(
            config_with(vec![record(&d, Some(INVITE), false)]),
            &[],
            body(&d.id, Utc::now().timestamp()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(
            error(&response),
            Some("device key auth requires id and signature")
        );
    }

    /// Another device's key cannot read this device's hash, and a body
    /// changed after signing is not the body that was signed.
    #[tokio::test]
    async fn only_the_device_s_own_key_over_the_exact_body_is_accepted() {
        let d = device();
        let other = device();
        let sent = body(&d.id, Utc::now().timestamp());
        let (status, response) = post(
            config_with(vec![record(&d, Some(INVITE), false)]),
            &[
                (TRACE_DEVICE_KEY_ID_HEADER, d.id.as_str()),
                (TRACE_DEVICE_SIGNATURE_HEADER, sign(&other, &sent).as_str()),
            ],
            sent.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        assert_eq!(error(&response), Some("invalid device key signature"));

        let signature = sign(&d, &sent);
        let tampered = sent.replace(TENANT, "tenant-other");
        let (status, response) = post(
            config_with(vec![record(&d, Some(INVITE), false)]),
            &[
                (TRACE_DEVICE_KEY_ID_HEADER, d.id.as_str()),
                (TRACE_DEVICE_SIGNATURE_HEADER, signature.as_str()),
            ],
            tampered,
        )
        .await;
        assert_ne!(status, StatusCode::OK, "{response}");
    }

    #[tokio::test]
    async fn the_header_and_the_body_must_name_the_same_device() {
        let d = device();
        let other = device();
        let (status, response) = signed(
            config_with(vec![record(&d, Some(INVITE), false)]),
            &d,
            body(&other.id, Utc::now().timestamp()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(error(&response), Some("device key id mismatch"));
    }

    #[tokio::test]
    async fn an_unknown_device_is_refused_by_name() {
        let d = device();
        let (status, response) =
            signed(config_with(vec![]), &d, body(&d.id, Utc::now().timestamp())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        assert_eq!(error(&response), Some("device_key_not_registered"));
    }

    #[tokio::test]
    async fn a_revoked_device_is_refused_by_name() {
        let d = device();
        let (status, response) = signed(
            config_with(vec![record(&d, Some(INVITE), true)]),
            &d,
            body(&d.id, Utc::now().timestamp()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        assert_eq!(error(&response), Some("device_key_revoked"));
    }

    /// A NEAR or NEAR AI device redeemed no invite.
    #[tokio::test]
    async fn a_device_onboarded_without_an_invite_is_refused_by_name() {
        let d = device();
        let (status, response) = signed(
            config_with(vec![record(&d, None, false)]),
            &d,
            body(&d.id, Utc::now().timestamp()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        assert_eq!(error(&response), Some("device_not_invite_onboarded"));
    }

    /// A minute past the bound either way. The route reads its clock
    /// after the test does, and both truncate to whole seconds, so a body
    /// one second past the bound can land on it and be served; the exact
    /// bound is `the_skew_bound_is_inclusive_either_way`, on a fixed
    /// clock.
    #[tokio::test]
    async fn a_stale_or_future_request_is_refused() {
        let d = device();
        let past_the_bound = DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS + 60;
        for offset in [-past_the_bound, past_the_bound] {
            let (status, response) = signed(
                config_with(vec![record(&d, Some(INVITE), false)]),
                &d,
                body(&d.id, Utc::now().timestamp() + offset),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
            assert_eq!(
                error(&response),
                Some("device_invite_subject_request_stale")
            );
        }
    }

    #[test]
    fn the_skew_bound_is_inclusive_either_way() {
        let now = 1_800_000_000;
        let bound = DEVICE_INVITE_SUBJECT_MAX_SKEW_SECONDS;
        for issued_at in [now, now - bound, now + bound] {
            assert!(
                device_invite_subject_is_fresh(now, issued_at),
                "{issued_at}"
            );
        }
        // `now + i64::MIN` is 2^63 seconds back: `now - issued_at`
        // overflows, which panics a debug build and wraps a release
        // build to `i64::MIN`, whose `abs()` stays negative and so read
        // as fresh.
        for issued_at in [
            now - bound - 1,
            now + bound + 1,
            i64::MIN,
            i64::MAX,
            now + i64::MIN,
        ] {
            assert!(
                !device_invite_subject_is_fresh(now, issued_at),
                "{issued_at}"
            );
        }
    }

    #[tokio::test]
    async fn without_a_device_registry_the_route_fails_closed() {
        let d = device();
        let (status, response) =
            signed(test_config(), &d, body(&d.id, Utc::now().timestamp())).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{response}");
        assert_eq!(
            error(&response),
            Some("device key registry is not configured")
        );
    }

    #[tokio::test]
    async fn an_unknown_field_is_refused() {
        let d = device();
        let mut value: serde_json::Value =
            serde_json::from_str(&body(&d.id, Utc::now().timestamp())).unwrap();
        value["invite_code"] = json!("SMUGGLED");
        let (status, response) = signed(
            config_with(vec![record(&d, Some(INVITE), false)]),
            &d,
            value.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    }
}

/// `POST /v1/invite/lookup` without a database: routing, throttling,
/// fail-closed behaviour and secret-free logging. The database-backed
/// behaviour (labels, no use spent) is `tests/trace_invite_registry_pg.rs`.
pub(crate) mod invite_lookup {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;
    use trace_commons_protocol::invite_lookup::{
        INVITE_LOOKUP_PATH, INVITE_LOOKUP_REQUEST_SCHEMA_VERSION,
    };

    const CODE: &str = "ABCDEFGHJKLMNPQR";

    std::thread_local! {
        static CAPTURE: std::cell::RefCell<Option<Arc<Mutex<Vec<u8>>>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Writes into this thread's capture buffer, if one is open.
    struct ThreadCapture;

    impl Write for ThreadCapture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            CAPTURE.with(|capture| {
                if let Some(sink) = capture.borrow().as_ref() {
                    sink.lock().unwrap().extend_from_slice(buf);
                }
            });
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Capture every event emitted on this thread while `f` runs.
    ///
    /// A process-global subscriber, not a scoped one. With a single
    /// scoped dispatcher, tracing-core resolves a callsite's first
    /// registration against whichever thread registers it, so a parallel
    /// test that first hit the handler's `info!` after a scoped
    /// subscriber went in cached it as disabled for every thread, and
    /// this test failed every run under `cargo test --lib invite_lookup`.
    /// A global subscriber gives every thread the same answer; its
    /// filter admits only threads with a capture open. The filter must
    /// be dynamic: a plain `filter_fn` caches its first answer per
    /// callsite, so whichever thread registered the handler's `info!`
    /// would decide for all of them again.
    ///
    /// The lib's only global subscriber, so other modules' tests that
    /// need a log capture call this one (`payout_pass`).
    pub(crate) fn capture_logs(f: impl FnOnce()) -> String {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(|| ThreadCapture)
                .with_ansi(false)
                .with_filter(tracing_subscriber::filter::dynamic_filter_fn(|_, _| {
                    CAPTURE.with(|capture| capture.borrow().is_some())
                }));
            tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
                .expect("no other global subscriber in the lib tests");
        });
        let sink = Arc::new(Mutex::new(Vec::new()));
        CAPTURE.with(|capture| *capture.borrow_mut() = Some(sink.clone()));
        f();
        CAPTURE.with(|capture| *capture.borrow_mut() = None);
        let bytes = sink.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    fn request_body(code: &str) -> String {
        json!({
            "schema_version": INVITE_LOOKUP_REQUEST_SCHEMA_VERSION,
            "invite_code": code,
        })
        .to_string()
    }

    fn state_with_rate(rate: u32) -> Arc<TraceUploadClaimIssuerState> {
        state_with_client_header(rate, None, 0)
    }

    /// `client_header` is the trusted header name, or `None` for the
    /// default (no header trusted).
    fn state_with_client_header(
        global: u32,
        client_header: Option<&'static str>,
        per_client: u32,
    ) -> Arc<TraceUploadClaimIssuerState> {
        let mut state = test_config().build_state().expect("state builds");
        let inner = Arc::get_mut(&mut state).expect("state is unshared");
        inner.invite_lookup_rate_per_min = global;
        inner.invite_lookup_client_ip_header =
            client_header.map(axum::http::HeaderName::from_static);
        inner.invite_lookup_per_client_rate_per_min = per_client;
        state
    }

    async fn post(
        state: Arc<TraceUploadClaimIssuerState>,
        body: String,
    ) -> (StatusCode, serde_json::Value) {
        post_from(state, body, None).await
    }

    /// POST as a caller whose proxy wrote `forwarded_for` into
    /// `X-Forwarded-For` (or wrote nothing).
    async fn post_from(
        state: Arc<TraceUploadClaimIssuerState>,
        body: String,
        forwarded_for: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let router = router_from_state(state, StdDuration::from_secs(5), 64 * 1024);
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(INVITE_LOOKUP_PATH)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = forwarded_for {
            request = request.header("x-forwarded-for", value);
        }
        let response = router
            .oneshot(request.body(Body::from(body)).expect("request builds"))
            .await
            .expect("request completes");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        (status, serde_json::from_slice(&bytes).expect("json"))
    }

    #[tokio::test]
    async fn a_malformed_code_is_answered_without_a_database() {
        let (status, body) = post(state_with_rate(30), request_body("short")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"valid": false, "reason_label": "malformed"}));
    }

    #[tokio::test]
    async fn a_well_formed_code_fails_closed_without_the_invite_database() {
        let (status, body) = post(state_with_rate(30), request_body(CODE)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, json!({"error": "invite_registry_not_configured"}));
    }

    #[tokio::test]
    async fn a_wrong_schema_or_extra_field_is_a_bad_request() {
        let state = state_with_rate(30);
        let wrong = json!({"schema_version": "nope", "invite_code": CODE}).to_string();
        assert_eq!(post(state.clone(), wrong).await.0, StatusCode::BAD_REQUEST);
        let extra = json!({
            "schema_version": INVITE_LOOKUP_REQUEST_SCHEMA_VERSION,
            "invite_code": CODE,
            "device_public_key": "x",
        })
        .to_string();
        assert_eq!(post(state, extra).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn lookups_past_the_budget_are_throttled() {
        let state = state_with_rate(2);
        for _ in 0..2 {
            let (status, _) = post(state.clone(), request_body("short")).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, body) = post(state.clone(), request_body("short")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body, json!({"error": "invite_lookup_rate_limited"}));
        // Throttled before the body is even parsed, so garbage is also
        // refused rather than answered.
        let (status, _) = post(state, "not json".to_string()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn no_client_header_is_trusted_unless_configured() {
        // Default: one shared bucket. A caller cannot buy a fresh budget
        // by sending a different X-Forwarded-For.
        let state = state_with_client_header(2, None, 100);
        for ip in ["203.0.113.1", "203.0.113.2"] {
            let (status, _) = post_from(state.clone(), request_body("short"), Some(ip)).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _) = post_from(state, request_body("short"), Some("203.0.113.3")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn a_configured_client_header_gives_each_client_its_own_budget() {
        let state = state_with_client_header(100, Some("x-forwarded-for"), 2);
        for _ in 0..2 {
            let (status, _) =
                post_from(state.clone(), request_body("short"), Some("203.0.113.1")).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, body) =
            post_from(state.clone(), request_body("short"), Some("203.0.113.1")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body, json!({"error": "invite_lookup_rate_limited"}));
        // A client-written prefix does not escape: the proxy's own hop is
        // the rightmost one.
        let (status, _) = post_from(
            state.clone(),
            request_body("short"),
            Some("198.51.100.9, 203.0.113.1"),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // Another client is unaffected by the first one's exhaustion.
        let (status, _) =
            post_from(state.clone(), request_body("short"), Some("203.0.113.2")).await;
        assert_eq!(status, StatusCode::OK);
        // Callers the proxy did not attribute share one bucket.
        for _ in 0..2 {
            let (status, _) = post_from(state.clone(), request_body("short"), None).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _) = post_from(state, request_body("short"), None).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn the_shared_budget_still_caps_all_clients_together() {
        let state = state_with_client_header(3, Some("x-forwarded-for"), 1);
        // One client over its own budget does not spend the shared one.
        let (status, _) =
            post_from(state.clone(), request_body("short"), Some("203.0.113.1")).await;
        assert_eq!(status, StatusCode::OK);
        for _ in 0..5 {
            let (status, _) =
                post_from(state.clone(), request_body("short"), Some("203.0.113.1")).await;
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        }
        for ip in ["203.0.113.2", "203.0.113.3"] {
            let (status, _) = post_from(state.clone(), request_body("short"), Some(ip)).await;
            assert_eq!(status, StatusCode::OK, "{ip}");
        }
        // Three fresh clients have now used the shared three.
        let (status, _) = post_from(state, request_body("short"), Some("203.0.113.4")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn a_zero_budget_refuses_every_lookup() {
        let (status, _) = post(state_with_rate(0), request_body("short")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn the_code_never_reaches_a_log_line() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let logged = capture_logs(|| {
            runtime.block_on(async {
                let state = state_with_rate(30);
                post(state.clone(), request_body(CODE)).await;
                post(state.clone(), request_body("short-but-secret")).await;
                post(state, "garbage-SECRETBODY".to_string()).await;
            });
        });
        assert!(logged.contains("invite lookup"), "no log emitted: {logged}");
        for secret in [CODE, "short-but-secret", "SECRETBODY"] {
            assert!(
                !logged.contains(secret),
                "{secret} reached the log: {logged}"
            );
        }
    }
}

// ---- Org tenants (enterprise spec 5.3, 11) ----

/// A device-key registry and grant store holding one org, one seat for the
/// device, and the device's grants.
struct StubOrgDb {
    device_keys: std::collections::HashMap<(String, String), crate::db::DeviceKeyRecord>,
    org: Option<crate::org::OrgRecord>,
    seat: Option<crate::org::OrgSeatRecord>,
    grants: Vec<TraceTenantAccessGrantRecord>,
}

#[async_trait::async_trait]
impl crate::trace_corpus_storage::TraceCorpusStore for StubOrgDb {
    async fn list_active_trace_tenant_access_grants_for_principal(
        &self,
        _tenant_id: &str,
        principal_ref: &str,
        _now: chrono::DateTime<Utc>,
    ) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError> {
        Ok(self
            .grants
            .iter()
            .filter(|grant| grant.principal_ref == principal_ref)
            .cloned()
            .collect())
    }
}

#[async_trait::async_trait]
impl crate::db::Database for StubOrgDb {
    async fn bind_device_upload_principal(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
        _actor_ref: &str,
    ) -> Result<(), DatabaseError> {
        Ok(())
    }

    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        Ok(())
    }

    async fn enroll_instance_user(&self, _p: InstanceUserProvision) -> Result<(), DatabaseError> {
        Err(DatabaseError::Pool("stub".into()))
    }

    async fn reserve_instance_enrollment(
        &self,
        _instance_subject_hash: &str,
        _user_subject_hash: &str,
        _tenant_id: &str,
        _max_enrollments: i64,
    ) -> Result<InstanceEnrollmentOutcome, DatabaseError> {
        Err(DatabaseError::Pool("stub".into()))
    }

    async fn instance_ledger_rls_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }

    async fn get_device_key(
        &self,
        tenant_id: &str,
        device_key_id: &str,
    ) -> Result<Option<DeviceKeyRecord>, DatabaseError> {
        Ok(self
            .device_keys
            .get(&(tenant_id.to_string(), device_key_id.to_string()))
            .cloned())
    }

    async fn get_org(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<crate::org::OrgRecord>, DatabaseError> {
        Ok(self.org.clone())
    }

    async fn get_org_seat_for_device(
        &self,
        _tenant_id: &str,
        _device_key_id: &str,
    ) -> Result<Option<crate::org::OrgSeatRecord>, DatabaseError> {
        Ok(self.seat.clone())
    }
}

const ORG_TEST_TENANT: &str =
    "org-abababababababababababababababababababababababababababababababab";

fn org_test_record() -> crate::org::OrgRecord {
    let policy = trace_commons_protocol::org_policy::OrgPolicy::conservative_default();
    crate::org::OrgRecord {
        tenant_id: ORG_TEST_TENANT.to_string(),
        status: trace_commons_protocol::org_policy::OrgStatus::Active,
        holder_account_id: Some(Uuid::new_v4()),
        holder_account_open: true,
        near_ai_org_bound: true,
        policy_digest: policy.digest(),
        policy,
        policy_version: 1,
        seat_limit: 10,
        server_limit: 2,
        settlement_cap_micros: Some(1_000_000),
    }
}

fn org_test_member_seat() -> crate::org::OrgSeatRecord {
    crate::org::OrgSeatRecord {
        account_id: Uuid::new_v4(),
        kind: trace_commons_protocol::org_policy::OrgSeatKind::Member,
        org_role: Some(trace_commons_protocol::org_policy::OrgRole::Member),
        membership_proved_at: Some(Utc::now()),
        requires_membership_proof: true,
        joined_at: Utc::now(),
        removed_at: None,
    }
}

fn org_claim_body(digest: Option<&str>, send_mode: &str, basis: &str) -> serde_json::Value {
    let mut body = device_claim_request_body(ORG_TEST_TENANT, None);
    body["consent_scopes"] = json!(["debugging_evaluation", "model_training"]);
    body["allowed_uses"] = json!([]);
    body["consent_basis"] = json!(basis);
    body["send_mode"] = json!(send_mode);
    if let Some(digest) = digest {
        body["policy_digest"] = json!(digest);
    }
    body
}

/// Post a signed device claim into the org test tenant. `grant` gives the
/// device a contributor grant for every scope and use when true.
async fn post_org_device_claim(
    org: Option<crate::org::OrgRecord>,
    seat: Option<crate::org::OrgSeatRecord>,
    grant: bool,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    use ring::signature::KeyPair;
    use std::sync::Arc;
    use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;

    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("keypair generates");
    let kp = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("keypair parses");
    let pk = kp.public_key().as_ref();
    let device_key_id = device_key_id_from_public_key_bytes(pk);
    let mut device_keys = std::collections::HashMap::new();
    device_keys.insert(
        (ORG_TEST_TENANT.to_string(), device_key_id.clone()),
        DeviceKeyRecord {
            device_key_id: device_key_id.clone(),
            tenant_id: ORG_TEST_TENANT.to_string(),
            public_key: base64::engine::general_purpose::STANDARD.encode(pk),
            invite_subject_hash: Some("sha256:stub".to_string()),
            client_info: json!({}),
            created_at: Utc::now(),
            revoked_at: None,
        },
    );
    let mut grants = Vec::new();
    if grant {
        let mut record = test_tenant_access_grant(
            Utc::now(),
            TraceTenantAccessGrantRole::Contributor,
            vec![
                "debugging_evaluation",
                "model_training",
                "public_attribution",
            ],
            vec!["debugging", "evaluation", "model_training"],
            None,
            None,
            None,
        );
        record.tenant_id = ORG_TEST_TENANT.to_string();
        record.principal_ref =
            crate::account_trust::device_principal_ref(ORG_TEST_TENANT, &device_key_id);
        grants.push(record);
    }
    let db = Arc::new(StubOrgDb {
        device_keys,
        org,
        seat,
        grants,
    });
    let config = TraceUploadClaimIssuerConfig {
        onboarding_device_key_db: Some(db.clone() as Arc<dyn crate::db::Database>),
        tenant_access_grant_db: Some(db as Arc<dyn crate::db::Database>),
        ..test_config()
    };
    let signature = kp.sign(body.to_string().as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(signature.as_ref());
    post_device_claim(config, &device_key_id, &signature, body).await
}

#[tokio::test]
async fn org_member_claim_is_narrowed_by_policy_and_records_consent_basis() {
    let org = org_test_record();
    let digest = org.policy_digest.clone();
    let (status, body) = post_org_device_claim(
        Some(org),
        Some(org_test_member_seat()),
        true,
        org_claim_body(Some(&digest), "submit", "member_grant"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let claims = decode_issued_claims(&body);
    assert_eq!(claims["consent_basis"], "member_grant");
    // The grant allows model training; the org policy does not, so the claim
    // carries only what both allow.
    assert_eq!(
        claims["allowed_consent_scopes"],
        json!(["debugging_evaluation"])
    );
    assert_eq!(claims["allowed_uses"], json!(["debugging", "evaluation"]));
}

#[tokio::test]
async fn org_claim_refusals_fail_closed() {
    let org = org_test_record();
    let digest = org.policy_digest.clone();
    let ok_body = || org_claim_body(Some(&digest), "submit", "member_grant");

    // No org record for an org-namespace tenant.
    let (status, body) =
        post_org_device_claim(None, Some(org_test_member_seat()), true, ok_body()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "org_record_missing");

    // A device with no seat.
    let (status, body) = post_org_device_claim(Some(org.clone()), None, true, ok_body()).await;
    assert_eq!(body["error"], "org_seat_missing", "{status}");

    // No grant: no floor in an org tenant.
    let (status, body) = post_org_device_claim(
        Some(org.clone()),
        Some(org_test_member_seat()),
        false,
        ok_body(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "org_seat_grant_missing");

    // Automatic while the org forbids it.
    let (_, body) = post_org_device_claim(
        Some(org.clone()),
        Some(org_test_member_seat()),
        true,
        org_claim_body(Some(&digest), "automatic", "member_grant"),
    )
    .await;
    assert_eq!(body["error"], "org_automatic_forbidden");

    // A stale digest, and none at all.
    let (_, body) = post_org_device_claim(
        Some(org.clone()),
        Some(org_test_member_seat()),
        true,
        org_claim_body(
            Some(&format!("sha256:{}", "00".repeat(32))),
            "submit",
            "member_grant",
        ),
    )
    .await;
    assert_eq!(body["error"], "org_policy_digest_stale");
    let (_, body) = post_org_device_claim(
        Some(org.clone()),
        Some(org_test_member_seat()),
        true,
        org_claim_body(None, "submit", "member_grant"),
    )
    .await;
    assert_eq!(body["error"], "org_policy_digest_missing");

    // A member claiming the org's consent.
    let (_, body) = post_org_device_claim(
        Some(org.clone()),
        Some(org_test_member_seat()),
        true,
        org_claim_body(Some(&digest), "submit", "org_policy"),
    )
    .await;
    assert_eq!(body["error"], "org_consent_basis_mismatch");

    // A removed seat, and a stale membership.
    let mut removed = org_test_member_seat();
    removed.removed_at = Some(Utc::now());
    let (_, body) = post_org_device_claim(Some(org.clone()), Some(removed), true, ok_body()).await;
    assert_eq!(body["error"], "org_seat_removed");
    let mut stale = org_test_member_seat();
    stale.membership_proved_at = Some(Utc::now() - Duration::days(2));
    let (_, body) = post_org_device_claim(Some(org.clone()), Some(stale), true, ok_body()).await;
    assert_eq!(body["error"], "org_membership_stale");

    // A per-user subject has no meaning for an org seat.
    let mut with_subject = ok_body();
    with_subject["subject"] = json!("user-1");
    let (_, body) = post_org_device_claim(
        Some(org.clone()),
        Some(org_test_member_seat()),
        true,
        with_subject,
    )
    .await;
    assert_eq!(body["error"], "org_subject_unsupported");

    // Sharing off.
    let mut off = org.clone();
    off.policy.sharing = trace_commons_protocol::org_policy::OrgSharing::Off;
    off.policy_digest = off.policy.digest();
    let off_digest = off.policy_digest.clone();
    let (_, body) = post_org_device_claim(
        Some(off),
        Some(org_test_member_seat()),
        true,
        org_claim_body(Some(&off_digest), "submit", "member_grant"),
    )
    .await;
    assert_eq!(body["error"], "org_sharing_off");
}

#[tokio::test]
async fn ordinary_tenant_claims_carry_no_consent_basis() {
    let tenant_id = "test-device-tenant";
    let mut body = device_claim_request_body(tenant_id, None);
    // A consent basis sent to an ordinary tenant is ignored, not recorded.
    body["consent_basis"] = json!("org_policy");
    let (status, response, _) = post_signed_device_claim_for_tenant(tenant_id, body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(
        decode_issued_claims(&response)
            .get("consent_basis")
            .is_none()
    );
}

#[tokio::test]
async fn workload_tokens_cannot_mint_into_an_org_tenant() {
    let state = test_config().build_state().expect("state builds");
    let workload = WorkloadClaims {
        sub: Some("workload-1".to_string()),
        principal_ref: None,
        tenant_id: Some(ORG_TEST_TENANT.to_string()),
        iss: None,
        aud: None,
        exp: (Utc::now() + Duration::minutes(5)).timestamp(),
        iat: None,
        allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: vec![TraceAllowedUse::Debugging],
        invite_code: None,
    };
    let request: TraceUploadClaimRequest =
        serde_json::from_value(device_claim_request_body(ORG_TEST_TENANT, None)).unwrap();
    let error = state
        .issue_claim(&workload, request)
        .await
        .expect_err("org tenant refused on the workload path");
    assert_eq!(error.message, "org_workload_unsupported");
}
