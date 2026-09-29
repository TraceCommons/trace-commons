// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The unbound-account reaper (V99) against a real PostgreSQL, run as the
//! non-superuser login it runs as in production. Explicit isolated database
//! only; never falls back to DATABASE_URL.

use trace_commons_server::account_reaper::UnboundAccountReaper;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

const URL_VAR: &str = "TRACE_COMMONS_UNBOUND_REAPER_PG_TEST_DATABASE_URL";
const LOGIN: &str = "unbound_reaper_test_login";
const TTL_DAYS: i64 = 30;

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
    Closed,
    /// No binding row: a legacy account.
    Legacy,
}

/// One seeded account and the ids the assertions need.
struct Seeded {
    tenant: String,
    account: Uuid,
}

/// Seed a tenant holding an account, its credential, an audit row and, when
/// asked, one session. `idle_days` backdates the binding and the session;
/// `session_expires_in_days` is negative for an expired session.
async fn seed(
    fx: &Fixture,
    binding: Binding,
    idle_days: i64,
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
        &[&tenant, &account, &(idle_days as i32)],
    )
    .await
    .unwrap();
    let state = match binding {
        Binding::Unbound => Some("unbound"),
        Binding::Bound => Some("bound"),
        Binding::Closed => Some("closed"),
        Binding::Legacy => None,
    };
    if let Some(state) = state {
        a.execute(
            "INSERT INTO trace_account_bindings(tenant_id, account_id, origin, state, created_at, bound_at)
             VALUES ($1, $2, 'passkey', $3, now() - make_interval(days => $4::int),
                     CASE WHEN $3 = 'bound' THEN now() END)",
            &[&tenant, &account, &state, &(idle_days as i32)],
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

#[tokio::test]
async fn an_idle_unbound_account_past_the_ttl_is_deleted_with_all_its_rows() {
    let Some(fx) = fixture().await else { return };
    // Old sessions: idle beyond the TTL, expired, and one revoked.
    let target = seed(&fx, Binding::Unbound, 45, Some((40, -33, false))).await;
    insert_session(&fx.admin, &target.tenant, target.account, 35, 5, true).await;
    let before = rows(&fx, &target.tenant).await;
    assert_eq!(
        before,
        [1, 1, 1, 1, 2, 1, 1],
        "fixture holds every row kind"
    );

    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!((summary.reaped, summary.skipped), (1, 0));
    // Tenant, account, binding, credential, sessions and account audit go.
    // The hash-chained trace_audit_events row is retained, as it is for every
    // other deletion in the repo.
    assert_eq!(rows(&fx, &target.tenant).await, [0, 0, 0, 0, 0, 0, 1]);

    let again = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!((again.reaped, again.skipped), (0, 0), "idempotent");
}

#[tokio::test]
async fn an_unbound_account_with_no_session_uses_the_binding_created_at() {
    let Some(fx) = fixture().await else { return };
    let old = seed(&fx, Binding::Unbound, 31, None).await;
    let young = seed(&fx, Binding::Unbound, 29, None).await;
    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!(summary.reaped, 1);
    assert_eq!(rows(&fx, &old.tenant).await[1], 0);
    assert_eq!(rows(&fx, &young.tenant).await[..6], WHOLE[..6]);
}

#[tokio::test]
async fn a_recently_active_or_live_account_survives() {
    let Some(fx) = fixture().await else { return };
    // Old binding, session last seen 5 days ago (expired since).
    let recent = seed(&fx, Binding::Unbound, 90, Some((5, -1, false))).await;
    // Old binding, last seen long ago, but the session is still unrevoked and unexpired.
    let live = seed(&fx, Binding::Unbound, 90, Some((60, 2, false))).await;
    // Same, but revoked and expired: idle, so it goes.
    let dead = seed(&fx, Binding::Unbound, 90, Some((60, -50, true))).await;
    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!((summary.reaped, summary.skipped), (1, 0));
    assert_eq!(rows(&fx, &recent.tenant).await[..5], [1, 1, 1, 1, 1]);
    assert_eq!(rows(&fx, &live.tenant).await[..5], [1, 1, 1, 1, 1]);
    assert_eq!(rows(&fx, &dead.tenant).await[..5], [0, 0, 0, 0, 0]);
}

#[tokio::test]
async fn bound_closed_and_legacy_accounts_survive() {
    let Some(fx) = fixture().await else { return };
    let bound = seed(&fx, Binding::Bound, 400, None).await;
    let closed = seed(&fx, Binding::Closed, 400, None).await;
    let legacy = seed(&fx, Binding::Legacy, 400, None).await;
    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!((summary.reaped, summary.skipped), (0, 0));
    assert_eq!(rows(&fx, &bound.tenant).await[..6], WHOLE[..6]);
    assert_eq!(rows(&fx, &closed.tenant).await[..6], WHOLE[..6]);
    // A legacy account has no binding row and is never a candidate.
    assert_eq!(rows(&fx, &legacy.tenant).await[..6], [1, 1, 0, 1, 0, 1]);
}

#[tokio::test]
async fn a_tenant_holding_another_account_keeps_its_tenant() {
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
    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!(summary.reaped, 1);
    let left = rows(&fx, &target.tenant).await;
    assert_eq!((left[0], left[1], left[2]), (1, 1, 0));
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
    let mid = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!(
        (mid.reaped, mid.skipped),
        (0, 1),
        "locked row is skipped, not waited on"
    );
    tx.commit().await.unwrap();

    // After the commit the row is bound, so it is not a candidate at all.
    let after = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!((after.reaped, after.skipped), (0, 0));
    assert_eq!(rows(&fx, &target.tenant).await[..6], WHOLE[..6]);
}

#[tokio::test]
async fn an_account_that_signs_in_during_the_sweep_survives() {
    let Some(fx) = fixture().await else { return };
    let target = seed(&fx, Binding::Unbound, 90, None).await;

    // A sign-in has inserted a session and not committed: the foreign key
    // holds a key-share lock on the account row.
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
    let mid = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!((mid.reaped, mid.skipped), (0, 1));
    tx.commit().await.unwrap();

    let after = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!(after.reaped, 0);
    assert_eq!(rows(&fx, &target.tenant).await[..5], [1, 1, 1, 1, 1]);
}

#[tokio::test]
async fn batches_are_bounded() {
    let Some(fx) = fixture().await else { return };
    let mut tenants = Vec::new();
    for i in 0..5 {
        tenants.push(seed(&fx, Binding::Unbound, 100 + i, None).await.tenant);
    }
    let mut reaped = Vec::new();
    for _ in 0..4 {
        reaped.push(fx.reaper.reap(TTL_DAYS, 2).await.unwrap().reaped);
    }
    assert_eq!(reaped, vec![2, 2, 1, 0]);
    for tenant in &tenants {
        assert_eq!(rows(&fx, tenant).await[1], 0);
    }
}

/// Candidates whose delete is permanently refused (a foreign key from a table
/// the cascade does not reach) sit at the head of the order. They must not
/// starve a deletable candidate behind them, even when they outnumber `limit`.
#[tokio::test]
async fn permanently_refused_candidates_do_not_stall_the_batch() {
    let Some(fx) = fixture().await else { return };
    // Stand-in for a non-cascading dependent table. Dropped before the
    // assertions so a failure cannot poison the shared fixture.
    fx.admin
        .batch_execute("DROP TABLE IF EXISTS reaper_refusal_probe")
        .await
        .unwrap();
    fx.admin
        .batch_execute(
            "CREATE TABLE reaper_refusal_probe (
                 tenant_id TEXT NOT NULL,
                 account_id UUID NOT NULL,
                 FOREIGN KEY (tenant_id, account_id)
                     REFERENCES trace_accounts (tenant_id, account_id)
             )",
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

    let first = fx.reaper.reap(TTL_DAYS, limit).await;
    let second = fx.reaper.reap(TTL_DAYS, limit).await;
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
    assert_eq!(deletable_rows[1], 0, "the deletable candidate was reaped");
    assert_eq!(first.reaped, 1);
    assert_eq!(first.skipped, refused.len() as u64);
    assert_eq!(second.reaped, 0);
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
    for (ttl, limit) in [(3600_i64, 10_i32), (86_400, 0), (86_400, 1001)] {
        let error = client
            .query(
                "SELECT * FROM trace_reap_unbound_accounts($1, $2)",
                &[&ttl, &limit],
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
    let blind = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    fx.admin
        .batch_execute(
            "ALTER POLICY trace_unbound_reaper_read ON trace_account_bindings USING (TRUE)",
        )
        .await
        .unwrap();
    assert_eq!(blind.reaped, 0, "the sweep is subject to row security");
    assert_eq!(rows(&fx, &target.tenant).await[1], 1);

    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!(summary.reaped, 1);
    assert_eq!(rows(&fx, &target.tenant).await[..6], [0; 6]);
}

/// Re-applying V99 changes nothing and leaves the reaper working. The
/// non-superuser migrator case is covered by migration_atomicity_pg.
#[tokio::test]
async fn reapplying_the_migration_changes_nothing() {
    let Some(fx) = fixture().await else { return };
    let target = seed(&fx, Binding::Unbound, 90, None).await;
    fx.admin
        .batch_execute(include_str!(
            "../../../migrations/V99__unbound_account_reaper.sql"
        ))
        .await
        .unwrap();
    let summary = fx.reaper.reap(TTL_DAYS, 100).await.unwrap();
    assert_eq!(summary.reaped, 1);
    assert_eq!(rows(&fx, &target.tenant).await[1], 0);
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
