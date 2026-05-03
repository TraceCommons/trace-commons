use std::collections::BTreeMap;

use chrono::Utc;
use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::error::DatabaseError;
use trace_commons_server::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceCreditAccountSettlementLineItem,
    TraceCreditEventType, TraceCreditEventWrite, TraceCreditHoldReason, TraceCreditHoldWrite,
    TraceCreditSettlementBatchStatus, TraceCreditSettlementBatchWrite,
    TraceCreditSettlementNearStatus, TraceCreditSettlementState, TraceDerivedRecordWrite,
    TraceDerivedStatus, TraceExportManifestItemWrite, TraceExportManifestMirrorWrite,
    TraceExportManifestWrite, TraceNearCreditOutboxItemWrite, TraceObjectArtifactKind,
    TraceObjectRefWrite, TraceRankingCalibrationRunWrite, TraceRankingFeatureWrite,
    TraceRankingLabelOutcome, TraceRankingLabelSource, TraceRankingLabelWrite,
    TraceRankingModelStatus, TraceRankingModelVersionWrite, TraceRankingPredictionWrite,
    TraceRankingUtilityCategory, TraceRankingWorkerRunKind, TraceRankingWorkerRunStatus,
    TraceRankingWorkerRunWrite, TraceSubmissionWrite, TraceUtilityAttestationWrite,
    TraceVectorEntrySourceProjection, TraceVectorEntryStatus, TraceVectorEntryWrite,
    TraceWorkerKind,
};
use uuid::Uuid;

fn postgres_test_config() -> Option<DatabaseConfig> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    Some(DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
    })
}

async fn postgres_backend() -> Option<PgBackend> {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return None;
    };

    match PgBackend::new(&config).await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("skipping: database unavailable ({e})");
            None
        }
    }
}

fn sample_submission(tenant_id: &str, submission_id: Uuid) -> TraceSubmissionWrite {
    let mut redaction_counts = BTreeMap::new();
    redaction_counts.insert("secret".to_string(), 2);
    redaction_counts.insert("private_email".to_string(), 1);

    TraceSubmissionWrite {
        tenant_id: tenant_id.to_string(),
        submission_id,
        trace_id: Uuid::new_v4(),
        auth_principal_ref: "principal:test-user".to_string(),
        contributor_pseudonym: Some("contributor:test".to_string()),
        submitted_tenant_scope_ref: Some(tenant_id.to_string()),
        schema_version: "ironclaw.trace_contribution.v1".to_string(),
        consent_policy_version: "2026-04-24".to_string(),
        consent_scopes: vec!["training_allowed".to_string()],
        allowed_uses: vec!["debugging".to_string(), "training".to_string()],
        retention_policy_id: "standard".to_string(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".to_string(),
        redaction_pipeline_version: "deterministic-v1".to_string(),
        redaction_counts,
        redaction_hash: "sha256:redaction".to_string(),
        canonical_summary_hash: Some("sha256:canonical".to_string()),
        submission_score: Some(0.82),
        credit_points_pending: Some(1.0),
        credit_points_final: None,
        expires_at: None,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ExportMirrorCounts {
    manifests: i64,
    object_refs: i64,
    items: i64,
}

async fn export_mirror_counts(
    backend: &PgBackend,
    tenant_id: &str,
    export_manifest_id: Uuid,
) -> ExportMirrorCounts {
    let mut client = backend.pool().get().await.expect("get count connection");
    let tx = client.transaction().await.expect("start count transaction");
    tx.execute(
        "SELECT set_config('trace-commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set count tenant context");
    let row = tx
        .query_one(
            "SELECT
                (SELECT COUNT(*) FROM trace_export_manifests
                 WHERE tenant_id = $1 AND export_manifest_id = $2) AS manifests,
                (SELECT COUNT(*) FROM trace_object_refs
                 WHERE tenant_id = $1 AND created_by_job_id = $2) AS object_refs,
                (SELECT COUNT(*) FROM trace_export_manifest_items
                 WHERE tenant_id = $1 AND export_manifest_id = $2) AS items",
            &[&tenant_id, &export_manifest_id],
        )
        .await
        .expect("count export mirror rows");
    tx.commit().await.expect("commit count transaction");

    ExportMirrorCounts {
        manifests: row.get("manifests"),
        object_refs: row.get("object_refs"),
        items: row.get("items"),
    }
}

async fn cleanup_tenant(backend: &PgBackend, tenant_id: &str) {
    let mut client = backend.pool().get().await.expect("get cleanup connection");
    let tx = client
        .transaction()
        .await
        .expect("start cleanup transaction");
    tx.execute(
        "SELECT set_config('trace-commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set cleanup tenant context");
    let _ = tx
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await;
    tx.commit().await.expect("commit cleanup transaction");
}

#[tokio::test]
async fn pg_store_rolls_back_export_manifest_mirror_when_item_ref_is_invalid() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_id = format!("pg-export-mirror-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    let mut submission = sample_submission(&tenant_id, submission_id);
    submission.trace_id = trace_id;
    backend
        .upsert_trace_submission(submission)
        .await
        .expect("insert submission");

    let export_id = Uuid::new_v4();
    let object_ref_id = Uuid::new_v4();
    let derived_id = Uuid::new_v4();
    let missing_derived_id = Uuid::new_v4();
    backend
        .append_trace_derived_record(TraceDerivedRecordWrite {
            tenant_id: tenant_id.clone(),
            derived_id,
            submission_id,
            trace_id,
            status: TraceDerivedStatus::Current,
            worker_kind: TraceWorkerKind::Summary,
            worker_version: "summary-worker-v1".to_string(),
            input_object_ref: None,
            input_hash: "sha256:input".to_string(),
            output_object_ref: None,
            canonical_summary: Some("Tenant alpha summary.".to_string()),
            canonical_summary_hash: Some("sha256:alpha-summary".to_string()),
            summary_model: "summary-model-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["memory_search".to_string()],
            tool_categories: vec!["memory".to_string()],
            coverage_tags: vec!["tool:memory_search".to_string()],
            duplicate_score: Some(0.1),
            novelty_score: Some(0.4),
            cluster_id: Some("cluster:alpha".to_string()),
        })
        .await
        .expect("insert valid derived record");

    let error = backend
        .upsert_trace_export_manifest_mirror(TraceExportManifestMirrorWrite {
            manifest: TraceExportManifestWrite {
                tenant_id: tenant_id.clone(),
                export_manifest_id: export_id,
                artifact_kind: TraceObjectArtifactKind::BenchmarkArtifact,
                purpose_code: Some("atomic_mirror_failure".to_string()),
                audit_event_id: Some(Uuid::new_v4()),
                source_submission_ids: vec![submission_id],
                source_submission_ids_hash: "sha256:atomic-sources".to_string(),
                item_count: 2,
                generated_at: Utc::now(),
            },
            object_refs: vec![TraceObjectRefWrite {
                tenant_id: tenant_id.clone(),
                object_ref_id,
                submission_id,
                artifact_kind: TraceObjectArtifactKind::BenchmarkArtifact,
                object_store: "trace_commons_file_store".to_string(),
                object_key: format!("{tenant_id}/benchmarks/export/artifact.json"),
                content_sha256: "sha256:artifact".to_string(),
                encryption_key_ref: format!("tenant:{tenant_id}"),
                size_bytes: 128,
                compression: None,
                created_by_job_id: Some(export_id),
            }],
            items: vec![
                TraceExportManifestItemWrite {
                    tenant_id: tenant_id.clone(),
                    export_manifest_id: export_id,
                    submission_id,
                    trace_id,
                    derived_id: Some(derived_id),
                    object_ref_id: Some(object_ref_id),
                    vector_entry_id: None,
                    source_status_at_export: TraceCorpusStatus::Accepted,
                    source_hash_at_export: "sha256:valid-source".to_string(),
                },
                TraceExportManifestItemWrite {
                    tenant_id: tenant_id.clone(),
                    export_manifest_id: export_id,
                    submission_id,
                    trace_id,
                    derived_id: Some(missing_derived_id),
                    object_ref_id: Some(object_ref_id),
                    vector_entry_id: None,
                    source_status_at_export: TraceCorpusStatus::Accepted,
                    source_hash_at_export: "sha256:invalid-source".to_string(),
                },
            ],
        })
        .await
        .expect_err("invalid item ref rolls back whole export mirror");
    assert!(
        matches!(error, DatabaseError::Constraint(_)),
        "unexpected mirror error: {error}"
    );

    let manifests = backend
        .list_trace_export_manifests(&tenant_id)
        .await
        .expect("list manifests after failed mirror");
    assert!(
        manifests
            .iter()
            .all(|manifest| manifest.export_manifest_id != export_id),
        "failed mirror must roll back staged export manifest"
    );
    let items = backend
        .list_trace_export_manifest_items(&tenant_id, export_id)
        .await
        .expect("list manifest items after failed mirror");
    assert!(items.is_empty());
    let object_refs = backend
        .list_trace_object_refs(&tenant_id, submission_id)
        .await
        .expect("list object refs after failed mirror");
    assert!(
        object_refs
            .iter()
            .all(|object_ref| object_ref.created_by_job_id != Some(export_id)),
        "failed mirror must roll back staged export object refs"
    );
    assert_eq!(
        export_mirror_counts(&backend, &tenant_id, export_id).await,
        ExportMirrorCounts {
            manifests: 0,
            object_refs: 0,
            items: 0,
        }
    );

    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn pg_store_invalidates_exact_vector_entry_with_tenant_submission_scope() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_alpha = format!("pg-vector-alpha-{}", Uuid::new_v4());
    let tenant_beta = format!("pg-vector-beta-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    let target_derived_id = Uuid::new_v4();
    let sibling_derived_id = Uuid::new_v4();
    let target_vector_entry_id = Uuid::new_v4();
    let sibling_vector_entry_id = Uuid::new_v4();

    for tenant_id in [&tenant_alpha, &tenant_beta] {
        let mut submission = sample_submission(tenant_id, submission_id);
        submission.trace_id = trace_id;
        backend
            .upsert_trace_submission(submission)
            .await
            .expect("insert scoped submission");
        for (derived_id, summary_hash) in [
            (target_derived_id, "sha256:target-summary"),
            (sibling_derived_id, "sha256:sibling-summary"),
        ] {
            backend
                .append_trace_derived_record(TraceDerivedRecordWrite {
                    tenant_id: tenant_id.clone(),
                    derived_id,
                    submission_id,
                    trace_id,
                    status: TraceDerivedStatus::Current,
                    worker_kind: TraceWorkerKind::DuplicatePrecheck,
                    worker_version: "duplicate-precheck-v1".to_string(),
                    input_object_ref: None,
                    input_hash: summary_hash.to_string(),
                    output_object_ref: None,
                    canonical_summary: Some(format!("{tenant_id} {summary_hash}")),
                    canonical_summary_hash: Some(summary_hash.to_string()),
                    summary_model: "summary-model-v1".to_string(),
                    task_success: Some("success".to_string()),
                    privacy_risk: Some("low".to_string()),
                    event_count: Some(2),
                    tool_sequence: vec!["memory_search".to_string()],
                    tool_categories: vec!["memory".to_string()],
                    coverage_tags: vec!["tool:memory_search".to_string()],
                    duplicate_score: Some(0.1),
                    novelty_score: Some(0.4),
                    cluster_id: Some("cluster:alpha".to_string()),
                })
                .await
                .expect("insert scoped derived record");
        }
        for (derived_id, vector_entry_id, source_hash) in [
            (
                target_derived_id,
                target_vector_entry_id,
                "sha256:target-summary",
            ),
            (
                sibling_derived_id,
                sibling_vector_entry_id,
                "sha256:sibling-summary",
            ),
        ] {
            backend
                .upsert_trace_vector_entry(TraceVectorEntryWrite {
                    tenant_id: tenant_id.clone(),
                    submission_id,
                    derived_id,
                    vector_entry_id,
                    vector_store: "trace-commons-main".to_string(),
                    embedding_model: "redacted-summary-feature-hash-v1".to_string(),
                    embedding_dimension: 64,
                    embedding_version: "embedding-v1".to_string(),
                    source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
                    source_hash: source_hash.to_string(),
                    status: TraceVectorEntryStatus::Active,
                    nearest_trace_ids: Vec::new(),
                    cluster_id: Some("cluster:alpha".to_string()),
                    duplicate_score: Some(0.1),
                    novelty_score: Some(0.4),
                    indexed_at: Some(Utc::now()),
                    invalidated_at: None,
                    deleted_at: None,
                })
                .await
                .expect("insert scoped vector entry");
        }
    }

    let invalidated = backend
        .invalidate_trace_vector_entry_for_submission(
            &tenant_alpha,
            submission_id,
            target_vector_entry_id,
        )
        .await
        .expect("invalidate exact vector entry");
    assert_eq!(invalidated, 1);

    let alpha_entries = backend
        .list_trace_vector_entries(&tenant_alpha)
        .await
        .expect("list alpha vectors");
    assert_eq!(alpha_entries.len(), 2);
    assert!(alpha_entries.iter().any(|entry| {
        entry.vector_entry_id == target_vector_entry_id
            && entry.status == TraceVectorEntryStatus::Invalidated
            && entry.invalidated_at.is_some()
    }));
    assert!(alpha_entries.iter().any(|entry| {
        entry.vector_entry_id == sibling_vector_entry_id
            && entry.status == TraceVectorEntryStatus::Active
            && entry.invalidated_at.is_none()
    }));

    let beta_entries = backend
        .list_trace_vector_entries(&tenant_beta)
        .await
        .expect("list beta vectors");
    assert!(
        beta_entries
            .iter()
            .all(|entry| entry.status == TraceVectorEntryStatus::Active)
    );

    let idempotent = backend
        .invalidate_trace_vector_entry_for_submission(
            &tenant_alpha,
            submission_id,
            target_vector_entry_id,
        )
        .await
        .expect("repeat exact vector invalidation");
    assert_eq!(idempotent, 0);

    cleanup_tenant(&backend, &tenant_alpha).await;
    cleanup_tenant(&backend, &tenant_beta).await;
}

#[tokio::test]
async fn pg_store_round_trips_tenant_scoped_ranking_evidence() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_alpha = format!("pg-ranking-alpha-{}", Uuid::new_v4());
    let tenant_beta = format!("pg-ranking-beta-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    for tenant_id in [&tenant_alpha, &tenant_beta] {
        let mut submission = sample_submission(tenant_id, submission_id);
        submission.trace_id = trace_id;
        submission.allowed_uses = vec!["model_training".to_string()];
        backend
            .upsert_trace_submission(submission)
            .await
            .expect("insert accepted ranking source");
    }

    let model = backend
        .upsert_trace_ranking_model_version(TraceRankingModelVersionWrite {
            tenant_id: tenant_alpha.clone(),
            model_version: "trace-ranker-v2".to_string(),
            feature_schema_version: "ranking-features-v2".to_string(),
            policy_version: "trace-credit-policy-v2".to_string(),
            status: TraceRankingModelStatus::Candidate,
            training_dataset_hash: "sha256:training-dataset".to_string(),
            calibration_dataset_hash: "sha256:calibration-dataset".to_string(),
            model_artifact_hash: "sha256:model-artifact".to_string(),
            actor_principal_ref: "principal:ranker-admin".to_string(),
        })
        .await
        .expect("upsert ranking model version");
    assert_eq!(model.model_version, "trace-ranker-v2");

    let feature_id = Uuid::new_v4();
    let feature = backend
        .upsert_trace_ranking_feature(TraceRankingFeatureWrite {
            tenant_id: tenant_alpha.clone(),
            ranking_feature_id: feature_id,
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            feature_schema_version: model.feature_schema_version.clone(),
            feature_vector_hash: "sha256:feature-vector".to_string(),
            feature_names_hash: "sha256:feature-names".to_string(),
            source_feature_hash: "sha256:redacted-summary-features".to_string(),
            duplicate_score: Some(0.02),
            novelty_score: Some(0.91),
            privacy_risk_score: Some(0.01),
            quality_score: Some(0.88),
            coverage_tags: vec!["tool:terminal".to_string(), "outcome:success".to_string()],
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("upsert ranking feature");
    assert_eq!(feature.ranking_feature_id, feature_id);
    assert_eq!(
        feature.coverage_tags,
        vec!["tool:terminal", "outcome:success"]
    );

    let prediction_id = Uuid::new_v4();
    let prediction = backend
        .upsert_trace_ranking_prediction(TraceRankingPredictionWrite {
            tenant_id: tenant_alpha.clone(),
            ranking_prediction_id: prediction_id,
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            model_version: model.model_version.clone(),
            feature_schema_version: model.feature_schema_version.clone(),
            prediction_policy_version: "trace-credit-policy-v2".to_string(),
            feature_vector_hash: feature.feature_vector_hash.clone(),
            predicted_utility_micros: 2_100_000,
            uncertainty_micros: 300_000,
            confidence: 0.82,
            risk_penalty_micros: 50_000,
            novelty_bonus_micros: 125_000,
            settlement_score_micros: 2_175_000,
            explanation_codes: vec!["novel_tool_success".to_string()],
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("upsert ranking prediction");
    assert_eq!(prediction.ranking_prediction_id, prediction_id);
    assert_eq!(prediction.settlement_score_micros, 2_175_000);

    let label = backend
        .upsert_trace_ranking_label(TraceRankingLabelWrite {
            tenant_id: tenant_alpha.clone(),
            ranking_label_id: Uuid::new_v4(),
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            label_source: TraceRankingLabelSource::FrontierLab,
            utility_category: TraceRankingUtilityCategory::ModelTraining,
            label_outcome: TraceRankingLabelOutcome::Useful,
            utility_delta_micros: 2_500_000,
            evidence_hash: "sha256:frontier-evidence".to_string(),
            external_ref_hash: "sha256:frontier-private-ref".to_string(),
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("upsert ranking label");
    let idempotent_label = backend
        .upsert_trace_ranking_label(TraceRankingLabelWrite {
            tenant_id: tenant_alpha.clone(),
            ranking_label_id: Uuid::new_v4(),
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            label_source: TraceRankingLabelSource::FrontierLab,
            utility_category: TraceRankingUtilityCategory::ModelTraining,
            label_outcome: TraceRankingLabelOutcome::Useful,
            utility_delta_micros: 2_500_000,
            evidence_hash: "sha256:frontier-evidence".to_string(),
            external_ref_hash: "sha256:frontier-private-ref".to_string(),
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("repeat ranking label upsert is idempotent");
    assert_eq!(idempotent_label.ranking_label_id, label.ranking_label_id);

    let calibration_run_id = Uuid::new_v4();
    let calibration_run = backend
        .upsert_trace_ranking_calibration_run(TraceRankingCalibrationRunWrite {
            tenant_id: tenant_alpha.clone(),
            calibration_run_id,
            model_version: model.model_version.clone(),
            target_use: "model_training".to_string(),
            policy_version: "trace-credit-policy-v2".to_string(),
            evaluation_dataset_hash: "sha256:calibration-eval-dataset".to_string(),
            prediction_count: 1,
            label_count: 1,
            joined_label_prediction_count: 1,
            joined_label_source_count: 1,
            joined_evidence_hash: "sha256:ranking-calibration-joined-evidence".to_string(),
            average_predicted_utility_micros: Some(2_100_000),
            average_label_utility_delta_micros: Some(2_500_000),
            average_absolute_error_micros: Some(400_000),
            max_label_source_average_absolute_error_micros: Some(400_000),
            max_error_label_source: Some("frontier_lab".to_string()),
            mean_signed_error_micros: Some(-400_000),
            low_confidence_prediction_count: 0,
            confidence_threshold: 0.5,
            min_label_count: 1,
            min_label_source_count: 1,
            max_average_absolute_error_micros: 500_000,
            promotable: true,
            reason_codes: Vec::new(),
            report_hash: "sha256:ranking-calibration-report".to_string(),
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("upsert ranking calibration run");
    assert_eq!(calibration_run.calibration_run_id, calibration_run_id);
    assert_eq!(calibration_run.joined_label_source_count, 1);
    assert_eq!(calibration_run.min_label_source_count, 1);
    assert_eq!(
        calibration_run.joined_evidence_hash,
        "sha256:ranking-calibration-joined-evidence"
    );
    assert_eq!(
        calibration_run.max_label_source_average_absolute_error_micros,
        Some(400_000)
    );
    assert_eq!(
        calibration_run.max_error_label_source.as_deref(),
        Some("frontier_lab")
    );
    assert_eq!(calibration_run.mean_signed_error_micros, Some(-400_000));
    assert!(calibration_run.promotable);

    let mut reason_counts = BTreeMap::new();
    reason_counts.insert("insufficient_labels".to_string(), 1);
    let worker_run_id = Uuid::new_v4();
    let worker_run = backend
        .upsert_trace_ranking_worker_run(TraceRankingWorkerRunWrite {
            tenant_id: tenant_alpha.clone(),
            ranking_worker_run_id: worker_run_id,
            run_kind: TraceRankingWorkerRunKind::ModelPromotion,
            status: TraceRankingWorkerRunStatus::Completed,
            dry_run: false,
            reason_hash: "sha256:ranking-worker-run-reason".to_string(),
            model_version: Some(model.model_version.clone()),
            target_use: Some("model_training".to_string()),
            policy_version: Some("trace-credit-policy-v2".to_string()),
            limit: 10,
            checked_count: 2,
            succeeded_count: 1,
            skipped_existing_count: 0,
            skipped_model_risk_count: 0,
            skipped_ineligible_count: 1,
            pending_after_count: 1,
            result_refs: vec![format!("ranking_model:{}", model.model_version)],
            reason_counts,
            actor_principal_ref: "principal:ranker-worker".to_string(),
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
            last_error_hash: None,
        })
        .await
        .expect("upsert ranking worker run");
    assert_eq!(worker_run.ranking_worker_run_id, worker_run_id);
    assert_eq!(
        worker_run.run_kind,
        TraceRankingWorkerRunKind::ModelPromotion
    );
    assert_eq!(worker_run.status, TraceRankingWorkerRunStatus::Completed);
    assert_eq!(worker_run.succeeded_count, 1);
    assert!(worker_run.completed_at.is_some());
    assert_eq!(
        worker_run.result_refs,
        vec![format!("ranking_model:{}", model.model_version)]
    );
    assert_eq!(
        worker_run.reason_counts.get("insufficient_labels"),
        Some(&1)
    );

    let alpha_models = backend
        .list_trace_ranking_model_versions(&tenant_alpha)
        .await
        .expect("list alpha ranking models");
    let alpha_features = backend
        .list_trace_ranking_features(&tenant_alpha)
        .await
        .expect("list alpha ranking features");
    let alpha_predictions = backend
        .list_trace_ranking_predictions(&tenant_alpha)
        .await
        .expect("list alpha ranking predictions");
    let alpha_labels = backend
        .list_trace_ranking_labels(&tenant_alpha)
        .await
        .expect("list alpha ranking labels");
    let alpha_calibration_runs = backend
        .list_trace_ranking_calibration_runs(&tenant_alpha)
        .await
        .expect("list alpha ranking calibration runs");
    let alpha_worker_runs = backend
        .list_trace_ranking_worker_runs(&tenant_alpha)
        .await
        .expect("list alpha ranking worker runs");
    assert_eq!(alpha_models.len(), 1);
    assert_eq!(alpha_features.len(), 1);
    assert_eq!(alpha_predictions.len(), 1);
    assert_eq!(alpha_labels.len(), 1);
    assert_eq!(alpha_calibration_runs.len(), 1);
    assert_eq!(alpha_worker_runs.len(), 1);

    assert!(
        backend
            .list_trace_ranking_labels(&tenant_beta)
            .await
            .expect("list beta ranking labels")
            .is_empty(),
        "ranking evidence must stay tenant scoped"
    );
    assert!(
        backend
            .list_trace_ranking_calibration_runs(&tenant_beta)
            .await
            .expect("list beta ranking calibration runs")
            .is_empty(),
        "ranking calibration runs must stay tenant scoped"
    );
    assert!(
        backend
            .list_trace_ranking_worker_runs(&tenant_beta)
            .await
            .expect("list beta ranking worker runs")
            .is_empty(),
        "ranking worker runs must stay tenant scoped"
    );

    cleanup_tenant(&backend, &tenant_alpha).await;
    cleanup_tenant(&backend, &tenant_beta).await;
}

#[tokio::test]
async fn pg_store_round_trips_tenant_scoped_credit_settlement_control_plane() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_alpha = format!("pg-settlement-alpha-{}", Uuid::new_v4());
    let tenant_beta = format!("pg-settlement-beta-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    for tenant_id in [&tenant_alpha, &tenant_beta] {
        let mut submission = sample_submission(tenant_id, submission_id);
        submission.trace_id = trace_id;
        submission.allowed_uses = vec!["ranking_model_training".to_string()];
        backend
            .upsert_trace_submission(submission)
            .await
            .expect("insert settlement source submission");
    }

    let credit_event_id = Uuid::new_v4();
    backend
        .append_trace_credit_event(TraceCreditEventWrite {
            credit_event_id,
            tenant_id: tenant_alpha.clone(),
            submission_id,
            trace_id,
            credit_account_ref: "principal:settlement-account".to_string(),
            event_type: TraceCreditEventType::RankingUtility,
            points_delta: "1.250000".to_string(),
            reason: "ranking settlement control-plane test".to_string(),
            external_ref: Some("ranker:settlement-control-plane".to_string()),
            actor_principal_ref: "principal:ranker-worker".to_string(),
            actor_role: "utility_worker".to_string(),
            settlement_state: TraceCreditSettlementState::Pending,
        })
        .await
        .expect("insert settlement source credit event");

    let attestation_id = Uuid::new_v4();
    let attestation = backend
        .upsert_trace_utility_attestation(TraceUtilityAttestationWrite {
            tenant_id: tenant_alpha.clone(),
            attestation_id,
            event_type: TraceCreditEventType::RankingUtility,
            use_category: "ranking".to_string(),
            policy_version: "trace-credit-policy-v3".to_string(),
            evidence_hash: "sha256:settlement-attestation-evidence".to_string(),
            external_ref_hash: "sha256:settlement-attestation-ref".to_string(),
            source_submission_ids: vec![submission_id],
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("upsert utility attestation");
    assert_eq!(attestation.attestation_id, attestation_id);
    assert_eq!(attestation.event_type, TraceCreditEventType::RankingUtility);

    let hold_id = Uuid::new_v4();
    let hold = backend
        .upsert_trace_credit_hold(TraceCreditHoldWrite {
            tenant_id: tenant_alpha.clone(),
            hold_id,
            credit_account_ref: "principal:settlement-account".to_string(),
            credit_account_hash: "sha256:settlement-account".to_string(),
            reason: TraceCreditHoldReason::AttestationDispute,
            reason_hash: "sha256:settlement-hold-reason".to_string(),
            actor_principal_ref: "principal:admin".to_string(),
            released_at: None,
        })
        .await
        .expect("upsert credit hold");
    assert_eq!(hold.hold_id, hold_id);
    assert_eq!(hold.reason, TraceCreditHoldReason::AttestationDispute);

    let settlement_batch_id = Uuid::new_v4();
    let source_list_hash = "sha256:settlement-source-list".to_string();
    let settlement = backend
        .upsert_trace_credit_settlement_batch(TraceCreditSettlementBatchWrite {
            tenant_id: tenant_alpha.clone(),
            settlement_batch_id,
            policy_version: "trace-credit-policy-v3".to_string(),
            status: TraceCreditSettlementBatchStatus::Finalized,
            reason_hash: "sha256:settlement-reason".to_string(),
            source_credit_event_ids: vec![credit_event_id],
            source_submission_ids: vec![submission_id],
            source_list_hash: source_list_hash.clone(),
            settled_credit_points: "1.250000".to_string(),
            settled_credit_micros: 1_250_000,
            line_items: vec![TraceCreditAccountSettlementLineItem {
                credit_account_ref: "principal:settlement-account".to_string(),
                credit_account_hash: "sha256:settlement-account".to_string(),
                settled_credit_delta_micros: 1_250_000,
                source_credit_event_ids: vec![credit_event_id],
                source_submission_ids: vec![submission_id],
                source_list_hash: source_list_hash.clone(),
                near_status: TraceCreditSettlementNearStatus::Pending,
                near_outbox_id: Some(Uuid::nil()),
            }],
            near_contract_id: Some("trace-credits.testnet".to_string()),
            ranking_model_version: Some("trace-ranker-settlement-v3".to_string()),
            ranking_target_use: Some("ranking_model_training".to_string()),
            ranking_calibration_run_id: Some(Uuid::new_v4()),
            ranking_calibration_report_hash: Some(
                "sha256:settlement-calibration-report".to_string(),
            ),
            ranking_calibration_joined_evidence_hash: Some(
                "sha256:settlement-calibration-joined-evidence".to_string(),
            ),
            ranking_credit_events_excluded_count: 0,
            ranking_credit_events_excluded_reason_counts: BTreeMap::from([(
                "missing_prediction_ref".to_string(),
                1,
            )]),
            actor_principal_ref: "principal:admin".to_string(),
        })
        .await
        .expect("upsert settlement batch");
    assert_eq!(settlement.settlement_batch_id, settlement_batch_id);
    assert_eq!(settlement.line_items.len(), 1);
    assert_eq!(
        settlement.ranking_model_version.as_deref(),
        Some("trace-ranker-settlement-v3")
    );
    assert_eq!(
        settlement
            .ranking_calibration_joined_evidence_hash
            .as_deref(),
        Some("sha256:settlement-calibration-joined-evidence")
    );
    assert_eq!(
        settlement
            .ranking_credit_events_excluded_reason_counts
            .get("missing_prediction_ref"),
        Some(&1)
    );

    let near_outbox_id = Uuid::new_v4();
    let near_item = backend
        .upsert_trace_near_credit_outbox_item(TraceNearCreditOutboxItemWrite {
            tenant_id: tenant_alpha.clone(),
            near_outbox_id,
            settlement_batch_id,
            credit_account_hash: "sha256:settlement-account".to_string(),
            near_call_json: serde_json::json!({
                "contract_id": "trace-credits.testnet",
                "method_name": "settle_credit_receipt",
                "args": {
                    "settlement_batch_id": settlement_batch_id,
                    "credit_account_hash": "sha256:settlement-account"
                },
                "idempotency_key": "sha256:settlement-near-call"
            }),
            status: TraceCreditSettlementNearStatus::Pending,
        })
        .await
        .expect("upsert NEAR outbox item");
    assert_eq!(near_item.near_outbox_id, near_outbox_id);
    assert_eq!(near_item.status, TraceCreditSettlementNearStatus::Pending);

    let updated = backend
        .update_trace_near_credit_outbox_status(
            &tenant_alpha,
            near_outbox_id,
            TraceCreditSettlementNearStatus::Submitted,
            Some("near-public-tx-hash".to_string()),
            None,
        )
        .await
        .expect("update NEAR outbox item")
        .expect("updated item exists");
    assert_eq!(updated.status, TraceCreditSettlementNearStatus::Submitted);
    assert_eq!(
        updated.near_transaction_hash.as_deref(),
        Some("near-public-tx-hash")
    );
    assert!(updated.submitted_at.is_some());

    assert_eq!(
        backend
            .list_trace_utility_attestations(&tenant_alpha)
            .await
            .expect("list alpha attestations")
            .len(),
        1
    );
    assert_eq!(
        backend
            .list_trace_credit_holds(&tenant_alpha)
            .await
            .expect("list alpha holds")
            .len(),
        1
    );
    assert_eq!(
        backend
            .list_trace_credit_settlement_batches(&tenant_alpha)
            .await
            .expect("list alpha settlement batches")
            .len(),
        1
    );
    assert_eq!(
        backend
            .list_trace_near_credit_outbox_items(&tenant_alpha)
            .await
            .expect("list alpha NEAR outbox")
            .len(),
        1
    );
    assert!(
        backend
            .list_trace_credit_settlement_batches(&tenant_beta)
            .await
            .expect("list beta settlement batches")
            .is_empty(),
        "settlement batches must stay tenant scoped"
    );
    assert!(
        backend
            .list_trace_near_credit_outbox_items(&tenant_beta)
            .await
            .expect("list beta NEAR outbox")
            .is_empty(),
        "NEAR outbox items must stay tenant scoped"
    );

    cleanup_tenant(&backend, &tenant_alpha).await;
    cleanup_tenant(&backend, &tenant_beta).await;
}
