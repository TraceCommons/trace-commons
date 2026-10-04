// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::{Duration, Utc};
use std::collections::BTreeMap;
use trace_commons_protocol::activity_missions::Qualification;
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, postgres::PgBackend},
    trace_corpus_storage::{TraceCorpusStatus, TraceCorpusStore, TraceSubmissionWrite},
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
        consent_scopes: vec!["training_allowed".into()],
        allowed_uses: vec!["training".into()],
        retention_policy_id: "standard".into(),
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

#[tokio::test]
async fn activity_missions_uses_owned_contributions_and_removes_withdrawn_progress() {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return;
    };
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_"));
    let db = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
        .await
        .unwrap();
    db.run_migrations().await.unwrap();
    let tenant = format!("mission-{}", Uuid::new_v4());
    let other = format!("mission-{}", Uuid::new_v4());
    let accepted = db
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    db.upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Received))
        .await
        .unwrap();
    db.upsert_trace_submission(submission(&tenant, "foreign", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    db.upsert_trace_submission(submission(&other, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    for status in [
        TraceCorpusStatus::Quarantined,
        TraceCorpusStatus::AwaitingPiiBackstop,
        TraceCorpusStatus::Rejected,
        TraceCorpusStatus::Revoked,
        TraceCorpusStatus::Expired,
        TraceCorpusStatus::Purged,
    ] {
        db.upsert_trace_submission(submission(&tenant, "owned", status))
            .await
            .unwrap();
    }
    // Terminal timestamps and both withdrawal ledgers win even if a worker
    // has not yet changed an otherwise Accepted row's status.
    let expired = db
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    let purged = db
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    let withdrawn = db
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    let source_withdrawn = db
        .upsert_trace_submission(submission(&tenant, "owned", TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    {
        let mut client = db.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.execute("UPDATE trace_submissions SET expires_at=now()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2",&[&tenant,&expired.submission_id]).await.unwrap();
        tx.execute(
            "UPDATE trace_submissions SET purged_at=now() WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &purged.submission_id],
        )
        .await
        .unwrap();
        tx.execute("INSERT INTO trace_withdrawals (tenant_id,submission_id,withdrawn_at,prior_status,distribution_reach) VALUES ($1,$2,now(),'accepted','not_distributed')",&[&tenant,&withdrawn.submission_id]).await.unwrap();
        let account = Uuid::new_v4();
        let digest = vec![7u8; 32];
        tx.execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES ($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
        tx.execute("INSERT INTO trace_source_sessions(tenant_id,account_id,session_digest,withdrawn_at) VALUES ($1,$2,$3,now())",&[&tenant,&account,&digest]).await.unwrap();
        tx.execute("INSERT INTO trace_submission_sessions(tenant_id,submission_id,account_id,session_digest) VALUES ($1,$2,$3,$4)",&[&tenant,&source_withdrawn.submission_id,&account,&digest]).await.unwrap();
        tx.commit().await.unwrap();
    }
    // Startup SET ROLE makes every pool connection the actual narrow runtime
    // role, without altering shared cluster roles. Assert that role really
    // has neither superuser nor BYPASSRLS privileges.
    // V90 documents that pre-V62 runtime table privileges are provisioned
    // by the operator, not migrations. Model the existing account-history
    // SELECT permissions, only in this fresh test database.
    db.trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(
            "GRANT SELECT ON trace_submissions, trace_withdrawals TO trace_ingest_runtime",
        )
        .await
        .unwrap();
    let mut runtime_url = parsed.clone();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let reader = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
        .await
        .unwrap();
    {
        let client = reader.trace_pool_for_test().get().await.unwrap();
        let row = client
            .query_one(
                "SELECT rolsuper,rolbypassrls FROM pg_roles WHERE rolname=current_user",
                &[],
            )
            .await
            .unwrap();
        assert!(!row.get::<_, bool>(0));
        assert!(!row.get::<_, bool>(1));
        let row = client
            .query_one("SELECT count(*) FROM trace_submissions", &[])
            .await
            .unwrap();
        assert_eq!(
            row.get::<_, i64>(0),
            0,
            "runtime sees no rows without tenant context"
        );
    }
    let now = Utc::now();
    let from = now - Duration::days(1);
    let owners = vec!["owned".into()];
    let accepted_days = reader
        .account_activity_days(&tenant, &owners, from, now, Qualification::Accepted)
        .await
        .unwrap();
    assert_eq!(
        accepted_days
            .iter()
            .map(|day| day.contributions)
            .sum::<u64>(),
        1
    );
    let received = reader
        .account_activity_days(
            &tenant,
            &owners,
            from,
            now,
            Qualification::ReceivedOrAccepted,
        )
        .await
        .unwrap();
    assert_eq!(received.iter().map(|day| day.contributions).sum::<u64>(), 2);
    assert!(
        reader
            .account_activity_days(&tenant, &[], from, now, Qualification::Accepted)
            .await
            .unwrap()
            .is_empty()
    );
    let mut client = db.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE trace_submissions SET revoked_at=now() WHERE tenant_id=$1 AND submission_id=$2",
        &[&tenant, &accepted.submission_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(
        reader
            .account_activity_days(&tenant, &owners, from, Utc::now(), Qualification::Accepted)
            .await
            .unwrap()
            .is_empty()
    );
}
