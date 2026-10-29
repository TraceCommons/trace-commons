// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral tables (V142) against a real PostgreSQL: forced RLS, the
//! one-referrer and one-code-index rules, the self-referral check, the
//! resolver's narrow cross-tenant read, append-only bonus and audit rows,
//! and the second payout stream on links and lines
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md).
//!
//! Fixtures are written by the migrator. Every check of what the service may
//! do runs as `trace_ingest_runtime` (NOBYPASSRLS), set as the session role
//! through the connection options, so a missing grant is a real refusal.
//!
//! Needs a database whose name starts with `admission_test_`, on the literal
//! host `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Without the
//! variable every test prints a skip line and returns.

use tokio_postgres::error::SqlState;
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, REFERRAL_RESOLVER_ASSUME_SQL, postgres::PgBackend},
};
use uuid::Uuid;

const REFERRAL_TABLES: [&str; 6] = [
    "trace_referral_codes",
    "trace_referral_attributions",
    "trace_referral_referees",
    "trace_referral_bonus_accruals",
    "trace_referral_audit",
    "trace_referral_network_marks",
];

struct Fixture {
    owner: PgBackend,
    runtime: PgBackend,
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
    // Bonus rows name a recorded rate epoch, as payout accruals do.
    owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(
            "INSERT INTO trace_payout_rate_epochs (rail_id, version, numerator, denominator)
             VALUES ('near_ai', 'v1', 1, 1) ON CONFLICT DO NOTHING;
             -- V90: the runtime's privileges on tables before V62 are the
             -- operator's. The apply reads the referee's account creation.
             GRANT SELECT ON trace_accounts TO trace_ingest_runtime;",
        )
        .await
        .expect("record the test rate epoch");

    let mut runtime_url = parsed.clone();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
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
    Some(Fixture { owner, runtime })
}

fn sha(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn code_of(err: &tokio_postgres::Error) -> Option<&SqlState> {
    err.code()
}

/// A tenant with one account, written by the migrator.
async fn tenant_with_account(owner: &PgBackend) -> (String, Uuid) {
    let tenant = format!("nearai-referral-{}", Uuid::new_v4().simple());
    let account = Uuid::new_v4();
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
    tx.execute(
        "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
        &[&tenant, &account],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (tenant, account)
}

/// Runs `sql` with `params` in one transaction under `tenant`'s context on
/// `backend`, committing on success.
async fn exec(
    backend: &PgBackend,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<u64, tokio_postgres::Error> {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let result = tx.execute(sql, params).await;
    if result.is_ok() {
        tx.commit().await.unwrap();
    }
    result
}

async fn count(backend: &PgBackend, tenant: &str, sql: &str) -> i64 {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.query_one(sql, &[]).await.unwrap().get(0)
}

const INSERT_CODE: &str = "INSERT INTO trace_referral_codes
       (tenant_id, code_id, account_id, code_index, sealed_code, code_key_ref, index_pepper_ref)
     VALUES (trace_current_tenant_id(), $1, $2, $3, '{\"v\":1}'::jsonb, 'key-test-1', 'pepper-test-1')";

async fn code(backend: &PgBackend, tenant: &str, account: Uuid) -> String {
    let index = sha(&format!("code:{}", Uuid::new_v4()));
    exec(
        backend,
        tenant,
        INSERT_CODE,
        &[&Uuid::new_v4(), &account, &index],
    )
    .await
    .expect("insert a code");
    index
}

const INSERT_ATTRIBUTION: &str = "INSERT INTO trace_referral_attributions
       (tenant_id, account_id, code_index, referrer_tenant_id, referrer_account_id,
        referee_ref_hash, referee_account_created_at, applied_via, earn_until)
     SELECT trace_current_tenant_id(), $1, $2, $3, $4, $5, a.created_at, 'later',
            a.created_at + interval '92 days'
       FROM trace_accounts a
      WHERE a.tenant_id = trace_current_tenant_id() AND a.account_id = $1";

const INSERT_REFEREE: &str = "INSERT INTO trace_referral_referees
       (tenant_id, referee_ref_hash, account_id, code_index, referee_account_created_at, applied_at)
     VALUES (trace_current_tenant_id(), $1, $2, $3, clock_timestamp(), clock_timestamp())";

const INSERT_BONUS: &str = "INSERT INTO trace_referral_bonus_accruals
       (tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind, source_accrual_id,
        bps, base_units, bonus_units, rate_version, instrument_id)
     VALUES (trace_current_tenant_id(), $1, 'near_ai', $2, $3, $4, $5, 1000,
             $6::TEXT::NUMERIC, $7::TEXT::NUMERIC, 'v1', 'near_ai_credit_usd')
     ON CONFLICT DO NOTHING";

/// A referrer with a code, a referee who applied it, and the referrer's
/// mirror row, all written as the runtime in one transaction per tenant.
struct Referral {
    referrer_tenant: String,
    referrer: Uuid,
    referee_tenant: String,
    referee: Uuid,
    referee_ref: String,
}

async fn referral(fixture: &Fixture) -> Referral {
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    let index = code(&fixture.runtime, &referrer_tenant, referrer).await;
    let referee_ref = sha(&format!("referee:{referee_tenant}:{referee}"));
    // The apply transaction: attribution under the referee's context, then
    // the mirror under the referrer's, committed together.
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&referee_tenant],
    )
    .await
    .unwrap();
    let written = tx
        .execute(
            INSERT_ATTRIBUTION,
            &[&referee, &index, &referrer_tenant, &referrer, &referee_ref],
        )
        .await
        .expect("attribution");
    assert_eq!(written, 1);
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&referrer_tenant],
    )
    .await
    .unwrap();
    tx.execute(INSERT_REFEREE, &[&referee_ref, &referrer, &index])
        .await
        .expect("mirror");
    tx.commit().await.unwrap();
    Referral {
        referrer_tenant,
        referrer,
        referee_tenant,
        referee,
        referee_ref,
    }
}

#[tokio::test]
async fn every_referral_table_forces_rls() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    for table in REFERRAL_TABLES {
        let row = client
            .query_one(
                "SELECT relrowsecurity, relforcerowsecurity FROM pg_class
                  WHERE oid = $1::text::regclass",
                &[&table],
            )
            .await
            .unwrap();
        assert!(row.get::<_, bool>(0), "{table}: RLS enabled");
        assert!(row.get::<_, bool>(1), "{table}: RLS forced");
    }
}

#[tokio::test]
async fn the_apply_transaction_writes_both_tenants_and_each_sees_only_its_own() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let r = referral(&fixture).await;
    let in_referee = count(
        &fixture.runtime,
        &r.referee_tenant,
        "SELECT count(*) FROM trace_referral_attributions",
    )
    .await;
    assert_eq!(in_referee, 1);
    let mirror_in_referee = count(
        &fixture.runtime,
        &r.referee_tenant,
        "SELECT count(*) FROM trace_referral_referees",
    )
    .await;
    assert_eq!(mirror_in_referee, 0, "the referee cannot see the mirror");
    for table in REFERRAL_TABLES {
        let seen = count(
            &fixture.runtime,
            &r.referrer_tenant,
            &format!("SELECT count(*) FROM {table} WHERE tenant_id <> trace_current_tenant_id()"),
        )
        .await;
        assert_eq!(seen, 0, "{table}: another tenant's rows are invisible");
    }
    let codes_in_referee = count(
        &fixture.runtime,
        &r.referee_tenant,
        "SELECT count(*) FROM trace_referral_codes",
    )
    .await;
    assert_eq!(
        codes_in_referee, 0,
        "the referee cannot read the referrer's code"
    );
}

#[tokio::test]
async fn an_account_has_at_most_one_referrer() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let r = referral(&fixture).await;
    let (other_tenant, other) = tenant_with_account(&fixture.owner).await;
    let other_code = code(&fixture.runtime, &other_tenant, other).await;
    let second = exec(
        &fixture.runtime,
        &r.referee_tenant,
        INSERT_ATTRIBUTION,
        &[
            &r.referee,
            &other_code,
            &other_tenant,
            &other,
            &sha(&format!("another-ref:{}", r.referee)),
        ],
    )
    .await
    .expect_err("a second referrer is refused");
    assert_eq!(code_of(&second), Some(&SqlState::UNIQUE_VIOLATION));
}

#[tokio::test]
async fn a_referral_within_one_tenant_is_refused() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let index = code(&fixture.runtime, &tenant, account).await;
    let err = exec(
        &fixture.runtime,
        &tenant,
        INSERT_ATTRIBUTION,
        &[
            &account,
            &index,
            &tenant,
            &account,
            &sha(&format!("self:{account}")),
        ],
    )
    .await
    .expect_err("self-referral is refused");
    assert_eq!(code_of(&err), Some(&SqlState::CHECK_VIOLATION));
}

#[tokio::test]
async fn a_code_index_names_one_referrer_across_tenants() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (first_tenant, first) = tenant_with_account(&fixture.owner).await;
    let (second_tenant, second) = tenant_with_account(&fixture.owner).await;
    let index = code(&fixture.runtime, &first_tenant, first).await;
    let err = exec(
        &fixture.runtime,
        &second_tenant,
        INSERT_CODE,
        &[&Uuid::new_v4(), &second, &index],
    )
    .await
    .expect_err("the same index in another tenant is refused");
    assert_eq!(code_of(&err), Some(&SqlState::UNIQUE_VIOLATION));
}

#[tokio::test]
async fn the_resolver_reads_code_owners_across_tenants_and_nothing_else() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let index = code(&fixture.runtime, &tenant, account).await;
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    // No tenant context: only the resolver's own policy can show the row.
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_referral_resolver")
        .await
        .unwrap();
    let row = tx
        .query_one(
            "SELECT tenant_id, account_id FROM trace_referral_codes
              WHERE code_index = $1 AND disabled_at IS NULL
                AND suspended_at IS NULL AND voided_at IS NULL",
            &[&index],
        )
        .await
        .expect("resolve the code");
    assert_eq!(row.get::<_, String>(0), tenant);
    assert_eq!(row.get::<_, Uuid>(1), account);
    let sealed = tx
        .query_one(
            "SELECT sealed_code FROM trace_referral_codes WHERE code_index = $1",
            &[&index],
        )
        .await
        .expect_err("the sealed code is not the resolver's");
    assert_eq!(code_of(&sealed), Some(&SqlState::INSUFFICIENT_PRIVILEGE));
    tx.rollback().await.unwrap();
    for table in [
        "trace_referral_attributions",
        "trace_referral_referees",
        "trace_referral_bonus_accruals",
        "trace_referral_audit",
        "trace_referral_network_marks",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE trace_referral_resolver")
            .await
            .unwrap();
        let err = tx
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .expect_err("the resolver reads only codes");
        assert_eq!(
            code_of(&err),
            Some(&SqlState::INSUFFICIENT_PRIVILEGE),
            "{table}"
        );
        tx.rollback().await.unwrap();
    }
    let role = client
        .query_one(
            "SELECT rolsuper, rolbypassrls, rolcanlogin FROM pg_roles
              WHERE rolname = 'trace_referral_resolver'",
            &[],
        )
        .await
        .unwrap();
    assert!(!role.get::<_, bool>(0) && !role.get::<_, bool>(1) && !role.get::<_, bool>(2));
    // SET LOCAL ROLE loads no role settings, so the role carries none and
    // the timeout comes from the transaction that assumes it.
    let config: Option<Vec<String>> = client
        .query_one(
            "SELECT rolconfig FROM pg_roles WHERE rolname = 'trace_referral_resolver'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(config, None, "a role setting would never apply");
    let tx = client.transaction().await.unwrap();
    tx.batch_execute(REFERRAL_RESOLVER_ASSUME_SQL)
        .await
        .unwrap();
    let row = tx
        .query_one(
            "SELECT current_user::TEXT, current_setting('statement_timeout')",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "trace_referral_resolver");
    assert_eq!(row.get::<_, String>(1), "2s");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn bonus_rows_are_priced_once_per_source_and_signed_by_kind() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let r = referral(&fixture).await;
    let source = Uuid::new_v4();
    let bonus =
        |kind: &'static str, source: Option<Uuid>, base: &'static str, units: &'static str| {
            let runtime = &fixture.runtime;
            let tenant = r.referrer_tenant.clone();
            let referrer = r.referrer;
            let referee_ref = r.referee_ref.clone();
            async move {
                exec(
                    runtime,
                    &tenant,
                    INSERT_BONUS,
                    &[
                        &Uuid::new_v4(),
                        &referrer,
                        &referee_ref,
                        &kind,
                        &source,
                        &base,
                        &units,
                    ],
                )
                .await
            }
        };
    assert_eq!(
        bonus("bonus", Some(source), "5000", "500").await.unwrap(),
        1
    );
    assert_eq!(
        bonus("bonus", Some(source), "5000", "500").await.unwrap(),
        0,
        "the same referee accrual is priced once"
    );
    assert_eq!(
        bonus("reversal", Some(Uuid::new_v4()), "-5000", "-500")
            .await
            .unwrap(),
        1
    );
    for (kind, source, base, units) in [
        ("bonus", Some(Uuid::new_v4()), "-1", "0"),
        ("bonus", Some(Uuid::new_v4()), "10", "-1"),
        ("reversal", Some(Uuid::new_v4()), "10", "1"),
        ("void", None, "1", "-1"),
        ("bonus", None, "10", "1"),
        ("void", Some(Uuid::new_v4()), "0", "-1"),
    ] {
        let err = bonus(kind, source, base, units)
            .await
            .expect_err("a misshapen bonus row is refused");
        assert_eq!(
            code_of(&err),
            Some(&SqlState::CHECK_VIOLATION),
            "{kind} {base} {units}"
        );
    }
    assert_eq!(bonus("void", None, "0", "-300").await.unwrap(), 1);
    assert_eq!(
        bonus("void", None, "0", "-300").await.unwrap(),
        0,
        "one void per referee and rail"
    );
    let owed: String = {
        let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&r.referrer_tenant],
        )
        .await
        .unwrap();
        tx.query_one(
            "SELECT SUM(bonus_units)::TEXT FROM trace_referral_bonus_accruals
              WHERE rail_id = 'near_ai' AND account_id = $1",
            &[&r.referrer],
        )
        .await
        .unwrap()
        .get(0)
    };
    assert_eq!(owed, "-300");
}

#[tokio::test]
async fn bonus_and_audit_rows_are_append_only_for_the_runtime() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let r = referral(&fixture).await;
    exec(
        &fixture.runtime,
        &r.referrer_tenant,
        INSERT_BONUS,
        &[
            &Uuid::new_v4(),
            &r.referrer,
            &r.referee_ref,
            &"bonus",
            &Some(Uuid::new_v4()),
            &"100",
            &"10",
        ],
    )
    .await
    .unwrap();
    exec(
        &fixture.runtime,
        &r.referrer_tenant,
        "INSERT INTO trace_referral_audit (tenant_id, audit_id, action, actor_ref_hash)
         VALUES (trace_current_tenant_id(), $1, 'code_created', $2)",
        &[&Uuid::new_v4(), &sha("actor")],
    )
    .await
    .unwrap();
    for sql in [
        "UPDATE trace_referral_bonus_accruals SET bonus_units = 0",
        "DELETE FROM trace_referral_bonus_accruals",
        "UPDATE trace_referral_audit SET label = 'x'",
        "DELETE FROM trace_referral_audit",
        "DELETE FROM trace_referral_codes",
        "UPDATE trace_referral_codes SET code_index = code_index",
        "UPDATE trace_referral_attributions SET referrer_account_id = referrer_account_id",
        "DELETE FROM trace_referral_referees",
    ] {
        let err = exec(&fixture.runtime, &r.referrer_tenant, sql, &[])
            .await
            .expect_err(sql);
        assert_eq!(
            code_of(&err),
            Some(&SqlState::INSUFFICIENT_PRIVILEGE),
            "{sql}"
        );
    }
    // What the store does change stays writable.
    exec(
        &fixture.runtime,
        &r.referee_tenant,
        "UPDATE trace_referral_attributions SET voided_at = clock_timestamp(),
                void_label = 'referral_abuse' WHERE account_id = $1",
        &[&r.referee],
    )
    .await
    .expect("an operator void is written by the runtime");
    exec(
        &fixture.runtime,
        &r.referrer_tenant,
        "UPDATE trace_referral_codes SET suspended_at = clock_timestamp(),
                suspension_labels = ARRAY['referral_refused_share']",
        &[],
    )
    .await
    .expect("a suspension is written by the runtime");
}

#[tokio::test]
async fn a_suspension_carries_its_labels_and_a_void_its_label() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    code(&fixture.runtime, &tenant, account).await;
    for sql in [
        "UPDATE trace_referral_codes SET suspended_at = clock_timestamp()",
        "UPDATE trace_referral_codes SET suspension_labels = ARRAY['referral_burst']",
        "UPDATE trace_referral_codes SET voided_at = clock_timestamp()",
    ] {
        let err = exec(&fixture.runtime, &tenant, sql, &[])
            .await
            .expect_err(sql);
        assert_eq!(code_of(&err), Some(&SqlState::CHECK_VIOLATION), "{sql}");
    }
}

/// A `near_ai` link in `state`, written by the migrator.
async fn payout_link(owner: &PgBackend, tenant: &str, account: Uuid, state: &str) -> Uuid {
    let link = Uuid::new_v4();
    exec(
        owner,
        tenant,
        "INSERT INTO trace_payout_links
           (tenant_id, account_id, rail_id, link_id, recipient_kind, state, sealed_recipient,
            recipient_index, recipient_key_ref, index_pepper_ref, subject_index, program)
         VALUES (trace_current_tenant_id(), $1, 'near_ai', $2, 'near_ai_org', $3,
                 '{\"v\":1}'::jsonb, $4, 'key-test-1', 'pepper-test-1', $5, $6)",
        &[
            &account,
            &link,
            &state,
            &sha(&format!("index:{link}")),
            &sha(&format!("subject:{link}")),
            &format!("tc1-{}", &sha(&link.to_string())[7..39]),
        ],
    )
    .await
    .expect("insert a link");
    link
}

const INSERT_LINE: &str = "INSERT INTO trace_payout_lines
       (tenant_id, line_id, account_id, rail_id, link_id, target_total, delta,
        owed_through_accrual_count, state, stream)
     VALUES (trace_current_tenant_id(), $1, $2, 'near_ai', $3, 10, 10, 1, 'pending', $4)";

#[tokio::test]
async fn a_link_has_one_pending_line_per_stream() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let link = payout_link(&fixture.owner, &tenant, account, "active").await;
    for stream in ["earned", "referral"] {
        exec(
            &fixture.runtime,
            &tenant,
            INSERT_LINE,
            &[&Uuid::new_v4(), &account, &link, &stream],
        )
        .await
        .unwrap_or_else(|err| panic!("first pending {stream} line: {err}"));
    }
    for stream in ["earned", "referral"] {
        let err = exec(
            &fixture.runtime,
            &tenant,
            INSERT_LINE,
            &[&Uuid::new_v4(), &account, &link, &stream],
        )
        .await
        .expect_err("a second pending line on one stream is refused");
        assert_eq!(code_of(&err), Some(&SqlState::UNIQUE_VIOLATION), "{stream}");
    }
    let err = exec(
        &fixture.runtime,
        &tenant,
        INSERT_LINE,
        &[&Uuid::new_v4(), &account, &link, &"bonus"],
    )
    .await
    .expect_err("an unknown stream is refused");
    assert_eq!(code_of(&err), Some(&SqlState::CHECK_VIOLATION));
    let defaulted = count(
        &fixture.owner,
        &tenant,
        "SELECT count(*) FROM trace_payout_lines
          WHERE tenant_id = trace_current_tenant_id() AND stream = 'earned'",
    )
    .await;
    assert_eq!(defaulted, 1);
}

#[tokio::test]
async fn a_referral_total_is_recorded_only_when_its_link_closes() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let link = payout_link(&fixture.owner, &tenant, account, "active").await;
    let program = format!("tr1-{}", &sha(&format!("referral:{link}"))[7..39]);
    exec(
        &fixture.runtime,
        &tenant,
        "UPDATE trace_payout_links SET referral_program = $2 WHERE link_id = $1",
        &[&link, &program],
    )
    .await
    .expect("the runtime sets a link's referral program");
    let err = exec(
        &fixture.runtime,
        &tenant,
        "UPDATE trace_payout_links SET final_paid_referral = 5 WHERE link_id = $1",
        &[&link],
    )
    .await
    .expect_err("an open link has no final referral total");
    assert_eq!(code_of(&err), Some(&SqlState::CHECK_VIOLATION));
    let err = exec(
        &fixture.runtime,
        &tenant,
        "UPDATE trace_payout_links SET referral_program = program WHERE link_id = $1",
        &[&link],
    )
    .await
    .expect_err("the referral program differs from the earned program");
    assert_eq!(code_of(&err), Some(&SqlState::CHECK_VIOLATION));
    exec(
        &fixture.runtime,
        &tenant,
        "UPDATE trace_payout_links SET state = 'closing' WHERE link_id = $1",
        &[&link],
    )
    .await
    .unwrap();
    exec(
        &fixture.runtime,
        &tenant,
        "UPDATE trace_payout_links SET state = 'closed', final_paid = 40,
                final_paid_referral = 5 WHERE link_id = $1",
        &[&link],
    )
    .await
    .expect("a link closes with both totals");
}

/// Network marks (owner decision 2026-10-29, "server-observed signals"):
/// one row per account and mark in the account's own tenant, readable and
/// refreshable by the runtime only under that tenant, deletable for
/// retention, and never anything but a `sha256:` hex mark.
#[tokio::test]
async fn network_marks_are_tenant_scoped_and_shaped() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let (other, _) = tenant_with_account(&fixture.owner).await;
    let mark = sha("mark-1");
    const INSERT_MARK: &str =
        "INSERT INTO trace_referral_network_marks (tenant_id, account_id, mark)
         VALUES (trace_current_tenant_id(), $1, $2)";
    exec(&fixture.runtime, &tenant, INSERT_MARK, &[&account, &mark])
        .await
        .expect("the runtime records a mark");
    let duplicate = exec(&fixture.runtime, &tenant, INSERT_MARK, &[&account, &mark])
        .await
        .expect_err("one row per account and mark");
    assert_eq!(code_of(&duplicate), Some(&SqlState::UNIQUE_VIOLATION));
    let raw = exec(
        &fixture.runtime,
        &tenant,
        INSERT_MARK,
        &[&account, &"203.0.113.0/24"],
    )
    .await
    .expect_err("a mark is a hash, never a prefix");
    assert_eq!(code_of(&raw), Some(&SqlState::CHECK_VIOLATION));
    let elsewhere = exec(
        &fixture.runtime,
        &other,
        "INSERT INTO trace_referral_network_marks (tenant_id, account_id, mark)
         VALUES ($1, $2, $3)",
        &[&tenant, &account, &sha("mark-2")],
    )
    .await
    .expect_err("a mark is written only in its own tenant");
    assert_eq!(code_of(&elsewhere), Some(&SqlState::INSUFFICIENT_PRIVILEGE));
    assert_eq!(
        count(
            &fixture.runtime,
            &tenant,
            "SELECT count(*) FROM trace_referral_network_marks"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &fixture.runtime,
            &other,
            "SELECT count(*) FROM trace_referral_network_marks"
        )
        .await,
        0,
        "another tenant's marks are invisible"
    );
    exec(
        &fixture.runtime,
        &tenant,
        "UPDATE trace_referral_network_marks SET last_seen_at = clock_timestamp()",
        &[],
    )
    .await
    .expect("the runtime refreshes last_seen_at");
    for sql in [
        "UPDATE trace_referral_network_marks SET mark = mark",
        "UPDATE trace_referral_network_marks SET first_seen_at = first_seen_at",
        "UPDATE trace_referral_network_marks SET account_id = account_id",
    ] {
        let err = exec(&fixture.runtime, &tenant, sql, &[])
            .await
            .expect_err(sql);
        assert_eq!(
            code_of(&err),
            Some(&SqlState::INSUFFICIENT_PRIVILEGE),
            "{sql}"
        );
    }
    exec(
        &fixture.runtime,
        &tenant,
        "DELETE FROM trace_referral_network_marks WHERE account_id = $1",
        &[&account],
    )
    .await
    .expect("retention deletes marks");
}

/// The pass reads the dedup cluster of each side's own gate decisions
/// through V142's column grant, not the operator's pilot grant.
#[tokio::test]
async fn the_runtime_reads_the_cluster_columns_of_gate_decisions() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, _) = tenant_with_account(&fixture.owner).await;
    count(
        &fixture.runtime,
        &tenant,
        "SELECT count(DISTINCT dedup_cluster_id) FROM trace_gate_decisions
          WHERE tenant_id = trace_current_tenant_id()
            AND submission_id IS NOT NULL",
    )
    .await;
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    let err = tx
        .query_one(
            "SELECT nearest_neighbor_hash FROM trace_gate_decisions",
            &[],
        )
        .await
        .expect_err("no other gate column is granted by V142");
    assert_eq!(code_of(&err), Some(&SqlState::INSUFFICIENT_PRIVILEGE));
}

/// Boot's check of the operator's grant: a login that holds
/// `trace_referral_resolver` (here the migrator) can assume it, and one
/// that was never granted it cannot. Ingest refuses to start with
/// referrals on in the second case (`referral_resolver_unprovisioned`).
#[tokio::test]
async fn only_a_login_granted_the_resolver_can_assume_it() {
    let Some(fixture) = fixture().await else {
        return;
    };
    assert!(fixture.owner.referral_resolver_assumable().await.unwrap());

    // A NOSUPERUSER login with no grant. It has a password so the check
    // runs under password authentication too.
    const LOGIN: &str = "tc_referral_resolver_probe";
    fixture
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{LOGIN}') THEN
                     CREATE ROLE {LOGIN} LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '{LOGIN}';
                 END IF;
             END $$;
             REVOKE trace_referral_resolver FROM {LOGIN};"
        ))
        .await
        .expect("create the probe login");
    let mut url =
        reqwest::Url::parse(&std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").unwrap()).unwrap();
    url.set_username(LOGIN).unwrap();
    url.set_password(Some(LOGIN)).unwrap();
    let probe = PgBackend::new(&DatabaseConfig::from_postgres_url(url.as_str(), 1))
        .await
        .expect("probe backend");
    assert!(!probe.referral_resolver_assumable().await.unwrap());

    // Granted as the runbook says (no inheritance, SET allowed), it can.
    fixture
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(&format!(
            "GRANT trace_referral_resolver TO {LOGIN} WITH INHERIT FALSE, SET TRUE"
        ))
        .await
        .expect("grant the resolver");
    assert!(probe.referral_resolver_assumable().await.unwrap());
}

/// Writes one attribution as the runtime, with the session in `zone`, and
/// rolls it back: whether the schema accepts `earn_until` for `created`.
async fn attribution_in_zone(
    fixture: &Fixture,
    zone: &str,
    created: chrono::DateTime<chrono::Utc>,
    earn_until: chrono::DateTime<chrono::Utc>,
) -> Result<u64, tokio_postgres::Error> {
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute(&format!("SET LOCAL TimeZone = '{zone}'"))
        .await
        .unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let result = tx
        .execute(
            "INSERT INTO trace_referral_attributions
               (tenant_id, account_id, code_index, referrer_tenant_id, referrer_account_id,
                referee_ref_hash, referee_account_created_at, applied_via, earn_until)
             VALUES (trace_current_tenant_id(), $1, $2, 'nearai-referral-elsewhere', $3, $4,
                     $5, 'later', $6)",
            &[
                &account,
                &sha(&format!("code:{}", Uuid::new_v4())),
                &Uuid::new_v4(),
                &sha(&format!("referee:{tenant}")),
                &created,
                &earn_until,
            ],
        )
        .await;
    tx.rollback().await.unwrap();
    result
}

/// The promised 92 days are 92 x 24 hours, as the store computes them, in
/// any session time zone. `+ interval '92 days'` is calendar days in the
/// session's zone: across a daylight-saving change it is an hour longer
/// (refusing the store's own earn_until) or shorter (accepting one under
/// the promise).
#[tokio::test]
async fn the_earning_promise_is_92_days_of_hours_in_any_session_time_zone() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let at = |text: &str| {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    let promise = chrono::Duration::days(92);
    // Before the November fall-back and before the March spring-forward,
    // both in the past (applied_at, now, is after creation).
    for created in [at("2025-10-20T17:00:00Z"), at("2026-02-01T17:00:00Z")] {
        for zone in ["UTC", "America/Los_Angeles", "Europe/Berlin"] {
            attribution_in_zone(&fixture, zone, created, created + promise)
                .await
                .unwrap_or_else(|error| panic!("{zone} {created}: the promise is kept: {error}"));
            let short = attribution_in_zone(
                &fixture,
                zone,
                created,
                created + promise - chrono::Duration::minutes(30),
            )
            .await
            .expect_err("half an hour under the promise is refused");
            assert_eq!(
                code_of(&short),
                Some(&SqlState::CHECK_VIOLATION),
                "{zone} {created}"
            );
        }
    }
}
