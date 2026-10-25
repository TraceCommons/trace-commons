// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit isolated database only: this fixture enables database-wide trust
//! invalidation. Never use the ordinary ingest test URL or DATABASE_URL.

use super::*;

const URL_VAR: &str = "TRACE_COMMONS_CLIENT_UPLOAD_TRUST_PG_TEST_URL";

fn database_config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: url.into(),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

/// Exercise production startup readiness and one actual applied scheduler
/// evaluation using a nonowner, NOBYPASSRLS runtime login. The caller creates
/// and disposes of the dedicated database; this test never resets one.
#[tokio::test]
#[ignore = "requires explicit isolated TRACE_COMMONS_CLIENT_UPLOAD_TRUST_PG_TEST_URL"]
async fn independent_upload_trust_scheduler_runs_applied_mode_without_admission_ledgers() {
    let url = std::env::var(URL_VAR).expect("explicit isolated upload-trust database URL required");
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_upload_trust_"));
    let admin_db = Arc::new(PgBackend::new(&database_config(url.clone())).await.unwrap());
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let main_privileges_before: bool = admin.query_one(
        "SELECT has_table_privilege(current_user,'public.trace_account_trust_facts','INSERT,UPDATE,DELETE')", &[])
        .await.unwrap().get(0);
    assert!(
        !admin_db
            .external_account_trust_runtime_ready()
            .await
            .unwrap(),
        "the migration/source login is deliberately not a narrow trust runtime"
    );
    let login = format!("tc_upload_trust_{}", Uuid::new_v4().simple());
    // A newly generated role only: never alter or replace an existing login.
    admin
        .batch_execute(&format!(
            "CREATE ROLE {login} LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS; \
         GRANT trace_account_admission_runtime, trace_account_trust_worker TO {login};"
        ))
        .await
        .unwrap();
    let mut runtime_url = parsed;
    runtime_url.set_username(&login).unwrap();
    runtime_url.set_password(None).unwrap();
    let runtime = Arc::new(
        PgBackend::new(&database_config(runtime_url.into()))
            .await
            .unwrap(),
    );
    account_trust_growth_routes::require_independent_upload_trust_runtime_ready(runtime.as_ref())
        .await
        .unwrap();

    let tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let account_id = Uuid::new_v4();
    let principal = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let anchor = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &account_id],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,'fixture-pepper','fixture-key')",
        &[&tenant, &account_id, &anchor, &serde_json::json!({"fixture": true})]).await.unwrap();
    admin.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,'fixture-key',NULL,'near')", &[&device, &tenant]).await.unwrap();
    admin.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant, &account_id, &principal]).await.unwrap();
    admin.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)", &[&tenant, &principal, &account_id, &device, &anchor]).await.unwrap();
    let submission_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4();
    let occurred_at = Utc::now() - chrono::TimeDelta::minutes(1);
    let digest = "a".repeat(64);
    admin.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash,received_at) VALUES($1,$2,$3,$4,'v1','v1','test','accepted','low','test',$5,$6)",
        &[&tenant, &submission_id, &trace_id, &principal, &digest, &occurred_at]).await.unwrap();
    admin.execute("INSERT INTO trace_credit_ledger(tenant_id,credit_event_id,submission_id,trace_id,credit_account_ref,event_type,points_delta,reason,actor_principal_ref,actor_role,settlement_state,occurred_at) VALUES($1,$2,$3,$4,'fixture','accepted','0','fixture',$5,'system','pending',$6)",
        &[&tenant, &Uuid::new_v4(), &submission_id, &trace_id, &principal, &occurred_at]).await.unwrap();
    admin.execute("INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,attestation_chain_hash,dedup_cluster_id,dedup_signal_version,decided_at) VALUES($1,$2,$3,'gate-v1',$4,1,1,TRUE,1,$4,TRUE,$4,$4,$5,'events.v2+simhash.v2',$6)",
        &[&tenant, &Uuid::new_v4(), &submission_id, &digest, &Uuid::new_v4(), &occurred_at]).await.unwrap();

    let version = format!("upload-tick-{}", &Uuid::new_v4().simple().to_string()[..12]);
    let independent = Arc::new(independent_upload_trust_policy(&version));
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(admin_db.clone() as Arc<dyn Database>),
        None,
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::get_mut(&mut state).unwrap();
    state_mut.require_db_mirror_writes = true;
    state_mut.require_postgres_trace_rls_ready = false;
    state_mut.client_upload_trust_database = Some(runtime.clone() as Arc<dyn Database>);
    state_mut.account_trust_applied_policy = Some(Arc::new(independent.growth.clone()));
    state_mut.client_upload_trust_policy = Some(independent);
    let config = account_trust_growth_routes::AccountTrustSchedulerConfig::new(
        "account-trust-worker-token-a".to_owned(),
        300,
        1000,
    );
    account_trust_growth_routes::validate_scheduler_config(state.as_ref(), Some(&config))
        .await
        .unwrap();
    let summary = account_trust_growth_routes::run_scheduler_tick(state.clone(), &config)
        .await
        .unwrap();
    assert!(
        summary.facts_recorded >= 1,
        "the seeded acceptance became a trust fact"
    );
    assert!(summary.evaluated >= 1, "the seeded account was evaluated");
    assert_eq!(summary.facts_failed, 0);
    assert_eq!(summary.evaluation_failed, 0);
    let account = trace_commons_server::account_trust::resolve_contribution_account(
        runtime.as_ref(),
        &tenant,
        &principal,
    )
    .await
    .unwrap();
    let evaluation = runtime
        .latest_account_trust_evaluation(
            &account,
            &version,
            trace_commons_server::account_trust_growth::APPLIED_MODE,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evaluation.units, 1);
    assert_eq!(evaluation.effective_allowance, 100);
    assert!(state.admission.is_none());
    assert!(state.account_admission.is_none());
    assert!(state.account_trust_shadow_policy.is_none());
    let reservations: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_admission_submissions WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        reservations, 0,
        "advisory client trust created no admission reservation"
    );
    let main_privileges_after: bool = admin.query_one(
        "SELECT has_table_privilege(current_user,'public.trace_account_trust_facts','INSERT,UPDATE,DELETE')", &[])
        .await.unwrap().get(0);
    assert_eq!(
        main_privileges_after, main_privileges_before,
        "the main source login's table privileges were not changed"
    );
    drop(state);
    drop(runtime);
    admin
        .batch_execute(&format!("DROP ROLE {login}"))
        .await
        .unwrap();
}
