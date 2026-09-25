// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit isolated PostgreSQL test: never falls back to DATABASE_URL or skips failure.
use std::sync::Arc;
use trace_commons_server::{
    account_trust::{
        TrustFactOutcome, TrustFactSource, parse_bounded_policy, resolve_contribution_account,
    },
    admission_ledger::{
        AccountAdmissionReservation, AdmissionDecision as D, AdmissionLimits, AdmissionReservation,
    },
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ACCOUNT_ADMISSION_PG_TEST_URL"]
async fn account_admission_atomicity_replay_and_revocation() {
    let url = std::env::var("TRACE_COMMONS_ACCOUNT_ADMISSION_PG_TEST_URL")
        .expect("explicit isolated account admission database URL required");
    let parsed = url.parse::<tokio_postgres::Config>().unwrap();
    assert!(
        matches!(parsed.get_hosts().first(), Some(tokio_postgres::config::Host::Tcp(host)) if host == "127.0.0.1" || host == "localhost")
    );
    assert!(
        parsed
            .get_dbname()
            .is_some_and(|name| name.starts_with("admission_test"))
    );
    let config = |url: String| DatabaseConfig {
        url: url.into(),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='admission_account_runtime') THEN CREATE ROLE admission_account_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT trace_account_admission_runtime TO admission_account_runtime;").await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url
        .set_username("admission_account_runtime")
        .unwrap();
    let runtime_url: String = runtime_url.into();
    let runtime = Arc::new(PgBackend::new(&config(runtime_url.clone())).await.unwrap());
    let anchor = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let account = Uuid::new_v4();
    let principal = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let other = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
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
            &[&tenant, &account],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,'fixture-pepper','fixture-key')", &[&tenant,&account,&anchor,&serde_json::json!({"fixture":true})]).await.unwrap();
    for (index, principal_ref) in [&principal, &other].into_iter().enumerate() {
        let device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
        admin.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near')", &[&device,&tenant,&format!("fixture-key-{index}")]).await.unwrap();
        admin.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant,&account,principal_ref]).await.unwrap();
        admin.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)", &[&tenant,principal_ref,&account,&device,&anchor]).await.unwrap();
    }
    let trust_account = resolve_contribution_account(&admin_db, &tenant, &principal)
        .await
        .unwrap();
    let policy = parse_bounded_policy(r#"{"version":"admission-test-v1","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#, &["admission-test-v1"]).unwrap();
    let request = |principal_ref: &str| AccountAdmissionReservation {
        account: trust_account.clone(),
        principal_ref: principal_ref.into(),
        expected_trust_version: None,
        submission_id: Uuid::new_v4(),
        body_hash: "b".repeat(64),
        lease_id: Uuid::new_v4(),
        policy: policy.clone(),
        lease_seconds: 60,
    };
    let first = request(&principal);
    let second = request(&other);
    let (a, b) = tokio::join!(
        runtime.reserve_account_admission(&first),
        runtime.reserve_account_admission(&second)
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()]
            .into_iter()
            .filter(|d| *d == D::Reserved)
            .count(),
        1,
        "two devices cannot overspend one account"
    );
    let winner = if runtime.reserve_account_admission(&first).await.unwrap() == D::Busy {
        &first
    } else {
        &second
    };
    assert_eq!(
        runtime.reserve_account_admission(winner).await.unwrap(),
        D::Busy
    );
    let mut changed = winner.clone();
    changed.body_hash = "c".repeat(64);
    assert_eq!(
        runtime.reserve_account_admission(&changed).await.unwrap(),
        D::Conflict
    );
    assert!(
        runtime
            .transition_account_admission(
                &tenant,
                &winner.principal_ref,
                account,
                winner.submission_id,
                winner.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    assert_eq!(
        runtime
            .reserve_account_admission(&request(&other))
            .await
            .unwrap(),
        D::Reserved,
        "pre-processing release refunds"
    );

    let invite = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    admin.execute("INSERT INTO trace_account_invite_grants(tenant_id,account_id,invite_subject_hash,trust_version) VALUES($1,$2,$3,2)", &[&tenant,&account,&invite]).await.unwrap();
    admin.execute("UPDATE trace_account_trust SET authority='invited',trust_version=2 WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap();
    let invited = request(&principal);
    assert_eq!(
        runtime.reserve_account_admission(&invited).await.unwrap(),
        D::Reserved,
        "invite bypasses exhausted cumulative allowance"
    );
    // Hold the grant row while processing attempts its liveness check. The
    // runtime transaction must visibly block on this independent revoke row.
    let mut revoke_client = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let revoke_tx = revoke_client.transaction().await.unwrap();
    revoke_tx.query_one("SELECT 1 FROM trace_account_invite_grants WHERE tenant_id=$1 AND account_id=$2 FOR UPDATE", &[&tenant,&account]).await.unwrap();
    let runtime_for_race = runtime.clone();
    let tenant_for_race = tenant.clone();
    let principal_for_race = principal.clone();
    let invited_for_race = invited.clone();
    let processing = tokio::spawn(async move {
        runtime_for_race
            .transition_account_admission(
                &tenant_for_race,
                &principal_for_race,
                account,
                invited_for_race.submission_id,
                invited_for_race.lease_id,
                "processing",
            )
            .await
            .unwrap()
    });
    let observed_block = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: bool = admin.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE query LIKE 'SELECT trace_account_admission_active_grant%' AND cardinality(pg_blocking_pids(pid)) > 0)", &[]).await.unwrap().get(0);
            if blocked { break true; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert!(observed_block);
    revoke_tx.execute("UPDATE trace_account_invite_grants SET revoked_at=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account]).await.unwrap();
    revoke_tx.commit().await.unwrap();
    assert!(
        !processing.await.unwrap(),
        "revoked grant cannot start processing"
    );
    let retry_policy = parse_bounded_policy(
        r#"{"version":"admission-test-v2","processing_cost_bound":10,"bounded_allowance":30,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
        &["admission-test-v2"],
    ).unwrap();
    let mut work = request(&principal);
    work.policy = retry_policy.clone();
    assert_eq!(
        runtime.reserve_account_admission(&work).await.unwrap(),
        D::Reserved
    );
    assert!(
        runtime
            .transition_account_admission(
                &tenant,
                &principal,
                account,
                work.submission_id,
                work.lease_id,
                "processing"
            )
            .await
            .unwrap()
    );
    assert!(
        !runtime
            .transition_account_admission(
                &tenant,
                &principal,
                account,
                work.submission_id,
                work.lease_id,
                "released"
            )
            .await
            .unwrap(),
        "processing work never refunds"
    );
    admin.execute("UPDATE trace_account_admission_submissions SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&work.submission_id]).await.unwrap();
    let mut retry = work.clone();
    retry.lease_id = Uuid::new_v4();
    assert_eq!(
        runtime.reserve_account_admission(&retry).await.unwrap(),
        D::Reserved,
        "expired processing lease charges again"
    );
    let used: i64 = admin.query_one("SELECT cost_used FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND policy_version=$3", &[&tenant,&account,&retry_policy.version()]).await.unwrap().get(0);
    assert_eq!(used, 20);
    assert!(
        runtime
            .transition_account_admission(
                &tenant,
                &principal,
                account,
                retry.submission_id,
                retry.lease_id,
                "processing"
            )
            .await
            .unwrap()
    );
    assert!(
        runtime
            .transition_account_admission(
                &tenant,
                &principal,
                account,
                retry.submission_id,
                retry.lease_id,
                "completed"
            )
            .await
            .unwrap()
    );
    assert_eq!(
        runtime.reserve_account_admission(&retry).await.unwrap(),
        D::Completed,
        "terminal retry costs zero"
    );
    let unchanged: i64 = admin.query_one("SELECT cost_used FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2 AND policy_version=$3", &[&tenant,&account,&retry_policy.version()]).await.unwrap().get(0);
    assert_eq!(unchanged, 20);
    let mut conflict = retry.clone();
    conflict.body_hash = "a".repeat(64);
    assert_eq!(
        runtime.reserve_account_admission(&conflict).await.unwrap(),
        D::Conflict
    );
    drop(runtime);
    let restarted = PgBackend::new(&config(runtime_url)).await.unwrap();
    assert_eq!(
        restarted.reserve_account_admission(&retry).await.unwrap(),
        D::Completed,
        "process restart preserves terminal identity and charge"
    );
    let fixed_policy = parse_bounded_policy(
        r#"{"version":"admission-test-fixed","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"fixed","seconds":2},"growth_rule":"none"}"#,
        &["admission-test-fixed"],
    ).unwrap();
    let mut fixed = request(&principal);
    fixed.policy = fixed_policy.clone();
    assert_eq!(
        restarted.reserve_account_admission(&fixed).await.unwrap(),
        D::Reserved
    );
    let mut next = request(&other);
    next.policy = fixed_policy.clone();
    assert_eq!(
        restarted.reserve_account_admission(&next).await.unwrap(),
        D::Exhausted
    );
    let status = restarted
        .account_admission_status(&trust_account, &principal, &fixed_policy)
        .await
        .unwrap()
        .unwrap();
    assert!(!status.ready);
    let delay = status
        .retry_after_seconds
        .expect("fixed period has a next boundary");
    assert!((1..=2).contains(&delay));
    tokio::time::sleep(std::time::Duration::from_secs((delay + 1) as u64)).await;
    assert_eq!(
        restarted.reserve_account_admission(&next).await.unwrap(),
        D::Reserved,
        "explicit period boundary replenishes"
    );

    let source_submission = Uuid::new_v4();
    let source_trace = Uuid::new_v4();
    admin.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash) VALUES($1,$2,$3,$4,'v1','v1','test','accepted','low','test',$5)", &[&tenant,&source_submission,&source_trace,&principal,&"a".repeat(64)]).await.unwrap();
    assert_eq!(
        restarted
            .record_account_trust_fact(
                &trust_account,
                TrustFactSource::AcceptedSubmission(source_submission)
            )
            .await
            .unwrap(),
        None,
        "status alone is not an accepted fact"
    );
    admin.execute("INSERT INTO trace_credit_ledger(tenant_id,credit_event_id,submission_id,trace_id,credit_account_ref,event_type,points_delta,reason,actor_principal_ref,actor_role,settlement_state) VALUES($1,$2,$3,$4,'fixture','accepted','0','fixture',$5,'system','pending')", &[&tenant,&Uuid::new_v4(),&source_submission,&source_trace,&principal]).await.unwrap();
    assert_eq!(
        restarted
            .record_account_trust_fact(
                &trust_account,
                TrustFactSource::AcceptedSubmission(source_submission)
            )
            .await
            .unwrap(),
        Some(TrustFactOutcome::Accepted)
    );
    assert_eq!(
        restarted
            .record_account_trust_fact(
                &trust_account,
                TrustFactSource::AcceptedSubmission(source_submission)
            )
            .await
            .unwrap(),
        Some(TrustFactOutcome::Accepted)
    );
    let fact_count: i64 = admin.query_one("SELECT count(*) FROM trace_account_trust_facts WHERE tenant_id=$1 AND account_id=$2 AND source_kind='accepted_submission'", &[&tenant,&account]).await.unwrap().get(0);
    assert_eq!(fact_count, 1, "stable source cannot count twice");
    let evaluation = Uuid::new_v4();
    admin.execute("UPDATE trace_submissions SET status='quarantined' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&source_submission]).await.unwrap();
    admin.execute("INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,attestation_chain_hash) VALUES($1,$2,$3,'test-evaluator-v1',$4,1,1,TRUE,1,$4,TRUE,$4,$4)", &[&tenant,&evaluation,&source_submission,&"a".repeat(64)]).await.unwrap();
    assert_eq!(
        restarted
            .record_account_trust_fact(&trust_account, TrustFactSource::GateEvaluation(evaluation))
            .await
            .unwrap(),
        Some(TrustFactOutcome::EvaluatedNotAccepted),
        "declined/quarantined work gets no positive outcome"
    );

    // Cutover recovery stays on V59's original authority. Released work had
    // refunded its bound; a crashed processing lease retains its old charge.
    let legacy_anchor = anchor.strip_prefix("sha256:").unwrap();
    let legacy_submission = Uuid::new_v4();
    let legacy_body = "b".repeat(64);
    admin.execute("INSERT INTO trace_admission_global_budget(singleton,cost_limit,cost_bound_used) VALUES(TRUE,100,0)", &[]).await.unwrap();
    admin.execute("INSERT INTO trace_admission_accounts(tenant_id,anchor_hash,attempt_limit,cost_limit) VALUES($1,$2,10,100)", &[&tenant,&legacy_anchor]).await.unwrap();
    admin.execute("INSERT INTO trace_admission_submissions(tenant_id,submission_id,anchor_hash,body_hash,kind,status,lease_id,lease_expires_at,last_cost_bound,attempt_held,ever_processed) VALUES($1,$2,$3,$4,'window','released',$5,now()+interval '60 seconds',10,FALSE,FALSE)", &[&tenant,&legacy_submission,&legacy_anchor,&legacy_body,&Uuid::new_v4()]).await.unwrap();
    let first_legacy_lease = Uuid::new_v4();
    assert_eq!(
        restarted
            .resume_legacy_admission(
                &tenant,
                legacy_anchor,
                legacy_submission,
                &legacy_body,
                first_legacy_lease,
                60
            )
            .await
            .unwrap(),
        D::Reserved,
        "released legacy reservation is reclaimable without account authority"
    );
    assert_eq!(
        restarted
            .resume_legacy_admission(
                &tenant,
                legacy_anchor,
                legacy_submission,
                &legacy_body,
                Uuid::new_v4(),
                60
            )
            .await
            .unwrap(),
        D::Busy,
        "unexpired legacy lease is not stolen"
    );
    assert_eq!(
        restarted
            .resume_legacy_admission(
                &tenant,
                legacy_anchor,
                legacy_submission,
                &"c".repeat(64),
                Uuid::new_v4(),
                60
            )
            .await
            .unwrap(),
        D::Conflict,
        "the old UUID never changes exact-body identity"
    );
    let charged_once: i64 = admin
        .query_one(
            "SELECT cost_bound_used FROM trace_admission_global_budget WHERE singleton",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(charged_once, 10);
    admin.execute("UPDATE trace_admission_submissions SET status='processing',ever_processed=TRUE,lease_expires_at=now()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&legacy_submission]).await.unwrap();
    assert_eq!(
        restarted
            .resume_legacy_admission(
                &tenant,
                legacy_anchor,
                legacy_submission,
                &legacy_body,
                Uuid::new_v4(),
                60
            )
            .await
            .unwrap(),
        D::Reserved,
        "expired processing lease is retried under V59 while old charge remains"
    );
    let charged_twice: i64 = admin
        .query_one(
            "SELECT cost_bound_used FROM trace_admission_global_budget WHERE singleton",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(charged_twice, 20, "one bound per actual processing attempt");
    let account_rows: i64 = admin.query_one("SELECT count(*) FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&legacy_submission]).await.unwrap().get(0);
    assert_eq!(
        account_rows, 0,
        "legacy retry cannot create a second authority"
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_PG_TEST_URL"]
async fn atomic_admission_replay_budget_recovery_and_rls() {
    let url = std::env::var("TRACE_COMMONS_ADMISSION_PG_TEST_URL")
        .expect("explicit isolated test URL required");
    let target = url.parse::<tokio_postgres::Config>().unwrap();
    assert!(
        matches!(target.get_hosts().first(), Some(tokio_postgres::config::Host::Tcp(host)) if host == "127.0.0.1" || host == "localhost")
    );
    assert!(
        target
            .get_dbname()
            .is_some_and(|name| name.starts_with("admission_test")),
        "dedicated admission_test database required before destructive fixtures"
    );
    let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        connection.await.unwrap();
    });
    admin.batch_execute("CREATE OR REPLACE FUNCTION trace_current_tenant_id() RETURNS TEXT LANGUAGE SQL STABLE AS $$ SELECT NULLIF(current_setting('trace_commons.trace_tenant_id',true),'') $$;").await.unwrap();
    admin
        .batch_execute(include_str!(
            "../../../migrations/V59__trace_admission_ledger.sql"
        ))
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='admission_test_runtime') THEN CREATE ROLE admission_test_runtime LOGIN NOBYPASSRLS; END IF; END $$;
        GRANT USAGE ON SCHEMA public TO admission_test_runtime;
        GRANT SELECT,INSERT,UPDATE ON trace_admission_challenges,trace_admission_accounts,trace_admission_submissions TO admission_test_runtime;
        GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),trace_transition_admission(TEXT,UUID,UUID,TEXT) TO admission_test_runtime;
        TRUNCATE trace_admission_challenges,trace_admission_accounts,trace_admission_submissions,trace_admission_receipts,trace_admission_global_budget;").await.unwrap();
    let mut parsed = url.parse::<tokio_postgres::Config>().unwrap();
    parsed.user("admission_test_runtime");
    let host = parsed.get_hosts().first().unwrap();
    let tokio_postgres::config::Host::Tcp(host) = host else {
        panic!("TCP localhost required")
    };
    assert!(
        host == "127.0.0.1" || host == "localhost",
        "isolated localhost only"
    );
    let runtime_url = format!(
        "postgresql://admission_test_runtime@{host}:{}/{}",
        parsed.get_ports()[0],
        parsed.get_dbname().unwrap()
    );
    let backend = PgBackend::new(&DatabaseConfig {
        url: runtime_url.clone().into(),
        pool_size: 1,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
    .await
    .unwrap();
    let limits = AdmissionLimits {
        window_attempts: 1,
        account_cost_limit: 50,
        global_cost_limit: 100,
        processing_cost_bound: 10,
        lease_seconds: 60,
        challenge_ttl_seconds: 60,
    };
    let make = |tenant: &str, anchor: &str| AdmissionReservation {
        tenant_id: tenant.into(),
        anchor_hash: anchor.repeat(64),
        submission_id: Uuid::new_v4(),
        body_hash: "b".repeat(64),
        receipt_hash: None,
        challenge_hash: None,
        lease_id: Uuid::new_v4(),
        limits: limits.clone(),
    };
    let mut first = make("tenant-a", "a");
    let (left, right) = tokio::join!(
        backend.reserve_submission_admission(&first),
        backend.reserve_submission_admission(&first)
    );
    let decisions = [left.unwrap(), right.unwrap()];
    assert!(decisions.contains(&D::Reserved) && decisions.contains(&D::Busy));
    assert_eq!(
        backend
            .reserve_submission_admission(&make("tenant-a", "a"))
            .await
            .unwrap(),
        D::Exhausted
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    first.lease_id = Uuid::new_v4();
    assert_eq!(
        backend.reserve_submission_admission(&first).await.unwrap(),
        D::Reserved
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "processing"
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "released"
            )
            .await
            .unwrap()
    );
    // A crashed processing attempt retains its bound and slot. A fresh
    // lease reserves another bound; the stale holder cannot complete it.
    let old_lease = first.lease_id;
    admin.execute("UPDATE trace_admission_submissions SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE submission_id=$1",&[&first.submission_id]).await.unwrap();
    first.lease_id = Uuid::new_v4();
    assert_eq!(
        backend.reserve_submission_admission(&first).await.unwrap(),
        D::Reserved
    );
    assert!(
        !backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                old_lease,
                "completed"
            )
            .await
            .unwrap()
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "processing"
            )
            .await
            .unwrap()
    );
    assert!(
        backend
            .transition_submission_admission(
                "tenant-a",
                first.submission_id,
                first.lease_id,
                "completed"
            )
            .await
            .unwrap()
    );
    assert_eq!(
        backend.reserve_submission_admission(&first).await.unwrap(),
        D::Completed
    );
    let mut changed = first.clone();
    changed.body_hash = "c".repeat(64);
    assert_eq!(
        backend
            .reserve_submission_admission(&changed)
            .await
            .unwrap(),
        D::Conflict
    );
    let lock = backend
        .acquire_admission_processing_lock("tenant-a", first.submission_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        backend
            .acquire_admission_processing_lock("tenant-a", first.submission_id)
            .await
            .unwrap()
            .is_none()
    );
    let second_guard = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        backend.acquire_admission_processing_lock("tenant-other", Uuid::new_v4()),
    )
    .await
    .expect("a held guard must not occupy the sole pool slot")
    .unwrap()
    .unwrap();
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.reserve_submission_admission(&first)
        )
        .await
        .expect("two guards cannot starve ledger pool")
        .unwrap(),
        D::Completed
    );
    drop(second_guard);
    drop(lock);
    let mut reacquired = None;
    for _ in 0..20 {
        reacquired = backend
            .acquire_admission_processing_lock("tenant-a", first.submission_id)
            .await
            .unwrap();
        if reacquired.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(reacquired.is_some());
    drop(reacquired);
    let mut attested = make("tenant-a", "a");
    attested.receipt_hash = Some("d".repeat(64));
    attested.challenge_hash = Some("e".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-a",
            &attested.anchor_hash,
            attested.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .reserve_submission_admission(&attested)
            .await
            .unwrap(),
        D::Reserved
    );
    let mut reused = make("tenant-b", "f");
    reused.receipt_hash = attested.receipt_hash.clone();
    reused.challenge_hash = Some("f".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-b",
            &reused.anchor_hash,
            reused.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend.reserve_submission_admission(&reused).await.unwrap(),
        D::Refused
    );
    reused.receipt_hash = Some("1".repeat(64));
    assert_eq!(
        backend.reserve_submission_admission(&reused).await.unwrap(),
        D::Reserved,
        "refused replay must not consume challenge"
    );
    let mut expired = make("tenant-c", "2");
    expired.receipt_hash = Some("2".repeat(64));
    expired.challenge_hash = Some("3".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-c",
            &expired.anchor_hash,
            expired.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() - chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .reserve_submission_admission(&expired)
            .await
            .unwrap(),
        D::Refused
    );
    let mut wrong = make("tenant-d", "4");
    wrong.receipt_hash = Some("4".repeat(64));
    wrong.challenge_hash = Some("5".repeat(64));
    backend
        .issue_admission_challenge(
            "tenant-d",
            &"5".repeat(64),
            wrong.challenge_hash.as_ref().unwrap(),
            chrono::Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(
        backend.reserve_submission_admission(&wrong).await.unwrap(),
        D::Refused
    );
    // Competing tenants share the same global ceiling, enforced atomically.
    let mut a = make("tenant-e", "6");
    a.limits.processing_cost_bound = 50;
    let mut b = make("tenant-f", "7");
    b.limits.processing_cost_bound = 50;
    let (a, b) = tokio::join!(
        backend.reserve_submission_admission(&a),
        backend.reserve_submission_admission(&b)
    );
    let results = [a.unwrap(), b.unwrap()];
    assert!(results.contains(&D::Reserved) && results.contains(&D::Exhausted));
    let (client, conn) = tokio_postgres::connect(&runtime_url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        conn.await.unwrap();
    });
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM trace_admission_submissions", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0,
        "missing tenant must hide all rows"
    );
    assert!(
        client
            .query("SELECT * FROM trace_admission_receipts", &[])
            .await
            .is_err(),
        "runtime cannot inspect global receipt hashes"
    );
    let row = admin
        .query_one(
            "SELECT cost_bound_used FROM trace_admission_global_budget",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 90);
}
