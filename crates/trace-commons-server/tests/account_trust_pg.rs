// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use secrecy::SecretString;
use std::sync::Arc;
use std::time::Duration;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{AccountInviteRedemption as Outcome, Database, postgres::PgBackend};
use trace_commons_server::trace_upload_claim_allowlist::hash_invite_code;
use uuid::Uuid;

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn seed_account(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    account: Uuid,
    anchor: bool,
) {
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    if anchor {
        let anchor_hash = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
        admin
            .execute(
                "INSERT INTO trace_near_account_anchors
                    (tenant_id, account_id, anchor_hash, sealed_account_name,
                     index_pepper_ref, account_name_key_ref)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &tenant,
                    &account,
                    &anchor_hash,
                    &serde_json::json!({"test_fixture": true}),
                    &"fixture-pepper",
                    &"fixture-key",
                ],
            )
            .await
            .unwrap();
    }
}

async fn seed_invite(admin: &deadpool_postgres::Object, code: &str, max_uses: i32) -> String {
    let hash = hash_invite_code(code);
    admin
        .execute(
            "INSERT INTO onboarding_invite_grants
                (invite_subject_hash, policy_label, tenant_mode, tenant_template_id,
                 policy_version, max_uses, issuance_source)
             VALUES ($1, 'pilot', 'derived', 'pilot', 'v1', $2, 'test')",
            &[&hash, &max_uses],
        )
        .await
        .unwrap();
    hash
}

#[tokio::test]
async fn account_invite_is_atomic_idempotent_and_rls_scoped() {
    let Ok(url) = std::env::var("TRACE_COMMONS_ACCOUNT_TRUST_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated account trust PostgreSQL fixture required");
        return;
    };
    assert!(url.contains("127.0.0.1/trace_account_z3_invite_test"));
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_account_invite_runtime') THEN CREATE ROLE tc_account_invite_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT trace_account_invite_runtime TO tc_account_invite_runtime;").await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url
        .set_username("tc_account_invite_runtime")
        .unwrap();
    let runtime = Arc::new(PgBackend::new(&config(runtime_url.into())).await.unwrap());

    let tenant_a = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let tenant_b = format!("nearai-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    seed_account(&admin, &tenant_a, a, true).await;
    seed_account(&admin, &tenant_b, b, true).await;
    seed_account(&admin, &tenant_a, c, false).await;
    // V77 broadens this V75 constraint when account admission is installed.
    // Reproduce that later schema here to test redemption of a bounded account
    // without pulling the account admission migration into this foundation PR.
    admin
        .batch_execute(
            "ALTER TABLE trace_account_trust DROP CONSTRAINT trace_account_trust_authority_check;
        ALTER TABLE trace_account_trust ADD CONSTRAINT trace_account_trust_authority_check
        CHECK (authority IN ('bounded', 'invited'));",
        )
        .await
        .unwrap();
    let bounded = Uuid::new_v4();
    seed_account(&admin, &tenant_a, bounded, true).await;
    admin
        .execute(
            "INSERT INTO trace_account_trust(tenant_id,account_id,authority,trust_version)
        VALUES($1,$2,'bounded',1)",
            &[&tenant_a, &bounded],
        )
        .await
        .unwrap();
    let promotion_invite = seed_invite(&admin, &format!("INVITE-{}", Uuid::new_v4()), 1).await;
    let promotion_key = Uuid::new_v4();
    let promoted = runtime
        .redeem_account_invite(&tenant_a, bounded, &promotion_invite, promotion_key)
        .await
        .unwrap();
    assert_eq!(promoted, Outcome::Invited { trust_version: 2 });
    let promoted_row = admin
        .query_one(
            "SELECT authority,trust_version FROM trace_account_trust
        WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant_a, &bounded],
        )
        .await
        .unwrap();
    assert_eq!(promoted_row.get::<_, String>(0), "invited");
    assert_eq!(promoted_row.get::<_, i64>(1), 2);
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, bounded, &promotion_invite, promotion_key)
            .await
            .unwrap(),
        promoted,
        "replay returns the same promotion"
    );
    let promotion_uses: i32 = admin
        .query_one(
            "SELECT consumed_uses FROM onboarding_invite_grants
        WHERE invite_subject_hash=$1",
            &[&promotion_invite],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(promotion_uses, 1, "replay must not consume a second use");
    let first = seed_invite(&admin, &format!("INVITE-{}", Uuid::new_v4()), 1).await;
    let same_key = Uuid::new_v4();
    let (left, right) = tokio::join!(
        runtime.redeem_account_invite(&tenant_a, a, &first, same_key),
        runtime.redeem_account_invite(&tenant_a, a, &first, same_key)
    );
    assert_eq!(left.unwrap(), Outcome::Invited { trust_version: 1 });
    assert_eq!(right.unwrap(), Outcome::Invited { trust_version: 1 });
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, a, &first, Uuid::new_v4())
            .await
            .unwrap(),
        Outcome::Invited { trust_version: 1 }
    );
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_b, b, &first, Uuid::new_v4())
            .await
            .unwrap(),
        Outcome::InvalidInvite
    );
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, c, &first, Uuid::new_v4())
            .await
            .unwrap(),
        Outcome::AccountIneligible
    );
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_b, a, &first, Uuid::new_v4())
            .await
            .unwrap(),
        Outcome::AccountIneligible
    );

    let second = seed_invite(&admin, &format!("INVITE-{}", Uuid::new_v4()), 1).await;
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, a, &second, same_key)
            .await
            .unwrap(),
        Outcome::IdempotencyConflict
    );
    let (left, right) = tokio::join!(
        runtime.redeem_account_invite(&tenant_a, a, &second, Uuid::new_v4()),
        runtime.redeem_account_invite(&tenant_b, b, &second, Uuid::new_v4())
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert!(outcomes.contains(&Outcome::InvalidInvite));
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Invited { .. }))
            .count(),
        1
    );

    let revoked = seed_invite(&admin, &format!("INVITE-{}", Uuid::new_v4()), 1).await;
    admin
        .execute(
            "UPDATE onboarding_invite_grants SET revoked_at = now() WHERE invite_subject_hash = $1",
            &[&revoked],
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, a, &revoked, Uuid::new_v4())
            .await
            .unwrap(),
        Outcome::InvalidInvite
    );
    let expired = seed_invite(&admin, &format!("INVITE-{}", Uuid::new_v4()), 1).await;
    admin.execute("UPDATE onboarding_invite_grants SET expires_at = now() - interval '1 second' WHERE invite_subject_hash = $1", &[&expired]).await.unwrap();
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, a, &expired, Uuid::new_v4())
            .await
            .unwrap(),
        Outcome::InvalidInvite
    );

    // Hold the durable invite row across its expiration. Wait until PostgreSQL
    // reports that the redeeming connection is blocked by this transaction,
    // then cross the expiry and release it. This catches transaction-start
    // `now()` checks that still see the pre-lock timestamp after waking.
    let lock_wait = seed_invite(&admin, &format!("INVITE-{}", Uuid::new_v4()), 1).await;
    let trust_before: i64 = admin
        .query_one(
            "SELECT trust_version FROM trace_account_trust
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant_a, &a],
        )
        .await
        .unwrap()
        .get(0);
    admin
        .execute(
            "UPDATE onboarding_invite_grants
                SET expires_at = clock_timestamp() + interval '2 seconds'
              WHERE invite_subject_hash = $1",
            &[&lock_wait],
        )
        .await
        .unwrap();
    let mut blocker = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let lock_tx = blocker.transaction().await.unwrap();
    let blocker_pid: i32 = lock_tx
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    lock_tx
        .query_one(
            "SELECT 1 FROM onboarding_invite_grants WHERE invite_subject_hash = $1 FOR UPDATE",
            &[&lock_wait],
        )
        .await
        .unwrap();
    let runtime_for_wait = runtime.clone();
    let wait_tenant = tenant_a.clone();
    let wait_hash = lock_wait.clone();
    let wait_key = Uuid::new_v4();
    let wait_redeem = tokio::spawn(async move {
        runtime_for_wait
            .redeem_account_invite(&wait_tenant, a, &wait_hash, wait_key)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let blocked: bool = admin
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                   WHERE $1 = ANY(pg_blocking_pids(pid)))",
                    &[&blocker_pid],
                )
                .await
                .unwrap()
                .get(0);
            if blocked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("redemption must be waiting for the invite row lock");
    tokio::time::sleep(Duration::from_millis(2200)).await;
    lock_tx.commit().await.unwrap();
    assert_eq!(wait_redeem.await.unwrap().unwrap(), Outcome::InvalidInvite);
    let uses: i32 = admin
        .query_one(
            "SELECT consumed_uses FROM onboarding_invite_grants WHERE invite_subject_hash = $1",
            &[&lock_wait],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(uses, 0, "expired invite must not spend a use");
    let trust_after: i64 = admin
        .query_one(
            "SELECT trust_version FROM trace_account_trust
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant_a, &a],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        trust_after, trust_before,
        "expired invite must not raise trust"
    );
    for table in ["trace_account_invite_grants", "trace_account_trust_events"] {
        let sql = format!("SELECT count(*) FROM {table} WHERE invite_subject_hash = $1");
        let count: i64 = admin.query_one(&sql, &[&lock_wait]).await.unwrap().get(0);
        assert_eq!(count, 0, "expired invite must not write {table}");
    }
    admin
        .execute(
            "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant_a, &a],
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .redeem_account_invite(&tenant_a, a, &first, same_key)
            .await
            .unwrap(),
        Outcome::AccountIneligible
    );

    let client = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let acl = client.query_one("SELECT has_table_privilege(current_user, 'onboarding_invite_grants', 'INSERT'), (SELECT rolbypassrls FROM pg_roles WHERE rolname = current_user)", &[]).await.unwrap();
    assert!(!acl.get::<_, bool>(0), "runtime must not issue invites");
    assert!(!acl.get::<_, bool>(1), "runtime must obey RLS");
    for table in [
        "trace_account_trust",
        "trace_account_invite_grants",
        "trace_account_trust_events",
    ] {
        let sql = format!("SELECT count(*) FROM {table}");
        let count: i64 = client.query_one(&sql, &[]).await.unwrap().get(0);
        assert_eq!(count, 0, "{table} must be hidden without tenant context");
        let row = admin.query_one("SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE oid = $1::text::regclass", &[&table]).await.unwrap();
        assert!(row.get::<_, bool>(0) && row.get::<_, bool>(1));
    }
    let uses: i32 = admin
        .query_one(
            "SELECT consumed_uses FROM onboarding_invite_grants WHERE invite_subject_hash = $1",
            &[&first],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(uses, 1);
    let mut scoped = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = scoped.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_a],
    )
    .await
    .unwrap();
    let visible: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_account_trust WHERE tenant_id = $1",
            &[&tenant_b],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(visible, 0, "another tenant's trust stays hidden under RLS");
}
