// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::public_run_tests::SelfHostedPrivacyEnvGuard;
use super::*;

use axum::body::Body;
use tower::ServiceExt;
use trace_commons_protocol::public_run::{
    PublicRunDraft, PublicRunEvidenceDraft, PublicRunPublishRequest, PublicRunReusePermission,
};
use trace_commons_protocol::trace_contribution::TraceContributionEnvelope;
use trace_commons_server::account_session::{
    AccountAuthMethod, AccountCtx, AccountId, AccountPrincipalSet, account_actor_ref,
};
use trace_commons_server::db::PublicRunWrite;
use wiremock::matchers::{method, path};

struct PublicRunRouteFixture {
    _privacy_env: SelfHostedPrivacyEnvGuard,
    _env_lock: tokio::sync::MutexGuard<'static, ()>,
    _privacy_server: wiremock::MockServer,
    _temp_directory: tempfile::TempDir,
    backend: Arc<PgBackend>,
    tenant_id: String,
    source_submission: Uuid,
    variation_submission: Uuid,
    source_publication_id: Uuid,
    source_slug: String,
    variation_slug: String,
    state: Arc<AppState>,
    ctx: AccountCtx,
    variation_envelope: TraceContributionEnvelope,
}

impl PublicRunRouteFixture {
    async fn new() -> Option<Self> {
        let env_lock = near_ai_env_lock().lock().await;
        let privacy_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"spans": []}]})),
            )
            .mount(&privacy_server)
            .await;
        let privacy_env = SelfHostedPrivacyEnvGuard::set(&privacy_server.uri());

        let backend = postgres_backend_for_ingest_test().await?;
        let unique = Uuid::new_v4().simple().to_string();
        let tenant_id = format!("public-run-route-{unique}");
        let principal_ref = format!("principal:public-run-route-{unique}");
        let source_submission =
            insert_account_test_submission(backend.as_ref(), &tenant_id, &principal_ref).await;
        let variation_submission =
            insert_account_test_submission(backend.as_ref(), &tenant_id, &principal_ref).await;
        let account_id = backend
            .create_or_reuse_account(&tenant_id, &principal_ref)
            .await
            .expect("create public-run route account");
        let source_publication_id = Uuid::new_v4();
        let source_slug = format!("run-source-{}", &unique[..12]);
        let variation_slug = format!("run-variation-{}", &unique[..12]);
        let write =
            |publication_id, submission_id, slug: String, source_publication_id| PublicRunWrite {
                tenant_id: tenant_id.clone(),
                publication_id,
                account_id,
                submission_id,
                slug,
                title: "Synthetic reviewed workflow".to_string(),
                outcome_summary: "The synthetic task completed.".to_string(),
                correction_excerpt: Some("Use the bounded retry.".to_string()),
                workflow: "Run the synthetic step, then verify its result.".to_string(),
                reuse_permission: PublicRunReusePermission::CcBy40,
                evidence: vec![PublicRunEvidenceDraft {
                    event_id: Uuid::new_v4(),
                    excerpt: "The synthetic result was observed.".to_string(),
                }],
                task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
                contributed_version: "trace.contribution.v1".to_string(),
                approval_sha256: format!("sha256:{}", "a".repeat(64)),
                source_publication_id,
                expected_publication_version: 0,
            };
        backend
            .upsert_public_run(write(
                source_publication_id,
                source_submission,
                source_slug.clone(),
                None,
            ))
            .await
            .expect("publish route source");
        backend
            .upsert_public_run(write(
                Uuid::new_v4(),
                variation_submission,
                variation_slug.clone(),
                Some(source_publication_id),
            ))
            .await
            .expect("publish route variation");

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
        let variation_envelope: TraceContributionEnvelope = serde_json::from_slice(
            &write_redacted_envelope_to_disk(state.as_ref(), &tenant_id, variation_submission)
                .await,
        )
        .expect("decode variation envelope");
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

        Some(Self {
            _privacy_env: privacy_env,
            _env_lock: env_lock,
            _privacy_server: privacy_server,
            _temp_directory: temp,
            backend,
            tenant_id,
            source_submission,
            variation_submission,
            source_publication_id,
            source_slug,
            variation_slug,
            state,
            ctx,
            variation_envelope,
        })
    }

    async fn public_response(&self, slug: &str) -> axum::response::Response {
        app(self.state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(format!("/v1/community/runs/{slug}"))
                    .body(Body::empty())
                    .expect("public request builds"),
            )
            .await
            .expect("public route response")
    }

    async fn owner_response(&self, submission_id: Uuid) -> axum::response::Response {
        account_public_run_handler(
            State(self.state.clone()),
            Extension(self.ctx.clone()),
            AxumPath(submission_id),
        )
        .await
        .expect("owner publication response")
    }

    async fn unpublish(&self, submission_id: Uuid) -> serde_json::Value {
        response_json(
            account_public_run_unpublish_handler(
                State(self.state.clone()),
                Extension(self.ctx.clone()),
                HeaderMap::new(),
                AxumPath(submission_id),
            )
            .await
            .expect("unpublish response"),
        )
        .await
    }

    fn variation_request(
        &self,
        expected_publication_version: u32,
        source_slug: Option<String>,
    ) -> PublicRunPublishRequest {
        let event = self
            .variation_envelope
            .events
            .first()
            .expect("variation envelope has evidence");
        let draft = PublicRunDraft {
            title: "Edited variation workflow".to_string(),
            outcome_summary: "The edited variation completed.".to_string(),
            correction_excerpt: self.variation_envelope.outcome.human_correction.clone(),
            workflow: "Run the edited variation and verify its result.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: event.event_id,
                excerpt: event
                    .redacted_content
                    .clone()
                    .expect("variation evidence is redacted"),
            }],
            source_slug,
        };
        PublicRunPublishRequest {
            approval_sha256: draft
                .approval_sha256(
                    self.variation_envelope.outcome.task_success,
                    &self.variation_envelope.schema_version,
                    expected_publication_version,
                )
                .expect("variation approval digest"),
            draft,
            task_success: self.variation_envelope.outcome.task_success,
            contributed_version: self.variation_envelope.schema_version.clone(),
            expected_publication_version,
        }
    }

    async fn publish_variation(
        &self,
        request: PublicRunPublishRequest,
    ) -> ApiResult<axum::response::Response> {
        account_public_run_publish_handler(
            State(self.state.clone()),
            Extension(self.ctx.clone()),
            HeaderMap::new(),
            AxumPath(self.variation_submission),
            Json(request),
        )
        .await
    }

    async fn cleanup(&self) {
        cleanup_pg_trace_tenant(self.backend.as_ref(), &self.tenant_id).await;
    }
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read JSON response"),
    )
    .expect("response JSON")
}

#[tokio::test]
async fn public_projection_omits_private_identifiers_and_owner_state_is_private() {
    let Some(fixture) = PublicRunRouteFixture::new().await else {
        return;
    };
    let response = fixture.public_response(&fixture.source_slug).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(axum::http::header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("public, max-age=30"))
    );
    let public_body = response_json(response).await;
    assert_eq!(public_body["slug"], fixture.source_slug);
    assert_eq!(public_body["variations"][0]["slug"], fixture.variation_slug);
    for forbidden in [
        "tenant_id",
        "account_id",
        "submission_id",
        "publication_id",
        "event_id",
        "approval_sha256",
    ] {
        assert!(
            !public_body.to_string().contains(forbidden),
            "public response exposed {forbidden}: {public_body}"
        );
    }

    let owner_response = fixture.owner_response(fixture.source_submission).await;
    assert_eq!(
        owner_response
            .headers()
            .get(axum::http::header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
    let owner_body = response_json(owner_response).await;
    assert_eq!(owner_body["publication"]["slug"], fixture.source_slug);
    assert_eq!(owner_body["expected_publication_version"], 1);
    fixture.cleanup().await;
}

#[tokio::test]
async fn source_withdrawal_is_idempotent_and_retains_variation_provenance() {
    let Some(fixture) = PublicRunRouteFixture::new().await else {
        return;
    };
    let first = fixture.unpublish(fixture.source_submission).await;
    assert_eq!(first["unpublished"], true);
    let withdrawn_owner =
        response_json(fixture.owner_response(fixture.source_submission).await).await;
    assert!(withdrawn_owner["publication"].is_null());
    assert_eq!(withdrawn_owner["expected_publication_version"], 2);
    let repeated = fixture.unpublish(fixture.source_submission).await;
    assert_eq!(repeated["unpublished"], false);
    assert_eq!(repeated["expected_publication_version"], 2);

    let variation_owner =
        response_json(fixture.owner_response(fixture.variation_submission).await).await;
    assert_eq!(variation_owner["retained_source_slug"], fixture.source_slug);
    let variation_state = fixture
        .backend
        .get_owned_public_run_state(
            &fixture.tenant_id,
            fixture.ctx.account_id.as_uuid(),
            fixture.variation_submission,
        )
        .await
        .expect("read variation after source withdrawal");
    assert_eq!(
        variation_state
            .row
            .expect("variation remains owned")
            .source_publication_id,
        Some(fixture.source_publication_id)
    );
    assert!(
        variation_state
            .page
            .expect("variation remains public")
            .source_unavailable
    );
    assert_eq!(
        fixture.public_response(&fixture.source_slug).await.status(),
        StatusCode::NOT_FOUND
    );
    let variation_public = fixture.public_response(&fixture.variation_slug).await;
    assert_eq!(variation_public.status(), StatusCode::OK);
    assert_eq!(
        response_json(variation_public).await["source_unavailable"],
        true
    );
    fixture.cleanup().await;
}

#[tokio::test]
async fn republish_requires_current_version_and_explicit_source_review() {
    let Some(fixture) = PublicRunRouteFixture::new().await else {
        return;
    };
    fixture.unpublish(fixture.source_submission).await;
    let before = fixture
        .backend
        .get_owned_public_run_state(
            &fixture.tenant_id,
            fixture.ctx.account_id.as_uuid(),
            fixture.variation_submission,
        )
        .await
        .expect("read variation before edit")
        .row
        .expect("variation remains owned");
    let reviewed = fixture.variation_request(1, Some(fixture.source_slug.clone()));
    let edited = fixture
        .publish_variation(reviewed.clone())
        .await
        .expect("edit variation with unavailable source");
    let edited_body = response_json(edited).await;
    assert_eq!(edited_body["version"], before.version + 1);
    assert_eq!(edited_body["source_unavailable"], true);
    let after = fixture
        .backend
        .get_owned_public_run_state(
            &fixture.tenant_id,
            fixture.ctx.account_id.as_uuid(),
            fixture.variation_submission,
        )
        .await
        .expect("reload edited variation")
        .row
        .expect("edited variation remains owned");
    assert_eq!(
        after.source_publication_id,
        Some(fixture.source_publication_id)
    );

    fixture.unpublish(fixture.variation_submission).await;
    let stale = fixture.variation_request(2, Some(fixture.source_slug.clone()));
    let stale_error = fixture
        .publish_variation(stale)
        .await
        .expect_err("pre-withdrawal approval must not republish");
    assert_eq!(stale_error.0, StatusCode::CONFLICT);

    let fresh = fixture.variation_request(3, Some(fixture.source_slug.clone()));
    let republished = fixture
        .publish_variation(fresh)
        .await
        .expect("fresh approval republishes variation");
    let republished_body = response_json(republished).await;
    assert_eq!(republished_body["version"], 4);
    assert_eq!(republished_body["source_unavailable"], true);

    let clear_source = fixture.variation_request(4, None);
    let source_removed = fixture
        .publish_variation(clear_source)
        .await
        .expect("reviewed source removal updates variation");
    let source_removed_body = response_json(source_removed).await;
    assert_eq!(source_removed_body["version"], 5);
    assert!(source_removed_body["source"].is_null());
    assert!(source_removed_body["source_unavailable"].is_null());
    let source_removed_row = fixture
        .backend
        .get_owned_public_run_state(
            &fixture.tenant_id,
            fixture.ctx.account_id.as_uuid(),
            fixture.variation_submission,
        )
        .await
        .expect("reload source-removed variation")
        .row
        .expect("source-removed variation remains owned");
    assert_eq!(source_removed_row.source_publication_id, None);
    fixture.cleanup().await;
}
