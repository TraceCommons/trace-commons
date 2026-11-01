// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI starter advance's plausibility features against a real
//! PostgreSQL (V131): the write-once record, the account read's filters,
//! tenant isolation under the ingest runtime role, and the dry-run report
//! over the earned-trust enumeration seam. Explicit isolated database only;
//! never falls back to DATABASE_URL.

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use trace_commons_server::account_trust::TrustAccount;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::starter_advance_report::starter_dry_run_report;
use trace_commons_server::starter_plausibility::{
    ModelFamily, STARTER_FEATURES_VERSION, StarterAgent, StarterEnvelopeFeatures,
};
use uuid::Uuid;

const URL_VAR: &str = "TRACE_COMMONS_STARTER_PG_TEST_DATABASE_URL";

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

struct Fixture {
    admin_db: PgBackend,
    admin: deadpool_postgres::Object,
    url: String,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// The report enumerates every account in the database, so the suite runs
/// one test at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var(URL_VAR) else {
        eprintln!("SKIPPED: isolated starter-advance PostgreSQL fixture required ({URL_VAR})");
        return None;
    };
    let serial = SERIAL.lock().await;
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_starter"));
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    Some(Fixture {
        admin_db,
        admin,
        url,
        _serial: serial,
    })
}

/// A NOBYPASSRLS login that holds exactly one NOLOGIN role.
async fn login_with_role(fx: &Fixture, login: &str, role: &str) -> Arc<PgBackend> {
    fx.admin
        .batch_execute(&format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{login}') \
             THEN CREATE ROLE {login} LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; \
             GRANT {role} TO {login};"
        ))
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(&fx.url).unwrap();
    url.set_username(login).unwrap();
    Arc::new(PgBackend::new(&config(url.into())).await.unwrap())
}

fn anchored_tenant() -> String {
    format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2))
}

fn principal() -> String {
    format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2))
}

fn at(hours: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap() + Duration::hours(hours)
}

async fn seed_account(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    principal_ref: &str,
) -> Uuid {
    let account = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
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
    admin
        .execute(
            "INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",
            &[&tenant, &account, &principal_ref],
        )
        .await
        .unwrap();
    account
}

async fn seed_submission(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    principal_ref: &str,
    status: &str,
    received_at: DateTime<Utc>,
) -> Uuid {
    let submission = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,
                schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,
                redaction_pipeline_version,redaction_hash,received_at)
             VALUES($1,$2,$3,$4,'v1','v1','test',$5,'low','test',$6,$7)",
            &[
                &tenant,
                &submission,
                &Uuid::new_v4(),
                &principal_ref,
                &status,
                &"a".repeat(64),
                &received_at,
            ],
        )
        .await
        .unwrap();
    submission
}

/// One Claude Code envelope's features, distinct per `seed`.
fn features(seed: u32) -> StarterEnvelopeFeatures {
    StarterEnvelopeFeatures {
        version: STARTER_FEATURES_VERSION.to_string(),
        agent: StarterAgent::ClaudeCode,
        session_hash: Some(format!("sha256:{seed:064x}")),
        content_hash: format!("sha256:{:064x}", u64::from(seed) + 1_000),
        first_event_hour: Some(at(i64::from(seed))),
        last_event_hour: Some(at(i64::from(seed) + 1)),
        future_dated: false,
        model_family: ModelFamily::Anthropic,
        tool_calls: 20,
        tool_results: 20,
        paired_tool_calls: 20,
        failed_tool_results: 1,
        known_tool_calls: 20,
        turn_gaps: 39,
        think_time_gaps: 20,
        gap_bucket_mask: 0b11_1111_1110,
        latency_bucket_mask: 0b10_1110_0000,
        assistant_events: 20,
        token_events: 20,
        input_token_steps: 19,
        input_token_rises: 19,
        cost_events: 20,
    }
}

async fn find_account(db: &PgBackend, tenant: &str, account: Uuid) -> TrustAccount {
    let mut after: Option<TrustAccount> = None;
    loop {
        let page = db
            .list_account_trust_worker_accounts(after.as_ref(), 500)
            .await
            .unwrap();
        if let Some(found) = page
            .iter()
            .find(|a| a.tenant_id() == tenant && a.account_id() == account)
        {
            return found.clone();
        }
        after = Some(page.last().cloned().expect("account is enumerated"));
    }
}

#[tokio::test]
async fn features_are_written_once_and_read_for_the_owning_account_only() {
    let Some(fx) = fixture().await else { return };
    let tenant = anchored_tenant();
    let mine = principal();
    let theirs = principal();
    let account = seed_account(&fx.admin, &tenant, &mine).await;
    let other = seed_account(&fx.admin, &tenant, &theirs).await;

    let first = seed_submission(&fx.admin, &tenant, &mine, "accepted", at(1)).await;
    let second = seed_submission(&fx.admin, &tenant, &mine, "accepted", at(2)).await;
    let quarantined = seed_submission(&fx.admin, &tenant, &mine, "quarantined", at(3)).await;
    let withdrawn = seed_submission(&fx.admin, &tenant, &mine, "accepted", at(4)).await;
    let someone_elses = seed_submission(&fx.admin, &tenant, &theirs, "accepted", at(5)).await;
    fx.admin
        .execute(
            "UPDATE trace_submissions SET withdrawn_at = now() WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &withdrawn],
        )
        .await
        .unwrap();

    let db = &fx.admin_db;
    assert!(
        db.record_starter_plausibility_features(&tenant, first, &features(1))
            .await
            .unwrap()
    );
    assert!(
        !db.record_starter_plausibility_features(&tenant, first, &features(99))
            .await
            .unwrap(),
        "a second record for one submission is a no-op"
    );
    for (submission, seed) in [
        (second, 2),
        (quarantined, 3),
        (withdrawn, 4),
        (someone_elses, 5),
    ] {
        assert!(
            db.record_starter_plausibility_features(&tenant, submission, &features(seed))
                .await
                .unwrap()
        );
    }

    let read = db
        .starter_plausibility_features_for_account(&find_account(db, &tenant, account).await)
        .await
        .unwrap();
    assert_eq!(
        read,
        vec![features(1), features(2)],
        "accepted, unwithdrawn, own, oldest first"
    );

    let theirs_read = db
        .starter_plausibility_features_for_account(&find_account(db, &tenant, other).await)
        .await
        .unwrap();
    assert_eq!(theirs_read, vec![features(5)]);

    // An unlinked principal's submissions stop counting for the account.
    fx.admin
        .execute(
            "UPDATE trace_account_principals SET unlinked_at = now()
              WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    assert!(
        db.starter_plausibility_features_for_account(&find_account(db, &tenant, account).await)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_record_needs_its_submission() {
    let Some(fx) = fixture().await else { return };
    let tenant = anchored_tenant();
    let mine = principal();
    seed_account(&fx.admin, &tenant, &mine).await;
    assert!(
        fx.admin_db
            .record_starter_plausibility_features(&tenant, Uuid::new_v4(), &features(1))
            .await
            .is_err(),
        "the foreign key refuses a record for an unknown submission"
    );
}

#[tokio::test]
async fn the_ingest_runtime_records_and_reads_within_its_tenant_only() {
    let Some(fx) = fixture().await else { return };
    // V90: the runtime's privileges on tables older than V62 are provisioned
    // by the operator, not by migrations. Model the two the account read
    // joins, only in this fresh test database, as activity_missions_pg does.
    fx.admin
        .batch_execute(
            "GRANT SELECT ON trace_submissions, trace_account_principals TO trace_ingest_runtime",
        )
        .await
        .unwrap();
    let runtime = login_with_role(&fx, "tc_starter_runtime_login", "trace_ingest_runtime").await;
    let tenant_a = anchored_tenant();
    let tenant_b = anchored_tenant();
    let principal_a = principal();
    let account_a = seed_account(&fx.admin, &tenant_a, &principal_a).await;
    seed_account(&fx.admin, &tenant_b, &principal()).await;
    let submission_a = seed_submission(&fx.admin, &tenant_a, &principal_a, "accepted", at(1)).await;

    assert!(
        runtime
            .record_starter_plausibility_features(&tenant_a, submission_a, &features(1))
            .await
            .unwrap()
    );
    // Recording tenant A's submission under tenant B's context sees no
    // such submission: the row's tenant must match the context.
    assert!(
        runtime
            .record_starter_plausibility_features(&tenant_b, submission_a, &features(2))
            .await
            .is_err()
    );
    let account = find_account(&fx.admin_db, &tenant_a, account_a).await;
    assert_eq!(
        runtime
            .starter_plausibility_features_for_account(&account)
            .await
            .unwrap(),
        vec![features(1)]
    );
    // The runtime cannot update or delete a record.
    let privileges = fx
        .admin
        .query_one(
            "SELECT has_table_privilege('trace_ingest_runtime','trace_starter_plausibility_features','UPDATE'),
                    has_table_privilege('trace_ingest_runtime','trace_starter_plausibility_features','DELETE')",
            &[],
        )
        .await
        .unwrap();
    assert!(!privileges.get::<_, bool>(0));
    assert!(!privileges.get::<_, bool>(1));
}

#[tokio::test]
async fn the_dry_run_report_reads_through_the_database() {
    let Some(fx) = fixture().await else { return };
    let tenant = anchored_tenant();
    let mine = principal();
    let account = seed_account(&fx.admin, &tenant, &mine).await;
    for seed in 0..9u32 {
        let submission =
            seed_submission(&fx.admin, &tenant, &mine, "accepted", at(i64::from(seed))).await;
        let mut record = features(seed);
        // Nine sessions over three days, ending well inside the age limit.
        let start = Utc::now() - Duration::days(4) + Duration::hours(i64::from(seed) * 8);
        record.first_event_hour = Some(start);
        record.last_event_hour = Some(start + Duration::hours(1));
        fx.admin_db
            .record_starter_plausibility_features(&tenant, submission, &record)
            .await
            .unwrap();
    }
    let _ = find_account(&fx.admin_db, &tenant, account).await;

    let db: &dyn Database = &fx.admin_db;
    let report = starter_dry_run_report(db, Some(10_000), Utc::now())
        .await
        .unwrap();
    assert!(report.accounts_scanned >= 1);
    assert!(
        report.would_grant >= 1,
        "the seeded account qualifies: {report:?}"
    );
    assert_eq!(report.failed, 0);
}

#[tokio::test]
async fn device_upload_features_follow_the_parent_account_membership() {
    use trace_commons_server::account_trust::{
        device_claim_actor, device_principal_ref, device_upload_principal_refs,
    };

    let Some(fx) = fixture().await else { return };
    let tenant = anchored_tenant();
    let device = principal();
    let parent = device_principal_ref(&tenant, &device);
    let account_id = seed_account(&fx.admin, &tenant, &parent).await;
    fx.admin
        .execute(
            "INSERT INTO device_keys(tenant_id,device_key_id,public_key,onboarding_origin)
         VALUES($1,$2,'fixture-public-key','near')",
            &[&tenant, &device],
        )
        .await
        .unwrap();
    fx.admin
        .execute(
            "INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,
            sealed_account_name,index_pepper_ref,account_name_key_ref)
         VALUES($1,$2,$3,'{}',$3,$3)",
            &[&tenant, &account_id, &device],
        )
        .await
        .unwrap();
    fx.admin
        .execute(
            "INSERT INTO trace_near_provisioned_devices(tenant_id,account_id,principal_ref,
            device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$4)",
            &[&tenant, &account_id, &parent, &device],
        )
        .await
        .unwrap();
    let subject = trace_commons_protocol::onboarding::user_subject_hash(&device);
    let actor = device_claim_actor(&tenant, &device, Some(&subject));
    let [alias, _] = device_upload_principal_refs(&tenant, &device, &actor).unwrap();
    fx.admin_db
        .bind_device_upload_principal(&tenant, &device, &actor)
        .await
        .unwrap();
    let submission = seed_submission(&fx.admin, &tenant, &alias, "accepted", at(1)).await;
    fx.admin
        .batch_execute(
            "GRANT SELECT ON trace_submissions, trace_account_principals TO trace_ingest_runtime",
        )
        .await
        .unwrap();
    let runtime = login_with_role(
        &fx,
        "tc_starter_upload_runtime_login",
        "trace_ingest_runtime",
    )
    .await;
    runtime
        .record_starter_plausibility_features(&tenant, submission, &features(7))
        .await
        .unwrap();
    let account = find_account(&fx.admin_db, &tenant, account_id).await;
    assert_eq!(
        runtime
            .starter_plausibility_features_for_account(&account)
            .await
            .unwrap(),
        vec![features(7)],
        "the account must count accepted contributions made with its device's upload claim",
    );

    fx.admin
        .execute(
            "UPDATE trace_account_principals SET unlinked_at=now()
         WHERE tenant_id=$1 AND account_id=$2 AND principal_ref=$3",
            &[&tenant, &account_id, &parent],
        )
        .await
        .unwrap();
    assert!(
        runtime
            .starter_plausibility_features_for_account(&account)
            .await
            .unwrap()
            .is_empty(),
        "an upload association must not outlive its parent account membership"
    );
}
