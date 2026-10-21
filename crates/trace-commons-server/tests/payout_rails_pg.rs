// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout rail tables (V131) against a real PostgreSQL.
//!
//! Every store call here runs as a real `LOGIN` role whose only privilege
//! sources are `trace_ingest_runtime` (inherited) and, with `INHERIT FALSE`,
//! the two narrow payout roles it may `SET ROLE` to. Never the migration
//! owner and never a superuser: either one ignores a missing grant, and a
//! test of a refusal would then pass without testing anything.
//!
//! Needs PostgreSQL 16 or later (`GRANT ... WITH INHERIT FALSE`), a database
//! whose name starts with `admission_test_`, on the literal host
//! `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Without the
//! variable every test prints a skip line and returns.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentId};
use trace_commons_gate_api::{RailId, RailRate, RecipientKind};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::{
    Database, PayoutAccrualKind, PayoutAccrualWrite, PayoutAuditAction, PayoutLineResolution,
    PayoutLineResolve, PayoutLineState, PayoutLineWrite, PayoutLinkAction, PayoutLinkInsert,
    PayoutLinkRequestInsert, PayoutLinkRequestWrite, PayoutLinkState, PayoutLinkWrite,
    PayoutRateEpochRecord, PayoutStore, PendingPayoutLineInsert, SealedRecipient,
};
use uuid::Uuid;

/// The runtime login these tests connect as. Roles are per server; the
/// login is re-provisioned to exactly this shape on every run.
const RUNTIME_LOGIN: &str = "tc_payout_runtime_test";
const ACTOR: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

struct Fixture {
    owner: PgBackend,
    runtime: PgBackend,
}

fn config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url.to_owned()),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).expect("database url");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_"));
    let owner = PgBackend::new(&config(&url)).await.expect("owner backend");
    owner.run_migrations().await.expect("apply migrations");

    let client = owner
        .trace_pool_for_test()
        .get()
        .await
        .expect("owner client");
    let sixteen: bool = client
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .expect("server version")
        .get(0);
    assert!(sixteen, "payout_rails_pg needs PostgreSQL 16 or later");
    client
        .batch_execute(&with_role_ddl_lock(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_LOGIN}') THEN
                     CREATE ROLE {RUNTIME_LOGIN} LOGIN;
                 END IF;
             END $$;
             ALTER ROLE {RUNTIME_LOGIN} LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
             GRANT trace_ingest_runtime TO {RUNTIME_LOGIN} WITH INHERIT TRUE;
             GRANT trace_payout_enumerator TO {RUNTIME_LOGIN} WITH INHERIT FALSE, SET TRUE;
             GRANT trace_payout_config_writer TO {RUNTIME_LOGIN} WITH INHERIT FALSE, SET TRUE;"
        )))
        .await
        .expect("provision the runtime login");
    drop(client);

    let mut runtime_url = parsed.clone();
    runtime_url
        .set_username(RUNTIME_LOGIN)
        .expect("runtime username");
    runtime_url.set_password(None).expect("no password");
    let runtime = PgBackend::new(&config(runtime_url.as_str()))
        .await
        .expect("runtime backend");
    let check = runtime
        .trace_pool_for_test()
        .get()
        .await
        .expect("runtime client");
    let row = check
        .query_one(
            "SELECT current_user::text, rolsuper, rolbypassrls
               FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .expect("runtime identity");
    assert_eq!(row.get::<_, String>(0), RUNTIME_LOGIN);
    assert!(
        !row.get::<_, bool>(1),
        "the runtime login must not be superuser"
    );
    assert!(
        !row.get::<_, bool>(2),
        "the runtime login must not bypass RLS"
    );
    drop(check);
    Some(Fixture { owner, runtime })
}

/// A tenant with one account, created as the owner.
async fn tenant_with_accounts(owner: &PgBackend, accounts: usize) -> (String, Vec<Uuid>) {
    let tenant = format!("payout-{}", Uuid::new_v4());
    let ids: Vec<Uuid> = (0..accounts).map(|_| Uuid::new_v4()).collect();
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
        &[&tenant],
    )
    .await
    .unwrap();
    for id in &ids {
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, id],
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    (tenant, ids)
}

/// A finalized settlement batch, created as the owner.
async fn finalized_batch(owner: &PgBackend, tenant: &str) -> Uuid {
    let batch = Uuid::new_v4();
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO trace_credit_settlement_batches (
             tenant_id, settlement_batch_id, policy_version, status, reason_hash,
             source_list_hash, settled_credit_points, settled_credit_micros,
             actor_principal_ref
         ) VALUES ($1, $2, 'test', 'finalized', $3, $4, '1', 1000000, 'operator')",
        &[
            &tenant,
            &batch,
            &sha(&format!("reason:{batch}")),
            &sha(&format!("list:{batch}")),
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    batch
}

fn sha(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn rail(value: &str) -> RailId {
    RailId::new(value).unwrap()
}

fn recipient(seed: &str) -> SealedRecipient {
    SealedRecipient {
        sealed: serde_json::json!({"v": 1, "ciphertext": sha(&format!("ct:{seed}"))}),
        recipient_index: sha(&format!("index:{seed}")),
        recipient_key_ref: "key-test-1".into(),
        index_pepper_ref: "pepper-test-1".into(),
        subject_index: sha(&format!("subject:{seed}")),
    }
}

fn link(account: Uuid, rail_id: &str, state: PayoutLinkState, seed: &str) -> PayoutLinkWrite {
    let link_id = Uuid::new_v4();
    PayoutLinkWrite {
        account_id: account,
        rail_id: rail(rail_id),
        link_id,
        recipient_kind: RecipientKind::new("test_recipient").unwrap(),
        recipient: recipient(seed),
        program: format!("tc1-{}", &sha(&link_id.to_string())[7..39]),
        state,
        actor_ref_hash: ACTOR.into(),
    }
}

fn line(link: &PayoutLinkWrite, target: u128, delta: u128) -> PayoutLineWrite {
    PayoutLineWrite {
        line_id: Uuid::new_v4(),
        link_id: link.link_id,
        account_id: link.account_id,
        rail_id: link.rail_id.clone(),
        target_total: AtomicUnits::from_raw(target),
        delta: AtomicUnits::from_raw(delta),
        owed_through_accrual_count: 1,
    }
}

fn credit(
    rail_id: &str,
    batch: Uuid,
    account: Option<Uuid>,
    hash: &str,
    award: u128,
    units: u128,
) -> PayoutAccrualWrite {
    PayoutAccrualWrite {
        rail_id: rail(rail_id),
        kind: PayoutAccrualKind::Credit,
        settlement_batch_id: batch,
        credit_account_hash: hash.into(),
        reversal_event_id: None,
        account_id: account,
        award_units: AtomicUnits::from_raw(award),
        rail_units: AtomicUnits::from_raw(units),
        rate_version: "v1".into(),
        instrument_id: InstrumentId::new("test_units").unwrap(),
    }
}

fn is_permission_denied(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
}

/// A raw runtime-login transaction in `tenant`'s context.
async fn runtime_exec(
    fixture: &Fixture,
    tenant: Option<&str>,
    sql: &str,
) -> Result<u64, tokio_postgres::Error> {
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    if let Some(tenant) = tenant {
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
    }
    let result = tx.execute(sql, &[]).await;
    if result.is_ok() {
        tx.commit().await.unwrap();
    }
    result
}

const PAYOUT_TABLES: [&str; 8] = [
    "trace_payout_links",
    "trace_payout_link_requests",
    "trace_payout_accruals",
    "trace_payout_lines",
    "trace_payout_holds",
    "trace_payout_audit",
    "trace_payout_rate_epochs",
    "trace_payout_control",
];

#[tokio::test]
async fn every_payout_table_forces_rls() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    for table in PAYOUT_TABLES {
        let row = client
            .query_one(
                "SELECT c.relrowsecurity, c.relforcerowsecurity,
                        (SELECT count(*) FROM pg_policies p WHERE p.tablename = c.relname)
                   FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE n.nspname = 'public' AND c.relname = $1",
                &[&table],
            )
            .await
            .unwrap_or_else(|error| panic!("{table}: {error}"));
        assert!(row.get::<_, bool>(0), "{table} must enable RLS");
        assert!(row.get::<_, bool>(1), "{table} must force RLS");
        assert!(row.get::<_, i64>(2) > 0, "{table} must carry a policy");
    }
    // The six tenant tables are in the central registry, so readiness and
    // the restore drill check them; the two deployment-wide tables have no
    // tenant column and are not.
    let central = trace_commons_server::db::postgres::TRACE_COMMONS_RLS_TABLES;
    for table in &PAYOUT_TABLES[..6] {
        assert!(central.contains(table), "{table} missing from the registry");
    }
    for table in &PAYOUT_TABLES[6..] {
        assert!(!central.contains(table), "{table} has no tenant column");
    }
}

#[tokio::test]
async fn a_tenant_cannot_read_another_tenants_links_or_lines() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant_a, accounts_a) = tenant_with_accounts(&fixture.owner, 1).await;
    let (tenant_b, accounts_b) = tenant_with_accounts(&fixture.owner, 1).await;
    let near_ai = rail("near_ai");
    let link_a = link(accounts_a[0], "near_ai", PayoutLinkState::Active, "a");
    assert_eq!(
        fixture
            .runtime
            .insert_payout_link(&tenant_a, &link_a)
            .await
            .unwrap(),
        PayoutLinkInsert::Inserted
    );
    let line_a = line(&link_a, 10, 10);
    assert_eq!(
        fixture
            .runtime
            .insert_pending_payout_line(&tenant_a, &line_a)
            .await
            .unwrap(),
        PendingPayoutLineInsert::Inserted
    );

    // Through the store: tenant B sees nothing of tenant A's account.
    assert!(
        fixture
            .runtime
            .payout_live_link(&tenant_b, accounts_a[0], &near_ai)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .runtime
            .pending_payout_line_for_link(&tenant_b, link_a.link_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .runtime
            .payout_live_link(&tenant_b, accounts_b[0], &near_ai)
            .await
            .unwrap()
            .is_none()
    );
    // Tenant A sees its own.
    let live = fixture
        .runtime
        .payout_live_link(&tenant_a, accounts_a[0], &near_ai)
        .await
        .unwrap()
        .expect("tenant A's own link");
    assert_eq!(live.link_id, link_a.link_id);
    assert_eq!(live.state, PayoutLinkState::Active);
    assert_eq!(
        live.recipient.recipient_index,
        link_a.recipient.recipient_index
    );

    // Raw: every payout table is empty under tenant B, and under no tenant.
    let client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    for table in &PAYOUT_TABLES[..6] {
        let none: i64 = client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(none, 0, "{table} visible without tenant context");
    }
    drop(client);
    for table in [
        "trace_payout_links",
        "trace_payout_lines",
        "trace_payout_audit",
    ] {
        let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_b],
        )
        .await
        .unwrap();
        let seen: i64 = tx
            .query_one(
                &format!("SELECT count(*) FROM {table} WHERE tenant_id = $1"),
                &[&tenant_a],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(seen, 0, "{table}: tenant B read tenant A's rows");
    }
    // And a write naming tenant A from tenant B's context is refused.
    let updated = runtime_exec(
        &fixture,
        Some(&tenant_b),
        &format!(
            "UPDATE trace_payout_lines SET attempts = attempts + 1 WHERE tenant_id = '{tenant_a}'"
        ),
    )
    .await
    .unwrap();
    assert_eq!(updated, 0, "tenant B updated tenant A's line");
}

#[tokio::test]
async fn at_most_one_active_waiting_and_closing_link_per_account_and_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let store = &fixture.runtime;
    for state in [PayoutLinkState::Active, PayoutLinkState::Waiting] {
        let first = link(accounts[0], "near_ai", state, "first");
        assert_eq!(
            store.insert_payout_link(&tenant, &first).await.unwrap(),
            PayoutLinkInsert::Inserted
        );
        let second = link(accounts[0], "near_ai", state, "second");
        assert_eq!(
            store.insert_payout_link(&tenant, &second).await.unwrap(),
            PayoutLinkInsert::StateTaken,
            "a second {state:?} link on near_ai for one account"
        );
        // The same state on another rail, and on another account, is fine.
        let other_rail = link(accounts[0], "test_keyed", state, "other-rail");
        assert_eq!(
            store
                .insert_payout_link(&tenant, &other_rail)
                .await
                .unwrap(),
            PayoutLinkInsert::Inserted
        );
        let other_account = link(accounts[1], "near_ai", state, "other-account");
        assert_eq!(
            store
                .insert_payout_link(&tenant, &other_account)
                .await
                .unwrap(),
            PayoutLinkInsert::Inserted
        );
    }
    // Closing: the active link moves to closing; a second closing is refused.
    let near_ai = rail("near_ai");
    let active = store
        .payout_live_link(&tenant, accounts[0], &near_ai)
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Active,
                PayoutLinkState::Closing,
                None,
                ACTOR,
                None,
            )
            .await
            .unwrap()
    );
    // A new active is now allowed beside the closing one...
    let next = link(accounts[0], "near_ai", PayoutLinkState::Active, "next");
    assert_eq!(
        store.insert_payout_link(&tenant, &next).await.unwrap(),
        PayoutLinkInsert::Inserted
    );
    // ...but it cannot also become closing while the first still is.
    let refused = store
        .transition_payout_link(
            &tenant,
            next.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            None,
            ACTOR,
            None,
        )
        .await;
    assert!(
        refused.is_err(),
        "two closing links on one account and rail"
    );
    // Closing needs the final total; closed then frees the slot.
    assert!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Closing,
                PayoutLinkState::Closed,
                None,
                ACTOR,
                None,
            )
            .await
            .is_err(),
        "a link closed without final_paid"
    );
    assert!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Closing,
                PayoutLinkState::Closed,
                Some(AtomicUnits::from_raw(7)),
                ACTOR,
                None,
            )
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .payout_closed_links_final_paid(&tenant, accounts[0], &near_ai)
            .await
            .unwrap(),
        7
    );
    // A transition from the wrong state changes nothing.
    assert!(
        !store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Active,
                PayoutLinkState::Closing,
                None,
                ACTOR,
                None,
            )
            .await
            .unwrap()
    );
    // Every insert and transition wrote one audit row, as the runtime role.
    let audit = store.payout_audit_actions(&tenant, &near_ai).await.unwrap();
    assert!(audit.contains(&PayoutAuditAction::LinkClosing));
    assert!(audit.contains(&PayoutAuditAction::LinkClosed));
    assert_eq!(
        audit
            .iter()
            .filter(|action| **action == PayoutAuditAction::LinkCreated)
            .count(),
        // active + waiting on accounts[0] and accounts[1], plus `next`
        5
    );
}

#[tokio::test]
async fn one_pending_line_per_link() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let live = link(accounts[0], "near_ai", PayoutLinkState::Active, "p");
    store.insert_payout_link(&tenant, &live).await.unwrap();
    let first = line(&live, 100, 100);
    assert_eq!(
        store
            .insert_pending_payout_line(&tenant, &first)
            .await
            .unwrap(),
        PendingPayoutLineInsert::Inserted
    );
    let second = line(&live, 200, 200);
    assert_eq!(
        store
            .insert_pending_payout_line(&tenant, &second)
            .await
            .unwrap(),
        PendingPayoutLineInsert::LinkHasPendingLine
    );
    let pending = store
        .pending_payout_line_for_link(&tenant, live.link_id)
        .await
        .unwrap()
        .expect("the first line");
    assert_eq!(pending.line_id, first.line_id);
    assert_eq!(pending.state, PayoutLineState::Pending);
    assert_eq!(pending.target_total.get(), 100);

    // A retry keeps it pending and counts the attempt.
    assert_eq!(
        store
            .resolve_payout_line(
                &tenant,
                first.line_id,
                &PayoutLineResolution::Retry {
                    label: "payout_retry".into()
                }
            )
            .await
            .unwrap(),
        PayoutLineResolve::Resolved
    );
    let pending = store
        .pending_payout_line_for_link(&tenant, live.link_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.attempts, 1);
    assert_eq!(pending.last_error_label.as_deref(), Some("payout_retry"));

    // Once resolved, the link takes a new line.
    store
        .resolve_payout_line(&tenant, first.line_id, &PayoutLineResolution::NoOp)
        .await
        .unwrap();
    assert_eq!(
        store
            .resolve_payout_line(&tenant, first.line_id, &PayoutLineResolution::NoOp)
            .await
            .unwrap(),
        PayoutLineResolve::NotPending
    );
    assert_eq!(
        store
            .insert_pending_payout_line(&tenant, &second)
            .await
            .unwrap(),
        PendingPayoutLineInsert::Inserted
    );
}

#[tokio::test]
async fn one_receipt_answers_one_line_per_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let store = &fixture.runtime;
    // The receipt index spans tenants (one external record answers one
    // line on a rail, whoever's), so each run takes a receipt of its own.
    let receipt = sha(&format!("receipt:{tenant}"));
    let link_one = link(accounts[0], "near_ai", PayoutLinkState::Active, "r1");
    let link_two = link(accounts[1], "near_ai", PayoutLinkState::Active, "r2");
    let link_keyed = link(accounts[0], "test_keyed", PayoutLinkState::Active, "r3");
    for l in [&link_one, &link_two, &link_keyed] {
        store.insert_payout_link(&tenant, l).await.unwrap();
    }
    let line_one = line(&link_one, 5, 5);
    let line_two = line(&link_two, 5, 5);
    let line_keyed = line(&link_keyed, 5, 5);
    for l in [&line_one, &line_two, &line_keyed] {
        store.insert_pending_payout_line(&tenant, l).await.unwrap();
    }
    let confirmed = |receipt: &str| PayoutLineResolution::Confirmed {
        receipt_hash: receipt.to_owned(),
        delta: AtomicUnits::from_raw(5),
    };
    assert_eq!(
        store
            .resolve_payout_line(&tenant, line_one.line_id, &confirmed(&receipt))
            .await
            .unwrap(),
        PayoutLineResolve::Resolved
    );
    assert_eq!(
        store
            .resolve_payout_line(&tenant, line_two.line_id, &confirmed(&receipt))
            .await
            .unwrap(),
        PayoutLineResolve::ReceiptAlreadyClaimed,
        "one near_ai receipt answered two lines"
    );
    // The refused line is untouched.
    let still = store
        .pending_payout_line_for_link(&tenant, link_two.link_id)
        .await
        .unwrap()
        .expect("still pending");
    assert_eq!(still.receipt_hash, None);
    // The same receipt hash on another rail is another rail's record.
    assert_eq!(
        store
            .resolve_payout_line(&tenant, line_keyed.line_id, &confirmed(&receipt))
            .await
            .unwrap(),
        PayoutLineResolve::Resolved
    );
    assert!(
        store
            .payout_receipt_is_claimed(&tenant, &rail("near_ai"), &receipt)
            .await
            .unwrap()
    );
    assert!(
        !store
            .payout_receipt_is_claimed(
                &tenant,
                &rail("near_ai"),
                &sha(&format!("receipt:other:{tenant}"))
            )
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn accrual_inserts_are_idempotent_per_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let batch = finalized_batch(&fixture.owner, &tenant).await;
    let store = &fixture.runtime;
    let hash = sha(&format!("account:{}", accounts[0]));
    let near_ai = credit(
        "near_ai",
        batch,
        Some(accounts[0]),
        &hash,
        1_000_000,
        2_000_000_000,
    );
    assert_eq!(
        store
            .insert_payout_accruals(&tenant, std::slice::from_ref(&near_ai))
            .await
            .unwrap(),
        1
    );
    // The same batch line again, even at another price, writes nothing.
    let mut repriced = near_ai.clone();
    repriced.rail_units = AtomicUnits::from_raw(9);
    assert_eq!(
        store
            .insert_payout_accruals(&tenant, &[near_ai.clone(), repriced])
            .await
            .unwrap(),
        0
    );
    // The same line on another rail is that rail's own row.
    let keyed = credit(
        "test_keyed",
        batch,
        Some(accounts[0]),
        &hash,
        1_000_000,
        1_000_000_000_000_000_000,
    );
    assert_eq!(
        store
            .insert_payout_accruals(&tenant, std::slice::from_ref(&keyed))
            .await
            .unwrap(),
        1
    );
    let owed = store
        .payout_owed(&tenant, accounts[0], &rail("near_ai"))
        .await
        .unwrap();
    assert_eq!(owed.net(), Some(2_000_000_000));
    assert_eq!(owed.accrual_count, 1);

    // A reversal is keyed by its event, once per rail, and takes back units.
    let event = Uuid::new_v4();
    let reversal = PayoutAccrualWrite {
        kind: PayoutAccrualKind::Reversal,
        reversal_event_id: Some(event),
        award_units: AtomicUnits::from_raw(500_000),
        rail_units: AtomicUnits::from_raw(1_000_000_000),
        ..near_ai.clone()
    };
    assert_eq!(
        store
            .insert_payout_accruals(&tenant, &[reversal.clone(), reversal.clone()])
            .await
            .unwrap(),
        1
    );
    let owed = store
        .payout_owed(&tenant, accounts[0], &rail("near_ai"))
        .await
        .unwrap();
    assert_eq!(owed.credited, 2_000_000_000);
    assert_eq!(owed.reversed, 1_000_000_000);
    assert_eq!(owed.net(), Some(1_000_000_000));
    assert_eq!(owed.accrual_count, 2);
    let keyed_owed = store
        .payout_owed(&tenant, accounts[0], &rail("test_keyed"))
        .await
        .unwrap();
    assert_eq!(keyed_owed.net(), Some(1_000_000_000_000_000_000));

    // An accrual with no owner yet is owed to nobody until it is claimed,
    // and the claim sets the owner once.
    let principal_hash = sha("principal:bare");
    let batch_two = finalized_batch(&fixture.owner, &tenant).await;
    let unowned = credit("near_ai", batch_two, None, &principal_hash, 10, 20);
    store
        .insert_payout_accruals(&tenant, std::slice::from_ref(&unowned))
        .await
        .unwrap();
    assert_eq!(
        store
            .payout_owed(&tenant, accounts[0], &rail("near_ai"))
            .await
            .unwrap()
            .net(),
        Some(1_000_000_000)
    );
    assert_eq!(
        store
            .claim_payout_accruals(&tenant, &principal_hash, accounts[0], ACTOR)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .claim_payout_accruals(&tenant, &principal_hash, Uuid::new_v4(), ACTOR)
            .await
            .unwrap(),
        0,
        "an owned accrual moved to a second account"
    );
    assert_eq!(
        store
            .payout_owed(&tenant, accounts[0], &rail("near_ai"))
            .await
            .unwrap()
            .net(),
        Some(1_000_000_020)
    );
}

#[tokio::test]
async fn atomic_units_round_trip_numeric_39() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let big = 10u128.pow(38) - 1;
    let above_u64 = u64::MAX as u128 + 1;

    // Accruals.
    let batch = finalized_batch(&fixture.owner, &tenant).await;
    let hash = sha("account:big");
    store
        .insert_payout_accruals(
            &tenant,
            &[credit("near_ai", batch, Some(accounts[0]), &hash, big, big)],
        )
        .await
        .unwrap();
    let owed = store
        .payout_owed(&tenant, accounts[0], &rail("near_ai"))
        .await
        .unwrap();
    assert_eq!(owed.net(), Some(big));
    let batch_two = finalized_batch(&fixture.owner, &tenant).await;
    store
        .insert_payout_accruals(
            &tenant,
            &[credit(
                "test_keyed",
                batch_two,
                Some(accounts[0]),
                &hash,
                above_u64,
                above_u64,
            )],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .payout_owed(&tenant, accounts[0], &rail("test_keyed"))
            .await
            .unwrap()
            .net(),
        Some(above_u64)
    );

    // Lines.
    let live = link(accounts[0], "near_ai", PayoutLinkState::Active, "big");
    store.insert_payout_link(&tenant, &live).await.unwrap();
    let pending = line(&live, big, above_u64);
    store
        .insert_pending_payout_line(&tenant, &pending)
        .await
        .unwrap();
    let read = store
        .pending_payout_line_for_link(&tenant, live.link_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.target_total.get(), big);
    assert_eq!(read.delta.get(), above_u64);

    // Closed links' final totals.
    store
        .transition_payout_link(
            &tenant,
            live.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            None,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    store
        .transition_payout_link(
            &tenant,
            live.link_id,
            PayoutLinkState::Closing,
            PayoutLinkState::Closed,
            Some(AtomicUnits::from_raw(big)),
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .payout_closed_links_final_paid(&tenant, accounts[0], &rail("near_ai"))
            .await
            .unwrap(),
        big
    );

    // u128::MAX itself is stored and read back; one above it is refused by
    // the column check, never truncated.
    let batch_three = finalized_batch(&fixture.owner, &tenant).await;
    store
        .insert_payout_accruals(
            &tenant,
            &[credit(
                "max_rail",
                batch_three,
                Some(accounts[0]),
                &hash,
                u128::MAX,
                u128::MAX,
            )],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .payout_owed(&tenant, accounts[0], &rail("max_rail"))
            .await
            .unwrap()
            .net(),
        Some(u128::MAX)
    );
    // A second u128::MAX accrual on that rail would make the sum overflow:
    // owed refuses rather than wrap.
    let batch_four = finalized_batch(&fixture.owner, &tenant).await;
    store
        .insert_payout_accruals(
            &tenant,
            &[credit(
                "max_rail",
                batch_four,
                Some(accounts[0]),
                &hash,
                1,
                1,
            )],
        )
        .await
        .unwrap();
    assert!(
        store
            .payout_owed(&tenant, accounts[0], &rail("max_rail"))
            .await
            .is_err(),
        "owed above u128::MAX must be refused, never wrapped"
    );
}

#[tokio::test]
async fn the_enumerator_role_sees_only_tenant_rail_and_state() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant_a, accounts_a) = tenant_with_accounts(&fixture.owner, 2).await;
    let (tenant_b, accounts_b) = tenant_with_accounts(&fixture.owner, 1).await;
    let (tenant_c, accounts_c) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    store
        .insert_payout_link(
            &tenant_a,
            &link(accounts_a[0], "near_ai", PayoutLinkState::Active, "ea"),
        )
        .await
        .unwrap();
    store
        .insert_payout_link(
            &tenant_a,
            &link(accounts_a[1], "near_ai", PayoutLinkState::Active, "ea2"),
        )
        .await
        .unwrap();
    store
        .insert_payout_link(
            &tenant_b,
            &link(accounts_b[0], "near_ai", PayoutLinkState::Waiting, "eb"),
        )
        .await
        .unwrap();
    store
        .insert_payout_link(
            &tenant_c,
            &link(accounts_c[0], "test_keyed", PayoutLinkState::Active, "ec"),
        )
        .await
        .unwrap();

    let tenants = store
        .tenants_with_live_payout_links(&rail("near_ai"))
        .await
        .unwrap();
    assert!(tenants.contains(&tenant_a));
    assert!(!tenants.contains(&tenant_b), "a waiting link is not live");
    assert!(!tenants.contains(&tenant_c), "another rail's link");
    assert_eq!(
        tenants.iter().filter(|tenant| **tenant == tenant_a).count(),
        1
    );

    // Under the role, anything beyond the three columns is refused.
    let mut client = store.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_payout_enumerator")
        .await
        .unwrap();
    let error = tx
        .query("SELECT recipient_index FROM trace_payout_links", &[])
        .await
        .expect_err("the enumerator read a recipient index");
    assert!(is_permission_denied(&error), "{error}");
    drop(tx);
    for (sql, what) in [
        (
            "SELECT sealed_recipient FROM trace_payout_links",
            "sealed recipient",
        ),
        ("SELECT account_id FROM trace_payout_links", "account id"),
        ("SELECT tenant_id FROM trace_payout_lines", "lines"),
        ("SELECT tenant_id FROM trace_payout_accruals", "accruals"),
        (
            "UPDATE trace_payout_links SET state = state",
            "a link update",
        ),
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE trace_payout_enumerator")
            .await
            .unwrap();
        let error = tx
            .execute(sql, &[])
            .await
            .expect_err(&format!("the enumerator reached {what}"));
        assert!(is_permission_denied(&error), "{what}: {error}");
    }
    // The role is local to the transaction: the pooled connection is back to
    // the runtime login afterwards.
    let who: String = client
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(who, RUNTIME_LOGIN);
}

#[tokio::test]
async fn rate_epochs_and_control_are_not_writable_by_the_runtime_role() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rail_id = format!("rate_{}", &Uuid::new_v4().simple().to_string()[..12]);
    for sql in [
        format!(
            "INSERT INTO trace_payout_rate_epochs (rail_id, version, numerator, denominator)
             VALUES ('{rail_id}', 'v1', 1, 1)"
        ),
        format!(
            "INSERT INTO trace_payout_control (rail_id, change_id, halted, actor_ref_hash)
             VALUES ('{rail_id}', '{}', TRUE, '{ACTOR}')",
            Uuid::new_v4()
        ),
        "UPDATE trace_payout_rate_epochs SET numerator = numerator".into(),
        "DELETE FROM trace_payout_rate_epochs".into(),
        "UPDATE trace_payout_control SET halted = FALSE".into(),
        "DELETE FROM trace_payout_control".into(),
    ] {
        let error = runtime_exec(&fixture, None, &sql)
            .await
            .expect_err(&format!("the runtime role wrote: {sql}"));
        assert!(is_permission_denied(&error), "{sql}: {error}");
    }

    // The writer role, which the runtime login may SET ROLE to, can append.
    let store = &fixture.runtime;
    let rid = rail(&rail_id);
    let rate = RailRate::new("v1", 2_000_000_000, 1_000_000).unwrap();
    assert_eq!(
        store
            .record_payout_rate_epoch(&rid, &rate, Some("pricing-test"))
            .await
            .unwrap(),
        PayoutRateEpochRecord::Recorded
    );
    assert_eq!(
        store
            .record_payout_rate_epoch(&rid, &rate, Some("pricing-test"))
            .await
            .unwrap(),
        PayoutRateEpochRecord::AlreadyRecorded
    );
    let changed = RailRate::new("v1", 3, 1).unwrap();
    assert_eq!(
        store
            .record_payout_rate_epoch(&rid, &changed, None)
            .await
            .unwrap(),
        PayoutRateEpochRecord::Conflict
    );
    let epochs = store.payout_rate_epochs(&rid).await.unwrap();
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0].rate, rate);
    assert_eq!(epochs[0].rate.numerator(), 2_000_000_000);

    // Even the writer cannot rewrite or remove an epoch, nor backdate one.
    for sql in [
        "UPDATE trace_payout_rate_epochs SET numerator = 1".to_string(),
        "DELETE FROM trace_payout_rate_epochs".to_string(),
        format!(
            "INSERT INTO trace_payout_rate_epochs
                 (rail_id, version, numerator, denominator, observed_at)
             VALUES ('{rail_id}', 'v0', 1, 1, now() - interval '1 day')"
        ),
        "UPDATE trace_payout_control SET halted = FALSE".to_string(),
    ] {
        let mut client = store.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE trace_payout_config_writer")
            .await
            .unwrap();
        let error = tx
            .execute(sql.as_str(), &[])
            .await
            .expect_err(&format!("the writer ran: {sql}"));
        assert!(is_permission_denied(&error), "{sql}: {error}");
    }

    // Halt and resume append; the latest row is the state.
    assert!(!store.payout_control(&rid).await.unwrap().halted);
    store
        .set_payout_control(&rid, true, Some("payout_halted"), ACTOR)
        .await
        .unwrap();
    let control = store.payout_control(&rid).await.unwrap();
    assert!(control.halted);
    assert_eq!(control.label.as_deref(), Some("payout_halted"));
    store
        .set_payout_control(&rid, false, None, ACTOR)
        .await
        .unwrap();
    assert!(!store.payout_control(&rid).await.unwrap().halted);
    let history: i64 = fixture
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT count(*) FROM trace_payout_control WHERE rail_id = $1",
            &[&rail_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(history, 2, "halt and resume each leave a row");
}

#[tokio::test]
async fn audit_rows_cannot_be_updated_or_deleted() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    store
        .insert_payout_link(
            &tenant,
            &link(accounts[0], "near_ai", PayoutLinkState::Active, "audit"),
        )
        .await
        .unwrap();
    assert!(
        store
            .open_payout_hold(
                &tenant,
                accounts[0],
                &rail("near_ai"),
                "payout_over_ceiling",
                ACTOR
            )
            .await
            .unwrap()
    );
    assert!(
        !store
            .open_payout_hold(
                &tenant,
                accounts[0],
                &rail("near_ai"),
                "payout_over_ceiling",
                ACTOR
            )
            .await
            .unwrap(),
        "an open hold is opened once"
    );
    assert_eq!(
        store
            .open_payout_hold_labels(&tenant, accounts[0], &rail("near_ai"))
            .await
            .unwrap(),
        vec!["payout_over_ceiling".to_string()]
    );
    assert!(
        store
            .clear_payout_hold(
                &tenant,
                accounts[0],
                &rail("near_ai"),
                "payout_over_ceiling",
                ACTOR
            )
            .await
            .unwrap()
    );
    assert!(
        store
            .open_payout_hold_labels(&tenant, accounts[0], &rail("near_ai"))
            .await
            .unwrap()
            .is_empty()
    );
    let actions = store
        .payout_audit_actions(&tenant, &rail("near_ai"))
        .await
        .unwrap();
    assert_eq!(
        actions,
        vec![
            PayoutAuditAction::LinkCreated,
            PayoutAuditAction::HoldSet,
            PayoutAuditAction::HoldCleared,
        ]
    );
    for sql in [
        "UPDATE trace_payout_audit SET label = 'rewritten'",
        "DELETE FROM trace_payout_audit",
        "DELETE FROM trace_payout_holds",
        "DELETE FROM trace_payout_lines",
        "DELETE FROM trace_payout_accruals",
        "DELETE FROM trace_payout_links",
        "UPDATE trace_payout_accruals SET rail_units = 0",
        "UPDATE trace_payout_lines SET target_total = 0",
        "UPDATE trace_payout_links SET sealed_recipient = '{}'::jsonb",
    ] {
        let error = runtime_exec(&fixture, Some(&tenant), sql)
            .await
            .expect_err(&format!("the runtime role ran: {sql}"));
        assert!(is_permission_denied(&error), "{sql}: {error}");
    }
    // Rows are still there for the owner.
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    // The owner is the migration superuser, which RLS does not bound, so
    // name the tenant.
    let rows: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_payout_audit WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 3);
}

#[tokio::test]
async fn two_accounts_link_the_same_recipient_index() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let (other_tenant, other_accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    // One recipient, named by two accounts in one tenant and one in another.
    for (tenant, account) in [
        (&tenant, accounts[0]),
        (&tenant, accounts[1]),
        (&other_tenant, other_accounts[0]),
    ] {
        assert_eq!(
            store
                .insert_payout_link(
                    tenant,
                    &link(account, "near_ai", PayoutLinkState::Active, "shared-org"),
                )
                .await
                .unwrap(),
            PayoutLinkInsert::Inserted
        );
    }
    for (tenant, account) in [(&tenant, accounts[0]), (&tenant, accounts[1])] {
        let live = store
            .payout_live_link(tenant, account, &rail("near_ai"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live.recipient.recipient_index, sha("index:shared-org"));
    }
}

#[tokio::test]
async fn a_link_request_is_replayed_once_and_taken_once() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let key = Uuid::new_v4();
    let request = PayoutLinkRequestWrite {
        account_id: accounts[0],
        rail_id: rail("near_ai"),
        idempotency_key: key,
        handle_hash: sha(&format!("handle:{key}")),
        request_digest: sha(&format!("digest:{key}")),
        action: PayoutLinkAction::Link,
        expected_current_version: None,
        recipient: Some((
            RecipientKind::new("test_recipient").unwrap(),
            recipient("request"),
        )),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        actor_ref_hash: ACTOR.into(),
    };
    assert_eq!(
        store
            .insert_payout_link_request(&tenant, &request)
            .await
            .unwrap(),
        PayoutLinkRequestInsert::Inserted
    );
    assert_eq!(
        store
            .insert_payout_link_request(&tenant, &request)
            .await
            .unwrap(),
        PayoutLinkRequestInsert::Replayed
    );
    let changed = PayoutLinkRequestWrite {
        request_digest: sha("digest:changed"),
        ..request.clone()
    };
    assert_eq!(
        store
            .insert_payout_link_request(&tenant, &changed)
            .await
            .unwrap(),
        PayoutLinkRequestInsert::IdempotencyConflict
    );
    // An unlink carries no recipient; one that names one is refused.
    let bad_unlink = PayoutLinkRequestWrite {
        idempotency_key: Uuid::new_v4(),
        handle_hash: sha(&format!("handle:bad:{key}")),
        action: PayoutLinkAction::Unlink,
        ..request.clone()
    };
    assert!(
        store
            .insert_payout_link_request(&tenant, &bad_unlink)
            .await
            .is_err()
    );
    // Another tenant cannot take it.
    let (other, _) = tenant_with_accounts(&fixture.owner, 1).await;
    assert!(
        store
            .take_payout_link_request(&other, accounts[0], &rail("near_ai"), &request.handle_hash)
            .await
            .unwrap()
            .is_none()
    );
    let taken = store
        .take_payout_link_request(&tenant, accounts[0], &rail("near_ai"), &request.handle_hash)
        .await
        .unwrap()
        .expect("taken once");
    assert_eq!(taken.idempotency_key, key);
    assert_eq!(taken.action, PayoutLinkAction::Link);
    let (kind, sealed) = taken.recipient.expect("a link request's recipient");
    assert_eq!(kind.as_str(), "test_recipient");
    assert_eq!(sealed, recipient("request"));
    assert!(
        store
            .take_payout_link_request(&tenant, accounts[0], &rail("near_ai"), &request.handle_hash)
            .await
            .unwrap()
            .is_none(),
        "a request taken twice"
    );
    assert_eq!(
        store
            .payout_audit_actions(&tenant, &rail("near_ai"))
            .await
            .unwrap(),
        vec![PayoutAuditAction::LinkRequested]
    );
}

/// An account with payout rows refuses deletion at the DELETE, not at
/// COMMIT, so the reaper's per-candidate subtransaction can roll back one
/// candidate and go on (V101). A tenant delete still removes everything.
#[tokio::test]
async fn an_account_with_payout_rows_refuses_deletion_at_the_statement() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let batch = finalized_batch(&fixture.owner, &tenant).await;
    let store = &fixture.runtime;
    store
        .insert_payout_link(
            &tenant,
            &link(accounts[0], "near_ai", PayoutLinkState::Active, "reap"),
        )
        .await
        .unwrap();
    store
        .insert_payout_accruals(
            &tenant,
            &[credit(
                "near_ai",
                batch,
                Some(accounts[0]),
                &sha("account:reap"),
                1,
                1,
            )],
        )
        .await
        .unwrap();
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SAVEPOINT candidate").await.unwrap();
    let refused = tx
        .execute(
            "DELETE FROM trace_accounts WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &accounts[0]],
        )
        .await
        .expect_err("the account delete must be refused at the statement");
    assert_eq!(
        refused.code(),
        Some(&tokio_postgres::error::SqlState::FOREIGN_KEY_VIOLATION)
    );
    tx.batch_execute("ROLLBACK TO SAVEPOINT candidate")
        .await
        .unwrap();
    tx.commit()
        .await
        .expect("the batch commits after skipping one");

    let deleted = client
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("a tenant delete cascades through accounts and payout rows");
    assert_eq!(deleted, 1);
}
