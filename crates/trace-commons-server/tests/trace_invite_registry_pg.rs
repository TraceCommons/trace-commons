//! Postgres-backed tests for the V42 invite-grant table.
//!
//! Skipped unless TRACE_COMMONS_PG_TEST_DATABASE_URL (or DATABASE_URL) is set.
//! CI does not run these; run them locally against a real PostgreSQL.

use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{postgres::PgBackend, Database};

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
        // Task 3 adds `invite_registry_url` to DatabaseConfig. This literal is
        // exhaustive, so Task 3 Step 1 must add the field here too or this
        // file stops compiling. That is expected and is called out there.
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
