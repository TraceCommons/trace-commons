// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::{DateTime, Utc};
use trace_commons_server::{
    account_trust::{parse_bounded_policy, resolve_contribution_account},
    admission_ledger::{AccountAdmissionReservation, AdmissionDecision},
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
    trace_corpus_storage::{DedupAssignmentWrite, TraceCorpusStore},
};
use uuid::Uuid;

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: url.into(),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn publish_current(evaluator: &mut tokio_postgres::Client, tenant: &str, account: Uuid) {
    let tx = evaluator
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
    let generation: i64 = tx
        .query_one(
            "SELECT trace_account_trust_input_generation($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap()
        .get(0);
    tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),2,30,$4,$5)", &[&tenant,&Uuid::new_v4(),&account,&format!("sha256:{}","5".repeat(64)),&generation]).await.unwrap();
    tx.commit().await.unwrap();
}

async fn seed_dependency_gate(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    submission: Uuid,
    cluster: Uuid,
    earlier: bool,
) -> Uuid {
    let decision = Uuid::new_v4();
    admin.execute("INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,attestation_chain_hash,dedup_cluster_id,dedup_signal_version,credit_quality_micros,credit_quality_calibration_version,decided_at) VALUES($1,$2,$3,'gate-fixture',$4,1,1,TRUE,1,$4,TRUE,$4,$4,$5,'signal-fixture',200,1,clock_timestamp()-CASE WHEN $6 THEN interval '1 day' ELSE interval '0' END)", &[&tenant,&decision,&submission,&"a".repeat(64),&cluster,&earlier]).await.unwrap();
    decision
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ACCOUNT_ADMISSION_PG_TEST_URL"]
async fn external_evaluation_scope_freshness_clamps_and_spend() {
    let url = std::env::var("TRACE_COMMONS_ACCOUNT_ADMISSION_PG_TEST_URL").unwrap();
    let parsed = url.parse::<tokio_postgres::Config>().unwrap();
    assert!(
        matches!(parsed.get_hosts().first(), Some(tokio_postgres::config::Host::Tcp(host)) if host == "127.0.0.1")
    );
    assert!(parsed.get_dbname().unwrap().starts_with("admission_test"));
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN
        IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='external_admission_login') THEN CREATE ROLE external_admission_login LOGIN NOSUPERUSER NOBYPASSRLS; END IF;
        IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='external_evaluator_login') THEN CREATE ROLE external_evaluator_login LOGIN NOSUPERUSER NOBYPASSRLS; END IF;
        END $$;
        GRANT trace_account_admission_runtime TO external_admission_login;
        GRANT trace_account_trust_evaluator TO external_evaluator_login;").await.unwrap();
    let login_url = |name: &str| {
        let mut url = reqwest::Url::parse(&url).unwrap();
        url.set_username(name).unwrap();
        String::from(url)
    };
    let runtime = PgBackend::new(&config(login_url("external_admission_login")))
        .await
        .unwrap();
    assert!(
        runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap()
    );
    assert!(
        !admin_db
            .external_account_trust_runtime_ready()
            .await
            .unwrap()
    );
    admin.batch_execute("REVOKE EXECUTE ON FUNCTION trace_record_external_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,INTEGER,BIGINT,TEXT,BIGINT) FROM trace_account_trust_evaluator").await.unwrap();
    assert!(
        !runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap()
    );
    admin.batch_execute("GRANT EXECUTE ON FUNCTION trace_record_external_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,INTEGER,BIGINT,TEXT,BIGINT) TO trace_account_trust_evaluator").await.unwrap();
    admin
        .batch_execute(
            "ALTER TABLE trace_gate_decisions DISABLE TRIGGER account_trust_gate_frontier",
        )
        .await
        .unwrap();
    assert!(
        !runtime
            .external_account_trust_runtime_ready()
            .await
            .unwrap(),
        "external readiness requires current gate invalidation"
    );
    admin
        .batch_execute(
            "ALTER TABLE trace_gate_decisions ENABLE TRIGGER account_trust_gate_frontier",
        )
        .await
        .unwrap();
    let (mut evaluator, conn) = tokio_postgres::connect(
        &login_url("external_evaluator_login"),
        tokio_postgres::NoTls,
    )
    .await
    .unwrap();
    tokio::spawn(async move {
        conn.await.unwrap();
    });
    let unsafe_role: bool = evaluator.query_one("SELECT rolsuper OR rolbypassrls OR pg_has_role(current_user,'trace_account_trust_worker','MEMBER') FROM pg_roles WHERE rolname=current_user", &[]).await.unwrap().get(0);
    assert!(!unsafe_role);
    for table in [
        "trace_account_trust_facts",
        "trace_accounts",
        "trace_account_trust",
        "trace_account_admission_budget",
        "trace_account_admission_submissions",
        "trace_account_trust_evaluations",
        "trace_account_trust_frontiers",
    ] {
        let writable: bool = evaluator
            .query_one(
                "SELECT has_table_privilege(current_user,$1,'INSERT,UPDATE,DELETE')",
                &[&table],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!writable, "unexpected writer on {table}");
    }
    let record_facts: bool = evaluator.query_one("SELECT has_function_privilege(current_user,'trace_record_account_trust_fact(text,uuid,text,uuid)','EXECUTE')", &[]).await.unwrap().get(0);
    assert!(!record_facts);
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
    admin.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,'fixture-pepper','fixture-key')", &[&tenant,&account_id,&anchor,&serde_json::json!({"fixture":true})]).await.unwrap();
    admin.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,'fixture-key',NULL,'near')", &[&device,&tenant]).await.unwrap();
    admin.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant,&account_id,&principal]).await.unwrap();
    admin.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)", &[&tenant,&principal,&account_id,&device,&anchor]).await.unwrap();
    let account = resolve_contribution_account(&runtime, &tenant, &principal)
        .await
        .unwrap();
    let policy = parse_bounded_policy(r#"{"version":"external-fixture","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"external","growth_policy_version":"growth-fixture","allowance_ceiling":30,"evaluation_max_age_seconds":60}"#, &["external-fixture"]).unwrap();
    let request = || AccountAdmissionReservation {
        account: account.clone(),
        principal_ref: principal.clone(),
        expected_trust_version: None,
        submission_id: Uuid::new_v4(),
        body_hash: "a".repeat(64),
        lease_id: Uuid::new_v4(),
        policy: policy.clone(),
        lease_seconds: 60,
    };
    let first = request();
    assert_eq!(
        runtime
            .reserve_account_admission(&first)
            .await
            .unwrap()
            .decision,
        AdmissionDecision::Reserved
    );
    let evidence = admin.query_one("SELECT earned_tier,trust_evaluation_digest FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&first.submission_id]).await.unwrap();
    assert_eq!(evidence.get::<_, Option<i32>>(0), Some(0));
    assert_eq!(evidence.get::<_, Option<String>>(1), None);
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    // Writer fails closed for wrong tenant and future timestamps, without touching rows.
    for (scope, offset) in [("other-tenant", 0_i64), (tenant.as_str(), 3600)] {
        let tx = evaluator.transaction().await.unwrap();
        tx.query_one(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&scope],
        )
        .await
        .unwrap();
        assert!(tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp()+make_interval(secs=>$4::bigint::double precision),1,30,$5,0)", &[&tenant,&Uuid::new_v4(),&account_id,&offset,&format!("sha256:{}","1".repeat(64))]).await.is_err());
        tx.rollback().await.unwrap();
    }
    let tx = evaluator.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    assert!(
        tx.query(
            "SELECT * FROM trace_account_trust_worker_accounts(NULL,NULL,10000)",
            &[]
        )
        .await
        .unwrap()
        .iter()
        .any(|row| row.get::<_, Uuid>(1) == account_id)
    );
    assert!(
        tx.query(
            "SELECT * FROM trace_account_trust_evaluation_inputs($1,$2)",
            &[&tenant, &account_id]
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(
        tx.query(
            "SELECT * FROM trace_account_trust_evaluation_inputs('other-tenant',$1)",
            &[&account_id]
        )
        .await
        .unwrap()
        .is_empty()
    );
    tx.commit().await.unwrap();
    let generation: i64 = {
        let tx = evaluator.transaction().await.unwrap();
        tx.query_one(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&tenant],
        )
        .await
        .unwrap();
        let row = tx
            .query_one(
                "SELECT trace_account_trust_input_generation($1,$2)",
                &[&tenant, &account_id],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        row.get(0)
    };
    // Wrong policy, shadow, stale and future rows cannot raise the base allowance.
    let digest = format!("sha256:{}", "2".repeat(64));
    for (version, mode, offset) in [
        ("other-policy", "applied", 0_i64),
        ("growth-fixture", "shadow", 0),
        ("growth-fixture", "applied", -120),
        ("growth-fixture", "applied", 3600),
    ] {
        admin.execute("INSERT INTO trace_account_trust_evaluations(tenant_id,evaluation_id,account_id,growth_policy_version,mode,as_of,tier,effective_allowance,facts_digest,input_generation) VALUES($1,$2,$3,$4,$5,clock_timestamp()+make_interval(secs=>$6::bigint::double precision),1,30,$7,$8)", &[&tenant,&Uuid::new_v4(),&account_id,&version,&mode,&offset,&digest,&generation]).await.unwrap();
        assert_eq!(
            runtime
                .reserve_account_admission(&request())
                .await
                .unwrap()
                .decision,
            AdmissionDecision::Exhausted
        );
        assert!(
            !runtime
                .account_admission_status(&account, &principal, &policy)
                .await
                .unwrap()
                .unwrap()
                .ready
        );
    }
    assert!(
        runtime
            .latest_account_trust_evaluation(&account, "growth-fixture", "shadow")
            .await
            .unwrap()
            .is_none(),
        "compact rows never enter the legacy feature decoder"
    );
    // Coherent same-timestamp evaluations use recorded_at and UUID tie breakers.
    let as_of: DateTime<Utc> = admin
        .query_one("SELECT clock_timestamp() - interval '1 second'", &[])
        .await
        .unwrap()
        .get(0);
    let mut ids = [Uuid::new_v4(), Uuid::new_v4()];
    ids.sort();
    let [low, high] = ids;
    let high_digest = format!("sha256:{}", "3".repeat(64));
    for (id, tier, allowance, hash) in [(high, 2_i32, 99_i64, &high_digest), (low, 1, 20, &digest)]
    {
        let tx = evaluator.transaction().await.unwrap();
        tx.query_one(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',$4,$5,$6,$7,$8)", &[&tenant,&id,&account_id,&as_of,&tier,&allowance,&hash,&generation]).await.unwrap();
        tx.commit().await.unwrap();
    }
    admin.execute("UPDATE trace_account_trust_evaluations SET recorded_at=$3 WHERE tenant_id=$1 AND evaluation_id IN ($2,$4)",&[&tenant,&low,&as_of,&high]).await.unwrap();
    assert!(
        runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    // A repeatable-read batch cannot publish an evaluation over a changed identity.
    let tx = evaluator
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
            &[&tenant, &account_id],
        )
        .await
        .unwrap()
        .get(0);
    admin.execute("UPDATE trace_account_principals SET unlinked_at=unlinked_at WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap();
    assert!(tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),2,30,$4,$5)", &[&tenant,&Uuid::new_v4(),&account_id,&digest,&old_generation]).await.is_err());
    tx.rollback().await.unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "identity mutation invalidates fresh applied evidence"
    );
    let tx = evaluator.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),2,30,$4,$5)", &[&tenant,&Uuid::new_v4(),&account_id,&digest,&(old_generation+1)]).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    // New adverse facts also invalidate an evaluation, even with historical occurred_at.
    let submission = Uuid::new_v4();
    admin.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash) VALUES($1,$2,$3,$4,'v1','v1','test','accepted','low','test',$5)", &[&tenant,&submission,&Uuid::new_v4(),&principal,&"a".repeat(64)]).await.unwrap();
    admin.execute("INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,submission_id,outcome,occurred_at) VALUES($1,$2,'submission_revoked',$3,$3,'revoked',clock_timestamp()-interval '1 year')", &[&tenant,&account_id,&submission]).await.unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "new historical adverse facts invalidate fresh evaluations"
    );
    // A consumed merge invalidates the survivor even without new facts or authority changes.
    let absorbed = Uuid::new_v4();
    let proposal = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &absorbed],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_account_merge_proposals(tenant_id,proposal_id,surviving_account_id,absorbed_account_id,expires_at) VALUES($1,$2,$3,$4,clock_timestamp()+interval '1 hour')", &[&tenant,&proposal,&account_id,&absorbed]).await.unwrap();
    let mut merge_client = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let merge_tx = merge_client.transaction().await.unwrap();
    merge_tx
        .query_one(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&tenant],
        )
        .await
        .unwrap();
    merge_tx.execute("UPDATE trace_account_merge_proposals SET consumed_at=clock_timestamp() WHERE tenant_id=$1 AND proposal_id=$2", &[&tenant,&proposal]).await.unwrap();
    merge_tx
        .query_one(
            "SELECT trace_account_trust_merge($1,$2,$3,$4)",
            &[&tenant, &account_id, &absorbed, &proposal],
        )
        .await
        .unwrap();
    merge_tx.commit().await.unwrap();
    let refreshed_generation: i64=admin.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap().get(0);
    assert_eq!(refreshed_generation, old_generation + 3);
    let tx = evaluator.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    assert!(tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),2,30,$4,$5)", &[&tenant,&Uuid::new_v4(),&account_id,&digest,&old_generation]).await.is_err(),"stale batch generation refused without a repeatable-read transaction too");
    tx.rollback().await.unwrap();
    let tx = evaluator.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),2,99,$4,$5)", &[&tenant,&Uuid::new_v4(),&account_id,&high_digest,&refreshed_generation]).await.unwrap();
    tx.commit().await.unwrap();
    let generation = refreshed_generation;
    let second = request();
    let third = request();
    let (a, b) = tokio::join!(
        runtime.reserve_account_admission(&second),
        runtime.reserve_account_admission(&third)
    );
    assert_eq!(a.unwrap().decision, AdmissionDecision::Reserved);
    assert_eq!(b.unwrap().decision, AdmissionDecision::Reserved);
    assert_eq!(
        runtime
            .reserve_account_admission(&request())
            .await
            .unwrap()
            .decision,
        AdmissionDecision::Exhausted,
        "ceiling is enforced"
    );
    let evidence=admin.query_one("SELECT earned_tier,trust_evaluation_digest FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&second.submission_id]).await.unwrap();
    assert_eq!(evidence.get::<_, Option<i32>>(0), Some(2));
    assert_eq!(evidence.get::<_, Option<String>>(1), Some(high_digest));
    // A newer below-base evaluation cannot erase previously charged spend.
    let tx = evaluator.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),0,1,$4,$5)", &[&tenant,&Uuid::new_v4(),&account_id,&digest,&generation]).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    assert_eq!(
        runtime
            .reserve_account_admission(&request())
            .await
            .unwrap()
            .decision,
        AdmissionDecision::Exhausted
    );
    let budget=admin.query_one("SELECT cost_used,cost_limit FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap();
    assert_eq!(budget.get::<_, i64>(0), 30);
    assert_eq!(
        budget.get::<_, i64>(1),
        10,
        "stored base invariant remains unchanged"
    );
    // An invited account retains its bypass and no evaluation attribution.
    admin.execute("INSERT INTO trace_account_invite_grants(tenant_id,account_id,invite_subject_hash,trust_version) VALUES($1,$2,$3,2)", &[&tenant,&account_id,&format!("sha256:{}","4".repeat(64))]).await.unwrap();
    admin.execute("UPDATE trace_account_trust SET authority='invited',trust_version=2 WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap();
    let invited = request();
    assert_eq!(
        runtime
            .reserve_account_admission(&invited)
            .await
            .unwrap()
            .authority,
        Some("invited")
    );
    assert!(
        runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    let evidence=admin.query_one("SELECT earned_tier,trust_evaluation_digest,last_charged FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&invited.submission_id]).await.unwrap();
    assert_eq!(evidence.get::<_, Option<i32>>(0), None);
    assert_eq!(evidence.get::<_, Option<String>>(1), None);
    assert!(!evidence.get::<_, bool>(2));
    // Releasing unprocessed reservations is an explicit refund, not an evaluation reset.
    for reservation in [&first, &second, &third] {
        assert!(
            runtime
                .transition_account_admission(
                    &tenant,
                    &principal,
                    account_id,
                    reservation.submission_id,
                    reservation.lease_id,
                    "released"
                )
                .await
                .unwrap()
        );
    }
    admin.execute("UPDATE trace_account_invite_grants SET revoked_at=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap();
    admin.execute("UPDATE trace_account_trust SET authority='bounded',trust_version=3 WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap();
    assert!(
        runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "below-base evaluation is clamped up to the base"
    );
    assert_eq!(
        runtime
            .reserve_account_admission(&request())
            .await
            .unwrap()
            .decision,
        AdmissionDecision::Reserved
    );
    // Every current gate projection is part of the coherent input generation.
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='external_gate_login') THEN CREATE ROLE external_gate_login LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT USAGE ON SCHEMA public TO external_gate_login; GRANT SELECT(tenant_id,decision_id), UPDATE(credit_quality_micros,credit_quality_anomaly_ratio_micros,credit_quality_calibration_version,dedup_simhash,dedup_cluster_id,dedup_cluster_size,dedup_signal_version) ON trace_gate_decisions TO external_gate_login;").await.unwrap();
    let gate_writer = PgBackend::new(&config(login_url("external_gate_login")))
        .await
        .unwrap();
    let unsafe_gate_role: bool = gate_writer
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname=current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!unsafe_gate_role);
    assert!(
        evaluator
            .query("SELECT * FROM trace_account_trust_dependency_locks", &[])
            .await
            .is_err(),
        "internal lock keys cannot be read by evaluator login"
    );
    let cluster = Uuid::new_v4();
    let gate = seed_dependency_gate(&admin, &tenant, submission, cluster, false).await;
    admin.execute("INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,submission_id,outcome,evaluator_version,occurred_at) VALUES($1,$2,'gate_evaluation',$3,$4,'evaluated_passed','gate-fixture',clock_timestamp())", &[&tenant,&account_id,&gate,&submission]).await.unwrap();
    publish_current(&mut evaluator, &tenant, account_id).await;
    assert!(
        runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    gate_writer
        .update_trace_gate_decision_credit_quality(&tenant, gate, 100, 0, 1)
        .await
        .unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "quality correction invalidates applied evidence"
    );
    publish_current(&mut evaluator, &tenant, account_id).await;
    gate_writer
        .update_trace_gate_decision_credit_quality(&tenant, gate, 100, 0, 2)
        .await
        .unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "calibration correction invalidates applied evidence"
    );
    publish_current(&mut evaluator, &tenant, account_id).await;
    gate_writer
        .update_trace_gate_decision_dedup(
            &tenant,
            gate,
            DedupAssignmentWrite {
                dedup_simhash: 1,
                dedup_cluster_id: cluster,
                dedup_cluster_size: 1,
                dedup_signal_version: "signal-fixture-next".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "signal version correction invalidates applied evidence"
    );
    publish_current(&mut evaluator, &tenant, account_id).await;
    let next_cluster = Uuid::new_v4();
    gate_writer
        .update_trace_gate_decision_dedup_cluster(&tenant, gate, next_cluster, 1)
        .await
        .unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "local reclustering invalidates applied evidence"
    );
    // A new earlier member in another tenant changes this account's cluster input.
    let remote_tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&remote_tenant],
        )
        .await
        .unwrap();
    publish_current(&mut evaluator, &tenant, account_id).await;
    let remote_gate =
        seed_dependency_gate(&admin, &remote_tenant, Uuid::new_v4(), next_cluster, true).await;
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "cross-tenant cluster insertion invalidates local evaluation"
    );
    let tx = evaluator.transaction().await.unwrap();
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let local_input=tx.query_one("SELECT first_in_cluster FROM trace_account_trust_evaluation_inputs($1,$2) WHERE source_id=$3",&[&tenant,&account_id,&gate]).await.unwrap();
    assert_eq!(local_input.get::<_, Option<bool>>(0), Some(false));
    tx.commit().await.unwrap();
    publish_current(&mut evaluator, &tenant, account_id).await;
    gate_writer
        .update_trace_gate_decision_dedup_cluster(&remote_tenant, remote_gate, Uuid::new_v4(), 1)
        .await
        .unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "old cluster dependencies also invalidate across tenants"
    );
    gate_writer
        .update_trace_gate_decision_dedup_cluster(&remote_tenant, remote_gate, next_cluster, 1)
        .await
        .unwrap();
    publish_current(&mut evaluator, &tenant, account_id).await;
    admin
        .execute(
            "DELETE FROM trace_gate_decisions WHERE tenant_id=$1 AND decision_id=$2",
            &[&remote_tenant, &remote_gate],
        )
        .await
        .unwrap();
    assert!(
        !runtime
            .account_admission_status(&account, &principal, &policy)
            .await
            .unwrap()
            .unwrap()
            .ready,
        "cross-tenant cluster deletion invalidates local evaluation"
    );
    // The same race refuses an evaluator's pre-change repeatable-read publication.
    let tx = evaluator
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
    let generation: i64 = tx
        .query_one(
            "SELECT trace_account_trust_input_generation($1,$2)",
            &[&tenant, &account_id],
        )
        .await
        .unwrap()
        .get(0);
    tx.query(
        "SELECT * FROM trace_account_trust_evaluation_inputs($1,$2)",
        &[&tenant, &account_id],
    )
    .await
    .unwrap();
    gate_writer
        .update_trace_gate_decision_credit_quality(&tenant, gate, 99, 0, 2)
        .await
        .unwrap();
    assert!(tx.query_one("SELECT trace_record_external_account_trust_evaluation($1,$2,$3,'growth-fixture','applied',transaction_timestamp(),2,30,$4,$5)",&[&tenant,&Uuid::new_v4(),&account_id,&digest,&generation]).await.is_err(),"gate mutation refuses a stale batch publication");
    tx.rollback().await.unwrap();
    // A gate mutation waiting behind a new fact must enumerate that committed fact.
    let late_account = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &late_account],
        )
        .await
        .unwrap();
    let mut fact_client = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let fact_tx = fact_client.transaction().await.unwrap();
    fact_tx.execute("INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,submission_id,outcome,evaluator_version,occurred_at) VALUES($1,$2,'gate_evaluation',$3,$4,'evaluated_passed','gate-fixture',clock_timestamp())", &[&tenant,&late_account,&gate,&submission]).await.unwrap();
    let writer_for_race = PgBackend::new(&config(login_url("external_gate_login")))
        .await
        .unwrap();
    let race_tenant = tenant.clone();
    let mut mutation = tokio::spawn(async move {
        writer_for_race
            .update_trace_gate_decision_credit_quality(&race_tenant, gate, 98, 0, 2)
            .await
    });
    // Observe the real database lock wait, rather than relying on a scheduling delay.
    let wait_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting:bool=admin.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE usename='external_gate_login' AND wait_event_type='Lock')",&[]).await.unwrap().get(0);
        if waiting {
            break;
        }
        assert!(
            tokio::time::Instant::now() < wait_deadline,
            "gate writer must reach dependency lock wait"
        );
        tokio::task::yield_now().await;
    }
    assert!(!mutation.is_finished());
    fact_tx.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), &mut mutation)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let late_generation:i64=admin.query_one("SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2",&[&tenant,&late_account]).await.unwrap().get(0);
    assert_eq!(
        late_generation, 2,
        "newly committed dependency is included after gate lock wait"
    );
}
