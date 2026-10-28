// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Adding the `model_training` use to traces already sent (V141): the
//! eligibility read, the add-only grant, and the withheld-award lookup, on a
//! real PostgreSQL, as the least-privilege ingest runtime role.

use chrono::{Duration, Utc};
use std::collections::BTreeMap;
use trace_commons_protocol::model_training_consent::ModelTrainingConsentOutcome;
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, ModelTrainingConsentGrant, postgres::PgBackend},
    trace_corpus_storage::{
        TraceCorpusStatus, TraceCorpusStore, TraceGateDecisionRow, TraceSubmissionWrite,
    },
};
use uuid::Uuid;

fn submission(tenant: &str, principal: &str, status: TraceCorpusStatus) -> TraceSubmissionWrite {
    TraceSubmissionWrite {
        tenant_id: tenant.into(),
        submission_id: Uuid::new_v4(),
        trace_id: Uuid::new_v4(),
        auth_principal_ref: principal.into(),
        contributor_pseudonym: None,
        submitted_tenant_scope_ref: None,
        schema_version: "ironclaw.trace_contribution.v1".into(),
        consent_policy_version: "test".into(),
        consent_scopes: vec!["debugging_evaluation".into(), "ranking_training".into()],
        allowed_uses: vec![
            "debugging".into(),
            "evaluation".into(),
            "ranking_model_training".into(),
            "aggregate_analytics".into(),
        ],
        retention_policy_id: "private_corpus_revocable".into(),
        status,
        privacy_risk: "low".into(),
        redaction_pipeline_version: "test".into(),
        redaction_counts: BTreeMap::new(),
        redaction_hash: "sha256:test".into(),
        canonical_summary_hash: None,
        submission_score: None,
        credit_points_pending: None,
        credit_points_final: None,
        expires_at: None,
        residual_risk_basis: None,
    }
}

fn gate_decision(
    submission_id: Uuid,
    passed: bool,
    withheld: Option<&str>,
    decided_at: chrono::DateTime<Utc>,
) -> TraceGateDecisionRow {
    TraceGateDecisionRow {
        decision_id: Uuid::new_v4(),
        submission_id,
        gate_policy_version: "enclave_near_ai_v1".into(),
        gate_version_hash: "sha256:gate-version".into(),
        perplexity_micros: 1_500_000,
        tail_fraction_micros: 750_000,
        perplexity_passed: passed,
        novelty_score_micros: 900_000,
        nearest_neighbor_hash: "sha256:neighbor".into(),
        novelty_passed: true,
        embedding_evidence_hash: "sha256:evidence".into(),
        attestation_chain_hash: "sha256:attestation".into(),
        decided_at,
        vector_entry_id: Some(Uuid::new_v4()),
        credit_withheld_reason: withheld.map(str::to_string),
        peak_perplexity_micros: None,
        peak_novelty_micros: None,
        chunk_count: None,
        total_chunk_count: None,
        qualifying_token_fraction_micros: None,
        agent_prose_perplexity_micros: None,
        agent_prose_tokens: None,
        tool_result_perplexity_micros: None,
        tool_result_tokens: None,
        attributed_token_fraction_micros: None,
        chunks_capped: Some(false),
        composite_score_micros: None,
        vector_index_snapshot_id: None,
        index_cardinality_at_scoring: None,
    }
}

/// The migrator's backend (to set fixtures up) and the ingest runtime's (to
/// exercise the methods under V141's grants, with forced RLS).
async fn backends() -> Option<(PgBackend, PgBackend)> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).unwrap();
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
        .await
        .unwrap();
    admin.run_migrations().await.unwrap();
    // V90: the runtime's pre-V62 table privileges are the operator's, not a
    // migration's. Model only the reads the eligibility and award lookups
    // need on those older tables; V141's own grants cover the rest.
    admin
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(
            "GRANT SELECT ON trace_submissions, trace_withdrawals, trace_submission_sessions,
                    trace_source_sessions, trace_gate_decisions, trace_tenants
               TO trace_ingest_runtime",
        )
        .await
        .unwrap();
    let mut runtime_url = parsed.clone();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
        .await
        .unwrap();
    let row = runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert!(!row.get::<_, bool>(0) && !row.get::<_, bool>(1));
    Some((admin, runtime))
}

async fn stored_consent(
    admin: &PgBackend,
    tenant: &str,
    submission_id: Uuid,
) -> (Vec<String>, Vec<String>) {
    let mut client = admin.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let row = tx
        .query_one(
            "SELECT consent_scopes, allowed_uses FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &submission_id],
        )
        .await
        .unwrap();
    let scopes: serde_json::Value = row.get(0);
    let uses: serde_json::Value = row.get(1);
    (
        serde_json::from_value(scopes).unwrap(),
        serde_json::from_value(uses).unwrap(),
    )
}

#[tokio::test]
async fn eligibility_lists_only_the_callers_live_accepted_traces_without_the_use() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("mtc-{}", Uuid::new_v4());
    let other_tenant = format!("mtc-{}", Uuid::new_v4());
    let owners = vec!["owned-a".to_string(), "owned-b".to_string()];
    let mut eligible = Vec::new();
    for principal in ["owned-a", "owned-b"] {
        eligible.push(
            admin
                .upsert_trace_submission(submission(
                    &tenant,
                    principal,
                    TraceCorpusStatus::Accepted,
                ))
                .await
                .unwrap()
                .submission_id,
        );
    }
    let mut with_use = submission(&tenant, "owned-a", TraceCorpusStatus::Accepted);
    with_use.consent_scopes.push("model_training".into());
    with_use.allowed_uses.push("model_training".into());
    admin.upsert_trace_submission(with_use).await.unwrap();
    admin
        .upsert_trace_submission(submission(&tenant, "foreign", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    admin
        .upsert_trace_submission(submission(
            &other_tenant,
            "owned-a",
            TraceCorpusStatus::Accepted,
        ))
        .await
        .unwrap();
    for status in [
        TraceCorpusStatus::Received,
        TraceCorpusStatus::Quarantined,
        TraceCorpusStatus::AwaitingPiiBackstop,
        TraceCorpusStatus::Rejected,
        TraceCorpusStatus::Revoked,
        TraceCorpusStatus::Expired,
        TraceCorpusStatus::Purged,
    ] {
        admin
            .upsert_trace_submission(submission(&tenant, "owned-a", status))
            .await
            .unwrap();
    }
    // Accepted, but past a lifecycle edge the status has not caught up with.
    let expired = admin
        .upsert_trace_submission(submission(&tenant, "owned-a", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    let withdrawn = admin
        .upsert_trace_submission(submission(&tenant, "owned-a", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    let source_withdrawn = admin
        .upsert_trace_submission(submission(&tenant, "owned-a", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    {
        let mut client = admin.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.execute(
            "UPDATE trace_submissions SET expires_at = now() - interval '1 second'
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &expired.submission_id],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_withdrawals
                 (tenant_id, submission_id, withdrawn_at, prior_status, distribution_reach)
             VALUES ($1, $2, now(), 'accepted', 'not_distributed')",
            &[&tenant, &withdrawn.submission_id],
        )
        .await
        .unwrap();
        let account = Uuid::new_v4();
        let digest = vec![9u8; 32];
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_source_sessions (tenant_id, account_id, session_digest, withdrawn_at)
             VALUES ($1, $2, $3, now())",
            &[&tenant, &account, &digest],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_submission_sessions (tenant_id, submission_id, account_id, session_digest)
             VALUES ($1, $2, $3, $4)",
            &[&tenant, &source_withdrawn.submission_id, &account, &digest],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    let listed = runtime
        .list_model_training_consent_eligible(&tenant, &owners, 500)
        .await
        .unwrap();
    assert_eq!(listed.eligible_count, 2);
    let mut ids = listed.submission_ids.clone();
    ids.sort();
    eligible.sort();
    assert_eq!(ids, eligible);

    // The count is every eligible trace; the list stops at the limit.
    let capped = runtime
        .list_model_training_consent_eligible(&tenant, &owners, 1)
        .await
        .unwrap();
    assert_eq!(capped.eligible_count, 2);
    assert_eq!(capped.submission_ids.len(), 1);

    // No owners, no traces: never "everyone's".
    let none = runtime
        .list_model_training_consent_eligible(&tenant, &[], 500)
        .await
        .unwrap();
    assert_eq!(none.eligible_count, 0);
    assert!(none.submission_ids.is_empty());
}

#[tokio::test]
async fn grant_adds_only_model_training_once_and_dates_it() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("mtc-{}", Uuid::new_v4());
    let owners = vec!["owned".to_string()];
    let mine = admin
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap()
        .submission_id;
    let foreign = admin
        .upsert_trace_submission(submission(&tenant, "foreign", TraceCorpusStatus::Accepted))
        .await
        .unwrap()
        .submission_id;
    let quarantined = admin
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Quarantined))
        .await
        .unwrap()
        .submission_id;
    let unknown = Uuid::new_v4();
    let before = stored_consent(&admin, &tenant, mine).await;

    let granted_at = Utc::now();
    let ids = [mine, foreign, quarantined, unknown];
    let rows = runtime
        .grant_model_training_consent(ModelTrainingConsentGrant {
            tenant_id: &tenant,
            owner_principal_refs: &owners,
            submission_ids: &ids,
            actor_principal_ref: "owned",
            granted_at,
        })
        .await
        .unwrap();
    let outcome = |id: Uuid| rows.iter().find(|row| row.submission_id == id).unwrap();
    assert_eq!(outcome(mine).outcome, ModelTrainingConsentOutcome::Granted);
    let recorded = outcome(mine).granted_at.expect("grant time recorded");
    assert!((recorded - granted_at).num_milliseconds().abs() < 1);
    for id in [foreign, quarantined, unknown] {
        assert_eq!(
            outcome(id).outcome,
            ModelTrainingConsentOutcome::NotEligible
        );
        assert_eq!(outcome(id).granted_at, None);
    }

    // Add-only: everything stored before is still there, in order, plus the
    // one scope and the one use, and nothing else.
    let after = stored_consent(&admin, &tenant, mine).await;
    let mut expected_scopes = before.0.clone();
    expected_scopes.push("model_training".into());
    let mut expected_uses = before.1.clone();
    expected_uses.push("model_training".into());
    assert_eq!(after, (expected_scopes, expected_uses));
    // The others are untouched.
    for id in [foreign, quarantined] {
        assert_eq!(stored_consent(&admin, &tenant, id).await, before);
    }

    // Idempotent: a repeat keeps the first date and adds nothing.
    let repeat = runtime
        .grant_model_training_consent(ModelTrainingConsentGrant {
            tenant_id: &tenant,
            owner_principal_refs: &owners,
            submission_ids: &[mine],
            actor_principal_ref: "owned",
            granted_at: granted_at + Duration::hours(1),
        })
        .await
        .unwrap();
    assert_eq!(
        repeat[0].outcome,
        ModelTrainingConsentOutcome::AlreadyGranted
    );
    assert_eq!(repeat[0].granted_at, Some(recorded));
    assert_eq!(
        stored_consent(&admin, &tenant, mine).await,
        after,
        "a repeat grant changes nothing"
    );
    assert!(
        runtime
            .list_model_training_consent_eligible(&tenant, &owners, 500)
            .await
            .unwrap()
            .submission_ids
            .is_empty()
    );

    // Forced RLS: without tenant context the runtime sees no grant row.
    let runtime_client = runtime.trace_pool_for_test().get().await.unwrap();
    let unscoped: i64 = runtime_client
        .query_one("SELECT count(*) FROM trace_consent_grants", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unscoped, 0,
        "the runtime sees no grant without tenant context"
    );
}

#[tokio::test]
async fn withheld_award_lookup_finds_only_a_latest_legacy_policy_mismatch() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("mtc-{}", Uuid::new_v4());
    let now = Utc::now();
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(
            admin
                .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
                .await
                .unwrap()
                .submission_id,
        );
    }
    // 0: withheld for lack of model_training -- the one to re-run.
    let withheld = gate_decision(ids[0], true, Some("policy_mismatch"), now);
    admin
        .insert_trace_gate_decision(&tenant, withheld.clone())
        .await
        .unwrap();
    // 1: failed a floor; nothing to re-run.
    admin
        .insert_trace_gate_decision(
            &tenant,
            gate_decision(ids[1], false, Some("policy_mismatch"), now),
        )
        .await
        .unwrap();
    // 2: withheld for another reason.
    admin
        .insert_trace_gate_decision(
            &tenant,
            gate_decision(ids[2], true, Some("central_issuer_denied"), now),
        )
        .await
        .unwrap();
    // 3: an older policy_mismatch superseded by a later decision.
    admin
        .insert_trace_gate_decision(
            &tenant,
            gate_decision(
                ids[3],
                true,
                Some("policy_mismatch"),
                now - Duration::minutes(5),
            ),
        )
        .await
        .unwrap();
    admin
        .insert_trace_gate_decision(&tenant, gate_decision(ids[3], true, None, now))
        .await
        .unwrap();
    // 4: never scored.

    let found = runtime
        .withheld_policy_mismatch_gate_award(&tenant, ids[0])
        .await
        .unwrap()
        .expect("the withheld award is found");
    assert_eq!(found.decision_id, withheld.decision_id);
    assert_eq!(found.gate_version_hash, withheld.gate_version_hash);
    assert_eq!(found.gate_policy_version, withheld.gate_policy_version);
    assert_eq!(found.vector_entry_id, withheld.vector_entry_id);
    assert!(!found.chunks_capped);
    for id in &ids[1..] {
        assert!(
            runtime
                .withheld_policy_mismatch_gate_award(&tenant, *id)
                .await
                .unwrap()
                .is_none()
        );
    }
    // Another tenant cannot see it.
    assert!(
        runtime
            .withheld_policy_mismatch_gate_award(&format!("mtc-{}", Uuid::new_v4()), ids[0])
            .await
            .unwrap()
            .is_none()
    );
}
