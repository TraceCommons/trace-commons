// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Corrected review versions retain the original pass and existing credit identity.

use super::*;
use std::sync::atomic::AtomicBool;
use trace_commons_protocol::trace_contribution::{
    PiiClassifyPolicy, ReviewCorrectionStatus, ReviewPrivacyRecheckEvidence,
};

const PRIVATE_MARKER: &str = "RECHECK_PRIVATE_MARKER";

struct RecheckBoundary {
    calls: AtomicUsize,
    complete: AtomicBool,
    survivor: AtomicBool,
}

impl RecheckBoundary {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            complete: AtomicBool::new(true),
            survivor: AtomicBool::new(false),
        })
    }
}

#[async_trait::async_trait]
impl PipelinePrivacyBoundary for RecheckBoundary {
    async fn rescrub_deterministic(
        &self,
        _envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        Ok(Vec::new())
    }

    async fn rescrub_classifier(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        // The initial stored pass holds the marker for human review.
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
        Ok(vec![ResidualRiskCondition::FoundAndRemoved])
    }

    async fn recheck_for_review(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<ReviewPrivacyRecheckEvidence> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        for event in &mut envelope.events {
            if let Some(text) = &mut event.redacted_content {
                *text = text.replace(PRIVATE_MARKER, "[reviewed]");
            }
        }
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
        *envelope
            .privacy
            .redaction_counts
            .entry("privacy_filter:private_person".into())
            .or_default() += 1;
        let mut basis = vec![ResidualRiskCondition::FoundAndRemoved];
        if self.survivor.load(Ordering::SeqCst) {
            basis.push(ResidualRiskCondition::ResidualSurvivor);
        }
        Ok(ReviewPrivacyRecheckEvidence {
            basis,
            coverage_complete: self.complete.load(Ordering::SeqCst),
            correction_status: ReviewCorrectionStatus::Absent,
            residual_scan_complete: true,
            useful_classifier_result: true,
            policy: PiiClassifyPolicy::AllEvents,
        })
    }
}

struct RecheckFixture {
    service: Arc<PipelineService>,
    index: Arc<IsolatedPipelineIndex>,
    adapters: Vec<Arc<RecordingSettlementAdapter>>,
    boundary: Arc<RecheckBoundary>,
    tenant: String,
    held: PipelineRunRecord,
    claim: PipelineReviewClaim,
}

async fn recheck_fixture(backend: &Arc<PgBackend>, dir: &tempfile::TempDir) -> RecheckFixture {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let embedder = Arc::new(ReferenceEmbedder::new());
    let package = MinimalPolicyBundle::minimal_package(
        &scored_config(true),
        scorer.as_ref(),
        embedder.as_ref(),
    )
    .unwrap();
    let index = IsolatedPipelineIndex::new();
    let adapters = vec![
        RecordingSettlementAdapter::new(
            InstrumentId::new("storage_rebate").unwrap(),
            "recording_storage_rebate_test_only",
            "none",
        ),
        RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "recording_trace_credit_test_only",
            "none",
        ),
    ];
    let registry = SettlementAdapterRegistry::new(
        adapters
            .iter()
            .cloned()
            .map(|adapter| adapter as Arc<dyn SettlementAdapter>)
            .collect(),
    )
    .unwrap();
    let boundary = RecheckBoundary::new();
    let service = Arc::new(
        PipelineServiceBuilder::new(
            backend.clone(),
            artifact_store(dir),
            package,
            index.clone(),
            index.clone(),
            registry,
            uncapped_caps(&["storage_rebate", InstrumentId::trace_credit().as_str()]),
        )
        .with_scorer(scorer)
        .with_embedder(embedder)
        .with_authority(allow_all_authority())
        .with_privacy(boundary.clone())
        .with_unqualified_routing(true)
        .build()
        .unwrap(),
    );
    let (tenant, created) =
        submit_with_markers(&service, "review-recheck", &[PRIVATE_MARKER]).await;
    let held = service
        .process_run(&tenant, created.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_held_by_the_pass(&service, &tenant, &held).await;
    let claim = claim_for_review(&service, &tenant, held.run_id, 'e').await;
    RecheckFixture {
        service,
        index,
        adapters,
        boundary,
        tenant,
        held,
        claim,
    }
}

async fn candidate(fixture: &RecheckFixture, source_hash: &str) -> PipelineReviewRecheck {
    let prepared = fixture
        .service
        .prepare_review_recheck(&fixture.claim, uuid::Uuid::new_v4(), source_hash)
        .await
        .unwrap();
    fixture
        .service
        .finish_review_recheck(prepared)
        .await
        .unwrap()
}

fn approval_reason() -> ReasonCode {
    ReasonCode::new("reviewed_corrected_version").unwrap()
}

fn resolved() -> Vec<ReasonCode> {
    vec![ReasonCode::new(PIPELINE_PRIVACY_PASS_REVIEW_REQUIRED_LABEL).unwrap()]
}

async fn approve_candidate(fixture: &RecheckFixture, candidate: &PipelineReviewRecheck) {
    fixture
        .service
        .store()
        .record_review_assessment_for_hold_with_recheck(
            &fixture.claim,
            ReviewRecommendation::Approve,
            approval_reason(),
            resolved(),
            Some(candidate.binding()),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn corrected_version_is_the_approved_indexed_and_once_credited_content() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let original_hash = fixture.held.privacy_pass_content_hash.clone().unwrap();
    let original_ref = fixture.held.privacy_pass_object_ref_id;
    let candidate = candidate(&fixture, &original_hash).await;
    assert_eq!(candidate.source_content_hash, original_hash);
    assert_eq!(candidate.source_object_ref_id, original_ref.unwrap());
    assert_ne!(candidate.content_hash, original_hash);
    let bytes = fixture
        .service
        .load_review_recheck_content(&fixture.claim, candidate.recheck_id)
        .await
        .unwrap();
    assert_eq!(dependency_content_hash(&bytes), candidate.content_hash);
    assert!(!String::from_utf8(bytes).unwrap().contains(PRIVATE_MARKER));
    approve_candidate(&fixture, &candidate).await;

    let reviewed = fixture
        .service
        .process_run(&fixture.tenant, fixture.held.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reviewed.next_phase, Some(Phase::Score));
    assert_eq!(reviewed.privacy_pass_object_ref_id, original_ref);
    assert_eq!(
        reviewed.privacy_pass_content_hash,
        Some(original_hash.clone())
    );
    assert_eq!(
        reviewed.approved_content_hash.as_deref(),
        Some(candidate.content_hash.as_str())
    );
    let approved = fixture
        .service
        .load_approved_bytes(&reviewed)
        .await
        .unwrap();
    assert!(
        !String::from_utf8(approved)
            .unwrap()
            .contains(PRIVATE_MARKER)
    );
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    let derived = tx
        .query_one(
            "SELECT input_object_ref_id,input_hash FROM trace_derived_records WHERE tenant_id=$1 AND derived_id=$2",
            &[&fixture.tenant, &reviewed.approved_revision_id.unwrap()],
        )
        .await
        .unwrap();
    assert_eq!(derived.get::<_, uuid::Uuid>(0), candidate.object_ref_id);
    assert_eq!(derived.get::<_, String>(1), candidate.content_hash);
    tx.commit().await.unwrap();
    drop(client);

    let scored = fixture
        .service
        .process_run(&fixture.tenant, reviewed.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    let settled = fixture
        .service
        .process_run(&fixture.tenant, reviewed.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.index_write_state, "complete");
    assert_eq!(
        fixture.index.revision_count(
            &pipeline_tenant_storage_ref(&fixture.tenant),
            MINIMAL_INDEX_ID
        ),
        1
    );
    assert_credit_settled_once(
        &backend,
        &fixture.service,
        &fixture.tenant,
        reviewed.run_id,
        "corrected version",
    )
    .await;
    let writes = fixture.index.writer_calls();
    for _ in 0..2 {
        assert!(
            fixture
                .service
                .process_run(&fixture.tenant, reviewed.run_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(fixture.index.writer_calls(), writes);
    for adapter in &fixture.adapters {
        assert!(
            adapter.requests().len() <= 1,
            "a settlement operation was repeated"
        );
    }
    assert_credit_settled_once(
        &backend,
        &fixture.service,
        &fixture.tenant,
        reviewed.run_id,
        "after worker retry",
    )
    .await;
    let after = fixture
        .service
        .store()
        .get_run(&fixture.tenant, reviewed.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.privacy_pass_content_hash, Some(original_hash));
}

#[tokio::test]
async fn recheck_chain_is_monotone_idempotent_and_refuses_stale_source() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let original_hash = fixture.held.privacy_pass_content_hash.clone().unwrap();
    let id = uuid::Uuid::new_v4();
    let first = fixture
        .service
        .finish_review_recheck(
            fixture
                .service
                .prepare_review_recheck(&fixture.claim, id, &original_hash)
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    let repeated = fixture
        .service
        .finish_review_recheck(
            fixture
                .service
                .prepare_review_recheck(&fixture.claim, id, &original_hash)
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first, repeated);
    assert_eq!(fixture.boundary.calls.load(Ordering::SeqCst), 1);
    assert!(
        fixture
            .service
            .prepare_review_recheck(&fixture.claim, uuid::Uuid::new_v4(), &original_hash)
            .await
            .is_err()
    );
    let second = candidate(&fixture, &first.content_hash).await;
    assert_eq!(second.ordinal, 2);
    assert_eq!(second.parent_recheck_id, Some(first.recheck_id));
    assert_eq!(second.source_object_ref_id, first.object_ref_id);
    assert_eq!(second.source_content_hash, first.content_hash);
    let history = fixture
        .service
        .store()
        .list_review_rechecks(&fixture.tenant, fixture.held.run_id, 10)
        .await
        .unwrap();
    assert_eq!(history, vec![second, first]);
    let stored = fixture
        .service
        .store()
        .get_run(&fixture.tenant, fixture.held.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.privacy_pass_content_hash, Some(original_hash));
}

#[tokio::test]
async fn assessment_requires_the_latest_exact_candidate_and_evidence_hash_binds_it() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let first = candidate(
        &fixture,
        fixture.held.privacy_pass_content_hash.as_deref().unwrap(),
    )
    .await;
    let second = candidate(&fixture, &first.content_hash).await;
    let store = fixture.service.store();
    for binding in [
        None,
        Some(first.binding()),
        Some(PipelineReviewRecheckBinding {
            recheck_id: second.recheck_id,
            content_hash: dependency_content_hash(b"wrong bytes"),
        }),
        Some(PipelineReviewRecheckBinding {
            recheck_id: uuid::Uuid::new_v4(),
            content_hash: second.content_hash.clone(),
        }),
    ] {
        let error = store
            .record_review_assessment_for_hold_with_recheck(
                &fixture.claim,
                ReviewRecommendation::Approve,
                approval_reason(),
                resolved(),
                binding,
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(PIPELINE_REVIEW_RECHECK_BINDING_REQUIRED_LABEL),
            "{error:?}"
        );
    }
    let mut wrong_tenant = fixture.claim.clone();
    wrong_tenant.tenant_id = format!("other-{}", uuid::Uuid::new_v4());
    assert!(
        fixture
            .service
            .load_review_recheck_content(&wrong_tenant, second.recheck_id)
            .await
            .is_err()
    );
    let (assessment, at_score) = store
        .record_review_assessment_for_hold_with_recheck(
            &fixture.claim,
            ReviewRecommendation::Approve,
            approval_reason(),
            resolved(),
            Some(second.binding()),
        )
        .await
        .unwrap();
    assert!(!at_score);
    let evidence = serde_json::json!({
        "schema": "trace_commons.pipeline_review_assessment.v2",
        "run_id": fixture.held.run_id,
        "recommendation": "approve",
        "reason": approval_reason().as_str(),
        "resolved_quarantine_reasons": resolved(),
        "recheck_id": second.recheck_id,
        "recheck_content_hash": second.content_hash,
        "recheck_source_hash": second.source_content_hash,
    });
    assert_eq!(
        assessment.evidence_hash,
        dependency_content_hash(&serde_json::to_vec(&evidence).unwrap())
    );
    let mut other_evidence = evidence;
    other_evidence["recheck_id"] = serde_json::json!(first.recheck_id);
    assert_ne!(
        assessment.evidence_hash,
        dependency_content_hash(&serde_json::to_vec(&other_evidence).unwrap())
    );
    let loaded = store
        .load_review_assessment(&fixture.tenant, fixture.held.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.assessment.evidence_hash, assessment.evidence_hash);
    assert_eq!(loaded.recheck.unwrap().recheck_id, second.recheck_id);
    assert!(
        store
            .record_review_assessment_for_hold_with_recheck(
                &fixture.claim,
                ReviewRecommendation::Approve,
                approval_reason(),
                resolved(),
                Some(second.binding())
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn incomplete_or_surviving_recheck_evidence_cannot_approve() {
    let backend = runtime_backend_or_skip!(4);
    for survivor in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let fixture = recheck_fixture(&backend, &dir).await;
        fixture.boundary.complete.store(survivor, Ordering::SeqCst);
        fixture.boundary.survivor.store(survivor, Ordering::SeqCst);
        let checked = candidate(
            &fixture,
            fixture.held.privacy_pass_content_hash.as_deref().unwrap(),
        )
        .await;
        assert!(!checked.approval_evidence_complete());
        let error = fixture
            .service
            .store()
            .record_review_assessment_for_hold_with_recheck(
                &fixture.claim,
                ReviewRecommendation::Approve,
                approval_reason(),
                resolved(),
                Some(checked.binding()),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(PIPELINE_REVIEW_RECHECK_UNRESOLVED_LABEL),
            "{error:?}"
        );
        let held = fixture
            .service
            .store()
            .get_run(&fixture.tenant, fixture.held.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(held.state, PipelineRunState::AwaitingReview);
        assert!(
            fixture
                .service
                .store()
                .list_settlements(&fixture.tenant, held.run_id)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn expired_or_replaced_claim_cannot_prepare_or_finish_a_candidate() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let prepared = fixture
        .service
        .prepare_review_recheck(
            &fixture.claim,
            uuid::Uuid::new_v4(),
            fixture.held.privacy_pass_content_hash.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    tx.execute("UPDATE pipeline_review_claims SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND run_id=$2", &[&fixture.tenant,&fixture.held.run_id]).await.unwrap();
    tx.commit().await.unwrap();
    drop(client);
    let replacement =
        claim_for_review(&fixture.service, &fixture.tenant, fixture.held.run_id, 'f').await;
    assert_ne!(replacement.lease_token, fixture.claim.lease_token);
    assert!(
        fixture
            .service
            .finish_review_recheck(prepared)
            .await
            .is_err()
    );
    assert!(
        fixture
            .service
            .prepare_review_recheck(
                &fixture.claim,
                uuid::Uuid::new_v4(),
                fixture.held.privacy_pass_content_hash.as_deref().unwrap()
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .service
            .store()
            .list_review_rechecks(&fixture.tenant, fixture.held.run_id, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn withdrawal_between_preparation_and_finish_cannot_resurrect_content() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let prepared = fixture
        .service
        .prepare_review_recheck(
            &fixture.claim,
            uuid::Uuid::new_v4(),
            fixture.held.privacy_pass_content_hash.as_deref().unwrap(),
        )
        .await
        .unwrap();
    withdraw(
        &fixture.service,
        &fixture.tenant,
        fixture.held.submission_id,
    )
    .await;
    assert!(
        fixture
            .service
            .finish_review_recheck(prepared)
            .await
            .is_err()
    );
    assert!(
        fixture
            .service
            .store()
            .list_review_rechecks(&fixture.tenant, fixture.held.run_id, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .service
            .store()
            .list_settlements(&fixture.tenant, fixture.held.run_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        count_credit_ledger_rows_for_run(&backend, &fixture.tenant, fixture.held.run_id).await,
        0
    );
}

#[tokio::test]
async fn discarded_candidate_is_swept_but_committed_candidate_survives_the_sweep() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let source = fixture.held.privacy_pass_content_hash.as_deref().unwrap();
    let discarded_id = uuid::Uuid::new_v4();
    let prepared = fixture
        .service
        .prepare_review_recheck(&fixture.claim, discarded_id, source)
        .await
        .unwrap();
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    let key:String=tx.query_one("SELECT object_key FROM pipeline_attempt_artifacts WHERE tenant_id=$1 AND run_id=$2 AND lease_token=$3 AND artifact='review-recheck'", &[&fixture.tenant,&fixture.held.run_id,&discarded_id]).await.unwrap().get(0);
    tx.commit().await.unwrap();
    drop(client);
    assert!(pass_object_present(
        &artifact_store(&dir),
        &fixture.tenant,
        &key
    ));
    fixture
        .service
        .discard_review_recheck(&prepared)
        .await
        .unwrap();
    assert!(!pass_object_present(
        &artifact_store(&dir),
        &fixture.tenant,
        &key
    ));
    backdate_attempt_artifacts(&fixture.tenant, fixture.held.run_id, Some(discarded_id)).await;
    assert_eq!(
        fixture
            .service
            .sweep_attempt_artifacts(&fixture.tenant, 10)
            .await
            .unwrap(),
        1
    );
    let committed = candidate(&fixture, source).await;
    backdate_attempt_artifacts(
        &fixture.tenant,
        fixture.held.run_id,
        Some(committed.recheck_id),
    )
    .await;
    assert_eq!(
        fixture
            .service
            .sweep_attempt_artifacts(&fixture.tenant, 10)
            .await
            .unwrap(),
        0
    );
    let bytes = fixture
        .service
        .load_review_recheck_content(&fixture.claim, committed.recheck_id)
        .await
        .unwrap();
    assert_eq!(dependency_content_hash(&bytes), committed.content_hash);
}

/// The database also fences a binary that does not know corrected versions.
#[tokio::test]
async fn legacy_assessment_insert_cannot_ignore_a_recorded_candidate() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let checked = candidate(
        &fixture,
        fixture.held.privacy_pass_content_hash.as_deref().unwrap(),
    )
    .await;
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    let error = tx.execute(
        "INSERT INTO pipeline_review_assessments(tenant_id,assessment_id,run_id,reviewer_principal_ref,recommendation,reason_code,resolved_quarantine_reasons,evidence_hash,phase,hold_label) VALUES($1,$2,$3,$4,'approve','legacy_review','[]',$5,'review',NULL)",
        &[&fixture.tenant, &uuid::Uuid::new_v4(), &fixture.held.run_id, &fixture.claim.reviewer_principal_ref, &dependency_content_hash(b"legacy evidence")],
    ).await.unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::RAISE_EXCEPTION)
    );
    assert!(
        error
            .as_db_error()
            .unwrap()
            .message()
            .contains(PIPELINE_REVIEW_RECHECK_BINDING_REQUIRED_LABEL)
    );
    tx.rollback().await.unwrap();
    drop(client);
    assert!(
        fixture
            .service
            .store()
            .load_review_assessment(&fixture.tenant, fixture.held.run_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .service
            .store()
            .list_review_rechecks(&fixture.tenant, fixture.held.run_id, 1)
            .await
            .unwrap(),
        vec![checked]
    );
}

/// A rollback after a new binary recorded the assessment must not let an old
/// worker release its original privacy-pass input. Exercise the DB trigger
/// directly, rather than the new application's commit_review guard.
#[tokio::test]
async fn legacy_worker_cannot_release_original_pass_after_bound_candidate_assessment() {
    let backend = runtime_backend_or_skip!(4);
    let dir = tempfile::tempdir().unwrap();
    let fixture = recheck_fixture(&backend, &dir).await;
    let original_ref = fixture.held.privacy_pass_object_ref_id.unwrap();
    let original_hash = fixture.held.privacy_pass_content_hash.clone().unwrap();
    let checked = candidate(&fixture, &original_hash).await;
    assert_ne!(checked.object_ref_id, original_ref);
    assert_ne!(checked.content_hash, original_hash);
    approve_candidate(&fixture, &checked).await;
    let worker_claim = fixture
        .service
        .store()
        .claim_run(
            &fixture.tenant,
            fixture.held.run_id,
            chrono::Duration::seconds(30),
        )
        .await
        .unwrap()
        .expect("an old worker can claim the resumed Review");
    let bad_revision = uuid::Uuid::new_v4();
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    tx.execute(
        "INSERT INTO trace_derived_records (
            tenant_id,derived_id,submission_id,trace_id,status,worker_kind,
            worker_version,input_object_ref_id,input_hash,output_object_ref_id,summary_model
         ) VALUES($1,$2,$3,$4,'current','summary','legacy_review_worker',$5,$6,$5,'legacy_review_worker')",
        &[&fixture.tenant,&bad_revision,&fixture.held.submission_id,&fixture.held.trace_id,&original_ref,&original_hash],
    ).await.expect("the old-style derived record is otherwise valid");
    let error = tx
        .execute(
            "UPDATE pipeline_runs SET approved_revision_id=$3,approved_object_ref_id=$4,
            approved_content_hash=$5,next_phase='score',state='pending',
            lease_token=NULL,lease_expires_at=NULL
         WHERE tenant_id=$1 AND run_id=$2 AND lease_token=$6",
            &[
                &fixture.tenant,
                &fixture.held.run_id,
                &bad_revision,
                &original_ref,
                &original_hash,
                &worker_claim.lease_token.unwrap(),
            ],
        )
        .await
        .expect_err(
            "V159 must reject release of the original pass after a corrected-version assessment",
        );
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::RAISE_EXCEPTION)
    );
    assert_eq!(
        error.as_db_error().unwrap().message(),
        "pipeline_review_recheck_approval_source_mismatch"
    );
    tx.rollback().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    let stale_derived_count: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_derived_records WHERE tenant_id=$1 AND derived_id=$2",
            &[&fixture.tenant, &bad_revision],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    assert_eq!(
        stale_derived_count, 0,
        "the failed old-worker transition rolls back its derived record"
    );
    drop(client);
    let after = fixture
        .service
        .store()
        .get_run(&fixture.tenant, fixture.held.run_id)
        .await
        .unwrap()
        .unwrap();
    assert!(after.approved_revision_id.is_none());
    assert!(after.approved_object_ref_id.is_none());
    assert_eq!(after.privacy_pass_object_ref_id, Some(original_ref));
    assert_eq!(after.privacy_pass_content_hash, Some(original_hash));
    assert_eq!(after.next_phase, Some(Phase::Review));
    let assessment = fixture
        .service
        .store()
        .load_review_assessment(&fixture.tenant, fixture.held.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(assessment.recheck.unwrap().recheck_id, checked.recheck_id);
}
