// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The passkey-account reaper (V99) against a real PostgreSQL, run as the
//! non-superuser login it runs as in production. Explicit isolated database
//! only; never falls back to DATABASE_URL.

use std::time::Duration;

use trace_commons_server::account_reaper::{ReapSummary, UnboundAccountReaper};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

const URL_VAR: &str = "TRACE_COMMONS_UNBOUND_REAPER_PG_TEST_DATABASE_URL";
const LOGIN: &str = "unbound_reaper_test_login";
const UNBOUND_TTL_DAYS: i64 = 7;
const CLOSED_TTL_DAYS: i64 = 30;

/// The sweep is cross-tenant, so tests share one database and run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Fixture {
    admin: deadpool_postgres::Object,
    reaper: UnboundAccountReaper,
    reaper_url: String,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: url.into(),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var(URL_VAR) else {
        eprintln!("SKIPPED: isolated PostgreSQL fixture required ({URL_VAR})");
        return None;
    };
    let serial = SERIAL.lock().await;
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_unbound_reaper"));
    let backend = PgBackend::new(&config(url.clone())).await.unwrap();
    backend.run_migrations().await.unwrap();
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin
        .batch_execute(&format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{LOGIN}') \
             THEN CREATE ROLE {LOGIN} LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; \
             GRANT trace_unbound_account_reaper TO {LOGIN}; \
             DROP TABLE IF EXISTS reaper_refusal_probe; \
             DELETE FROM trace_tenants; \
             DELETE FROM trace_audit_events;"
        ))
        .await
        .unwrap();
    let mut reaper_url = reqwest::Url::parse(&url).unwrap();
    reaper_url.set_username(LOGIN).unwrap();
    let reaper_url = reaper_url.to_string();
    let reaper = UnboundAccountReaper::connect(&reaper_url).unwrap();
    Some(Fixture {
        admin,
        reaper,
        reaper_url,
        _serial: serial,
    })
}

#[derive(Clone, Copy)]
enum Binding {
    Unbound,
    Bound,
    /// No binding row: a legacy account.
    Legacy,
}

/// One seeded account and the ids the assertions need.
struct Seeded {
    tenant: String,
    account: Uuid,
}

/// Seed a tenant holding an account, its credential, an audit row and, when
/// asked, one session `(seen_days_ago, expires_in_days, revoked)`.
/// `age_days` backdates the account and the binding's `created_at`.
async fn seed(
    fx: &Fixture,
    binding: Binding,
    age_days: i64,
    session: Option<(i64, i64, bool)>,
) -> Seeded {
    let tenant = format!("passkey-{}", Uuid::new_v4().simple());
    let account = Uuid::new_v4();
    let a = &fx.admin;
    a.execute(
        "INSERT INTO trace_tenants(tenant_id) VALUES ($1)",
        &[&tenant],
    )
    .await
    .unwrap();
    a.execute(
        "INSERT INTO trace_accounts(tenant_id, account_id, created_at)
         VALUES ($1, $2, now() - make_interval(days => $3::int))",
        &[&tenant, &account, &(age_days as i32)],
    )
    .await
    .unwrap();
    let state = match binding {
        Binding::Unbound => Some("unbound"),
        Binding::Bound => Some("bound"),
        Binding::Legacy => None,
    };
    if let Some(state) = state {
        a.execute(
            "INSERT INTO trace_account_bindings(tenant_id, account_id, origin, state, created_at, bound_at)
             VALUES ($1, $2, 'passkey', $3, now() - make_interval(days => $4::int),
                     CASE WHEN $3 = 'bound' THEN now() END)",
            &[&tenant, &account, &state, &(age_days as i32)],
        )
        .await
        .unwrap();
    }
    let credential = format!("cred-{}", Uuid::new_v4().simple());
    a.execute(
        "INSERT INTO trace_webauthn_credentials(tenant_id, credential_id, account_id, passkey)
         VALUES ($1, $2, $3, '{}'::jsonb)",
        &[&tenant, &credential, &account],
    )
    .await
    .unwrap();
    a.execute(
        "INSERT INTO trace_account_audit(tenant_id, action, actor_ref, outcome)
         VALUES ($1, 'passkey_created', 'label', 'ok')",
        &[&tenant],
    )
    .await
    .unwrap();
    a.execute(
        "INSERT INTO trace_audit_events(tenant_id, audit_sequence, audit_event_id,
            actor_principal_ref, actor_role, action)
         VALUES ($1, 1, $2, 'label', 'account', 'passkey_created')",
        &[&tenant, &Uuid::new_v4()],
    )
    .await
    .unwrap();
    if let Some((seen_days_ago, expires_in_days, revoked)) = session {
        insert_session(a, &tenant, account, seen_days_ago, expires_in_days, revoked).await;
    }
    Seeded { tenant, account }
}

async fn insert_session(
    client: &deadpool_postgres::Object,
    tenant: &str,
    account: Uuid,
    seen_days_ago: i64,
    expires_in_days: i64,
    revoked: bool,
) {
    let token_hash = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    client
        .execute(
            "INSERT INTO trace_sessions(tenant_id, session_id, account_id, token_hash,
                last_seen_at, expires_at, revoked_at)
             VALUES ($1, $2, $3, $4,
                     now() - make_interval(days => $5::int),
                     now() + make_interval(days => $6::int),
                     CASE WHEN $7 THEN now() END)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &account,
                &token_hash,
                &(seen_days_ago as i32),
                &(expires_in_days as i32),
                &revoked,
            ],
        )
        .await
        .unwrap();
}

/// Mirror what S2's create/finish leaves: the binding and exactly one native
/// session, both stamped with the creation transaction's now(), the session
/// with a 12 hour expiry. S5 is stacked on S1, so nothing here calls S2.
async fn seed_created(fx: &Fixture, created_days_ago: i64) -> Seeded {
    let s = seed(fx, Binding::Unbound, created_days_ago, None).await;
    insert_native_session(&fx.admin, &s, created_days_ago * 24, 0, 12).await;
    s
}

/// A native session created `created_hours_ago`, last seen `seen_hours_after`
/// hours after that, expiring `lifetime_hours` after creation.
async fn insert_native_session(
    client: &deadpool_postgres::Object,
    s: &Seeded,
    created_hours_ago: i64,
    seen_hours_after: i64,
    lifetime_hours: i64,
) {
    let token_hash = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    client
        .execute(
            "INSERT INTO trace_sessions(tenant_id, session_id, account_id, token_hash,
                client_kind, created_at, last_seen_at, expires_at)
             VALUES ($1, $2, $3, $4, 'native',
                     now() - make_interval(hours => $5::int),
                     now() - make_interval(hours => ($5::int - $6::int)),
                     now() - make_interval(hours => ($5::int - $7::int)))",
            &[
                &s.tenant,
                &Uuid::new_v4(),
                &s.account,
                &token_hash,
                &(created_hours_ago as i32),
                &(seen_hours_after as i32),
                &(lifetime_hours as i32),
            ],
        )
        .await
        .unwrap();
}

/// Close the account exactly as S3's refuse branch does
/// (`near_ai_bind_close_refused`, #1135): the binding goes `unbound` ->
/// `closed` with `bound_at` NULL, every session and credential is revoked,
/// `trace_accounts.closed_at` is set, and a label-only audit row is written.
/// Every timestamp S3 stamps with now() is backdated by `closed_days_ago`.
async fn close_like_s3(fx: &Fixture, s: &Seeded, closed_days_ago: i64) {
    fx.admin
        .execute(
            "WITH b AS (UPDATE trace_account_bindings SET state = 'closed'
                          WHERE tenant_id = $1 AND account_id = $2 AND state = 'unbound'
                        RETURNING 1),
                  s AS (UPDATE trace_sessions SET revoked_at = now() - make_interval(days => $3::int)
                          WHERE tenant_id = $1 AND account_id = $2 AND revoked_at IS NULL
                        RETURNING 1),
                  c AS (UPDATE trace_webauthn_credentials
                           SET revoked_at = now() - make_interval(days => $3::int)
                          WHERE tenant_id = $1 AND account_id = $2 AND revoked_at IS NULL
                        RETURNING 1),
                  a AS (UPDATE trace_accounts SET closed_at = now() - make_interval(days => $3::int)
                          WHERE tenant_id = $1 AND account_id = $2 AND closed_at IS NULL
                        RETURNING 1)
             INSERT INTO trace_account_audit(tenant_id, action, actor_ref, outcome, safe_metadata)
             SELECT $1, 'account_binding_refused', 'label', 'denied',
                    '{\"reason\":\"near_ai_account_exists\"}'::jsonb
              WHERE (SELECT count(*) FROM b) = 1 AND (SELECT count(*) FROM a) = 1",
            &[&s.tenant, &s.account, &(closed_days_ago as i32)],
        )
        .await
        .unwrap();
    let state: String = fx
        .admin
        .query_one(
            "SELECT state FROM trace_account_bindings WHERE tenant_id = $1",
            &[&s.tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(state, "closed", "fixture closed the account");
}

/// A closed account as S3 leaves it: created `created_days_ago`, used for a
/// native session, then closed `closed_days_ago`.
async fn seed_closed(fx: &Fixture, created_days_ago: i64, closed_days_ago: i64) -> Seeded {
    let s = seed(fx, Binding::Unbound, created_days_ago, None).await;
    // Created on the day of the close, still unexpired when it was revoked.
    insert_native_session(&fx.admin, &s, closed_days_ago * 24, 0, 12).await;
    close_like_s3(fx, &s, closed_days_ago).await;
    s
}

/// (tenants, accounts, bindings, credentials, sessions, account_audit, audit_events)
async fn rows(fx: &Fixture, tenant: &str) -> [i64; 7] {
    let row = fx
        .admin
        .query_one(
            "SELECT (SELECT count(*) FROM trace_tenants WHERE tenant_id = $1),
                    (SELECT count(*) FROM trace_accounts WHERE tenant_id = $1),
                    (SELECT count(*) FROM trace_account_bindings WHERE tenant_id = $1),
                    (SELECT count(*) FROM trace_webauthn_credentials WHERE tenant_id = $1),
                    (SELECT count(*) FROM trace_sessions WHERE tenant_id = $1),
                    (SELECT count(*) FROM trace_account_audit WHERE tenant_id = $1),
                    (SELECT count(*) FROM trace_audit_events WHERE tenant_id = $1)",
            &[&tenant],
        )
        .await
        .unwrap();
    std::array::from_fn(|i| row.get(i))
}

const WHOLE: [i64; 7] = [1, 1, 1, 1, 0, 1, 1];
/// After a reap: the account and the passkey-origin rows that cascade from it
/// (binding, credential, sessions) are gone. The tenant row is never deleted
/// (decided 2026-09-29), and the tenant-keyed audit rows stay with it.
const REAPED: [i64; 7] = [1, 0, 0, 0, 0, 1, 1];

impl Fixture {
    async fn reap(&self, limit: i32) -> ReapSummary {
        self.reaper
            .reap(UNBOUND_TTL_DAYS, CLOSED_TTL_DAYS, limit)
            .await
            .unwrap()
    }
}

fn counts(s: ReapSummary) -> (u64, u64, u64) {
    (s.reaped_unbound, s.reaped_closed, s.skipped)
}

#[tokio::test]
async fn an_unbound_account_past_the_ttl_is_deleted_with_all_its_rows() {
    let Some(fx) = fixture().await else { return };
    // Old sessions: expired, and one revoked.
    let target = seed(&fx, Binding::Unbound, 45, Some((40, -33, false))).await;
    insert_session(&fx.admin, &target.tenant, target.account, 35, 5, true).await;
    assert_eq!(
        rows(&fx, &target.tenant).await,
        [1, 1, 1, 1, 2, 1, 1],
        "fixture holds every row kind"
    );

    assert_eq!(counts(fx.reap(100).await), (1, 0, 0));
    // Account, binding, credential and sessions go. The tenant row stays,
    // and with it the tenant-keyed trace_account_audit and hash-chained
    // trace_audit_events rows.
    assert_eq!(rows(&fx, &target.tenant).await, REAPED);
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0), "idempotent");
}

/// Decision 1 (2026-09-29): reap on bound status, not on use. An account that
/// signed in again on day 2 used to move to a 30-day idle window; now it goes
/// at day 8 like any other unbound account, once its sessions are dead.
#[tokio::test]
async fn an_unbound_account_that_signed_in_again_is_reaped_at_eight_days() {
    let Some(fx) = fixture().await else { return };
    // Created 8 days ago; a second session (a later sign-in) on day 2, expired.
    let second_session = seed_created(&fx, 8).await;
    insert_native_session(&fx.admin, &second_session, 6 * 24, 0, 12).await;
    // Created 8 days ago; the creation session itself presented again on day 2.
    let same_session = seed(&fx, Binding::Unbound, 8, None).await;
    insert_native_session(&fx.admin, &same_session, 8 * 24, 2 * 24, 7 * 24).await;

    assert_eq!(counts(fx.reap(100).await), (2, 0, 0));
    for s in [&second_session, &same_session] {
        assert_eq!(rows(&fx, &s.tenant).await, REAPED);
    }
}

#[tokio::test]
async fn an_unbound_account_with_a_live_session_survives() {
    let Some(fx) = fixture().await else { return };
    // Same shape as above, but the day-2 sign-in holds a session that is
    // still unrevoked and unexpired.
    let live = seed_created(&fx, 8).await;
    insert_native_session(&fx.admin, &live, 6 * 24, 0, 7 * 24).await;
    // Unexpired but revoked is not live.
    let revoked = seed(&fx, Binding::Unbound, 90, Some((60, 2, true))).await;
    // Unrevoked but expired is not live.
    let expired = seed(&fx, Binding::Unbound, 90, Some((5, -1, false))).await;

    assert_eq!(counts(fx.reap(100).await), (2, 0, 0));
    assert_eq!(rows(&fx, &live.tenant).await[..5], [1, 1, 1, 1, 2]);
    assert_eq!(rows(&fx, &revoked.tenant).await, REAPED);
    assert_eq!(rows(&fx, &expired.tenant).await, REAPED);
}

#[tokio::test]
async fn an_unbound_account_is_reaped_at_eight_days_and_survives_at_six() {
    let Some(fx) = fixture().await else { return };
    let eight = seed_created(&fx, 8).await;
    let six = seed_created(&fx, 6).await;
    let six_no_session = seed(&fx, Binding::Unbound, 6, None).await;
    assert_eq!(counts(fx.reap(100).await), (1, 0, 0));
    assert_eq!(rows(&fx, &eight.tenant).await, REAPED);
    assert_eq!(rows(&fx, &six.tenant).await[..6], [1, 1, 1, 1, 1, 1]);
    assert_eq!(rows(&fx, &six_no_session.tenant).await, WHOLE);
}

#[tokio::test]
async fn a_bound_account_survives() {
    let Some(fx) = fixture().await else { return };
    let eight = seed(&fx, Binding::Bound, 8, None).await;
    insert_native_session(&fx.admin, &eight, 8 * 24, 0, 12).await;
    let old = seed(&fx, Binding::Bound, 400, None).await;
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0));
    assert_eq!(rows(&fx, &eight.tenant).await[..6], [1, 1, 1, 1, 1, 1]);
    assert_eq!(rows(&fx, &old.tenant).await, WHOLE);
}

#[tokio::test]
async fn a_legacy_account_survives() {
    let Some(fx) = fixture().await else { return };
    let legacy = seed(&fx, Binding::Legacy, 400, None).await;
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0));
    // A legacy account has no binding row and is never a candidate.
    assert_eq!(rows(&fx, &legacy.tenant).await, [1, 1, 0, 1, 0, 1, 1]);
}

/// Decision 2 (2026-09-29): a closed passkey account is deleted 30 days after
/// it was closed. The clock is `trace_accounts.closed_at`, which S3 sets in the
/// transaction that closes the binding, never the binding's `created_at`.
#[tokio::test]
async fn a_closed_account_is_reaped_at_thirty_one_days_and_survives_at_twenty_nine() {
    let Some(fx) = fixture().await else { return };
    let thirty_one = seed_closed(&fx, 400, 31).await;
    // Created long ago: only closed_at can keep it.
    let twenty_nine = seed_closed(&fx, 400, 29).await;
    assert_eq!(
        rows(&fx, &thirty_one.tenant).await,
        [1, 1, 1, 1, 1, 2, 1],
        "fixture holds what S3 leaves, plus its refusal audit row"
    );

    assert_eq!(counts(fx.reap(100).await), (0, 1, 0));
    assert_eq!(rows(&fx, &thirty_one.tenant).await, [1, 0, 0, 0, 0, 2, 1]);
    assert_eq!(rows(&fx, &twenty_nine.tenant).await, [1, 1, 1, 1, 1, 2, 1]);
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0), "idempotent");
}

/// A closed account is not held to the unbound window: 8 days after creation
/// and 2 days after closing, it survives.
#[tokio::test]
async fn a_recently_closed_account_is_not_reaped_on_the_unbound_window() {
    let Some(fx) = fixture().await else { return };
    let s = seed_closed(&fx, 8, 2).await;
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0));
    assert_eq!(rows(&fx, &s.tenant).await[..3], [1, 1, 1]);
}

/// Decided 2026-09-29: the reaper deletes the account, never the tenant. What
/// the tenant holds outside the account is left alone and does not refuse the
/// candidate; what cascades from the account (a login link here) goes with it.
#[tokio::test]
async fn the_tenant_row_and_its_other_rows_survive_a_reap() {
    let Some(fx) = fixture().await else { return };
    // A tenant-keyed row that is not the account's.
    let policy = seed(&fx, Binding::Unbound, 90, None).await;
    insert_tenant_policy(&fx, &policy.tenant).await;
    // An account-keyed row that cascades from the account.
    let login_link = seed(&fx, Binding::Unbound, 90, None).await;
    fx.admin
        .execute(
            "INSERT INTO trace_login_links(tenant_id, link_id, account_id, code_hash,
                created_principal_ref, expires_at)
             VALUES ($1, $2, $3, $4, 'label', now() - interval '80 days')",
            &[
                &login_link.tenant,
                &Uuid::new_v4(),
                &login_link.account,
                &format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2)),
            ],
        )
        .await
        .unwrap();
    // The same tenant-keyed row, for a closed account.
    let closed = seed_closed(&fx, 400, 31).await;
    insert_tenant_policy(&fx, &closed.tenant).await;

    assert_eq!(counts(fx.reap(100).await), (2, 1, 0));
    assert_eq!(rows(&fx, &policy.tenant).await, REAPED);
    assert_eq!(tenant_policies(&fx, &policy.tenant).await, 1, "kept");
    assert_eq!(rows(&fx, &login_link.tenant).await, REAPED);
    let links: i64 = fx
        .admin
        .query_one(
            "SELECT count(*) FROM trace_login_links WHERE tenant_id = $1",
            &[&login_link.tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(links, 0, "the login link cascades from the account");
    assert_eq!(rows(&fx, &closed.tenant).await, [1, 0, 0, 0, 0, 2, 1]);
    assert_eq!(tenant_policies(&fx, &closed.tenant).await, 1, "kept");
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0), "idempotent");
}

/// Decided 2026-09-29: a second account in the candidate's tenant does not
/// refuse it. The delete is keyed to the candidate's (tenant_id, account_id)
/// and cascades only from that row, so the other account, its binding,
/// credential and session, and the tenant row all survive.
#[tokio::test]
async fn another_account_in_the_tenant_neither_refuses_nor_is_touched() {
    let Some(fx) = fixture().await else { return };
    let target = seed(&fx, Binding::Unbound, 90, None).await;
    let other = Uuid::new_v4();
    fx.admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id, account_id) VALUES ($1, $2)",
            &[&target.tenant, &other],
        )
        .await
        .unwrap();
    fx.admin
        .execute(
            "INSERT INTO trace_account_bindings(tenant_id, account_id, origin, state, bound_at)
             VALUES ($1, $2, 'passkey', 'bound', now())",
            &[&target.tenant, &other],
        )
        .await
        .unwrap();
    fx.admin
        .execute(
            "INSERT INTO trace_webauthn_credentials(tenant_id, credential_id, account_id, passkey)
             VALUES ($1, $2, $3, '{}'::jsonb)",
            &[
                &target.tenant,
                &format!("cred-{}", Uuid::new_v4().simple()),
                &other,
            ],
        )
        .await
        .unwrap();
    insert_session(&fx.admin, &target.tenant, other, 0, 5, false).await;

    assert_eq!(counts(fx.reap(100).await), (1, 0, 0));
    // Only the other account's rows are left in the tenant.
    assert_eq!(rows(&fx, &target.tenant).await, [1, 1, 1, 1, 1, 1, 1]);
    let survivor: Uuid = fx
        .admin
        .query_one(
            "SELECT account_id FROM trace_accounts WHERE tenant_id = $1",
            &[&target.tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(survivor, other);
}

/// What a reap deletes is whatever cascades from trace_accounts. That set is
/// pinned here so a migration that adds or removes an ON DELETE CASCADE into
/// trace_accounts changes this list on purpose, in review, instead of silently
/// changing what the reaper deletes. It is not an obligation on the migration
/// (no grant, no policy), just one visible line.
#[tokio::test]
async fn the_account_cascade_set_is_pinned() {
    let Some(fx) = fixture().await else { return };
    let tables: Vec<String> = fx
        .admin
        .query(
            "SELECT DISTINCT conrelid::regclass::text FROM pg_constraint
              WHERE contype = 'f' AND confrelid = 'trace_accounts'::regclass
                AND confdeltype = 'c'
              ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    let mut expected = vec![
        // The passkey-origin rows a reap is meant to take.
        "trace_account_bindings",
        "trace_login_links",
        "trace_sessions",
        "trace_webauthn_credentials",
        // Behind the unbound gate an unbound or closed account has no route
        // that writes these, but if one holds any, the reap deletes them too.
        "trace_account_inference_connections",
        "trace_account_merge_proposals",
        "trace_account_principals",
        "trace_near_identities",
        "trace_public_runs",
        "trace_source_sessions",
    ];
    expected.sort_unstable();
    assert_eq!(tables, expected);
}

async fn insert_tenant_policy(fx: &Fixture, tenant: &str) {
    fx.admin
        .execute(
            "INSERT INTO trace_tenant_policies(tenant_id, policy_version, allowed_consent_scopes,
                allowed_uses, updated_by_principal_ref)
             VALUES ($1, 'v1', '[]'::jsonb, '[]'::jsonb, 'label')",
            &[&tenant],
        )
        .await
        .unwrap();
}

async fn tenant_policies(fx: &Fixture, tenant: &str) -> i64 {
    fx.admin
        .query_one(
            "SELECT count(*) FROM trace_tenant_policies WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn an_account_that_binds_during_the_sweep_survives() {
    let Some(fx) = fixture().await else { return };
    let target = seed(&fx, Binding::Unbound, 90, None).await;

    // The bind transaction has updated the row and not committed.
    let mut binder = fx.admin_connection().await;
    let tx = binder.transaction().await.unwrap();
    tx.execute(
        "UPDATE trace_account_bindings SET state = 'bound', bound_at = now()
          WHERE tenant_id = $1 AND account_id = $2",
        &[&target.tenant, &target.account],
    )
    .await
    .unwrap();
    assert_eq!(
        counts(fx.reap(100).await),
        (0, 0, 1),
        "locked row is skipped, not waited on"
    );
    tx.commit().await.unwrap();

    // After the commit the row is bound, so it is not a candidate at all.
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0));
    assert_eq!(rows(&fx, &target.tenant).await, WHOLE);
}

#[tokio::test]
async fn an_account_that_signs_in_during_the_sweep_survives() {
    let Some(fx) = fixture().await else { return };
    let target = seed_created(&fx, 8).await;

    // A sign-in has inserted a session and not committed: the foreign keys
    // hold key-share locks on the account and tenant rows.
    let mut login = fx.admin_connection().await;
    let tx = login.transaction().await.unwrap();
    tx.execute(
        "INSERT INTO trace_sessions(tenant_id, session_id, account_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, now() + interval '7 days')",
        &[
            &target.tenant,
            &Uuid::new_v4(),
            &target.account,
            &format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2)),
        ],
    )
    .await
    .unwrap();
    assert_eq!(counts(fx.reap(100).await), (0, 0, 1));
    tx.commit().await.unwrap();

    // Committed, the new session is live, so the account is not a candidate.
    assert_eq!(counts(fx.reap(100).await), (0, 0, 0));
    assert_eq!(rows(&fx, &target.tenant).await[..5], [1, 1, 1, 1, 2]);
}

/// An account-keyed insert that begins between the reaper's session re-check
/// and its delete waits on the account row lock the reaper holds, so it cannot
/// slip a row in to be cascaded away. Holding that lock is what can deadlock:
/// here a peer locks the candidate's session row first, the reaper's cascade
/// waits on it, and the peer then waits on the reaper's account lock. The reaper
/// must count that candidate skipped (40P01), not fail the batch.
#[tokio::test]
async fn a_deadlock_is_skipped() {
    let Some(fx) = fixture().await else { return };
    let superuser: bool = fx
        .admin
        .query_one(
            "SELECT rolsuper FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        superuser,
        "the deadlock test sets deadlock_timeout on its peer, which needs a superuser fixture"
    );
    let target = seed(&fx, Binding::Unbound, 90, Some((60, -50, false))).await;
    let clean = seed(&fx, Binding::Unbound, 10, None).await;

    let mut peer = fx.admin_connection().await;
    // The peer must never be the one that detects the cycle.
    peer.batch_execute("SET deadlock_timeout = '30s'")
        .await
        .unwrap();
    let tx = peer.transaction().await.unwrap();
    // A row lock on the candidate's (expired) session. A non-key update
    // takes no lock on the parent rows, so the reaper still locks them.
    tx.execute(
        "UPDATE trace_sessions SET last_seen_at = last_seen_at
          WHERE tenant_id = $1 AND account_id = $2",
        &[&target.tenant, &target.account],
    )
    .await
    .unwrap();

    let reaper = fx.reaper.clone();
    let sweep =
        tokio::spawn(async move { reaper.reap(UNBOUND_TTL_DAYS, CLOSED_TTL_DAYS, 100).await });
    // Wait until the reaper's cascade is blocked on the peer's session lock.
    let mut waited = false;
    for _ in 0..200 {
        let blocked: bool = fx
            .admin
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                                 WHERE usename = $1 AND wait_event_type = 'Lock')",
                &[&LOGIN],
            )
            .await
            .unwrap()
            .get(0);
        if blocked {
            waited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        waited,
        "the reaper never blocked on the peer's session lock"
    );
    // Close the cycle: a new session needs a key-share lock on the account
    // row the reaper holds FOR UPDATE. This returns once the reaper's
    // subtransaction is rolled back by the deadlock detector.
    tx.execute(
        "INSERT INTO trace_sessions(tenant_id, session_id, account_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, now() + interval '7 days')",
        &[
            &target.tenant,
            &Uuid::new_v4(),
            &target.account,
            &format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2)),
        ],
    )
    .await
    .unwrap();
    let summary = sweep.await.unwrap();
    tx.rollback().await.unwrap();

    let summary = summary.expect("a deadlock is a skip, not a failed batch");
    assert_eq!(counts(summary), (1, 0, 1));
    assert_eq!(rows(&fx, &clean.tenant).await, REAPED);
    assert_eq!(rows(&fx, &target.tenant).await[..5], [1, 1, 1, 1, 1]);
}

#[tokio::test]
async fn batches_are_bounded() {
    let Some(fx) = fixture().await else { return };
    let mut tenants = Vec::new();
    for i in 0..3 {
        tenants.push(seed(&fx, Binding::Unbound, 100 + i, None).await.tenant);
    }
    for i in 0..2 {
        tenants.push(seed_closed(&fx, 400, 40 + i).await.tenant);
    }
    let mut reaped = Vec::new();
    for _ in 0..4 {
        reaped.push(fx.reap(2).await.reaped());
    }
    assert_eq!(reaped, vec![2, 2, 1, 0]);
    for tenant in &tenants {
        assert_eq!(rows(&fx, tenant).await[1], 0);
    }
}

/// Candidates whose delete is permanently refused sit at the head of the
/// order. They must not starve a deletable candidate behind them, even when
/// they outnumber `limit`.
#[tokio::test]
async fn permanently_refused_candidates_do_not_stall_the_batch() {
    let Some(fx) = fixture().await else { return };
    // Stand-in for an account-keyed table holding something of value, with a
    // foreign key that does not cascade: deleting a holder's account raises
    // 23503, which refuses that candidate. Dropped before the assertions so a
    // failure cannot poison the shared fixture.
    fx.admin
        .batch_execute(
            "CREATE TABLE reaper_refusal_probe (
                 tenant_id TEXT NOT NULL,
                 account_id UUID NOT NULL,
                 FOREIGN KEY (tenant_id, account_id)
                     REFERENCES trace_accounts (tenant_id, account_id)
             );",
        )
        .await
        .unwrap();
    let limit = 2;
    let mut refused = Vec::new();
    for i in 0..(limit as i64 + 1) {
        // Older than the deletable one, so they lead the order.
        let s = seed(&fx, Binding::Unbound, 300 + i, None).await;
        fx.admin
            .execute(
                "INSERT INTO reaper_refusal_probe(tenant_id, account_id) VALUES ($1, $2)",
                &[&s.tenant, &s.account],
            )
            .await
            .unwrap();
        refused.push(s);
    }
    let deletable = seed(&fx, Binding::Unbound, 100, None).await;

    let first = fx
        .reaper
        .reap(UNBOUND_TTL_DAYS, CLOSED_TTL_DAYS, limit)
        .await;
    let second = fx
        .reaper
        .reap(UNBOUND_TTL_DAYS, CLOSED_TTL_DAYS, limit)
        .await;
    let deletable_rows = rows(&fx, &deletable.tenant).await;
    let mut refused_rows = Vec::new();
    for s in &refused {
        refused_rows.push(rows(&fx, &s.tenant).await[1]);
    }
    fx.admin
        .batch_execute("DROP TABLE reaper_refusal_probe")
        .await
        .unwrap();

    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(deletable_rows, REAPED, "the deletable candidate was reaped");
    assert_eq!(counts(first), (1, 0, refused.len() as u64));
    assert_eq!(second.reaped(), 0);
    assert_eq!(
        refused_rows,
        vec![1; refused.len()],
        "refused accounts kept"
    );
}

#[tokio::test]
async fn the_function_refuses_a_ttl_under_a_day_and_an_unbounded_batch() {
    let Some(fx) = fixture().await else { return };
    let target = seed(&fx, Binding::Unbound, 90, None).await;
    let client = fx.reaper_client().await;
    let day = 86_400_i64;
    for (unbound, closed, limit) in [
        (3600_i64, day, 10_i32),
        (day, 3600, 10),
        (day, day, 0),
        (day, day, 1001),
    ] {
        let error = client
            .query(
                "SELECT * FROM trace_reap_unbound_accounts($1, $2, $3)",
                &[&unbound, &closed, &limit],
            )
            .await
            .expect_err("refused");
        assert_eq!(
            error.as_db_error().map(|e| e.message()),
            Some("unbound_reaper_refused")
        );
    }
    assert_eq!(rows(&fx, &target.tenant).await[1], 1);
}

/// The reaper login must be the narrow role it is in production, and the pass
/// must depend on the guard's own policies: a superuser or BYPASSRLS owner
/// would delete regardless, and this fails for it.
#[tokio::test]
async fn it_works_as_the_non_superuser_role_and_not_by_bypassing_row_security() {
    let Some(fx) = fixture().await else { return };
    let client = fx.reaper_client().await;
    let privileged: bool = client
        .query_one(
            "SELECT rolsuper OR rolbypassrls OR rolcreaterole
               FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!privileged, "the reaper login must be unprivileged");
    let owner_privileged: bool = fx
        .admin
        .query_one(
            "SELECT r.rolsuper OR r.rolbypassrls
               FROM pg_proc p JOIN pg_roles r ON r.oid = p.proowner
              WHERE p.proname = 'trace_reap_unbound_accounts'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!owner_privileged, "the function owner must not bypass RLS");
    // The login itself has no table access at all.
    let direct = client.query("SELECT 1 FROM trace_accounts", &[]).await;
    assert!(
        direct.is_err(),
        "the login must hold EXECUTE and nothing else"
    );
    let direct = client.execute("DELETE FROM trace_tenants", &[]).await;
    assert!(direct.is_err());

    let target = seed(&fx, Binding::Unbound, 90, None).await;
    // Control: remove the guard's cross-tenant read policy. A definer that
    // bypasses row security would still delete; the real one sees nothing.
    fx.admin
        .batch_execute(
            "ALTER POLICY trace_unbound_reaper_read ON trace_account_bindings USING (FALSE)",
        )
        .await
        .unwrap();
    let blind = fx.reaper.reap(UNBOUND_TTL_DAYS, CLOSED_TTL_DAYS, 100).await;
    fx.admin
        .batch_execute(
            "ALTER POLICY trace_unbound_reaper_read ON trace_account_bindings USING (TRUE)",
        )
        .await
        .unwrap();
    assert_eq!(
        blind.unwrap().reaped(),
        0,
        "the sweep is subject to row security"
    );
    assert_eq!(rows(&fx, &target.tenant).await[1], 1);

    assert_eq!(counts(fx.reap(100).await), (1, 0, 0));
    assert_eq!(rows(&fx, &target.tenant).await, REAPED);
}

/// Boot does one `pool.get()` on the reaper pool, so a login that cannot
/// connect fails boot rather than the first tick.
#[tokio::test]
async fn a_bad_reaper_login_fails_the_startup_check() {
    let Some(fx) = fixture().await else { return };
    fx.reaper
        .verify_login()
        .await
        .expect("the real login connects");
    let mut bad = reqwest::Url::parse(&fx.reaper_url).unwrap();
    bad.set_username("unbound_reaper_no_such_login").unwrap();
    let bad = UnboundAccountReaper::connect(bad.as_str()).expect("builds lazily");
    assert!(bad.verify_login().await.is_err());
}

/// Re-applying V99 changes nothing and leaves the reaper working. The
/// non-superuser migrator case is covered by migration_atomicity_pg.
#[tokio::test]
async fn reapplying_the_migration_changes_nothing() {
    let Some(fx) = fixture().await else { return };
    let target = seed(&fx, Binding::Unbound, 90, None).await;
    let closed = seed_closed(&fx, 400, 31).await;
    fx.admin
        .batch_execute(include_str!(
            "../../../migrations/V99__unbound_account_reaper.sql"
        ))
        .await
        .unwrap();
    assert_eq!(counts(fx.reap(100).await), (1, 1, 0));
    assert_eq!(rows(&fx, &target.tenant).await, REAPED);
    assert_eq!(rows(&fx, &closed.tenant).await, [1, 0, 0, 0, 0, 2, 1]);
}

/// A database that ran the earlier draft of V99 (CI and scratch only) held a
/// guard grant and a `trace_unbound_reaper_scope` policy on every tenant-keyed
/// table, and grants and policies on trace_tenants. Re-applying V99 must
/// remove all of it, leaving the guard with privileges and policies on exactly
/// the three tables it reads.
#[tokio::test]
async fn reapplying_the_migration_removes_the_earlier_drafts_scope_grants() {
    let Some(fx) = fixture().await else { return };
    // Recreate what the earlier draft left, on a sample of tables.
    fx.admin
        .batch_execute(
            "GRANT SELECT (tenant_id), DELETE ON trace_tenants TO trace_unbound_account_reaper_guard;
             GRANT UPDATE (created_at) ON trace_tenants TO trace_unbound_account_reaper_guard;
             DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_tenants;
             CREATE POLICY trace_unbound_reaper_read ON trace_tenants
                 FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
             DROP POLICY IF EXISTS trace_unbound_reaper_lock ON trace_tenants;
             CREATE POLICY trace_unbound_reaper_lock ON trace_tenants
                 FOR UPDATE TO trace_unbound_account_reaper_guard USING (TRUE) WITH CHECK (TRUE);
             DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_tenants;
             CREATE POLICY trace_unbound_reaper_delete ON trace_tenants
                 FOR DELETE TO trace_unbound_account_reaper_guard USING (TRUE);
             GRANT SELECT (tenant_id) ON trace_tenant_policies TO trace_unbound_account_reaper_guard;
             DROP POLICY IF EXISTS trace_unbound_reaper_scope ON trace_tenant_policies;
             CREATE POLICY trace_unbound_reaper_scope ON trace_tenant_policies
                 FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
             GRANT SELECT (tenant_id) ON trace_login_links TO trace_unbound_account_reaper_guard;
             DROP POLICY IF EXISTS trace_unbound_reaper_scope ON trace_login_links;
             CREATE POLICY trace_unbound_reaper_scope ON trace_login_links
                 FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);",
        )
        .await
        .unwrap();
    assert_eq!(
        guard_tables(&fx).await.0.len(),
        6,
        "the earlier draft's policies are in place"
    );
    fx.admin
        .batch_execute(include_str!(
            "../../../migrations/V99__unbound_account_reaper.sql"
        ))
        .await
        .unwrap();

    let three = ["trace_account_bindings", "trace_accounts", "trace_sessions"];
    let (policy_tables, granted) = guard_tables(&fx).await;
    assert_eq!(policy_tables, three, "policies left on other tables");
    assert_eq!(granted, three, "grants left on other tables");

    let target = seed(&fx, Binding::Unbound, 90, None).await;
    assert_eq!(counts(fx.reap(100).await), (1, 0, 0));
    assert_eq!(rows(&fx, &target.tenant).await, REAPED);
}

/// (tables with a policy naming the guard, tables the guard holds any grant
/// on), each sorted and distinct.
async fn guard_tables(fx: &Fixture) -> (Vec<String>, Vec<String>) {
    let policy_tables = fx
        .admin
        .query(
            "SELECT DISTINCT p.polrelid::regclass::text FROM pg_policy p
               JOIN pg_roles r ON r.oid = ANY (p.polroles)
              WHERE r.rolname = 'trace_unbound_account_reaper_guard'
              ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect::<Vec<String>>();
    let granted = fx
        .admin
        .query(
            "SELECT table_name::text FROM information_schema.column_privileges
              WHERE grantee = 'trace_unbound_account_reaper_guard'
             UNION
             SELECT table_name::text FROM information_schema.table_privileges
              WHERE grantee = 'trace_unbound_account_reaper_guard'
             ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect::<Vec<String>>();
    (policy_tables, granted)
}

impl Fixture {
    async fn admin_connection(&self) -> deadpool_postgres::Object {
        let url = std::env::var(URL_VAR).unwrap();
        let backend = PgBackend::new(&config(url)).await.unwrap();
        // Leak the backend so the pooled connection outlives this call.
        let backend: &'static PgBackend = Box::leak(Box::new(backend));
        backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap()
    }

    async fn reaper_client(&self) -> tokio_postgres::Client {
        let (client, connection) = tokio_postgres::connect(&self.reaper_url, tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(connection);
        client
    }
}
