// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral bonus pass against a real PostgreSQL
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 4 and 7): the cross-tenant bonus write, its window and reversals, the
//! referrer-gone void, the referral-stream holds, code scoring and
//! suspension, the dry_run trip, and voids taking back unsent bonus.
//!
//! Fixtures (tenants, accounts, batches, accruals, codes, attributions,
//! mirror rows) are written by the migrator. Everything the pass and the
//! store do runs as `trace_ingest_runtime` (NOBYPASSRLS), set as the session
//! role through the connection options, so a missing grant is a real
//! refusal. The resolver and config-writer roles are taken with
//! `SET LOCAL ROLE` from that session.
//!
//! Needs a database whose name starts with `admission_test_`, on the literal
//! host `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Without the
//! variable every test prints a skip line and returns. Run with
//! `--test-threads=1`: the referral stream's trip is deployment-wide.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use trace_commons_gate_api::RailId;
use trace_commons_gate_api::pipeline::InstrumentId;
use trace_commons_server::{
    config::DatabaseConfig,
    db::{
        Database, PayoutStore, ReferralBonusKind, ReferralBonusStore, ReferralBonusWrite,
        postgres::PgBackend,
    },
    payout_rate::PayoutMode,
    referral_bonus::{ReferralBonusHook, near_ai_referral_control_rail},
    referral_config::ReferralBonusSettings,
};
use uuid::Uuid;

const ACTOR: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";

struct Fixture {
    owner: PgBackend,
    runtime: Arc<PgBackend>,
}

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).expect("database url");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_"));
    let owner = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
        .await
        .expect("owner backend");
    owner.run_migrations().await.expect("apply migrations");
    owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(
            "INSERT INTO trace_payout_rate_epochs (rail_id, version, numerator, denominator)
             VALUES ('near_ai', 'v1', 1, 1) ON CONFLICT DO NOTHING;
             -- V90: the runtime's privileges on tables before V62 are the
             -- operator's. The pass reads these through the pilot grant.
             GRANT SELECT ON trace_accounts, trace_credit_settlement_batches,
                 trace_account_principals, trace_credit_holds, trace_submissions
                 TO trace_ingest_runtime;",
        )
        .await
        .expect("record the test rate epoch and pilot grants");
    let mut runtime_url = parsed.clone();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 4))
        .await
        .expect("runtime backend");
    let row = runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT current_user::text, rolsuper, rolbypassrls
               FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "trace_ingest_runtime");
    assert!(!row.get::<_, bool>(1) && !row.get::<_, bool>(2));
    Some(Fixture {
        owner,
        runtime: Arc::new(runtime),
    })
}

fn sha(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn near_ai() -> RailId {
    RailId::near_ai()
}

fn settings() -> ReferralBonusSettings {
    ReferralBonusSettings {
        mode: PayoutMode::DryRun,
        ..ReferralBonusSettings::default()
    }
}

fn hook(fixture: &Fixture, settings: ReferralBonusSettings) -> ReferralBonusHook {
    ReferralBonusHook::new(
        fixture.runtime.clone() as Arc<dyn ReferralBonusStore>,
        fixture.runtime.clone() as Arc<dyn PayoutStore>,
        settings,
    )
}

/// Runs `sql` as the migrator in `tenant`'s context.
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

async fn runtime_query(
    fixture: &Fixture,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Vec<tokio_postgres::Row> {
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
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

/// A tenant with one account created at `created_at`.
async fn tenant_with_account(fixture: &Fixture, created_at: DateTime<Utc>) -> (String, Uuid) {
    let tenant = format!("nearai-referral-bonus-{}", Uuid::new_v4().simple());
    let account = Uuid::new_v4();
    owner_exec(
        fixture,
        &tenant,
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
        &[&tenant],
    )
    .await;
    owner_exec(
        fixture,
        &tenant,
        "INSERT INTO trace_accounts (tenant_id, account_id, created_at) VALUES ($1, $2, $3)",
        &[&tenant, &account, &created_at],
    )
    .await;
    (tenant, account)
}

/// A referrer and their code.
struct Referrer {
    tenant: String,
    account: Uuid,
    code_index: String,
}

async fn referrer(fixture: &Fixture) -> Referrer {
    let (tenant, account) = tenant_with_account(fixture, Utc::now() - Duration::days(400)).await;
    let code_index = code(fixture, &tenant, account).await;
    Referrer {
        tenant,
        account,
        code_index,
    }
}

async fn code(fixture: &Fixture, tenant: &str, account: Uuid) -> String {
    let code_index = sha(&format!("code:{}", Uuid::new_v4()));
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_referral_codes
             (tenant_id, code_id, account_id, code_index, sealed_code, code_key_ref,
              index_pepper_ref)
         VALUES ($1, $2, $3, $4, '{\"v\":1}'::jsonb, 'key-test-1', 'pepper-test-1')",
        &[&tenant, &Uuid::new_v4(), &account, &code_index],
    )
    .await;
    code_index
}

struct Referee {
    tenant: String,
    account: Uuid,
    referee_ref: String,
}

/// A referee created `age_days` ago who applied `code_index` `applied_days`
/// after creation, with its attribution and the referrer's mirror row.
async fn referee(
    fixture: &Fixture,
    referrer: &Referrer,
    code_index: &str,
    age_days: i64,
    applied_days: i64,
) -> Referee {
    let created = Utc::now() - Duration::days(age_days);
    let applied = created + Duration::days(applied_days);
    let (tenant, account) = tenant_with_account(fixture, created).await;
    let referee_ref = sha(&format!("tc-referral-referee-v1:{tenant}:{account}"));
    owner_exec(
        fixture,
        &tenant,
        "INSERT INTO trace_referral_attributions
             (tenant_id, account_id, code_index, referrer_tenant_id, referrer_account_id,
              referee_ref_hash, referee_account_created_at, applied_via, applied_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'later', $8)",
        &[
            &tenant,
            &account,
            &code_index,
            &referrer.tenant,
            &referrer.account,
            &referee_ref,
            &created,
            &applied,
        ],
    )
    .await;
    owner_exec(
        fixture,
        &referrer.tenant,
        "INSERT INTO trace_referral_referees
             (tenant_id, referee_ref_hash, account_id, code_index,
              referee_account_created_at, applied_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &referrer.tenant,
            &referee_ref,
            &referrer.account,
            &code_index,
            &created,
            &applied,
        ],
    )
    .await;
    Referee {
        tenant,
        account,
        referee_ref,
    }
}

/// A finalized batch created at `created_at`, and the account's credit
/// accrual on `near_ai` from it. Returns `(batch, line hash)`.
async fn credit(
    fixture: &Fixture,
    tenant: &str,
    account: Uuid,
    units: i64,
    created_at: DateTime<Utc>,
) -> (Uuid, String) {
    let batch = Uuid::new_v4();
    let line = sha(&format!("line:{batch}"));
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_credit_settlement_batches (
             tenant_id, settlement_batch_id, policy_version, status, reason_hash,
             source_list_hash, settled_credit_points, settled_credit_micros,
             actor_principal_ref, created_at
         ) VALUES ($1, $2, 'test', 'finalized', $3, $4, '1', 1000000, 'operator', $5)",
        &[
            &tenant,
            &batch,
            &sha(&format!("reason:{batch}")),
            &sha(&format!("list:{batch}")),
            &created_at,
        ],
    )
    .await;
    let units = units.to_string();
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_payout_accruals (
             tenant_id, accrual_id, rail_id, kind, settlement_batch_id, credit_account_hash,
             account_id, award_units, rail_units, rate_version, instrument_id
         ) VALUES ($1, $2, 'near_ai', 'credit', $3, $4, $5,
                   $6::TEXT::NUMERIC, $6::TEXT::NUMERIC, 'v1', 'near_ai_usd')",
        &[&tenant, &Uuid::new_v4(), &batch, &line, &account, &units],
    )
    .await;
    (batch, line)
}

/// A reversal accrual of `units` against the line `(batch, line)`.
async fn reversal(
    fixture: &Fixture,
    tenant: &str,
    account: Uuid,
    (batch, line): &(Uuid, String),
    units: i64,
) {
    let units = format!("-{units}");
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_payout_accruals (
             tenant_id, accrual_id, rail_id, kind, settlement_batch_id, credit_account_hash,
             reversal_event_id, account_id, award_units, rail_units, rate_version,
             instrument_id
         ) VALUES ($1, $2, 'near_ai', 'reversal', $3, $4, $5, $6,
                   $7::TEXT::NUMERIC, $7::TEXT::NUMERIC, 'v1', 'near_ai_usd')",
        &[
            &tenant,
            &Uuid::new_v4(),
            batch,
            line,
            &Uuid::new_v4(),
            &account,
            &units,
        ],
    )
    .await;
}

/// The referrer's bonus rows for one referee: `(kind, bps, base, bonus)`.
async fn bonus_rows(
    fixture: &Fixture,
    referrer: &Referrer,
    referee: &Referee,
) -> Vec<(String, i32, String, String)> {
    runtime_query(
        fixture,
        &referrer.tenant,
        "SELECT kind, bps, base_units::TEXT, bonus_units::TEXT
           FROM trace_referral_bonus_accruals
          WHERE referee_ref_hash = $1
          ORDER BY created_at, bonus_id",
        &[&referee.referee_ref],
    )
    .await
    .iter()
    .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
    .collect()
}

async fn open_holds(fixture: &Fixture, tenant: &str, account: Uuid) -> Vec<String> {
    fixture
        .runtime
        .open_payout_hold_labels(tenant, account, &near_ai())
        .await
        .unwrap()
}

#[tokio::test]
async fn the_pass_writes_a_tenth_of_each_credit_in_the_referrers_tenant_once() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 10, 1).await;
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        1_234_567,
        Utc::now() - Duration::days(2),
    )
    .await;
    let hook = hook(&fixture, settings());

    let report = hook.run_tenant(&referee.tenant, &near_ai()).await;
    assert_eq!(report.referees, 1, "{report:?}");
    assert_eq!(report.bonus_rows, 1, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(
        bonus_rows(&fixture, &referrer, &referee).await,
        vec![("bonus".into(), 1000, "1234567".into(), "123456".into())]
    );
    // The referee's tenant never sees the referrer's rows.
    let seen = runtime_query(
        &fixture,
        &referee.tenant,
        "SELECT count(*) FROM trace_referral_bonus_accruals",
        &[],
    )
    .await[0]
        .get::<_, i64>(0);
    assert_eq!(seen, 0);
    // The mirror row was refreshed.
    let refreshed: bool = runtime_query(
        &fixture,
        &referrer.tenant,
        "SELECT signals_updated_at IS NOT NULL FROM trace_referral_referees
          WHERE referee_ref_hash = $1",
        &[&referee.referee_ref],
    )
    .await[0]
        .get(0);
    assert!(refreshed);

    // A rerun writes nothing new.
    let again = hook.run_tenant(&referee.tenant, &near_ai()).await;
    assert_eq!(again.bonus_rows, 0, "{again:?}");
    assert_eq!(bonus_rows(&fixture, &referrer, &referee).await.len(), 1);
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!(owed.owed, 123_456);
    assert_eq!(owed.sent, 0);

    // Disabled, or another rail: nothing runs.
    let disabled = self::hook(&fixture, ReferralBonusSettings::default());
    assert_eq!(
        disabled
            .run_tenant(&referee.tenant, &near_ai())
            .await
            .referees,
        0
    );
    let other = RailId::new("test_keyed").unwrap();
    assert_eq!(hook.run_tenant(&referee.tenant, &other).await.referees, 0);
}

#[tokio::test]
async fn only_batches_inside_the_window_earn_and_raising_the_days_extends_it() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    // Created 120 days ago, applied 5 days after: the 92-day window ended
    // 28 days ago.
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 120, 5).await;
    let created = Utc::now() - Duration::days(120);
    // Before the code was applied.
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        1_000,
        created + Duration::days(2),
    )
    .await;
    // Inside the window.
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        2_000,
        created + Duration::days(30),
    )
    .await;
    // After 92 days.
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        4_000,
        Utc::now() - Duration::days(1),
    )
    .await;

    let report = hook(&fixture, settings())
        .run_tenant(&referee.tenant, &near_ai())
        .await;
    assert_eq!(report.bonus_rows, 1, "{report:?}");
    assert_eq!(report.skipped["referral_outside_window"], 2);
    assert_eq!(
        bonus_rows(&fixture, &referrer, &referee).await,
        vec![("bonus".into(), 1000, "2000".into(), "200".into())]
    );

    let extended = hook(
        &fixture,
        ReferralBonusSettings {
            earn_days: 180,
            ..settings()
        },
    );
    let report = extended.run_tenant(&referee.tenant, &near_ai()).await;
    assert_eq!(report.bonus_rows, 1, "{report:?}");
    let rows = bonus_rows(&fixture, &referrer, &referee).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], ("bonus".into(), 1000, "4000".into(), "400".into()));
}

#[tokio::test]
async fn a_reversal_is_mirrored_only_when_its_credit_earned_a_bonus() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 30, 10).await;
    let created = Utc::now() - Duration::days(30);
    let before = credit(
        &fixture,
        &referee.tenant,
        referee.account,
        1_000,
        created + Duration::days(1),
    )
    .await;
    let inside = credit(
        &fixture,
        &referee.tenant,
        referee.account,
        1_000,
        created + Duration::days(20),
    )
    .await;
    reversal(&fixture, &referee.tenant, referee.account, &before, 500).await;
    reversal(&fixture, &referee.tenant, referee.account, &inside, 500).await;

    let report = hook(&fixture, settings())
        .run_tenant(&referee.tenant, &near_ai())
        .await;
    assert_eq!(report.bonus_rows, 1, "{report:?}");
    assert_eq!(report.reversal_rows, 1, "{report:?}");
    assert_eq!(report.skipped["referral_reversal_without_bonus"], 1);
    assert_eq!(
        bonus_rows(&fixture, &referrer, &referee).await,
        vec![
            ("bonus".into(), 1000, "1000".into(), "100".into()),
            ("reversal".into(), 1000, "-500".into(), "-50".into()),
        ]
    );
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!(owed.owed, 50);
}

#[tokio::test]
async fn a_held_referee_earns_no_new_bonus_until_the_hold_clears() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 10, 1).await;
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        1_000,
        Utc::now() - Duration::days(2),
    )
    .await;
    let hold = Uuid::new_v4();
    let reference = format!("account:{}", referee.account);
    owner_exec(
        &fixture,
        &referee.tenant,
        "INSERT INTO trace_credit_holds (
             tenant_id, hold_id, credit_account_ref, credit_account_hash, reason,
             reason_hash, actor_principal_ref
         ) VALUES ($1, $2, $3, $4, 'test', $5, 'operator')",
        &[
            &referee.tenant,
            &hold,
            &reference,
            &sha(&reference),
            &sha("reason"),
        ],
    )
    .await;
    let hook = hook(&fixture, settings());
    let report = hook.run_tenant(&referee.tenant, &near_ai()).await;
    assert_eq!(report.bonus_rows, 0, "{report:?}");
    assert_eq!(report.skipped["referral_referee_held"], 1);
    // The credit hold is also the referee's signal.
    let labels: Vec<String> = runtime_query(
        &fixture,
        &referrer.tenant,
        "SELECT signal_labels FROM trace_referral_referees WHERE referee_ref_hash = $1",
        &[&referee.referee_ref],
    )
    .await[0]
        .get(0);
    assert_eq!(labels, vec!["referral_credit_hold_share".to_owned()]);

    owner_exec(
        &fixture,
        &referee.tenant,
        "UPDATE trace_credit_holds SET released_at = now() WHERE tenant_id = $1 AND hold_id = $2",
        &[&referee.tenant, &hold],
    )
    .await;
    let report = hook.run_tenant(&referee.tenant, &near_ai()).await;
    assert_eq!(report.bonus_rows, 1, "{report:?}");
}

#[tokio::test]
async fn a_missing_referrer_voids_the_attribution() {
    let Some(fixture) = fixture().await else {
        return;
    };
    // An attribution naming a referrer tenant that does not exist (deleted,
    // say): its referrer tenant id is a bare label with no foreign key.
    let created = Utc::now() - Duration::days(10);
    let (tenant, account) = tenant_with_account(&fixture, created).await;
    let referee = Referee {
        referee_ref: sha(&format!("tc-referral-referee-v1:{tenant}:{account}")),
        tenant,
        account,
    };
    owner_exec(
        &fixture,
        &referee.tenant,
        "INSERT INTO trace_referral_attributions
             (tenant_id, account_id, code_index, referrer_tenant_id, referrer_account_id,
              referee_ref_hash, referee_account_created_at, applied_via, applied_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'later', $8)",
        &[
            &referee.tenant,
            &referee.account,
            &sha("code:gone"),
            &format!("nearai-gone-{}", Uuid::new_v4().simple()),
            &Uuid::new_v4(),
            &referee.referee_ref,
            &created,
            &(created + Duration::days(1)),
        ],
    )
    .await;
    let report = hook(&fixture, settings())
        .run_tenant(&referee.tenant, &near_ai())
        .await;
    assert_eq!(report.voided, 1, "{report:?}");
    let row = &runtime_query(
        &fixture,
        &referee.tenant,
        "SELECT voided_at IS NOT NULL, void_label FROM trace_referral_attributions",
        &[],
    )
    .await[0];
    assert!(row.get::<_, bool>(0));
    assert_eq!(row.get::<_, String>(1), "referral_referrer_gone");
    let audit: i64 = runtime_query(
        &fixture,
        &referee.tenant,
        "SELECT count(*) FROM trace_referral_audit
          WHERE action = 'referral_voided' AND label = 'referral_referrer_gone'",
        &[],
    )
    .await[0]
        .get(0);
    assert_eq!(audit, 1);
}

/// A live `near_ai` link for `account` with the given indexes.
async fn link(fixture: &Fixture, tenant: &str, account: Uuid, recipient: &str, subject: &str) {
    let link = Uuid::new_v4();
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_payout_links
           (tenant_id, account_id, rail_id, link_id, recipient_kind, state, sealed_recipient,
            recipient_index, recipient_key_ref, index_pepper_ref, subject_index, program)
         VALUES ($1, $2, 'near_ai', $3, 'near_ai_org', 'active',
                 '{\"v\":1}'::jsonb, $4, 'key-test-1', 'pepper-test-1', $5, $6)",
        &[
            &tenant,
            &account,
            &link,
            &sha(recipient),
            &sha(subject),
            &format!("tc1-{}", &sha(&link.to_string())[7..39]),
        ],
    )
    .await;
}

#[tokio::test]
async fn sharing_a_payout_org_or_subject_holds_only_the_referral_stream() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 10, 1).await;
    let org = format!("org:{}", Uuid::new_v4());
    let subject = format!("subject:{}", Uuid::new_v4());
    link(&fixture, &referrer.tenant, referrer.account, &org, &subject).await;
    link(&fixture, &referee.tenant, referee.account, &org, &subject).await;
    let report = hook(&fixture, settings())
        .run_tenant(&referee.tenant, &near_ai())
        .await;
    assert_eq!(report.held["referral_same_payout_org"], 1, "{report:?}");
    assert_eq!(report.held["referral_same_payout_subject"], 1, "{report:?}");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec![
            "referral_same_payout_org".to_owned(),
            "referral_same_payout_subject".to_owned()
        ]
    );
    // The matches are evidence on the referee's mirror row.
    assert_eq!(
        mirror_labels(&fixture, &referrer, &referee).await,
        vec![
            "referral_same_payout_org".to_owned(),
            "referral_same_payout_subject".to_owned()
        ]
    );
    // The referee's mirror row carries their link's indexes.
    let row = &runtime_query(
        &fixture,
        &referrer.tenant,
        "SELECT recipient_index, subject_index FROM trace_referral_referees
          WHERE referee_ref_hash = $1",
        &[&referee.referee_ref],
    )
    .await[0];
    assert_eq!(row.get::<_, Option<String>>(0), Some(sha(&org)));
    assert_eq!(row.get::<_, Option<String>>(1), Some(sha(&subject)));
    // The referee is not held.
    assert!(
        open_holds(&fixture, &referee.tenant, referee.account)
            .await
            .is_empty()
    );
    for label in open_holds(&fixture, &referrer.tenant, referrer.account).await {
        assert!(
            trace_commons_server::referral_abuse::hold_holds_referral_stream_only(&label),
            "{label}"
        );
    }
}

/// A referrer whose code has five referees, `bad` of them with a reversal
/// (the credit-hold signal). Runs the pass in each referee's tenant.
async fn scored_code(fixture: &Fixture, hook: &ReferralBonusHook, bad: usize) -> Referrer {
    let referrer = referrer(fixture).await;
    let code_index = referrer.code_index.clone();
    let mut referees = Vec::new();
    for _ in 0..5 {
        referees.push(referee(fixture, &referrer, &code_index, 40, 1).await);
    }
    for referee in referees.iter().take(bad) {
        let line = credit(
            fixture,
            &referee.tenant,
            referee.account,
            1_000,
            Utc::now() - Duration::days(20),
        )
        .await;
        reversal(fixture, &referee.tenant, referee.account, &line, 1_000).await;
    }
    for referee in &referees {
        hook.run_tenant(&referee.tenant, &near_ai()).await;
    }
    referrer
}

#[tokio::test]
async fn a_code_over_a_threshold_is_suspended_and_holds_its_referrer() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let hook = hook(&fixture, settings());
    // 1 of 5 is 20%: not over.
    let fine = scored_code(&fixture, &hook, 1).await;
    let codes = fixture.runtime.referral_codes(&fine.tenant).await.unwrap();
    assert!(codes[0].suspended_at.is_none());
    assert!(
        open_holds(&fixture, &fine.tenant, fine.account)
            .await
            .is_empty()
    );

    // 2 of 5 is 40%: suspended with the label, and held.
    let bad = scored_code(&fixture, &hook, 2).await;
    let codes = fixture.runtime.referral_codes(&bad.tenant).await.unwrap();
    assert!(codes[0].suspended_at.is_some());
    assert_eq!(
        codes[0].suspension_labels,
        vec!["referral_credit_hold_share".to_owned()]
    );
    let holds = open_holds(&fixture, &bad.tenant, bad.account).await;
    assert!(
        holds.contains(&"referral_code_suspended".to_owned()),
        "{holds:?}"
    );
    assert!(
        holds.contains(&"referral_referrer_suspended".to_owned()),
        "the only code's rates are the referrer's: {holds:?}"
    );
    let audit: i64 = runtime_query(
        &fixture,
        &bad.tenant,
        "SELECT count(*) FROM trace_referral_audit
          WHERE action = 'code_suspended' AND code_index = $1
            AND label = 'referral_credit_hold_share'",
        &[&bad.code_index],
    )
    .await[0]
        .get(0);
    assert_eq!(audit, 1);
    // The suspended code is visible to the deployment-wide reads.
    let suspended = fixture
        .runtime
        .referral_suspended_code_tenants()
        .await
        .unwrap();
    assert!(suspended.contains(&(bad.tenant.clone(), bad.code_index.clone())));
    let listed = fixture
        .runtime
        .referral_suspended_codes(&bad.tenant, &near_ai())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].referees, 5);

    // Cleared with the label exempted, it does not trip again.
    let owner = fixture
        .runtime
        .resume_referral_code(
            &bad.tenant,
            &bad.code_index,
            &["referral_credit_hold_share".to_owned()],
            ACTOR,
        )
        .await
        .unwrap();
    assert_eq!(owner, Some(bad.account));
    let codes = fixture.runtime.referral_codes(&bad.tenant).await.unwrap();
    assert!(
        codes[0]
            .exempt_labels
            .contains("referral_credit_hold_share")
    );
    let mirror = fixture
        .runtime
        .referral_mirror_rows(&bad.tenant, bad.account)
        .await
        .unwrap();
    let rerun = trace_commons_server::referral_abuse::score_code(
        &bad.code_index,
        &mirror,
        &settings().abuse,
        &codes[0].exempt_labels,
    );
    assert!(!rerun.is_tripped());
}

#[tokio::test]
async fn more_than_two_suspensions_in_a_day_trip_the_stream_to_dry_run() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let control = near_ai_referral_control_rail();
    fixture
        .runtime
        .set_payout_control(&control, false, Some("test_reset"), ACTOR)
        .await
        .unwrap();
    let hook = hook(&fixture, settings());
    for _ in 0..3 {
        scored_code(&fixture, &hook, 3).await;
    }
    let state = fixture.runtime.payout_control(&control).await.unwrap();
    assert!(state.halted);
    assert_eq!(state.label.as_deref(), Some("referral_tripped"));
    assert_eq!(hook.tripped(), Some(true));
    let since = Utc::now() - Duration::hours(24);
    assert!(
        fixture
            .runtime
            .referral_codes_suspended_since(since)
            .await
            .unwrap()
            >= 3
    );
    // The earned rail's own kill switch is untouched.
    assert!(
        !fixture
            .runtime
            .payout_control(&near_ai())
            .await
            .unwrap()
            .halted
    );
}

#[tokio::test]
async fn a_void_takes_back_only_unsent_bonus_and_stops_new_bonus() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let code_index = referrer.code_index.clone();
    let first = referee(&fixture, &referrer, &code_index, 10, 1).await;
    let second = referee(&fixture, &referrer, &code_index, 10, 1).await;
    for referee in [&first, &second] {
        credit(
            &fixture,
            &referee.tenant,
            referee.account,
            1_000,
            Utc::now() - Duration::days(2),
        )
        .await;
    }
    let hook = hook(&fixture, settings());
    for referee in [&first, &second] {
        hook.run_tenant(&referee.tenant, &near_ai()).await;
    }
    // 200 owed; 150 already sent on the referral stream.
    let link = Uuid::new_v4();
    owner_exec(
        &fixture,
        &referrer.tenant,
        "INSERT INTO trace_payout_links
           (tenant_id, account_id, rail_id, link_id, recipient_kind, state, sealed_recipient,
            recipient_index, recipient_key_ref, index_pepper_ref, subject_index, program)
         VALUES ($1, $2, 'near_ai', $3, 'near_ai_org', 'active',
                 '{\"v\":1}'::jsonb, $4, 'key-test-1', 'pepper-test-1', $5, $6)",
        &[
            &referrer.tenant,
            &referrer.account,
            &link,
            &sha(&format!("index:{link}")),
            &sha(&format!("subject:{link}")),
            &format!("tc1-{}", &sha(&link.to_string())[7..39]),
        ],
    )
    .await;
    owner_exec(
        &fixture,
        &referrer.tenant,
        "INSERT INTO trace_payout_lines
           (tenant_id, line_id, account_id, rail_id, link_id, target_total, delta,
            owed_through_accrual_count, state, stream, receipt_hash, resolved_at)
         VALUES ($1, $2, $3, 'near_ai', $4, 150, 150, 2, 'confirmed', 'referral', $5, now())",
        &[
            &referrer.tenant,
            &Uuid::new_v4(),
            &referrer.account,
            &link,
            &sha(&format!("receipt:{link}")),
        ],
    )
    .await;
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!((owed.owed, owed.sent, owed.unsent()), (200, 150, 50));

    let outcome = fixture
        .runtime
        .void_referral_code(
            &referrer.tenant,
            &code_index,
            "operator_void",
            &near_ai(),
            ACTOR,
        )
        .await
        .unwrap()
        .expect("a live code is voided");
    assert_eq!(outcome.referees_voided, 2);
    assert_eq!(outcome.taken_back, 50, "never below what was sent");
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!(owed.unsent(), 0);
    assert!(
        fixture
            .runtime
            .void_referral_code(
                &referrer.tenant,
                &code_index,
                "operator_void",
                &near_ai(),
                ACTOR
            )
            .await
            .unwrap()
            .is_none(),
        "a void is once"
    );

    // The pass carries the void to each attribution and prices nothing new.
    credit(
        &fixture,
        &first.tenant,
        first.account,
        5_000,
        Utc::now() - Duration::days(1),
    )
    .await;
    let report = hook.run_tenant(&first.tenant, &near_ai()).await;
    assert_eq!(report.voided, 1, "{report:?}");
    assert_eq!(report.bonus_rows, 0, "{report:?}");
    let label: String = runtime_query(
        &fixture,
        &first.tenant,
        "SELECT void_label FROM trace_referral_attributions",
        &[],
    )
    .await[0]
        .get(0);
    assert_eq!(label, "referral_voided_by_operator");
    let again = hook.run_tenant(&first.tenant, &near_ai()).await;
    assert_eq!((again.voided, again.bonus_rows), (0, 0), "{again:?}");
}

/// Records `units` sent on the referrer's referral stream: an active link
/// and one confirmed referral line.
async fn sent_on_referral_stream(fixture: &Fixture, referrer: &Referrer, units: i64) {
    let link = Uuid::new_v4();
    owner_exec(
        fixture,
        &referrer.tenant,
        "INSERT INTO trace_payout_links
           (tenant_id, account_id, rail_id, link_id, recipient_kind, state, sealed_recipient,
            recipient_index, recipient_key_ref, index_pepper_ref, subject_index, program)
         VALUES ($1, $2, 'near_ai', $3, 'near_ai_org', 'active',
                 '{\"v\":1}'::jsonb, $4, 'key-test-1', 'pepper-test-1', $5, $6)",
        &[
            &referrer.tenant,
            &referrer.account,
            &link,
            &sha(&format!("index:{link}")),
            &sha(&format!("subject:{link}")),
            &format!("tc1-{}", &sha(&link.to_string())[7..39]),
        ],
    )
    .await;
    let units = units.to_string();
    owner_exec(
        fixture,
        &referrer.tenant,
        "INSERT INTO trace_payout_lines
           (tenant_id, line_id, account_id, rail_id, link_id, target_total, delta,
            owed_through_accrual_count, state, stream, receipt_hash, resolved_at)
         VALUES ($1, $2, $3, 'near_ai', $4, $5::TEXT::NUMERIC, $5::TEXT::NUMERIC, 1,
                 'confirmed', 'referral', $6, now())",
        &[
            &referrer.tenant,
            &Uuid::new_v4(),
            &referrer.account,
            &link,
            &units,
            &sha(&format!("receipt:{link}")),
        ],
    )
    .await;
}

/// A referrer whose first referee earned 100 and was sent 60 of it, and
/// whose second referee then earned 100 that nothing has sent.
async fn one_partly_sent_one_unsent(
    fixture: &Fixture,
    hook: &ReferralBonusHook,
) -> (Referrer, Referee, Referee) {
    let referrer = referrer(fixture).await;
    let code_index = referrer.code_index.clone();
    let partly = referee(fixture, &referrer, &code_index, 10, 1).await;
    let unsent = referee(fixture, &referrer, &code_index, 10, 1).await;
    credit(
        fixture,
        &partly.tenant,
        partly.account,
        1_000,
        Utc::now() - Duration::days(3),
    )
    .await;
    hook.run_tenant(&partly.tenant, &near_ai()).await;
    sent_on_referral_stream(fixture, &referrer, 60).await;
    credit(
        fixture,
        &unsent.tenant,
        unsent.account,
        1_000,
        Utc::now() - Duration::days(2),
    )
    .await;
    hook.run_tenant(&unsent.tenant, &near_ai()).await;
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!((owed.owed, owed.sent, owed.unsent()), (200, 60, 140));
    (referrer, partly, unsent)
}

async fn void_one(fixture: &Fixture, referrer: &Referrer, referee: &Referee) -> u128 {
    fixture
        .runtime
        .void_referral_referee(
            &referrer.tenant,
            &referee.referee_ref,
            "operator_void",
            &near_ai(),
            ACTOR,
        )
        .await
        .unwrap()
        .expect("a live referee is voided")
        .taken_back
}

/// What was sent is allotted to the referee who earned first, so a void
/// takes back only the voided referee's unsent bonus: never what was sent
/// for them, and never another referee's unsent bonus.
#[tokio::test]
async fn a_void_takes_back_only_that_referees_unsent_bonus() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let hook = hook(&fixture, settings());

    // Voiding the referee nothing was sent for takes back all of theirs,
    // and leaves the partly-sent referee's 40 owed.
    let (referrer, partly, unsent) = one_partly_sent_one_unsent(&fixture, &hook).await;
    assert_eq!(void_one(&fixture, &referrer, &unsent).await, 100);
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!(owed.unsent(), 40);
    assert_eq!(void_one(&fixture, &referrer, &partly).await, 40);

    // The reverse: voiding the partly-sent referee takes back only their
    // unsent 40, not the other referee's 100.
    let (referrer, partly, unsent) = one_partly_sent_one_unsent(&fixture, &hook).await;
    assert_eq!(void_one(&fixture, &referrer, &partly).await, 40);
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!(
        (owed.owed, owed.unsent()),
        (160, 100),
        "the other referee's bonus is still owed"
    );
    assert_eq!(void_one(&fixture, &referrer, &unsent).await, 100);
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!((owed.owed, owed.unsent()), (60, 0), "what was sent stays");
}

/// A bonus row planned from a view read before an operator void commits
/// is dropped when it reaches the store after it: the void took back only
/// the bonus that existed, and nothing later takes back a row written
/// behind it.
#[tokio::test]
async fn bonus_planned_before_a_void_is_not_written_after_it() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let code_index = referrer.code_index.clone();
    let referee = referee(&fixture, &referrer, &code_index, 10, 1).await;
    let planned = |bonus_units: u128| ReferralBonusWrite {
        rail_id: near_ai(),
        account_id: referrer.account,
        referee_ref_hash: referee.referee_ref.clone(),
        kind: ReferralBonusKind::Bonus,
        source_accrual_id: Some(Uuid::new_v4()),
        bps: 1_000,
        base_units: bonus_units * 10,
        bonus_units,
        rate_version: "v1".to_owned(),
        instrument_id: InstrumentId::new("near_ai_usd").unwrap(),
    };
    // Before the void, a planned row lands.
    assert_eq!(
        fixture
            .runtime
            .insert_referral_bonus_rows(&referrer.tenant, &[planned(100)])
            .await
            .unwrap(),
        1
    );
    let outcome = fixture
        .runtime
        .void_referral_referee(
            &referrer.tenant,
            &referee.referee_ref,
            "operator_void",
            &near_ai(),
            ACTOR,
        )
        .await
        .unwrap()
        .expect("a live referee is voided");
    assert_eq!(outcome.taken_back, 100);
    // A row planned before the void, arriving after it, is dropped.
    assert_eq!(
        fixture
            .runtime
            .insert_referral_bonus_rows(&referrer.tenant, &[planned(500)])
            .await
            .unwrap(),
        0
    );
    let kinds: Vec<String> = bonus_rows(&fixture, &referrer, &referee)
        .await
        .into_iter()
        .map(|(kind, ..)| kind)
        .collect();
    assert_eq!(kinds, vec!["bonus".to_owned(), "void".to_owned()]);
    let owed = fixture
        .runtime
        .referral_bonus_owed(&referrer.tenant, referrer.account, &near_ai())
        .await
        .unwrap();
    assert_eq!(owed.unsent(), 0, "nothing is owed after the void");
}

#[tokio::test]
async fn the_tenant_report_splits_owed_into_sendable_and_held() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 10, 1).await;
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        10_000,
        Utc::now() - Duration::days(2),
    )
    .await;
    let hook = hook(&fixture, settings());
    hook.run_tenant(&referee.tenant, &near_ai()).await;
    let report = fixture
        .runtime
        .referral_tenant_report(&referrer.tenant, &near_ai())
        .await
        .unwrap();
    assert_eq!(report.codes_by_state.get("live"), Some(&1));
    assert_eq!(report.referees, 1);
    assert_eq!(
        (report.owed, report.sendable, report.held),
        (1_000, 1_000, 0)
    );
    fixture
        .runtime
        .open_payout_hold(
            &referrer.tenant,
            referrer.account,
            &near_ai(),
            "referral_over_ceiling",
            ACTOR,
        )
        .await
        .unwrap();
    let report = fixture
        .runtime
        .referral_tenant_report(&referrer.tenant, &near_ai())
        .await
        .unwrap();
    assert_eq!(
        (report.owed, report.sendable, report.held),
        (1_000, 0, 1_000)
    );
    assert!(
        fixture
            .runtime
            .referral_code_tenants()
            .await
            .unwrap()
            .contains(&referrer.tenant)
    );
}

#[tokio::test]
async fn the_ceiling_hold_opens_over_it_and_lifts_once_it_fits() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 10, 1).await;
    credit(
        &fixture,
        &referee.tenant,
        referee.account,
        10_000,
        Utc::now() - Duration::days(2),
    )
    .await;
    let tight = hook(
        &fixture,
        ReferralBonusSettings {
            account_ceiling: 999,
            ..settings()
        },
    );
    let report = tight.run_tenant(&referee.tenant, &near_ai()).await;
    assert_eq!(report.held["referral_over_ceiling"], 1, "{report:?}");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec!["referral_over_ceiling".to_owned()]
    );
    hook(&fixture, settings())
        .run_tenant(&referee.tenant, &near_ai())
        .await;
    assert!(
        open_holds(&fixture, &referrer.tenant, referrer.account)
            .await
            .is_empty()
    );
}

// -- Server-observed signals (owner decision 2026-10-29) ---------------------
//
// The client is open source, so nothing it sends counts. The pass compares
// what the server observed itself, each side read under its own tenant:
// near-duplicate trace clusters and keyed network marks.

/// One accepted submission by `account` (through a principal it owns) with
/// a gate decision in duplicate cluster `cluster`.
async fn trace_in_cluster(fixture: &Fixture, tenant: &str, account: Uuid, cluster: Uuid) {
    let principal = format!("principal:{}", Uuid::new_v4());
    let submission = Uuid::new_v4();
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
         VALUES ($1, $2, $3)",
        &[&tenant, &account, &principal],
    )
    .await;
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_submissions (tenant_id, submission_id, trace_id, auth_principal_ref,
             schema_version, consent_policy_version, retention_policy_id, status,
             privacy_risk, redaction_pipeline_version, redaction_hash)
         VALUES ($1, $2, $3, $4, 'v1', 'v1', 'test', 'accepted', 'low', 'test', $5)",
        &[
            &tenant,
            &submission,
            &Uuid::new_v4(),
            &principal,
            &"a".repeat(64),
        ],
    )
    .await;
    owner_exec(
        fixture,
        tenant,
        "INSERT INTO trace_gate_decisions (tenant_id, decision_id, submission_id,
             gate_policy_version, gate_version_hash, perplexity_micros, tail_fraction_micros,
             perplexity_passed, novelty_score_micros, nearest_neighbor_hash, novelty_passed,
             embedding_evidence_hash, attestation_chain_hash, dedup_cluster_id,
             dedup_signal_version, credit_quality_micros, credit_quality_calibration_version,
             decided_at)
         VALUES ($1, $2, $3, 'gate-fixture', $4, 1, 1, TRUE, 1, $4, TRUE, $4, $4, $5,
                 'signal-fixture', 200, 1, clock_timestamp())",
        &[
            &tenant,
            &Uuid::new_v4(),
            &submission,
            &"a".repeat(64),
            &cluster,
        ],
    )
    .await;
}

async fn mirror_labels(fixture: &Fixture, referrer: &Referrer, referee: &Referee) -> Vec<String> {
    runtime_query(
        fixture,
        &referrer.tenant,
        "SELECT signal_labels FROM trace_referral_referees WHERE referee_ref_hash = $1",
        &[&referee.referee_ref],
    )
    .await[0]
        .get(0)
}

fn mark_key() -> trace_commons_server::referral_network::ReferralNetworkMarkKey {
    trace_commons_server::referral_network::ReferralNetworkMarkKey::from_bytes(&[3u8; 32]).unwrap()
}

/// Records the mark of `address` for `account` through the runtime store.
async fn seen_at(fixture: &Fixture, tenant: &str, account: Uuid, address: &str) -> bool {
    use trace_commons_server::db::ReferralStore;
    let mark = mark_key()
        .mark_for_address(address.parse().unwrap())
        .expect("a public address");
    fixture
        .runtime
        .record_referral_network_mark(tenant, account, &mark, 122)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_shared_trace_cluster_holds_the_referrer_and_labels_the_referee() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let code_index = referrer.code_index.clone();
    let copier = referee(&fixture, &referrer, &code_index, 10, 1).await;
    let honest = referee(&fixture, &referrer, &code_index, 10, 1).await;
    let shared = Uuid::new_v4();
    trace_in_cluster(&fixture, &referrer.tenant, referrer.account, shared).await;
    trace_in_cluster(&fixture, &copier.tenant, copier.account, shared).await;
    trace_in_cluster(&fixture, &honest.tenant, honest.account, Uuid::new_v4()).await;

    let hook = hook(&fixture, settings());
    let report = hook.run_tenant(&honest.tenant, &near_ai()).await;
    assert!(report.held.is_empty(), "{report:?}");
    assert!(
        !mirror_labels(&fixture, &referrer, &honest)
            .await
            .contains(&"referral_duplicate_traces".to_owned())
    );

    let report = hook.run_tenant(&copier.tenant, &near_ai()).await;
    assert_eq!(report.held["referral_same_traces"], 1, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec!["referral_same_traces".to_owned()]
    );
    assert_eq!(
        mirror_labels(&fixture, &referrer, &copier).await,
        vec!["referral_duplicate_traces".to_owned()]
    );
    // A hold, not a void: the referral stands, and the referee is not held.
    let voided: bool = runtime_query(
        &fixture,
        &copier.tenant,
        "SELECT voided_at IS NOT NULL FROM trace_referral_attributions",
        &[],
    )
    .await[0]
        .get(0);
    assert!(!voided);
    assert!(
        open_holds(&fixture, &copier.tenant, copier.account)
            .await
            .is_empty()
    );

    // An operator who clears the hold is not overruled by the next pass.
    fixture
        .runtime
        .clear_payout_hold(
            &referrer.tenant,
            referrer.account,
            &near_ai(),
            "referral_same_traces",
            ACTOR,
        )
        .await
        .unwrap();
    let report = hook.run_tenant(&copier.tenant, &near_ai()).await;
    assert!(report.held.is_empty(), "{report:?}");
    assert!(
        open_holds(&fixture, &referrer.tenant, referrer.account)
            .await
            .is_empty()
    );
    assert_eq!(
        mirror_labels(&fixture, &referrer, &copier).await,
        vec!["referral_duplicate_traces".to_owned()],
        "the evidence stays on the mirror"
    );
}

#[tokio::test]
async fn a_shared_network_mark_holds_the_referrer_and_labels_the_referee() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let code_index = referrer.code_index.clone();
    let neighbour = referee(&fixture, &referrer, &code_index, 10, 1).await;
    let stranger = referee(&fixture, &referrer, &code_index, 10, 1).await;
    // One /24, two hosts.
    assert!(seen_at(&fixture, &referrer.tenant, referrer.account, "203.0.113.10").await);
    assert!(
        seen_at(
            &fixture,
            &neighbour.tenant,
            neighbour.account,
            "203.0.113.200"
        )
        .await
    );
    assert!(seen_at(&fixture, &stranger.tenant, stranger.account, "198.51.100.7").await);

    let hook = hook(&fixture, settings());
    let report = hook.run_tenant(&stranger.tenant, &near_ai()).await;
    assert!(report.held.is_empty(), "{report:?}");
    let report = hook.run_tenant(&neighbour.tenant, &near_ai()).await;
    assert_eq!(report.held["referral_same_network"], 1, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec!["referral_same_network".to_owned()]
    );
    assert_eq!(
        mirror_labels(&fixture, &referrer, &neighbour).await,
        vec!["referral_same_network".to_owned()]
    );
    assert!(
        mirror_labels(&fixture, &referrer, &stranger)
            .await
            .is_empty()
    );
}

/// Colleagues on one office network are the main legitimate case: however
/// many of a code's referees share the referrer's network, the code is
/// never suspended for it. The referral stream is held for an operator.
#[tokio::test]
async fn a_shared_network_never_suspends_a_code() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let code_index = referrer.code_index.clone();
    assert!(seen_at(&fixture, &referrer.tenant, referrer.account, "203.0.113.1").await);
    let mut referees = Vec::new();
    for host in 2..8 {
        let referee = referee(&fixture, &referrer, &code_index, 40, 1).await;
        assert!(
            seen_at(
                &fixture,
                &referee.tenant,
                referee.account,
                &format!("203.0.113.{host}")
            )
            .await
        );
        referees.push(referee);
    }
    let hook = hook(&fixture, settings());
    for referee in &referees {
        let report = hook.run_tenant(&referee.tenant, &near_ai()).await;
        assert_eq!(report.suspended, 0, "{report:?}");
    }
    for referee in &referees {
        assert!(
            mirror_labels(&fixture, &referrer, referee)
                .await
                .contains(&"referral_same_network".to_owned())
        );
    }
    let codes = fixture
        .runtime
        .referral_codes(&referrer.tenant)
        .await
        .unwrap();
    assert!(codes[0].suspended_at.is_none(), "never suspended");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec!["referral_same_network".to_owned()]
    );
}

#[tokio::test]
async fn network_marks_stay_in_their_tenant_and_program_and_expire() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let referee = referee(&fixture, &referrer, &referrer.code_index.clone(), 10, 1).await;
    // An account outside the program leaves no mark.
    let (outsider_tenant, outsider) =
        tenant_with_account(&fixture, Utc::now() - Duration::days(10)).await;
    assert!(!seen_at(&fixture, &outsider_tenant, outsider, "203.0.113.9").await);
    assert!(
        fixture
            .runtime
            .referral_network_marks(&outsider_tenant, outsider, 122, 10)
            .await
            .unwrap()
            .is_empty()
    );
    // A program account's mark is recorded once and refreshed at most
    // hourly.
    assert!(seen_at(&fixture, &referee.tenant, referee.account, "203.0.113.9").await);
    assert!(!seen_at(&fixture, &referee.tenant, referee.account, "203.0.113.10").await);
    let marks = fixture
        .runtime
        .referral_network_marks(&referee.tenant, referee.account, 122, 10)
        .await
        .unwrap();
    assert_eq!(marks.len(), 1);
    assert!(marks[0].starts_with("sha256:") && !marks[0].contains("203"));
    // Under the referrer's tenant the referee's marks are invisible, even
    // named by the referee's account id: forced RLS.
    assert!(
        fixture
            .runtime
            .referral_network_marks(&referrer.tenant, referee.account, 122, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let seen: i64 = runtime_query(
        &fixture,
        &referrer.tenant,
        "SELECT count(*) FROM trace_referral_network_marks",
        &[],
    )
    .await[0]
        .get(0);
    assert_eq!(seen, 0);
    // Retention: a mark last seen past the earning period plus 30 days is
    // never read, and the pass deletes it.
    owner_exec(
        &fixture,
        &referee.tenant,
        "UPDATE trace_referral_network_marks
            SET first_seen_at = now() - interval '200 days',
                last_seen_at = now() - interval '123 days'",
        &[],
    )
    .await;
    assert!(
        fixture
            .runtime
            .referral_network_marks(&referee.tenant, referee.account, 122, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let report = hook(&fixture, settings())
        .run_tenant(&referee.tenant, &near_ai())
        .await;
    assert!(report.skipped.is_empty(), "{report:?}");
    let left: i64 = runtime_query(
        &fixture,
        &referee.tenant,
        "SELECT count(*) FROM trace_referral_network_marks",
        &[],
    )
    .await[0]
        .get(0);
    assert_eq!(left, 0, "pruned");
    // A tenant with nothing stale prunes nothing.
    assert_eq!(
        fixture
            .runtime
            .prune_referral_network_marks(&outsider_tenant, 122)
            .await
            .unwrap(),
        0
    );
}

/// The payout org and subject holds open once per referee, as the trace and
/// network holds do: an operator's clear stands on the next pass, and only a
/// new referee with the same payout org opens the hold again.
#[tokio::test]
async fn a_cleared_payout_org_hold_stays_cleared_until_a_new_referee_matches() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let referrer = referrer(&fixture).await;
    let code_index = referrer.code_index.clone();
    let first = referee(&fixture, &referrer, &code_index, 10, 1).await;
    let org = format!("org:{}", Uuid::new_v4());
    link(
        &fixture,
        &referrer.tenant,
        referrer.account,
        &org,
        &format!("subject:{}", Uuid::new_v4()),
    )
    .await;
    link(
        &fixture,
        &first.tenant,
        first.account,
        &org,
        &format!("subject:{}", Uuid::new_v4()),
    )
    .await;
    let hook = hook(&fixture, settings());
    let report = hook.run_tenant(&first.tenant, &near_ai()).await;
    assert_eq!(report.held["referral_same_payout_org"], 1, "{report:?}");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec!["referral_same_payout_org".to_owned()]
    );
    assert_eq!(
        mirror_labels(&fixture, &referrer, &first).await,
        vec!["referral_same_payout_org".to_owned()]
    );

    fixture
        .runtime
        .clear_payout_hold(
            &referrer.tenant,
            referrer.account,
            &near_ai(),
            "referral_same_payout_org",
            ACTOR,
        )
        .await
        .unwrap();
    let report = hook.run_tenant(&first.tenant, &near_ai()).await;
    assert!(report.held.is_empty(), "{report:?}");
    assert!(
        open_holds(&fixture, &referrer.tenant, referrer.account)
            .await
            .is_empty(),
        "the operator's clear stands"
    );
    assert_eq!(
        mirror_labels(&fixture, &referrer, &first).await,
        vec!["referral_same_payout_org".to_owned()],
        "the evidence stays on the mirror"
    );

    // A new referee on the same payout org opens it again.
    let second = referee(&fixture, &referrer, &code_index, 10, 1).await;
    link(
        &fixture,
        &second.tenant,
        second.account,
        &org,
        &format!("subject:{}", Uuid::new_v4()),
    )
    .await;
    let report = hook.run_tenant(&second.tenant, &near_ai()).await;
    assert_eq!(report.held["referral_same_payout_org"], 1, "{report:?}");
    assert_eq!(
        open_holds(&fixture, &referrer.tenant, referrer.account).await,
        vec!["referral_same_payout_org".to_owned()]
    );
    // Evidence only: the code is not suspended for it.
    let codes = fixture
        .runtime
        .referral_codes(&referrer.tenant)
        .await
        .unwrap();
    assert!(codes[0].suspended_at.is_none());
}

/// One more referee of `referrer`'s code with a reversal (the credit-hold
/// signal), and the pass in its tenant.
async fn bad_referee(fixture: &Fixture, hook: &ReferralBonusHook, referrer: &Referrer) -> Referee {
    let referee = referee(fixture, referrer, &referrer.code_index, 40, 1).await;
    let line = credit(
        fixture,
        &referee.tenant,
        referee.account,
        1_000,
        Utc::now() - Duration::days(20),
    )
    .await;
    reversal(fixture, &referee.tenant, referee.account, &line, 1_000).await;
    hook.run_tenant(&referee.tenant, &near_ai()).await;
    referee
}

/// The referrer-level hold opens on new evidence, as the trace, network
/// and payout org holds do, so an operator's clear is not overruled by the
/// next pass over the same referees. A new referee carrying a tripping
/// label opens it again, unless the operator exempted that label on the
/// referee's code.
#[tokio::test]
async fn a_cleared_referrer_hold_stays_cleared_until_the_evidence_grows() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let hook = hook(&fixture, settings());
    // 2 of 5 is 40%: the code and the referrer trip.
    let referrer = scored_code(&fixture, &hook, 2).await;
    let held = |holds: Vec<String>| holds.contains(&"referral_referrer_suspended".to_owned());
    assert!(held(
        open_holds(&fixture, &referrer.tenant, referrer.account).await
    ));
    let clear = || async {
        fixture
            .runtime
            .clear_payout_hold(
                &referrer.tenant,
                referrer.account,
                &near_ai(),
                "referral_referrer_suspended",
                ACTOR,
            )
            .await
            .unwrap();
    };
    clear().await;

    // Every referee visited again, nothing new: the clear stands.
    let referees: Vec<String> = runtime_query(
        &fixture,
        &referrer.tenant,
        "SELECT referee_ref_hash FROM trace_referral_referees WHERE tenant_id = $1",
        &[&referrer.tenant],
    )
    .await
    .iter()
    .map(|row| row.get(0))
    .collect();
    assert_eq!(referees.len(), 5);
    for tenant in referee_tenants(&fixture, &referrer).await {
        let report = hook.run_tenant(&tenant, &near_ai()).await;
        assert!(
            !report.held.contains_key("referral_referrer_suspended"),
            "{report:?}"
        );
    }
    assert!(
        !held(open_holds(&fixture, &referrer.tenant, referrer.account).await),
        "the operator's clear stands"
    );

    // A sixth referee with the same signal is new evidence.
    bad_referee(&fixture, &hook, &referrer).await;
    assert!(held(
        open_holds(&fixture, &referrer.tenant, referrer.account).await
    ));

    // Exempted on the code, the label is not evidence against the
    // referrer either.
    fixture
        .runtime
        .resume_referral_code(
            &referrer.tenant,
            &referrer.code_index,
            &["referral_credit_hold_share".to_owned()],
            ACTOR,
        )
        .await
        .unwrap();
    clear().await;
    bad_referee(&fixture, &hook, &referrer).await;
    assert!(!held(
        open_holds(&fixture, &referrer.tenant, referrer.account).await
    ));
}

/// The tenants of `referrer`'s referees, from their attributions.
async fn referee_tenants(fixture: &Fixture, referrer: &Referrer) -> Vec<String> {
    let client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    client
        .query(
            "SELECT tenant_id FROM trace_referral_attributions
              WHERE referrer_tenant_id = $1 AND referrer_account_id = $2",
            &[&referrer.tenant, &referrer.account],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}
