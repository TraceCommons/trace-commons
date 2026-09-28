// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Postgres-backed tests for the V42 invite-grant table.
//!
//! Skipped unless TRACE_COMMONS_PG_TEST_DATABASE_URL (or DATABASE_URL) is set.
//! The `database suites against a real PostgreSQL` CI job sets it and runs
//! them; run them locally against a real PostgreSQL too, because with the
//! variable unset every test here returns early and the run still prints ok.

use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};

// DatabaseConfig has no Default impl and secrecy 0.10 uses From, not new.
// This mirrors postgres_test_config() in tests/trace_corpus_pg_store.rs
// exactly; keep the two in step when fields are added.
fn postgres_test_config() -> Option<DatabaseConfig> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    Some(DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: DatabaseConfig::login_resolver_url_from_env(),
        gate_driver_url: DatabaseConfig::gate_driver_url_from_env(),
        pii_backstop_driver_url: DatabaseConfig::pii_backstop_driver_url_from_env(),
        invite_registry_url: DatabaseConfig::invite_registry_url_from_env(),
    })
}

// Every test in this file runs against the same database, and cargo runs the
// suite in parallel by default. Fixed subject hashes and a shared
// `policy_label` therefore let the tests destroy each other's fixtures:
// `cleanup_test_invites` deletes by `policy_label`, so a test starting up
// wiped rows a test already running had just inserted, and two tests inserting
// the same subject hash collided on the primary key. Each test now mints its
// own hashes and its own policy label, so isolation is a property of the
// fixtures rather than of scheduling luck. Every assertion is unchanged -- each
// still asserts about exactly the rows its own test wrote.
static NEXT_FIXTURE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn fixture_nonce() -> u128 {
    let n = NEXT_FIXTURE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    (u128::from(std::process::id()) << 64) | u128::from(n)
}

/// A canonical `sha256:<64 lowercase hex>` value distinct from every other
/// value this process mints.
fn unique_hash() -> String {
    format!("sha256:{:064x}", fixture_nonce())
}

/// A `policy_label` distinct from every other label this process mints, so a
/// per-label cleanup can only ever touch the calling test's own rows.
fn unique_label(prefix: &str) -> String {
    format!("{prefix}-{:032x}", fixture_nonce())
}

/// Insert two rows, then read them back as a NON-superuser role with the
/// invite_lookup policy in force. Without the GUC set the role must see
/// nothing; with it set the role must see exactly the matching row.
#[tokio::test]
async fn invite_lookup_policy_confines_reads_to_the_presented_hash() {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let hash_a = unique_hash();
    let hash_b = unique_hash();
    let policy_label = unique_label("test-pool");

    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");

    // Provision the stand-in runtime role this test reads as. It is a test
    // fixture, not a production role, so no migration creates it -- and the
    // GRANT is per-database, so it has to be (re)applied against whatever
    // database the run points at, not just the first one it ever ran on.
    if let Err(error) = client
        .batch_execute(
            "DO $$ BEGIN
                 IF NOT EXISTS (
                     SELECT 1 FROM pg_roles
                      WHERE rolname = 'trace_invite_registry_test_reader'
                 ) THEN
                     CREATE ROLE trace_invite_registry_test_reader NOLOGIN NOBYPASSRLS;
                 END IF;
             END $$;",
        )
        .await
    {
        if error.code() == Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE) {
            eprintln!("skipping: CREATEROLE privilege unavailable for invite reader fixture");
            return;
        }
        panic!("create NOBYPASSRLS reader fixture role: {error}");
    }
    client
        .batch_execute(
            "GRANT USAGE ON SCHEMA public TO trace_invite_registry_test_reader;
             GRANT SELECT ON onboarding_invite_grants
                 TO trace_invite_registry_test_reader;",
        )
        .await
        .expect("grant access to the NOBYPASSRLS reader fixture role");

    // Seed through the registry role. Do NOT seed as the connection's own
    // user: local dev users are frequently superusers, which bypasses RLS
    // outright and would make this test pass for the wrong reason. The
    // runtime role has a SELECT-only policy and cannot insert at all.
    let seed = client.transaction().await.expect("seed tx");
    seed.batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");
    for hash in [&hash_a, &hash_b] {
        seed.execute(
            "INSERT INTO onboarding_invite_grants (
                     invite_subject_hash, policy_label, tenant_mode,
                     tenant_template_id, policy_version, issuance_source
                 ) VALUES ($1, $2, 'derived', 'tmpl-1', 'v1', 'operator')
                 ON CONFLICT (invite_subject_hash) DO NOTHING",
            &[hash, &policy_label],
        )
        .await
        .expect("seed");
    }
    seed.commit().await.expect("seed commit");

    let tx = client.transaction().await.expect("tx");
    // A NOBYPASSRLS role: policies actually apply. Superuser would not.
    tx.batch_execute("SET LOCAL ROLE trace_invite_registry_test_reader")
        .await
        .expect("set role");

    // No GUC set: the invite_lookup predicate matches nothing.
    let rows = tx
        .query(
            "SELECT invite_subject_hash FROM onboarding_invite_grants",
            &[],
        )
        .await
        .expect("query");
    assert_eq!(
        rows.len(),
        0,
        "runtime role must see no invites without trace_commons.invite_subject set"
    );

    // GUC set to A: exactly one row, and it is A.
    tx.execute(
        "SELECT set_config('trace_commons.invite_subject', $1, true)",
        &[&hash_a],
    )
    .await
    .expect("set guc");
    let rows = tx
        .query(
            "SELECT invite_subject_hash FROM onboarding_invite_grants",
            &[],
        )
        .await
        .expect("query");
    assert_eq!(rows.len(), 1, "exactly the presented invite is visible");
    assert_eq!(rows[0].get::<_, String>(0), hash_a);

    tx.rollback().await.expect("rollback");
}

/// The registry role's permissive policy must expose every row, so cache
/// refresh and admin listing work.
#[tokio::test]
async fn registry_role_sees_all_invites() {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let hash_a = unique_hash();
    let hash_b = unique_hash();
    let policy_label = unique_label("test-pool");

    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");
    // Seed through the registry role, not the connection's own user.
    let seed = client.transaction().await.expect("seed tx");
    seed.batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");
    for hash in [&hash_a, &hash_b] {
        seed.execute(
            "INSERT INTO onboarding_invite_grants (
                     invite_subject_hash, policy_label, tenant_mode,
                     tenant_template_id, policy_version, issuance_source
                 ) VALUES ($1, $2, 'derived', 'tmpl-1', 'v1', 'operator')
                 ON CONFLICT (invite_subject_hash) DO NOTHING",
            &[hash, &policy_label],
        )
        .await
        .expect("seed");
    }
    seed.commit().await.expect("seed commit");

    let tx = client.transaction().await.expect("tx");
    tx.batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");
    let rows = tx
        .query(
            "SELECT invite_subject_hash FROM onboarding_invite_grants
              WHERE invite_subject_hash IN ($1, $2)",
            &[&hash_a, &hash_b],
        )
        .await
        .expect("query");
    assert_eq!(rows.len(), 2, "registry role must see all invites");
    tx.rollback().await.expect("rollback");
}

/// The tenant_mode pairing constraint must reject every wrong combination.
#[tokio::test]
async fn tenant_mode_pairing_constraint_rejects_mismatches() {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let hash_a = unique_hash();
    let hash_b = unique_hash();
    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");
    let client = client.transaction().await.expect("tx");
    client
        .batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");

    // fixed mode with no fixed_tenant_id
    let err = client
        .execute(
            "INSERT INTO onboarding_invite_grants (
                 invite_subject_hash, policy_label, tenant_mode,
                 policy_version, issuance_source
             ) VALUES ($1, 'p', 'fixed', 'v1', 'operator')",
            &[&hash_a],
        )
        .await;
    assert!(err.is_err(), "fixed mode requires fixed_tenant_id");

    // derived mode carrying a fixed_tenant_id
    let err = client
        .execute(
            "INSERT INTO onboarding_invite_grants (
                 invite_subject_hash, policy_label, tenant_mode,
                 tenant_template_id, fixed_tenant_id, policy_version, issuance_source
             ) VALUES ($1, 'p', 'derived', 'tmpl', 'tenant-x', 'v1', 'operator')",
            &[&hash_b],
        )
        .await;
    assert!(err.is_err(), "derived mode must not carry fixed_tenant_id");
}

use trace_commons_server::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use trace_commons_server::trace_invite_registry::InviteTenantMode;

fn registry_test_config() -> Option<DatabaseConfig> {
    let mut config = postgres_test_config()?;
    // The registry pool is required for these tests. Fall back to the same
    // URL when no dedicated registry URL is configured locally; the RLS tests
    // above are what prove the roles are distinct in production.
    let registry_url = std::env::var("TRACE_COMMONS_INVITE_REGISTRY_TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok())
        .or_else(|| std::env::var("DATABASE_URL").ok())?;
    config.invite_registry_url = Some(SecretString::from(registry_url));
    Some(config)
}

/// Teardown only. Runs as the test connection's own user, which must own the
/// table or be a superuser -- FORCE RLS gives the runtime role no DELETE policy
/// and production deliberately has no DELETE grant (revocation is a soft
/// UPDATE). Never use this to set up an assertion; seed through the
/// trace_invite_registry role for that.
async fn cleanup_test_invites(backend: &PgBackend, policy_label: &str) {
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = $1",
            &[&policy_label],
        )
        .await
        .expect("cleanup");
}

fn derived_write(hash: &str, policy_label: &str) -> InviteGrantWrite {
    InviteGrantWrite {
        invite_subject_hash: hash.to_string(),
        policy_label: policy_label.to_string(),
        tenant_mode: InviteTenantMode::Derived,
        fixed_tenant_id: None,
        tenant_template_id: Some("tmpl-1".to_string()),
        policy_version: "v1".to_string(),
        allowed_consent_scopes: vec!["model_training".to_string()],
        allowed_uses: vec!["research".to_string()],
        max_uses: 3,
        expires_at: None,
        issuance_source: "operator".to_string(),
        issued_by_label: None,
        credential_binding_hash: None,
        note_label: None,
        issuer_display_name: None,
        credit_range: None,
    }
}

/// Fixture for `insert_then_list_round_trips_every_field`. Every string field
/// gets a distinct, recognizable value (never sharing a value with another
/// field, e.g. two fields both set to "v1") so a transposition bug -- say,
/// `policy_version` and `issuance_source` swapped in the INSERT column list
/// or the row mapper -- would fail the round-trip instead of passing
/// silently. `expires_at` and `credential_binding_hash` are set to non-None
/// values so both are actually exercised by the assertions.
fn round_trip_fixture(hash: &str, policy_label: &str, credential: &str) -> InviteGrantWrite {
    InviteGrantWrite {
        invite_subject_hash: hash.to_string(),
        policy_label: policy_label.to_string(),
        tenant_mode: InviteTenantMode::Derived,
        fixed_tenant_id: None,
        tenant_template_id: Some("tmpl-round-trip".to_string()),
        policy_version: "policy-version-round-trip".to_string(),
        allowed_consent_scopes: vec!["model_training".to_string()],
        allowed_uses: vec!["research".to_string()],
        max_uses: 3,
        // A fixed, whole-second timestamp (not `Utc::now()`): `timestamptz`
        // truncates to microsecond precision, and `Utc::now()` carries
        // nanoseconds, which would make the round-trip equality assertion
        // below flaky.
        expires_at: Some(
            chrono::DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
                .expect("parse fixture expires_at")
                .with_timezone(&chrono::Utc),
        ),
        issuance_source: "issuance-source-round-trip".to_string(),
        issued_by_label: Some("issued-by-round-trip".to_string()),
        credential_binding_hash: Some(credential.to_string()),
        note_label: Some("note-label-round-trip".to_string()),
        issuer_display_name: Some("display-name-round-trip".to_string()),
        credit_range: Some((11, 29)),
    }
}

#[tokio::test]
async fn insert_then_list_round_trips_every_field() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    let credential = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let fixture = round_trip_fixture(&hash_c, &policy_label, &credential);
    let expected_expires_at = fixture.expires_at.expect("fixture sets expires_at");
    let outcome = backend.insert_invite_grant(fixture).await.expect("insert");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    let all = backend.list_invite_grants().await.expect("list");
    let found = all
        .iter()
        .find(|e| e.invite_subject_hash == hash_c)
        .expect("inserted invite present in listing");
    assert_eq!(found.invite_subject_hash, hash_c);
    assert_eq!(found.policy_label, policy_label);
    assert_eq!(found.tenant_mode, InviteTenantMode::Derived);
    assert_eq!(found.tenant_template_id.as_deref(), Some("tmpl-round-trip"));
    assert_eq!(found.fixed_tenant_id, None);
    assert_eq!(found.policy_version, "policy-version-round-trip");
    assert_eq!(found.allowed_consent_scopes, vec!["model_training"]);
    assert_eq!(found.allowed_uses, vec!["research"]);
    assert_eq!(found.max_uses, 3);
    assert_eq!(found.expires_at, Some(expected_expires_at));
    assert_eq!(found.issuance_source, "issuance-source-round-trip");
    assert_eq!(
        found.issued_by_label.as_deref(),
        Some("issued-by-round-trip")
    );
    assert_eq!(
        found.credential_binding_hash.as_deref(),
        Some(credential.as_str())
    );
    assert_eq!(found.note_label.as_deref(), Some("note-label-round-trip"));
    assert_eq!(
        found.issuer_display_name.as_deref(),
        Some("display-name-round-trip")
    );
    assert_eq!(found.credit_range, Some((11, 29)));
    assert!(found.revoked_at.is_none());
}

#[tokio::test]
async fn a_second_live_invite_for_one_credential_is_refused() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    let hash_d = unique_hash();
    let credential = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let mut first = derived_write(&hash_c, &policy_label);
    first.credential_binding_hash = Some(credential.clone());
    let _ = backend
        .insert_invite_grant(first)
        .await
        .expect("first insert");

    let mut second = derived_write(&hash_d, &policy_label);
    second.credential_binding_hash = Some(credential.clone());
    let outcome = backend
        .insert_invite_grant(second)
        .await
        .expect("second insert must not error");
    assert!(
        matches!(outcome, InviteGrantInsertOutcome::CredentialAlreadyBound),
        "one credential must not mint two live invites"
    );
}

#[tokio::test]
async fn revoking_frees_the_credential_binding_for_reissue() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    let hash_d = unique_hash();
    let credential = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let mut first = derived_write(&hash_c, &policy_label);
    first.credential_binding_hash = Some(credential.clone());
    let _ = backend.insert_invite_grant(first).await.expect("insert");

    let revoked = backend.revoke_invite_grant(&hash_c).await.expect("revoke");
    assert!(revoked, "revoking a live invite reports true");

    let mut second = derived_write(&hash_d, &policy_label);
    second.credential_binding_hash = Some(credential.clone());
    let outcome = backend.insert_invite_grant(second).await.expect("reissue");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    // Revoking an already-revoked invite is a no-op, not an error.
    let again = backend
        .revoke_invite_grant(&hash_c)
        .await
        .expect("second revoke");
    assert!(!again);
}

#[tokio::test]
async fn listing_excludes_revoked_invites() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let _ = backend
        .insert_invite_grant(derived_write(&hash_c, &policy_label))
        .await
        .expect("insert");
    let _ = backend.revoke_invite_grant(&hash_c).await.expect("revoke");

    let all = backend.list_invite_grants().await.expect("list");
    assert!(
        !all.iter().any(|e| e.invite_subject_hash == hash_c),
        "cache refresh must not load revoked invites"
    );
}

/// Mirrors `listing_excludes_revoked_invites`, but for expiry: `list_invite_grants`
/// feeds a cache of LIVE invites only, so a row whose `expires_at` is already in
/// the past must never appear, exactly like a revoked row. Dropping the
/// `expires_at` predicate from the query would let an expired invite into the
/// cache and offer it as redeemable, so this must be covered independently
/// of the revoked-row test above.
#[tokio::test]
async fn listing_excludes_expired_invites() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let mut expired = derived_write(&hash_c, &policy_label);
    expired.expires_at = Some(
        chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .expect("parse fixture expires_at")
            .with_timezone(&chrono::Utc),
    );
    let outcome = backend
        .insert_invite_grant(expired)
        .await
        .expect("insert expired invite");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    let all = backend.list_invite_grants().await.expect("list");
    assert!(
        !all.iter().any(|e| e.invite_subject_hash == hash_c),
        "cache refresh must not load expired invites"
    );
}

use trace_commons_protocol::onboarding::derive_user_tenant_id;

#[tokio::test]
async fn a_derived_mode_invite_provisions_the_derived_tenant() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let _ = backend
        .insert_invite_grant(derived_write(&hash_c, &policy_label))
        .await
        .expect("insert");

    let outcome = backend
        .redeem_invite_grant(&hash_c, "user-subject-1")
        .await
        .expect("no database error")
        .expect("invite is redeemable");

    assert_eq!(
        outcome.tenant_id,
        derive_user_tenant_id("tmpl-1", "user-subject-1"),
        "derived mode must resolve the tenant from template + user subject"
    );
    assert_eq!(outcome.allowed_consent_scopes, vec!["model_training"]);
    assert_eq!(outcome.allowed_uses, vec!["research"]);
    assert_eq!(outcome.policy_version, "v1");
}

#[tokio::test]
async fn a_fixed_mode_invite_uses_its_tenant_verbatim() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let mut write = derived_write(&hash_c, &policy_label);
    write.tenant_mode = InviteTenantMode::Fixed;
    write.tenant_template_id = None;
    write.fixed_tenant_id = Some("tenant-zaki-pilot".to_string());
    let _ = backend.insert_invite_grant(write).await.expect("insert");

    let outcome = backend
        .redeem_invite_grant(&hash_c, "user-subject-1")
        .await
        .expect("no database error")
        .expect("invite is redeemable");
    assert_eq!(outcome.tenant_id, "tenant-zaki-pilot");
}

#[tokio::test]
async fn a_revoked_invite_cannot_be_redeemed_even_from_a_warm_cache() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let _ = backend
        .insert_invite_grant(derived_write(&hash_c, &policy_label))
        .await
        .expect("insert");
    let _ = backend.revoke_invite_grant(&hash_c).await.expect("revoke");

    // The cache is deliberately bypassed here: this asserts the database, not
    // the cache, is what refuses a revoked invite.
    let result = backend
        .redeem_invite_grant(&hash_c, "user-subject-1")
        .await
        .expect("a revoked invite is not a database error");
    assert!(
        result.is_none(),
        "the in-transaction re-check must refuse a revoked invite"
    );
}

use trace_commons_server::trace_invite_registry::import_file_invites;

/// Substitutes the calling test's own policy label and subject hashes into an
/// allowlist fixture. The literals in these fixtures are placeholders for that
/// reason: two import tests running concurrently would otherwise write the same
/// primary key under the same label and delete each other's rows.
fn allowlist_fixture(template: &str, policy_label: &str, hashes: &[&str]) -> String {
    let mut rendered = template.replace("POLICY_LABEL", policy_label);
    for (index, hash) in hashes.iter().enumerate() {
        rendered = rendered.replace(&format!("SUBJECT_HASH_{}", index + 1), hash);
    }
    rendered
}

#[tokio::test]
async fn importing_the_same_file_twice_is_idempotent() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    let hash_1 = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(
            r#"{
            "version": 1,
            "generated_at": "2026-05-17T18:00:00Z",
            "policy_label": "POLICY_LABEL",
            "entries": [
                {"subject_hash": "SUBJECT_HASH_1",
                 "tenant_id": "tenant-zaki-pilot", "note_label": "batch-1", "max_uses": 3},
                {"kind": "instance", "instance_id": "inst-1",
                 "instance_public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                 "max_enrollments": 5,
                 "policy_template": {"policy_version": "v1",
                                     "allowed_consent_scopes": [], "allowed_uses": []}}
            ]
        }"#,
            &policy_label,
            &[&hash_1],
        ),
    )
    .expect("write file");

    let first = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect("first import");
    assert_eq!(first.imported, 1);
    assert_eq!(first.already_present, 0);
    assert_eq!(
        first.skipped_non_invite, 1,
        "instance entries stay in the file and must not be imported"
    );

    let created_at: chrono::DateTime<chrono::Utc> = client
        .query_one(
            "SELECT created_at FROM onboarding_invite_grants WHERE policy_label = $1",
            &[&policy_label],
        )
        .await
        .expect("row")
        .get(0);

    let second = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect("second import");
    assert_eq!(second.imported, 0);
    assert_eq!(second.already_present, 1);

    let created_at_after: chrono::DateTime<chrono::Utc> = client
        .query_one(
            "SELECT created_at FROM onboarding_invite_grants WHERE policy_label = $1",
            &[&policy_label],
        )
        .await
        .expect("row")
        .get(0);
    assert_eq!(
        created_at, created_at_after,
        "re-import must not rewrite the row"
    );
}

#[tokio::test]
async fn imported_invites_are_fixed_mode_and_keep_their_tenant() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    let hash_1 = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(
            r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"POLICY_LABEL",
            "entries":[{"subject_hash":"SUBJECT_HASH_1",
            "tenant_id":"tenant-zaki-pilot","note_label":"batch-1","max_uses":3}]}"#,
            &policy_label,
            &[&hash_1],
        ),
    )
    .expect("write file");

    let _ = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect("import");

    let all = backend.list_invite_grants().await.expect("list");
    let found = all
        .iter()
        .find(|e| e.policy_label == policy_label)
        .expect("imported invite present");
    assert_eq!(found.tenant_mode, InviteTenantMode::Fixed);
    assert_eq!(found.fixed_tenant_id.as_deref(), Some("tenant-zaki-pilot"));
    assert_eq!(found.max_uses, 3);
    assert_eq!(found.note_label.as_deref(), Some("batch-1"));
    assert_eq!(found.issuance_source, "import:file");
}

#[tokio::test]
async fn import_fails_when_the_file_does_not_exist() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("does-not-exist.json");

    let err = import_file_invites(&backend, &missing, &policy_label)
        .await
        .expect_err("missing file must fail");
    assert!(
        err.to_string().contains("does-not-exist.json"),
        "error should name the file path: {err}"
    );

    let all = backend.list_invite_grants().await.expect("list");
    assert!(
        !all.iter().any(|e| e.policy_label == policy_label),
        "a failure before any read must leave zero rows behind"
    );
}

#[tokio::test]
async fn import_fails_on_invalid_json() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(&path, "this is not json").expect("write file");

    let err = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect_err("invalid JSON must fail");
    assert!(
        err.to_string().contains("allowlist.json"),
        "error should name the file path: {err}"
    );

    let all = backend.list_invite_grants().await.expect("list");
    assert!(
        !all.iter().any(|e| e.policy_label == policy_label),
        "a failure before any write must leave zero rows behind"
    );
}

#[tokio::test]
async fn import_fails_on_unsupported_version() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(r#"{"version":2,"generated_at":"2026-05-17T18:00:00Z","policy_label":"POLICY_LABEL","entries":[]}"#, &policy_label, &[]),
    )
    .expect("write file");

    let err = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect_err("unsupported version must fail");
    assert!(err.to_string().contains("PilotAllowlistMalformed"));

    let all = backend.list_invite_grants().await.expect("list");
    assert!(!all.iter().any(|e| e.policy_label == policy_label));
}

#[tokio::test]
async fn import_fails_when_an_invite_entry_is_missing_subject_hash() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(
            r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"POLICY_LABEL",
            "entries":[{"tenant_id":"tenant-zaki-pilot","note_label":"batch-1","max_uses":3}]}"#,
            &policy_label,
            &[],
        ),
    )
    .expect("write file");

    let err = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect_err("missing subject_hash must fail");
    assert!(err.to_string().contains("missing subject_hash"), "{err}");
    assert!(err.to_string().contains("entry 1"), "{err}");

    let all = backend.list_invite_grants().await.expect("list");
    assert!(!all.iter().any(|e| e.policy_label == policy_label));
}

#[tokio::test]
async fn import_fails_when_an_invite_entry_is_missing_tenant_id() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    let hash_1 = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(
            r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"POLICY_LABEL",
            "entries":[{"subject_hash":"SUBJECT_HASH_1",
            "note_label":"batch-1","max_uses":3}]}"#,
            &policy_label,
            &[&hash_1],
        ),
    )
    .expect("write file");

    let err = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect_err("missing tenant_id must fail");
    assert!(err.to_string().contains("missing tenant_id"), "{err}");

    let all = backend.list_invite_grants().await.expect("list");
    assert!(!all.iter().any(|e| e.policy_label == policy_label));
}

#[tokio::test]
async fn import_fails_on_a_malformed_subject_hash_shape() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    cleanup_test_invites(&backend, &policy_label).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"POLICY_LABEL",
            "entries":[{"subject_hash":"not-a-sha256","tenant_id":"tenant-zaki-pilot","max_uses":3}]}"#, &policy_label, &[]),
    )
    .expect("write file");

    let err = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect_err("malformed subject_hash must fail");
    // Must name the entry position, never echo the offending hash value.
    assert!(err.to_string().contains("entry 1"), "{err}");
    assert!(
        !err.to_string().contains("not-a-sha256"),
        "error must not echo the malformed hash value: {err}"
    );

    let all = backend.list_invite_grants().await.expect("list");
    assert!(!all.iter().any(|e| e.policy_label == policy_label));
}

#[tokio::test]
async fn a_mid_import_failure_reports_the_partial_summary() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("import-test");
    let hash_1 = unique_hash();
    let hash_2 = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    // Two good invites, one instance entry, then a bad invite at entry 4:
    // the good entries must land before the failure and be reflected in the
    // error message's counts.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        allowlist_fixture(
            r#"{
            "version": 1,
            "generated_at": "2026-05-17T18:00:00Z",
            "policy_label": "POLICY_LABEL",
            "entries": [
                {"subject_hash": "SUBJECT_HASH_1",
                 "tenant_id": "tenant-zaki-pilot", "max_uses": 3},
                {"subject_hash": "SUBJECT_HASH_2",
                 "tenant_id": "tenant-zaki-pilot", "max_uses": 3},
                {"kind": "instance", "instance_id": "inst-1",
                 "instance_public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                 "max_enrollments": 5,
                 "policy_template": {"policy_version": "v1",
                                     "allowed_consent_scopes": [], "allowed_uses": []}},
                {"subject_hash": "not-a-sha256", "tenant_id": "tenant-zaki-pilot", "max_uses": 3}
            ]
        }"#,
            &policy_label,
            &[&hash_1, &hash_2],
        ),
    )
    .expect("write file");

    let err = import_file_invites(&backend, &path, &policy_label)
        .await
        .expect_err("the fourth entry is malformed and must fail the whole import");
    let message = err.to_string();
    assert!(
        message.contains("imported=2"),
        "must report the two invites already committed: {message}"
    );
    assert!(message.contains("already_present=0"), "message: {message}");
    assert!(
        message.contains("skipped_non_invite=1"),
        "must report the instance entry counted before the failure: {message}"
    );
    assert!(message.contains("entry 4"), "message: {message}");

    // The two good entries must actually be in the database -- the error
    // return must not roll them back, since re-running is what recovers.
    let all = backend.list_invite_grants().await.expect("list");
    let count = all
        .iter()
        .filter(|e| e.policy_label == policy_label)
        .count();
    assert_eq!(
        count, 2,
        "the two valid entries committed before the failure must persist"
    );
}

#[tokio::test]
async fn import_refuses_reserved_fixed_tenant_after_partial_progress() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    for reserved in ["near-", "nearai-", "NeAr-?!", "NEARAI-abc"] {
        let policy_label = unique_label("reserved-import");
        let hash_1 = unique_hash();
        let hash_2 = unique_hash();
        let hash_3 = unique_hash();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        let file = serde_json::json!({
            "version": 1,
            "generated_at": "2026-05-17T18:00:00Z",
            "policy_label": policy_label,
            "entries": [
                {"subject_hash": hash_1, "tenant_id": "nearby-1", "max_uses": 3},
                {"subject_hash": hash_2, "tenant_id": reserved, "max_uses": 3},
                {"subject_hash": hash_3, "tenant_id": "tenant-zaki-pilot", "max_uses": 3}
            ]
        });
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).expect("write file");

        let err = import_file_invites(&backend, &path, &policy_label)
            .await
            .expect_err("reserved fixed tenant must stop the import");
        let message = err.to_string();
        assert!(message.contains("imported=1"), "message: {message}");
        assert!(message.contains("entry 2"), "message: {message}");
        assert!(
            message.contains("PilotAllowlistMalformed: fixed tenant namespace reserved"),
            "message: {message}"
        );
        assert!(!message.contains(reserved), "tenant ID leaked: {message}");
        assert!(!message.contains(&hash_2), "subject hash leaked: {message}");

        let all = backend.list_invite_grants().await.expect("list");
        let imported: Vec<_> = all
            .iter()
            .filter(|entry| entry.policy_label == policy_label)
            .collect();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].invite_subject_hash, hash_1);
        assert_eq!(imported[0].fixed_tenant_id.as_deref(), Some("nearby-1"));
    }
}

#[tokio::test]
async fn an_expired_invite_cannot_be_redeemed() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let policy_label = unique_label("test-pool");
    let hash_c = unique_hash();
    cleanup_test_invites(&backend, &policy_label).await;

    let mut write = derived_write(&hash_c, &policy_label);
    write.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    let _ = backend.insert_invite_grant(write).await.expect("insert");

    let result = backend
        .redeem_invite_grant(&hash_c, "user-subject-1")
        .await
        .expect("an expired invite is not a database error");
    assert!(result.is_none(), "an expired invite must not redeem");
}

// ---- V96 public face and the non-redeeming lookup route (#1118 Z1) ----

mod lookup_route {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;
    use trace_commons_server::trace_invite_registry::DbInviteRegistry;
    use trace_commons_server::trace_upload_claim_allowlist::hash_invite_code;
    use trace_commons_server::trace_upload_claim_issuer::{
        TraceUploadClaimIssuerConfig, generate_upload_claim_keypair,
        trace_upload_claim_issuer_router,
    };

    const SECRET_POLICY_SUFFIX: &str = "policy-never-shown";
    const SECRET_ISSUED_BY: &str = "issued-by-never-shown";
    const SECRET_NOTE: &str = "note-never-shown";

    #[derive(Clone, Default)]
    struct LogSink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {
        type Writer = LogSink;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// A 16-character code in the invite alphabet, distinct per call.
    fn unique_code() -> String {
        let n = fixture_nonce();
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut code = String::new();
        let mut rest = n;
        for _ in 0..16 {
            code.push(alphabet[(rest % 26) as usize] as char);
            rest /= 26;
            if rest == 0 {
                rest = n.rotate_left(7) | 1;
            }
        }
        code
    }

    fn write_for(code: &str, policy_label: &str) -> InviteGrantWrite {
        let mut write = derived_write(&hash_invite_code(code), policy_label);
        write.issued_by_label = Some(SECRET_ISSUED_BY.to_string());
        write.note_label = Some(SECRET_NOTE.to_string());
        write.issuer_display_name = Some("Trace Commons Pilot".to_string());
        write.credit_range = Some((2, 6));
        write
    }

    async fn router_for(backend: Arc<PgBackend>) -> axum::Router {
        let issuer_keys = generate_upload_claim_keypair().unwrap();
        let workload_keys = generate_upload_claim_keypair().unwrap();
        let registry = Arc::new(
            DbInviteRegistry::new(
                backend.clone(),
                std::time::Duration::from_secs(60),
                std::time::Duration::from_secs(600),
            )
            .await
            .expect("registry warms"),
        );
        trace_upload_claim_issuer_router(TraceUploadClaimIssuerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            signing_private_key_pem: issuer_keys.private_key_pem.clone(),
            signing_public_key_pem: issuer_keys.public_key_pem.clone(),
            signing_kid: "issuer-test".into(),
            issuer: "trace-commons-upload-issuer".into(),
            audience: "trace-commons-upload".into(),
            max_ttl_seconds: 300,
            workload_public_key_pem: workload_keys.public_key_pem.clone(),
            workload_issuer: None,
            workload_audience: None,
            tenant_access_grant_db: None,
            require_tenant_access_grants: false,
            shutdown_grace_seconds: 30,
            request_timeout_seconds: 10,
            max_request_bytes: 64 * 1024,
            allowlist_source: None,
            allowlist_refresh_interval_seconds: 60,
            allowlist_max_stale_seconds: 3600,
            onboarding_device_key_db: None,
            onboarding_ingest_url: None,
            onboarding_community_url: None,
            onboarding_profile_url: None,
            onboarding_leaderboard_url: None,
            admin_bind: None,
            invite_admin_backend: Some(backend),
            invite_admin_registry: Some(registry),
            invite_registry_authoritative: true,
        })
        .expect("router")
    }

    async fn lookup(router: &axum::Router, code: &str) -> (StatusCode, serde_json::Value, String) {
        let body = serde_json::json!({
            "schema_version": trace_commons_protocol::invite_lookup::INVITE_LOOKUP_REQUEST_SCHEMA_VERSION,
            "invite_code": code,
        })
        .to_string();
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/invite/lookup")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        (status, serde_json::from_str(&text).unwrap(), text)
    }

    async fn stored_uses(backend: &PgBackend, hash: &str) -> (i32, chrono::DateTime<chrono::Utc>) {
        let client = backend.trace_pool_for_test().get().await.unwrap();
        let row = client
            .query_one(
                "SELECT consumed_uses, updated_at FROM onboarding_invite_grants
                  WHERE invite_subject_hash = $1",
                &[&hash],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    }

    #[tokio::test]
    async fn lookup_names_the_issuer_and_spends_nothing() {
        let Some(config) = registry_test_config() else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let backend = Arc::new(PgBackend::new(&config).await.expect("backend"));
        backend.run_migrations().await.expect("migrations");
        let policy_label = unique_label(SECRET_POLICY_SUFFIX);
        let code = unique_code();
        let hash = hash_invite_code(&code);
        assert_eq!(
            backend
                .insert_invite_grant(write_for(&code, &policy_label))
                .await
                .unwrap(),
            InviteGrantInsertOutcome::Inserted
        );
        let router = router_for(backend.clone()).await;
        let before = stored_uses(&backend, &hash).await;

        let sink = LogSink::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(sink.clone())
                .with_max_level(tracing::Level::TRACE)
                .finish(),
        );
        // A callsite another test thread cached as disabled would otherwise
        // ignore this scoped subscriber.
        tracing::callsite::rebuild_interest_cache();
        for _ in 0..7 {
            let (status, body, text) = lookup(&router, &code).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            assert_eq!(
                body,
                serde_json::json!({
                    "valid": true,
                    "issuer_display_name": "Trace Commons Pilot",
                    "credit_range": {"min": 2, "max": 6, "unit": "points_per_accepted_trace"},
                })
            );
            for secret in [
                SECRET_ISSUED_BY,
                SECRET_NOTE,
                SECRET_POLICY_SUFFIX,
                "max_uses",
            ] {
                assert!(!text.contains(secret), "{secret} leaked: {text}");
            }
        }

        // max_uses is 3 and we looked up 7 times: the stored count, and the
        // whole row's updated_at, are exactly as seeded.
        assert_eq!(stored_uses(&backend, &hash).await, before);
        assert_eq!(before.0, 0);

        // The log names the invite by subject hash and never by code.
        let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(logged.contains(&hash), "hash missing from log: {logged}");
        assert!(!logged.contains(&code), "code reached the log: {logged}");

        cleanup_test_invites(&backend, &policy_label).await;
    }

    #[tokio::test]
    async fn lookup_labels_each_way_an_invite_can_be_unusable() {
        let Some(config) = registry_test_config() else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let backend = Arc::new(PgBackend::new(&config).await.expect("backend"));
        backend.run_migrations().await.expect("migrations");
        let policy_label = unique_label(SECRET_POLICY_SUFFIX);

        let revoked = unique_code();
        let expired = unique_code();
        let exhausted = unique_code();
        let unknown = unique_code();
        for code in [&revoked, &expired, &exhausted] {
            backend
                .insert_invite_grant(write_for(code, &policy_label))
                .await
                .unwrap();
        }
        assert!(
            backend
                .revoke_invite_grant(&hash_invite_code(&revoked))
                .await
                .unwrap()
        );
        {
            let client = backend.trace_pool_for_test().get().await.unwrap();
            client
                .execute(
                    "UPDATE onboarding_invite_grants
                        SET expires_at = NOW() - INTERVAL '1 hour'
                      WHERE invite_subject_hash = $1",
                    &[&hash_invite_code(&expired)],
                )
                .await
                .unwrap();
            client
                .execute(
                    "UPDATE onboarding_invite_grants SET consumed_uses = max_uses
                      WHERE invite_subject_hash = $1",
                    &[&hash_invite_code(&exhausted)],
                )
                .await
                .unwrap();
        }

        let router = router_for(backend.clone()).await;
        for (code, label) in [
            (&revoked, "revoked"),
            (&expired, "expired"),
            (&exhausted, "exhausted"),
            (&unknown, "not_found"),
        ] {
            let (status, body, text) = lookup(&router, code).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            assert_eq!(
                body,
                serde_json::json!({"valid": false, "reason_label": label}),
                "{text}"
            );
        }

        cleanup_test_invites(&backend, &policy_label).await;
    }

    #[tokio::test]
    async fn the_database_refuses_a_backwards_or_half_set_range_and_a_bad_name() {
        let Some(config) = registry_test_config() else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let backend = Arc::new(PgBackend::new(&config).await.expect("backend"));
        backend.run_migrations().await.expect("migrations");
        let policy_label = unique_label("range-check");

        let mut backwards = write_for(&unique_code(), &policy_label);
        backwards.credit_range = Some((6, 2));
        assert!(backend.insert_invite_grant(backwards).await.is_err());

        let mut negative = write_for(&unique_code(), &policy_label);
        negative.credit_range = Some((-1, 2));
        assert!(backend.insert_invite_grant(negative).await.is_err());

        let mut padded = write_for(&unique_code(), &policy_label);
        padded.issuer_display_name = Some(" padded ".to_string());
        assert!(backend.insert_invite_grant(padded).await.is_err());

        let mut newline = write_for(&unique_code(), &policy_label);
        newline.issuer_display_name = Some("two\nlines".to_string());
        assert!(backend.insert_invite_grant(newline).await.is_err());

        // Half-set: the write type cannot express it, so go around it.
        let client = backend.trace_pool_for_test().get().await.unwrap();
        let hash = hash_invite_code(&unique_code());
        let half = client
            .execute(
                "INSERT INTO onboarding_invite_grants
                    (invite_subject_hash, policy_label, tenant_mode, tenant_template_id,
                     policy_version, issuance_source, credit_range_min)
                 VALUES ($1, $2, 'derived', 'tmpl', 'v1', 'operator', 1)",
                &[&hash, &policy_label],
            )
            .await;
        assert!(half.is_err(), "min without max must be refused");

        // A valid write with neither field still lands, and so does an
        // equal-bounds range.
        let mut bare = write_for(&unique_code(), &policy_label);
        bare.issuer_display_name = None;
        bare.credit_range = None;
        assert_eq!(
            backend.insert_invite_grant(bare).await.unwrap(),
            InviteGrantInsertOutcome::Inserted
        );
        let mut flat = write_for(&unique_code(), &policy_label);
        flat.credit_range = Some((3, 3));
        assert_eq!(
            backend.insert_invite_grant(flat).await.unwrap(),
            InviteGrantInsertOutcome::Inserted
        );

        cleanup_test_invites(&backend, &policy_label).await;
    }
}
