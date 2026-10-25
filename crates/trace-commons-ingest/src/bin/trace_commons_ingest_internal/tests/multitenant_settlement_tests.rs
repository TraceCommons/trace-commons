// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn multitenant_settlement_explicit_config_needs_no_tenant_bearer() {
    const CHILD: &str = "TC_MULTITENANT_SETTLEMENT_CONFIG_TEST";
    if let Ok(case) = std::env::var(CHILD) {
        let result = parse_trace_credit_settlement_scheduler_config_from_env();
        if case == "valid" {
            assert!(result.unwrap().is_some());
        } else {
            assert!(result.is_err(), "{case} must fail closed");
        }
        return;
    }
    for case in ["valid", "disabled", "token", "invalid-scope"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.arg("--exact")
            .arg("ingest::app::tests::multitenant_settlement_tests::multitenant_settlement_explicit_config_needs_no_tenant_bearer")
            .arg("--nocapture")
            .env(CHILD, case)
            .env(CREDIT_SETTLEMENT_SCHEDULER_SCOPE_ENV, if case == "invalid-scope" { "invalid" } else { "all_tenants" })
            .env(TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED, if case == "disabled" { "false" } else { "true" })
            .env(TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION, "staging-2026-10")
            .env_remove(TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN);
        if case == "token" {
            command.env(
                TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN,
                "tenant-bearer",
            );
        }
        assert!(command.status().unwrap().success(), "child {case}");
    }
}

#[tokio::test]
async fn multitenant_settlement_pg_rotates_failed_tenants_without_bearers_and_replays_safely() {
    let _settlement_guard = SETTLEMENT_TEST_LOCK.lock().await;
    let Some(test_server) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    // Wraparound must run against a private candidate set: a shared suite DB
    // can contain payable events belonging to unrelated tests.
    let admin = test_server
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let database_name = format!("admission_test_scheduler_{}", Uuid::new_v4().simple());
    admin
        .batch_execute(&format!("CREATE DATABASE {database_name}"))
        .await
        .unwrap();
    let configured_url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let mut isolated_url = reqwest::Url::parse(&configured_url).unwrap();
    isolated_url.set_path(&database_name);
    let backend = Arc::new(
        PgBackend::new(&DatabaseConfig::from_postgres_url(isolated_url.as_str(), 4))
            .await
            .unwrap(),
    );
    backend.run_migrations().await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(backend.clone()),
        None,
        false,
        true,
        false,
        false,
    );
    let prefix = format!("scheduler-{}", Uuid::new_v4());
    let a = format!("{prefix}-a");
    let b = format!("{prefix}-b");
    {
        let s = Arc::make_mut(&mut state);
        s.require_db_mirror_writes = true;
        s.credit_settlement_allowed_policy_versions =
            Arc::new(BTreeSet::from(["trace-credit-policy-v1".to_owned()]));
        for auth in Arc::make_mut(&mut s.tokens).values_mut() {
            if auth.tenant_id == "tenant-a" {
                auth.tenant_id = a.clone();
            }
            if auth.tenant_id == "tenant-b" {
                auth.tenant_id = b.clone();
            }
        }
    }
    let mut submissions = Vec::new();
    for (token, worker) in [
        ("token-a", "utility-worker-token-a"),
        ("token-b", "admin-token-b"),
    ] {
        let id = submit_consented_metadata_fixture(
            state.clone(),
            token,
            ConsentScope::ModelTraining,
            TraceAllowedUse::ModelTraining,
        )
        .await;
        let Json(credit) = utility_credit_handler(
            State(state.clone()),
            auth_headers(worker),
            Json(TraceUtilityCreditJobRequest {
                event_type: TraceCreditLedgerEventType::TrainingUtility,
                credit_points_delta: 2.25,
                reason: "scheduler account fixture".to_owned(),
                external_ref: format!("training:{id}"),
                submission_ids: vec![id],
            }),
        )
        .await
        .expect("fixture utility credit");
        assert_eq!(credit.appended_count, 1);
        submissions.push(id);
    }
    Arc::make_mut(&mut state).tokens = Arc::new(BTreeMap::new());
    let actor = internal_credit_settlement_actor(&a);
    assert!(authenticate(&state, &auth_headers(&actor.principal_ref)).is_err());
    assert!(authenticate(&state, &HeaderMap::new()).is_err());
    let cursor = Arc::new(tokio::sync::Mutex::new(Some(prefix.clone())));
    let config = TraceCreditSettlementSchedulerConfig {
        scope: CreditSettlementSchedulerScope::AllTenants {
            cursor: cursor.clone(),
        },
        worker_token: SecretString::from(String::new()),
        interval: StdDuration::from_secs(60),
        dry_run: false,
        policy_version: "trace-credit-policy-v1".to_owned(),
        reason: "automatic settlement regression".to_owned(),
        issuer_approval_evidence_hash: None,
        near_contract_id: Some("trace-credits.testnet".to_owned()),
        ranking_model_version: None,
        ranking_target_use: None,
        limit: 100,
    };
    validate_trace_credit_settlement_scheduler_config(&state, Some(&config))
        .await
        .unwrap();
    Arc::make_mut(&mut state).db_reviewer_reads = false;
    assert!(
        validate_trace_credit_settlement_scheduler_config(&state, Some(&config))
            .await
            .is_err()
    );
    Arc::make_mut(&mut state).db_reviewer_reads = true;
    Arc::make_mut(&mut state).credit_settlement_central_issuer_principal_refs = Arc::new(
        BTreeSet::from([format!("principal_sha256:{}", "0".repeat(64))]),
    );
    assert!(
        validate_trace_credit_settlement_scheduler_config(&state, Some(&config))
            .await
            .is_err()
    );
    Arc::make_mut(&mut state).credit_settlement_central_issuer_principal_refs =
        Arc::new(BTreeSet::from([actor.principal_ref.clone()]));
    validate_trace_credit_settlement_scheduler_config(&state, Some(&config))
        .await
        .unwrap();
    let conn = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let consent: serde_json::Value = conn
        .query_one(
            "SELECT consent_scopes FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2",
            &[&a, &submissions[0]],
        )
        .await
        .unwrap()
        .get(0);
    conn.execute("UPDATE trace_submissions SET consent_scopes='\"invalid\"'::jsonb WHERE tenant_id=$1 AND submission_id=$2", &[&a, &submissions[0]]).await.unwrap();
    let first = run_all_tenant_credit_settlement_tick(state.clone(), &config, 1)
        .await
        .unwrap();
    assert_eq!(
        (
            first.tenants_checked,
            first.tenants_failed,
            first.settled_source_event_count
        ),
        (1, 1, 0)
    );
    assert_eq!(*cursor.lock().await, Some(a.clone()));
    let second = run_all_tenant_credit_settlement_tick(state.clone(), &config, 1)
        .await
        .unwrap();
    assert_eq!(
        (
            second.tenants_checked,
            second.tenants_failed,
            second.settled_source_event_count
        ),
        (1, 0, 1)
    );
    assert_eq!(*cursor.lock().await, Some(b.clone()));
    conn.execute(
        "UPDATE trace_submissions SET consent_scopes=$3 WHERE tenant_id=$1 AND submission_id=$2",
        &[&a, &submissions[0], &consent],
    )
    .await
    .unwrap();
    let recovered = run_all_tenant_credit_settlement_tick(state.clone(), &config, 1)
        .await
        .unwrap();
    assert_eq!(
        (
            recovered.tenants_checked,
            recovered.tenants_failed,
            recovered.settled_source_event_count
        ),
        (1, 0, 1)
    );
    for _ in 0..2 {
        let replay = run_all_tenant_credit_settlement_tick(state.clone(), &config, 1)
            .await
            .unwrap();
        assert_eq!(
            (
                replay.tenants_checked,
                replay.tenants_failed,
                replay.settled_source_event_count
            ),
            (0, 0, 0)
        );
    }
    for tenant in [&a, &b] {
        let row = conn.query_one("SELECT count(*),sum(cardinality(source_credit_event_ids)) FROM trace_credit_settlement_batches WHERE tenant_id=$1 AND status='finalized'", &[tenant]).await.unwrap();
        assert_eq!(row.get::<_, i64>(0), 1);
        assert_eq!(row.get::<_, Option<i64>>(1), Some(1));
    }
    drop(conn);
    drop(state);
    drop(backend);
    admin
        .batch_execute(&format!("DROP DATABASE {database_name} WITH (FORCE)"))
        .await
        .unwrap();
}
