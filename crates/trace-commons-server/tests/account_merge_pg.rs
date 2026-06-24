//! DB-backed (real PostgreSQL) tests for the Slice 3b device-principal merge
//! core: `stage_merge_proposal` (consume device B's login-link as proof and
//! stage a proposal) and `execute_merge` (atomically fold absorbed account B
//! into surviving account A).
//!
//! These mirror the harness in `trace_corpus_pg_store.rs`: a real PgBackend
//! against `TRACE_COMMONS_PG_TEST_DATABASE_URL` / `DATABASE_URL`, with
//! `run_migrations()` applied. Each test uses a distinct tenant id so the suite
//! is order-independent, and MUST run with `--test-threads=1`.
//!
//! SECURITY invariants under test: the whole `execute_merge` is one atomic
//! tenant transaction (a partial merge is never observable); the login-link
//! consume + proposal are the proof-of-control; `execute_merge` re-validates the
//! surviving account's ownership and that B is still open before mutating;
//! audit rows are hash-only / label-only (no principal_refs, public_keys, or
//! account uuids in metadata).

use secrecy::SecretString;
use tokio_postgres::types::ToSql;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

/// Run a tenant-scoped statement on the raw test pool (RLS GUC set for
/// `tenant_id`), committing the tx. Mirrors the raw-pool pattern in
/// `trace_corpus_pg_store.rs`. Test-only.
async fn raw_execute(
    backend: &PgBackend,
    tenant_id: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let tx = client.transaction().await.expect("raw tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant context");
    tx.execute(sql, params).await.expect("raw execute");
    tx.commit().await.expect("raw commit");
}

/// Run a tenant-scoped `SELECT count(*)`-style scalar query on the raw test pool.
async fn raw_scalar_i64(
    backend: &PgBackend,
    tenant_id: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> i64 {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let tx = client.transaction().await.expect("raw tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant context");
    let row = tx.query_one(sql, params).await.expect("raw scalar");
    let value: i64 = row.get(0);
    tx.commit().await.expect("raw commit");
    value
}

/// Run a tenant-scoped query returning at most one UUID column.
async fn raw_opt_uuid(
    backend: &PgBackend,
    tenant_id: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Option<Uuid> {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let tx = client.transaction().await.expect("raw tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant context");
    let row = tx.query_opt(sql, params).await.expect("raw opt");
    let value = row.map(|r| r.get::<_, Uuid>(0));
    tx.commit().await.expect("raw commit");
    value
}

fn postgres_test_config() -> Option<DatabaseConfig> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    Some(DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: DatabaseConfig::login_resolver_url_from_env(),
    })
}

async fn postgres_backend() -> Option<PgBackend> {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: TRACE_COMMONS_PG_TEST_DATABASE_URL or DATABASE_URL not configured");
        return None;
    };

    match PgBackend::new(&config).await {
        Ok(backend) => {
            backend.run_migrations().await.expect("run migrations");
            Some(backend)
        }
        Err(e) => {
            eprintln!("skipping: database unavailable ({e})");
            None
        }
    }
}

fn unique_tenant(label: &str) -> String {
    format!("merge-{label}-{}", Uuid::new_v4())
}

/// A unique, constraint-valid login-link code hash: `sha256:` + 64 hex chars.
/// (`trace_login_links` enforces `^sha256:[0-9a-f]{64}$`.) The bytes are random,
/// never derived from a real secret.
fn unique_code_hash() -> String {
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("sha256:{a}{b}")
}

/// Insert a login-link row for `account_id` directly (mirrors the columns the
/// app writes in `create_login_link`). `expires_at` is set 1h in the future and
/// `consumed_at` is NULL unless `consumed` is true. The raw `code_hash` is a
/// sha256-prefixed label, never a secret.
async fn seed_login_link(
    backend: &PgBackend,
    tenant_id: &str,
    account_id: Uuid,
    code_hash: &str,
    consumed: bool,
    expired: bool,
) {
    let expires_sql = if expired {
        "now() - interval '1 minute'"
    } else {
        "now() + interval '1 hour'"
    };
    let consumed_sql = if consumed { "now()" } else { "NULL" };
    raw_execute(
        backend,
        tenant_id,
        &format!(
            "INSERT INTO trace_login_links (
                tenant_id, link_id, account_id, code_hash,
                created_principal_ref, created_at, expires_at, consumed_at
             ) VALUES (
                trace_current_tenant_id(), $1, $2, $3, $4, now(), {expires_sql}, {consumed_sql}
             )"
        ),
        &[
            &Uuid::new_v4(),
            &account_id,
            &code_hash.to_string(),
            &format!("principal:link-{}", Uuid::new_v4()),
        ],
    )
    .await;
}

/// Insert a browser session for `account_id` (hash-only token). Returns the
/// token hash so the test can assert revocation.
async fn seed_session(backend: &PgBackend, tenant_id: &str, account_id: Uuid) -> String {
    let token_hash = unique_code_hash();
    raw_execute(
        backend,
        tenant_id,
        "INSERT INTO trace_sessions (
            tenant_id, session_id, account_id, token_hash,
            client_kind, created_at, last_seen_at, expires_at
         ) VALUES (
            trace_current_tenant_id(), $1, $2, $3, 'web', now(), now(),
            now() + interval '7 days'
         )",
        &[&Uuid::new_v4(), &account_id, &token_hash],
    )
    .await;
    token_hash
}

/// Count active (unlinked_at IS NULL) principals for `account_id`.
async fn active_principal_count(backend: &PgBackend, tenant_id: &str, account_id: Uuid) -> i64 {
    raw_scalar_i64(
        backend,
        tenant_id,
        "SELECT count(*) FROM trace_account_principals
          WHERE tenant_id = trace_current_tenant_id()
            AND account_id = $1
            AND unlinked_at IS NULL",
        &[&account_id],
    )
    .await
}

async fn account_closed(backend: &PgBackend, tenant_id: &str, account_id: Uuid) -> bool {
    raw_scalar_i64(
        backend,
        tenant_id,
        "SELECT count(*) FROM trace_accounts
          WHERE tenant_id = trace_current_tenant_id()
            AND account_id = $1
            AND closed_at IS NOT NULL",
        &[&account_id],
    )
    .await
        > 0
}

async fn session_live(backend: &PgBackend, tenant_id: &str, token_hash: &str) -> bool {
    raw_scalar_i64(
        backend,
        tenant_id,
        "SELECT count(*) FROM trace_sessions
          WHERE tenant_id = trace_current_tenant_id()
            AND token_hash = $1
            AND revoked_at IS NULL",
        &[&token_hash.to_string()],
    )
    .await
        > 0
}

async fn near_identity_account(
    backend: &PgBackend,
    tenant_id: &str,
    public_key: &str,
) -> Option<Uuid> {
    raw_opt_uuid(
        backend,
        tenant_id,
        "SELECT account_id FROM trace_near_identities
          WHERE tenant_id = trace_current_tenant_id()
            AND public_key = $1
            AND revoked_at IS NULL",
        &[&public_key.to_string()],
    )
    .await
}

async fn near_identity_payout_designated(
    backend: &PgBackend,
    tenant_id: &str,
    public_key: &str,
) -> bool {
    raw_scalar_i64(
        backend,
        tenant_id,
        "SELECT count(*) FROM trace_near_identities
          WHERE tenant_id = trace_current_tenant_id()
            AND public_key = $1
            AND payout_designated_at IS NOT NULL",
        &[&public_key.to_string()],
    )
    .await
        > 0
}

async fn credential_account(
    backend: &PgBackend,
    tenant_id: &str,
    credential_id: &str,
) -> Option<Uuid> {
    raw_opt_uuid(
        backend,
        tenant_id,
        "SELECT account_id FROM trace_webauthn_credentials
          WHERE tenant_id = trace_current_tenant_id()
            AND credential_id = $1
            AND revoked_at IS NULL",
        &[&credential_id.to_string()],
    )
    .await
}

async fn merge_audit_count(backend: &PgBackend, tenant_id: &str) -> i64 {
    raw_scalar_i64(
        backend,
        tenant_id,
        "SELECT count(*) FROM trace_account_audit
          WHERE tenant_id = trace_current_tenant_id()
            AND action = 'account_merged'",
        &[],
    )
    .await
}

#[tokio::test]
async fn merge_round_trip_moves_everything_and_closes_absorbed() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("round-trip");

    // Account A (surviving) with principal P1.
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:merge-a-p1")
        .await
        .expect("mint A");
    // Account B (absorbed) with principal P2.
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:merge-b-p2")
        .await
        .expect("mint B");
    assert_ne!(account_a, account_b);

    // B owns a webauthn credential, a NEAR identity, and a live session.
    let cred_id = format!("cred-{}", Uuid::new_v4());
    backend
        .insert_webauthn_credential(
            &tenant,
            account_b,
            &cred_id,
            &serde_json::json!({"fake": "passkey"}),
            Some("b-key"),
        )
        .await
        .expect("seed webauthn for B");
    let near_pk = format!("ed25519:merge-b-{}", Uuid::new_v4());
    backend
        .insert_near_identity(&tenant, account_b, &near_pk, "b.near", Some("b-wallet"))
        .await
        .expect("seed near for B");
    let session_hash = seed_session(&backend, &tenant, account_b).await;

    // A login-link for B is the proof-of-control device B presents.
    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;

    // Stage: consuming B's link yields a proposal naming B as absorbed.
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");
    assert_eq!(staged.absorbed_account_id, account_b);
    assert_eq!(staged.absorbed_principal_count, 1);
    // The link is now consumed (re-staging the same code denies).
    assert!(
        backend
            .stage_merge_proposal(&tenant, account_a, &code_hash)
            .await
            .expect("stage retry ok")
            .is_none(),
        "consumed link must not re-stage"
    );

    // Execute the staged proposal.
    let executed = backend
        .execute_merge(&tenant, account_a, staged.proposal_id)
        .await
        .expect("execute ok")
        .expect("executed some");
    assert_eq!(executed.principals_moved, 1);
    assert_eq!(executed.authenticators_moved, 2);

    // P2 now links to A; A holds both principals.
    assert_eq!(active_principal_count(&backend, &tenant, account_a).await, 2);
    assert_eq!(active_principal_count(&backend, &tenant, account_b).await, 0);
    // B's webauthn + NEAR identity now belong to A.
    assert_eq!(
        credential_account(&backend, &tenant, &cred_id).await,
        Some(account_a)
    );
    assert_eq!(
        near_identity_account(&backend, &tenant, &near_pk).await,
        Some(account_a)
    );
    // B's session is revoked.
    assert!(!session_live(&backend, &tenant, &session_hash).await);
    // B is closed.
    assert!(account_closed(&backend, &tenant, account_b).await);
    // An account_merged audit row exists.
    assert_eq!(merge_audit_count(&backend, &tenant).await, 1);
}

#[tokio::test]
async fn stage_rejects_merge_into_self() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("self");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:self-a")
        .await
        .expect("mint A");
    // A login-link for A itself.
    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_a, &code_hash, false, false).await;

    assert!(
        backend
            .stage_merge_proposal(&tenant, account_a, &code_hash)
            .await
            .expect("stage ok")
            .is_none(),
        "cannot merge an account into itself"
    );
}

#[tokio::test]
async fn stage_rejects_consumed_and_expired_links() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("badlink");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:badlink-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:badlink-b")
        .await
        .expect("mint B");

    let consumed_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &consumed_hash, true, false).await;
    assert!(
        backend
            .stage_merge_proposal(&tenant, account_a, &consumed_hash)
            .await
            .expect("stage ok")
            .is_none(),
        "already-consumed link must deny"
    );

    let expired_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &expired_hash, false, true).await;
    assert!(
        backend
            .stage_merge_proposal(&tenant, account_a, &expired_hash)
            .await
            .expect("stage ok")
            .is_none(),
        "expired link must deny"
    );

    let unknown_hash = unique_code_hash();
    assert!(
        backend
            .stage_merge_proposal(&tenant, account_a, &unknown_hash)
            .await
            .expect("stage ok")
            .is_none(),
        "unknown link must deny"
    );
}

#[tokio::test]
async fn stage_rejects_when_absorbed_already_closed() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("closed-b");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:closedb-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:closedb-b")
        .await
        .expect("mint B");

    // Close B out of band.
    raw_execute(
        &backend,
        &tenant,
        "UPDATE trace_accounts SET closed_at = now()
          WHERE tenant_id = trace_current_tenant_id() AND account_id = $1",
        &[&account_b],
    )
    .await;

    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;

    assert!(
        backend
            .stage_merge_proposal(&tenant, account_a, &code_hash)
            .await
            .expect("stage ok")
            .is_none(),
        "cannot merge an already-closed account"
    );
}

#[tokio::test]
async fn execute_rejects_expired_proposal_without_mutating() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("exp-prop");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:expprop-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:expprop-b")
        .await
        .expect("mint B");

    // Insert an ALREADY-expired proposal directly.
    let proposal_id = Uuid::new_v4();
    raw_execute(
        &backend,
        &tenant,
        "INSERT INTO trace_account_merge_proposals (
            tenant_id, proposal_id, surviving_account_id, absorbed_account_id,
            absorbed_principal_count, created_at, expires_at
         ) VALUES (
            trace_current_tenant_id(), $1, $2, $3, 1,
            now() - interval '1 hour', now() - interval '1 minute'
         )",
        &[&proposal_id, &account_a, &account_b],
    )
    .await;

    assert!(
        backend
            .execute_merge(&tenant, account_a, proposal_id)
            .await
            .expect("execute ok")
            .is_none(),
        "expired proposal must deny"
    );
    // No mutation: B still open, B's principal still on B.
    assert!(!account_closed(&backend, &tenant, account_b).await);
    assert_eq!(active_principal_count(&backend, &tenant, account_b).await, 1);
    assert_eq!(merge_audit_count(&backend, &tenant).await, 0);
}

#[tokio::test]
async fn execute_rejects_proposal_owned_by_different_surviving_account() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("wrong-owner");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:wrong-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:wrong-b")
        .await
        .expect("mint B");
    let account_c = backend
        .create_or_reuse_account(&tenant, "principal:wrong-c")
        .await
        .expect("mint C");

    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;
    // Proposal is owned by A (surviving = A).
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");

    // C tries to execute A's proposal -> deny, no mutation.
    assert!(
        backend
            .execute_merge(&tenant, account_c, staged.proposal_id)
            .await
            .expect("execute ok")
            .is_none(),
        "a different surviving account must not execute the proposal"
    );
    assert!(!account_closed(&backend, &tenant, account_b).await);
    assert_eq!(active_principal_count(&backend, &tenant, account_b).await, 1);

    // The legitimate owner can still execute it.
    let executed = backend
        .execute_merge(&tenant, account_a, staged.proposal_id)
        .await
        .expect("execute ok")
        .expect("executed some");
    assert_eq!(executed.principals_moved, 1);
}

#[tokio::test]
async fn execute_is_single_use() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("single-use");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:single-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:single-b")
        .await
        .expect("mint B");

    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");

    assert!(
        backend
            .execute_merge(&tenant, account_a, staged.proposal_id)
            .await
            .expect("execute ok")
            .is_some(),
        "first execute succeeds"
    );
    assert!(
        backend
            .execute_merge(&tenant, account_a, staged.proposal_id)
            .await
            .expect("execute ok")
            .is_none(),
        "second execute of a consumed proposal denies"
    );
    // Exactly one audit row from the single successful merge.
    assert_eq!(merge_audit_count(&backend, &tenant).await, 1);
}

#[tokio::test]
async fn merge_clears_absorbed_payout_designation() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("payout-clear");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:payout-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:payout-b")
        .await
        .expect("mint B");

    // A keeps its own designated payout identity.
    let a_pk = format!("ed25519:payout-a-{}", Uuid::new_v4());
    backend
        .insert_near_identity(&tenant, account_a, &a_pk, "a.near", None)
        .await
        .expect("seed a near");
    assert!(
        backend
            .designate_payout_near_identity(&tenant, account_a, &a_pk)
            .await
            .expect("designate a")
    );

    // B has a payout-designated identity too.
    let b_pk = format!("ed25519:payout-b-{}", Uuid::new_v4());
    backend
        .insert_near_identity(&tenant, account_b, &b_pk, "b.near", None)
        .await
        .expect("seed b near");
    assert!(
        backend
            .designate_payout_near_identity(&tenant, account_b, &b_pk)
            .await
            .expect("designate b")
    );

    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");
    backend
        .execute_merge(&tenant, account_a, staged.proposal_id)
        .await
        .expect("execute ok")
        .expect("executed some");

    // B's NEAR identity moved to A with designation CLEARED.
    assert_eq!(
        near_identity_account(&backend, &tenant, &b_pk).await,
        Some(account_a)
    );
    assert!(
        !near_identity_payout_designated(&backend, &tenant, &b_pk).await,
        "absorbed identity's payout designation must be cleared on merge"
    );
    // A keeps its own designation (no partial-unique-index violation).
    assert!(near_identity_payout_designated(&backend, &tenant, &a_pk).await);
}
