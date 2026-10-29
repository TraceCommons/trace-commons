// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout rail tables (V132) against a real PostgreSQL.
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

// Only the V62 grant is read here, to pin the subset below against it.
#[allow(dead_code)]
#[path = "support/pilot_runtime_grants.rs"]
mod pilot_runtime_grants;

use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind,
};
use trace_commons_gate_api::{RailDescriptor, RailId, RailRate, RecipientKind};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::{
    Database, PayoutAccrualKind, PayoutAccrualWrite, PayoutAuditAction, PayoutHold,
    PayoutLineResolution, PayoutLineResolve, PayoutLineState, PayoutLineWrite, PayoutLinkAction,
    PayoutLinkClose, PayoutLinkInsert, PayoutLinkRequestInsert, PayoutLinkRequestWrite,
    PayoutLinkState, PayoutLinkTransition, PayoutLinkWrite, PayoutRateEpochRecord, PayoutStore,
    PendingPayoutLineInsert, SealedRecipient, payout_account_id_hash,
};
use uuid::Uuid;

/// The runtime login these tests connect as. Roles are per server; the
/// login is re-provisioned to exactly this shape on every run.
const RUNTIME_LOGIN: &str = "tc_payout_runtime_test";
const ACTOR: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The tables accrual reads that predate V62, which the runtime group
/// reaches on the pilot through its table-wide V62 grant. Granted alone
/// rather than the whole V62 grant, whose `ON ALL TABLES` would also cover
/// the payout tables and hide the narrower grants V132 gives them, which
/// this suite asserts the runtime cannot exceed.
const ACCRUAL_PILOT_TABLES: [&str; 3] = [
    "trace_credit_settlement_batches",
    "trace_credit_ledger",
    "trace_account_principals",
];
const ACCRUAL_PILOT_GRANTS: &str = "GRANT SELECT ON trace_credit_settlement_batches, \
    trace_credit_ledger, trace_account_principals TO trace_ingest_runtime;";

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
             GRANT trace_payout_config_writer TO {RUNTIME_LOGIN} WITH INHERIT FALSE, SET TRUE;
             -- Accrual only reads settled batches, the credit ledger and the
             -- principal links. Those tables predate V62, so the runtime
             -- group reaches them through the pilot's table-wide V62 grant
             -- (support/pilot_runtime_grants.rs), which a database migrated
             -- straight to head lacks. This is the part of it accrual uses;
             -- `accrual_grants_are_covered_by_the_pilot_v62_grant` pins it.
             {ACCRUAL_PILOT_GRANTS}"
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
    // Every accrual names a recorded rate epoch. The rails the tests price
    // on record version v1 at one fixed rate, the same on every run.
    for rail_id in ["near_ai", "test_keyed", "max_rail"] {
        let recorded = runtime
            .record_payout_rate_epoch(
                &RailId::new(rail_id).unwrap(),
                &RailRate::new("v1", 1, 1).unwrap(),
                None,
            )
            .await
            .expect("record the test rate epoch");
        assert_ne!(recorded, PayoutRateEpochRecord::Conflict, "{rail_id} v1");
    }
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
        recipient_ref: None,
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
        stream: trace_commons_server::db::PayoutStream::Earned,
        referral_program: None,
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
    // Closing: the active link moves to closing, and closing shares the
    // active slot, so neither a new active link nor a second closing one
    // fits beside it.
    let near_ai = rail("near_ai");
    let active = store
        .payout_live_link(&tenant, accounts[0], &near_ai)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Active,
                PayoutLinkState::Closing,
                PayoutAuditAction::LinkClosing,
                ACTOR,
                None,
            )
            .await
            .unwrap(),
        PayoutLinkTransition::Moved
    );
    let next = link(accounts[0], "near_ai", PayoutLinkState::Active, "next");
    assert_eq!(
        store.insert_payout_link(&tenant, &next).await.unwrap(),
        PayoutLinkInsert::StateTaken,
        "an active link inserted beside a closing one"
    );
    // An action that does not fit the move is refused before any write.
    assert!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Closing,
                PayoutLinkState::Closed,
                PayoutAuditAction::LinkClosed,
                ACTOR,
                None,
            )
            .await
            .is_err(),
        "closing to closed goes through close_payout_link"
    );
    assert!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Active,
                PayoutLinkState::Closing,
                PayoutAuditAction::LinkActivated,
                ACTOR,
                None,
            )
            .await
            .is_err(),
        "an audit action that does not fit the move"
    );
    // The close records the final total and frees the slot.
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                active.link_id,
                AtomicUnits::from_raw(7),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );
    assert_eq!(
        store
            .payout_closed_links_final_paid(&tenant, accounts[0], &near_ai)
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        store.insert_payout_link(&tenant, &next).await.unwrap(),
        PayoutLinkInsert::Inserted
    );
    // A transition or a close from the wrong state changes nothing.
    assert_eq!(
        store
            .transition_payout_link(
                &tenant,
                active.link_id,
                PayoutLinkState::Active,
                PayoutLinkState::Closing,
                PayoutAuditAction::LinkReleased,
                ACTOR,
                None,
            )
            .await
            .unwrap(),
        PayoutLinkTransition::NotInState
    );
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                active.link_id,
                AtomicUnits::from_raw(8),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::NotClosing
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
            .claim_payout_accruals(
                &tenant,
                &rail("near_ai"),
                &principal_hash,
                accounts[0],
                ACTOR
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .claim_payout_accruals(
                &tenant,
                &rail("near_ai"),
                &principal_hash,
                Uuid::new_v4(),
                ACTOR
            )
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

    // Closed links' final totals. The pending line is resolved first: a
    // link with a pending line does not close.
    store
        .resolve_payout_line(&tenant, pending.line_id, &PayoutLineResolution::NoOp)
        .await
        .unwrap();
    assert_eq!(
        store
            .transition_payout_link(
                &tenant,
                live.link_id,
                PayoutLinkState::Active,
                PayoutLinkState::Closing,
                PayoutAuditAction::LinkClosing,
                ACTOR,
                None,
            )
            .await
            .unwrap(),
        PayoutLinkTransition::Moved
    );
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                live.link_id,
                AtomicUnits::from_raw(big),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );
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
    let (tenant_d, accounts_d) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    // Tenant d's only link is closed: nothing left for a pass to do.
    let done = link(accounts_d[0], "near_ai", PayoutLinkState::Active, "ed");
    store.insert_payout_link(&tenant_d, &done).await.unwrap();
    store
        .transition_payout_link(
            &tenant_d,
            done.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .close_payout_link(
                &tenant_d,
                done.link_id,
                AtomicUnits::from_raw(0),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );
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
    // Every confirmed link is written `waiting` (spec section 5, step 7),
    // and only the pass makes it `active`, so a tenant whose only link is
    // waiting must be enumerated or its first link never activates.
    assert!(
        tenants.contains(&tenant_b),
        "a tenant whose only link is waiting must be enumerated"
    );
    assert!(!tenants.contains(&tenant_c), "another rail's link");
    assert!(
        !tenants.contains(&tenant_d),
        "a tenant whose only link is closed"
    );
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

/// The operator's view of a rail's holds: every open hold in one tenant on
/// one rail, by account and label. Cleared holds, other rails and other
/// tenants are not listed.
#[tokio::test]
async fn open_holds_are_listed_per_tenant_and_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let (other, other_accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let open = |tenant: &str, account: Uuid, rail_id: &str, label: &str| {
        let (tenant, rail_id, label) = (tenant.to_owned(), rail_id.to_owned(), label.to_owned());
        async move {
            assert!(
                store
                    .open_payout_hold(&tenant, account, &rail(&rail_id), &label, ACTOR)
                    .await
                    .unwrap()
            );
        }
    };
    open(&tenant, accounts[0], "near_ai", "payout_recovery_unmatched").await;
    open(&tenant, accounts[0], "near_ai", "payout_link_unreadable").await;
    open(&tenant, accounts[1], "near_ai", "payout_over_ceiling").await;
    open(&tenant, accounts[1], "test_keyed", "payout_over_ceiling").await;
    open(&other, other_accounts[0], "near_ai", "payout_over_ceiling").await;
    assert!(
        store
            .clear_payout_hold(
                &tenant,
                accounts[1],
                &rail("near_ai"),
                "payout_over_ceiling",
                ACTOR
            )
            .await
            .unwrap()
    );

    let mut listed = store
        .open_payout_holds(&tenant, &rail("near_ai"))
        .await
        .unwrap();
    listed.sort_by(|a, b| a.label.cmp(&b.label));
    assert_eq!(
        listed,
        vec![
            PayoutHold {
                account_id: accounts[0],
                label: "payout_link_unreadable".into()
            },
            PayoutHold {
                account_id: accounts[0],
                label: "payout_recovery_unmatched".into()
            },
        ]
    );
    let hash = payout_account_id_hash(accounts[0]);
    assert!(hash.starts_with("sha256:") && hash.len() == 71);
    assert!(!hash.contains(&accounts[0].to_string()));
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

#[tokio::test]
async fn a_waiting_link_cannot_activate_beside_a_closing_one() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let old = link(accounts[0], "near_ai", PayoutLinkState::Active, "g1");
    store.insert_payout_link(&tenant, &old).await.unwrap();
    let moved = store
        .transition_payout_link(
            &tenant,
            old.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(moved, PayoutLinkTransition::Moved);
    let next = link(accounts[0], "near_ai", PayoutLinkState::Waiting, "g2");
    store.insert_payout_link(&tenant, &next).await.unwrap();
    // The new link's target leaves out what the closing link was paid, so
    // activating it before the close would pay that amount twice.
    let activated = store
        .transition_payout_link(
            &tenant,
            next.link_id,
            PayoutLinkState::Waiting,
            PayoutLinkState::Active,
            PayoutAuditAction::LinkActivated,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        activated,
        PayoutLinkTransition::LiveLinkTaken,
        "a waiting link went active while the account's closing link was open"
    );
    // The close activates it instead, in the same transaction.
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                old.link_id,
                AtomicUnits::from_raw(4),
                Some(next.link_id),
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );
    let live = store
        .payout_live_link(&tenant, accounts[0], &rail("near_ai"))
        .await
        .unwrap()
        .expect("the successor is active");
    assert_eq!(live.link_id, next.link_id);
    let audit = store
        .payout_audit_actions(&tenant, &rail("near_ai"))
        .await
        .unwrap();
    assert_eq!(
        &audit[audit.len() - 2..],
        &[
            PayoutAuditAction::LinkClosed,
            PayoutAuditAction::LinkActivated
        ]
    );
}

#[tokio::test]
async fn a_close_with_a_bad_successor_changes_nothing() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let store = &fixture.runtime;
    let old = link(accounts[0], "near_ai", PayoutLinkState::Active, "s1");
    store.insert_payout_link(&tenant, &old).await.unwrap();
    store
        .transition_payout_link(
            &tenant,
            old.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    // Another account's waiting link is not this link's successor.
    let other = link(accounts[1], "near_ai", PayoutLinkState::Waiting, "s2");
    store.insert_payout_link(&tenant, &other).await.unwrap();
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                old.link_id,
                AtomicUnits::from_raw(1),
                Some(other.link_id),
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::SuccessorNotWaiting
    );
    // The close was rolled back with it: the link is still closing.
    assert_eq!(
        store
            .payout_closed_links_final_paid(&tenant, accounts[0], &rail("near_ai"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                old.link_id,
                AtomicUnits::from_raw(1),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );
}

#[tokio::test]
async fn a_closing_link_with_a_pending_line_cannot_close() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let old = link(accounts[0], "near_ai", PayoutLinkState::Active, "c1");
    store.insert_payout_link(&tenant, &old).await.unwrap();
    store
        .insert_pending_payout_line(&tenant, &line(&old, 9, 9))
        .await
        .unwrap();
    store
        .transition_payout_link(
            &tenant,
            old.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    // Recovery must resolve the line first: closing now would count a
    // final_paid that a later re-send to the closed program could exceed.
    let closed = store
        .close_payout_link(
            &tenant,
            old.link_id,
            AtomicUnits::from_raw(0),
            None,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        closed,
        PayoutLinkClose::PendingLineOpen,
        "a link closed with a pending line still on it"
    );
}

#[tokio::test]
async fn a_line_is_written_only_to_an_active_link() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let old = link(accounts[0], "near_ai", PayoutLinkState::Active, "l1");
    store.insert_payout_link(&tenant, &old).await.unwrap();
    store
        .transition_payout_link(
            &tenant,
            old.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    let waiting = link(accounts[0], "near_ai", PayoutLinkState::Waiting, "l2");
    store.insert_payout_link(&tenant, &waiting).await.unwrap();
    for (target, what) in [(&old, "closing"), (&waiting, "waiting")] {
        let inserted = store
            .insert_pending_payout_line(&tenant, &line(target, 3, 3))
            .await;
        assert_eq!(
            inserted.unwrap(),
            PendingPayoutLineInsert::LinkNotActive,
            "a line was written to a {what} link"
        );
    }
}

fn is_check_violation(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION)
}

/// The column grants let the runtime role write these columns at all; the
/// V132 triggers keep a settled value from being written again. Each
/// statement here runs as the runtime login, the role the store uses.
#[tokio::test]
async fn an_owned_accrual_keeps_its_owner() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let batch = finalized_batch(&fixture.owner, &tenant).await;
    let store = &fixture.runtime;
    let hash = sha(&format!("owned:{tenant}"));
    store
        .insert_payout_accruals(
            &tenant,
            &[credit("near_ai", batch, Some(accounts[0]), &hash, 5, 5)],
        )
        .await
        .unwrap();
    for (sql, what) in [
        (
            format!(
                "UPDATE trace_payout_accruals SET account_id = '{}' WHERE credit_account_hash = '{hash}'",
                accounts[1]
            ),
            "moved to another account",
        ),
        (
            format!(
                "UPDATE trace_payout_accruals SET account_id = NULL, claimed_at = NULL WHERE credit_account_hash = '{hash}'"
            ),
            "released back to nobody",
        ),
    ] {
        let error = runtime_exec(&fixture, Some(&tenant), &sql)
            .await
            .expect_err(&format!("an owned accrual was {what}"));
        assert!(is_check_violation(&error), "{what}: {error}");
    }
    // An unowned accrual is still claimed once, through the store.
    let unowned = sha(&format!("unowned:{tenant}"));
    let batch_two = finalized_batch(&fixture.owner, &tenant).await;
    store
        .insert_payout_accruals(
            &tenant,
            &[credit("near_ai", batch_two, None, &unowned, 3, 3)],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .claim_payout_accruals(&tenant, &rail("near_ai"), &unowned, accounts[1], ACTOR)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn a_resolved_line_is_final() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let live = link(accounts[0], "near_ai", PayoutLinkState::Active, "final");
    store.insert_payout_link(&tenant, &live).await.unwrap();
    let sent = line(&live, 10, 10);
    store
        .insert_pending_payout_line(&tenant, &sent)
        .await
        .unwrap();
    store
        .resolve_payout_line(
            &tenant,
            sent.line_id,
            &PayoutLineResolution::Confirmed {
                receipt_hash: sha(&format!("final-receipt:{tenant}")),
                delta: AtomicUnits::from_raw(10),
            },
        )
        .await
        .unwrap();
    let id = sent.line_id;
    for (sql, what) in [
        (
            format!(
                "UPDATE trace_payout_lines SET state = 'pending', resolved_at = NULL WHERE line_id = '{id}'"
            ),
            "reopened",
        ),
        (
            format!("UPDATE trace_payout_lines SET delta = 0 WHERE line_id = '{id}'"),
            "re-priced",
        ),
        (
            format!(
                "UPDATE trace_payout_lines SET receipt_hash = '{}' WHERE line_id = '{id}'",
                sha("another receipt")
            ),
            "given another receipt",
        ),
    ] {
        let error = runtime_exec(&fixture, Some(&tenant), &sql)
            .await
            .expect_err(&format!("a resolved line was {what}"));
        assert!(is_check_violation(&error), "{what}: {error}");
    }
}

#[tokio::test]
async fn a_closed_link_is_final_and_links_move_only_forward() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let store = &fixture.runtime;
    let old = link(accounts[0], "near_ai", PayoutLinkState::Active, "fin1");
    store.insert_payout_link(&tenant, &old).await.unwrap();
    store
        .transition_payout_link(
            &tenant,
            old.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    store
        .close_payout_link(
            &tenant,
            old.link_id,
            AtomicUnits::from_raw(6),
            None,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    let closed = old.link_id;
    let waiting = link(accounts[1], "near_ai", PayoutLinkState::Waiting, "fin2");
    store.insert_payout_link(&tenant, &waiting).await.unwrap();
    // A pending line on a closing link: the store refuses the close, and
    // so does the database if the store's check is ever raced or skipped.
    let busy = link(accounts[1], "test_keyed", PayoutLinkState::Active, "fin3");
    store.insert_payout_link(&tenant, &busy).await.unwrap();
    store
        .insert_pending_payout_line(&tenant, &line(&busy, 2, 2))
        .await
        .unwrap();
    runtime_exec(
        &fixture,
        Some(&tenant),
        &format!(
            "UPDATE trace_payout_links SET state = 'closing' WHERE link_id = '{}'",
            busy.link_id
        ),
    )
    .await
    .unwrap();
    for (sql, what) in [
        (
            format!("UPDATE trace_payout_links SET final_paid = 999 WHERE link_id = '{closed}'"),
            "a closed link's final_paid rewritten",
        ),
        (
            format!(
                "UPDATE trace_payout_links SET state = 'active', final_paid = NULL WHERE link_id = '{closed}'"
            ),
            "a closed link reopened",
        ),
        (
            format!(
                "UPDATE trace_payout_links SET state = 'closing' WHERE link_id = '{}'",
                waiting.link_id
            ),
            "a waiting link moved straight to closing",
        ),
        (
            format!(
                "UPDATE trace_payout_links SET state = 'closed', final_paid = 0 WHERE link_id = '{}'",
                busy.link_id
            ),
            "a link closed over a pending line",
        ),
    ] {
        let error = runtime_exec(&fixture, Some(&tenant), &sql)
            .await
            .expect_err(what);
        assert!(is_check_violation(&error), "{what}: {error}");
    }
}

#[tokio::test]
async fn the_pass_reads_and_marks_links_and_purges_expired_requests() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let store = &fixture.runtime;
    let near_ai = rail("near_ai");
    let active = link(accounts[0], "near_ai", PayoutLinkState::Active, "m1");
    let waiting = link(accounts[1], "near_ai", PayoutLinkState::Waiting, "m2");
    let other_rail = link(accounts[0], "test_keyed", PayoutLinkState::Waiting, "m3");
    for l in [&active, &waiting, &other_rail] {
        store.insert_payout_link(&tenant, l).await.unwrap();
    }

    // Waiting and closing links are readable per rail and state.
    let read = store
        .payout_links(&tenant, &near_ai, &[PayoutLinkState::Waiting])
        .await
        .unwrap();
    assert_eq!(
        read.iter().map(|l| l.link_id).collect::<Vec<_>>(),
        vec![waiting.link_id]
    );
    let read = store
        .payout_links(
            &tenant,
            &near_ai,
            &[PayoutLinkState::Active, PayoutLinkState::Waiting],
        )
        .await
        .unwrap();
    assert_eq!(read.len(), 2);

    // A confirmed unlink marks the active link once.
    assert!(
        store
            .mark_payout_link_unlink_requested(&tenant, active.link_id, ACTOR)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_payout_link_unlink_requested(&tenant, active.link_id, ACTOR)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_payout_link_unlink_requested(&tenant, waiting.link_id, ACTOR)
            .await
            .unwrap(),
        "only a live link is unlinked"
    );
    // A 404 from the rail marks the link's recipient unavailable once.
    assert!(
        store
            .mark_payout_link_recipient_unavailable(
                &tenant,
                active.link_id,
                ACTOR,
                Some("organization_unavailable")
            )
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_payout_link_recipient_unavailable(&tenant, active.link_id, ACTOR, None)
            .await
            .unwrap()
    );
    let marked = store
        .payout_live_link(&tenant, accounts[0], &near_ai)
        .await
        .unwrap()
        .unwrap();
    assert!(marked.unlink_requested_at.is_some());
    assert!(marked.recipient_unavailable_at.is_some());
    assert_eq!(marked.version, 3, "each mark is a new version");

    // An operator release records its own audit action.
    store
        .transition_payout_link(
            &tenant,
            active.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            PayoutAuditAction::LinkReleased,
            ACTOR,
            Some("operator_release"),
        )
        .await
        .unwrap();
    let audit = store.payout_audit_actions(&tenant, &near_ai).await.unwrap();
    for action in [
        PayoutAuditAction::UnlinkRequested,
        PayoutAuditAction::LinkRecipientUnavailable,
        PayoutAuditAction::LinkReleased,
    ] {
        assert!(audit.contains(&action), "{action:?} not audited");
    }
    let closing = store
        .payout_links(&tenant, &near_ai, &[PayoutLinkState::Closing])
        .await
        .unwrap();
    assert_eq!(closing.len(), 1);
    assert_eq!(closing[0].link_id, active.link_id);

    // Expired requests are purged; a live one stays.
    let request = |key: Uuid, expires: chrono::DateTime<chrono::Utc>| PayoutLinkRequestWrite {
        account_id: accounts[0],
        rail_id: near_ai.clone(),
        idempotency_key: key,
        handle_hash: sha(&format!("purge-handle:{key}")),
        request_digest: sha(&format!("purge-digest:{key}")),
        action: PayoutLinkAction::Unlink,
        expected_current_version: None,
        recipient: None,
        expires_at: expires,
        actor_ref_hash: ACTOR.into(),
    };
    let soon = request(
        Uuid::new_v4(),
        chrono::Utc::now() + chrono::Duration::minutes(5),
    );
    let later = request(
        Uuid::new_v4(),
        chrono::Utc::now() + chrono::Duration::minutes(5),
    );
    for r in [&soon, &later] {
        store.insert_payout_link_request(&tenant, r).await.unwrap();
    }
    // Expire `soon` by the database clock, as the owner, rather than wait
    // on the wall clock.
    let owner = fixture.owner.trace_pool_for_test().get().await.unwrap();
    assert_eq!(
        owner
            .execute(
                "UPDATE trace_payout_link_requests
                    SET created_at = clock_timestamp() - interval '2 hours',
                        expires_at = clock_timestamp() - interval '1 hour'
                  WHERE tenant_id = $1 AND handle_hash = $2",
                &[&tenant, &soon.handle_hash],
            )
            .await
            .unwrap(),
        1
    );
    drop(owner);
    assert_eq!(
        store
            .purge_expired_payout_link_requests(&tenant)
            .await
            .unwrap(),
        1
    );
    assert!(
        store
            .take_payout_link_request(&tenant, accounts[0], &near_ai, &soon.handle_hash)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .take_payout_link_request(&tenant, accounts[0], &near_ai, &later.handle_hash)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn the_later_control_row_wins_at_an_equal_timestamp() {
    let Some(fixture) = fixture().await else {
        return;
    };
    // A rail of this run's own, so other runs' rows cannot interleave.
    let rail_id = format!("ord_{}", &Uuid::new_v4().simple().to_string()[..12]);
    // As the owner, which may set changed_at: a halt, then a resume at the
    // same microsecond. Their random change ids order the other way round.
    let client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    for (change_id, halted) in [
        ("ffffffff-ffff-4fff-bfff-ffffffffffff", true),
        ("00000000-0000-4000-8000-000000000001", false),
    ] {
        client
            .execute(
                "INSERT INTO trace_payout_control
                     (rail_id, change_id, halted, actor_ref_hash, changed_at)
                 VALUES ($1, $2::TEXT::UUID, $3, $4, '2026-01-01T00:00:00Z')",
                &[&rail_id, &change_id, &halted, &ACTOR],
            )
            .await
            .unwrap();
    }
    drop(client);
    let control = fixture
        .runtime
        .payout_control(&rail(&rail_id))
        .await
        .unwrap();
    assert!(!control.halted, "the earlier halt beat the later resume");
}

/// The audit row's hash of an account id, as the store computes it.
fn account_id_hash(account: Uuid) -> String {
    sha(&format!("payout-account-v1:{account}"))
}

#[tokio::test]
async fn hold_and_link_audit_rows_name_the_account_by_hash() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let store = &fixture.runtime;
    let near_ai = rail("near_ai");
    store
        .open_payout_hold(&tenant, accounts[0], &near_ai, "ceiling", ACTOR)
        .await
        .unwrap();
    store
        .clear_payout_hold(&tenant, accounts[0], &near_ai, "ceiling", ACTOR)
        .await
        .unwrap();
    let live = link(accounts[0], "near_ai", PayoutLinkState::Waiting, "acct");
    store.insert_payout_link(&tenant, &live).await.unwrap();
    store
        .transition_payout_link(
            &tenant,
            live.link_id,
            PayoutLinkState::Waiting,
            PayoutLinkState::Active,
            PayoutAuditAction::LinkActivated,
            ACTOR,
            None,
        )
        .await
        .unwrap();

    let mut client = store.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let rows = tx
        .query(
            "SELECT action, account_id_hash FROM trace_payout_audit
              WHERE rail_id = 'near_ai' ORDER BY created_at, audit_id",
            &[],
        )
        .await
        .unwrap();
    let expected = account_id_hash(accounts[0]);
    let seen: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    assert_eq!(
        seen,
        ["hold_set", "hold_cleared", "link_created", "link_activated"]
    );
    for row in &rows {
        let action: String = row.get(0);
        let hash: Option<String> = row.get(1);
        assert_eq!(hash.as_deref(), Some(expected.as_str()), "{action}");
    }
    // Hash-only: the raw account id is in no audit column.
    let raw: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_payout_audit a
              WHERE strpos(row_to_json(a)::TEXT, $1) > 0",
            &[&accounts[0].to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(raw, 0);
}

#[tokio::test]
async fn an_accrual_names_a_recorded_rate_epoch() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let batch = finalized_batch(&fixture.owner, &tenant).await;
    let mut priced = credit(
        "near_ai",
        batch,
        Some(accounts[0]),
        &sha(&format!("epoch:{tenant}")),
        4,
        4,
    );
    // A version no boot ever recorded for this rail: the price cannot be
    // traced to a rate, so the row is refused.
    priced.rate_version = format!("never-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let error = fixture
        .runtime
        .insert_payout_accruals(&tenant, std::slice::from_ref(&priced))
        .await
        .expect_err("an accrual priced at an unrecorded rate epoch");
    assert!(
        format!("{error:?}").contains("trace_payout_accruals_rate_epoch"),
        "{error:?}"
    );
}

// ---------------------------------------------------------------------------
// Rate epochs and accrual (spec section 3 "Rate source", section 4 "Rate",
// "Accrual", "Reversals"). Rate epochs are deployment-wide and immutable,
// and the fixture pins v1 = 1/1 on near_ai, test_keyed and max_rail for
// every test, so every test below that records an epoch does it on a rail
// of its own. Rates are synthetic.
// ---------------------------------------------------------------------------

use chrono::{DateTime, Duration, Utc};
use trace_commons_server::db::PayoutAccrual;
use trace_commons_server::payout_accrual::{AccrualReport, accrue_rail};
use trace_commons_server::payout_rate::{
    PayoutMode, PayoutRateError, RailRateSource, RateEpochAtBoot, near_ai_rate_epoch_at_boot,
    record_rate_epoch,
};
use trace_commons_server::trace_corpus_storage::{
    TraceCreditAccountSettlementLineItem, TraceCreditSettlementNearStatus,
};

/// A rail id no other test uses.
fn fresh_rail(prefix: &str) -> RailId {
    rail(&format!(
        "{prefix}-{}",
        &Uuid::new_v4().simple().to_string()[..12]
    ))
}

struct FixedRate(Option<RailRate>);

impl RailRateSource for FixedRate {
    fn current(&self) -> Option<RailRate> {
        self.0.clone()
    }
}

async fn record_epoch(fixture: &Fixture, rail_id: &RailId, version: &str, num: u128, den: u128) {
    let outcome = record_rate_epoch(
        &fixture.runtime,
        rail_id,
        &FixedRate(Some(RailRate::new(version, num, den).unwrap())),
    )
    .await
    .expect("record the epoch");
    assert_eq!(outcome, RateEpochAtBoot::Recorded);
}

async fn observed_at(fixture: &Fixture, rail_id: &RailId, version: &str) -> DateTime<Utc> {
    fixture
        .runtime
        .payout_rate_epochs(rail_id)
        .await
        .unwrap()
        .into_iter()
        .find(|epoch| epoch.rate.version() == version)
        .expect("the epoch")
        .observed_at
}

fn account_ref(account: Uuid) -> String {
    format!("account:{account}")
}

fn item(
    credit_account_ref: &str,
    micros: i64,
    events: &[Uuid],
    hold: Option<&str>,
) -> TraceCreditAccountSettlementLineItem {
    TraceCreditAccountSettlementLineItem {
        credit_account_ref: credit_account_ref.to_owned(),
        credit_account_hash: sha(&format!("credit-account:{credit_account_ref}")),
        settled_credit_delta_micros: micros,
        source_credit_event_ids: events.to_vec(),
        source_submission_ids: Vec::new(),
        source_list_hash: sha(&format!("items:{credit_account_ref}:{events:?}")),
        near_status: TraceCreditSettlementNearStatus::Disabled,
        near_outbox_id: None,
        near_payout_hold_reason: hold.map(str::to_owned),
    }
}

/// A settlement batch with real line items, created as the owner, with an
/// explicit status and creation time.
async fn batch_with_lines(
    owner: &PgBackend,
    tenant: &str,
    status: &str,
    created_at: DateTime<Utc>,
    lines: &[TraceCreditAccountSettlementLineItem],
) -> Uuid {
    batch_with_sources(owner, tenant, status, created_at, lines, &[]).await
}

/// As [`batch_with_lines`], with `extra_sources` added to the batch-level
/// `source_credit_event_ids` but to no line: an event the batch settled
/// that no line carries.
async fn batch_with_sources(
    owner: &PgBackend,
    tenant: &str,
    status: &str,
    created_at: DateTime<Utc>,
    lines: &[TraceCreditAccountSettlementLineItem],
    extra_sources: &[Uuid],
) -> Uuid {
    let batch = Uuid::new_v4();
    let events: Vec<Uuid> = lines
        .iter()
        .flat_map(|line| line.source_credit_event_ids.iter().copied())
        .chain(extra_sources.iter().copied())
        .collect();
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
             source_credit_event_ids, source_list_hash, settled_credit_points,
             settled_credit_micros, line_items_json, actor_principal_ref, created_at
         ) VALUES ($1, $2, 'test', $3, $4, $5, $6, '1', 1000000, $7, 'operator', $8)",
        &[
            &tenant,
            &batch,
            &status,
            &sha(&format!("reason:{batch}")),
            &events,
            &sha(&format!("list:{batch}")),
            &serde_json::to_value(lines).unwrap(),
            &created_at,
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    batch
}

/// A credit ledger row (with the submission it needs), created as the owner.
async fn ledger_event(
    owner: &PgBackend,
    tenant: &str,
    event_id: Uuid,
    principal: &str,
    points: &str,
    settlement_state: &str,
    external_ref: Option<String>,
) {
    let submission = Uuid::new_v4();
    let trace = Uuid::new_v4();
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO trace_submissions (
             tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
             consent_policy_version, retention_policy_id, status, privacy_risk,
             redaction_pipeline_version, redaction_hash
         ) VALUES ($1, $2, $3, $4, 'v1', 'v1', 'test', 'accepted', 'low', 'test', $5)",
        &[&tenant, &submission, &trace, &principal, &"a".repeat(64)],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO trace_credit_ledger (
             tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
             event_type, points_delta, reason, external_ref, actor_principal_ref,
             actor_role, settlement_state
         ) VALUES ($1, $2, $3, $4, $5, 'accepted', $6, 'fixture', $7, 'operator',
                   'admin', $8)",
        &[
            &tenant,
            &event_id,
            &submission,
            &trace,
            &principal,
            &points,
            &external_ref,
            &settlement_state,
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

fn reversal_ref(event: Uuid) -> Option<String> {
    Some(format!("revocation_credit_reversal:{event}"))
}

/// Links `principal` to `account` in `trace_account_principals`, as the owner.
async fn link_principal(owner: &PgBackend, tenant: &str, account: Uuid, principal: &str) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(
        "DELETE FROM trace_account_principals WHERE tenant_id = $1 AND principal_ref = $2",
        &[&tenant, &principal],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
         VALUES ($1, $2, $3)",
        &[&tenant, &account, &principal],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// A test rail's descriptor: the rail and its instrument, with fixed
/// synthetic values for the rest.
fn rail_descriptor(rail_id: &RailId, instrument: &str) -> RailDescriptor {
    RailDescriptor {
        rail_id: rail_id.clone(),
        instrument_id: InstrumentId::new(instrument).unwrap(),
        instrument: InstrumentDescriptor {
            kind: InstrumentKind::Erc20,
            network: "1".to_owned(),
            contract: "0x00000000000000000000000000000000000000a1".to_owned(),
            decimals: 18,
        },
        recipient_kind: RecipientKind::new("test_address").unwrap(),
        wire_max: None,
    }
}

async fn accrue(
    fixture: &Fixture,
    tenant: &str,
    rail_id: &RailId,
    instrument: &str,
) -> AccrualReport {
    accrue_rail(
        &fixture.runtime,
        &fixture.runtime,
        tenant,
        &rail_descriptor(rail_id, instrument),
        ACTOR,
    )
    .await
    .expect("accrue the rail")
}

async fn accruals(fixture: &Fixture, tenant: &str, rail_id: &RailId) -> Vec<PayoutAccrual> {
    fixture
        .runtime
        .payout_accruals(tenant, rail_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_changed_rate_under_one_version_refuses_boot() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rail_id = fresh_rail("boot");
    let source = |num| FixedRate(Some(RailRate::new("table-1", num, 1_000_000).unwrap()));
    assert_eq!(
        record_rate_epoch(&fixture.runtime, &rail_id, &source(2_000_000_000)).await,
        Ok(RateEpochAtBoot::Recorded)
    );
    assert_eq!(
        record_rate_epoch(&fixture.runtime, &rail_id, &source(2_000_000_000)).await,
        Ok(RateEpochAtBoot::AlreadyRecorded),
        "the same rate under the same version boots"
    );
    assert_eq!(
        record_rate_epoch(&fixture.runtime, &rail_id, &source(3_000_000_000)).await,
        Err(PayoutRateError::VersionConflict),
        "a re-priced version refuses boot"
    );
    assert_eq!(
        PayoutRateError::VersionConflict.label(),
        "payout_rate_version_conflict"
    );
    let epochs = fixture.runtime.payout_rate_epochs(&rail_id).await.unwrap();
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0].rate.numerator(), 2_000_000_000, "unchanged");

    // Through the NEAR AI boot hook: the fixture recorded near_ai v1 = 1/1.
    // A settlement table that reuses version v1 at another rate refuses boot
    // and records nothing.
    let table: trace_commons_protocol::settlement_estimate::SettlementEstimateTable =
        serde_json::from_value(serde_json::json!({
            "schema_version": 2,
            "features_version": "lef1",
            "version": "v1",
            "pricing_version": "pricing-test",
            "instrument_id": "trace_credit",
            "atomic_units_per_credit": 1_000_000,
            "usd_micros_per_credit": 7,
            "published_at": "2026-10-20T00:00:00Z",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "cut_offs": [5.0],
            "tiers": [
                {"mean_atomic_units": 1_000_000, "withheld_share_micros": 300_000},
                {"mean_atomic_units": 4_000_000, "withheld_share_micros": 100_000}
            ],
            "aggregate_band": {"low_ratio_micros": 600_000, "high_ratio_micros": 1_500_000},
            "fit_rows": 40
        }))
        .unwrap();
    let before = fixture
        .runtime
        .payout_rate_epochs(&RailId::near_ai())
        .await
        .unwrap();
    for mode in [PayoutMode::DryRun, PayoutMode::Live] {
        assert_eq!(
            near_ai_rate_epoch_at_boot(mode, Some(&table), Some(&fixture.runtime)).await,
            Err(PayoutRateError::VersionConflict)
        );
    }
    assert_eq!(
        near_ai_rate_epoch_at_boot(PayoutMode::Disabled, Some(&table), Some(&fixture.runtime))
            .await,
        Ok(None),
        "a disabled rail records nothing and so cannot conflict"
    );
    assert_eq!(
        fixture
            .runtime
            .payout_rate_epochs(&RailId::near_ai())
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn epochs_cannot_be_backdated() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rail_id = fresh_rail("backdate");
    let clock = || async {
        let client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
        client
            .query_one("SELECT clock_timestamp()", &[])
            .await
            .unwrap()
            .get::<_, DateTime<Utc>>(0)
    };
    let before = clock().await;
    record_epoch(&fixture, &rail_id, "v1", 1, 1).await;
    let after = clock().await;
    let observed = observed_at(&fixture, &rail_id, "v1").await;
    assert!(
        before <= observed && observed <= after,
        "observed_at is the database clock at the insert"
    );

    // Not even the writer role can name its own observed_at.
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_payout_config_writer")
        .await
        .unwrap();
    let error = tx
        .execute(
            "INSERT INTO trace_payout_rate_epochs
                 (rail_id, version, numerator, denominator, observed_at)
             VALUES ($1, 'v0', 1, 1, now() - interval '30 days')",
            &[&rail_id.as_str()],
        )
        .await
        .expect_err("a backdated epoch");
    assert!(is_permission_denied(&error), "{error}");
}

#[tokio::test]
async fn a_batch_is_priced_at_the_epoch_in_force_at_its_time() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let rail_id = fresh_rail("epoch");
    record_epoch(&fixture, &rail_id, "v1", 2, 1).await;
    let first = observed_at(&fixture, &rail_id, "v1").await;
    record_epoch(&fixture, &rail_id, "v2", 3, 1).await;
    let second = observed_at(&fixture, &rail_id, "v2").await;
    assert!(first < second);

    let at_first = batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        first,
        &[item(&account_ref(accounts[0]), 100, &[], None)],
    )
    .await;
    let after_second = batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        // After the second epoch, and (unlike a whole second later) behind
        // the database clock by the time accrual reads it, so it is priced.
        second + Duration::microseconds(1),
        &[item(&account_ref(accounts[0]), 1_000, &[], None)],
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(report.credits_inserted, 2);

    let rows = accruals(&fixture, &tenant, &rail_id).await;
    let priced = |batch: Uuid| {
        rows.iter()
            .find(|row| row.settlement_batch_id == batch)
            .map(|row| (row.rate_version.clone(), row.rail_units.get()))
            .expect("an accrual for the batch")
    };
    assert_eq!(priced(at_first), ("v1".to_owned(), 200));
    assert_eq!(priced(after_second), ("v2".to_owned(), 3_000));
    assert!(rows.iter().all(|row| row.account_id == Some(accounts[0])));
}

#[tokio::test]
async fn a_batch_older_than_the_first_epoch_uses_the_first() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let rail_id = fresh_rail("first");
    record_epoch(&fixture, &rail_id, "v1", 5, 1).await;
    let first = observed_at(&fixture, &rail_id, "v1").await;
    record_epoch(&fixture, &rail_id, "v2", 7, 1).await;
    let batch = batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        first - Duration::days(30),
        &[item(&account_ref(accounts[0]), 10, &[], None)],
    )
    .await;
    accrue(&fixture, &tenant, &rail_id, "test_units").await;
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].settlement_batch_id, batch);
    assert_eq!(rows[0].rate_version, "v1");
    assert_eq!(rows[0].rail_units.get(), 50);
}

/// A batch dated ahead of the database clock (application clock skew) is
/// not priced yet: an epoch recorded before the database reaches its time
/// would be the one in force for it.
#[tokio::test]
async fn a_batch_dated_ahead_of_the_database_clock_is_not_priced_yet() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let rail_id = fresh_rail("future");
    record_epoch(&fixture, &rail_id, "v1", 1, 1).await;
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now() + Duration::hours(1),
        &[item(&account_ref(accounts[0]), 5, &[], None)],
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(report.credits_inserted, 0);
    assert_eq!(report.skipped.get("payout_batch_in_future"), Some(&1));
    assert!(accruals(&fixture, &tenant, &rail_id).await.is_empty());
}

#[tokio::test]
async fn a_rail_without_an_epoch_accrues_nothing() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let rail_id = fresh_rail("norate");
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[item(&account_ref(accounts[0]), 10, &[], None)],
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert!(report.no_rate_epoch);
    assert_eq!(report.credits_inserted, 0);
    assert!(accruals(&fixture, &tenant, &rail_id).await.is_empty());
}

#[tokio::test]
async fn only_finalized_positive_lines_accrue() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let rail_id = fresh_rail("final");
    record_epoch(&fixture, &rail_id, "v1", 1, 1).await;
    let owner_ref = account_ref(accounts[0]);
    let now = Utc::now();
    for status in ["dry_run", "failed"] {
        batch_with_lines(
            &fixture.owner,
            &tenant,
            status,
            now,
            &[item(&owner_ref, 10, &[], None)],
        )
        .await;
    }
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        now,
        &[
            item(&owner_ref, 0, &[], None),
            item("principal-negative", -5, &[], None),
        ],
    )
    .await;
    let paid = batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        now,
        &[item(&owner_ref, 9, &[], None)],
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(report.credits_inserted, 1);
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].settlement_batch_id, paid);
    assert_eq!(rows[0].award_units.get(), 9);
}

#[tokio::test]
async fn a_bare_principal_line_accrues_unowned_and_is_claimed_when_linked() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let rail_id = fresh_rail("bare");
    let other = fresh_rail("bare-other");
    record_epoch(&fixture, &rail_id, "v1", 4, 1).await;
    record_epoch(&fixture, &other, "v1", 1, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[item(&principal, 25, &[], None)],
    )
    .await;

    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!((report.credits_inserted, report.claimed), (1, 0));
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].account_id, None, "owed to nobody yet");
    let owed = fixture
        .runtime
        .payout_owed(&tenant, accounts[0], &rail_id)
        .await
        .unwrap();
    assert_eq!(owed.net(), Some(0));

    accrue(&fixture, &tenant, &other, "test_units").await;

    link_principal(&fixture.owner, &tenant, accounts[0], &principal).await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!((report.credits_inserted, report.claimed), (0, 1));
    assert_eq!(
        accruals(&fixture, &tenant, &other).await[0].account_id,
        None,
        "one rail's claim writes nothing on another rail"
    );
    assert_eq!(
        accrue(&fixture, &tenant, &other, "test_units")
            .await
            .claimed,
        1
    );
    assert_eq!(
        accruals(&fixture, &tenant, &other).await[0].account_id,
        Some(accounts[0])
    );
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    assert_eq!(rows[0].account_id, Some(accounts[0]));
    let owed = fixture
        .runtime
        .payout_owed(&tenant, accounts[0], &rail_id)
        .await
        .unwrap();
    assert_eq!(owed.net(), Some(100));
    assert_eq!(
        fixture
            .runtime
            .payout_audit_actions(&tenant, &rail_id)
            .await
            .unwrap(),
        vec![PayoutAuditAction::AccrualClaimed]
    );
}

#[tokio::test]
async fn a_claimed_accrual_never_moves_to_a_second_account() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let rail_id = fresh_rail("moved");
    record_epoch(&fixture, &rail_id, "v1", 1, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[item(&principal, 30, &[], None)],
    )
    .await;
    link_principal(&fixture.owner, &tenant, accounts[0], &principal).await;
    assert_eq!(
        accrue(&fixture, &tenant, &rail_id, "test_units")
            .await
            .claimed,
        1
    );

    // The principal moves to a second account.
    link_principal(&fixture.owner, &tenant, accounts[1], &principal).await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!((report.credits_inserted, report.claimed), (0, 0));
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    assert_eq!(
        rows[0].account_id,
        Some(accounts[0]),
        "stays with the first"
    );
    for (account, expected) in [(accounts[0], 30), (accounts[1], 0)] {
        let owed = fixture
            .runtime
            .payout_owed(&tenant, account, &rail_id)
            .await
            .unwrap();
        assert_eq!(owed.net(), Some(expected));
    }
}

#[tokio::test]
async fn a_near_wallet_hold_label_does_not_stop_near_ai_accrual() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    // The fixture recorded near_ai v1; nothing in this suite records a
    // different near_ai epoch.
    let near_ai = RailId::near_ai();
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[
            item(&account_ref(accounts[0]), 40, &[], Some("none_enrolled")),
            item(
                &account_ref(accounts[1]),
                60,
                &[],
                Some("ambiguous_no_designation"),
            ),
        ],
    )
    .await;
    let report = accrue(&fixture, &tenant, &near_ai, "near_ai_usd").await;
    assert_eq!(report.credits_inserted, 2);
    assert_eq!(report.holds_opened, 0);
    let rows = accruals(&fixture, &tenant, &near_ai).await;
    for (account, award) in [(accounts[0], 40), (accounts[1], 60)] {
        let row = rows
            .iter()
            .find(|row| row.account_id == Some(account))
            .expect("the line accrued to its own account");
        assert_eq!(row.award_units.get(), award);
        assert!(
            fixture
                .runtime
                .open_payout_hold_labels(&tenant, account, &near_ai)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn a_reversal_takes_back_exactly_its_credit_on_each_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    link_principal(&fixture.owner, &tenant, accounts[0], &principal).await;
    let near_like = fresh_rail("rev-nano");
    let keyed_like = fresh_rail("rev-wei");
    record_epoch(&fixture, &near_like, "v1", 2_000_000_000, 1_000_000).await;
    record_epoch(&fixture, &keyed_like, "v1", 1_000_000_000_000, 1).await;

    let credited = Uuid::new_v4();
    let reversal = Uuid::new_v4();
    ledger_event(
        &fixture.owner,
        &tenant,
        credited,
        &principal,
        "1.5",
        "final",
        None,
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        reversal,
        &principal,
        "-1.5",
        "reversed",
        reversal_ref(credited),
    )
    .await;
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[item(
            &account_ref(accounts[0]),
            1_500_000,
            &[credited],
            None,
        )],
    )
    .await;

    for (rail_id, instrument, credit_units) in [
        (&near_like, "near_ai_usd", 3_000_000_000_u128),
        (&keyed_like, "test_token", 1_500_000_000_000_000_000),
    ] {
        let report = accrue(&fixture, &tenant, rail_id, instrument).await;
        assert_eq!((report.credits_inserted, report.reversals_inserted), (1, 1));
        let rows = accruals(&fixture, &tenant, rail_id).await;
        let credit = rows
            .iter()
            .find(|row| row.kind == PayoutAccrualKind::Credit)
            .unwrap();
        let taken = rows
            .iter()
            .find(|row| row.kind == PayoutAccrualKind::Reversal)
            .unwrap();
        assert_eq!(credit.rail_units.get(), credit_units);
        assert_eq!(taken.rail_units, credit.rail_units, "exactly its credit");
        assert_eq!(taken.award_units, credit.award_units);
        assert_eq!(taken.rate_version, credit.rate_version);
        assert_eq!(taken.reversal_event_id, Some(reversal));
        assert_eq!(taken.account_id, Some(accounts[0]));
        assert_eq!(taken.settlement_batch_id, credit.settlement_batch_id);
        let owed = fixture
            .runtime
            .payout_owed(&tenant, accounts[0], rail_id)
            .await
            .unwrap();
        assert_eq!((owed.credited, owed.reversed), (credit_units, credit_units));
        assert_eq!(owed.net(), Some(0));
    }
}

/// Ingest writes each event's amount as `format!("{:.4}", f32)` and the
/// line as the sum of each event's own rounding, so two events' text can
/// sum to more than their line. Reversing both takes back the line, once.
#[tokio::test]
async fn reversals_of_every_event_in_a_line_take_back_exactly_the_line() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    link_principal(&fixture.owner, &tenant, accounts[0], &principal).await;
    let rail_id = fresh_rail("rev-many");
    record_epoch(&fixture, &rail_id, "v1", 3, 7).await;
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    for source in [first, second] {
        ledger_event(
            &fixture.owner,
            &tenant,
            source,
            &principal,
            "0.8000",
            "final",
            None,
        )
        .await;
    }
    batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now() - Duration::hours(1),
        &[item(
            &account_ref(accounts[0]),
            1_000_000,
            &[first, second],
            None,
        )],
    )
    .await;
    let owed = || {
        let fixture = &fixture;
        let tenant = tenant.clone();
        let rail_id = rail_id.clone();
        let account = accounts[0];
        async move {
            fixture
                .runtime
                .payout_owed(&tenant, account, &rail_id)
                .await
                .unwrap()
        }
    };

    // The first reversal arrives alone and takes back its own amount.
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-0.8000",
        "reversed",
        reversal_ref(first),
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!((report.credits_inserted, report.reversals_inserted), (1, 1));
    // floor(1,000,000 * 3 / 7) and floor(800,000 * 3 / 7).
    let after_first = owed().await;
    assert_eq!(
        (after_first.credited, after_first.reversed),
        (428_571, 342_857)
    );

    // The second, in a later run, takes back only what is left of the line.
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-0.8000",
        "reversed",
        reversal_ref(second),
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(report.reversals_inserted, 1);
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    let taken: u128 = rows
        .iter()
        .filter(|row| row.kind == PayoutAccrualKind::Reversal)
        .map(|row| row.award_units.get())
        .sum();
    assert_eq!(taken, 1_000_000, "never more than the line settled");
    let after_both = owed().await;
    assert_eq!(after_both.credited, after_both.reversed);
    assert_eq!(after_both.net(), Some(0));
}

#[tokio::test]
async fn an_unlocatable_reversal_holds_only_that_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 3).await;
    let settled_principal = format!("principal-{}", Uuid::new_v4());
    let unbatched_principal = format!("principal-{}", Uuid::new_v4());
    let pending_principal = format!("principal-{}", Uuid::new_v4());
    link_principal(&fixture.owner, &tenant, accounts[0], &settled_principal).await;
    link_principal(&fixture.owner, &tenant, accounts[1], &unbatched_principal).await;
    link_principal(&fixture.owner, &tenant, accounts[2], &pending_principal).await;
    let rail_a = fresh_rail("hold-a");
    let rail_b = fresh_rail("hold-b");
    record_epoch(&fixture, &rail_a, "v1", 1, 1).await;
    record_epoch(&fixture, &rail_b, "v1", 1, 1).await;

    // A settled event: a finalized batch lists it, but no line carries it.
    // Its reversal cannot be priced, so the account is held on the rail
    // that tried.
    let settled = Uuid::new_v4();
    ledger_event(
        &fixture.owner,
        &tenant,
        settled,
        &settled_principal,
        "2.0000",
        "final",
        None,
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &settled_principal,
        "-2.0000",
        "reversed",
        reversal_ref(settled),
    )
    .await;
    batch_with_sources(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now() - Duration::hours(1),
        &[],
        &[settled],
    )
    .await;
    // An event ingest wrote `final` (as it writes every ordinary event)
    // that no finalized batch carries: it never settled, so nothing was
    // accrued and nothing is written or held.
    let unbatched = Uuid::new_v4();
    ledger_event(
        &fixture.owner,
        &tenant,
        unbatched,
        &unbatched_principal,
        "3.0000",
        "final",
        None,
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &unbatched_principal,
        "-3.0000",
        "reversed",
        reversal_ref(unbatched),
    )
    .await;
    // A pending event: likewise.
    let pending = Uuid::new_v4();
    ledger_event(
        &fixture.owner,
        &tenant,
        pending,
        &pending_principal,
        "4.0000",
        "pending",
        None,
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &pending_principal,
        "-4.0000",
        "reversed",
        reversal_ref(pending),
    )
    .await;

    let labels = |account: Uuid, rail_id: RailId| {
        let fixture = &fixture;
        let tenant = tenant.clone();
        async move {
            fixture
                .runtime
                .open_payout_hold_labels(&tenant, account, &rail_id)
                .await
                .unwrap()
        }
    };
    let report = accrue(&fixture, &tenant, &rail_a, "test_units").await;
    assert_eq!(report.holds_opened, 1);
    assert_eq!(report.reversals_inserted, 0);
    assert_eq!(report.skipped.get("payout_reversal_unsettled"), Some(&2));
    assert_eq!(
        labels(accounts[0], rail_a.clone()).await,
        vec!["payout_reversal_unpriced".to_owned()]
    );
    assert!(
        labels(accounts[0], rail_b.clone()).await.is_empty(),
        "rail b is not held by rail a's pass"
    );
    assert!(
        labels(accounts[1], rail_a.clone()).await.is_empty(),
        "a final event in no finalized batch never settled"
    );
    assert!(labels(accounts[2], rail_a.clone()).await.is_empty());

    accrue(&fixture, &tenant, &rail_b, "test_units").await;
    assert_eq!(
        labels(accounts[0], rail_b.clone()).await,
        vec!["payout_reversal_unpriced".to_owned()]
    );
    assert_eq!(labels(accounts[0], rail_a.clone()).await.len(), 1);
    assert!(labels(accounts[1], rail_b.clone()).await.is_empty());
    assert!(accruals(&fixture, &tenant, &rail_a).await.is_empty());
}

/// An operator clears a `payout_reversal_unpriced` hold; later accrual runs
/// do not reopen it for the same reversal, but a new reversal that cannot
/// be priced opens a fresh one.
#[tokio::test]
async fn a_cleared_reversal_hold_stays_cleared() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    link_principal(&fixture.owner, &tenant, accounts[0], &principal).await;
    let rail_id = fresh_rail("hold-clear");
    record_epoch(&fixture, &rail_id, "v1", 1, 1).await;
    let (lost, later) = (Uuid::new_v4(), Uuid::new_v4());
    for source in [lost, later] {
        ledger_event(
            &fixture.owner,
            &tenant,
            source,
            &principal,
            "2.0000",
            "final",
            None,
        )
        .await;
    }
    // Both settled by a batch's own list, neither carried by a line.
    batch_with_sources(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now() - Duration::hours(1),
        &[],
        &[lost, later],
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-2.0000",
        "reversed",
        reversal_ref(lost),
    )
    .await;
    let labels = || {
        let fixture = &fixture;
        let tenant = tenant.clone();
        let rail_id = rail_id.clone();
        let account = accounts[0];
        async move {
            fixture
                .runtime
                .open_payout_hold_labels(&tenant, account, &rail_id)
                .await
                .unwrap()
        }
    };
    let hold_sets = || {
        let fixture = &fixture;
        let tenant = tenant.clone();
        let rail_id = rail_id.clone();
        async move {
            fixture
                .runtime
                .payout_audit_actions(&tenant, &rail_id)
                .await
                .unwrap()
                .into_iter()
                .filter(|action| *action == PayoutAuditAction::HoldSet)
                .count()
        }
    };

    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(report.holds_opened, 1);
    assert_eq!(labels().await, vec!["payout_reversal_unpriced".to_owned()]);
    assert!(
        fixture
            .runtime
            .clear_payout_hold(
                &tenant,
                accounts[0],
                &rail_id,
                "payout_reversal_unpriced",
                ACTOR
            )
            .await
            .unwrap()
    );

    for _ in 0..2 {
        let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
        assert_eq!(report.holds_opened, 0, "the cleared hold stays cleared");
    }
    assert!(labels().await.is_empty());
    assert_eq!(hold_sets().await, 1, "no hold_set row per run");

    // A new reversal that cannot be priced is a new decision.
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-2.0000",
        "reversed",
        reversal_ref(later),
    )
    .await;
    let report = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(report.holds_opened, 1);
    assert_eq!(labels().await, vec!["payout_reversal_unpriced".to_owned()]);
    assert_eq!(hold_sets().await, 2);
}

#[tokio::test]
async fn rails_accrue_the_same_line_independently() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 1).await;
    let nano = fresh_rail("ind-nano");
    let wei = fresh_rail("ind-wei");
    record_epoch(&fixture, &nano, "v1", 1_000, 1).await;
    record_epoch(&fixture, &wei, "v1", 1_000_000_000_000, 1).await;
    let batch = batch_with_lines(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[item(&account_ref(accounts[0]), 2_000_000, &[], None)],
    )
    .await;

    accrue(&fixture, &tenant, &nano, "near_ai_usd").await;
    assert!(
        accruals(&fixture, &tenant, &wei).await.is_empty(),
        "one rail's accrual writes nothing on another"
    );
    accrue(&fixture, &tenant, &wei, "test_token").await;

    for (rail_id, instrument, units) in [
        (&nano, "near_ai_usd", 2_000_000_000_u128),
        (&wei, "test_token", 2_000_000_000_000_000_000),
    ] {
        let rows = accruals(&fixture, &tenant, rail_id).await;
        assert_eq!(rows.len(), 1, "{rail_id:?}");
        assert_eq!(rows[0].settlement_batch_id, batch);
        assert_eq!(rows[0].instrument_id.as_str(), instrument);
        assert_eq!(rows[0].award_units.get(), 2_000_000);
        assert_eq!(rows[0].rail_units.get(), units);
    }
}

#[tokio::test]
async fn rerunning_accrual_writes_nothing_new() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let rail_id = fresh_rail("rerun");
    record_epoch(&fixture, &rail_id, "v1", 3, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    link_principal(&fixture.owner, &tenant, accounts[1], &principal).await;
    let credited = Uuid::new_v4();
    ledger_event(
        &fixture.owner,
        &tenant,
        credited,
        &principal,
        "1",
        "final",
        None,
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-1",
        "reversed",
        reversal_ref(credited),
    )
    .await;
    let lost = Uuid::new_v4();
    ledger_event(
        &fixture.owner,
        &tenant,
        lost,
        &principal,
        "1",
        "final",
        None,
    )
    .await;
    ledger_event(
        &fixture.owner,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-1",
        "reversed",
        reversal_ref(lost),
    )
    .await;
    // `lost` is settled by the batch's own list but carried by no line, so
    // its reversal cannot be priced and holds the account.
    batch_with_sources(
        &fixture.owner,
        &tenant,
        "finalized",
        Utc::now(),
        &[
            item(&account_ref(accounts[0]), 7, &[], None),
            item(&principal, 1_000_000, &[credited], None),
        ],
        &[lost],
    )
    .await;

    let first = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(
        (
            first.credits_inserted,
            first.reversals_inserted,
            first.claimed,
            first.holds_opened
        ),
        (2, 1, 2, 1)
    );
    let rows = accruals(&fixture, &tenant, &rail_id).await;
    let audit = fixture
        .runtime
        .payout_audit_actions(&tenant, &rail_id)
        .await
        .unwrap();

    let second = accrue(&fixture, &tenant, &rail_id, "test_units").await;
    assert_eq!(
        (
            second.credits_inserted,
            second.reversals_inserted,
            second.claimed,
            second.holds_opened
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(accruals(&fixture, &tenant, &rail_id).await, rows);
    assert_eq!(
        fixture
            .runtime
            .payout_audit_actions(&tenant, &rail_id)
            .await
            .unwrap(),
        audit
    );
}

/// The fixture's hand-picked grant is a part of the pilot's V62 grant, not
/// something beside it: every table it names existed by V62 (so the
/// pilot's `ON ALL TABLES` grant covers it) and is not revoked there, and
/// it grants only SELECT. Accrual needs no write outside the payout tables.
#[test]
fn accrual_grants_are_covered_by_the_pilot_v62_grant() {
    let pilot = pilot_runtime_grants::PILOT_V62_RUNTIME_GRANTS;
    assert!(pilot.contains(
        "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO trace_ingest_runtime;"
    ));
    let granted = ACCRUAL_PILOT_GRANTS
        .strip_prefix("GRANT SELECT ON ")
        .and_then(|rest| rest.strip_suffix(" TO trace_ingest_runtime;"))
        .expect("a SELECT-only grant to the runtime group");
    let granted: Vec<&str> = granted.split(',').map(str::trim).collect();
    assert_eq!(granted, ACCRUAL_PILOT_TABLES);

    let migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    for table in ACCRUAL_PILOT_TABLES {
        let revoked = pilot
            .lines()
            .any(|line| line.trim_start().starts_with("REVOKE") && line.contains(table));
        assert!(!revoked, "{table} is revoked in the pilot's V62 grant");
        let created_in: Vec<u32> = std::fs::read_dir(&migrations)
            .expect("the migrations directory")
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                let name = path.file_name()?.to_str()?.to_owned();
                let version: u32 = name.strip_prefix('V')?.split("__").next()?.parse().ok()?;
                let sql = std::fs::read_to_string(&path).ok()?;
                let creates = [
                    format!("CREATE TABLE {table} ("),
                    format!("CREATE TABLE IF NOT EXISTS {table} ("),
                ]
                .iter()
                .any(|statement| sql.contains(statement.as_str()));
                creates.then_some(version)
            })
            .collect();
        assert_eq!(created_in.len(), 1, "{table}: {created_in:?}");
        assert!(created_in[0] <= 62, "{table} was created after V62");
    }
}
