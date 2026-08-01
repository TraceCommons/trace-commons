//! Postgres-backed tests for the V42 invite-grant table.
//!
//! Skipped unless TRACE_COMMONS_PG_TEST_DATABASE_URL (or DATABASE_URL) is set.
//! CI does not run these; run them locally against a real PostgreSQL.

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

const TEST_HASH_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEST_HASH_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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

    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");

    // Seed through the registry role. Do NOT seed as the connection's own
    // user: local dev users are frequently superusers, which bypasses RLS
    // outright and would make this test pass for the wrong reason. The
    // runtime role has a SELECT-only policy and cannot insert at all.
    let seed = client.transaction().await.expect("seed tx");
    seed.batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");
    for hash in [TEST_HASH_A, TEST_HASH_B] {
        seed.execute(
            "INSERT INTO onboarding_invite_grants (
                     invite_subject_hash, policy_label, tenant_mode,
                     tenant_template_id, policy_version, issuance_source
                 ) VALUES ($1, 'test-pool', 'derived', 'tmpl-1', 'v1', 'operator')
                 ON CONFLICT (invite_subject_hash) DO NOTHING",
            &[&hash],
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
        &[&TEST_HASH_A],
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
    assert_eq!(rows[0].get::<_, String>(0), TEST_HASH_A);

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

    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");
    // Seed through the registry role, not the connection's own user.
    let seed = client.transaction().await.expect("seed tx");
    seed.batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");
    for hash in [TEST_HASH_A, TEST_HASH_B] {
        seed.execute(
            "INSERT INTO onboarding_invite_grants (
                     invite_subject_hash, policy_label, tenant_mode,
                     tenant_template_id, policy_version, issuance_source
                 ) VALUES ($1, 'test-pool', 'derived', 'tmpl-1', 'v1', 'operator')
                 ON CONFLICT (invite_subject_hash) DO NOTHING",
            &[&hash],
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
            &[&TEST_HASH_A, &TEST_HASH_B],
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
            &[&TEST_HASH_A],
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
            &[&TEST_HASH_B],
        )
        .await;
    assert!(err.is_err(), "derived mode must not carry fixed_tenant_id");
}

use trace_commons_server::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use trace_commons_server::trace_invite_registry::InviteTenantMode;

const TEST_HASH_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const TEST_HASH_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const TEST_CRED: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

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

fn derived_write(hash: &str) -> InviteGrantWrite {
    InviteGrantWrite {
        invite_subject_hash: hash.to_string(),
        policy_label: "test-pool".to_string(),
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
    }
}

/// Fixture for `insert_then_list_round_trips_every_field`. Every string field
/// gets a distinct, recognizable value (never sharing a value with another
/// field, e.g. two fields both set to "v1") so a transposition bug -- say,
/// `policy_version` and `issuance_source` swapped in the INSERT column list
/// or the row mapper -- would fail the round-trip instead of passing
/// silently. `expires_at` and `credential_binding_hash` are set to non-None
/// values so both are actually exercised by the assertions.
fn round_trip_fixture(hash: &str) -> InviteGrantWrite {
    InviteGrantWrite {
        invite_subject_hash: hash.to_string(),
        policy_label: "test-pool".to_string(),
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
        credential_binding_hash: Some(TEST_CRED.to_string()),
        note_label: Some("note-label-round-trip".to_string()),
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
    cleanup_test_invites(&backend, "test-pool").await;

    let fixture = round_trip_fixture(TEST_HASH_C);
    let expected_expires_at = fixture.expires_at.expect("fixture sets expires_at");
    let outcome = backend.insert_invite_grant(fixture).await.expect("insert");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    let all = backend.list_invite_grants().await.expect("list");
    let found = all
        .iter()
        .find(|e| e.invite_subject_hash == TEST_HASH_C)
        .expect("inserted invite present in listing");
    assert_eq!(found.invite_subject_hash, TEST_HASH_C);
    assert_eq!(found.policy_label, "test-pool");
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
    assert_eq!(found.credential_binding_hash.as_deref(), Some(TEST_CRED));
    assert_eq!(found.note_label.as_deref(), Some("note-label-round-trip"));
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
    cleanup_test_invites(&backend, "test-pool").await;

    let mut first = derived_write(TEST_HASH_C);
    first.credential_binding_hash = Some(TEST_CRED.to_string());
    let _ = backend
        .insert_invite_grant(first)
        .await
        .expect("first insert");

    let mut second = derived_write(TEST_HASH_D);
    second.credential_binding_hash = Some(TEST_CRED.to_string());
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
    cleanup_test_invites(&backend, "test-pool").await;

    let mut first = derived_write(TEST_HASH_C);
    first.credential_binding_hash = Some(TEST_CRED.to_string());
    let _ = backend.insert_invite_grant(first).await.expect("insert");

    let revoked = backend
        .revoke_invite_grant(TEST_HASH_C)
        .await
        .expect("revoke");
    assert!(revoked, "revoking a live invite reports true");

    let mut second = derived_write(TEST_HASH_D);
    second.credential_binding_hash = Some(TEST_CRED.to_string());
    let outcome = backend.insert_invite_grant(second).await.expect("reissue");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    // Revoking an already-revoked invite is a no-op, not an error.
    let again = backend
        .revoke_invite_grant(TEST_HASH_C)
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
    cleanup_test_invites(&backend, "test-pool").await;

    let _ = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");
    let _ = backend
        .revoke_invite_grant(TEST_HASH_C)
        .await
        .expect("revoke");

    let all = backend.list_invite_grants().await.expect("list");
    assert!(
        !all.iter().any(|e| e.invite_subject_hash == TEST_HASH_C),
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
    cleanup_test_invites(&backend, "test-pool").await;

    let mut expired = derived_write(TEST_HASH_C);
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
        !all.iter().any(|e| e.invite_subject_hash == TEST_HASH_C),
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
    cleanup_test_invites(&backend, "test-pool").await;

    let _ = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");

    let outcome = backend
        .redeem_invite_grant(TEST_HASH_C, "user-subject-1")
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
    cleanup_test_invites(&backend, "test-pool").await;

    let mut write = derived_write(TEST_HASH_C);
    write.tenant_mode = InviteTenantMode::Fixed;
    write.tenant_template_id = None;
    write.fixed_tenant_id = Some("tenant-zaki-pilot".to_string());
    let _ = backend.insert_invite_grant(write).await.expect("insert");

    let outcome = backend
        .redeem_invite_grant(TEST_HASH_C, "user-subject-1")
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
    cleanup_test_invites(&backend, "test-pool").await;

    let _ = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");
    let _ = backend
        .revoke_invite_grant(TEST_HASH_C)
        .await
        .expect("revoke");

    // The cache is deliberately bypassed here: this asserts the database, not
    // the cache, is what refuses a revoked invite.
    let result = backend
        .redeem_invite_grant(TEST_HASH_C, "user-subject-1")
        .await
        .expect("a revoked invite is not a database error");
    assert!(
        result.is_none(),
        "the in-transaction re-check must refuse a revoked invite"
    );
}

#[tokio::test]
async fn an_expired_invite_cannot_be_redeemed() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    cleanup_test_invites(&backend, "test-pool").await;

    let mut write = derived_write(TEST_HASH_C);
    write.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    let _ = backend.insert_invite_grant(write).await.expect("insert");

    let result = backend
        .redeem_invite_grant(TEST_HASH_C, "user-subject-1")
        .await
        .expect("an expired invite is not a database error");
    assert!(result.is_none(), "an expired invite must not redeem");
}
