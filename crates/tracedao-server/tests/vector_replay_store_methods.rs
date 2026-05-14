//! PG-backed tests for the new `TraceCorpusStore` methods added for the
//! `tracedao-vector-replay` recovery binary:
//!
//! - `stream_trace_gate_decisions_for_replay`
//! - `is_vector_entry_revoked`
//!
//! These tests opt in via `TRACEDAO_PG_TEST_DATABASE_URL` (or `DATABASE_URL`).
//! When neither is set, every test early-returns silently — matching the
//! pattern used by the other `crates/tracedao-server/tests/*_pg_*.rs` suites
//! so hermetic `cargo test` runs stay green on hosts without PostgreSQL.

use std::collections::BTreeMap;

use chrono::{Duration, Utc};
use secrecy::SecretString;
use tracedao_server::config::{DatabaseConfig, SslMode};
use tracedao_server::db::{Database, postgres::PgBackend};
use tracedao_server::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceGateDecisionRow, TraceRevocationPropagationAction,
    TraceRevocationPropagationItemStatus, TraceRevocationPropagationItemWrite,
    TraceRevocationPropagationTarget, TraceSubmissionWrite,
};
use uuid::Uuid;

fn postgres_test_config() -> Option<DatabaseConfig> {
    let url = std::env::var("TRACEDAO_PG_TEST_DATABASE_URL")
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
        eprintln!("skipping: TRACEDAO_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
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
        allowed_uses: vec!["training_allowed".to_string()],
        retention_policy_id: "default".to_string(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".to_string(),
        redaction_pipeline_version: "ironclaw-redaction.v1".to_string(),
        redaction_counts: BTreeMap::new(),
        redaction_hash: "sha256:fixture-redaction".to_string(),
        canonical_summary_hash: Some("sha256:fixture-summary".to_string()),
        submission_score: None,
        credit_points_pending: None,
        credit_points_final: None,
        expires_at: None,
    }
}

fn sample_gate_decision(
    submission_id: Uuid,
    decided_at_offset_seconds: i64,
    with_vector_entry: bool,
) -> TraceGateDecisionRow {
    TraceGateDecisionRow {
        decision_id: Uuid::new_v4(),
        submission_id,
        gate_policy_version: "vector_replay_test_v1".to_string(),
        gate_version_hash: "sha256:vector_replay_test_v1".to_string(),
        perplexity_micros: 1_500_000,
        tail_fraction_micros: 750_000,
        perplexity_passed: true,
        novelty_score_micros: 900_000,
        nearest_neighbor_hash: "sha256:fixture-neighbor".to_string(),
        novelty_passed: true,
        embedding_evidence_hash: "sha256:fixture-evidence".to_string(),
        attestation_chain_hash: "sha256:fixture-attestation".to_string(),
        decided_at: Utc::now() + Duration::seconds(decided_at_offset_seconds),
        vector_entry_id: if with_vector_entry {
            Some(Uuid::new_v4())
        } else {
            None
        },
        credit_withheld_reason: None,
    }
}

#[tokio::test]
async fn stream_trace_gate_decisions_for_replay_filters_null_vector_entries_and_pages() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_alpha = format!("pg-replay-stream-{}", Uuid::new_v4());
    let tenant_beta = format!("pg-replay-stream-other-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let other_submission_id = Uuid::new_v4();
    for tenant_id in [&tenant_alpha, &tenant_beta] {
        backend
            .upsert_trace_submission(sample_submission(tenant_id, submission_id))
            .await
            .expect("insert submission alpha");
        backend
            .upsert_trace_submission(sample_submission(tenant_id, other_submission_id))
            .await
            .expect("insert submission beta");
    }

    // Three with vector_entry_id (replay candidates), two without (should be
    // filtered out by the query), one row under the other tenant (must not
    // leak across tenant scope).
    let mut alpha_with_entry: Vec<TraceGateDecisionRow> = Vec::new();
    for i in 0..3 {
        let d = sample_gate_decision(submission_id, i * 60, true);
        alpha_with_entry.push(d.clone());
        backend
            .insert_trace_gate_decision(&tenant_alpha, d)
            .await
            .expect("insert with vector_entry_id");
    }
    for i in 0..2 {
        let d = sample_gate_decision(submission_id, 1000 + i * 60, false);
        backend
            .insert_trace_gate_decision(&tenant_alpha, d)
            .await
            .expect("insert without vector_entry_id");
    }
    let beta_decision = sample_gate_decision(submission_id, 30, true);
    backend
        .insert_trace_gate_decision(&tenant_beta, beta_decision.clone())
        .await
        .expect("insert tenant_beta gate decision");

    // First page: page_size=2 should return the two earliest replay
    // candidates (decided_at ASC), and NO null-entry rows.
    let page1 = backend
        .stream_trace_gate_decisions_for_replay(&tenant_alpha, 2, None)
        .await
        .expect("page1");
    assert_eq!(page1.len(), 2);
    assert!(page1.iter().all(|r| r.vector_entry_id.is_some()));
    assert!(page1[0].decided_at <= page1[1].decided_at);

    let cursor = Some((page1[1].decided_at, page1[1].decision_id));
    let page2 = backend
        .stream_trace_gate_decisions_for_replay(&tenant_alpha, 2, cursor)
        .await
        .expect("page2");
    assert_eq!(page2.len(), 1);
    assert!(page2[0].vector_entry_id.is_some());
    assert!(page2[0].decided_at >= page1[1].decided_at);

    // page3 must be empty (no more rows after the cursor).
    let cursor = Some((page2[0].decided_at, page2[0].decision_id));
    let page3 = backend
        .stream_trace_gate_decisions_for_replay(&tenant_alpha, 2, cursor)
        .await
        .expect("page3");
    assert!(page3.is_empty());

    // No tenant_beta rows must appear in the tenant_alpha stream.
    let all_alpha = backend
        .stream_trace_gate_decisions_for_replay(&tenant_alpha, 100, None)
        .await
        .expect("all alpha");
    assert_eq!(
        all_alpha.len(),
        3,
        "exactly the three with-vector-entry alpha rows"
    );
    assert!(
        all_alpha
            .iter()
            .all(|r| r.decision_id != beta_decision.decision_id)
    );
}

#[tokio::test]
async fn is_vector_entry_revoked_returns_true_only_for_done_invalidate_vector() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_id = format!("pg-replay-revoked-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert submission");

    let entry_done = Uuid::new_v4();
    let entry_pending = Uuid::new_v4();
    let entry_other_kind = Uuid::new_v4();
    let entry_never = Uuid::new_v4();

    // Done VectorEntry/InvalidateVector row — must be reported revoked.
    backend
        .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
            tenant_id: tenant_id.clone(),
            propagation_item_id: Uuid::new_v4(),
            source_submission_id: submission_id,
            target: TraceRevocationPropagationTarget::VectorEntry {
                vector_entry_id: entry_done,
            },
            action: TraceRevocationPropagationAction::InvalidateVector,
            status: TraceRevocationPropagationItemStatus::Done,
            idempotency_key: format!("idem-done-{}", entry_done),
            reason: "test:done".to_string(),
            attempt_count: 1,
            last_error: None,
            next_attempt_at: None,
            completed_at: Some(Utc::now()),
            evidence_hash: Some("sha256:done".to_string()),
            metadata: BTreeMap::new(),
        })
        .await
        .expect("insert done");

    // Pending VectorEntry/InvalidateVector row — must NOT be reported as
    // already-revoked (replay can still proceed; the worker hasn't run).
    backend
        .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
            tenant_id: tenant_id.clone(),
            propagation_item_id: Uuid::new_v4(),
            source_submission_id: submission_id,
            target: TraceRevocationPropagationTarget::VectorEntry {
                vector_entry_id: entry_pending,
            },
            action: TraceRevocationPropagationAction::InvalidateVector,
            status: TraceRevocationPropagationItemStatus::Pending,
            idempotency_key: format!("idem-pending-{}", entry_pending),
            reason: "test:pending".to_string(),
            attempt_count: 0,
            last_error: None,
            next_attempt_at: None,
            completed_at: None,
            evidence_hash: None,
            metadata: BTreeMap::new(),
        })
        .await
        .expect("insert pending");

    // Done row of a different target_kind that happens to share the same
    // entry id — must NOT match the VectorEntry-specific query.
    backend
        .upsert_trace_revocation_propagation_item(TraceRevocationPropagationItemWrite {
            tenant_id: tenant_id.clone(),
            propagation_item_id: Uuid::new_v4(),
            source_submission_id: submission_id,
            target: TraceRevocationPropagationTarget::DerivedRecord {
                derived_id: entry_other_kind,
            },
            action: TraceRevocationPropagationAction::InvalidateMetadata,
            status: TraceRevocationPropagationItemStatus::Done,
            idempotency_key: format!("idem-other-{}", entry_other_kind),
            reason: "test:other-kind".to_string(),
            attempt_count: 1,
            last_error: None,
            next_attempt_at: None,
            completed_at: Some(Utc::now()),
            evidence_hash: Some("sha256:other".to_string()),
            metadata: BTreeMap::new(),
        })
        .await
        .expect("insert other kind");

    assert!(
        backend
            .is_vector_entry_revoked(&tenant_id, entry_done)
            .await
            .expect("done")
    );
    assert!(
        !backend
            .is_vector_entry_revoked(&tenant_id, entry_pending)
            .await
            .expect("pending")
    );
    assert!(
        !backend
            .is_vector_entry_revoked(&tenant_id, entry_other_kind)
            .await
            .expect("other kind")
    );
    assert!(
        !backend
            .is_vector_entry_revoked(&tenant_id, entry_never)
            .await
            .expect("never recorded")
    );

    // Cross-tenant: a Done row for entry_done under tenant_id must NOT be
    // visible from a different tenant context.
    let other_tenant_id = format!("pg-replay-revoked-other-{}", Uuid::new_v4());
    backend
        .upsert_trace_submission(sample_submission(&other_tenant_id, submission_id))
        .await
        .expect("insert other-tenant submission");
    assert!(
        !backend
            .is_vector_entry_revoked(&other_tenant_id, entry_done)
            .await
            .expect("cross-tenant")
    );
}
