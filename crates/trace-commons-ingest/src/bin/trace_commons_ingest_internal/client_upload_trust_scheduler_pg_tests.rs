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
    exercise_narrow_upload_trust_role(false).await;
}

#[tokio::test]
#[ignore = "requires explicit isolated TRACE_COMMONS_CLIENT_UPLOAD_TRUST_PG_TEST_URL"]
async fn independent_upload_trust_role_rejects_broad_and_column_authority() {
    exercise_narrow_upload_trust_role(true).await;
}

fn run_bootstrap(url: &str, database: &str, login: &str, group: &str) -> std::process::Output {
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/staging-gcp/prepare-upload-trust-role.sql"
    ))
    .replace("tc_upload_trust_login", login)
    .replace("trace_client_upload_trust_runtime", group);
    // PGDATABASE is a literal database name, not a connection URI. Parse
    // once with the same driver as the fixture, including percent decoding,
    // and keep credentials out of argv and diagnostic output.
    let connection: tokio_postgres::Config = url.parse().expect("isolated bootstrap connection");
    let [tokio_postgres::config::Host::Tcp(host)] = connection.get_hosts() else {
        panic!("isolated bootstrap requires one explicit TCP host");
    };
    assert_eq!(host, "127.0.0.1");
    assert_eq!(connection.get_dbname(), Some(database));
    assert!(database.starts_with("admission_test_upload_trust_"));
    let mut command = std::process::Command::new("psql");
    command
        .arg("--no-psqlrc")
        .arg(format!("--set=expected_database={database}"))
        .env("PGHOST", host)
        .env(
            "PGPORT",
            connection
                .get_ports()
                .first()
                .copied()
                .unwrap_or(5432)
                .to_string(),
        )
        .env("PGDATABASE", database)
        .env(
            "PGSSLMODE",
            match connection.get_ssl_mode() {
                tokio_postgres::config::SslMode::Disable => "disable",
                tokio_postgres::config::SslMode::Require => "require",
                _ => "prefer",
            },
        )
        .env_remove("PGHOSTADDR")
        .env_remove("PGUSER")
        .env_remove("PGPASSWORD")
        .env_remove("PGOPTIONS")
        .env_remove("PGSERVICE")
        .env_remove("PGSERVICEFILE")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(user) = connection.get_user() {
        command.env("PGUSER", user);
    }
    if let Some(password) = connection.get_password() {
        command.env(
            "PGPASSWORD",
            std::str::from_utf8(password).expect("fixture password encoding"),
        );
    }
    if let Some(options) = connection.get_options() {
        command.env("PGOPTIONS", options);
    }
    let mut child = command.spawn().expect("start isolated bootstrap");
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), script.as_bytes()).unwrap();
    child
        .wait_with_output()
        .expect("read isolated bootstrap result")
}

async fn exercise_narrow_upload_trust_role(check_privileges: bool) {
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
    let group = format!("tc_upload_group_{}", Uuid::new_v4().simple());
    let database = parsed.path().trim_start_matches('/').to_owned();
    let prepared = run_bootstrap(&url, &database, &login, &group);
    assert!(
        prepared.status.success(),
        "isolated bootstrap: {}",
        String::from_utf8_lossy(&prepared.stderr)
    );
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
    assert!(runtime.client_upload_trust_runtime_ready().await.unwrap());
    assert!(
        !runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap(),
        "the unchanged admission contract still requires reservation/prune seams"
    );
    if check_privileges {
        for table in [
            "trace_accounts",
            "trace_account_principals",
            "trace_near_provisioned_devices",
            "device_keys",
            "trace_account_invite_grants",
            "trace_account_trust",
            "trace_account_admission_budget",
            "trace_account_admission_submissions",
            "trace_account_trust_facts",
            "trace_account_trust_evaluations",
            "trace_submissions",
            "trace_gate_decisions",
            "trace_credit_ledger",
            "trace_audit_events",
        ] {
            let row = admin.query_one("SELECT has_table_privilege($1,$2,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES'), has_any_column_privilege($1,$2,'INSERT,UPDATE,REFERENCES')",
                &[&login, &format!("public.{table}")]).await.unwrap();
            assert!(
                !row.get::<_, bool>(0) && !row.get::<_, bool>(1),
                "no direct write authority on {table}"
            );
        }
        for function in [
            "public.trace_transition_admission(text,uuid,uuid,text)",
            "public.trace_resume_legacy_admission(text,text,uuid,text,uuid,bigint)",
            "public.trace_account_trust_lock_input_generation(text,uuid)",
            "public.trace_account_trust_prune_dependency_locks(integer,boolean)",
        ] {
            assert!(
                !admin
                    .query_one(
                        "SELECT has_function_privilege($1,$2,'EXECUTE')",
                        &[&login, &function]
                    )
                    .await
                    .unwrap()
                    .get::<_, bool>(0)
            );
        }
        // These isolated column grants evade has_table_privilege. Both the
        // independent runtime check and repeatable bootstrap must refuse them.
        for (table, column) in [
            ("trace_accounts", "closed_at"),
            ("trace_account_trust", "authority"),
            ("trace_account_admission_budget", "cost_used"),
            ("trace_account_trust_facts", "outcome"),
        ] {
            admin
                .batch_execute(&format!(
                    "GRANT UPDATE({column}) ON public.{table} TO {login}"
                ))
                .await
                .unwrap();
            assert!(
                !admin
                    .query_one(
                        "SELECT has_table_privilege($1,$2,'UPDATE')",
                        &[&login, &format!("public.{table}")]
                    )
                    .await
                    .unwrap()
                    .get::<_, bool>(0)
            );
            assert!(!runtime.client_upload_trust_runtime_ready().await.unwrap());
            assert!(
                !run_bootstrap(&url, &database, &login, &group)
                    .status
                    .success()
            );
            admin
                .batch_execute(&format!(
                    "REVOKE UPDATE({column}) ON public.{table} FROM {login}"
                ))
                .await
                .unwrap();
            assert!(runtime.client_upload_trust_runtime_ready().await.unwrap());
        }
        admin
            .batch_execute(&format!(
                "GRANT SELECT(public_key) ON public.device_keys TO {login}"
            ))
            .await
            .unwrap();
        assert!(!runtime.client_upload_trust_runtime_ready().await.unwrap());
        assert!(
            !run_bootstrap(&url, &database, &login, &group)
                .status
                .success()
        );
        admin
            .batch_execute(&format!(
                "REVOKE SELECT(public_key) ON public.device_keys FROM {login}"
            ))
            .await
            .unwrap();
        admin.batch_execute(&format!("GRANT EXECUTE ON FUNCTION public.trace_transition_admission(text,uuid,uuid,text) TO {login}")).await.unwrap();
        assert!(!runtime.client_upload_trust_runtime_ready().await.unwrap());
        assert!(
            !run_bootstrap(&url, &database, &login, &group)
                .status
                .success()
        );
        admin.batch_execute(&format!("REVOKE EXECUTE ON FUNCTION public.trace_transition_admission(text,uuid,uuid,text) FROM {login}")).await.unwrap();
        admin.batch_execute(&format!("GRANT trace_account_admission_runtime TO {login} WITH ADMIN FALSE, INHERIT TRUE, SET FALSE")).await.unwrap();
        assert!(!runtime.client_upload_trust_runtime_ready().await.unwrap());
        assert!(
            !run_bootstrap(&url, &database, &login, &group)
                .status
                .success()
        );
        admin
            .batch_execute(&format!(
                "REVOKE trace_account_admission_runtime FROM {login}"
            ))
            .await
            .unwrap();
        assert!(runtime.client_upload_trust_runtime_ready().await.unwrap());
        // Membership without inheritance can still authorize SET ROLE. A
        // clean current_user privilege check alone must never accept it.
        for privilege in ["transition", "raw_select"] {
            let extra = format!("tc_upload_extra_{}", Uuid::new_v4().simple());
            admin.batch_execute(&format!("CREATE ROLE {extra} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS")).await.unwrap();
            if privilege == "transition" {
                admin.batch_execute(&format!("GRANT EXECUTE ON FUNCTION public.trace_transition_admission(text,uuid,uuid,text) TO {extra}")).await.unwrap();
            } else {
                admin
                    .batch_execute(&format!(
                        "GRANT SELECT(auth_principal_ref) ON public.trace_submissions TO {extra}"
                    ))
                    .await
                    .unwrap();
            }
            admin
                .batch_execute(&format!(
                    "GRANT {extra} TO {login} WITH ADMIN FALSE, INHERIT FALSE, SET TRUE"
                ))
                .await
                .unwrap();
            let mut connection = runtime
                .raw_pool_for_tests_and_diagnostics()
                .get()
                .await
                .unwrap();
            let effective: bool = connection.query_one(if privilege == "transition" {
                "SELECT has_function_privilege(current_user,'public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE')"
            } else {
                "SELECT has_column_privilege(current_user,'public.trace_submissions','auth_principal_ref','SELECT')"
            }, &[]).await.unwrap().get(0);
            assert!(!effective, "the injected authority is not inherited");
            assert!(!runtime.client_upload_trust_runtime_ready().await.unwrap());
            let tx = connection.transaction().await.unwrap();
            tx.batch_execute(&format!("SET LOCAL ROLE {extra}"))
                .await
                .unwrap();
            let reachable: bool = tx.query_one(if privilege == "transition" {
                "SELECT has_function_privilege(current_user,'public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE')"
            } else {
                "SELECT has_column_privilege(current_user,'public.trace_submissions','auth_principal_ref','SELECT')"
            }, &[]).await.unwrap().get(0);
            assert!(reachable, "SET ROLE exposes the otherwise hidden authority");
            tx.rollback().await.unwrap();
            drop(connection);
            admin
                .batch_execute(&format!(
                    "REVOKE {extra} FROM {login}; DROP OWNED BY {extra}; DROP ROLE {extra}"
                ))
                .await
                .unwrap();
            assert!(runtime.client_upload_trust_runtime_ready().await.unwrap());
        }
        // The endpoint's ownership resolver needs every positive status
        // column. A role with fewer reads is also an unusable configuration.
        admin
            .batch_execute(&format!(
                "REVOKE SELECT(anchor_hash) ON public.trace_near_provisioned_devices FROM {group}"
            ))
            .await
            .unwrap();
        assert!(!runtime.client_upload_trust_runtime_ready().await.unwrap());
        admin
            .batch_execute(&format!(
                "GRANT SELECT(anchor_hash) ON public.trace_near_provisioned_devices TO {group}"
            ))
            .await
            .unwrap();
        assert!(runtime.client_upload_trust_runtime_ready().await.unwrap());
        let connection = runtime
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let denied = connection
            .batch_execute("UPDATE public.trace_accounts SET closed_at=closed_at WHERE FALSE")
            .await
            .unwrap_err();
        assert_eq!(
            denied.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }

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
    for _ in 0..5 {
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
    }

    let version = format!("upload-tick-{}", &Uuid::new_v4().simple().to_string()[..12]);
    let template = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/staging-gcp/client-upload-trust-policy.json.template"
    ))
    .replace("staging-earned-upload-v1", &version)
    .replace("${TC_UPLOAD_TRUST_GATE_POLICY_VERSION}", "gate-v1")
    .replace(
        "${TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION}",
        "events.v2+simhash.v2",
    );
    let independent = Arc::new(upload_allowance::ClientUploadTrustPolicy {
        growth: trace_commons_server::account_trust_rule::parse_growth_policy(
            &template,
            &[version.as_str()],
        )
        .unwrap(),
    });
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
        summary.facts_recorded >= 5,
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
    assert_eq!(evaluation.units, 5);
    assert_eq!(evaluation.effective_allowance, 200);
    let ctx = AccountCtx {
        account_id: AccountId::from_uuid(account_id),
        principal_set: AccountPrincipalSet::from_iter_for_test_only([principal.clone()]),
        auth_method: AccountAuthMethod::NativeToken,
        tenant_id: tenant.clone(),
        actor_ref: principal.clone(),
        auth_credential_id: None,
        client_kind: "native".to_owned(),
        fresh_near_ai_login: false,
        session_token_hash: None,
    };
    let response = upload_allowance::handler(State(state.clone()), Extension(ctx))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let allowance: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        allowance,
        serde_json::json!({"allowance": null}),
        "no client daily limit is advertised"
    );
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
        .batch_execute(&format!(
            "DROP OWNED BY {login}; DROP ROLE {login}; DROP OWNED BY {group}; DROP ROLE {group}"
        ))
        .await
        .unwrap();
}
