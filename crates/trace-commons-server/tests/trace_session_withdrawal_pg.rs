// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use secrecy::SecretString;
use tokio_postgres::NoTls;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceSourceSessionStatus, TraceSubmissionWrite,
};
use uuid::Uuid;

fn database_url() -> Option<String> {
    std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
}

fn database_config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url.to_string()),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn migrated_client(url: &str) -> tokio_postgres::Client {
    let config = database_config(url);
    let backend = PgBackend::new(&config)
        .await
        .expect("connect to PostgreSQL");
    backend.run_migrations().await.expect("apply migrations");
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("direct PostgreSQL connection");
    tokio::spawn(async move {
        connection.await.expect("PostgreSQL connection");
    });
    client
}

fn submission(tenant: &str, id: Uuid, status: TraceCorpusStatus) -> TraceSubmissionWrite {
    TraceSubmissionWrite {
        tenant_id: tenant.into(),
        submission_id: id,
        trace_id: Uuid::new_v4(),
        auth_principal_ref: "principal:z4".into(),
        contributor_pseudonym: None,
        submitted_tenant_scope_ref: None,
        schema_version: "ironclaw.trace_contribution.v1".into(),
        consent_policy_version: "2026-04-24".into(),
        consent_scopes: vec!["debugging".into()],
        allowed_uses: vec!["debugging".into()],
        retention_policy_id: "standard".into(),
        status,
        privacy_risk: "low".into(),
        redaction_pipeline_version: "deterministic-v1".into(),
        redaction_counts: BTreeMap::new(),
        redaction_hash: "sha256:z4-test".into(),
        canonical_summary_hash: None,
        submission_score: None,
        credit_points_pending: None,
        credit_points_final: None,
        expires_at: None,
        residual_risk_basis: None,
    }
}

#[tokio::test]
async fn session_tables_have_forced_rls_and_mapping_outlives_content() {
    let Some(url) = database_url() else {
        eprintln!("skipping: PostgreSQL test URL not configured");
        return;
    };
    let client = migrated_client(&url).await;
    for table in ["trace_source_sessions", "trace_submission_sessions"] {
        let row = client
            .query_one(
                "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE oid = to_regclass($1::text)",
                &[&table],
            )
            .await
            .expect("table in migration registry");
        assert!(row.get::<_, bool>(0), "RLS enabled for {table}");
        assert!(row.get::<_, bool>(1), "RLS forced for {table}");
    }

    let tenant = format!("z4-{}", Uuid::new_v4());
    let account = Uuid::new_v4();
    let submission = Uuid::new_v4();
    let digest = vec![0x5au8; 32];
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_source_sessions (tenant_id, account_id, session_digest) VALUES ($1, $2, $3)",
            &[&tenant, &account, &digest],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_submission_sessions (tenant_id, submission_id, account_id, session_digest) VALUES ($1, $2, $3, $4)",
            &[&tenant, &submission, &account, &digest],
        )
        .await
        .expect("mapping needs no content row");

    client
        .batch_execute(
            "DO $$ BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_z4_rls_probe') THEN
                    CREATE ROLE trace_z4_rls_probe NOLOGIN NOBYPASSRLS;
                END IF;
             END $$;
             GRANT USAGE ON SCHEMA public TO trace_z4_rls_probe;
             GRANT SELECT ON trace_source_sessions, trace_submission_sessions TO trace_z4_rls_probe;
             BEGIN;
             SET ROLE trace_z4_rls_probe;",
        )
        .await
        .expect("use a non-bypass role for the RLS assertion");
    let invisible = client
        .query_one("SELECT count(*) FROM trace_source_sessions", &[])
        .await
        .unwrap();
    assert_eq!(
        invisible.get::<_, i64>(0),
        0,
        "raw connection has no tenant context"
    );
    client
        .query_one(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
    let visible = client
        .query_one("SELECT count(*) FROM trace_source_sessions", &[])
        .await
        .unwrap();
    assert_eq!(
        visible.get::<_, i64>(0),
        1,
        "tenant-scoped transaction sees its row"
    );
    client.batch_execute("ROLLBACK").await.unwrap();

    let retargeted = client
        .execute(
            "UPDATE trace_submission_sessions SET account_id = account_id WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &submission],
        )
        .await;
    assert!(
        retargeted.is_err(),
        "even a no-op update cannot retarget a mapping"
    );
    client
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .unwrap();
}

#[tokio::test]
async fn all_accepted_status_writers_refuse_a_withdrawn_mapped_session() {
    let Some(url) = database_url() else { return };
    let client = migrated_client(&url).await;
    let backend = PgBackend::new(&database_config(&url)).await.unwrap();
    let tenant = format!("z4-{}", Uuid::new_v4());
    let account = Uuid::new_v4();
    let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let digest = vec![0x7bu8; 32];
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    for (id, status) in ids.into_iter().zip([
        TraceCorpusStatus::Quarantined,
        TraceCorpusStatus::Quarantined,
        TraceCorpusStatus::AwaitingPiiBackstop,
    ]) {
        backend
            .upsert_trace_submission(submission(&tenant, id, status))
            .await
            .unwrap();
    }
    client.execute("INSERT INTO trace_source_sessions (tenant_id, account_id, session_digest, withdrawn_at) VALUES ($1, $2, $3, NOW())", &[&tenant, &account, &digest]).await.unwrap();
    for id in ids {
        client.execute("INSERT INTO trace_submission_sessions (tenant_id, submission_id, account_id, session_digest) VALUES ($1, $2, $3, $4)", &[&tenant, &id, &account, &digest]).await.unwrap();
    }
    assert!(
        backend
            .upsert_trace_submission_with_witness(
                submission(&tenant, ids[0], TraceCorpusStatus::Accepted),
                None
            )
            .await
            .is_err()
    );
    assert!(
        backend
            .update_trace_submission_status(
                &tenant,
                ids[1],
                TraceCorpusStatus::Accepted,
                "system",
                None
            )
            .await
            .is_err()
    );
    assert!(
        backend
            .release_pii_backstop_hold(&tenant, ids[2], TraceCorpusStatus::Accepted, "system", None)
            .await
            .is_err()
    );
    for id in ids {
        assert_ne!(
            backend
                .get_trace_submission(&tenant, id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TraceCorpusStatus::Accepted
        );
    }
    client
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .unwrap();
}

#[tokio::test]
async fn withdrawal_covers_resumed_ids_and_is_account_scoped_after_reconnect() {
    let Some(url) = database_url() else { return };
    let client = migrated_client(&url).await;
    let backend = PgBackend::new(&database_config(&url)).await.unwrap();
    let tenant = format!("z4-{}", Uuid::new_v4());
    let account = Uuid::new_v4();
    let other_account = Uuid::new_v4();
    let digest = [0x42u8; 32];
    let original = Uuid::new_v4();
    let resumed = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    for id in [account, other_account] {
        client
            .execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[&tenant, &id],
            )
            .await
            .unwrap();
    }
    assert_eq!(
        backend
            .claim_trace_source_session(&tenant, account, &digest, original)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    assert!(
        backend
            .claim_trace_source_session(&tenant, other_account, &digest, original)
            .await
            .is_err(),
        "one submission ID cannot be retargeted to another account"
    );
    assert_eq!(
        backend
            .claim_trace_source_session(&tenant, account, &digest, resumed)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    assert_eq!(
        backend
            .get_trace_source_session_status(&tenant, other_account, &digest)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    assert!(
        backend
            .withdraw_trace_source_session(&tenant, other_account, original, chrono::Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    backend
        .upsert_trace_submission(submission(&tenant, original, TraceCorpusStatus::Accepted))
        .await
        .unwrap();
    backend
        .upsert_trace_submission(submission(&tenant, resumed, TraceCorpusStatus::Quarantined))
        .await
        .unwrap();
    let first = backend
        .withdraw_trace_source_session(&tenant, account, original, chrono::Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.affected_submission_ids.len(), 2);
    let second = backend
        .withdraw_trace_source_session(&tenant, account, resumed, chrono::Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.withdrawn_at, second.withdrawn_at);
    assert_eq!(
        first.affected_submission_ids,
        second.affected_submission_ids
    );
    assert_eq!(
        backend
            .get_trace_submission(&tenant, original)
            .await
            .unwrap()
            .unwrap()
            .status,
        TraceCorpusStatus::Revoked
    );
    assert_eq!(
        backend
            .get_trace_submission(&tenant, resumed)
            .await
            .unwrap()
            .unwrap()
            .status,
        TraceCorpusStatus::Revoked
    );
    drop(backend);
    let reopened = PgBackend::new(&database_config(&url)).await.unwrap();
    assert_eq!(
        reopened
            .get_trace_source_session_status(&tenant, account, &digest)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Withdrawn
    );
    assert_eq!(
        reopened
            .get_trace_source_session_status(&tenant, other_account, &digest)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    assert_eq!(
        reopened
            .claim_trace_source_session(&tenant, account, &digest, Uuid::new_v4())
            .await
            .unwrap(),
        TraceSourceSessionStatus::Withdrawn
    );
    let other_tenant = format!("z4-{}", Uuid::new_v4());
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&other_tenant],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&other_tenant, &account],
        )
        .await
        .unwrap();
    assert_eq!(
        reopened
            .get_trace_source_session_status(&other_tenant, account, &digest)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active,
    );
    client
        .execute(
            "DELETE FROM trace_tenants WHERE tenant_id = $1",
            &[&other_tenant],
        )
        .await
        .unwrap();
    client
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_claim_and_withdrawal_leave_no_usable_resumed_version() {
    let Some(url) = database_url() else { return };
    let client = migrated_client(&url).await;
    let backend = std::sync::Arc::new(PgBackend::new(&database_config(&url)).await.unwrap());
    for _ in 0..8 {
        let tenant = format!("z4-{}", Uuid::new_v4());
        let account = Uuid::new_v4();
        let original = Uuid::new_v4();
        let resumed = Uuid::new_v4();
        let digest = [0x6du8; 32];
        client
            .execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
                &[&tenant],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[&tenant, &account],
            )
            .await
            .unwrap();
        backend
            .claim_trace_source_session(&tenant, account, &digest, original)
            .await
            .unwrap();
        backend
            .upsert_trace_submission(submission(
                &tenant,
                original,
                TraceCorpusStatus::Quarantined,
            ))
            .await
            .unwrap();
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(3));
        let claim = {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let start = start.clone();
            tokio::spawn(async move {
                start.wait().await;
                backend
                    .claim_trace_source_session(&tenant, account, &digest, resumed)
                    .await
                    .unwrap()
            })
        };
        let withdraw = {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let start = start.clone();
            tokio::spawn(async move {
                start.wait().await;
                backend
                    .withdraw_trace_source_session(&tenant, account, original, chrono::Utc::now())
                    .await
                    .unwrap()
                    .unwrap()
            })
        };
        start.wait().await;
        let (claimed, withdrawn) = tokio::join!(claim, withdraw);
        let claimed = claimed.unwrap();
        let withdrawn = withdrawn.unwrap();
        assert_eq!(
            backend
                .get_trace_source_session_status(&tenant, account, &digest)
                .await
                .unwrap(),
            TraceSourceSessionStatus::Withdrawn
        );
        match claimed {
            TraceSourceSessionStatus::Active => {
                assert!(withdrawn.affected_submission_ids.contains(&resumed));
                assert!(
                    backend
                        .upsert_trace_submission(submission(
                            &tenant,
                            resumed,
                            TraceCorpusStatus::Accepted
                        ))
                        .await
                        .is_err()
                );
            }
            TraceSourceSessionStatus::Withdrawn => {
                assert!(!withdrawn.affected_submission_ids.contains(&resumed));
            }
        }
        client
            .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn concurrent_approval_and_withdrawal_never_leave_accepted_content() {
    let Some(url) = database_url() else { return };
    let client = migrated_client(&url).await;
    let backend = std::sync::Arc::new(PgBackend::new(&database_config(&url)).await.unwrap());
    for _ in 0..8 {
        let tenant = format!("z4-{}", Uuid::new_v4());
        let account = Uuid::new_v4();
        let id = Uuid::new_v4();
        let digest = [0x58u8; 32];
        client
            .execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
                &[&tenant],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[&tenant, &account],
            )
            .await
            .unwrap();
        backend
            .claim_trace_source_session(&tenant, account, &digest, id)
            .await
            .unwrap();
        backend
            .upsert_trace_submission(submission(&tenant, id, TraceCorpusStatus::Quarantined))
            .await
            .unwrap();
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(3));
        let approval = {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let start = start.clone();
            tokio::spawn(async move {
                start.wait().await;
                backend
                    .update_trace_submission_status(
                        &tenant,
                        id,
                        TraceCorpusStatus::Accepted,
                        "system",
                        None,
                    )
                    .await
            })
        };
        let withdrawal = {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let start = start.clone();
            tokio::spawn(async move {
                start.wait().await;
                backend
                    .withdraw_trace_source_session(&tenant, account, id, chrono::Utc::now())
                    .await
                    .unwrap()
                    .unwrap()
            })
        };
        start.wait().await;
        let (approved, withdrawn) = tokio::join!(approval, withdrawal);
        let approved = approved.unwrap();
        let withdrawn = withdrawn.unwrap();
        assert!(withdrawn.affected_submission_ids.contains(&id));
        assert_eq!(
            backend
                .get_trace_submission(&tenant, id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TraceCorpusStatus::Revoked
        );
        if approved.is_ok() {
            assert_eq!(withdrawn.requested_tombstone.prior_status, "accepted");
        } else {
            assert_eq!(withdrawn.requested_tombstone.prior_status, "quarantined");
        }
        client
            .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
            .await
            .unwrap();
    }
}
