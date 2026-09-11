// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn account_public_run_routes_require_a_valid_native_session() {
    use axum::body::Body;
    use tower::ServiceExt;

    let db = Arc::new(NativeAuthTestDb::default());
    let (_temp, state) = native_test_state(db.clone());
    let submission_id = Uuid::new_v4();
    let routes = [
        format!("/v1/account/traces/{submission_id}/session-detail"),
        format!("/v1/account/traces/{submission_id}/publication"),
    ];
    let invalid = native_token_value("tenant-a", "invalid-public-run-secret");

    for route in &routes {
        let missing = app(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(route)
                    .body(Body::empty())
                    .expect("missing-session request builds"),
            )
            .await
            .expect("missing-session response");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let invalid = app(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(route)
                    .header(AUTHORIZATION, format!("Bearer {invalid}"))
                    .body(Body::empty())
                    .expect("invalid-session request builds"),
            )
            .await
            .expect("invalid-session response");
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    let secret = "valid-public-run-native-secret";
    db.insert_session(
        "tenant-a",
        Uuid::new_v4(),
        &hash_secret(secret),
        NATIVE_SESSION_CLIENT_KIND,
        Utc::now() + Duration::hours(1),
    );
    let valid = native_token_value("tenant-a", secret);
    for (route, accepted_status) in routes.iter().zip([StatusCode::NOT_FOUND, StatusCode::OK]) {
        let response = app(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(route)
                    .header(AUTHORIZATION, format!("Bearer {valid}"))
                    .body(Body::empty())
                    .expect("valid-session request builds"),
            )
            .await
            .expect("valid-session response");
        assert_eq!(response.status(), accepted_status);
    }
}

#[tokio::test]
async fn public_run_provenance_accepts_only_exact_redacted_sources() {
    use trace_commons_protocol::public_run::{
        PublicRunDraft, PublicRunEvidenceDraft, PublicRunReusePermission,
    };
    use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;

    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let submission_id = Uuid::new_v4();
    let body =
        write_redacted_envelope_to_disk(state.as_ref(), "tenant-public-run", submission_id).await;
    let envelope: TraceContributionEnvelope =
        serde_json::from_slice(&body).expect("decode redacted envelope fixture");
    let event = envelope.events.first().expect("fixture carries one event");
    let excerpt = event
        .redacted_content
        .clone()
        .expect("fixture event carries redacted content");
    let mut draft = PublicRunDraft {
        title: "Reviewed recovery".to_string(),
        outcome_summary: "The redacted workflow completed.".to_string(),
        correction_excerpt: envelope.outcome.human_correction.clone(),
        workflow: "Run the reviewed recovery steps.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: event.event_id,
            excerpt,
        }],
        source_slug: None,
    };

    assert_eq!(validate_public_run_provenance(&draft, &envelope), Ok(()));
    draft.evidence[0].excerpt = "invented evidence".to_string();
    assert_eq!(
        validate_public_run_provenance(&draft, &envelope),
        Err("public run evidence changed")
    );
    draft.evidence[0].event_id = Uuid::new_v4();
    assert_eq!(
        validate_public_run_provenance(&draft, &envelope),
        Err("public run evidence changed")
    );
    draft.evidence[0].event_id = event.event_id;
    draft.evidence[0].excerpt = event
        .redacted_content
        .clone()
        .expect("fixture content remains available");
    draft.correction_excerpt = Some("invented correction".to_string());
    assert_eq!(
        validate_public_run_provenance(&draft, &envelope),
        Err("public run correction changed")
    );
}

#[tokio::test]
async fn account_public_run_publish_route_checks_owner_source_and_exact_approval() {
    use axum::body::Body;
    use tower::ServiceExt;
    use trace_commons_protocol::public_run::{
        PublicRunDraft, PublicRunEvidenceDraft, PublicRunPublishRequest, PublicRunReusePermission,
    };
    use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;
    use trace_commons_server::account_session::{
        AccountAuthMethod, AccountCtx, AccountId, AccountPrincipalSet, account_actor_ref,
    };
    use wiremock::matchers::{method, path};

    let _env_lock = near_ai_env_lock().lock().await;
    let privacy_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data": [{"spans": []}]})),
        )
        .mount(&privacy_server)
        .await;
    let _privacy_env = SelfHostedPrivacyEnvGuard::set(&privacy_server.uri());

    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("public-run-publish-route-{unique}");
    let principal_ref = format!("principal:public-run-publish-route-{unique}");
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(backend.clone() as Arc<dyn Database>),
        None,
        false,
        false,
        false,
        false,
    );
    let account_id = backend
        .create_or_reuse_account(&tenant_id, &principal_ref)
        .await
        .expect("create publication route account");
    let submission_id =
        insert_account_test_submission(backend.as_ref(), &tenant_id, &principal_ref).await;
    let envelope: TraceContributionEnvelope = serde_json::from_slice(
        &write_redacted_envelope_to_disk(state.as_ref(), &tenant_id, submission_id).await,
    )
    .expect("decode accepted redacted envelope");
    let event = envelope.events.first().expect("fixture has evidence");
    let draft = PublicRunDraft {
        title: "Reviewed synthetic recovery".to_string(),
        outcome_summary: "The reviewed synthetic recovery completed.".to_string(),
        correction_excerpt: envelope.outcome.human_correction.clone(),
        workflow: "Apply the reviewed synthetic recovery steps.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: event.event_id,
            excerpt: event
                .redacted_content
                .clone()
                .expect("fixture event has redacted text"),
        }],
        source_slug: None,
    };
    let request = PublicRunPublishRequest {
        approval_sha256: draft
            .approval_sha256(envelope.outcome.task_success, &envelope.schema_version, 0)
            .expect("approval digest"),
        draft,
        task_success: envelope.outcome.task_success,
        contributed_version: envelope.schema_version,
        expected_publication_version: 0,
    };
    let http_request = axum::http::Request::builder()
        .method("PUT")
        .uri(format!("/v1/account/traces/{submission_id}/publication"))
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&request).expect("serialize publication request"),
        ))
        .expect("publication request builds");
    let account_id = AccountId::from_uuid(account_id);
    let ctx = AccountCtx {
        account_id,
        principal_set: AccountPrincipalSet::from_iter_for_test_only([principal_ref]),
        auth_method: AccountAuthMethod::NativeToken,
        tenant_id: tenant_id.clone(),
        actor_ref: account_actor_ref(&account_id),
        auth_credential_id: None,
        client_kind: "native".to_string(),
    };
    let response = Router::new()
        .route(
            "/v1/account/traces/{submission_id}/publication",
            put(account_public_run_publish_handler),
        )
        .layer(Extension(ctx.clone()))
        .with_state(state.clone())
        .oneshot(http_request)
        .await
        .expect("publication route response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(axum::http::header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read publication response"),
    )
    .expect("publication response JSON");
    assert_eq!(body["title"], "Reviewed synthetic recovery");
    assert_eq!(
        body["task_success"],
        serde_json::json!(request.task_success)
    );
    for forbidden in [
        "tenant_id",
        "account_id",
        "submission_id",
        "publication_id",
        "event_id",
        "approval_sha256",
    ] {
        assert!(!body.to_string().contains(forbidden));
    }

    let detail_response = account_public_run_session_detail_handler(
        State(state),
        Extension(ctx),
        AxumPath(submission_id),
    )
    .await
    .expect("session detail response");
    let detail: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(detail_response.into_body(), usize::MAX)
            .await
            .expect("read session detail response"),
    )
    .expect("session detail response JSON");
    assert_eq!(
        detail["task_success"],
        serde_json::json!(request.task_success)
    );
    assert_eq!(detail["owner_state"]["publication"]["slug"], body["slug"]);
    assert_eq!(detail["owner_state"]["expected_publication_version"], 1);
    assert_eq!(
        detail["evidence"][0]["event_id"],
        event.event_id.to_string()
    );
    assert_eq!(
        detail["evidence"][0]["excerpt"],
        event.redacted_content.as_deref().unwrap()
    );

    cleanup_pg_trace_tenant(backend.as_ref(), &tenant_id).await;
}

#[tokio::test]
async fn account_public_run_publish_route_rejects_unauthorized_and_stale_writes() {
    use trace_commons_protocol::public_run::{
        PublicRunDraft, PublicRunEvidenceDraft, PublicRunPublishRequest, PublicRunReusePermission,
    };
    use trace_commons_protocol::trace_contribution::{TaskSuccess, TraceContributionEnvelope};
    use trace_commons_server::account_session::{
        AccountAuthMethod, AccountCtx, AccountId, AccountPrincipalSet, account_actor_ref,
    };
    use wiremock::matchers::{method, path};

    let _env_lock = near_ai_env_lock().lock().await;
    let privacy_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data": [{"spans": []}]})),
        )
        .mount(&privacy_server)
        .await;
    let _privacy_env = SelfHostedPrivacyEnvGuard::set(&privacy_server.uri());

    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("public-run-rejections-{unique}");
    let principal_ref = format!("principal:public-run-rejections-{unique}");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, &principal_ref)
        .await
        .expect("create publication owner");
    let submission_id =
        insert_account_test_submission(backend.as_ref(), &tenant_id, &principal_ref).await;
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(backend.clone() as Arc<dyn Database>),
        None,
        false,
        false,
        false,
        false,
    );
    let envelope: TraceContributionEnvelope = serde_json::from_slice(
        &write_redacted_envelope_to_disk(state.as_ref(), &tenant_id, submission_id).await,
    )
    .expect("decode accepted redacted envelope");
    let event = envelope.events.first().expect("fixture has evidence");
    let draft = PublicRunDraft {
        title: "Reviewed rejection test".to_string(),
        outcome_summary: "The reviewed rejection test completed.".to_string(),
        correction_excerpt: envelope.outcome.human_correction.clone(),
        workflow: "Run the reviewed rejection steps.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: event.event_id,
            excerpt: event
                .redacted_content
                .clone()
                .expect("fixture event has redacted text"),
        }],
        source_slug: None,
    };
    let request = PublicRunPublishRequest {
        approval_sha256: draft
            .approval_sha256(envelope.outcome.task_success, &envelope.schema_version, 0)
            .expect("approval digest"),
        draft,
        task_success: envelope.outcome.task_success,
        contributed_version: envelope.schema_version.clone(),
        expected_publication_version: 0,
    };
    let account_id = AccountId::from_uuid(account_id);
    let ctx = AccountCtx {
        account_id,
        principal_set: AccountPrincipalSet::from_iter_for_test_only([principal_ref]),
        auth_method: AccountAuthMethod::NativeToken,
        tenant_id: tenant_id.clone(),
        actor_ref: account_actor_ref(&account_id),
        auth_credential_id: None,
        client_kind: "native".to_string(),
    };

    let mut cross_origin_headers = HeaderMap::new();
    cross_origin_headers.insert(
        axum::http::header::ORIGIN,
        HeaderValue::from_static("https://attacker.invalid"),
    );
    cross_origin_headers.insert(
        axum::http::header::HOST,
        HeaderValue::from_static("tracecommons.ai"),
    );
    let cross_origin = account_public_run_publish_handler(
        State(state.clone()),
        Extension(ctx.clone()),
        cross_origin_headers,
        AxumPath(submission_id),
        Json(request.clone()),
    )
    .await
    .expect_err("cross-origin publication must fail");
    assert_eq!(cross_origin.0, StatusCode::FORBIDDEN);

    let other_principal = format!("principal:public-run-other-{unique}");
    let other_account_id = AccountId::from_uuid(
        backend
            .create_or_reuse_account(&tenant_id, &other_principal)
            .await
            .expect("create non-owner account"),
    );
    let other_ctx = AccountCtx {
        account_id: other_account_id,
        principal_set: AccountPrincipalSet::from_iter_for_test_only([other_principal]),
        auth_method: AccountAuthMethod::NativeToken,
        tenant_id: tenant_id.clone(),
        actor_ref: account_actor_ref(&other_account_id),
        auth_credential_id: None,
        client_kind: "native".to_string(),
    };
    let unowned = account_public_run_publish_handler(
        State(state.clone()),
        Extension(other_ctx),
        HeaderMap::new(),
        AxumPath(submission_id),
        Json(request.clone()),
    )
    .await
    .expect_err("non-owner publication must fail");
    assert_eq!(unowned.0, StatusCode::NOT_FOUND);

    let mut stale_outcome = request.clone();
    stale_outcome.task_success = TaskSuccess::Failure;
    stale_outcome.approval_sha256 = stale_outcome
        .draft
        .approval_sha256(
            stale_outcome.task_success,
            &stale_outcome.contributed_version,
            stale_outcome.expected_publication_version,
        )
        .expect("stale outcome digest");
    let stale_outcome_error = account_public_run_publish_handler(
        State(state.clone()),
        Extension(ctx.clone()),
        HeaderMap::new(),
        AxumPath(submission_id),
        Json(stale_outcome),
    )
    .await
    .expect_err("stale task outcome must fail");
    assert_eq!(stale_outcome_error.0, StatusCode::CONFLICT);

    let mut stale_version = request.clone();
    stale_version.contributed_version = "trace.contribution.v2".to_string();
    stale_version.approval_sha256 = stale_version
        .draft
        .approval_sha256(
            stale_version.task_success,
            &stale_version.contributed_version,
            stale_version.expected_publication_version,
        )
        .expect("stale version digest");
    let stale_version_error = account_public_run_publish_handler(
        State(state.clone()),
        Extension(ctx.clone()),
        HeaderMap::new(),
        AxumPath(submission_id),
        Json(stale_version),
    )
    .await
    .expect_err("stale contributed version must fail");
    assert_eq!(stale_version_error.0, StatusCode::CONFLICT);

    let mut missing_source = request.clone();
    missing_source.draft.source_slug = Some("run-missing-source".to_string());
    missing_source.approval_sha256 = missing_source
        .draft
        .approval_sha256(
            missing_source.task_success,
            &missing_source.contributed_version,
            missing_source.expected_publication_version,
        )
        .expect("missing-source digest");
    let missing_source_error = account_public_run_publish_handler(
        State(state.clone()),
        Extension(ctx.clone()),
        HeaderMap::new(),
        AxumPath(submission_id),
        Json(missing_source),
    )
    .await
    .expect_err("missing source must fail");
    assert_eq!(missing_source_error.0, StatusCode::NOT_FOUND);
    assert!(
        backend
            .get_owned_public_run_state(&tenant_id, account_id.as_uuid(), submission_id)
            .await
            .expect("verify rejected writes")
            .row
            .is_none()
    );

    account_public_run_publish_handler(
        State(state.clone()),
        Extension(ctx.clone()),
        HeaderMap::new(),
        AxumPath(submission_id),
        Json(request.clone()),
    )
    .await
    .expect("publish valid reviewed run");
    let published = backend
        .get_owned_public_run_state(&tenant_id, account_id.as_uuid(), submission_id)
        .await
        .expect("load valid reviewed run")
        .row
        .expect("valid reviewed run exists");
    let mut self_source = request;
    self_source.expected_publication_version = 1;
    self_source.draft.source_slug = Some(published.slug.clone());
    self_source.approval_sha256 = self_source
        .draft
        .approval_sha256(
            self_source.task_success,
            &self_source.contributed_version,
            self_source.expected_publication_version,
        )
        .expect("self-source digest");
    let self_source_error = account_public_run_publish_handler(
        State(state),
        Extension(ctx),
        HeaderMap::new(),
        AxumPath(submission_id),
        Json(self_source),
    )
    .await
    .expect_err("self-sourcing publication must fail");
    assert_eq!(self_source_error.0, StatusCode::BAD_REQUEST);
    let unchanged = backend
        .get_owned_public_run_state(&tenant_id, account_id.as_uuid(), submission_id)
        .await
        .expect("reload valid reviewed run")
        .row
        .expect("valid reviewed run remains");
    assert_eq!(unchanged.version, published.version);
    assert_eq!(unchanged.source_publication_id, None);

    cleanup_pg_trace_tenant(backend.as_ref(), &tenant_id).await;
}

pub(super) struct SelfHostedPrivacyEnvGuard {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl SelfHostedPrivacyEnvGuard {
    pub(super) fn set(base_url: &str) -> Self {
        let vars = [
            ("TRACE_PRIVACY_FILTER_BACKEND", "self-hosted"),
            ("TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL", base_url),
        ];
        let previous = vars
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect();
        for (key, value) in vars {
            // SAFETY: every mutation is serialized by `near_ai_env_lock`.
            unsafe { std::env::set_var(key, value) };
        }
        Self { previous }
    }
}

impl Drop for SelfHostedPrivacyEnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => {
                    // SAFETY: every mutation is serialized by `near_ai_env_lock`.
                    unsafe { std::env::set_var(key, value) };
                }
                None => {
                    // SAFETY: every mutation is serialized by `near_ai_env_lock`.
                    unsafe { std::env::remove_var(key) };
                }
            }
        }
    }
}
