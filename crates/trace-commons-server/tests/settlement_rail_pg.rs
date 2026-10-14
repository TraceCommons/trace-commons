// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! V129's cumulative settlement ledger under a least-privilege runtime role,
//! driven end to end against the fake of NEAR AI's proposed monotonic
//! `trace-commons/credit` route. Self-skips without
//! `TRACE_COMMONS_SETTLEMENT_RAIL_PG_TEST_DATABASE_URL`.

#[path = "support/fake_near_ai_admin.rs"]
mod fake;

use chrono::Utc;
use fake::{FakeState, Permission, Shared};
use secrecy::SecretString;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::near_ai_admin::{HttpNearAiAdminApi, NearAiOrgId};
use trace_commons_server::near_ai_credit_statement::{StatementSigner, verify};
use trace_commons_server::near_ai_inference_credit::{
    PullTraceCommonsCreditRail, PushTraceCommonsCreditRail, RAIL_ID,
};
use trace_commons_server::settlement_rail::ConversionRate;
use trace_commons_server::settlement_rail_store::{
    AccrueOutcome, PublishSummary, ReconcileSummary, StoreError, accrue_batch,
    bind_near_ai_recipient, publish_tick, reconcile_tick, signed_statement, unfreeze,
};
use uuid::Uuid;

const UA: &str = "trace-commons-settlement/test";
const RUNTIME_ROLE: &str = "tc_settlement_rail_runtime";
const LEASE: Duration = Duration::from_secs(60);

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

async fn seed_tenant(pg: &Pg) -> String {
    let tenant = format!("nearai-{}", Uuid::new_v4().simple().to_string().repeat(2));
    pg.admin
        .get()
        .await
        .unwrap()
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    tenant
}

async fn seed_batch(pg: &Pg, tenant: &str, micros: i64) -> Uuid {
    let batch = Uuid::new_v4();
    pg.admin
        .get()
        .await
        .unwrap()
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

struct Near {
    state: Shared,
    push: PushTraceCommonsCreditRail,
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
        Duration::from_secs(5),
    )
    .unwrap();
    Near {
        state,
        push: PushTraceCommonsCreditRail::new(Arc::new(api), 1_000_000_000_000, false),
        org,
    }
}

impl Near {
    fn current(&self) -> i64 {
        self.state.lock().unwrap().current(self.org)
    }
    fn raises(&self) -> usize {
        self.state.lock().unwrap().raises.len()
    }
}

async fn ledger(pg: &Pg, tenant: &str) -> (String, String, String, Option<String>) {
    let row = pg
        .admin
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT cumulative_units::text, confirmed_units::text, publish_state, last_error_label
               FROM trace_settlement_rail_ledgers WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2), row.get(3))
}

async fn recipient_of(pg: &Pg, tenant: &str) -> Uuid {
    pg.admin
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT recipient_id FROM trace_near_ai_credit_recipients
              WHERE tenant_id = $1 AND revoked_at IS NULL",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0)
}

async fn make_due(pg: &Pg, tenant: &str) {
    pg.admin
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_settlement_rail_ledgers
                SET publish_state = 'due', next_attempt_at = NOW() - INTERVAL '1 second'
              WHERE tenant_id = $1 AND publish_state IN ('due', 'idle')",
            &[&tenant],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn settlement_rail_end_to_end_under_the_runtime_role() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let t = seed_tenant(&pg).await;
    let rate = ConversionRate::parse("1000/3").unwrap();
    let org = NearAiOrgId::from_uuid(near.org);
    let rail = &near.push;

    let b1 = seed_batch(&pg, &t, 1_000_000).await;
    assert_eq!(
        accrue_batch(&pg.runtime, &t, b1, rail, &rate)
            .await
            .unwrap(),
        AccrueOutcome::NoRecipient
    );
    assert_eq!(
        bind_near_ai_recipient(&pg.runtime, &t, org, "member").await,
        Err(StoreError::Refused("near_ai_org_role_insufficient"))
    );
    bind_near_ai_recipient(&pg.runtime, &t, org, "owner")
        .await
        .unwrap();

    // 1_000_000 micros * 1000 / 3 = 333_333_333 remainder 1.
    assert_eq!(
        accrue_batch(&pg.runtime, &t, b1, rail, &rate)
            .await
            .unwrap(),
        AccrueOutcome::Accrued {
            units: 333_333_333,
            cumulative_units: 333_333_333
        }
    );
    assert_eq!(
        accrue_batch(&pg.runtime, &t, b1, rail, &rate)
            .await
            .unwrap(),
        AccrueOutcome::AlreadyAccrued
    );
    // The carried remainder joins the next batch: (2 * 1000 + 1) / 3 = 667.
    let b2 = seed_batch(&pg, &t, 2).await;
    assert_eq!(
        accrue_batch(&pg.runtime, &t, b2, rail, &rate)
            .await
            .unwrap(),
        AccrueOutcome::Accrued {
            units: 667,
            cumulative_units: 333_334_000
        }
    );

    // Two workers publishing at once still credit the total once.
    let (left, right) = tokio::join!(
        publish_tick(&pg.runtime, &t, rail, 10, LEASE),
        publish_tick(&pg.runtime, &t, rail, 10, LEASE)
    );
    assert_eq!(left.unwrap().confirmed + right.unwrap().confirmed, 1);
    assert_eq!(near.current(), 333_334_000);
    assert_eq!(near.raises(), 1);
    assert_eq!(
        ledger(&pg, &t).await,
        ("333334000".into(), "333334000".into(), "idle".into(), None)
    );
    assert_eq!(
        reconcile_tick(&pg.runtime, &t, rail, 10).await.unwrap(),
        ReconcileSummary {
            verified: 1,
            ..Default::default()
        }
    );

    // Republishing an unchanged total is a no-op on NEAR AI's side.
    make_due(&pg, &t).await;
    let s = publish_tick(&pg.runtime, &t, rail, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(s.confirmed, 1);
    assert_eq!(near.raises(), 1);

    // A publish that committed behind a failed response is retried, and the
    // retry writes nothing new.
    let b3 = seed_batch(&pg, &t, 3).await;
    accrue_batch(&pg.runtime, &t, b3, rail, &rate)
        .await
        .unwrap();
    near.state.lock().unwrap().bad_gateway_after_commit = true;
    let s = publish_tick(&pg.runtime, &t, rail, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(s.retry, 1);
    assert_eq!(near.raises(), 2);
    make_due(&pg, &t).await;
    let s = publish_tick(&pg.runtime, &t, rail, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(s.confirmed, 1);
    assert_eq!(near.raises(), 2);
    assert_eq!(near.current(), 333_335_000);

    // NEAR AI holding more than we ever accrued freezes the ledger.
    near.state
        .lock()
        .unwrap()
        .credit
        .insert(near.org, 999_999_999_999);
    let r = reconcile_tick(&pg.runtime, &t, rail, 10).await.unwrap();
    assert_eq!(r.frozen, 1);
    let (_, _, state, label) = ledger(&pg, &t).await;
    assert_eq!(
        (state.as_str(), label.as_deref()),
        ("frozen", Some("settlement_rail_external_ahead"))
    );
    // Accrual continues while frozen; nothing is published.
    let b4 = seed_batch(&pg, &t, 30).await;
    accrue_batch(&pg.runtime, &t, b4, rail, &rate)
        .await
        .unwrap();
    assert_eq!(
        publish_tick(&pg.runtime, &t, rail, 10, LEASE)
            .await
            .unwrap(),
        PublishSummary::default()
    );
    // Unfreezing without fixing the cause freezes again on the next publish.
    let recipient: Uuid = pg
        .admin
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT recipient_id FROM trace_settlement_rail_ledgers WHERE tenant_id = $1",
            &[&t],
        )
        .await
        .unwrap()
        .get(0);
    assert!(unfreeze(&pg.runtime, &t, RAIL_ID, recipient).await.unwrap());
    let s = publish_tick(&pg.runtime, &t, rail, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(s.frozen, 1);

    // A server answering below the total it was given is refused, not
    // retried forever.
    near.state.lock().unwrap().credit.insert(near.org, 0);
    near.state.lock().unwrap().lie_next_put = Some(5);
    assert!(unfreeze(&pg.runtime, &t, RAIL_ID, recipient).await.unwrap());
    let s = publish_tick(&pg.runtime, &t, rail, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(s.frozen, 1);
    let (_, _, _, label) = ledger(&pg, &t).await;
    assert_eq!(
        label.as_deref(),
        Some("settlement_rail_external_refused_total")
    );
}

#[tokio::test]
async fn settlement_rail_tables_are_tenant_isolated_and_totals_only_grow() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let a = seed_tenant(&pg).await;
    let b = seed_tenant(&pg).await;
    let rate = ConversionRate::parse("1/1").unwrap();
    let org = NearAiOrgId::from_uuid(near.org);
    let batch_a = seed_batch(&pg, &a, 10).await;
    bind_near_ai_recipient(&pg.runtime, &a, org, "admin")
        .await
        .unwrap();
    assert_eq!(
        bind_near_ai_recipient(&pg.runtime, &b, org, "owner").await,
        Err(StoreError::Refused("near_ai_org_bound_elsewhere"))
    );
    accrue_batch(&pg.runtime, &a, batch_a, &near.push, &rate)
        .await
        .unwrap();

    // NEAR AI's total for an organization never decreases, so it must always
    // equal one ledger. Moving to another organization and back reuses the
    // same recipient and ledger instead of restarting at zero ...
    let first = recipient_of(&pg, &a).await;
    let other = NearAiOrgId::from_uuid(Uuid::new_v4());
    bind_near_ai_recipient(&pg.runtime, &a, other, "owner")
        .await
        .unwrap();
    let back = bind_near_ai_recipient(&pg.runtime, &a, org, "owner")
        .await
        .unwrap();
    assert_eq!(back, first);
    assert_eq!(ledger(&pg, &a).await.0, "10");
    // ... and an organization that has ever been bound can never move to
    // another tenant, even after its first tenant revoked it.
    bind_near_ai_recipient(&pg.runtime, &a, other, "owner")
        .await
        .unwrap();
    assert_eq!(
        bind_near_ai_recipient(&pg.runtime, &b, org, "owner").await,
        Err(StoreError::Refused("near_ai_org_bound_elsewhere"))
    );
    bind_near_ai_recipient(&pg.runtime, &a, org, "owner")
        .await
        .unwrap();

    let mut client = pg.runtime.get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&b],
    )
    .await
    .unwrap();
    for table in [
        "trace_near_ai_credit_recipients",
        "trace_settlement_rail_credits",
        "trace_settlement_rail_ledgers",
    ] {
        let n: i64 = tx
            .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 0, "{table} leaks across tenants");
    }
    drop(tx);

    // Under tenant a's own context: no delete, no rewriting a credit, and the
    // ledger total cannot go down.
    for statement in [
        "DELETE FROM trace_settlement_rail_ledgers",
        "UPDATE trace_settlement_rail_credits SET atomic_units = 0",
        "UPDATE trace_settlement_rail_ledgers SET cumulative_units = 0",
        "UPDATE trace_settlement_rail_ledgers SET confirmed_units = cumulative_units + 1",
    ] {
        let mut client = pg.runtime.get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&a],
        )
        .await
        .unwrap();
        assert!(tx.execute(statement, &[]).await.is_err(), "{statement}");
    }
}

#[tokio::test]
async fn a_batch_pays_out_on_exactly_one_path() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let t = seed_tenant(&pg).await;
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
    accrue_batch(&pg.runtime, &t, b1, &near.push, &rate)
        .await
        .unwrap();
    assert!(insert_near(b1).await.is_err());
    // NEAR receipt first: the rail is refused.
    let b2 = seed_batch(&pg, &t, 5).await;
    insert_near(b2).await.unwrap();
    assert!(
        accrue_batch(&pg.runtime, &t, b2, &near.push, &rate)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pull_mode_serves_a_signed_cumulative_statement() {
    let Some(pg) = pg().await else { return };
    let near = near().await;
    let t = seed_tenant(&pg).await;
    let rate = ConversionRate::parse("1/1").unwrap();
    let pull = PullTraceCommonsCreditRail::new(None, 1_000_000);
    let signer = StatementSigner::from_base64_pkcs8(&{
        use base64::Engine;
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(doc.as_ref())
    })
    .unwrap();
    assert!(
        signed_statement(&pg.runtime, &t, RAIL_ID, &signer, Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    bind_near_ai_recipient(&pg.runtime, &t, NearAiOrgId::from_uuid(near.org), "owner")
        .await
        .unwrap();
    for micros in [40, 2] {
        let b = seed_batch(&pg, &t, micros).await;
        accrue_batch(&pg.runtime, &t, b, &pull, &rate)
            .await
            .unwrap();
    }
    let s = publish_tick(&pg.runtime, &t, &pull, 10, LEASE)
        .await
        .unwrap();
    assert_eq!(s.deferred, 1);
    let (_, confirmed, state, _) = ledger(&pg, &t).await;
    assert_eq!((confirmed.as_str(), state.as_str()), ("0", "published"));

    let signed = signed_statement(&pg.runtime, &t, RAIL_ID, &signer, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(signed.statement.cumulative_nano_usd, "42");
    assert_eq!(signed.statement.org_id, near.org.to_string());
    assert!(!signed.statement.statement_ref.contains(&t));
    assert_eq!(verify(&signed, &signer.public_key()), Ok(()));
    // No admin call was made in pull mode.
    assert_eq!(near.state.lock().unwrap().put_calls, 0);
}
