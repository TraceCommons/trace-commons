// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The recheck HTTP contract under the pilot runtime role and tenant RLS.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use trace_commons_protocol::trace_contribution::{
    PiiClassifyPolicy, ReviewCorrectionStatus, ReviewPrivacyRecheckEvidence,
};

const REVIEWED_NOTE: &str = "HTTP reviewed replacement";

struct RecheckBoundary {
    calls: AtomicUsize,
    gate: Option<(Arc<Notify>, Arc<Notify>)>,
}

impl RecheckBoundary {
    fn new(gate: Option<(Arc<Notify>, Arc<Notify>)>) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            gate,
        })
    }
}

#[async_trait::async_trait]
impl PipelinePrivacyBoundary for RecheckBoundary {
    async fn rescrub_deterministic(
        &self,
        _: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        Ok(Vec::new())
    }
    async fn rescrub_classifier(
        &self,
        _: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        Ok(Vec::new())
    }
    async fn recheck_for_review(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<ReviewPrivacyRecheckEvidence> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some((started, resume)) = &self.gate {
            started.notify_one();
            resume.notified().await;
        }
        envelope.replay.replay_notes.push(REVIEWED_NOTE.to_owned());
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        Ok(ReviewPrivacyRecheckEvidence {
            key_diagnostics: None,
            basis: Vec::new(),
            coverage_complete: true,
            correction_status: ReviewCorrectionStatus::Absent,
            residual_scan_complete: true,
            useful_classifier_result: true,
            policy: PiiClassifyPolicy::AllEvents,
        })
    }
    fn production_qualified(&self) -> bool {
        true
    }
    fn classifies_prose_pii(&self) -> bool {
        true
    }
}

struct RecheckFixture {
    base: WithdrawalFixture,
    boundary: Arc<RecheckBoundary>,
    reviewer: String,
    other_reviewer: String,
    scoped_reviewer: String,
    foreign_reviewer: String,
    run: trace_commons_server::versioned_pipeline::PipelineRunRecord,
    lease: serde_json::Value,
}

async fn recheck_fixture(gate: Option<(Arc<Notify>, Arc<Notify>)>) -> Option<RecheckFixture> {
    let boundary = RecheckBoundary::new(gate);
    let injected = boundary.clone();
    let mut base = withdrawal_fixture_with(
        move |runtime, artifacts| {
            let scorer = Arc::new(ReferencePerplexityScorer::new());
            let embedder = Arc::new(ReferenceEmbedder::new());
            let package =
                minimal_storage_rebate_package(scorer.as_ref(), embedder.as_ref()).unwrap();
            let index = IsolatedPipelineIndex::new();
            let registry = SettlementAdapterRegistry::new(vec![RecordingSettlementAdapter::new(
                InstrumentId::new("storage_rebate").unwrap(),
                "http_review_recheck_storage_rebate",
                "none",
            )
                as Arc<dyn SettlementAdapter>])
            .unwrap();
            let object_store_name = ConfiguredTraceArtifactStore::legacy(artifacts.clone())
                .object_store_name()
                .to_string();
            Arc::new(
                PipelineServiceBuilder::new(
                    runtime,
                    artifacts,
                    package,
                    index.clone(),
                    index,
                    registry,
                    PipelineCaps {
                        per_instrument_atomic_units: BTreeMap::from([(
                            "storage_rebate".into(),
                            AtomicUnits::from_raw(u128::MAX),
                        )]),
                    },
                )
                .with_scorer(scorer)
                .with_embedder(embedder)
                .with_object_store_name(object_store_name)
                .with_authority(allow_all_test_authority())
                .with_privacy(injected)
                .with_unqualified_routing(true)
                .build()
                .unwrap(),
            )
        },
        true,
    )
    .await?;
    let suffix = Uuid::new_v4().simple().to_string();
    let reviewer = format!("review-recheck-{suffix}");
    let other_reviewer = format!("review-recheck-other-{suffix}");
    let scoped_reviewer = format!("review-recheck-scoped-{suffix}");
    let foreign_reviewer = format!("review-recheck-foreign-{suffix}");
    let mut tokens = (*base.state.tokens).clone();
    for token in [&reviewer, &other_reviewer, &scoped_reviewer] {
        insert_token(&mut tokens, &base.tenant, token, TokenRole::Reviewer);
    }
    insert_token(
        &mut tokens,
        &format!("tenant-recheck-foreign-{suffix}"),
        &foreign_reviewer,
        TokenRole::Reviewer,
    );
    tokens
        .get_mut(&scoped_reviewer)
        .unwrap()
        .allowed_consent_scopes = BTreeSet::from([ConsentScope::ModelTraining]);
    {
        let state = Arc::make_mut(&mut base.state);
        state.tokens = Arc::new(tokens);
        state.require_db_mirror_writes = true;
        state.pipeline_store = Some(Arc::new(PgPipelineStore::new(base.runtime.clone())));
    }
    let principal = static_token_principal_ref(&base.token);
    let run = parked_pipeline_run(&base.service, &base.tenant, &principal).await;
    let (status, lease) = route_request(
        base.state.clone(),
        "POST",
        &format!("/v1/review/pipeline/runs/{}/claim", run.run_id),
        auth_headers(&reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    Some(RecheckFixture {
        base,
        boundary,
        reviewer,
        other_reviewer,
        scoped_reviewer,
        foreign_reviewer,
        run,
        lease: lease["lease_token"].clone(),
    })
}

fn recheck_uri(fixture: &RecheckFixture) -> String {
    format!("/v1/review/pipeline/runs/{}/recheck", fixture.run.run_id)
}
fn recheck_body(fixture: &RecheckFixture, id: Uuid) -> serde_json::Value {
    serde_json::json!({"lease_token":fixture.lease,"recheck_id":id,
        "expected_source_hash":fixture.run.privacy_pass_content_hash.as_ref().unwrap()})
}
async fn save_recheck(fixture: &RecheckFixture, id: Uuid) -> serde_json::Value {
    let (status, candidate) = route_request(
        fixture.base.state.clone(),
        "POST",
        &recheck_uri(fixture),
        auth_headers(&fixture.reviewer),
        Some(recheck_body(fixture, id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{candidate}");
    candidate
}
async fn candidate_count(fixture: &RecheckFixture) -> i64 {
    let mut client = fixture
        .base
        .runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut client, &fixture.base.tenant).await;
    let count = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2",
            &[&fixture.base.tenant, &fixture.run.run_id],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    count
}
async fn content_response(
    fixture: &RecheckFixture,
    candidate: &serde_json::Value,
    reviewer: &str,
    lease: Option<&str>,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut request = axum::http::Request::builder()
        .uri(format!(
            "/v1/review/pipeline/runs/{}/rechecks/{}/content",
            fixture.run.run_id,
            candidate["candidate_id"].as_str().unwrap()
        ))
        .body(axum::body::Body::empty())
        .unwrap();
    request.headers_mut().extend(auth_headers(reviewer));
    if let Some(lease) = lease {
        request.headers_mut().insert(
            "x-review-lease-token",
            axum::http::HeaderValue::from_str(lease).unwrap(),
        );
    }
    app(fixture.base.state.clone())
        .oneshot(request)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_saves_once_lists_and_reads_the_exact_audited_version() {
    let Some(fixture) = recheck_fixture(None).await else {
        return;
    };
    let (status, queue) = route_request(
        fixture.base.state.clone(),
        "GET",
        "/v1/review/pipeline/quarantine",
        auth_headers(&fixture.reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{queue}");
    assert_eq!(
        queue[0]["review_source_hash"],
        fixture
            .run
            .privacy_pass_content_hash
            .as_ref()
            .unwrap()
            .as_str()
    );
    let id = Uuid::new_v4();
    let candidate = save_recheck(&fixture, id).await;
    assert_eq!(candidate["candidate_id"], id.to_string());
    assert_eq!(candidate["coverage_complete"], true);
    assert_eq!(candidate["residual_pii_risk"], "low");
    assert_ne!(candidate["content_hash"], candidate["source_hash"]);
    assert_eq!(save_recheck(&fixture, id).await, candidate);
    assert_eq!(fixture.boundary.calls.load(Ordering::SeqCst), 1);
    assert_eq!(candidate_count(&fixture).await, 1);
    let (status, versions) = route_request(
        fixture.base.state.clone(),
        "GET",
        &format!("/v1/review/pipeline/runs/{}/rechecks", fixture.run.run_id),
        auth_headers(&fixture.reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert_eq!(versions.as_array().unwrap().len(), 1);
    assert_eq!(versions[0], candidate);
    let (status, queue) = route_request(
        fixture.base.state.clone(),
        "GET",
        "/v1/review/pipeline/quarantine",
        auth_headers(&fixture.reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{queue}");
    assert_eq!(queue[0]["review_source_hash"], candidate["content_hash"]);
    assert_eq!(
        queue[0]["review_candidate"]["candidate_id"],
        candidate["candidate_id"]
    );
    assert_eq!(
        content_response(&fixture, &candidate, &fixture.reviewer, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        content_response(
            &fixture,
            &candidate,
            &fixture.other_reviewer,
            fixture.lease.as_str()
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let response = content_response(
        &fixture,
        &candidate,
        &fixture.reviewer,
        fixture.lease.as_str(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert_eq!(
        sha256_prefixed(std::str::from_utf8(&bytes).unwrap()),
        candidate["content_hash"].as_str().unwrap()
    );
    let envelope: TraceContributionEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert!(
        envelope
            .replay
            .replay_notes
            .iter()
            .any(|note| note == REVIEWED_NOTE)
    );
    let mut client = fixture
        .base
        .runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut client, &fixture.base.tenant).await;
    let reads:i64=tx.query_one("SELECT COUNT(*) FROM trace_audit_events a JOIN pipeline_review_rechecks c ON c.tenant_id=a.tenant_id AND c.object_ref_id=a.object_ref_id WHERE a.tenant_id=$1 AND c.run_id=$2 AND a.action='read'",&[&fixture.base.tenant,&fixture.run.run_id]).await.unwrap().get(0);
    tx.commit().await.unwrap();
    assert_eq!(reads, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_requires_the_latest_exact_candidate_for_assessment() {
    let Some(fixture) = recheck_fixture(None).await else {
        return;
    };
    let candidate = save_recheck(&fixture, Uuid::new_v4()).await;
    let mut body = serde_json::json!({"lease_token":fixture.lease,"recommendation":"approve","reason":"privacy_rechecked","resolved_quarantine_reasons":[fixture.run.admission_reason.clone().unwrap()]});
    let uri = format!("/v1/review/pipeline/runs/{}/assessment", fixture.run.run_id);
    let (status, _) = route_request(
        fixture.base.state.clone(),
        "POST",
        &uri,
        auth_headers(&fixture.reviewer),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    body["candidate_id"] = candidate["candidate_id"].clone();
    body["candidate_content_hash"] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
    let (status, _) = route_request(
        fixture.base.state.clone(),
        "POST",
        &uri,
        auth_headers(&fixture.reviewer),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    body["candidate_content_hash"] = candidate["content_hash"].clone();
    let (status, assessed) = route_request(
        fixture.base.state.clone(),
        "POST",
        &uri,
        auth_headers(&fixture.reviewer),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assessed}");
    assert!(assessed["assessment_id"].is_string());
    let current = fixture
        .base
        .service
        .store()
        .get_run(&fixture.base.tenant, fixture.run.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, PipelineRunState::Pending);
    assert_eq!(
        current.privacy_pass_object_ref_id,
        fixture.run.privacy_pass_object_ref_id
    );
    assert_eq!(
        current.privacy_pass_content_hash,
        fixture.run.privacy_pass_content_hash
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_denies_role_tenant_and_consent_before_classification() {
    let Some(fixture) = recheck_fixture(None).await else {
        return;
    };
    for (token, expected) in [
        (&fixture.base.token, StatusCode::FORBIDDEN),
        (&fixture.foreign_reviewer, StatusCode::NOT_FOUND),
        (&fixture.scoped_reviewer, StatusCode::FORBIDDEN),
    ] {
        let (status, body) = route_request(
            fixture.base.state.clone(),
            "POST",
            &recheck_uri(&fixture),
            auth_headers(token),
            Some(recheck_body(&fixture, Uuid::new_v4())),
        )
        .await;
        assert_eq!(status, expected, "{body}");
    }
    assert_eq!(fixture.boundary.calls.load(Ordering::SeqCst), 0);
    assert_eq!(candidate_count(&fixture).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_rejects_oversized_unknown_and_stale_requests_before_classification()
{
    let Some(fixture) = recheck_fixture(None).await else {
        return;
    };
    let mut unknown = recheck_body(&fixture, Uuid::new_v4());
    unknown["content"] = serde_json::json!("caller supplied replacement");
    let (status, _) = route_request(
        fixture.base.state.clone(),
        "POST",
        &recheck_uri(&fixture),
        auth_headers(&fixture.reviewer),
        Some(unknown),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut oversized = recheck_body(&fixture, Uuid::new_v4());
    oversized["padding"] = serde_json::json!("x".repeat((1 << 20) + 1));
    let (status, _) = route_request(
        fixture.base.state.clone(),
        "POST",
        &recheck_uri(&fixture),
        auth_headers(&fixture.reviewer),
        Some(oversized),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let mut stale = recheck_body(&fixture, Uuid::new_v4());
    stale["expected_source_hash"] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
    let (status, _) = route_request(
        fixture.base.state.clone(),
        "POST",
        &recheck_uri(&fixture),
        auth_headers(&fixture.reviewer),
        Some(stale),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let mut client = fixture
        .base
        .runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut client, &fixture.base.tenant).await;
    tx.execute("UPDATE pipeline_review_claims SET lease_expires_at=NOW()-INTERVAL '1 second' WHERE tenant_id=$1 AND run_id=$2",&[&fixture.base.tenant,&fixture.run.run_id]).await.unwrap();
    tx.commit().await.unwrap();
    drop(client);
    let (status, _) = route_request(
        fixture.base.state.clone(),
        "POST",
        &recheck_uri(&fixture),
        auth_headers(&fixture.reviewer),
        Some(recheck_body(&fixture, Uuid::new_v4())),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(fixture.boundary.calls.load(Ordering::SeqCst), 0);
    assert_eq!(candidate_count(&fixture).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_content_read_fails_closed_when_audit_append_fails() {
    let Some(fixture) = recheck_fixture(None).await else {
        return;
    };
    let candidate = save_recheck(&fixture, Uuid::new_v4()).await;
    let log = audit_events_path(&fixture.base.state.root, &fixture.base.tenant);
    if log.is_file() {
        std::fs::remove_file(&log).unwrap();
    }
    std::fs::create_dir_all(log).unwrap();
    let response = content_response(
        &fixture,
        &candidate,
        &fixture.reviewer,
        fixture.lease.as_str(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(REVIEWED_NOTE));
    assert_eq!(candidate_count(&fixture).await, 1);
}

async fn blocked_recheck(
    fixture: &RecheckFixture,
) -> tokio::task::JoinHandle<(StatusCode, serde_json::Value)> {
    let state = fixture.base.state.clone();
    let token = fixture.reviewer.clone();
    let uri = recheck_uri(fixture);
    let body = recheck_body(fixture, Uuid::new_v4());
    tokio::spawn(async move {
        route_request(state, "POST", &uri, auth_headers(&token), Some(body)).await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_withdrawal_during_classification_cannot_save_a_candidate() {
    let started = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let Some(fixture) = recheck_fixture(Some((started.clone(), resume.clone()))).await else {
        return;
    };
    let worker = blocked_recheck(&fixture).await;
    tokio::time::timeout(std::time::Duration::from_secs(20), started.notified())
        .await
        .expect("classifier started");
    let mut client = fixture
        .base
        .runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut client, &fixture.base.tenant).await;
    tx.execute("INSERT INTO trace_withdrawals(tenant_id,submission_id,withdrawn_at,prior_status,distribution_reach) VALUES($1,$2,NOW(),'quarantined','not_distributed')",&[&fixture.base.tenant,&fixture.run.submission_id]).await.unwrap();
    tx.commit().await.unwrap();
    drop(client);
    resume.notify_one();
    let (status, body) = worker.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(candidate_count(&fixture).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_consent_narrowed_during_classification_cannot_save_a_candidate() {
    let started = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let Some(mut fixture) = recheck_fixture(Some((started.clone(), resume.clone()))).await else {
        return;
    };
    Arc::make_mut(&mut fixture.base.state).db_tenant_policy_reads = true;
    let write = |scopes: Vec<String>, uses: Vec<String>| StorageTraceTenantPolicyWrite {
        tenant_id: fixture.base.tenant.clone(),
        policy_version: "recheck-test-policy".into(),
        allowed_consent_scopes: scopes,
        allowed_uses: uses,
        updated_by_principal_ref: principal_storage_ref("recheck-policy-admin"),
    };
    fixture
        .base
        .runtime
        .upsert_trace_tenant_policy(write(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let worker = blocked_recheck(&fixture).await;
    tokio::time::timeout(std::time::Duration::from_secs(20), started.notified())
        .await
        .expect("classifier started");
    fixture
        .base
        .runtime
        .upsert_trace_tenant_policy(write(
            vec![serde_storage_string(&ConsentScope::ModelTraining).unwrap()],
            vec![serde_storage_string(&TraceAllowedUse::ModelTraining).unwrap()],
        ))
        .await
        .unwrap();
    resume.notify_one();
    let (status, body) = worker.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(candidate_count(&fixture).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_recheck_http_access_grant_revoked_during_classification_cannot_save_a_candidate()
{
    let started = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let Some(mut fixture) = recheck_fixture(Some((started.clone(), resume.clone()))).await else {
        return;
    };
    // Provision an actual DB grant, then require it on the same request that
    // will pause in the classifier. The claim was already made by this exact
    // reviewer; only its authority, not its claim or consent, changes below.
    Arc::make_mut(&mut fixture.base.state).require_tenant_access_grants = true;
    let mut grant = StorageTraceTenantAccessGrantWrite {
        tenant_id: fixture.base.tenant.clone(),
        grant_id: Uuid::new_v4(),
        principal_ref: static_token_principal_ref(&fixture.reviewer),
        role: StorageTraceTenantAccessGrantRole::Reviewer,
        status: StorageTraceTenantAccessGrantStatus::Active,
        allowed_consent_scopes: Vec::new(),
        allowed_uses: Vec::new(),
        issuer: None,
        audience: None,
        subject: None,
        issued_at: Utc::now(),
        expires_at: None,
        revoked_at: None,
        created_by_principal_ref: Some(principal_storage_ref("recheck-grant-admin")),
        revoked_by_principal_ref: None,
        reason: Some("recheck_regression".to_owned()),
        metadata: BTreeMap::new(),
    };
    fixture
        .base
        .owner
        .upsert_trace_tenant_access_grant(grant.clone())
        .await
        .unwrap();
    let worker = blocked_recheck(&fixture).await;
    tokio::time::timeout(std::time::Duration::from_secs(20), started.notified())
        .await
        .expect("the active grant permits the classifier to start");
    grant.status = StorageTraceTenantAccessGrantStatus::Revoked;
    grant.revoked_at = Some(Utc::now());
    grant.revoked_by_principal_ref = Some(principal_storage_ref("recheck-grant-admin"));
    fixture
        .base
        .owner
        .upsert_trace_tenant_access_grant(grant)
        .await
        .unwrap();
    resume.notify_one();

    let (status, body) = worker.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "active tenant access grant required");
    assert_eq!(fixture.boundary.calls.load(Ordering::SeqCst), 1);
    assert_eq!(candidate_count(&fixture).await, 0);
}
