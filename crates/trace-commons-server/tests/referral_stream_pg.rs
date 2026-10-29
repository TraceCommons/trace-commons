// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral payout stream against a real PostgreSQL and a loopback mock
//! of cloud-api #1271 (docs/superpowers/specs/2026-10-29-referral-rewards-
//! design.md, section 5): a link's second program (`tr1-`), lines per
//! stream, the dedicated referral rail object, per-stream crash recovery,
//! close with both totals, the referral-only holds and ceiling, and the
//! referral mode.
//!
//! Bonus rows are seeded directly by the migrator: the bonus accrual that
//! writes them is another change. Every store call the pass makes runs as a
//! real `LOGIN` role holding only `trace_ingest_runtime` and, with
//! `INHERIT FALSE`, the two narrow payout roles, as `payout_pass_pg` does,
//! so a missing grant fails here as it would in production.
//!
//! The referral rail is a second `NearAiRail` on the same mock: the mock
//! keeps one sender org's history, as #1271 does whichever credential sends,
//! so the two streams are told apart by program alone.
//!
//! Needs PostgreSQL 16 or later, a database named `admission_test_*` on the
//! literal host `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`.
//! Without the variable every test prints a skip line and returns. Run with
//! `--test-threads=1`: the rail's kill switch is deployment-wide.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

#[path = "support/near_ai_payout_mock.rs"]
#[allow(dead_code)]
mod near_ai_payout_mock;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use near_ai_payout_mock::{Mock, Transfer, recipient_org};
use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_gate_api::pipeline::AtomicUnits;
use trace_commons_gate_api::{PayoutRail, RailId, RailRate, RecipientKind, RecipientRef};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::{
    Database, PayoutLineWrite, PayoutLink, PayoutLinkClose, PayoutLinkInsert, PayoutLinkState,
    PayoutLinkWrite, PayoutRateEpochRecord, PayoutStore, PayoutStream, PendingPayoutLineInsert,
    SealedRecipient,
};
use trace_commons_server::near_ai_rail::{
    NEAR_AI_PROGRAM_PREFIX, NEAR_AI_REFERRAL_PROGRAM_PREFIX, near_ai_receipt_hash,
    near_ai_referral_program_for_link,
};
use trace_commons_server::payout_pass::{
    PayoutPass, PayoutPassOutcome, PayoutPassReport, PayoutPassSettings, PayoutRecipientOpener,
    REFERRAL_OVER_CEILING, ReferralStreamSettings,
};
use trace_commons_server::payout_rate::PayoutMode;
use trace_commons_server::referral_bonus::near_ai_referral_control_rail;
use trace_commons_server::trace_corpus_storage::{
    TraceCreditAccountSettlementLineItem, TraceCreditSettlementNearStatus,
};
use uuid::Uuid;

const RUNTIME_LOGIN: &str = "tc_referral_stream_runtime_test";
const ACTOR: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const CEILING: u128 = 1_000_000_000_000;
const REFERRAL_CEILING: u128 = 1_000_000;

/// As `payout_pass_pg`: the pilot tables accrual and the account standing
/// read, granted to the runtime because a database migrated straight to
/// head lacks the pilot group's V62 grant.
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

fn near_ai() -> RailId {
    RailId::near_ai()
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
    let recorded = runtime
        .record_payout_rate_epoch(&near_ai(), &RailRate::new("v1", 1, 1).unwrap(), None)
        .await
        .unwrap();
    assert_ne!(recorded, PayoutRateEpochRecord::Conflict);
    if runtime.payout_control(&near_ai()).await.unwrap().halted {
        runtime
            .set_payout_control(&near_ai(), false, Some("test_reset"), ACTOR)
            .await
            .unwrap();
    }
    // The referral stream's self-trip is deployment-wide too, and the bonus
    // suite trips it.
    let control = near_ai_referral_control_rail();
    if runtime.payout_control(&control).await.unwrap().halted {
        runtime
            .set_payout_control(&control, false, Some("test_reset"), ACTOR)
            .await
            .unwrap();
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

/// Runs `sql` as the owner (the migrator) in `tenant`'s context.
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

/// Reads one row as the owner in `tenant`'s context.
async fn owner_query(
    fixture: &Fixture,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Vec<tokio_postgres::Row> {
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let rows = tx.query(sql, params).await.unwrap();
    tx.commit().await.unwrap();
    rows
}

async fn tenant_with_accounts(fixture: &Fixture, accounts: usize) -> (String, Vec<Uuid>) {
    let tenant = format!("referral-stream-{}", Uuid::new_v4());
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

/// Settles `micros` of the account's own credit (one to one into rail
/// units): the earned stream's owed.
async fn settle(fixture: &Fixture, tenant: &str, account: Uuid, micros: i64) {
    let credit_account_ref = format!("account:{account}");
    let item = TraceCreditAccountSettlementLineItem {
        credit_account_ref: credit_account_ref.clone(),
        credit_account_hash: sha(&format!("credit-account:{credit_account_ref}")),
        settled_credit_delta_micros: micros,
        source_credit_event_ids: Vec::new(),
        source_submission_ids: Vec::new(),
        source_list_hash: sha(&format!("items:{credit_account_ref}:{}", Uuid::new_v4())),
        near_status: TraceCreditSettlementNearStatus::Disabled,
        near_outbox_id: None,
        near_payout_hold_reason: None,
    };
    let batch = Uuid::new_v4();
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
            &Vec::<Uuid>::new(),
            &sha(&format!("list:{batch}")),
            &serde_json::to_value([item]).unwrap(),
            &created_at,
        ],
    )
    .await;
}

/// A referee mirror row for `referrer`, as the bonus pass would leave it.
async fn referee(fixture: &Fixture, tenant: &str, referrer: Uuid) -> String {
    let referee_ref_hash = sha(&format!("referee:{}", Uuid::new_v4()));
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_referral_referees
             (tenant_id, referee_ref_hash, account_id, code_index,
              referee_account_created_at, applied_at)
         VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp())",
        &[
            &tenant,
            &referee_ref_hash,
            &referrer,
            &sha(&format!("code:{}", Uuid::new_v4())),
        ],
    )
    .await;
    referee_ref_hash
}

/// One bonus row worth `units` to `referrer` from `referee_ref_hash`, as
/// the bonus accrual (another change) writes it: 10% of a base ten times
/// its size.
async fn bonus(
    fixture: &Fixture,
    tenant: &str,
    referrer: Uuid,
    referee_ref_hash: &str,
    units: u128,
) {
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_referral_bonus_accruals
             (tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
              source_accrual_id, bps, base_units, bonus_units, rate_version, instrument_id)
         VALUES ($1, $2, 'near_ai', $3, $4, 'bonus', $5, 1000,
                 $6::TEXT::NUMERIC, $7::TEXT::NUMERIC, 'v1', 'near_ai_usd')",
        &[
            &tenant,
            &Uuid::new_v4(),
            &referrer,
            &referee_ref_hash,
            &Uuid::new_v4(),
            &(units * 10).to_string(),
            &units.to_string(),
        ],
    )
    .await;
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

async fn link(
    fixture: &Fixture,
    tenant: &str,
    account: Uuid,
    rail: &dyn PayoutRail,
    opener: &TestOpener,
    org: Uuid,
    state: PayoutLinkState,
) -> PayoutLinkWrite {
    let link_id = Uuid::new_v4();
    let write = PayoutLinkWrite {
        account_id: account,
        rail_id: near_ai(),
        link_id,
        recipient_kind: RecipientKind::new("test_recipient").unwrap(),
        recipient: sealed(&org.to_string()),
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
    opener.0.lock().unwrap().insert(link_id, org.to_string());
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

fn referral_settings(mode: PayoutMode) -> ReferralStreamSettings {
    ReferralStreamSettings {
        mode,
        account_ceiling: AtomicUnits::from_raw(REFERRAL_CEILING),
    }
}

/// A rail pass with a referral stream in `referral` mode over its own rail
/// object.
struct Rails {
    mock: Mock,
    earned: Arc<dyn PayoutRail>,
    referral: Arc<dyn PayoutRail>,
}

impl Rails {
    async fn start() -> Self {
        let mock = Mock::start().await;
        let earned: Arc<dyn PayoutRail> = Arc::new(mock.rail());
        let referral: Arc<dyn PayoutRail> = Arc::new(mock.rail());
        Self {
            mock,
            earned,
            referral,
        }
    }

    fn pass(
        &self,
        fixture: &Fixture,
        opener: &Arc<TestOpener>,
        mode: PayoutMode,
        referral: Option<ReferralStreamSettings>,
    ) -> PayoutPass {
        let pass = PayoutPass::new(
            self.earned.clone(),
            fixture.runtime.clone() as Arc<dyn PayoutStore>,
            fixture.runtime.clone() as Arc<dyn Database>,
            opener.clone() as Arc<dyn PayoutRecipientOpener>,
            settings(mode),
        );
        match referral {
            Some(settings) => pass.with_referral_stream(Some(self.referral.clone()), settings),
            None => pass,
        }
    }

    fn live(&self, fixture: &Fixture, opener: &Arc<TestOpener>) -> PayoutPass {
        self.pass(
            fixture,
            opener,
            PayoutMode::Live,
            Some(referral_settings(PayoutMode::Live)),
        )
    }

    /// Transfers to `org` whose program starts with `prefix`.
    fn sent(&self, org: Uuid, prefix: &str) -> Vec<Transfer> {
        self.mock
            .transfers()
            .into_iter()
            .filter(|t| t.recipient == org && t.program.starts_with(prefix))
            .collect()
    }

    fn total(&self, org: Uuid, prefix: &str) -> i64 {
        self.sent(org, prefix).iter().map(|t| t.amount).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineRow {
    link_id: Uuid,
    stream: String,
    state: String,
    target: String,
    receipt: Option<String>,
    label: Option<String>,
}

async fn lines(fixture: &Fixture, tenant: &str) -> Vec<LineRow> {
    owner_query(
        fixture,
        tenant,
        "SELECT link_id, stream, state, target_total::TEXT, receipt_hash, last_error_label
           FROM trace_payout_lines
          WHERE tenant_id = $1 AND rail_id = 'near_ai'
          ORDER BY created_at, line_id",
        &[&tenant],
    )
    .await
    .iter()
    .map(|row| LineRow {
        link_id: row.get(0),
        stream: row.get(1),
        state: row.get(2),
        target: row.get(3),
        receipt: row.get(4),
        label: row.get(5),
    })
    .collect()
}

/// A link's state, final totals and referral program, as text.
async fn link_row(
    fixture: &Fixture,
    tenant: &str,
    link_id: Uuid,
) -> (String, Option<String>, Option<String>, Option<String>) {
    let rows = owner_query(
        fixture,
        tenant,
        "SELECT state, final_paid::TEXT, final_paid_referral::TEXT, referral_program
           FROM trace_payout_links WHERE tenant_id = $1 AND link_id = $2",
        &[&tenant, &link_id],
    )
    .await;
    (
        rows[0].get(0),
        rows[0].get(1),
        rows[0].get(2),
        rows[0].get(3),
    )
}

async fn holds(fixture: &Fixture, tenant: &str, account: Uuid) -> Vec<String> {
    let mut labels = fixture
        .runtime
        .open_payout_hold_labels(tenant, account, &near_ai())
        .await
        .unwrap();
    labels.sort();
    labels
}

fn referral_line(link: &PayoutLinkWrite, target: u128, delta: u128) -> PayoutLineWrite {
    PayoutLineWrite {
        line_id: Uuid::new_v4(),
        link_id: link.link_id,
        account_id: link.account_id,
        rail_id: near_ai(),
        target_total: AtomicUnits::from_raw(target),
        delta: AtomicUnits::from_raw(delta),
        owed_through_accrual_count: 1,
        stream: PayoutStream::Referral,
        referral_program: Some(near_ai_referral_program_for_link(link.link_id)),
    }
}

fn earned_line(link: &PayoutLinkWrite, target: u128, delta: u128) -> PayoutLineWrite {
    PayoutLineWrite {
        stream: PayoutStream::Earned,
        referral_program: None,
        ..referral_line(link, target, delta)
    }
}

fn completed(report: &PayoutPassReport) {
    assert_eq!(report.outcome, PayoutPassOutcome::Completed, "{report:?}");
    assert_eq!(report.referral_stopped, None, "{report:?}");
}

#[tokio::test]
async fn earned_and_referral_sends_are_independent() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    let account = accounts[0];
    let org = recipient_org(1);
    settle(&fixture, &tenant, account, 3_000).await;
    let referee_ref = referee(&fixture, &tenant, account).await;
    bonus(&fixture, &tenant, account, &referee_ref, 200).await;
    bonus(&fixture, &tenant, account, &referee_ref, 300).await;
    let linked = link(
        &fixture,
        &tenant,
        account,
        rails.earned.as_ref(),
        &opener,
        org,
        PayoutLinkState::Active,
    )
    .await;

    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    assert_eq!((report.sent, report.confirmed), (1, 1), "{report:?}");
    assert_eq!(
        (report.referral_sent, report.referral_confirmed),
        (1, 1),
        "{report:?}"
    );

    // Two programs on one link: own credit on tc1-, referral on tr1-.
    let earned = rails.sent(org, NEAR_AI_PROGRAM_PREFIX);
    let referral = rails.sent(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX);
    assert_eq!(earned.len(), 1);
    assert_eq!(
        (earned[0].program.as_str(), earned[0].amount),
        (linked.program.as_str(), 3_000)
    );
    let program = near_ai_referral_program_for_link(linked.link_id);
    assert_eq!(referral.len(), 1);
    assert_eq!(
        (referral[0].program.as_str(), referral[0].amount),
        (program.as_str(), 500)
    );
    // The program is set on the link with its first referral line.
    assert_eq!(
        link_row(&fixture, &tenant, linked.link_id).await,
        ("active".into(), None, None, Some(program.clone()))
    );
    let rows = lines(&fixture, &tenant).await;
    assert_eq!(rows.len(), 2);
    let by_stream = |stream: &str| rows.iter().find(|r| r.stream == stream).unwrap().clone();
    assert_eq!(by_stream("earned").state, "confirmed");
    assert_eq!(by_stream("earned").target, "3000");
    assert_eq!(
        by_stream("earned").receipt.as_deref(),
        Some(near_ai_receipt_hash(earned[0].id).as_str())
    );
    assert_eq!(by_stream("referral").state, "confirmed");
    assert_eq!(by_stream("referral").target, "500");
    assert_eq!(
        by_stream("referral").receipt.as_deref(),
        Some(near_ai_receipt_hash(referral[0].id).as_str())
    );

    // New own credit alone moves the earned stream alone.
    settle(&fixture, &tenant, account, 1_000).await;
    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    assert_eq!((report.sent, report.referral_sent), (1, 0), "{report:?}");
    assert_eq!(rails.total(org, NEAR_AI_PROGRAM_PREFIX), 4_000);
    assert_eq!(rails.total(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 500);

    // New bonus alone moves the referral stream alone.
    bonus(&fixture, &tenant, account, &referee_ref, 250).await;
    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    assert_eq!((report.sent, report.referral_sent), (0, 1), "{report:?}");
    assert_eq!(rails.total(org, NEAR_AI_PROGRAM_PREFIX), 4_000);
    assert_eq!(rails.total(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 750);

    // The account's status amounts, per stream.
    for (stream, sent) in [(PayoutStream::Earned, 4_000), (PayoutStream::Referral, 750)] {
        assert_eq!(
            fixture
                .runtime
                .payout_account_stream_sent(&tenant, account, &near_ai(), stream)
                .await
                .unwrap(),
            sent,
            "{stream:?}"
        );
    }
    let owed = fixture
        .runtime
        .payout_referral_owed(&tenant, account, &near_ai())
        .await
        .unwrap();
    assert_eq!((owed.net, owed.bonus_count), (Some(750), 3));

    // Nothing more on a rerun.
    completed(&rails.live(&fixture, &opener).run_once().await);
    assert_eq!(rails.mock.transfers().len(), 4);
}

#[tokio::test]
async fn each_stream_sends_exactly_once_across_a_crash() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
    // Account 0: the earned line never went out, the referral line landed
    // and its response was lost. Account 1: the reverse.
    let mut links = Vec::new();
    for (n, account) in accounts.iter().enumerate() {
        settle(&fixture, &tenant, *account, 3_000).await;
        let referee_ref = referee(&fixture, &tenant, *account).await;
        bonus(&fixture, &tenant, *account, &referee_ref, 400).await;
        let linked = link(
            &fixture,
            &tenant,
            *account,
            rails.earned.as_ref(),
            &opener,
            recipient_org(10 + n as u128),
            PayoutLinkState::Active,
        )
        .await;
        for line in [
            earned_line(&linked, 3_000, 3_000),
            referral_line(&linked, 400, 400),
        ] {
            assert_eq!(
                fixture
                    .runtime
                    .insert_pending_payout_line(&tenant, &line)
                    .await
                    .unwrap(),
                PendingPayoutLineInsert::Inserted,
                "one pending line per stream"
            );
        }
        // A second pending line on a stream is refused.
        assert_eq!(
            fixture
                .runtime
                .insert_pending_payout_line(&tenant, &referral_line(&linked, 400, 400))
                .await
                .unwrap(),
            PendingPayoutLineInsert::LinkHasPendingLine
        );
        links.push(linked);
    }
    let lost_referral = rails.mock.seed(
        recipient_org(10),
        &near_ai_referral_program_for_link(links[0].link_id),
        400,
    );
    let lost_earned = rails.mock.seed(recipient_org(11), &links[1].program, 3_000);

    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    // One new 201 per stream: account 0's earned, account 1's referral.
    assert_eq!(rails.mock.transfers().len(), 4, "{report:?}");
    assert_eq!(
        rails.sent(recipient_org(10), NEAR_AI_PROGRAM_PREFIX).len(),
        1
    );
    assert_eq!(
        rails
            .sent(recipient_org(10), NEAR_AI_REFERRAL_PROGRAM_PREFIX)
            .len(),
        1
    );
    assert_eq!(
        rails.sent(recipient_org(11), NEAR_AI_PROGRAM_PREFIX).len(),
        1
    );
    assert_eq!(
        rails
            .sent(recipient_org(11), NEAR_AI_REFERRAL_PROGRAM_PREFIX)
            .len(),
        1
    );
    let rows = lines(&fixture, &tenant).await;
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|r| r.state == "confirmed"), "{rows:?}");
    let receipt = |link: &PayoutLinkWrite, stream: &str| {
        rows.iter()
            .find(|r| r.link_id == link.link_id && r.stream == stream)
            .unwrap()
            .receipt
            .clone()
    };
    // Recovery found each lost send on its own stream's program.
    assert_eq!(
        receipt(&links[0], "referral").as_deref(),
        Some(near_ai_receipt_hash(lost_referral).as_str())
    );
    assert_eq!(
        receipt(&links[1], "earned").as_deref(),
        Some(near_ai_receipt_hash(lost_earned).as_str())
    );
    assert!(holds(&fixture, &tenant, accounts[0]).await.is_empty());
    assert!(holds(&fixture, &tenant, accounts[1]).await.is_empty());

    completed(&rails.live(&fixture, &opener).run_once().await);
    assert_eq!(rails.mock.transfers().len(), 4, "nothing more on a rerun");
}

#[tokio::test]
async fn a_lost_referral_send_is_never_matched_to_an_earned_record() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    let referee_ref = referee(&fixture, &tenant, accounts[0]).await;
    bonus(&fixture, &tenant, accounts[0], &referee_ref, 700).await;
    let linked = link(
        &fixture,
        &tenant,
        accounts[0],
        rails.earned.as_ref(),
        &opener,
        recipient_org(20),
        PayoutLinkState::Active,
    )
    .await;
    fixture
        .runtime
        .insert_pending_payout_line(&tenant, &referral_line(&linked, 700, 700))
        .await
        .unwrap();
    // The lost send landed on the referral program. A record of the same
    // size and cumulative total, newer, sits on the earned program (an
    // outside send): the history walk meets it first, and must pass it by.
    let referral_record = rails.mock.seed(
        recipient_org(20),
        &near_ai_referral_program_for_link(linked.link_id),
        700,
    );
    let earned_record = rails.mock.seed(recipient_org(20), &linked.program, 700);

    let report = rails.live(&fixture, &opener).run_once().await;
    assert_eq!(report.outcome, PayoutPassOutcome::Completed, "{report:?}");
    let rows = lines(&fixture, &tenant).await;
    let referral = rows.iter().find(|r| r.stream == "referral").unwrap();
    assert_eq!(referral.state, "confirmed");
    assert_eq!(
        referral.receipt.as_deref(),
        Some(near_ai_receipt_hash(referral_record).as_str()),
        "receipt lookup reads the referral program only"
    );
    assert_ne!(
        referral.receipt.as_deref(),
        Some(near_ai_receipt_hash(earned_record).as_str())
    );
    // The referral stream sent nothing new.
    assert_eq!(
        rails
            .sent(recipient_org(20), NEAR_AI_REFERRAL_PROGRAM_PREFIX)
            .len(),
        1
    );
}

#[tokio::test]
async fn relinking_pays_referral_owed_once_across_programs() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    let account = accounts[0];
    let referee_ref = referee(&fixture, &tenant, account).await;
    let live = || rails.live(&fixture, &opener);

    settle(&fixture, &tenant, account, 2_000).await;
    bonus(&fixture, &tenant, account, &referee_ref, 1_000).await;
    let first = link(
        &fixture,
        &tenant,
        account,
        rails.earned.as_ref(),
        &opener,
        recipient_org(30),
        PayoutLinkState::Waiting,
    )
    .await;
    completed(&live().run_once().await);
    // Change to org 31, then back to org 30 under new programs.
    bonus(&fixture, &tenant, account, &referee_ref, 500).await;
    let second = link(
        &fixture,
        &tenant,
        account,
        rails.earned.as_ref(),
        &opener,
        recipient_org(31),
        PayoutLinkState::Waiting,
    )
    .await;
    completed(&live().run_once().await);
    bonus(&fixture, &tenant, account, &referee_ref, 250).await;
    let back = link(
        &fixture,
        &tenant,
        account,
        rails.earned.as_ref(),
        &opener,
        recipient_org(30),
        PayoutLinkState::Waiting,
    )
    .await;
    completed(&live().run_once().await);
    completed(&live().run_once().await);

    let referral: Vec<Transfer> = rails
        .mock
        .transfers()
        .into_iter()
        .filter(|t| t.program.starts_with(NEAR_AI_REFERRAL_PROGRAM_PREFIX))
        .collect();
    assert_eq!(
        referral.iter().map(|t| t.amount).sum::<i64>(),
        1_750,
        "every referral unit owed, once"
    );
    let programs: std::collections::BTreeSet<&str> =
        referral.iter().map(|t| t.program.as_str()).collect();
    assert_eq!(
        programs.len(),
        3,
        "one referral program per link generation"
    );
    // Own credit was owed once too, all on the first link.
    let earned: i64 = rails
        .mock
        .transfers()
        .iter()
        .filter(|t| t.program.starts_with(NEAR_AI_PROGRAM_PREFIX))
        .map(|t| t.amount)
        .sum();
    assert_eq!(earned, 2_000);

    // Each closed link recorded both programs' totals.
    let first_row = link_row(&fixture, &tenant, first.link_id).await;
    assert_eq!(
        first_row,
        (
            "closed".into(),
            Some("2000".into()),
            Some("1000".into()),
            Some(near_ai_referral_program_for_link(first.link_id))
        )
    );
    let second_row = link_row(&fixture, &tenant, second.link_id).await;
    assert_eq!(
        (
            second_row.0.as_str(),
            second_row.1.as_deref(),
            second_row.2.as_deref()
        ),
        ("closed", Some("0"), Some("500"))
    );
    assert_eq!(link_row(&fixture, &tenant, back.link_id).await.0, "active");
    assert_eq!(
        rails.total(
            recipient_org(30),
            &near_ai_referral_program_for_link(back.link_id)
        ),
        250
    );
    assert_eq!(
        fixture
            .runtime
            .payout_closed_links_final_paid_referral(&tenant, account, &near_ai())
            .await
            .unwrap(),
        1_500
    );
}

#[tokio::test]
async fn a_referral_hold_stops_only_the_referral_stream() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 2).await;
    // Account 0: the bonus pass held its referral stream (its code was
    // suspended). Account 1: its referral owed is over the referral ceiling.
    let mut orgs = Vec::new();
    for (n, account) in accounts.iter().enumerate() {
        settle(&fixture, &tenant, *account, 1_000).await;
        let referee_ref = referee(&fixture, &tenant, *account).await;
        let units = if n == 0 { 300 } else { REFERRAL_CEILING + 1 };
        bonus(&fixture, &tenant, *account, &referee_ref, units).await;
        let org = recipient_org(40 + n as u128);
        link(
            &fixture,
            &tenant,
            *account,
            rails.earned.as_ref(),
            &opener,
            org,
            PayoutLinkState::Active,
        )
        .await;
        orgs.push(org);
    }
    assert!(
        fixture
            .runtime
            .open_payout_hold(
                &tenant,
                accounts[0],
                &near_ai(),
                "referral_code_suspended",
                ACTOR
            )
            .await
            .unwrap()
    );

    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    // Own credit reaches both; referral credit reaches neither.
    for org in &orgs {
        assert_eq!(rails.total(*org, NEAR_AI_PROGRAM_PREFIX), 1_000);
        assert_eq!(rails.total(*org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 0);
    }
    assert_eq!(report.referral_sent, 0, "{report:?}");
    assert_eq!(
        holds(&fixture, &tenant, accounts[0]).await,
        vec!["referral_code_suspended".to_string()]
    );
    assert_eq!(
        holds(&fixture, &tenant, accounts[1]).await,
        vec![REFERRAL_OVER_CEILING.to_string()]
    );
    assert!(
        lines(&fixture, &tenant)
            .await
            .iter()
            .all(|r| r.stream == "earned"),
        "no referral line is written while held"
    );

    // A referral hold never holds the earned stream: new own credit pays.
    for account in &accounts {
        settle(&fixture, &tenant, *account, 500).await;
    }
    completed(&rails.live(&fixture, &opener).run_once().await);
    for org in &orgs {
        assert_eq!(rails.total(*org, NEAR_AI_PROGRAM_PREFIX), 1_500);
        assert_eq!(rails.total(*org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 0);
    }

    // Cleared, the referral stream pays what it was owed.
    assert!(
        fixture
            .runtime
            .clear_payout_hold(
                &tenant,
                accounts[0],
                &near_ai(),
                "referral_code_suspended",
                ACTOR
            )
            .await
            .unwrap()
    );
    completed(&rails.live(&fixture, &opener).run_once().await);
    assert_eq!(rails.total(orgs[0], NEAR_AI_REFERRAL_PROGRAM_PREFIX), 300);
    assert_eq!(rails.total(orgs[1], NEAR_AI_REFERRAL_PROGRAM_PREFIX), 0);

    // The ceiling hold lifts itself once the referrer fits a raised ceiling.
    let raised = ReferralStreamSettings {
        mode: PayoutMode::Live,
        account_ceiling: AtomicUnits::from_raw(REFERRAL_CEILING * 2),
    };
    completed(
        &rails
            .pass(&fixture, &opener, PayoutMode::Live, Some(raised))
            .run_once()
            .await,
    );
    assert_eq!(
        rails.total(orgs[1], NEAR_AI_REFERRAL_PROGRAM_PREFIX),
        (REFERRAL_CEILING + 1) as i64
    );
    assert!(holds(&fixture, &tenant, accounts[1]).await.is_empty());
}

#[tokio::test]
async fn an_earned_hold_does_not_stop_the_referral_stream() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    let referee_ref = referee(&fixture, &tenant, accounts[0]).await;
    bonus(&fixture, &tenant, accounts[0], &referee_ref, 300).await;
    let org = recipient_org(50);
    link(
        &fixture,
        &tenant,
        accounts[0],
        rails.earned.as_ref(),
        &opener,
        org,
        PayoutLinkState::Active,
    )
    .await;
    // An earned-stream hold, as an outside send to the earned program
    // leaves.
    fixture
        .runtime
        .open_payout_hold(
            &tenant,
            accounts[0],
            &near_ai(),
            "payout_program_total_mismatch",
            ACTOR,
        )
        .await
        .unwrap();
    completed(&rails.live(&fixture, &opener).run_once().await);
    assert_eq!(rails.total(org, NEAR_AI_PROGRAM_PREFIX), 0);
    assert_eq!(rails.total(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 300);

    // A closed account holds both.
    owner_exec(
        &fixture,
        &tenant,
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&tenant, &accounts[0]],
    )
    .await;
    bonus(&fixture, &tenant, accounts[0], &referee_ref, 100).await;
    completed(&rails.live(&fixture, &opener).run_once().await);
    assert_eq!(rails.total(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 300);
}

#[tokio::test]
async fn referral_mode_disabled_sends_nothing_and_dry_run_only_counts() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    let referee_ref = referee(&fixture, &tenant, accounts[0]).await;
    bonus(&fixture, &tenant, accounts[0], &referee_ref, 300).await;
    let org = recipient_org(60);
    let linked = link(
        &fixture,
        &tenant,
        accounts[0],
        rails.earned.as_ref(),
        &opener,
        org,
        PayoutLinkState::Active,
    )
    .await;

    // No referral stream at all, then one configured `disabled`: earned
    // sends as before, and nothing touches the referral program.
    for referral in [None, Some(referral_settings(PayoutMode::Disabled))] {
        let report = rails
            .pass(&fixture, &opener, PayoutMode::Live, referral)
            .run_once()
            .await;
        completed(&report);
        assert_eq!(report.referral_mode, PayoutMode::Disabled);
        assert_eq!(report.referral_sent, 0);
        assert_eq!(report.referral_dry_run_units, 0);
    }
    assert_eq!(rails.total(org, NEAR_AI_PROGRAM_PREFIX), 1_000);
    assert!(rails.sent(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX).is_empty());
    assert_eq!(link_row(&fixture, &tenant, linked.link_id).await.3, None);
    assert!(
        lines(&fixture, &tenant)
            .await
            .iter()
            .all(|r| r.stream == "earned")
    );

    // `dry_run` counts the referral owed and writes nothing, even in a live
    // pass; a `live` stream in a `dry_run` pass is a dry run too.
    let posts = rails.mock.posts();
    for (mode, referral) in [
        (PayoutMode::Live, PayoutMode::DryRun),
        (PayoutMode::DryRun, PayoutMode::Live),
    ] {
        let report = rails
            .pass(&fixture, &opener, mode, Some(referral_settings(referral)))
            .run_once()
            .await;
        completed(&report);
        assert_eq!(report.referral_mode, PayoutMode::DryRun);
        assert_eq!(report.referral_dry_run_accounts, 1, "{report:?}");
        assert_eq!(report.referral_dry_run_units, 300, "{report:?}");
        assert_eq!(report.referral_sent, 0);
    }
    assert!(rails.sent(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX).is_empty());
    assert_eq!(link_row(&fixture, &tenant, linked.link_id).await.3, None);
    assert!(
        lines(&fixture, &tenant)
            .await
            .iter()
            .all(|r| r.stream == "earned")
    );
    // Nothing was owed on the earned stream, so neither dry run wrote to
    // the rail.
    assert_eq!(rails.mock.posts(), posts);

    // The kill switch stops the referral stream too.
    fixture
        .runtime
        .set_payout_control(&near_ai(), true, Some("test_halt"), ACTOR)
        .await
        .unwrap();
    let report = rails.live(&fixture, &opener).run_once().await;
    assert_eq!(report.outcome, PayoutPassOutcome::Halted);
    fixture
        .runtime
        .set_payout_control(&near_ai(), false, Some("test_reset"), ACTOR)
        .await
        .unwrap();
    assert!(rails.sent(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX).is_empty());
}

/// Spec section 7: once the abuse check has tripped the referral stream
/// (the `near_ai.referral` control row halted), a `live` stream runs as a
/// dry run: it counts, sends nothing and writes no referral line, while the
/// earned stream pays as before. Cleared, it sends again.
#[tokio::test]
async fn a_tripped_referral_stream_sends_nothing_until_cleared() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = Arc::new(TestOpener::default());
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    settle(&fixture, &tenant, accounts[0], 1_000).await;
    let referee_ref = referee(&fixture, &tenant, accounts[0]).await;
    bonus(&fixture, &tenant, accounts[0], &referee_ref, 300).await;
    let org = recipient_org(70);
    link(
        &fixture,
        &tenant,
        accounts[0],
        rails.earned.as_ref(),
        &opener,
        org,
        PayoutLinkState::Active,
    )
    .await;
    let control = near_ai_referral_control_rail();
    fixture
        .runtime
        .set_payout_control(&control, true, Some("referral_tripped"), ACTOR)
        .await
        .unwrap();

    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    assert_eq!(report.referral_mode, PayoutMode::DryRun, "{report:?}");
    assert_eq!(report.referral_sent, 0, "{report:?}");
    assert_eq!(report.referral_dry_run_accounts, 1, "{report:?}");
    assert_eq!(report.referral_dry_run_units, 300, "{report:?}");
    assert_eq!(rails.total(org, NEAR_AI_PROGRAM_PREFIX), 1_000);
    assert!(rails.sent(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX).is_empty());
    assert!(
        lines(&fixture, &tenant)
            .await
            .iter()
            .all(|r| r.stream == "earned"),
        "no referral line is written while the stream is tripped"
    );

    fixture
        .runtime
        .set_payout_control(&control, false, Some("test_reset"), ACTOR)
        .await
        .unwrap();
    let report = rails.live(&fixture, &opener).run_once().await;
    completed(&report);
    assert_eq!(report.referral_mode, PayoutMode::Live, "{report:?}");
    assert_eq!(rails.total(org, NEAR_AI_REFERRAL_PROGRAM_PREFIX), 300);
}

#[tokio::test]
async fn the_store_keeps_streams_apart_and_closes_with_both_totals() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let rails = Rails::start().await;
    let opener = TestOpener::default();
    let (tenant, accounts) = tenant_with_accounts(&fixture, 1).await;
    let linked = link(
        &fixture,
        &tenant,
        accounts[0],
        rails.earned.as_ref(),
        &opener,
        recipient_org(70),
        PayoutLinkState::Active,
    )
    .await;
    let store = fixture.runtime.as_ref();

    // A referral line names its program; an earned line names none.
    let mut nameless = referral_line(&linked, 10, 10);
    nameless.referral_program = None;
    assert!(
        store
            .insert_pending_payout_line(&tenant, &nameless)
            .await
            .is_err()
    );
    let mut named = earned_line(&linked, 10, 10);
    named.referral_program = Some("tr1-x".into());
    assert!(
        store
            .insert_pending_payout_line(&tenant, &named)
            .await
            .is_err()
    );

    // The first referral line sets the program; another program is refused.
    assert_eq!(
        store
            .insert_pending_payout_line(&tenant, &referral_line(&linked, 10, 10))
            .await
            .unwrap(),
        PendingPayoutLineInsert::Inserted
    );
    let program = near_ai_referral_program_for_link(linked.link_id);
    assert_eq!(
        link_row(&fixture, &tenant, linked.link_id).await.3,
        Some(program.clone())
    );
    let mut other = referral_line(&linked, 10, 10);
    other.referral_program = Some("tr1-00000000000000000000000000000000".into());
    assert_eq!(
        store
            .insert_pending_payout_line(&tenant, &other)
            .await
            .unwrap(),
        PendingPayoutLineInsert::ReferralProgramMismatch
    );

    // Each stream reads its own pending line.
    let pending_referral = store
        .pending_payout_line_for_stream(&tenant, linked.link_id, PayoutStream::Referral)
        .await
        .unwrap()
        .expect("the referral line");
    assert_eq!(pending_referral.stream, PayoutStream::Referral);
    assert!(
        store
            .pending_payout_line_for_link(&tenant, linked.link_id)
            .await
            .unwrap()
            .is_none(),
        "the earned read never returns a referral line"
    );
    store
        .resolve_payout_line(
            &tenant,
            pending_referral.line_id,
            &trace_commons_server::db::PayoutLineResolution::Confirmed {
                receipt_hash: sha(&format!("receipt:{}", Uuid::new_v4())),
                delta: AtomicUnits::from_raw(10),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .payout_link_stream_confirmed_total(&tenant, linked.link_id, PayoutStream::Referral)
            .await
            .unwrap(),
        10
    );
    assert_eq!(
        store
            .payout_link_confirmed_total(&tenant, linked.link_id)
            .await
            .unwrap(),
        0
    );

    // Closing a link with a referral program needs its referral total.
    store
        .transition_payout_link(
            &tenant,
            linked.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            trace_commons_server::db::PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                linked.link_id,
                AtomicUnits::from_raw(0),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::ReferralTotalMismatch
    );
    assert_eq!(
        store
            .close_payout_link_streams(
                &tenant,
                linked.link_id,
                AtomicUnits::from_raw(0),
                Some(AtomicUnits::from_raw(10)),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );
    assert_eq!(
        link_row(&fixture, &tenant, linked.link_id).await,
        (
            "closed".into(),
            Some("0".into()),
            Some("10".into()),
            Some(program)
        )
    );
    assert_eq!(
        store
            .payout_closed_links_final_paid_referral(&tenant, accounts[0], &near_ai())
            .await
            .unwrap(),
        10
    );

    // A link that never carried referral credit closes without one, and is
    // refused one.
    let plain = link(
        &fixture,
        &tenant,
        accounts[0],
        rails.earned.as_ref(),
        &opener,
        recipient_org(71),
        PayoutLinkState::Active,
    )
    .await;
    store
        .transition_payout_link(
            &tenant,
            plain.link_id,
            PayoutLinkState::Active,
            PayoutLinkState::Closing,
            trace_commons_server::db::PayoutAuditAction::LinkClosing,
            ACTOR,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .close_payout_link_streams(
                &tenant,
                plain.link_id,
                AtomicUnits::from_raw(0),
                Some(AtomicUnits::from_raw(1)),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::ReferralTotalMismatch
    );
    assert_eq!(
        store
            .close_payout_link(
                &tenant,
                plain.link_id,
                AtomicUnits::from_raw(0),
                None,
                ACTOR,
                None
            )
            .await
            .unwrap(),
        PayoutLinkClose::Closed
    );

    // Bonus rows that sum below zero are refused as an owed amount, never
    // wrapped.
    let referee_ref = referee(&fixture, &tenant, accounts[0]).await;
    owner_exec(
        &fixture,
        &tenant,
        "INSERT INTO trace_referral_bonus_accruals
             (tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
              source_accrual_id, bps, base_units, bonus_units, rate_version, instrument_id)
         VALUES ($1, $2, 'near_ai', $3, $4, 'void', NULL, 1000, 0, -5, 'v1', 'near_ai_usd')",
        &[&tenant, &Uuid::new_v4(), &accounts[0], &referee_ref],
    )
    .await;
    let owed = store
        .payout_referral_owed(&tenant, accounts[0], &near_ai())
        .await
        .unwrap();
    assert_eq!((owed.net, owed.bonus_count), (None, 1));
}
