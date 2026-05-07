use std::collections::BTreeMap;

use chrono::Utc;
use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::error::DatabaseError;
use trace_commons_server::trace_corpus_storage::{
    TraceBenchmarkRegistryOutboxItemWrite, TraceBenchmarkRegistryOutboxOperation,
    TraceBenchmarkRegistryOutboxStatus, TraceCorpusStatus, TraceCorpusStore,
    TraceCreditAccountSettlementLineItem, TraceCreditEventType, TraceCreditEventWrite,
    TraceCreditHoldReason, TraceCreditHoldWrite, TraceCreditSettlementBatchStatus,
    TraceCreditSettlementBatchWrite, TraceCreditSettlementNearStatus, TraceCreditSettlementState,
    TraceDerivedRecordWrite, TraceDerivedStatus, TraceExportManifestItemWrite,
    TraceExportManifestMirrorWrite, TraceExportManifestWrite, TraceNearCreditOutboxItemWrite,
    TraceObjectArtifactKind, TraceObjectRefWrite, TraceRankingCalibrationDatasetStatus,
    TraceRankingCalibrationDatasetStatusUpdate, TraceRankingCalibrationDatasetWrite,
    TraceRankingCalibrationRunWrite, TraceRankingFeatureWrite, TraceRankingLabelOutcome,
    TraceRankingLabelSource, TraceRankingLabelWrite, TraceRankingModelStatus,
    TraceRankingModelVersionWrite, TraceRankingPredictionWrite, TraceRankingPreferenceLabelWrite,
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
async fn pg_store_round_trips_tenant_scoped_benchmark_registry_outbox() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_alpha = format!("pg-benchmark-outbox-alpha-{}", Uuid::new_v4());
    let tenant_beta = format!("pg-benchmark-outbox-beta-{}", Uuid::new_v4());
    let benchmark_outbox_id = Uuid::new_v4();
    let conversion_id = Uuid::new_v4();

    let inserted = backend
        .upsert_trace_benchmark_registry_outbox_item(TraceBenchmarkRegistryOutboxItemWrite {
            tenant_id: tenant_alpha.clone(),
            benchmark_outbox_id,
            conversion_id,
            operation: TraceBenchmarkRegistryOutboxOperation::Publish,
            registry_ref: "benchmark-registry:tenant-alpha:conversion".to_string(),
            artifact_payload_hash: "sha256:benchmark-artifact-payload".to_string(),
            source_submission_ids_hash: "sha256:benchmark-sources".to_string(),
            evaluator_ref: Some("deterministic-benchmark-evaluator:v1".to_string()),
            evaluation_score: Some(1.0),
            status: TraceBenchmarkRegistryOutboxStatus::Pending,
        })
        .await
        .expect("upsert benchmark registry outbox item");
    assert_eq!(inserted.benchmark_outbox_id, benchmark_outbox_id);
    assert_eq!(inserted.status, TraceBenchmarkRegistryOutboxStatus::Pending);

    backend
        .upsert_trace_benchmark_registry_outbox_item(TraceBenchmarkRegistryOutboxItemWrite {
            tenant_id: tenant_beta.clone(),
            benchmark_outbox_id,
            conversion_id,
            operation: TraceBenchmarkRegistryOutboxOperation::Publish,
            registry_ref: "benchmark-registry:tenant-beta:conversion".to_string(),
            artifact_payload_hash: "sha256:benchmark-artifact-payload-beta".to_string(),
            source_submission_ids_hash: "sha256:benchmark-sources-beta".to_string(),
            evaluator_ref: Some("deterministic-benchmark-evaluator:v1".to_string()),
            evaluation_score: Some(0.9),
            status: TraceBenchmarkRegistryOutboxStatus::Pending,
        })
        .await
        .expect("upsert beta benchmark registry outbox item with same ids");

    let submitted = backend
        .update_trace_benchmark_registry_outbox_status(
            &tenant_alpha,
            benchmark_outbox_id,
            TraceBenchmarkRegistryOutboxStatus::Submitted,
            Some("external-registry:submission:alpha".to_string()),
            None,
        )
        .await
        .expect("update benchmark registry outbox")
        .expect("updated item exists");
    assert_eq!(
        submitted.status,
        TraceBenchmarkRegistryOutboxStatus::Submitted
    );
    assert_eq!(
        submitted.external_receipt_ref.as_deref(),
        Some("external-registry:submission:alpha")
    );
    assert!(submitted.submitted_at.is_some());

    let alpha_items = backend
        .list_trace_benchmark_registry_outbox_items(&tenant_alpha)
        .await
        .expect("list alpha benchmark registry outbox");
    assert_eq!(alpha_items.len(), 1);
    assert_eq!(alpha_items[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_items[0].benchmark_outbox_id, benchmark_outbox_id);
    assert_eq!(alpha_items[0].conversion_id, conversion_id);
    assert_eq!(
        alpha_items[0].status,
        TraceBenchmarkRegistryOutboxStatus::Submitted
    );
    assert_eq!(
        alpha_items[0].artifact_payload_hash.as_str(),
        "sha256:benchmark-artifact-payload"
    );
    assert_eq!(
        alpha_items[0].external_receipt_ref.as_deref(),
        Some("external-registry:submission:alpha")
    );

    let beta_items = backend
        .list_trace_benchmark_registry_outbox_items(&tenant_beta)
        .await
        .expect("list beta benchmark registry outbox");
    assert_eq!(beta_items.len(), 1);
    assert_eq!(beta_items[0].tenant_id, tenant_beta);
    assert_eq!(beta_items[0].benchmark_outbox_id, benchmark_outbox_id);
    assert_eq!(beta_items[0].conversion_id, conversion_id);
    assert_eq!(
        beta_items[0].status,
        TraceBenchmarkRegistryOutboxStatus::Pending
    );
    assert_eq!(
        beta_items[0].artifact_payload_hash.as_str(),
        "sha256:benchmark-artifact-payload-beta"
    );
    assert!(
        beta_items[0].external_receipt_ref.is_none(),
        "benchmark registry outbox status updates must stay tenant scoped even when ids overlap"
    );

    cleanup_tenant(&backend, &tenant_alpha).await;
    cleanup_tenant(&backend, &tenant_beta).await;
}

#[tokio::test]
async fn pg_store_preserves_ranking_calibration_dataset_manifest_on_status_update() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_id = format!("pg-ranking-calibration-{}", Uuid::new_v4());
    let initial = TraceRankingCalibrationDatasetWrite {
        tenant_id: tenant_id.clone(),
        calibration_dataset_hash: "sha256:pg-calibration-dataset".to_string(),
        target_use: "model_training".to_string(),
        policy_version: "trace-credit-policy-v2".to_string(),
        source_manifest_hash: "sha256:pg-calibration-source-manifest-v1".to_string(),
        source_count: 32,
        label_source_count: 2,
        label_actor_count: 2,
        status: TraceRankingCalibrationDatasetStatus::Candidate,
        actor_principal_ref: "principal:ranker-admin".to_string(),
    };

    backend
        .upsert_trace_ranking_calibration_dataset(initial.clone())
        .await
        .expect("insert ranking calibration dataset");

    let mut status_update = initial.clone();
    status_update.status = TraceRankingCalibrationDatasetStatus::Active;
    let active = backend
        .upsert_trace_ranking_calibration_dataset(status_update.clone())
        .await
        .expect("status-only update keeps immutable manifest metadata");
    assert_eq!(active.status, TraceRankingCalibrationDatasetStatus::Active);
    assert_eq!(active.source_manifest_hash, initial.source_manifest_hash);

    let mut rewrite = status_update;
    rewrite.source_manifest_hash = "sha256:pg-calibration-source-manifest-v2".to_string();
    let error = backend
        .upsert_trace_ranking_calibration_dataset(rewrite)
        .await
        .expect_err("manifest rewrite is rejected by the database store");
    assert!(matches!(
        error,
        DatabaseError::Constraint(message) if message.contains("immutable")
    ));

    let records = backend
        .list_trace_ranking_calibration_datasets(&tenant_id)
        .await
        .expect("list ranking calibration datasets");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].source_manifest_hash,
        initial.source_manifest_hash
    );
    assert_eq!(
        records[0].status,
        TraceRankingCalibrationDatasetStatus::Active
    );

    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn pg_store_archives_ranking_calibration_dataset_without_manifest_update() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_id = format!("pg-ranking-calibration-archive-{}", Uuid::new_v4());
    let initial = TraceRankingCalibrationDatasetWrite {
        tenant_id: tenant_id.clone(),
        calibration_dataset_hash: "sha256:pg-calibration-dataset-archive".to_string(),
        target_use: "model_training".to_string(),
        policy_version: "trace-credit-policy-v2".to_string(),
        source_manifest_hash: "sha256:pg-calibration-source-manifest-original".to_string(),
        source_count: 32,
        label_source_count: 2,
        label_actor_count: 2,
        status: TraceRankingCalibrationDatasetStatus::Candidate,
        actor_principal_ref: "principal:ranker-admin".to_string(),
    };

    backend
        .upsert_trace_ranking_calibration_dataset(initial.clone())
        .await
        .expect("insert ranking calibration dataset");

    let archived = backend
        .update_trace_ranking_calibration_dataset_status(
            TraceRankingCalibrationDatasetStatusUpdate {
                tenant_id: tenant_id.clone(),
                calibration_dataset_hash: initial.calibration_dataset_hash.clone(),
                target_use: initial.target_use.clone(),
                policy_version: initial.policy_version.clone(),
                status: TraceRankingCalibrationDatasetStatus::Archived,
                actor_principal_ref: "principal:ranker-admin-quarantine".to_string(),
            },
        )
        .await
        .expect("archive status update preserves immutable manifest metadata");

    assert_eq!(
        archived.source_manifest_hash,
        "sha256:pg-calibration-source-manifest-original"
    );
    assert_eq!(archived.source_count, 32);
    assert_eq!(
        archived.status,
        TraceRankingCalibrationDatasetStatus::Archived
    );
    assert_eq!(
        archived.actor_principal_ref,
        "principal:ranker-admin-quarantine"
    );

    cleanup_tenant(&backend, &tenant_id).await;
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

    backend
        .upsert_trace_ranking_model_version(TraceRankingModelVersionWrite {
            tenant_id: tenant_beta.clone(),
            model_version: model.model_version.clone(),
            feature_schema_version: model.feature_schema_version.clone(),
            policy_version: model.policy_version.clone(),
            status: TraceRankingModelStatus::Candidate,
            training_dataset_hash: "sha256:training-dataset-beta".to_string(),
            calibration_dataset_hash: model.calibration_dataset_hash.clone(),
            model_artifact_hash: "sha256:model-artifact-beta".to_string(),
            actor_principal_ref: "principal:ranker-admin-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking model version with same model id");

    let calibration_dataset = backend
        .upsert_trace_ranking_calibration_dataset(TraceRankingCalibrationDatasetWrite {
            tenant_id: tenant_alpha.clone(),
            calibration_dataset_hash: model.calibration_dataset_hash.clone(),
            target_use: "model_training".to_string(),
            policy_version: model.policy_version.clone(),
            source_manifest_hash: "sha256:calibration-source-manifest".to_string(),
            source_count: 32,
            label_source_count: 2,
            label_actor_count: 2,
            status: TraceRankingCalibrationDatasetStatus::Candidate,
            actor_principal_ref: "principal:ranker-admin".to_string(),
        })
        .await
        .expect("upsert ranking calibration dataset");
    assert_eq!(
        calibration_dataset.calibration_dataset_hash,
        "sha256:calibration-dataset"
    );
    assert_eq!(
        calibration_dataset.status,
        TraceRankingCalibrationDatasetStatus::Candidate
    );

    backend
        .upsert_trace_ranking_calibration_dataset(TraceRankingCalibrationDatasetWrite {
            tenant_id: tenant_beta.clone(),
            calibration_dataset_hash: model.calibration_dataset_hash.clone(),
            target_use: "model_training".to_string(),
            policy_version: model.policy_version.clone(),
            source_manifest_hash: "sha256:calibration-source-manifest-beta".to_string(),
            source_count: 24,
            label_source_count: 1,
            label_actor_count: 1,
            status: TraceRankingCalibrationDatasetStatus::Candidate,
            actor_principal_ref: "principal:ranker-admin-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking calibration dataset with same dataset key");

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

    backend
        .upsert_trace_ranking_feature(TraceRankingFeatureWrite {
            tenant_id: tenant_beta.clone(),
            ranking_feature_id: feature_id,
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            feature_schema_version: model.feature_schema_version.clone(),
            feature_vector_hash: "sha256:feature-vector-beta".to_string(),
            feature_names_hash: "sha256:feature-names-beta".to_string(),
            source_feature_hash: "sha256:redacted-summary-features-beta".to_string(),
            duplicate_score: Some(0.12),
            novelty_score: Some(0.81),
            privacy_risk_score: Some(0.03),
            quality_score: Some(0.77),
            coverage_tags: vec!["tool:editor".to_string()],
            actor_principal_ref: "principal:ranker-worker-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking feature with same id");

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

    backend
        .upsert_trace_ranking_prediction(TraceRankingPredictionWrite {
            tenant_id: tenant_beta.clone(),
            ranking_prediction_id: prediction_id,
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            model_version: model.model_version.clone(),
            feature_schema_version: model.feature_schema_version.clone(),
            prediction_policy_version: "trace-credit-policy-v2".to_string(),
            feature_vector_hash: "sha256:feature-vector-beta".to_string(),
            predicted_utility_micros: 1_800_000,
            uncertainty_micros: 400_000,
            confidence: 0.72,
            risk_penalty_micros: 75_000,
            novelty_bonus_micros: 50_000,
            settlement_score_micros: 1_775_000,
            explanation_codes: vec!["beta_tool_success".to_string()],
            actor_principal_ref: "principal:ranker-worker-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking prediction with same id");

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

    backend
        .upsert_trace_ranking_label(TraceRankingLabelWrite {
            tenant_id: tenant_beta.clone(),
            ranking_label_id: label.ranking_label_id,
            submission_id,
            trace_id,
            target_use: "model_training".to_string(),
            label_source: TraceRankingLabelSource::FrontierLab,
            utility_category: TraceRankingUtilityCategory::ModelTraining,
            label_outcome: TraceRankingLabelOutcome::Useful,
            utility_delta_micros: 1_900_000,
            evidence_hash: "sha256:frontier-evidence-beta".to_string(),
            external_ref_hash: "sha256:frontier-private-ref".to_string(),
            actor_principal_ref: "principal:ranker-worker-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking label with same idempotency key");

    let second_submission_id = Uuid::new_v4();
    let second_trace_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(TraceSubmissionWrite {
            tenant_id: tenant_alpha.clone(),
            submission_id: second_submission_id,
            trace_id: second_trace_id,
            auth_principal_ref: "principal:contributor".to_string(),
            contributor_pseudonym: Some("contributor-2".to_string()),
            submitted_tenant_scope_ref: Some("tenant-scope".to_string()),
            schema_version: "ironclaw.trace_contribution.v1".to_string(),
            consent_policy_version: "trace-credit-policy-v2".to_string(),
            consent_scopes: vec!["ranking_training".to_string()],
            allowed_uses: vec!["ranking_model_training".to_string()],
            retention_policy_id: "private_corpus_revocable".to_string(),
            status: TraceCorpusStatus::Accepted,
            privacy_risk: "low".to_string(),
            redaction_pipeline_version: "server-rescrub-v1".to_string(),
            redaction_counts: BTreeMap::new(),
            redaction_hash: "sha256:second-redaction".to_string(),
            canonical_summary_hash: Some("sha256:second-summary".to_string()),
            submission_score: Some(0.5),
            credit_points_pending: Some(0.0),
            credit_points_final: None,
            expires_at: None,
        })
        .await
        .expect("insert second accepted ranking source");
    backend
        .upsert_trace_submission(TraceSubmissionWrite {
            tenant_id: tenant_beta.clone(),
            submission_id: second_submission_id,
            trace_id: second_trace_id,
            auth_principal_ref: "principal:contributor-beta".to_string(),
            contributor_pseudonym: Some("contributor-2-beta".to_string()),
            submitted_tenant_scope_ref: Some("tenant-scope-beta".to_string()),
            schema_version: "ironclaw.trace_contribution.v1".to_string(),
            consent_policy_version: "trace-credit-policy-v2".to_string(),
            consent_scopes: vec!["ranking_training".to_string()],
            allowed_uses: vec!["ranking_model_training".to_string()],
            retention_policy_id: "private_corpus_revocable".to_string(),
            status: TraceCorpusStatus::Accepted,
            privacy_risk: "low".to_string(),
            redaction_pipeline_version: "server-rescrub-v1".to_string(),
            redaction_counts: BTreeMap::new(),
            redaction_hash: "sha256:second-redaction-beta".to_string(),
            canonical_summary_hash: Some("sha256:second-summary-beta".to_string()),
            submission_score: Some(0.4),
            credit_points_pending: Some(0.0),
            credit_points_final: None,
            expires_at: None,
        })
        .await
        .expect("insert beta second accepted ranking source with same ids");
    let preference_label = backend
        .upsert_trace_ranking_preference_label(TraceRankingPreferenceLabelWrite {
            tenant_id: tenant_alpha.clone(),
            preference_label_id: Uuid::new_v4(),
            preferred_submission_id: submission_id,
            preferred_trace_id: trace_id,
            rejected_submission_id: second_submission_id,
            rejected_trace_id: second_trace_id,
            target_use: "ranking_model_training".to_string(),
            label_source: TraceRankingLabelSource::FrontierLab,
            utility_category: TraceRankingUtilityCategory::RankingTraining,
            preference_strength_micros: 850_000,
            evidence_hash: "sha256:pairwise-frontier-evidence".to_string(),
            external_ref_hash: "sha256:frontier-private-pair-ref".to_string(),
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("upsert ranking preference label");
    let idempotent_preference = backend
        .upsert_trace_ranking_preference_label(TraceRankingPreferenceLabelWrite {
            tenant_id: tenant_alpha.clone(),
            preference_label_id: Uuid::new_v4(),
            preferred_submission_id: submission_id,
            preferred_trace_id: trace_id,
            rejected_submission_id: second_submission_id,
            rejected_trace_id: second_trace_id,
            target_use: "ranking_model_training".to_string(),
            label_source: TraceRankingLabelSource::FrontierLab,
            utility_category: TraceRankingUtilityCategory::RankingTraining,
            preference_strength_micros: 850_000,
            evidence_hash: "sha256:pairwise-frontier-evidence".to_string(),
            external_ref_hash: "sha256:frontier-private-pair-ref".to_string(),
            actor_principal_ref: "principal:ranker-worker".to_string(),
        })
        .await
        .expect("repeat ranking preference label upsert is idempotent");
    assert_eq!(
        idempotent_preference.preference_label_id,
        preference_label.preference_label_id
    );

    backend
        .upsert_trace_ranking_preference_label(TraceRankingPreferenceLabelWrite {
            tenant_id: tenant_beta.clone(),
            preference_label_id: preference_label.preference_label_id,
            preferred_submission_id: submission_id,
            preferred_trace_id: trace_id,
            rejected_submission_id: second_submission_id,
            rejected_trace_id: second_trace_id,
            target_use: "ranking_model_training".to_string(),
            label_source: TraceRankingLabelSource::FrontierLab,
            utility_category: TraceRankingUtilityCategory::RankingTraining,
            preference_strength_micros: 650_000,
            evidence_hash: "sha256:pairwise-frontier-evidence-beta".to_string(),
            external_ref_hash: "sha256:frontier-private-pair-ref".to_string(),
            actor_principal_ref: "principal:ranker-worker-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking preference label with same idempotency key");

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

    backend
        .upsert_trace_ranking_calibration_run(TraceRankingCalibrationRunWrite {
            tenant_id: tenant_beta.clone(),
            calibration_run_id,
            model_version: model.model_version.clone(),
            target_use: "model_training".to_string(),
            policy_version: "trace-credit-policy-v2".to_string(),
            evaluation_dataset_hash: "sha256:calibration-eval-dataset-beta".to_string(),
            prediction_count: 1,
            label_count: 1,
            joined_label_prediction_count: 1,
            joined_label_source_count: 1,
            joined_evidence_hash: "sha256:ranking-calibration-joined-evidence-beta".to_string(),
            average_predicted_utility_micros: Some(1_800_000),
            average_label_utility_delta_micros: Some(1_900_000),
            average_absolute_error_micros: Some(100_000),
            max_label_source_average_absolute_error_micros: Some(100_000),
            max_error_label_source: Some("frontier_lab".to_string()),
            mean_signed_error_micros: Some(-100_000),
            low_confidence_prediction_count: 0,
            confidence_threshold: 0.5,
            min_label_count: 1,
            min_label_source_count: 1,
            max_average_absolute_error_micros: 500_000,
            promotable: true,
            reason_codes: Vec::new(),
            report_hash: "sha256:ranking-calibration-report-beta".to_string(),
            actor_principal_ref: "principal:ranker-worker-beta".to_string(),
        })
        .await
        .expect("upsert beta ranking calibration run with same id");

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

    let mut beta_reason_counts = BTreeMap::new();
    beta_reason_counts.insert("beta_pending".to_string(), 1);
    backend
        .upsert_trace_ranking_worker_run(TraceRankingWorkerRunWrite {
            tenant_id: tenant_beta.clone(),
            ranking_worker_run_id: worker_run_id,
            run_kind: TraceRankingWorkerRunKind::ModelPromotion,
            status: TraceRankingWorkerRunStatus::Running,
            dry_run: false,
            reason_hash: "sha256:ranking-worker-run-reason-beta".to_string(),
            model_version: Some(model.model_version.clone()),
            target_use: Some("model_training".to_string()),
            policy_version: Some("trace-credit-policy-v2".to_string()),
            limit: 10,
            checked_count: 1,
            succeeded_count: 0,
            skipped_existing_count: 0,
            skipped_model_risk_count: 0,
            skipped_ineligible_count: 1,
            pending_after_count: 1,
            result_refs: vec![format!("ranking_model:{}", model.model_version)],
            reason_counts: beta_reason_counts,
            actor_principal_ref: "principal:ranker-worker-beta".to_string(),
            created_at: Utc::now(),
            completed_at: None,
            last_error_hash: None,
        })
        .await
        .expect("upsert beta ranking worker run with same id");

    let alpha_models = backend
        .list_trace_ranking_model_versions(&tenant_alpha)
        .await
        .expect("list alpha ranking models");
    let alpha_calibration_datasets = backend
        .list_trace_ranking_calibration_datasets(&tenant_alpha)
        .await
        .expect("list alpha ranking calibration datasets");
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
    let alpha_preference_labels = backend
        .list_trace_ranking_preference_labels(&tenant_alpha)
        .await
        .expect("list alpha ranking preference labels");
    let alpha_calibration_runs = backend
        .list_trace_ranking_calibration_runs(&tenant_alpha)
        .await
        .expect("list alpha ranking calibration runs");
    let alpha_worker_runs = backend
        .list_trace_ranking_worker_runs(&tenant_alpha)
        .await
        .expect("list alpha ranking worker runs");
    assert_eq!(alpha_models.len(), 1);
    assert_eq!(alpha_models[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_models[0].model_version, model.model_version);
    assert_eq!(
        alpha_models[0].model_artifact_hash.as_str(),
        "sha256:model-artifact"
    );
    assert_eq!(alpha_calibration_datasets.len(), 1);
    assert_eq!(alpha_calibration_datasets[0].tenant_id, tenant_alpha);
    assert_eq!(
        alpha_calibration_datasets[0].source_manifest_hash.as_str(),
        "sha256:calibration-source-manifest"
    );
    assert_eq!(alpha_features.len(), 1);
    assert_eq!(alpha_features[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_features[0].ranking_feature_id, feature_id);
    assert_eq!(
        alpha_features[0].feature_vector_hash.as_str(),
        "sha256:feature-vector"
    );
    assert_eq!(alpha_predictions.len(), 1);
    assert_eq!(alpha_predictions[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_predictions[0].ranking_prediction_id, prediction_id);
    assert_eq!(alpha_predictions[0].settlement_score_micros, 2_175_000);
    assert_eq!(alpha_labels.len(), 1);
    assert_eq!(alpha_labels[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_labels[0].ranking_label_id, label.ranking_label_id);
    assert_eq!(alpha_labels[0].utility_delta_micros, 2_500_000);
    assert_eq!(alpha_preference_labels.len(), 1);
    assert_eq!(alpha_preference_labels[0].tenant_id, tenant_alpha);
    assert_eq!(
        alpha_preference_labels[0].preference_label_id,
        preference_label.preference_label_id
    );
    assert_eq!(
        alpha_preference_labels[0].preference_strength_micros,
        850_000
    );
    assert_eq!(alpha_calibration_runs.len(), 1);
    assert_eq!(alpha_calibration_runs[0].tenant_id, tenant_alpha);
    assert_eq!(
        alpha_calibration_runs[0].calibration_run_id,
        calibration_run_id
    );
    assert_eq!(
        alpha_calibration_runs[0].report_hash.as_str(),
        "sha256:ranking-calibration-report"
    );
    assert_eq!(alpha_worker_runs.len(), 1);
    assert_eq!(alpha_worker_runs[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_worker_runs[0].ranking_worker_run_id, worker_run_id);
    assert_eq!(
        alpha_worker_runs[0].status,
        TraceRankingWorkerRunStatus::Completed
    );

    let beta_models = backend
        .list_trace_ranking_model_versions(&tenant_beta)
        .await
        .expect("list beta ranking models");
    let beta_calibration_datasets = backend
        .list_trace_ranking_calibration_datasets(&tenant_beta)
        .await
        .expect("list beta ranking calibration datasets");
    let beta_features = backend
        .list_trace_ranking_features(&tenant_beta)
        .await
        .expect("list beta ranking features");
    let beta_predictions = backend
        .list_trace_ranking_predictions(&tenant_beta)
        .await
        .expect("list beta ranking predictions");
    let beta_labels = backend
        .list_trace_ranking_labels(&tenant_beta)
        .await
        .expect("list beta ranking labels");
    let beta_preference_labels = backend
        .list_trace_ranking_preference_labels(&tenant_beta)
        .await
        .expect("list beta ranking preference labels");
    let beta_calibration_runs = backend
        .list_trace_ranking_calibration_runs(&tenant_beta)
        .await
        .expect("list beta ranking calibration runs");
    let beta_worker_runs = backend
        .list_trace_ranking_worker_runs(&tenant_beta)
        .await
        .expect("list beta ranking worker runs");
    assert_eq!(beta_models.len(), 1);
    assert_eq!(beta_models[0].tenant_id, tenant_beta);
    assert_eq!(beta_models[0].model_version, model.model_version);
    assert_eq!(
        beta_models[0].model_artifact_hash.as_str(),
        "sha256:model-artifact-beta"
    );
    assert_eq!(beta_calibration_datasets.len(), 1);
    assert_eq!(beta_calibration_datasets[0].tenant_id, tenant_beta);
    assert_eq!(
        beta_calibration_datasets[0].source_manifest_hash.as_str(),
        "sha256:calibration-source-manifest-beta"
    );
    assert_eq!(beta_features.len(), 1);
    assert_eq!(beta_features[0].tenant_id, tenant_beta);
    assert_eq!(beta_features[0].ranking_feature_id, feature_id);
    assert_eq!(
        beta_features[0].feature_vector_hash.as_str(),
        "sha256:feature-vector-beta"
    );
    assert_eq!(beta_predictions.len(), 1);
    assert_eq!(beta_predictions[0].tenant_id, tenant_beta);
    assert_eq!(beta_predictions[0].ranking_prediction_id, prediction_id);
    assert_eq!(beta_predictions[0].settlement_score_micros, 1_775_000);
    assert_eq!(beta_labels.len(), 1);
    assert_eq!(beta_labels[0].tenant_id, tenant_beta);
    assert_eq!(beta_labels[0].ranking_label_id, label.ranking_label_id);
    assert_eq!(beta_labels[0].utility_delta_micros, 1_900_000);
    assert_eq!(beta_preference_labels.len(), 1);
    assert_eq!(beta_preference_labels[0].tenant_id, tenant_beta);
    assert_eq!(
        beta_preference_labels[0].preference_label_id,
        preference_label.preference_label_id
    );
    assert_eq!(
        beta_preference_labels[0].preference_strength_micros,
        650_000
    );
    assert_eq!(beta_calibration_runs.len(), 1);
    assert_eq!(beta_calibration_runs[0].tenant_id, tenant_beta);
    assert_eq!(
        beta_calibration_runs[0].calibration_run_id,
        calibration_run_id
    );
    assert_eq!(
        beta_calibration_runs[0].report_hash.as_str(),
        "sha256:ranking-calibration-report-beta"
    );
    assert_eq!(beta_worker_runs.len(), 1);
    assert_eq!(beta_worker_runs[0].tenant_id, tenant_beta);
    assert_eq!(beta_worker_runs[0].ranking_worker_run_id, worker_run_id);
    assert_eq!(
        beta_worker_runs[0].status,
        TraceRankingWorkerRunStatus::Running,
        "ranking worker-run status must stay tenant scoped even when ids overlap"
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
    let attestation_id = Uuid::new_v4();
    let hold_id = Uuid::new_v4();
    let settlement_batch_id = Uuid::new_v4();
    let near_outbox_id = Uuid::new_v4();
    let ranking_calibration_run_id = Uuid::new_v4();
    let tenant_cases = [
        (
            &tenant_alpha,
            "alpha",
            "principal:settlement-alpha-account",
            "sha256:settlement-alpha-account",
            "sha256:settlement-alpha-attestation-evidence",
            "sha256:settlement-alpha-attestation-ref",
            "sha256:settlement-alpha-hold-reason",
            "sha256:settlement-alpha-reason",
            "sha256:settlement-alpha-source-list",
            "sha256:settlement-alpha-near-call",
            1_250_000,
        ),
        (
            &tenant_beta,
            "beta",
            "principal:settlement-beta-account",
            "sha256:settlement-beta-account",
            "sha256:settlement-beta-attestation-evidence",
            "sha256:settlement-beta-attestation-ref",
            "sha256:settlement-beta-hold-reason",
            "sha256:settlement-beta-reason",
            "sha256:settlement-beta-source-list",
            "sha256:settlement-beta-near-call",
            2_500_000,
        ),
    ];

    for (
        tenant_id,
        label,
        account_ref,
        account_hash,
        evidence_hash,
        external_ref_hash,
        hold_reason_hash,
        settlement_reason_hash,
        source_list_hash,
        idempotency_key,
        settled_micros,
    ) in tenant_cases
    {
        backend
            .append_trace_credit_event(TraceCreditEventWrite {
                credit_event_id,
                tenant_id: tenant_id.clone(),
                submission_id,
                trace_id,
                credit_account_ref: account_ref.to_string(),
                event_type: TraceCreditEventType::RankingUtility,
                points_delta: "1.250000".to_string(),
                reason: format!("ranking settlement control-plane test {label}"),
                external_ref: Some(format!("ranker:settlement-control-plane:{label}")),
                actor_principal_ref: "principal:ranker-worker".to_string(),
                actor_role: "utility_worker".to_string(),
                settlement_state: TraceCreditSettlementState::Pending,
            })
            .await
            .expect("insert tenant settlement source credit event");

        let attestation = backend
            .upsert_trace_utility_attestation(TraceUtilityAttestationWrite {
                tenant_id: tenant_id.clone(),
                attestation_id,
                event_type: TraceCreditEventType::RankingUtility,
                use_category: "ranking".to_string(),
                policy_version: "trace-credit-policy-v3".to_string(),
                evidence_hash: evidence_hash.to_string(),
                external_ref_hash: external_ref_hash.to_string(),
                source_submission_ids: vec![submission_id],
                actor_principal_ref: "principal:ranker-worker".to_string(),
            })
            .await
            .expect("upsert tenant utility attestation");
        assert_eq!(attestation.tenant_id, *tenant_id);
        assert_eq!(attestation.attestation_id, attestation_id);
        assert_eq!(attestation.evidence_hash, evidence_hash);

        let hold = backend
            .upsert_trace_credit_hold(TraceCreditHoldWrite {
                tenant_id: tenant_id.clone(),
                hold_id,
                credit_account_ref: account_ref.to_string(),
                credit_account_hash: account_hash.to_string(),
                reason: TraceCreditHoldReason::AttestationDispute,
                reason_hash: hold_reason_hash.to_string(),
                actor_principal_ref: "principal:admin".to_string(),
                released_at: None,
            })
            .await
            .expect("upsert tenant credit hold");
        assert_eq!(hold.tenant_id, *tenant_id);
        assert_eq!(hold.hold_id, hold_id);
        assert_eq!(hold.credit_account_hash, account_hash);

        let settlement = backend
            .upsert_trace_credit_settlement_batch(TraceCreditSettlementBatchWrite {
                tenant_id: tenant_id.clone(),
                settlement_batch_id,
                policy_version: "trace-credit-policy-v3".to_string(),
                status: TraceCreditSettlementBatchStatus::Finalized,
                reason_hash: settlement_reason_hash.to_string(),
                source_credit_event_ids: vec![credit_event_id],
                source_submission_ids: vec![submission_id],
                source_list_hash: source_list_hash.to_string(),
                settled_credit_points: "1.250000".to_string(),
                settled_credit_micros: settled_micros,
                line_items: vec![TraceCreditAccountSettlementLineItem {
                    credit_account_ref: account_ref.to_string(),
                    credit_account_hash: account_hash.to_string(),
                    settled_credit_delta_micros: settled_micros,
                    source_credit_event_ids: vec![credit_event_id],
                    source_submission_ids: vec![submission_id],
                    source_list_hash: source_list_hash.to_string(),
                    near_status: TraceCreditSettlementNearStatus::Pending,
                    near_outbox_id: Some(near_outbox_id),
                }],
                near_contract_id: Some("trace-credits.testnet".to_string()),
                ranking_model_version: Some("trace-ranker-settlement-v3".to_string()),
                ranking_target_use: Some("ranking_model_training".to_string()),
                ranking_calibration_run_id: Some(ranking_calibration_run_id),
                ranking_calibration_report_hash: Some(format!(
                    "sha256:settlement-{label}-calibration-report"
                )),
                ranking_calibration_joined_evidence_hash: Some(format!(
                    "sha256:settlement-{label}-calibration-joined-evidence"
                )),
                ranking_credit_events_excluded_count: 0,
                ranking_credit_events_excluded_reason_counts: BTreeMap::from([(
                    format!("missing_prediction_ref_{label}"),
                    1,
                )]),
                actor_principal_ref: "principal:admin".to_string(),
            })
            .await
            .expect("upsert tenant settlement batch");
        assert_eq!(settlement.tenant_id, *tenant_id);
        assert_eq!(settlement.settlement_batch_id, settlement_batch_id);
        assert_eq!(settlement.source_list_hash, source_list_hash);
        assert_eq!(settlement.line_items.len(), 1);
        assert_eq!(settlement.line_items[0].credit_account_hash, account_hash);
        assert_eq!(
            settlement.ranking_model_version.as_deref(),
            Some("trace-ranker-settlement-v3")
        );
        assert_eq!(
            settlement
                .ranking_calibration_joined_evidence_hash
                .as_deref(),
            Some(format!("sha256:settlement-{label}-calibration-joined-evidence").as_str())
        );
        assert_eq!(
            settlement
                .ranking_credit_events_excluded_reason_counts
                .get(format!("missing_prediction_ref_{label}").as_str()),
            Some(&1)
        );

        let near_item = backend
            .upsert_trace_near_credit_outbox_item(TraceNearCreditOutboxItemWrite {
                tenant_id: tenant_id.clone(),
                near_outbox_id,
                settlement_batch_id,
                credit_account_hash: account_hash.to_string(),
                near_call_json: serde_json::json!({
                    "contract_id": "trace-credits.testnet",
                    "method_name": "settle_credit_receipt",
                    "args": {
                        "settlement_batch_id": settlement_batch_id,
                        "credit_account_hash": account_hash
                    },
                    "idempotency_key": idempotency_key
                }),
                status: TraceCreditSettlementNearStatus::Pending,
            })
            .await
            .expect("upsert tenant NEAR outbox item");
        assert_eq!(near_item.tenant_id, *tenant_id);
        assert_eq!(near_item.near_outbox_id, near_outbox_id);
        assert_eq!(near_item.credit_account_hash, account_hash);
        assert_eq!(near_item.status, TraceCreditSettlementNearStatus::Pending);
    }

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

    let alpha_events = backend
        .list_trace_credit_events(&tenant_alpha)
        .await
        .expect("list alpha credit events");
    assert_eq!(alpha_events.len(), 1);
    assert_eq!(alpha_events[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_events[0].credit_event_id, credit_event_id);
    assert_eq!(
        alpha_events[0].credit_account_ref,
        "principal:settlement-alpha-account"
    );

    let beta_events = backend
        .list_trace_credit_events(&tenant_beta)
        .await
        .expect("list beta credit events");
    assert_eq!(beta_events.len(), 1);
    assert_eq!(beta_events[0].tenant_id, tenant_beta);
    assert_eq!(beta_events[0].credit_event_id, credit_event_id);
    assert_eq!(
        beta_events[0].credit_account_ref,
        "principal:settlement-beta-account"
    );

    let alpha_attestations = backend
        .list_trace_utility_attestations(&tenant_alpha)
        .await
        .expect("list alpha attestations");
    assert_eq!(alpha_attestations.len(), 1);
    assert_eq!(alpha_attestations[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_attestations[0].attestation_id, attestation_id);
    assert_eq!(
        alpha_attestations[0].evidence_hash,
        "sha256:settlement-alpha-attestation-evidence"
    );

    let beta_attestations = backend
        .list_trace_utility_attestations(&tenant_beta)
        .await
        .expect("list beta attestations");
    assert_eq!(beta_attestations.len(), 1);
    assert_eq!(beta_attestations[0].tenant_id, tenant_beta);
    assert_eq!(beta_attestations[0].attestation_id, attestation_id);
    assert_eq!(
        beta_attestations[0].evidence_hash,
        "sha256:settlement-beta-attestation-evidence"
    );

    let alpha_holds = backend
        .list_trace_credit_holds(&tenant_alpha)
        .await
        .expect("list alpha holds");
    assert_eq!(alpha_holds.len(), 1);
    assert_eq!(alpha_holds[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_holds[0].hold_id, hold_id);
    assert_eq!(
        alpha_holds[0].credit_account_hash,
        "sha256:settlement-alpha-account"
    );

    let beta_holds = backend
        .list_trace_credit_holds(&tenant_beta)
        .await
        .expect("list beta holds");
    assert_eq!(beta_holds.len(), 1);
    assert_eq!(beta_holds[0].tenant_id, tenant_beta);
    assert_eq!(beta_holds[0].hold_id, hold_id);
    assert_eq!(
        beta_holds[0].credit_account_hash,
        "sha256:settlement-beta-account"
    );

    let alpha_settlements = backend
        .list_trace_credit_settlement_batches(&tenant_alpha)
        .await
        .expect("list alpha settlement batches");
    assert_eq!(alpha_settlements.len(), 1);
    assert_eq!(alpha_settlements[0].tenant_id, tenant_alpha);
    assert_eq!(
        alpha_settlements[0].settlement_batch_id,
        settlement_batch_id
    );
    assert_eq!(
        alpha_settlements[0].source_list_hash,
        "sha256:settlement-alpha-source-list"
    );
    assert_eq!(
        alpha_settlements[0].line_items[0].credit_account_hash,
        "sha256:settlement-alpha-account"
    );

    let beta_settlements = backend
        .list_trace_credit_settlement_batches(&tenant_beta)
        .await
        .expect("list beta settlement batches");
    assert_eq!(beta_settlements.len(), 1);
    assert_eq!(beta_settlements[0].tenant_id, tenant_beta);
    assert_eq!(beta_settlements[0].settlement_batch_id, settlement_batch_id);
    assert_eq!(
        beta_settlements[0].source_list_hash,
        "sha256:settlement-beta-source-list"
    );
    assert_eq!(
        beta_settlements[0].line_items[0].credit_account_hash,
        "sha256:settlement-beta-account"
    );

    let alpha_near = backend
        .list_trace_near_credit_outbox_items(&tenant_alpha)
        .await
        .expect("list alpha NEAR outbox");
    assert_eq!(alpha_near.len(), 1);
    assert_eq!(alpha_near[0].tenant_id, tenant_alpha);
    assert_eq!(alpha_near[0].near_outbox_id, near_outbox_id);
    assert_eq!(
        alpha_near[0].status,
        TraceCreditSettlementNearStatus::Submitted
    );
    assert_eq!(
        alpha_near[0].near_transaction_hash.as_deref(),
        Some("near-public-tx-hash")
    );
    assert_eq!(
        alpha_near[0].near_call_json["idempotency_key"].as_str(),
        Some("sha256:settlement-alpha-near-call")
    );

    let beta_near = backend
        .list_trace_near_credit_outbox_items(&tenant_beta)
        .await
        .expect("list beta NEAR outbox");
    assert_eq!(beta_near.len(), 1);
    assert_eq!(beta_near[0].tenant_id, tenant_beta);
    assert_eq!(beta_near[0].near_outbox_id, near_outbox_id);
    assert_eq!(
        beta_near[0].status,
        TraceCreditSettlementNearStatus::Pending
    );
    assert_eq!(beta_near[0].near_transaction_hash, None);
    assert_eq!(
        beta_near[0].near_call_json["idempotency_key"].as_str(),
        Some("sha256:settlement-beta-near-call")
    );

    cleanup_tenant(&backend, &tenant_alpha).await;
    cleanup_tenant(&backend, &tenant_beta).await;
}
