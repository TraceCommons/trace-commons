// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! V127's settlement rail outbox under a least-privilege runtime role, driven
//! end to end against the fake NEAR AI admin API. Self-skips without
//! `TRACE_COMMONS_SETTLEMENT_RAIL_PG_TEST_DATABASE_URL`.

#[path = "support/fake_near_ai_admin.rs"]
mod fake;

use fake::{FakeState, Permission, Shared};
use secrecy::SecretString;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::near_ai_admin::{HttpNearAiAdminApi, NearAiOrgId};
use trace_commons_server::near_ai_inference_credit::NearAiInferenceCreditRail;
use trace_commons_server::settlement_rail::ConversionRate;
use trace_commons_server::settlement_rail_store::{
    DispatchSummary, EnqueueOutcome, ReconcileSummary, StoreError, bind_near_ai_recipient, cancel,
    dispatch_tick, enqueue_batch, lease_next, reconcile_tick, unfreeze,
};
use uuid::Uuid;

const UA: &str = "trace-commons-settlement/test";
const RUNTIME_ROLE: &str = "tc_settlement_rail_runtime";

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

struct Pg {
    admin: deadpool_postgres::Pool,
    runtime: deadpool_postgres::Pool,
}

async fn pg() -> Option<Pg> {
    let Ok(url) = std::env::var("TRACE_COMMONS_SETTLEMENT_RAIL_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TRACE_COMMONS_SETTLEMENT_RAIL_PG_TEST_DATABASE_URL not set");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test"));
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db.raw_pool_for_tests_and_diagnostics();
    // The runtime group's grants on tables older than V62 are the pilot's
    // hand-made ones; reproduce the three this path reads.
    admin
        .get()
        .await
        .unwrap()
        .batch_execute(&format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_ROLE}') THEN
                 CREATE ROLE {RUNTIME_ROLE} LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT trace_ingest_runtime TO {RUNTIME_ROLE};
             GRANT USAGE ON SCHEMA public TO trace_ingest_runtime;
             GRANT SELECT ON trace_credit_settlement_batches TO trace_ingest_runtime;
             GRANT SELECT, INSERT ON trace_near_credit_outbox TO trace_ingest_runtime;"
        ))
        .await
        .unwrap();
    let mut runtime_url = parsed.clone();
    runtime_url.set_username(RUNTIME_ROLE).unwrap();
    let runtime_db = PgBackend::new(&config(runtime_url.into())).await.unwrap();
    Some(Pg {
        admin,
        runtime: runtime_db.raw_pool_for_tests_and_diagnostics(),
    })
}

async fn seed_batch(pg: &Pg, tenant: &str, micros: i64) -> Uuid {
    let client = pg.admin.get().await.unwrap();
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    let batch = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_credit_settlement_batches
                (tenant_id, settlement_batch_id, policy_version, status, reason_hash,
                 source_list_hash, settled_credit_points, settled_credit_micros,
                 actor_principal_ref)
             VALUES ($1, $2, 'v1', 'finalized', 'sha256:00', $3, '0', $4, 'test')",
            &[&tenant, &batch, &format!("sha256:{batch}"), &micros],
        )
        .await
        .unwrap();
    batch
}

fn tenant() -> String {
    format!("nearai-{}", Uuid::new_v4().simple().to_string().repeat(2))
}

struct Near {
    state: Shared,
    rail: NearAiInferenceCreditRail,
    org: Uuid,
}

async fn near() -> Near {
    let state: Shared = Arc::new(Mutex::new(FakeState::default()));
    let org = Uuid::new_v4();
    {
        let mut s = state.lock().unwrap();
        s.orgs.insert(org, true);
        s.keys
            .insert("adm_write".into(), (Permission::ReadWrite, Some(UA.into())));
    }
    let base = fake::start(state.clone()).await;
    let api = HttpNearAiAdminApi::new(
        &base,
        SecretString::from("adm_write".to_string()),
        UA,
        Duration::from_millis(500),
    )
    .unwrap();
    Near {
        state,
        rail: NearAiInferenceCreditRail::new(Arc::new(api), "grant", 1_000_000_000_000, false),
        org,
    }
}

async fn make_due(pg: &Pg, tenant: &str) {
    pg.admin
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_settlement_rail_outbox SET next_attempt_at = NOW() - INTERVAL '1 second'
              WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap();
}

async fn states(pg: &Pg, tenant: &str) -> Vec<String> {
    pg.admin
        .get()
        .await
        .unwrap()
        .query(
            "SELECT state FROM trace_settlement_rail_outbox WHERE tenant_id = $1 ORDER BY created_at",
            &[&tenant],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
}

#[tokio::test]
async fn settlement_rail_end_to_end_under_the_runtime_role() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let t = tenant();
    let rate = ConversionRate::parse("1000/3").unwrap();
    let org = NearAiOrgId::from_uuid(near.org);

    // No recipient, no queue.
    let b1 = seed_batch(&pg, &t, 1_000_000).await;
    assert_eq!(
        enqueue_batch(&pg.runtime, &t, b1, &near.rail, &rate)
            .await
            .unwrap(),
        EnqueueOutcome::NoRecipient
    );
    assert_eq!(
        bind_near_ai_recipient(&pg.runtime, &t, org, "member").await,
        Err(StoreError::Refused("near_ai_org_role_insufficient"))
    );
    bind_near_ai_recipient(&pg.runtime, &t, org, "owner")
        .await
        .unwrap();

    // 1_000_000 micros * 1000 / 3 = 333_333_333 rem 1.
    assert!(matches!(
        enqueue_batch(&pg.runtime, &t, b1, &near.rail, &rate)
            .await
            .unwrap(),
        EnqueueOutcome::Queued {
            units: 333_333_333,
            ..
        }
    ));
    assert_eq!(
        enqueue_batch(&pg.runtime, &t, b1, &near.rail, &rate)
            .await
            .unwrap(),
        EnqueueOutcome::AlreadyQueued
    );
    let units: String = pg
        .admin
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT atomic_units::text FROM trace_settlement_rail_outbox WHERE tenant_id = $1",
            &[&t],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(units, "333333333");
    // The next batch picks up the carried remainder: (2 * 1000 + 1) / 3 = 667.
    let b2 = seed_batch(&pg, &t, 2).await;
    assert!(matches!(
        enqueue_batch(&pg.runtime, &t, b2, &near.rail, &rate)
            .await
            .unwrap(),
        EnqueueOutcome::Queued { units: 667, .. }
    ));
    // A batch that converts to zero is carried, not sent.
    let b0 = seed_batch(&pg, &t, 0).await;
    assert!(matches!(
        enqueue_batch(&pg.runtime, &t, b0, &near.rail, &rate)
            .await
            .unwrap(),
        EnqueueOutcome::Carried { .. }
    ));

    // Only one lease in flight per tenant and rail.
    let leased = lease_next(
        &pg.runtime,
        &t,
        "near_ai_org_limits",
        Duration::from_secs(60),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        lease_next(
            &pg.runtime,
            &t,
            "near_ai_org_limits",
            Duration::from_secs(60)
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(leased.delivery.expected_prior, None);
    pg.admin
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_settlement_rail_outbox SET state = 'pending', lease_token = NULL,
                    lease_expires_at = NULL WHERE tenant_id = $1 AND state = 'leased'",
            &[&t],
        )
        .await
        .unwrap();

    // Dispatch both; the second builds on the first.
    let summary = dispatch_tick(&pg.runtime, &t, &near.rail, 10, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(
        summary,
        DispatchSummary {
            delivered: 2,
            ..Default::default()
        }
    );
    assert_eq!(
        near.state
            .lock()
            .unwrap()
            .active(near.org, "grant")
            .unwrap()
            .amount,
        333_333_333 + 667
    );
    assert_eq!(
        reconcile_tick(&pg.runtime, &t, &near.rail, 10)
            .await
            .unwrap(),
        ReconcileSummary {
            reconciled: 2,
            ..Default::default()
        }
    );
    assert_eq!(
        states(&pg, &t).await,
        ["reconciled", "reconciled", "carried"]
    );

    // A write that lands behind a timeout is retried without a second PATCH.
    let b3 = seed_batch(&pg, &t, 3).await;
    enqueue_batch(&pg.runtime, &t, b3, &near.rail, &rate)
        .await
        .unwrap();
    near.state.lock().unwrap().stall_after_commit = Some(Duration::from_secs(2));
    let s = dispatch_tick(&pg.runtime, &t, &near.rail, 10, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(s.retry, 1);
    let calls = near.state.lock().unwrap().patch_calls;
    make_due(&pg, &t).await;
    let s = dispatch_tick(&pg.runtime, &t, &near.rail, 10, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(s.delivered, 1);
    assert_eq!(near.state.lock().unwrap().patch_calls, calls);
    assert_eq!(
        near.state
            .lock()
            .unwrap()
            .active(near.org, "grant")
            .unwrap()
            .amount,
        333_333_333 + 667 + 1000
    );

    // Someone else writes the bucket: the reconciler freezes the recipient,
    // and nothing further is leased for it.
    near.state
        .lock()
        .unwrap()
        .write(near.org, "grant", Some("nearai"), 5, Some("ops"), None);
    let r = reconcile_tick(&pg.runtime, &t, &near.rail, 10)
        .await
        .unwrap();
    assert_eq!(r.overwritten, 1);
    let b4 = seed_batch(&pg, &t, 30).await;
    let EnqueueOutcome::Queued { outbox_id, .. } =
        enqueue_batch(&pg.runtime, &t, b4, &near.rail, &rate)
            .await
            .unwrap()
    else {
        panic!("expected queued")
    };
    assert!(
        lease_next(
            &pg.runtime,
            &t,
            "near_ai_org_limits",
            Duration::from_secs(60)
        )
        .await
        .unwrap()
        .is_none()
    );
    // Clearing the recipient freeze without fixing the bucket still cannot
    // build on the foreign value: the rail freezes the row instead.
    pg.admin
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_near_ai_credit_recipients SET frozen_label = NULL, frozen_at = NULL
              WHERE tenant_id = $1",
            &[&t],
        )
        .await
        .unwrap();
    let s = dispatch_tick(&pg.runtime, &t, &near.rail, 10, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(s.frozen, 1);
    assert_eq!(
        near.state
            .lock()
            .unwrap()
            .active(near.org, "grant")
            .unwrap()
            .amount,
        5
    );
    // A frozen row without an effect can be cancelled; then the batch is
    // free for another path, and the row's identity stays immutable.
    assert!(cancel(&pg.runtime, &t, outbox_id).await.unwrap());
    assert!(!unfreeze(&pg.runtime, &t, outbox_id).await.unwrap());
}

#[tokio::test]
async fn settlement_rail_tables_are_tenant_isolated_and_one_org_has_one_tenant() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let a = tenant();
    let b = tenant();
    let rate = ConversionRate::parse("1/1").unwrap();
    let org = NearAiOrgId::from_uuid(near.org);
    let batch_a = seed_batch(&pg, &a, 10).await;
    seed_batch(&pg, &b, 10).await;
    bind_near_ai_recipient(&pg.runtime, &a, org, "admin")
        .await
        .unwrap();
    assert_eq!(
        bind_near_ai_recipient(&pg.runtime, &b, org, "owner").await,
        Err(StoreError::Refused("near_ai_org_bound_elsewhere"))
    );
    enqueue_batch(&pg.runtime, &a, batch_a, &near.rail, &rate)
        .await
        .unwrap();

    // Under tenant b's context, none of tenant a's rows is visible.
    let mut client = pg.runtime.get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&b],
    )
    .await
    .unwrap();
    for table in [
        "trace_settlement_rail_outbox",
        "trace_near_ai_credit_recipients",
        "trace_settlement_rail_carry",
    ] {
        let n: i64 = tx
            .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 0, "{table} leaks across tenants");
    }
    // The runtime cannot delete, and cannot rewrite an amount.
    assert!(
        tx.execute("DELETE FROM trace_settlement_rail_outbox", &[])
            .await
            .is_err()
    );
    drop(tx);
    let mut client = pg.runtime.get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&a],
    )
    .await
    .unwrap();
    assert!(
        tx.execute(
            "UPDATE trace_settlement_rail_outbox SET atomic_units = atomic_units + 1",
            &[]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn a_batch_pays_out_on_exactly_one_path() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let t = tenant();
    let rate = ConversionRate::parse("1/1").unwrap();
    bind_near_ai_recipient(&pg.runtime, &t, NearAiOrgId::from_uuid(near.org), "owner")
        .await
        .unwrap();
    let insert_near = |batch: Uuid| {
        let t = t.clone();
        let pool = pg.admin.clone();
        async move {
            pool.get()
                .await
                .unwrap()
                .execute(
                    "INSERT INTO trace_near_credit_outbox
                        (tenant_id, near_outbox_id, settlement_batch_id, credit_account_hash,
                         near_call_json, status)
                     VALUES ($1, $2, $3, 'sha256:00', '{}'::jsonb, 'pending')",
                    &[&t, &Uuid::new_v4(), &batch],
                )
                .await
        }
    };
    // Rail first: the NEAR receipt path is refused.
    let b1 = seed_batch(&pg, &t, 5).await;
    enqueue_batch(&pg.runtime, &t, b1, &near.rail, &rate)
        .await
        .unwrap();
    assert!(insert_near(b1).await.is_err());
    // NEAR receipt first: the rail is refused.
    let b2 = seed_batch(&pg, &t, 5).await;
    insert_near(b2).await.unwrap();
    assert!(
        enqueue_batch(&pg.runtime, &t, b2, &near.rail, &rate)
            .await
            .is_err()
    );
}
