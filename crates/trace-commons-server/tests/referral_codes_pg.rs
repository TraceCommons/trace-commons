// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral code store (`ReferralStore` over PostgreSQL) against a real
//! database: the five-live-codes limit, the cross-tenant code index, listing
//! with referee counts and bonus totals, disabling, and applying a code with
//! every check of spec section 3 in order, the one-label refusal recorded
//! precisely in the referee's tenant, the refusal rate limit, and the
//! two-tenant apply transaction
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md).
//!
//! Every store call runs as a real `LOGIN` role whose only privilege
//! sources are `trace_ingest_runtime` (inherited) and, with `INHERIT FALSE`,
//! `trace_referral_resolver`, which the store must `SET ROLE` to. Never the
//! migration owner and never a superuser: either one ignores a missing
//! grant. Fixture rows the store never writes are written by the owner.
//!
//! Needs PostgreSQL 16 or later (`GRANT ... WITH INHERIT FALSE`), a database
//! whose name starts with `admission_test_`, on the literal host
//! `127.0.0.1`, and `TRACE_COMMONS_PG_TEST_DATABASE_URL`. Without the
//! variable every test prints a skip line and returns.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

use std::sync::Arc;

use base64::Engine as _;
use chrono::{Duration, Utc};
use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use trace_commons_protocol::referral::ReferralAppliedVia;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::{
    Database, ReferralApply, ReferralApplyPolicy, ReferralCodeCreate, ReferralCodeDisable,
    ReferralCodeWrite, ReferralRefusal, ReferralStore,
};
use trace_commons_server::near_account_identity::NearAccountIdentity;
use trace_commons_server::referral_code::{
    generate_referral_code, open_referral_code, referee_ref_hash, referral_code_index,
    seal_referral_code,
};
use uuid::Uuid;

/// The runtime login these tests connect as. Roles are per server; the
/// login is re-provisioned to exactly this shape on every run.
const RUNTIME_LOGIN: &str = "tc_referral_runtime_test";

/// The tables the store reads that predate V62, which the runtime group
/// reaches on the pilot through its table-wide V62 grant.
const PILOT_GRANTS: &str = "GRANT SELECT ON trace_accounts, trace_near_account_anchors, \
    trace_near_provisioned_devices TO trace_ingest_runtime;";

struct Fixture {
    owner: PgBackend,
    runtime: Arc<PgBackend>,
    identity: Arc<NearAccountIdentity>,
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

fn identity() -> Arc<NearAccountIdentity> {
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("r".repeat(32)))
            .expect("fixture SecretsCrypto");
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "referral-codes-fixture",
        ),
    );
    Arc::new(
        NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([9u8; 32])),
            Some(kek),
        )
        .expect("fixture identity"),
    )
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
    assert!(sixteen, "referral_codes_pg needs PostgreSQL 16 or later");
    client
        .batch_execute(&with_role_ddl_lock(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RUNTIME_LOGIN}') THEN
                     CREATE ROLE {RUNTIME_LOGIN} LOGIN;
                 END IF;
             END $$;
             ALTER ROLE {RUNTIME_LOGIN} LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
             GRANT trace_ingest_runtime TO {RUNTIME_LOGIN} WITH INHERIT TRUE;
             GRANT trace_referral_resolver TO {RUNTIME_LOGIN} WITH INHERIT FALSE, SET TRUE;
             {PILOT_GRANTS}
             INSERT INTO trace_payout_rate_epochs (rail_id, version, numerator, denominator)
             VALUES ('near_ai', 'v1', 1, 1) ON CONFLICT DO NOTHING;"
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
    assert_eq!(row.get::<_, String>(0), RUNTIME_LOGIN);
    assert!(!row.get::<_, bool>(1) && !row.get::<_, bool>(2));
    Some(Fixture {
        owner,
        runtime: Arc::new(runtime),
        identity: identity(),
    })
}

/// A program that opened a day ago and has not ended.
fn open_policy() -> ReferralApplyPolicy {
    ReferralApplyPolicy {
        program_start: Some(Utc::now() - Duration::days(1)),
        program_end: None,
        late_days: 30,
        earn_days: 92,
    }
}

/// A tenant with `accounts` accounts, written by the owner.
async fn tenant_with_accounts(owner: &PgBackend, accounts: usize) -> (String, Vec<Uuid>) {
    let tenant = format!("nearai-referral-codes-{}", Uuid::new_v4().simple());
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

async fn tenant_with_account(owner: &PgBackend) -> (String, Uuid) {
    let (tenant, ids) = tenant_with_accounts(owner, 1).await;
    (tenant, ids[0])
}

/// Runs `sql` as the owner under `tenant`'s context.
async fn owner_exec(
    owner: &PgBackend,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> u64 {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let written = tx.execute(sql, params).await.expect(sql);
    tx.commit().await.unwrap();
    written
}

/// One text column of every row `sql` returns, read as the owner.
async fn owner_texts(owner: &PgBackend, sql: &str, tenant: &str) -> Vec<String> {
    owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query(sql, &[&tenant])
        .await
        .expect(sql)
        .iter()
        .map(|row| row.get(0))
        .collect()
}

/// The refusal labels recorded in `tenant`, oldest first.
async fn refusal_labels(owner: &PgBackend, tenant: &str) -> Vec<String> {
    owner_texts(
        owner,
        "SELECT label FROM trace_referral_audit
          WHERE tenant_id = $1 AND action = 'referral_refused'
          ORDER BY created_at, audit_id",
        tenant,
    )
    .await
}

async fn audit_actions(owner: &PgBackend, tenant: &str) -> Vec<String> {
    owner_texts(
        owner,
        "SELECT action FROM trace_referral_audit WHERE tenant_id = $1
          ORDER BY created_at, audit_id",
        tenant,
    )
    .await
}

impl Fixture {
    /// Creates a fresh code for `account`, returning the canonical code and
    /// the row written.
    async fn code(&self, tenant: &str, account: Uuid) -> (String, ReferralCodeWrite) {
        let code = generate_referral_code();
        let write = seal_referral_code(&self.identity, Uuid::new_v4(), &code).unwrap();
        let created = self
            .runtime
            .create_referral_code(tenant, account, &write)
            .await
            .expect("create a code");
        assert!(
            matches!(created, ReferralCodeCreate::Created { .. }),
            "{created:?}"
        );
        (code, write)
    }

    async fn apply(&self, tenant: &str, account: Uuid, code: Option<&str>) -> ReferralApply {
        self.apply_with(tenant, account, code, &open_policy()).await
    }

    async fn apply_with(
        &self,
        tenant: &str,
        account: Uuid,
        code: Option<&str>,
        policy: &ReferralApplyPolicy,
    ) -> ReferralApply {
        let index = code.map(|code| referral_code_index(&self.identity, code));
        self.runtime
            .apply_referral_code(
                tenant,
                account,
                index.as_deref(),
                ReferralAppliedVia::Later,
                policy,
            )
            .await
            .expect("apply")
    }
}

#[tokio::test]
async fn an_account_holds_at_most_five_live_codes() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let mut writes = Vec::new();
    for _ in 0..5 {
        writes.push(fixture.code(&tenant, account).await.1);
    }
    let sixth =
        seal_referral_code(&fixture.identity, Uuid::new_v4(), &generate_referral_code()).unwrap();
    assert_eq!(
        fixture
            .runtime
            .create_referral_code(&tenant, account, &sixth)
            .await
            .unwrap(),
        ReferralCodeCreate::LimitReached
    );

    // Turning one off frees its place; turning it off again changes nothing.
    assert_eq!(
        fixture
            .runtime
            .disable_referral_code(&tenant, account, writes[0].code_id)
            .await
            .unwrap(),
        ReferralCodeDisable::Disabled
    );
    assert_eq!(
        fixture
            .runtime
            .disable_referral_code(&tenant, account, writes[0].code_id)
            .await
            .unwrap(),
        ReferralCodeDisable::AlreadyDisabled
    );
    assert_eq!(
        fixture
            .runtime
            .disable_referral_code(&tenant, account, Uuid::new_v4())
            .await
            .unwrap(),
        ReferralCodeDisable::NotFound
    );
    assert!(matches!(
        fixture
            .runtime
            .create_referral_code(&tenant, account, &sixth)
            .await
            .unwrap(),
        ReferralCodeCreate::Created { .. }
    ));

    // A suspended code is not live either, so it frees a place too.
    owner_exec(
        &fixture.owner,
        &tenant,
        "UPDATE trace_referral_codes SET suspended_at = clock_timestamp(),
                suspension_labels = ARRAY['referral_burst'] WHERE code_id = $1",
        &[&writes[1].code_id],
    )
    .await;
    fixture.code(&tenant, account).await;

    let actions = audit_actions(&fixture.owner, &tenant).await;
    assert_eq!(
        actions.iter().filter(|a| *a == "code_created").count(),
        7,
        "{actions:?}"
    );
    assert_eq!(
        actions.iter().filter(|a| *a == "code_disabled").count(),
        1,
        "{actions:?}"
    );
}

#[tokio::test]
async fn another_accounts_code_cannot_be_disabled() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let (_, write) = fixture.code(&tenant, accounts[0]).await;
    assert_eq!(
        fixture
            .runtime
            .disable_referral_code(&tenant, accounts[1], write.code_id)
            .await
            .unwrap(),
        ReferralCodeDisable::NotFound
    );
    let (other_tenant, other) = tenant_with_account(&fixture.owner).await;
    assert_eq!(
        fixture
            .runtime
            .disable_referral_code(&other_tenant, other, write.code_id)
            .await
            .unwrap(),
        ReferralCodeDisable::NotFound
    );
    let view = fixture
        .runtime
        .referral_codes(&tenant, accounts[0])
        .await
        .unwrap();
    assert!(view.codes[0].is_live());
}

#[tokio::test]
async fn a_closed_account_makes_no_codes() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    owner_exec(
        &fixture.owner,
        &tenant,
        "UPDATE trace_accounts SET closed_at = now() WHERE account_id = $1",
        &[&account],
    )
    .await;
    let write =
        seal_referral_code(&fixture.identity, Uuid::new_v4(), &generate_referral_code()).unwrap();
    assert_eq!(
        fixture
            .runtime
            .create_referral_code(&tenant, account, &write)
            .await
            .unwrap(),
        ReferralCodeCreate::AccountIneligible
    );
    let (missing_tenant, _) = tenant_with_account(&fixture.owner).await;
    assert_eq!(
        fixture
            .runtime
            .create_referral_code(&missing_tenant, Uuid::new_v4(), &write)
            .await
            .unwrap(),
        ReferralCodeCreate::AccountIneligible
    );
}

#[tokio::test]
async fn a_code_index_taken_in_any_tenant_is_refused_and_writes_nothing() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (first_tenant, first) = tenant_with_account(&fixture.owner).await;
    let (second_tenant, second) = tenant_with_account(&fixture.owner).await;
    let (code, _) = fixture.code(&first_tenant, first).await;
    let again = seal_referral_code(&fixture.identity, Uuid::new_v4(), &code).unwrap();
    assert_eq!(
        fixture
            .runtime
            .create_referral_code(&second_tenant, second, &again)
            .await
            .unwrap(),
        ReferralCodeCreate::IndexTaken
    );
    assert!(
        audit_actions(&fixture.owner, &second_tenant)
            .await
            .is_empty()
    );
    let view = fixture
        .runtime
        .referral_codes(&second_tenant, second)
        .await
        .unwrap();
    assert!(view.codes.is_empty());
}

#[tokio::test]
async fn listing_shows_each_code_with_its_referees_and_bonus() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (first_code, first) = fixture.code(&referrer_tenant, referrer).await;
    let (_, second) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    assert!(matches!(
        fixture
            .apply(&referee_tenant, referee, Some(&first_code))
            .await,
        ReferralApply::Applied { .. }
    ));

    // A bonus and a reversal from the referee, and one confirmed line on the
    // referral stream, as the bonus pass and the sender would write them.
    let referee_ref = referee_ref_hash(&referee_tenant, referee);
    for (kind, base, units) in [("bonus", "5000", "500"), ("reversal", "-1000", "-100")] {
        owner_exec(
            &fixture.owner,
            &referrer_tenant,
            "INSERT INTO trace_referral_bonus_accruals
               (tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
                source_accrual_id, bps, base_units, bonus_units, rate_version, instrument_id)
             VALUES (trace_current_tenant_id(), $1, 'near_ai', $2, $3, $4, $5, 1000,
                     $6::TEXT::NUMERIC, $7::TEXT::NUMERIC, 'v1', 'near_ai_credit_usd')",
            &[
                &Uuid::new_v4(),
                &referrer,
                &referee_ref,
                &kind,
                &Uuid::new_v4(),
                &base,
                &units,
            ],
        )
        .await;
    }
    let link = Uuid::new_v4();
    let sha = |value: String| {
        use sha2::{Digest, Sha256};
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    };
    owner_exec(
        &fixture.owner,
        &referrer_tenant,
        "INSERT INTO trace_payout_links
           (tenant_id, account_id, rail_id, link_id, recipient_kind, state, sealed_recipient,
            recipient_index, recipient_key_ref, index_pepper_ref, subject_index, program)
         VALUES (trace_current_tenant_id(), $1, 'near_ai', $2, 'near_ai_org', 'active',
                 '{\"v\":1}'::jsonb, $3, 'key-test-1', 'pepper-test-1', $4, $5)",
        &[
            &referrer,
            &link,
            &sha(format!("index:{link}")),
            &sha(format!("subject:{link}")),
            &format!("tc1-{}", &sha(link.to_string())[7..39]),
        ],
    )
    .await;
    for (stream, delta) in [("referral", 300_i64), ("earned", 9_000)] {
        owner_exec(
            &fixture.owner,
            &referrer_tenant,
            "INSERT INTO trace_payout_lines
               (tenant_id, line_id, account_id, rail_id, link_id, target_total, delta,
                owed_through_accrual_count, state, stream, receipt_hash, resolved_at)
             VALUES (trace_current_tenant_id(), $1, $2, 'near_ai', $3, $4::TEXT::NUMERIC,
                     $4::TEXT::NUMERIC, 1,
                     'confirmed', $5, $6, clock_timestamp())",
            &[
                &Uuid::new_v4(),
                &referrer,
                &link,
                &delta.to_string(),
                &stream,
                &sha(Uuid::new_v4().to_string()),
            ],
        )
        .await;
    }

    let view = fixture
        .runtime
        .referral_codes(&referrer_tenant, referrer)
        .await
        .unwrap();
    assert_eq!(view.codes.len(), 2);
    assert_eq!(view.codes[0].code_id, first.code_id);
    assert_eq!(view.codes[0].referee_count, 1);
    assert_eq!(view.codes[0].bonus_owed, 400);
    assert_eq!(view.codes[1].code_id, second.code_id);
    assert_eq!(view.codes[1].referee_count, 0);
    assert_eq!(view.codes[1].bonus_owed, 0);
    assert_eq!(view.bonus_owed, 400);
    assert_eq!(view.bonus_sent, 300, "only the referral stream counts");
    let row = &view.codes[0];
    assert_eq!(
        open_referral_code(
            &fixture.identity,
            &row.code_index,
            &row.sealed_code,
            &row.code_key_ref,
            &row.index_pepper_ref,
        )
        .as_deref(),
        Some(first_code.as_str()),
        "the owner can read its code back"
    );
    // The referee sees none of it.
    let referee_view = fixture
        .runtime
        .referral_codes(&referee_tenant, referee)
        .await
        .unwrap();
    assert!(referee_view.codes.is_empty());
    assert_eq!(referee_view.bonus_owed, 0);
}

#[tokio::test]
async fn applying_a_code_writes_both_tenants_in_one_go() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, write) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    // Typed loosely: the store sees only the index of the canonical form.
    let applied = fixture.apply(&referee_tenant, referee, Some(&code)).await;
    let ReferralApply::Applied {
        applied_at,
        referee_account_created_at,
        earn_until,
    } = applied
    else {
        panic!("{applied:?}");
    };
    assert!(applied_at >= referee_account_created_at);
    // The policy's earning period is stamped on the attribution.
    assert_eq!(earn_until, referee_account_created_at + Duration::days(92));

    let referee_ref = referee_ref_hash(&referee_tenant, referee);
    let attribution = owner_texts(
        &fixture.owner,
        "SELECT referrer_tenant_id || ' ' || referrer_account_id::TEXT || ' ' ||
                code_index || ' ' || referee_ref_hash || ' ' || applied_via
           FROM trace_referral_attributions WHERE tenant_id = $1",
        &referee_tenant,
    )
    .await;
    assert_eq!(
        attribution,
        vec![format!(
            "{referrer_tenant} {referrer} {} {referee_ref} later",
            write.code_index
        )]
    );
    let mirror = owner_texts(
        &fixture.owner,
        "SELECT referee_ref_hash || ' ' || account_id::TEXT || ' ' || code_index
           FROM trace_referral_referees WHERE tenant_id = $1",
        &referrer_tenant,
    )
    .await;
    assert_eq!(
        mirror,
        vec![format!("{referee_ref} {referrer} {}", write.code_index)]
    );
    // An applied row in each tenant. In the referrer's, the referee is named
    // by its hash only, never by its tenant or account.
    assert_eq!(
        audit_actions(&fixture.owner, &referee_tenant).await,
        vec!["referral_applied"]
    );
    let referrer_rows = owner_texts(
        &fixture.owner,
        "SELECT row_to_json(a)::TEXT FROM trace_referral_audit a
          WHERE tenant_id = $1 AND action = 'referral_applied'",
        &referrer_tenant,
    )
    .await;
    assert_eq!(referrer_rows.len(), 1);
    assert!(referrer_rows[0].contains(&referee_ref));
    assert!(!referrer_rows[0].contains(&referee_tenant));
    assert!(!referrer_rows[0].contains(&referee.to_string()));
    let referrer_mirror_dump = owner_texts(
        &fixture.owner,
        "SELECT row_to_json(r)::TEXT FROM trace_referral_referees r WHERE tenant_id = $1",
        &referrer_tenant,
    )
    .await;
    assert!(!referrer_mirror_dump[0].contains(&referee_tenant));
    assert!(!referrer_mirror_dump[0].contains(&referee.to_string()));

    let status = fixture
        .runtime
        .referral_status(&referee_tenant, referee)
        .await
        .unwrap();
    let attribution = status.attribution.expect("an attribution");
    assert_eq!(attribution.applied_via, ReferralAppliedVia::Later);
    assert_eq!(attribution.applied_at, applied_at);
    assert_eq!(
        attribution.referee_account_created_at,
        referee_account_created_at
    );
    assert_eq!(status.account_created_at, Some(referee_account_created_at));
    assert!(!attribution.voided);

    // Once applied, never changed.
    let (other_tenant, other) = tenant_with_account(&fixture.owner).await;
    let (other_code, _) = fixture.code(&other_tenant, other).await;
    assert_eq!(
        fixture
            .apply(&referee_tenant, referee, Some(&other_code))
            .await,
        ReferralApply::Refused(ReferralRefusal::AlreadyReferred)
    );
    assert_eq!(
        refusal_labels(&fixture.owner, &referee_tenant).await,
        vec!["referral_already_referred"]
    );
    assert!(
        refusal_labels(&fixture.owner, &other_tenant)
            .await
            .is_empty(),
        "a refusal is recorded in the referee's tenant only"
    );
}

#[tokio::test]
async fn a_signup_apply_is_recorded_as_one() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, _) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    let applied = fixture
        .runtime
        .apply_referral_code(
            &referee_tenant,
            referee,
            Some(&referral_code_index(&fixture.identity, &code)),
            ReferralAppliedVia::Signup,
            &open_policy(),
        )
        .await
        .unwrap();
    assert!(matches!(applied, ReferralApply::Applied { .. }));
    let status = fixture
        .runtime
        .referral_status(&referee_tenant, referee)
        .await
        .unwrap();
    assert_eq!(
        status.attribution.unwrap().applied_via,
        ReferralAppliedVia::Signup
    );
}

#[tokio::test]
async fn nothing_is_applied_while_the_program_is_closed() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, _) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    let now = Utc::now();
    for policy in [
        ReferralApplyPolicy {
            program_start: None,
            program_end: None,
            late_days: 30,
            earn_days: 92,
        },
        ReferralApplyPolicy {
            program_start: Some(now + Duration::hours(1)),
            program_end: None,
            late_days: 30,
            earn_days: 92,
        },
        ReferralApplyPolicy {
            program_start: Some(now - Duration::days(10)),
            program_end: Some(now - Duration::hours(1)),
            late_days: 30,
            earn_days: 92,
        },
    ] {
        for typed in [Some(code.as_str()), Some("0000000000"), None] {
            assert_eq!(
                fixture
                    .apply_with(&referee_tenant, referee, typed, &policy)
                    .await,
                ReferralApply::Refused(ReferralRefusal::Unavailable),
                "{policy:?}"
            );
        }
    }
    assert_eq!(
        refusal_labels(&fixture.owner, &referee_tenant).await,
        vec!["referral_unavailable"; 9]
    );
}

#[tokio::test]
async fn an_unknown_or_dead_code_is_unknown() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (disabled_code, disabled) = fixture.code(&referrer_tenant, referrer).await;
    let (suspended_code, suspended) = fixture.code(&referrer_tenant, referrer).await;
    let (voided_code, voided) = fixture.code(&referrer_tenant, referrer).await;
    fixture
        .runtime
        .disable_referral_code(&referrer_tenant, referrer, disabled.code_id)
        .await
        .unwrap();
    owner_exec(
        &fixture.owner,
        &referrer_tenant,
        "UPDATE trace_referral_codes SET suspended_at = clock_timestamp(),
                suspension_labels = ARRAY['referral_burst'] WHERE code_id = $1",
        &[&suspended.code_id],
    )
    .await;
    owner_exec(
        &fixture.owner,
        &referrer_tenant,
        "UPDATE trace_referral_codes SET voided_at = clock_timestamp(),
                void_label = 'referral_abuse' WHERE code_id = $1",
        &[&voided.code_id],
    )
    .await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    for typed in [
        None,
        Some(generate_referral_code().as_str()),
        Some(disabled_code.as_str()),
        Some(suspended_code.as_str()),
        Some(voided_code.as_str()),
    ] {
        assert_eq!(
            fixture.apply(&referee_tenant, referee, typed).await,
            ReferralApply::Refused(ReferralRefusal::CodeUnknown)
        );
    }
    assert_eq!(
        refusal_labels(&fixture.owner, &referee_tenant).await,
        vec!["referral_code_unknown"; 5]
    );
    assert!(
        owner_texts(
            &fixture.owner,
            "SELECT tenant_id FROM trace_referral_attributions WHERE tenant_id = $1",
            &referee_tenant,
        )
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn an_account_too_old_for_the_program_is_refused() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, _) = fixture.code(&referrer_tenant, referrer).await;

    // Created 40 days ago, in a program that opened 100 days ago: past the
    // 30 days.
    let (late_tenant, late) = tenant_with_account(&fixture.owner).await;
    owner_exec(
        &fixture.owner,
        &late_tenant,
        "UPDATE trace_accounts SET created_at = now() - interval '40 days'
          WHERE account_id = $1",
        &[&late],
    )
    .await;
    let long_open = ReferralApplyPolicy {
        program_start: Some(Utc::now() - Duration::days(100)),
        program_end: None,
        late_days: 30,
        earn_days: 92,
    };
    assert_eq!(
        fixture
            .apply_with(&late_tenant, late, Some(&code), &long_open)
            .await,
        ReferralApply::Refused(ReferralRefusal::AccountTooOld)
    );
    // The same account inside a longer late window is fine.
    let generous = ReferralApplyPolicy {
        late_days: 45,
        ..long_open
    };
    assert!(matches!(
        fixture
            .apply_with(&late_tenant, late, Some(&code), &generous)
            .await,
        ReferralApply::Applied { .. }
    ));

    // Created before the program started.
    let (early_tenant, early) = tenant_with_account(&fixture.owner).await;
    owner_exec(
        &fixture.owner,
        &early_tenant,
        "UPDATE trace_accounts SET created_at = now() - interval '2 hours'
          WHERE account_id = $1",
        &[&early],
    )
    .await;
    let just_opened = ReferralApplyPolicy {
        program_start: Some(Utc::now() - Duration::hours(1)),
        program_end: None,
        late_days: 30,
        earn_days: 92,
    };
    assert_eq!(
        fixture
            .apply_with(&early_tenant, early, Some(&code), &just_opened)
            .await,
        ReferralApply::Refused(ReferralRefusal::AccountTooOld)
    );

    // Closed, or no account row at all.
    let (closed_tenant, closed) = tenant_with_account(&fixture.owner).await;
    owner_exec(
        &fixture.owner,
        &closed_tenant,
        "UPDATE trace_accounts SET closed_at = now() WHERE account_id = $1",
        &[&closed],
    )
    .await;
    assert_eq!(
        fixture.apply(&closed_tenant, closed, Some(&code)).await,
        ReferralApply::Refused(ReferralRefusal::AccountTooOld)
    );
    assert_eq!(
        fixture
            .apply(&closed_tenant, Uuid::new_v4(), Some(&code))
            .await,
        ReferralApply::Refused(ReferralRefusal::AccountTooOld)
    );
}

#[tokio::test]
async fn a_code_from_ones_own_tenant_is_self_referral() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let (own_code, _) = fixture.code(&tenant, accounts[0]).await;
    assert_eq!(
        fixture.apply(&tenant, accounts[0], Some(&own_code)).await,
        ReferralApply::Refused(ReferralRefusal::SelfReferral)
    );
    assert_eq!(
        fixture.apply(&tenant, accounts[1], Some(&own_code)).await,
        ReferralApply::Refused(ReferralRefusal::SelfReferral),
        "another account in the same tenant is the same person's"
    );
    assert_eq!(
        refusal_labels(&fixture.owner, &tenant).await,
        vec!["referral_self"; 2]
    );
}

#[tokio::test]
async fn the_checks_run_in_the_spec_order() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, _) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    assert!(matches!(
        fixture.apply(&referee_tenant, referee, Some(&code)).await,
        ReferralApply::Applied { .. }
    ));
    // Already referred, and also too old: already referred comes first.
    owner_exec(
        &fixture.owner,
        &referee_tenant,
        "UPDATE trace_accounts SET created_at = now() - interval '2 days'
          WHERE account_id = $1",
        &[&referee],
    )
    .await;
    let just_opened = ReferralApplyPolicy {
        program_start: Some(Utc::now() - Duration::hours(1)),
        program_end: None,
        late_days: 30,
        earn_days: 92,
    };
    assert_eq!(
        fixture
            .apply_with(&referee_tenant, referee, Some(&code), &just_opened)
            .await,
        ReferralApply::Refused(ReferralRefusal::AlreadyReferred)
    );
    // An unknown code comes before already referred.
    assert_eq!(
        fixture.apply(&referee_tenant, referee, None).await,
        ReferralApply::Refused(ReferralRefusal::CodeUnknown)
    );
    // Too old comes before self.
    let (tenant, accounts) = tenant_with_accounts(&fixture.owner, 2).await;
    let (own_code, _) = fixture.code(&tenant, accounts[0]).await;
    owner_exec(
        &fixture.owner,
        &tenant,
        "UPDATE trace_accounts SET created_at = now() - interval '2 days'
          WHERE account_id = $1",
        &[&accounts[1]],
    )
    .await;
    assert_eq!(
        fixture
            .apply_with(&tenant, accounts[1], Some(&own_code), &just_opened)
            .await,
        ReferralApply::Refused(ReferralRefusal::AccountTooOld)
    );
}

#[tokio::test]
async fn ten_refusals_in_a_day_hold_every_further_attempt() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, _) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    for _ in 0..10 {
        assert_eq!(
            fixture
                .apply(&referee_tenant, referee, Some(&generate_referral_code()))
                .await,
            ReferralApply::Refused(ReferralRefusal::CodeUnknown)
        );
    }
    // Even the right code is held now, and the hold is recorded too.
    assert_eq!(
        fixture.apply(&referee_tenant, referee, Some(&code)).await,
        ReferralApply::Refused(ReferralRefusal::RateLimited)
    );
    let labels = refusal_labels(&fixture.owner, &referee_tenant).await;
    assert_eq!(labels.len(), 11);
    assert_eq!(labels[10], "referral_rate_limited");

    // Another account is not held by this one's refusals.
    let (fresh_tenant, fresh) = tenant_with_account(&fixture.owner).await;
    assert!(matches!(
        fixture.apply(&fresh_tenant, fresh, Some(&code)).await,
        ReferralApply::Applied { .. }
    ));

    // Refusals older than a day no longer count.
    owner_exec(
        &fixture.owner,
        &referee_tenant,
        "UPDATE trace_referral_audit SET created_at = now() - interval '25 hours'
          WHERE action = 'referral_refused'",
        &[],
    )
    .await;
    assert!(matches!(
        fixture.apply(&referee_tenant, referee, Some(&code)).await,
        ReferralApply::Applied { .. }
    ));
}

#[tokio::test]
async fn a_code_disabled_between_resolve_and_apply_is_not_applied() {
    // The apply re-reads the code under the referrer's tenant inside its own
    // transaction. Simulated by disabling it with a lock held: the resolve
    // runs, then the disable commits before the apply's re-read.
    let Some(fixture) = fixture().await else {
        return;
    };
    let (referrer_tenant, referrer) = tenant_with_account(&fixture.owner).await;
    let (code, write) = fixture.code(&referrer_tenant, referrer).await;
    let (referee_tenant, referee) = tenant_with_account(&fixture.owner).await;
    // Hold the referee's apply lock so the apply blocks after resolving.
    let mut blocker = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let held = blocker.transaction().await.unwrap();
    held.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended(
             'referral-apply-v1:' || $1 || ':' || $2::UUID::TEXT, 0))",
        &[&referee_tenant, &referee],
    )
    .await
    .unwrap();
    let apply = {
        let runtime = fixture.runtime.clone();
        let index = referral_code_index(&fixture.identity, &code);
        let tenant = referee_tenant.clone();
        tokio::spawn(async move {
            runtime
                .apply_referral_code(
                    &tenant,
                    referee,
                    Some(&index),
                    ReferralAppliedVia::Later,
                    &open_policy(),
                )
                .await
                .unwrap()
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    fixture
        .runtime
        .disable_referral_code(&referrer_tenant, referrer, write.code_id)
        .await
        .unwrap();
    held.commit().await.unwrap();
    assert_eq!(
        apply.await.unwrap(),
        ReferralApply::Refused(ReferralRefusal::CodeUnknown)
    );
}

#[tokio::test]
async fn an_account_without_a_referral_has_only_its_creation_time() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let (tenant, account) = tenant_with_account(&fixture.owner).await;
    let status = fixture
        .runtime
        .referral_status(&tenant, account)
        .await
        .unwrap();
    assert_eq!(status.attribution, None);
    assert!(status.account_created_at.is_some());
    let missing = fixture
        .runtime
        .referral_status(&tenant, Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(missing.account_created_at, None);
}
