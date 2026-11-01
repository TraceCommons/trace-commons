// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The per-rail payout pass against a real PostgreSQL, a loopback mock of
//! cloud-api #1271 (the `near_ai` rail) and the keyed 18-decimal test rail
//! (payout spec sections 3, 4, 5 "Change and unlink", and 7).
//!
//! Links are created through the store directly: the link routes arrive in
//! a later change. Every store call runs as a real `LOGIN` role holding only
//! `trace_ingest_runtime` and, with `INHERIT FALSE`, the two narrow payout
//! roles, never the migration owner or a superuser, so a missing grant fails
//! here as it would in production.
//!
//! The rails are deployment-wide, so a pass enumerates every tenant with a
//! link on its rail, including tenants other tests left behind. The test
//! opener opens only the links its own test made; any other link will not
//! open and is held, so assertions read one test's own tenant.
//!
//! Needs PostgreSQL 16 or later, a database named `admission_test_*` on the
//! literal host `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`.
//! Without the variable every test prints a skip line and returns. Run with
//! `--test-threads=1`: the rails' kill switches are deployment-wide.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

#[path = "support/near_ai_payout_mock.rs"]
mod near_ai_payout_mock;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use near_ai_payout_mock::{Mock, Scripted, recipient_org};
use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::AtomicUnits;
use trace_commons_gate_api::{
    PayoutRail, RailCapacity, RailDescriptor, RailHistoryEntry, RailId, RailLine, RailRate,
    RailRefusal, ReceiptLookup, RecipientKind, RecipientRef, SendOutcome,
};
use trace_commons_gate_enclave::MockKeyedPayoutRail;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::{
    Database, PayoutLineResolution, PayoutLineWrite, PayoutLink, PayoutLinkInsert, PayoutLinkState,
    PayoutLinkWrite, PayoutRateEpochRecord, PayoutStore, PendingPayoutLineInsert, SealedRecipient,
};
use trace_commons_server::near_ai_rail::near_ai_receipt_hash;
use trace_commons_server::payout_pass::{
    PayoutPass, PayoutPassOutcome, PayoutPassReport, PayoutPassSettings, PayoutRecipientOpener,
};
use trace_commons_server::payout_rate::PayoutMode;
use trace_commons_server::trace_corpus_storage::{
    TraceCreditAccountSettlementLineItem, TraceCreditSettlementNearStatus,
};
use uuid::Uuid;

const RUNTIME_LOGIN: &str = "tc_payout_pass_runtime_test";
const ACTOR: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const CEILING: u128 = 1_000_000_000_000;

/// The pilot tables the pass reads that predate V62: settlement batches,
/// the ledger and principal links (accrual), and accounts and credit holds
/// (the shared account standing). The pilot runtime group reaches them
/// through its table-wide V62 grant; a database migrated straight to head
/// lacks it, so this suite grants exactly these.
const PASS_PILOT_GRANTS: &str = "GRANT SELECT ON trace_credit_settlement_batches, \
    trace_credit_ledger, trace_account_principals, trace_accounts, trace_credit_holds \
    TO trace_ingest_runtime;";

struct Fixture {
    owner: PgBackend,
    runtime: Arc<PgBackend>,
}

fn config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url.to_owned()),
        pool_size: 6,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

fn rail_id(value: &str) -> RailId {
    RailId::new(value).unwrap()
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
    let client = owner.trace_pool_for_test().get().await.unwrap();
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
             {PASS_PILOT_GRANTS}"
        )))
        .await
        .expect("provision the runtime login");
    drop(client);
    let mut runtime_url = parsed.clone();
    runtime_url.set_username(RUNTIME_LOGIN).unwrap();
    runtime_url.set_password(None).unwrap();
    let runtime = PgBackend::new(&config(runtime_url.as_str()))
        .await
        .expect("runtime backend");
    let row = runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert!(!row.get::<_, bool>(0) && !row.get::<_, bool>(1));
    // Both rails price at version v1, one to one, the same on every run and
    // in the schema suite.
    for rail in ["near_ai", "test_keyed"] {
        let recorded = runtime
            .record_payout_rate_epoch(&rail_id(rail), &RailRate::new("v1", 1, 1).unwrap(), None)
            .await
            .unwrap();
        assert_ne!(recorded, PayoutRateEpochRecord::Conflict, "{rail}");
        // A test that halted a rail and then failed must not stop the next.
        if runtime.payout_control(&rail_id(rail)).await.unwrap().halted {
            runtime
                .set_payout_control(&rail_id(rail), false, Some("test_reset"), ACTOR)
                .await
                .unwrap();
        }
    }
    Some(Fixture {
        owner,
        runtime: Arc::new(runtime),
    })
}

fn sha(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

/// Runs `sql` as the owner in `tenant`'s context.
async fn owner_exec(
    fixture: &Fixture,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) {
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(sql, params).await.unwrap();
    tx.commit().await.unwrap();
}

async fn tenant_with_accounts(fixture: &Fixture, accounts: usize) -> (String, Vec<Uuid>) {
    let tenant = format!("payout-pass-{}", Uuid::new_v4());
    let ids: Vec<Uuid> = (0..accounts).map(|_| Uuid::new_v4()).collect();
    owner_exec(
        fixture,
        &tenant,
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
        &[&tenant],
    )
    .await;
    for id in &ids {
        owner_exec(
            fixture,
            &tenant,
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, id],
        )
        .await;
    }
    (tenant, ids)
}

fn account_ref(account: Uuid) -> String {
    format!("account:{account}")
}

fn item(
    credit_account_ref: &str,
    micros: i64,
    events: &[Uuid],
) -> TraceCreditAccountSettlementLineItem {
    TraceCreditAccountSettlementLineItem {
        credit_account_ref: credit_account_ref.to_owned(),
        credit_account_hash: sha(&format!("credit-account:{credit_account_ref}")),
        settled_credit_delta_micros: micros,
        source_credit_event_ids: events.to_vec(),
        source_submission_ids: Vec::new(),
        source_list_hash: sha(&format!(
            "items:{credit_account_ref}:{events:?}:{}",
            Uuid::new_v4()
        )),
        near_status: TraceCreditSettlementNearStatus::Disabled,
        near_outbox_id: None,
        near_payout_hold_reason: None,
    }
}

/// A finalized batch carrying `lines`, dated just before now.
async fn settle_lines(
    fixture: &Fixture,
    tenant: &str,
    lines: &[TraceCreditAccountSettlementLineItem],
) {
    let batch = Uuid::new_v4();
    let events: Vec<Uuid> = lines
        .iter()
        .flat_map(|line| line.source_credit_event_ids.iter().copied())
        .collect();
    let created_at = Utc::now() - Duration::seconds(2);
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_credit_settlement_batches (
             tenant_id, settlement_batch_id, policy_version, status, reason_hash,
             source_credit_event_ids, source_list_hash, settled_credit_points,
             settled_credit_micros, line_items_json, actor_principal_ref, created_at
         ) VALUES ($1, $2, 'test', 'finalized', $3, $4, $5, '1', 1000000, $6, 'operator', $7)",
        &[
            &tenant,
            &batch,
            &sha(&format!("reason:{batch}")),
            &events,
            &sha(&format!("list:{batch}")),
            &serde_json::to_value(lines).unwrap(),
            &created_at,
        ],
    )
    .await;
}

/// Settles `micros` of credit to `account` (one to one into rail units).
async fn settle(fixture: &Fixture, tenant: &str, account: Uuid, micros: i64) {
    settle_lines(fixture, tenant, &[item(&account_ref(account), micros, &[])]).await;
}

/// Opens the links its own test made, and nothing else.
#[derive(Default)]
struct TestOpener(Mutex<HashMap<Uuid, String>>);

impl PayoutRecipientOpener for TestOpener {
    fn open(&self, link: &PayoutLink) -> Option<RecipientRef> {
        let known = self.0.lock().unwrap().get(&link.link_id).cloned()?;
        RecipientRef::new(known).ok()
    }
}

fn sealed(seed: &str) -> SealedRecipient {
    SealedRecipient {
        sealed: serde_json::json!({"v": 1, "ciphertext": sha(&format!("ct:{seed}"))}),
        recipient_index: sha(&format!("index:{seed}")),
        recipient_key_ref: "key-test-1".into(),
        index_pepper_ref: "pepper-test-1".into(),
        subject_index: sha(&format!("subject:{seed}")),
        recipient_ref: None,
    }
}

/// Inserts a link for `account` on `rail` to `recipient`, known to `opener`.
async fn link(
    fixture: &Fixture,
    tenant: &str,
    account: Uuid,
    rail: &dyn PayoutRail,
    opener: &TestOpener,
    recipient: &str,
    state: PayoutLinkState,
) -> PayoutLinkWrite {
    let link_id = Uuid::new_v4();
    let write = PayoutLinkWrite {
        account_id: account,
        rail_id: rail.descriptor().rail_id.clone(),
        link_id,
        recipient_kind: RecipientKind::new("test_recipient").unwrap(),
        recipient: sealed(recipient),
        program: rail.program_for_link(link_id),
        state,
        actor_ref_hash: ACTOR.into(),
    };
    assert_eq!(
        fixture
            .runtime
            .insert_payout_link(tenant, &write)
            .await
            .unwrap(),
        PayoutLinkInsert::Inserted
    );
    opener
        .0
        .lock()
        .unwrap()
        .insert(link_id, recipient.to_owned());
    write
}

fn settings(mode: PayoutMode) -> PayoutPassSettings {
    PayoutPassSettings {
        mode,
        account_ceiling: AtomicUnits::from_raw(CEILING),
        local_send_limit: 50,
        rate_in_force: true,
    }
}

fn pass(
    fixture: &Fixture,
    rail: Arc<dyn PayoutRail>,
    opener: &Arc<TestOpener>,
    settings: PayoutPassSettings,
) -> PayoutPass {
    PayoutPass::new(
        rail,
        fixture.runtime.clone() as Arc<dyn PayoutStore>,
        fixture.runtime.clone() as Arc<dyn Database>,
        opener.clone() as Arc<dyn PayoutRecipientOpener>,
        settings,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineRow {
    link_id: Uuid,
    state: String,
    target: String,
    delta: String,
    receipt: Option<String>,
    attempts: i32,
    label: Option<String>,
}

async fn lines(fixture: &Fixture, tenant: &str, rail: &str) -> Vec<LineRow> {
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let rows = tx
        .query(
            "SELECT link_id, state, target_total::TEXT, delta::TEXT, receipt_hash, attempts,
                    last_error_label
               FROM trace_payout_lines
              WHERE tenant_id = $1 AND rail_id = $2
              ORDER BY created_at, line_id",
            &[&tenant, &rail],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    rows.iter()
        .map(|row| LineRow {
            link_id: row.get(0),
            state: row.get(1),
            target: row.get(2),
            delta: row.get(3),
            receipt: row.get(4),
            attempts: row.get(5),
            label: row.get(6),
        })
        .collect()
}

/// Every payout row of `tenant` on `rail`, as text, for "changed nothing".
async fn snapshot(fixture: &Fixture, tenant: &str, rail: &str) -> Vec<String> {
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    for table in [
        "trace_payout_links",
        "trace_payout_lines",
        "trace_payout_holds",
        "trace_payout_audit",
    ] {
        let rows = tx
            .query(
                &format!(
                    "SELECT row_to_json(t)::TEXT FROM {table} t
                      WHERE tenant_id = $1 AND rail_id = $2 ORDER BY 1"
                ),
                &[&tenant, &rail],
            )
            .await
            .unwrap();
        out.extend(rows.iter().map(|row| row.get::<_, String>(0)));
    }
    tx.commit().await.unwrap();
    out
}

async fn link_state(fixture: &Fixture, tenant: &str, link_id: Uuid) -> (String, Option<String>) {
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let row = tx
        .query_one(
            "SELECT state, final_paid::TEXT FROM trace_payout_links
              WHERE tenant_id = $1 AND link_id = $2",
            &[&tenant, &link_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (row.get(0), row.get(1))
}

async fn holds(fixture: &Fixture, tenant: &str, account: Uuid, rail: &str) -> Vec<String> {
    let mut labels = fixture
        .runtime
        .open_payout_hold_labels(tenant, account, &rail_id(rail))
        .await
        .unwrap();
    labels.sort();
    labels
}

fn pending_line(link: &PayoutLinkWrite, target: u128, delta: u128) -> PayoutLineWrite {
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

fn org(n: u128) -> String {
    recipient_org(n).to_string()
}

fn completed(report: &PayoutPassReport) {
    assert_eq!(report.outcome, PayoutPassOutcome::Completed, "{report:?}");
}

#[tokio::test]
async fn a_linked_account_is_paid_owed_and_an_unlinked_one_accrues_only() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    settle(&fixture, &tenant, accounts[1], 2_000).await;
    let linked = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;

    let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    completed(&report);

    let transfers = mock.transfers();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].recipient, recipient_org(1));
    assert_eq!(transfers[0].program, linked.program);
    assert_eq!(
        (transfers[0].amount, transfers[0].cumulative),
        (3_000, 3_000)
    );
    let rows = lines(&fixture, &tenant, "near_ai").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "confirmed");
    assert_eq!(rows[0].target, "3000");
    assert_eq!(
        rows[0].receipt.as_deref(),
        Some(near_ai_receipt_hash(transfers[0].id).as_str())
    );
    // The unlinked account accrued and is owed, with nothing sent.
    let owed = fixture
        .runtime
        .payout_owed(&tenant, accounts[1], &rail_id("near_ai"))
        .await
        .unwrap();
    assert_eq!(owed.net(), Some(2_000));

    // A second pass sends nothing more.
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1);
    assert_eq!(lines(&fixture, &tenant, "near_ai").await.len(), 1);
}

/// A rail whose rate source is gone pays nothing from stored epochs: the
/// fixture's epoch `v1` is still recorded, the account is owed and linked,
/// and the pass accrues nothing, calls nothing and writes nothing, in
/// `live` and in `dry_run` alike.
#[tokio::test]
async fn a_stored_epoch_without_a_current_rate_pays_nothing() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    let before = snapshot(&fixture, &tenant, "near_ai").await;
    for mode in [PayoutMode::Live, PayoutMode::DryRun] {
        let report = pass(
            &fixture,
            rail.clone(),
            &opener,
            PayoutPassSettings {
                rate_in_force: false,
                ..settings(mode)
            },
        )
        .run_once()
        .await;
        assert_eq!(
            report.outcome,
            PayoutPassOutcome::Stopped {
                label: "payout_rate_unavailable"
            },
            "{report:?}"
        );
        assert_eq!(
            (report.accrued, report.sent, report.dry_run_accounts),
            (0, 0, 0)
        );
    }
    assert!(mock.requests().is_empty(), "no call to the rail at all");
    assert_eq!(
        fixture
            .runtime
            .payout_owed(&tenant, accounts[0], &rail_id("near_ai"))
            .await
            .unwrap()
            .accrual_count,
        0,
        "nothing accrued from the stored epoch"
    );
    assert_eq!(snapshot(&fixture, &tenant, "near_ai").await, before);
}

#[tokio::test]
async fn crash_after_pending_sends_once_on_near_ai() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    settle(&fixture, &tenant, accounts[1], 4_000).await;
    // A: the pass crashed after writing the line and before sending it.
    let never_sent = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    // B: the send committed and the response was lost.
    let lost = link(
        &fixture,
        &tenant,
        accounts[1],
        rail.as_ref(),
        &opener,
        &org(2),
        PayoutLinkState::Active,
    )
    .await;
    for (link, target) in [(&never_sent, 3_000), (&lost, 4_000)] {
        assert_eq!(
            fixture
                .runtime
                .insert_pending_payout_line(&tenant, &pending_line(link, target, target))
                .await
                .unwrap(),
            PendingPayoutLineInsert::Inserted
        );
    }
    let seeded = mock.seed(recipient_org(2), &lost.program, 4_000);

    completed(
        &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );

    let transfers = mock.transfers();
    assert_eq!(transfers.len(), 2, "one new 201, for A only");
    let to_a: Vec<_> = transfers
        .iter()
        .filter(|t| t.recipient == recipient_org(1))
        .collect();
    assert_eq!(to_a.len(), 1);
    assert_eq!(to_a[0].amount, 3_000);
    let rows = lines(&fixture, &tenant, "near_ai").await;
    assert_eq!(rows.len(), 2);
    let row = |link: &PayoutLinkWrite| {
        rows.iter()
            .find(|r| r.link_id == link.link_id)
            .unwrap()
            .clone()
    };
    assert_eq!(row(&never_sent).state, "confirmed");
    assert_eq!(
        row(&never_sent).receipt.as_deref(),
        Some(near_ai_receipt_hash(to_a[0].id).as_str())
    );
    assert_eq!(row(&lost).state, "confirmed");
    assert_eq!(
        row(&lost).receipt.as_deref(),
        Some(near_ai_receipt_hash(seeded).as_str())
    );
    assert_eq!(row(&lost).delta, "4000");

    // Nothing more on a rerun.
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 2);
}

#[tokio::test]
async fn crash_after_pending_sends_once_on_keyed_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let rail: Arc<dyn PayoutRail> = keyed.clone();
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 5_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        "keyed-recipient-1",
        PayoutLinkState::Active,
    )
    .await;

    // The send commits and its response is lost.
    keyed.lose_next_response();
    completed(
        &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(keyed.effects(), 1);
    let rows = lines(&fixture, &tenant, "test_keyed").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "pending");
    assert_eq!(rows[0].attempts, 1);
    assert_eq!(rows[0].label.as_deref(), Some("lost_response"));

    // The next pass resends the same line: one effect, the first receipt.
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(keyed.effects(), 1);
    let rows = lines(&fixture, &tenant, "test_keyed").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "confirmed");
    assert_eq!(rows[0].delta, "5000");
    assert!(rows[0].receipt.is_some());
}

#[tokio::test]
async fn recovery_without_a_matching_receipt_holds_the_line() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    let linked = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    fixture
        .runtime
        .insert_pending_payout_line(&tenant, &pending_line(&linked, 3_000, 3_000))
        .await
        .unwrap();
    // Something outside this line put 5000 on the program: the resend
    // answers no_op, and no record lands exactly on the line's target.
    mock.seed(recipient_org(1), &linked.program, 5_000);

    completed(
        &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1, "nothing sent");
    let rows = lines(&fixture, &tenant, "near_ai").await;
    assert_eq!(rows[0].state, "held");
    assert_eq!(rows[0].label.as_deref(), Some("payout_recovery_unmatched"));
    assert_eq!(
        holds(&fixture, &tenant, accounts[0], "near_ai").await,
        vec!["payout_recovery_unmatched".to_string()]
    );

    // A hold about this account stays: a later pass sends nothing to it.
    completed(
        &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1);
    assert_eq!(lines(&fixture, &tenant, "near_ai").await.len(), 1);
    assert_eq!(
        holds(&fixture, &tenant, accounts[0], "near_ai").await,
        vec!["payout_recovery_unmatched".to_string()]
    );
    // Until an operator clears it: the next pass looks at the account
    // afresh, finds the outside 5000 on the program, and holds it for that
    // instead, still sending nothing.
    assert!(
        fixture
            .runtime
            .clear_payout_hold(
                &tenant,
                accounts[0],
                &rail_id("near_ai"),
                "payout_recovery_unmatched",
                ACTOR
            )
            .await
            .unwrap()
    );
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1);
    assert_eq!(
        holds(&fixture, &tenant, accounts[0], "near_ai").await,
        vec!["payout_program_total_mismatch".to_string()]
    );
}

#[tokio::test]
async fn a_stopped_rail_does_not_block_another() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let near: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        near.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        keyed.as_ref(),
        &opener,
        "keyed-recipient-2",
        PayoutLinkState::Active,
    )
    .await;
    // "Unchanged" compares this tenant's link, line, hold and audit rows.
    let before = snapshot(&fixture, &tenant, "near_ai").await;
    mock.script(Scripted::Error(403, "credential_refused"));

    let near_pass = pass(&fixture, near.clone(), &opener, settings(PayoutMode::Live));
    let keyed_pass = pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live));
    let (near_report, keyed_report) = tokio::join!(near_pass.run_once(), keyed_pass.run_once());

    assert_eq!(
        near_report.outcome,
        PayoutPassOutcome::Stopped {
            label: "near_ai_credential_refused"
        }
    );
    completed(&keyed_report);
    assert_eq!(keyed.effects(), 1);
    let keyed_rows = lines(&fixture, &tenant, "test_keyed").await;
    assert_eq!(keyed_rows.len(), 1);
    assert_eq!(keyed_rows[0].state, "confirmed");
    assert_eq!(snapshot(&fixture, &tenant, "near_ai").await, before);
    assert!(mock.transfers().is_empty());
}

#[tokio::test]
async fn a_halted_rail_does_not_block_another() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let near: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        near.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        keyed.as_ref(),
        &opener,
        "keyed-recipient-3",
        PayoutLinkState::Active,
    )
    .await;
    fixture
        .runtime
        .set_payout_control(&rail_id("near_ai"), true, Some("operator_halt"), ACTOR)
        .await
        .unwrap();

    let near_report = pass(&fixture, near, &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    let keyed_report = pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    fixture
        .runtime
        .set_payout_control(&rail_id("near_ai"), false, Some("operator_resume"), ACTOR)
        .await
        .unwrap();

    assert_eq!(near_report.outcome, PayoutPassOutcome::Halted);
    assert!(mock.requests().is_empty(), "a halted rail calls nothing");
    assert!(lines(&fixture, &tenant, "near_ai").await.is_empty());
    completed(&keyed_report);
    assert_eq!(keyed.effects(), 1);
}

#[tokio::test]
async fn a_credential_refusal_stops_the_rail_and_changes_no_row() {
    let Some(fixture) = fixture().await else {
        return;
    };
    // On the read: capacity answers, then the zero read is refused.
    for status in [401, 403] {
        let mock = Mock::start().await;
        let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
        let opener = Arc::new(TestOpener::default());
        let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
        settle(&fixture, &tenant, accounts[0], 3_000).await;
        link(
            &fixture,
            &tenant,
            accounts[0],
            rail.as_ref(),
            &opener,
            &org(1),
            PayoutLinkState::Active,
        )
        .await;
        let before = snapshot(&fixture, &tenant, "near_ai").await;
        mock.script(Scripted::Pass);
        mock.script(Scripted::Error(status, "credential_refused"));
        let report = pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await;
        assert_eq!(
            report.outcome,
            PayoutPassOutcome::Stopped {
                label: "near_ai_credential_refused"
            },
            "{status}"
        );
        assert_eq!(
            snapshot(&fixture, &tenant, "near_ai").await,
            before,
            "{status}"
        );
        assert!(
            holds(&fixture, &tenant, accounts[0], "near_ai")
                .await
                .is_empty()
        );
    }

    // On the send: the line was written pending before the send and stays
    // exactly as written; no hold, no mark, no audit row.
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    let linked = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    mock.script(Scripted::Pass); // capacity
    mock.script(Scripted::Pass); // the zero read
    mock.script(Scripted::Error(403, "credential_refused")); // the send
    let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    assert_eq!(
        report.outcome,
        PayoutPassOutcome::Stopped {
            label: "near_ai_credential_refused"
        }
    );
    let rows = lines(&fixture, &tenant, "near_ai").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        LineRow {
            link_id: linked.link_id,
            state: "pending".into(),
            target: "3000".into(),
            delta: "3000".into(),
            receipt: None,
            attempts: 0,
            label: None,
        }
    );
    assert!(
        holds(&fixture, &tenant, accounts[0], "near_ai")
            .await
            .is_empty()
    );
    assert_eq!(
        link_state(&fixture, &tenant, linked.link_id).await.0,
        "active"
    );

    // With the credential fixed, the same line lands once.
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1);
    assert_eq!(
        lines(&fixture, &tenant, "near_ai").await[0].state,
        "confirmed"
    );
}

/// A limit on our own credential or org refuses a send: that is the
/// sender's budget, not the recipient's standing. The pass stops, holds no
/// account, and leaves the line pending exactly as written; the next pass
/// resends it and pays everyone.
#[tokio::test]
async fn a_sender_limit_stops_the_rail_and_holds_no_recipient() {
    let Some(fixture) = fixture().await else {
        return;
    };
    for code in [
        "credential_transfer_limit_exceeded",
        "organization_transfer_limit_exceeded",
    ] {
        let mock = Mock::start().await;
        let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
        let opener = Arc::new(TestOpener::default());
        let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
        for (n, account) in accounts.iter().enumerate() {
            settle(&fixture, &tenant, *account, 1_000).await;
            link(
                &fixture,
                &tenant,
                *account,
                rail.as_ref(),
                &opener,
                &org(10 + n as u128),
                PayoutLinkState::Active,
            )
            .await;
        }
        // Capacity, the first zero read, then the first send is refused.
        mock.script(Scripted::Pass);
        mock.script(Scripted::Pass);
        mock.script(Scripted::Error(409, code));
        let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await;
        assert!(
            matches!(report.outcome, PayoutPassOutcome::Stopped { .. }),
            "{code} {report:?}"
        );
        assert!(
            !report
                .held
                .contains_key("near_ai_credential_transfer_limit_exceeded")
                && !report
                    .held
                    .contains_key("near_ai_organization_transfer_limit_exceeded"),
            "{code} {report:?}"
        );
        for account in &accounts {
            assert!(
                holds(&fixture, &tenant, *account, "near_ai")
                    .await
                    .is_empty(),
                "{code}"
            );
        }
        let rows = lines(&fixture, &tenant, "near_ai").await;
        assert_eq!(rows.len(), 1, "{code}");
        assert_eq!(rows[0].state, "pending", "{code}");
        assert!(mock.transfers().is_empty());

        completed(
            &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
                .run_once()
                .await,
        );
        assert_eq!(mock.transfers().len(), 2, "{code}");
        let rows = lines(&fixture, &tenant, "near_ai").await;
        assert!(
            rows.iter().all(|row| row.state == "confirmed"),
            "{code} {rows:?}"
        );
    }
}

#[tokio::test]
async fn back_pay_steps_up_by_the_per_send_max() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    mock.set_per_send_max(1_000);
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 2_500).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;

    for _ in 0..4 {
        completed(
            &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
                .run_once()
                .await,
        );
    }
    let amounts: Vec<i64> = mock.transfers().iter().map(|t| t.amount).collect();
    assert_eq!(amounts, vec![1_000, 1_000, 500]);
    let targets: Vec<String> = lines(&fixture, &tenant, "near_ai")
        .await
        .into_iter()
        .map(|row| row.target)
        .collect();
    assert_eq!(targets, vec!["1000", "2000", "2500"]);
}

#[tokio::test]
async fn a_target_over_the_account_ceiling_holds_and_sends_nothing() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    let low = PayoutPassSettings {
        account_ceiling: AtomicUnits::from_raw(2_999),
        ..settings(PayoutMode::Live)
    };

    completed(&pass(&fixture, rail.clone(), &opener, low).run_once().await);
    assert!(mock.transfers().is_empty());
    assert_eq!(mock.posts(), 0, "no read or send for a held account");
    assert!(lines(&fixture, &tenant, "near_ai").await.is_empty());
    assert_eq!(
        holds(&fixture, &tenant, accounts[0], "near_ai").await,
        vec!["payout_over_ceiling".to_string()]
    );
    // A second pass under the same ceiling still sends nothing.
    completed(&pass(&fixture, rail.clone(), &opener, low).run_once().await);
    assert!(mock.transfers().is_empty());
    // Spec section 4, "Ceilings": an operator lifts it by raising the
    // setting. The next pass clears the hold, with its audit row, and pays.
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert!(
        holds(&fixture, &tenant, accounts[0], "near_ai")
            .await
            .is_empty()
    );
    assert_eq!(
        mock.transfers()
            .iter()
            .map(|t| t.amount)
            .collect::<Vec<_>>(),
        vec![3_000]
    );
    assert!(
        fixture
            .runtime
            .payout_audit_actions(&tenant, &rail_id("near_ai"))
            .await
            .unwrap()
            .contains(&trace_commons_server::db::PayoutAuditAction::HoldCleared)
    );
}

#[tokio::test]
async fn a_live_rail_with_no_linked_tenant_calls_nothing() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    // A rail that answers any call with a stop: the pass must not reach it.
    keyed.refuse_next(RailRefusal::StopPass("test_should_not_be_called"));
    let report = pass(
        &fixture,
        Arc::new(NoTenantRail(keyed.clone())),
        &Arc::new(TestOpener::default()),
        settings(PayoutMode::Live),
    )
    .run_once()
    .await;
    completed(&report);
    assert_eq!(report.tenants, 0);
}

/// The keyed rail under an id no link in the database uses.
struct NoTenantRail(Arc<MockKeyedPayoutRail>);

static NO_TENANT_DESCRIPTOR: std::sync::OnceLock<RailDescriptor> = std::sync::OnceLock::new();

#[async_trait]
impl PayoutRail for NoTenantRail {
    fn descriptor(&self) -> &RailDescriptor {
        NO_TENANT_DESCRIPTOR.get_or_init(|| RailDescriptor {
            rail_id: rail_id("test_no_tenant"),
            ..self.0.descriptor().clone()
        })
    }
    fn adapter_identity(&self) -> &str {
        self.0.adapter_identity()
    }
    fn program_for_link(&self, link_id: Uuid) -> String {
        self.0.program_for_link(link_id)
    }
    async fn preflight(&self) -> Result<RailCapacity, RailRefusal> {
        self.0.preflight().await
    }
    async fn read_total(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<AtomicUnits, RailRefusal> {
        self.0.read_total(recipient, program).await
    }
    async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal> {
        self.0.send(line).await
    }
    async fn find_receipt(
        &self,
        line: &RailLine<'_>,
        exclude: &[String],
    ) -> Result<ReceiptLookup, RailRefusal> {
        self.0.find_receipt(line, exclude).await
    }
    async fn history(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<Vec<RailHistoryEntry>, RailRefusal> {
        self.0.history(recipient, program).await
    }
    async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal> {
        self.0.probe_recipient(recipient).await
    }
}

#[tokio::test]
async fn a_spent_send_window_writes_no_line_and_waits() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    // #1271 reports every source at 0 when the minute's sends are spent.
    mock.set_sends_remaining(0);
    let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    completed(&report);
    assert!(
        report.skipped.contains_key("payout_send_window_exhausted"),
        "{report:?}"
    );
    assert_eq!(mock.posts(), 0);
    assert!(lines(&fixture, &tenant, "near_ai").await.is_empty());
    assert!(
        holds(&fixture, &tenant, accounts[0], "near_ai")
            .await
            .is_empty()
    );

    mock.set_sends_remaining(60);
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1);
}

/// Every write call spends the rail's send window, not just a send: on
/// `near_ai` the zero read before a new line and the read that closes a
/// link are POSTs too. A pass never makes more of them than the window it
/// was given, and the next pass picks up the rest.
#[tokio::test]
async fn reads_and_sends_together_stay_inside_the_send_window() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 3).await;
    for (n, account) in accounts.iter().enumerate() {
        settle(&fixture, &tenant, *account, 1_000).await;
        let made = link(
            &fixture,
            &tenant,
            *account,
            rail.as_ref(),
            &opener,
            &org(20 + n as u128),
            PayoutLinkState::Active,
        )
        .await;
        if n == 2 {
            // Paid in full already, and unlinking: closing it is one read.
            mock.seed(recipient_org(22), &made.program, 1_000);
            fixture
                .runtime
                .mark_payout_link_unlink_requested(&tenant, made.link_id, ACTOR)
                .await
                .unwrap();
        }
    }
    // Two new lines (a read and a send each) and one close (a read) want
    // five write calls; the window has three.
    mock.set_sends_remaining(3);
    let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    completed(&report);
    assert!(
        mock.posts() <= 3,
        "{} write calls: {report:?}",
        mock.posts()
    );
    assert!(
        report.skipped.contains_key("payout_send_window_exhausted"),
        "{report:?}"
    );
    assert!(mock.posts() >= 2, "the window was used: {report:?}");

    // The next passes, each with a fresh window, finish the work.
    for _ in 0..3 {
        mock.set_sends_remaining(3);
        let before = mock.posts();
        completed(
            &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
                .run_once()
                .await,
        );
        assert!(mock.posts() - before <= 3);
    }
    // Two sends, beside the seeded record on the closed link.
    assert_eq!(mock.transfers().len(), 3);
    let rows = lines(&fixture, &tenant, "near_ai").await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.state == "confirmed"), "{rows:?}");
}

#[tokio::test]
async fn a_credit_hold_or_closed_account_stops_sends_on_every_rail() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let near: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 3).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    owner_exec(
        &fixture,
        &tenant,
        "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
         VALUES ($1, $2, $3)",
        &[&tenant, &accounts[2], &principal],
    )
    .await;
    for (n, account) in accounts.iter().enumerate() {
        settle(&fixture, &tenant, *account, 1_000).await;
        link(
            &fixture,
            &tenant,
            *account,
            near.as_ref(),
            &opener,
            &org(10 + n as u128),
            PayoutLinkState::Active,
        )
        .await;
        link(
            &fixture,
            &tenant,
            *account,
            keyed.as_ref(),
            &opener,
            &format!("keyed-held-{n}"),
            PayoutLinkState::Active,
        )
        .await;
    }
    // 0: a credit hold on its account key; 1: closed; 2: a credit hold on a
    // principal linked to it.
    for credit_ref in [account_ref(accounts[0]), principal.clone()] {
        owner_exec(
            &fixture,
            &tenant,
            "INSERT INTO trace_credit_holds (
                 tenant_id, hold_id, credit_account_ref, credit_account_hash, reason,
                 reason_hash, actor_principal_ref
             ) VALUES ($1, $2, $3, $4, 'fixture', $5, 'operator')",
            &[
                &tenant,
                &Uuid::new_v4(),
                &credit_ref,
                &sha(&credit_ref),
                &sha("fixture"),
            ],
        )
        .await;
    }
    owner_exec(
        &fixture,
        &tenant,
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&tenant, &accounts[1]],
    )
    .await;

    for rail in [near.clone(), keyed.clone() as Arc<dyn PayoutRail>] {
        let report = pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await;
        completed(&report);
        // Other tests' tenants are enumerated too, so these are floors.
        assert!(
            report.skipped.get("payout_account_credit_hold") >= Some(&2),
            "{report:?}"
        );
        assert!(
            report.skipped.get("payout_account_closed") >= Some(&1),
            "{report:?}"
        );
    }
    // A shared standing is checked, not recorded: no rail hold is written.
    for account in &accounts {
        assert!(
            holds(&fixture, &tenant, *account, "near_ai")
                .await
                .is_empty()
        );
        assert!(
            holds(&fixture, &tenant, *account, "test_keyed")
                .await
                .is_empty()
        );
    }
    assert!(mock.transfers().is_empty());
    assert_eq!(mock.posts(), 0);
    assert_eq!(keyed.effects(), 0);
    assert!(lines(&fixture, &tenant, "near_ai").await.is_empty());
    assert!(lines(&fixture, &tenant, "test_keyed").await.is_empty());
}

#[tokio::test]
async fn a_link_with_a_pending_line_gets_no_new_line() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    let live = || pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live));

    // capacity, zero read, then the send answers 503: the line stays pending.
    mock.script(Scripted::Pass);
    mock.script(Scripted::Pass);
    mock.script(Scripted::Error(503, "transfer_unavailable"));
    completed(&live().run_once().await);
    settle(&fixture, &tenant, accounts[0], 500).await;
    // capacity, then the resend of the same line answers 503 again.
    mock.script(Scripted::Pass);
    mock.script(Scripted::Error(503, "transfer_unavailable"));
    completed(&live().run_once().await);
    let rows = lines(&fixture, &tenant, "near_ai").await;
    assert_eq!(rows.len(), 1, "no second line beside a pending one");
    assert_eq!((rows[0].state.as_str(), rows[0].attempts), ("pending", 2));
    assert_eq!(rows[0].target, "1000");

    // The pending line lands first, at its own target; the rest follows.
    completed(&live().run_once().await);
    assert_eq!(
        mock.transfers()
            .iter()
            .map(|t| t.amount)
            .collect::<Vec<_>>(),
        vec![1_000]
    );
    completed(&live().run_once().await);
    assert_eq!(
        mock.transfers()
            .iter()
            .map(|t| t.amount)
            .collect::<Vec<_>>(),
        vec![1_000, 500]
    );
    let states: Vec<String> = lines(&fixture, &tenant, "near_ai")
        .await
        .into_iter()
        .map(|row| row.state)
        .collect();
    assert_eq!(states, vec!["confirmed", "confirmed"]);
}

#[tokio::test]
async fn closing_resolves_pending_lines_by_reads_only_then_activates_the_successor() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    settle(&fixture, &tenant, accounts[1], 9_000).await;
    // Account 0: its pending line landed before a crash. Account 1: its
    // pending line never went out.
    let old_a = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    let old_b = link(
        &fixture,
        &tenant,
        accounts[1],
        rail.as_ref(),
        &opener,
        &org(3),
        PayoutLinkState::Active,
    )
    .await;
    fixture
        .runtime
        .insert_pending_payout_line(&tenant, &pending_line(&old_a, 3_000, 3_000))
        .await
        .unwrap();
    fixture
        .runtime
        .insert_pending_payout_line(&tenant, &pending_line(&old_b, 9_000, 9_000))
        .await
        .unwrap();
    let landed = mock.seed(recipient_org(1), &old_a.program, 3_000);
    let new_a = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(2),
        PayoutLinkState::Waiting,
    )
    .await;
    let new_b = link(
        &fixture,
        &tenant,
        accounts[1],
        rail.as_ref(),
        &opener,
        &org(4),
        PayoutLinkState::Waiting,
    )
    .await;

    let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    completed(&report);

    // Nothing new reached either old org: closing reads, never sends.
    assert!(
        mock.transfers()
            .iter()
            .all(|t| t.recipient != recipient_org(1) || t.id == landed)
    );
    assert!(
        mock.transfers()
            .iter()
            .all(|t| t.recipient != recipient_org(3))
    );
    let rows = lines(&fixture, &tenant, "near_ai").await;
    let row = |link: &PayoutLinkWrite| {
        rows.iter()
            .find(|r| r.link_id == link.link_id)
            .unwrap()
            .clone()
    };
    assert_eq!(row(&old_a).state, "confirmed");
    assert_eq!(
        row(&old_a).receipt.as_deref(),
        Some(near_ai_receipt_hash(landed).as_str())
    );
    assert_eq!(row(&old_b).state, "no_op");
    assert_eq!(
        link_state(&fixture, &tenant, old_a.link_id).await,
        ("closed".into(), Some("3000".into()))
    );
    assert_eq!(
        link_state(&fixture, &tenant, old_b.link_id).await,
        ("closed".into(), Some("0".into()))
    );
    assert_eq!(
        link_state(&fixture, &tenant, new_a.link_id).await.0,
        "active"
    );
    assert_eq!(
        link_state(&fixture, &tenant, new_b.link_id).await.0,
        "active"
    );
    assert_eq!(report.closed, 2);

    // The successors are paid what the old programs did not get.
    completed(
        &pass(&fixture, rail, &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    let to = |n: u128| -> i64 {
        mock.transfers()
            .iter()
            .filter(|t| t.recipient == recipient_org(n))
            .map(|t| t.amount)
            .sum()
    };
    assert_eq!((to(1), to(2), to(3), to(4)), (3_000, 0, 0, 9_000));
}

#[tokio::test]
async fn closing_after_a_404_completes_from_history() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 2_000).await;
    let old = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    completed(
        &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(mock.transfers().len(), 1);

    // The old org is gone; the account unlinks.
    mock.deactivate(recipient_org(1));
    assert!(
        fixture
            .runtime
            .mark_payout_link_unlink_requested(&tenant, old.link_id, ACTOR)
            .await
            .unwrap()
    );
    completed(
        &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(
        link_state(&fixture, &tenant, old.link_id).await,
        ("closed".into(), Some("2000".into()))
    );

    // Credit keeps accruing; a relink is paid only what the old program
    // did not get.
    settle(&fixture, &tenant, accounts[0], 700).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(2),
        PayoutLinkState::Waiting,
    )
    .await;
    for _ in 0..2 {
        completed(
            &pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
                .run_once()
                .await,
        );
    }
    let to_new: i64 = mock
        .transfers()
        .iter()
        .filter(|t| t.recipient == recipient_org(2))
        .map(|t| t.amount)
        .sum();
    assert_eq!(to_new, 700);
}

#[tokio::test]
async fn relinking_pays_owed_once_across_programs() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    let live = || pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live));

    settle(&fixture, &tenant, accounts[0], 1_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Waiting,
    )
    .await;
    completed(&live().run_once().await);
    // Change to org 2, then back to org 1 under a new program.
    settle(&fixture, &tenant, accounts[0], 500).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(2),
        PayoutLinkState::Waiting,
    )
    .await;
    completed(&live().run_once().await);
    settle(&fixture, &tenant, accounts[0], 250).await;
    let back = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Waiting,
    )
    .await;
    completed(&live().run_once().await);
    completed(&live().run_once().await);

    let total: i64 = mock.transfers().iter().map(|t| t.amount).sum();
    assert_eq!(total, 1_750, "everything owed, once");
    let programs: std::collections::BTreeSet<String> =
        mock.transfers().iter().map(|t| t.program.clone()).collect();
    assert_eq!(programs.len(), 3, "one program per link generation");
    assert_eq!(
        link_state(&fixture, &tenant, back.link_id).await.0,
        "active"
    );
    let on_back: i64 = mock
        .transfers()
        .iter()
        .filter(|t| t.program == back.program)
        .map(|t| t.amount)
        .sum();
    assert_eq!(on_back, 250);
}

#[tokio::test]
async fn netting_after_a_reversal_sends_nothing_until_new_credit_exceeds_the_gap() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    let principal = format!("principal-{}", Uuid::new_v4());
    owner_exec(
        &fixture,
        &tenant,
        "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
         VALUES ($1, $2, $3)",
        &[&tenant, &accounts[0], &principal],
    )
    .await;
    let credited = Uuid::new_v4();
    ledger_event(
        &fixture, &tenant, credited, &principal, "0.003", "final", None,
    )
    .await;
    settle_lines(
        &fixture,
        &tenant,
        &[item(&account_ref(accounts[0]), 3_000, &[credited])],
    )
    .await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    let live = || pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live));
    completed(&live().run_once().await);
    assert_eq!(mock.transfers().len(), 1);

    // The credit is reversed after it was paid: owed drops to 0, below the
    // 3000 the program holds. Nothing is taken back and nothing is sent.
    ledger_event(
        &fixture,
        &tenant,
        Uuid::new_v4(),
        &principal,
        "-0.003",
        "reversed",
        Some(format!("revocation_credit_reversal:{credited}")),
    )
    .await;
    completed(&live().run_once().await);
    settle(&fixture, &tenant, accounts[0], 2_000).await;
    completed(&live().run_once().await);
    assert_eq!(
        mock.transfers().len(),
        1,
        "owed 2000 is under the 3000 sent"
    );
    settle(&fixture, &tenant, accounts[0], 1_500).await;
    completed(&live().run_once().await);
    let amounts: Vec<i64> = mock.transfers().iter().map(|t| t.amount).collect();
    assert_eq!(amounts, vec![3_000, 500], "only what exceeds the gap");
}

#[tokio::test]
async fn dry_run_writes_no_line_and_makes_no_write_call() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let rail: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
    settle(&fixture, &tenant, accounts[0], 3_000).await;
    settle(&fixture, &tenant, accounts[1], 1_000).await;
    let active = link(
        &fixture,
        &tenant,
        accounts[0],
        rail.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    let waiting = link(
        &fixture,
        &tenant,
        accounts[1],
        rail.as_ref(),
        &opener,
        &org(2),
        PayoutLinkState::Waiting,
    )
    .await;
    let links_before: Vec<String> = snapshot(&fixture, &tenant, "near_ai")
        .await
        .into_iter()
        .filter(|row| row.contains("\"link_id\"") && row.contains("\"program\""))
        .collect();

    let report = pass(&fixture, rail, &opener, settings(PayoutMode::DryRun))
        .run_once()
        .await;
    completed(&report);
    assert_eq!(
        mock.posts(),
        0,
        "no write call at all, not even a zero read"
    );
    assert!(mock.transfers().is_empty());
    assert!(lines(&fixture, &tenant, "near_ai").await.is_empty());
    // Accrual runs in dry_run; links do not move.
    let owed = fixture
        .runtime
        .payout_owed(&tenant, accounts[0], &rail_id("near_ai"))
        .await
        .unwrap();
    assert_eq!(owed.net(), Some(3_000));
    assert_eq!(
        link_state(&fixture, &tenant, active.link_id).await.0,
        "active"
    );
    assert_eq!(
        link_state(&fixture, &tenant, waiting.link_id).await.0,
        "waiting"
    );
    let links_after: Vec<String> = snapshot(&fixture, &tenant, "near_ai")
        .await
        .into_iter()
        .filter(|row| row.contains("\"link_id\"") && row.contains("\"program\""))
        .collect();
    assert_eq!(links_before, links_after);
    assert!(report.dry_run_accounts >= 2, "{report:?}");
    assert!(report.dry_run_units >= 4_000, "{report:?}");
}

#[tokio::test]
async fn a_second_pass_on_one_rail_is_refused_while_the_first_holds_the_lock() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        keyed.as_ref(),
        &opener,
        "keyed-lock-1",
        PayoutLinkState::Active,
    )
    .await;

    let held = fixture
        .runtime
        .try_acquire_payout_pass_lock(&rail_id("test_keyed"))
        .await
        .unwrap()
        .expect("the lock is free");
    let report = pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    assert_eq!(report.outcome, PayoutPassOutcome::LockBusy);
    assert_eq!(keyed.effects(), 0);
    assert!(lines(&fixture, &tenant, "test_keyed").await.is_empty());
    held.release().await.unwrap();

    completed(
        &pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(keyed.effects(), 1);
    // The pass released its lock on the way out.
    let again = fixture
        .runtime
        .try_acquire_payout_pass_lock(&rail_id("test_keyed"))
        .await
        .unwrap()
        .expect("released after the pass");
    again.release().await.unwrap();
}

#[tokio::test]
async fn a_dropped_lock_is_released_with_its_connection() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rail = rail_id("test_keyed");
    let held = fixture
        .runtime
        .try_acquire_payout_pass_lock(&rail)
        .await
        .unwrap()
        .expect("free");
    assert!(
        fixture
            .runtime
            .try_acquire_payout_pass_lock(&rail)
            .await
            .unwrap()
            .is_none()
    );
    drop(held);
    // The session closes asynchronously; give it a bounded number of tries.
    let mut freed = None;
    for _ in 0..50 {
        if let Some(lock) = fixture
            .runtime
            .try_acquire_payout_pass_lock(&rail)
            .await
            .unwrap()
        {
            freed = Some(lock);
            break;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    freed
        .expect("the dropped lock's session ended")
        .release()
        .await
        .unwrap();
}

#[tokio::test]
async fn passes_on_two_rails_run_concurrently() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let mock = Mock::start().await;
    let near: Arc<dyn PayoutRail> = Arc::new(mock.rail());
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        near.as_ref(),
        &opener,
        &org(1),
        PayoutLinkState::Active,
    )
    .await;
    link(
        &fixture,
        &tenant,
        accounts[0],
        keyed.as_ref(),
        &opener,
        "keyed-concurrent-1",
        PayoutLinkState::Active,
    )
    .await;

    // One rail's lock held elsewhere does not keep the other from running.
    let held = fixture
        .runtime
        .try_acquire_payout_pass_lock(&rail_id("near_ai"))
        .await
        .unwrap()
        .expect("free");
    completed(
        &pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(keyed.effects(), 1);
    held.release().await.unwrap();

    settle(&fixture, &tenant, accounts[0], 500).await;
    let near_pass = pass(&fixture, near, &opener, settings(PayoutMode::Live));
    let keyed_pass = pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live));
    let (a, b) = tokio::join!(near_pass.run_once(), keyed_pass.run_once());
    completed(&a);
    completed(&b);
    assert_eq!(
        mock.transfers().iter().map(|t| t.amount).sum::<i64>(),
        1_500
    );
    assert_eq!(keyed.effects(), 2);
}

/// A rail that halts itself through the store after its first send, to
/// show the pass re-reads the kill switch before every send.
struct HaltAfterFirstSend {
    inner: Arc<MockKeyedPayoutRail>,
    store: Arc<PgBackend>,
    sent: AtomicBool,
}

#[async_trait]
impl PayoutRail for HaltAfterFirstSend {
    fn descriptor(&self) -> &RailDescriptor {
        self.inner.descriptor()
    }
    fn adapter_identity(&self) -> &str {
        self.inner.adapter_identity()
    }
    fn program_for_link(&self, link_id: Uuid) -> String {
        self.inner.program_for_link(link_id)
    }
    async fn preflight(&self) -> Result<RailCapacity, RailRefusal> {
        self.inner.preflight().await
    }
    async fn read_total(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<AtomicUnits, RailRefusal> {
        self.inner.read_total(recipient, program).await
    }
    async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal> {
        let outcome = self.inner.send(line).await;
        if !self.sent.swap(true, Ordering::SeqCst) {
            self.store
                .set_payout_control(&rail_id("test_keyed"), true, Some("operator_halt"), ACTOR)
                .await
                .unwrap();
        }
        outcome
    }
    async fn find_receipt(
        &self,
        line: &RailLine<'_>,
        exclude: &[String],
    ) -> Result<ReceiptLookup, RailRefusal> {
        self.inner.find_receipt(line, exclude).await
    }
    async fn history(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<Vec<RailHistoryEntry>, RailRefusal> {
        self.inner.history(recipient, program).await
    }
    async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal> {
        self.inner.probe_recipient(recipient).await
    }
}

#[tokio::test]
async fn the_kill_switch_stops_before_the_next_send() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let rail: Arc<dyn PayoutRail> = Arc::new(HaltAfterFirstSend {
        inner: keyed.clone(),
        store: fixture.runtime.clone(),
        sent: AtomicBool::new(false),
    });
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 3).await;
    for (n, account) in accounts.iter().enumerate() {
        settle(&fixture, &tenant, *account, 1_000).await;
        link(
            &fixture,
            &tenant,
            *account,
            rail.as_ref(),
            &opener,
            &format!("keyed-halt-{n}"),
            PayoutLinkState::Active,
        )
        .await;
    }

    let report = pass(&fixture, rail.clone(), &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    fixture
        .runtime
        .set_payout_control(
            &rail_id("test_keyed"),
            false,
            Some("operator_resume"),
            ACTOR,
        )
        .await
        .unwrap();
    assert_eq!(report.outcome, PayoutPassOutcome::Halted);
    assert_eq!(keyed.effects(), 1, "nothing after the halt");
    let rows = lines(&fixture, &tenant, "test_keyed").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "confirmed");
}

#[tokio::test]
async fn an_unreadable_link_holds_that_rail_only() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    let made = link(
        &fixture,
        &tenant,
        accounts[0],
        keyed.as_ref(),
        &opener,
        "keyed-unreadable",
        PayoutLinkState::Active,
    )
    .await;
    opener.0.lock().unwrap().remove(&made.link_id);

    completed(
        &pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(keyed.effects(), 0);
    assert_eq!(
        holds(&fixture, &tenant, accounts[0], "test_keyed").await,
        vec!["payout_link_unreadable".to_string()]
    );
    assert!(
        holds(&fixture, &tenant, accounts[0], "near_ai")
            .await
            .is_empty()
    );
}

/// A rail that, mid-send, ends its line by another path (as an operator or a
/// second writer might), so the send commits on a line no longer pending.
struct ResolveBeforeSend {
    inner: Arc<MockKeyedPayoutRail>,
    store: Arc<PgBackend>,
    tenant: String,
    link_id: Uuid,
}

#[async_trait]
impl PayoutRail for ResolveBeforeSend {
    fn descriptor(&self) -> &RailDescriptor {
        self.inner.descriptor()
    }
    fn adapter_identity(&self) -> &str {
        self.inner.adapter_identity()
    }
    fn program_for_link(&self, link_id: Uuid) -> String {
        self.inner.program_for_link(link_id)
    }
    async fn preflight(&self) -> Result<RailCapacity, RailRefusal> {
        self.inner.preflight().await
    }
    async fn read_total(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<AtomicUnits, RailRefusal> {
        self.inner.read_total(recipient, program).await
    }
    async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal> {
        let pending = self
            .store
            .pending_payout_line_for_link(&self.tenant, self.link_id)
            .await
            .unwrap()
            .expect("the pass wrote the line pending before the send");
        self.store
            .resolve_payout_line(&self.tenant, pending.line_id, &PayoutLineResolution::NoOp)
            .await
            .unwrap();
        self.inner.send(line).await
    }
    async fn find_receipt(
        &self,
        line: &RailLine<'_>,
        exclude: &[String],
    ) -> Result<ReceiptLookup, RailRefusal> {
        self.inner.find_receipt(line, exclude).await
    }
    async fn history(
        &self,
        recipient: &RecipientRef,
        program: &str,
    ) -> Result<Vec<RailHistoryEntry>, RailRefusal> {
        self.inner.history(recipient, program).await
    }
    async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal> {
        self.inner.probe_recipient(recipient).await
    }
}

/// A send that committed on a line no longer pending moved money no row
/// records: the pass never drops that receipt silently. It holds the
/// account for an operator, who finds the record in the rail's history.
#[tokio::test]
async fn a_committed_send_on_a_line_no_longer_pending_holds_the_account() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let keyed = Arc::new(MockKeyedPayoutRail::new());
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    let made = link(
        &fixture,
        &tenant,
        accounts[0],
        keyed.as_ref(),
        &opener,
        "keyed-not-pending",
        PayoutLinkState::Active,
    )
    .await;
    let rail: Arc<dyn PayoutRail> = Arc::new(ResolveBeforeSend {
        inner: keyed.clone(),
        store: fixture.runtime.clone(),
        tenant: tenant.clone(),
        link_id: made.link_id,
    });

    let report = pass(&fixture, rail, &opener, settings(PayoutMode::Live))
        .run_once()
        .await;
    completed(&report);
    assert_eq!(keyed.effects(), 1, "the send moved money");
    assert_eq!(report.confirmed, 0, "{report:?}");
    assert_eq!(
        holds(&fixture, &tenant, accounts[0], "test_keyed").await,
        vec!["payout_receipt_unrecorded".to_string()]
    );
    // And the next pass sends nothing more to it.
    completed(
        &pass(&fixture, keyed.clone(), &opener, settings(PayoutMode::Live))
            .run_once()
            .await,
    );
    assert_eq!(keyed.effects(), 1);
}

/// A credit ledger row with the submission it needs, as the owner.
async fn ledger_event(
    fixture: &Fixture,
    tenant: &str,
    event_id: Uuid,
    principal: &str,
    points: &str,
    settlement_state: &str,
    external_ref: Option<String>,
) {
    let submission = Uuid::new_v4();
    let trace = Uuid::new_v4();
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_submissions (
             tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
             consent_policy_version, retention_policy_id, status, privacy_risk,
             redaction_pipeline_version, redaction_hash
         ) VALUES ($1, $2, $3, $4, 'v1', 'v1', 'test', 'accepted', 'low', 'test', $5)",
        &[&tenant, &submission, &trace, &principal, &"a".repeat(64)],
    )
    .await;
    owner_exec(
        fixture,
        tenant,
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
    .await;
}
