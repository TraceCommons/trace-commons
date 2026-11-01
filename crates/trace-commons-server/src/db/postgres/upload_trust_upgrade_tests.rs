// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Real pre-V137 upgrade: already-enabled applied rows must become stale.

use super::{MIGRATIONS, PgBackend, apply_and_record_migration};
use crate::config::{DatabaseConfig, SslMode};
use crate::db::Database;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires fresh isolated TRACE_COMMONS_UPLOAD_TRUST_PG_UPGRADE_TEST_URL"]
async fn v137_invalidates_already_enabled_pre_migration_applied_generations_once() {
    let url = std::env::var("TRACE_COMMONS_UPLOAD_TRUST_PG_UPGRADE_TEST_URL")
        .expect("explicit isolated upgrade URL");
    let target = url.parse::<tokio_postgres::Config>().unwrap();
    assert!(
        matches!(target.get_hosts(),[tokio_postgres::config::Host::Tcp(host)] if host=="127.0.0.1")
    );
    assert!(
        target
            .get_dbname()
            .unwrap()
            .starts_with("admission_test_upload_trust_upgrade_")
    );
    let (mut client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        connection.await.unwrap();
    });
    client.batch_execute("CREATE TABLE _trace_commons_migrations (version INTEGER PRIMARY KEY,name TEXT NOT NULL,applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW())").await.unwrap();
    for (version, name, sql) in MIGRATIONS.iter().filter(|(version, _, _)| *version <= 136) {
        apply_and_record_migration(&mut client, *version, name, sql)
            .await
            .unwrap();
    }
    let tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let account = Uuid::new_v4();
    let submission = Uuid::new_v4();
    let decision = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash) VALUES($1,$2,$3,'fixture','v1','v1','test','accepted','low','test',$4)", &[&tenant,&submission,&Uuid::new_v4(),&"a".repeat(64)]).await.unwrap();
    client.execute("INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,attestation_chain_hash) VALUES($1,$2,$3,'gate-v1',$4,1,1,TRUE,1,$4,TRUE,$4,$4)", &[&tenant,&decision,&submission,&"a".repeat(64)]).await.unwrap();
    let backend = PgBackend::new(&DatabaseConfig {
        url: url.into(),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
    .await
    .unwrap();
    backend
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    let tx = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .start()
        .await
        .unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let old_generation: i64 = tx
        .query_one(
            "SELECT trace_account_trust_input_generation($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap()
        .get(0);
    tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'pre-v137','applied',transaction_timestamp(),1,200,$4,$5)", &[&tenant,&Uuid::new_v4(),&account,&format!("sha256:{}","5".repeat(64)),&old_generation]).await.unwrap();
    tx.commit().await.unwrap();
    client.execute("UPDATE trace_gate_decisions SET novelty_passed=FALSE WHERE tenant_id=$1 AND decision_id=$2", &[&tenant,&decision]).await.unwrap();
    let before:i64=client.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap().get(0);
    assert_eq!(
        before, old_generation,
        "pre-V137 did not invalidate this gate-boolean correction"
    );
    backend.run_migrations().await.unwrap();
    let state=client.query_one("SELECT f.generation,e.input_generation,(f.generation=e.input_generation) FROM trace_account_trust_frontiers f JOIN trace_account_trust_evaluations e USING(tenant_id,account_id) WHERE f.tenant_id=$1 AND f.account_id=$2", &[&tenant,&account]).await.unwrap();
    assert_eq!(state.get::<_, i64>(0), old_generation + 1);
    assert_eq!(
        state.get::<_, i64>(1),
        old_generation,
        "migration does not rewrite evaluation rows"
    );
    assert!(
        !state.get::<_, bool>(2),
        "old uplift is no longer generation-current"
    );
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1",
                &[&tenant]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM trace_credit_ledger WHERE tenant_id=$1",
                &[&tenant]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    backend.run_migrations().await.unwrap();
    assert_eq!(client.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap().get::<_,i64>(0),old_generation+1,
        "the migration runner never applies the metadata invalidation twice");
}
