// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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
use trace_commons_protocol::public_run::{PublicRunEvidenceDraft, PublicRunReusePermission};
use trace_commons_protocol::trace_contribution::TaskSuccess;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, PublicRunWrite, postgres::PgBackend};
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
        gate_driver_url: DatabaseConfig::gate_driver_url_from_env(),
        pii_backstop_driver_url: DatabaseConfig::pii_backstop_driver_url_from_env(),
        invite_registry_url: None,
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

async fn seed_accepted_submission(
    backend: &PgBackend,
    tenant_id: &str,
    submission_id: Uuid,
    principal_ref: &str,
) {
    let principal_ref = principal_ref.to_string();
    raw_execute(
        backend,
        tenant_id,
        "INSERT INTO trace_submissions (
            tenant_id, submission_id, trace_id, auth_principal_ref,
            schema_version, consent_policy_version, retention_policy_id,
            status, privacy_risk, redaction_pipeline_version, redaction_hash
         ) VALUES (
            trace_current_tenant_id(), $1, $2, $3,
            'trace.contribution.v1', 'consent.v1', 'retention.v1',
            'accepted', 'low', 'redaction.v1', 'sha256:synthetic'
         )",
        &[&submission_id, &Uuid::new_v4(), &principal_ref],
    )
    .await;
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

    let public_submission_id = Uuid::new_v4();
    seed_accepted_submission(
        &backend,
        &tenant,
        public_submission_id,
        "principal:merge-b-p2",
    )
    .await;
    let public_run = PublicRunWrite {
        tenant_id: tenant.clone(),
        publication_id: Uuid::new_v4(),
        account_id: account_b,
        submission_id: public_submission_id,
        slug: format!("run-{}", &Uuid::new_v4().simple().to_string()[..16]),
        title: "Synthetic merge workflow".to_string(),
        outcome_summary: "The synthetic merge workflow completed.".to_string(),
        correction_excerpt: None,
        workflow: "Apply the bounded synthetic merge steps.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: Uuid::new_v4(),
            excerpt: "The synthetic merge result was observed.".to_string(),
        }],
        task_success: TaskSuccess::Success,
        contributed_version: "trace.contribution.v1".to_string(),
        approval_sha256: format!("sha256:{}", "a".repeat(64)),
        source_publication_id: None,
        expected_publication_version: 0,
    };
    backend
        .upsert_public_run(public_run.clone())
        .await
        .expect("publish absorbed account workflow");

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
    assert_eq!(
        active_principal_count(&backend, &tenant, account_a).await,
        2
    );
    assert_eq!(
        active_principal_count(&backend, &tenant, account_b).await,
        0
    );
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

    let moved_public_run = backend
        .get_owned_public_run_state(&tenant, account_a, public_submission_id)
        .await
        .expect("read moved public workflow")
        .row
        .expect("moved public workflow exists");
    assert_eq!(moved_public_run.account_id, account_a);
    assert!(
        backend
            .get_owned_public_run_state(&tenant, account_b, public_submission_id)
            .await
            .expect("read absorbed account workflow")
            .row
            .is_none()
    );

    let mut update = public_run;
    update.account_id = account_a;
    update.publication_id = Uuid::new_v4();
    update.title = "Synthetic merge workflow revised".to_string();
    update.approval_sha256 = format!("sha256:{}", "b".repeat(64));
    update.expected_publication_version = 1;
    let updated = backend
        .upsert_public_run(update)
        .await
        .expect("surviving account updates workflow");
    assert_eq!(updated.row.version, 2);
    assert_eq!(updated.row.publication_id, moved_public_run.publication_id);
    let first_unpublish = backend
        .unpublish_public_run(&tenant, account_a, public_submission_id)
        .await
        .expect("surviving account unpublishes workflow");
    assert!(first_unpublish.unpublished);
    assert_eq!(first_unpublish.expected_publication_version, 3);
    let repeated_unpublish = backend
        .unpublish_public_run(&tenant, account_a, public_submission_id)
        .await
        .expect("repeat unpublish is idempotent");
    assert!(!repeated_unpublish.unpublished);
    assert_eq!(repeated_unpublish.expected_publication_version, 3);
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
    assert_eq!(
        active_principal_count(&backend, &tenant, account_b).await,
        1
    );
    assert_eq!(merge_audit_count(&backend, &tenant).await, 0);
}

#[tokio::test]
async fn execute_rejects_when_surviving_closed() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("closed-a");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:closeda-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:closeda-b")
        .await
        .expect("mint B");

    // Stage a valid, fresh proposal (A surviving, B absorbed).
    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");

    // The surviving account A is soft-closed AFTER staging but BEFORE execute.
    // Folding B onto a tombstoned A would be irreversible identity corruption, so
    // execute must deny and mutate nothing.
    raw_execute(
        &backend,
        &tenant,
        "UPDATE trace_accounts SET closed_at = now()
          WHERE tenant_id = trace_current_tenant_id() AND account_id = $1",
        &[&account_a],
    )
    .await;

    assert!(
        backend
            .execute_merge(&tenant, account_a, staged.proposal_id)
            .await
            .expect("execute ok")
            .is_none(),
        "a closed surviving account must deny execute"
    );
    // No mutation: B still open with its own principal; nothing folded onto A.
    assert!(!account_closed(&backend, &tenant, account_b).await);
    assert_eq!(
        active_principal_count(&backend, &tenant, account_b).await,
        1
    );
    assert_eq!(
        active_principal_count(&backend, &tenant, account_a).await,
        1
    );
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
    assert_eq!(
        active_principal_count(&backend, &tenant, account_b).await,
        1
    );

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

// ---------------------------------------------------------------------------
// Invite trust carry-over (V80).
//
// Rule: the survivor ends with the highest trust its combined, unrevoked
// grants support. The absorbed account's invite grant rows are copied onto the
// survivor with their original grant version, time, and revocation. A revoked
// grant never confers trust, and when both accounts hold a grant from the same
// invite, a revocation on either side wins. An authority change moves the
// survivor's trust version past both accounts' versions; an unchanged
// authority keeps the survivor's version.
// ---------------------------------------------------------------------------

/// Trust fixture: `(authority, trust_version)` plus grants `(invite, revoked)`.
struct TrustSeed<'a> {
    trust: Option<(&'a str, i64)>,
    grants: &'a [(&'a str, bool)],
}

fn invite_hash(label: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(label.as_bytes())))
}

async fn seed_trust(backend: &PgBackend, tenant_id: &str, account_id: Uuid, seed: &TrustSeed<'_>) {
    if let Some((authority, version)) = seed.trust {
        raw_execute(
            backend,
            tenant_id,
            "INSERT INTO trace_account_trust (tenant_id, account_id, authority, trust_version)
             VALUES (trace_current_tenant_id(), $1, $2, $3)",
            &[&account_id, &authority.to_string(), &version],
        )
        .await;
    }
    for (invite, revoked) in seed.grants {
        let revoked_sql = if *revoked {
            "now() - interval '1 minute'"
        } else {
            "NULL"
        };
        raw_execute(
            backend,
            tenant_id,
            &format!(
                "INSERT INTO trace_account_invite_grants
                    (tenant_id, account_id, invite_subject_hash, trust_version,
                     granted_at, revoked_at)
                 VALUES (trace_current_tenant_id(), $1, $2, 1,
                     now() - interval '1 day', {revoked_sql})"
            ),
            &[&account_id, &invite_hash(invite)],
        )
        .await;
    }
}

async fn trust_state(
    backend: &PgBackend,
    tenant_id: &str,
    account_id: Uuid,
) -> Option<(String, i64)> {
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    client
        .query_opt(
            "SELECT authority, trust_version FROM trace_account_trust
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant_id, &account_id],
        )
        .await
        .expect("read trust")
        .map(|row| (row.get(0), row.get(1)))
}

/// `(invite_subject_hash, revoked)` for every grant row on the account, sorted.
async fn grant_state(
    backend: &PgBackend,
    tenant_id: &str,
    account_id: Uuid,
) -> Vec<(String, bool)> {
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    client
        .query(
            "SELECT invite_subject_hash, revoked_at IS NOT NULL
               FROM trace_account_invite_grants
              WHERE tenant_id = $1 AND account_id = $2
              ORDER BY invite_subject_hash",
            &[&tenant_id, &account_id],
        )
        .await
        .expect("read grants")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

/// A login holding only the privileges a merge had before trust existed and
/// nothing on the trust tables, so any carry-over must come from V80's
/// definer function rather than the caller's rights.
async fn restricted_merge_backend(backend: &PgBackend) -> PgBackend {
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    client
        // One transaction under an advisory lock: the suite runs in parallel,
        // and concurrent GRANTs on one table fail with "tuple concurrently
        // updated" (and concurrent CREATE ROLE with a duplicate key).
        .batch_execute(
            "BEGIN;
             SELECT pg_advisory_xact_lock(hashtextextended('account_merge_pg restricted login', 0));
             DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_trust_merge_runtime') THEN CREATE ROLE trace_trust_merge_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT SELECT,INSERT,UPDATE ON trace_tenants TO trace_trust_merge_runtime;
             GRANT SELECT,UPDATE ON trace_accounts,trace_account_merge_proposals,trace_account_principals,
                 trace_webauthn_credentials,trace_near_identities,trace_public_runs,trace_sessions TO trace_trust_merge_runtime;
             GRANT INSERT ON trace_account_audit TO trace_trust_merge_runtime;
             GRANT USAGE ON SEQUENCE trace_account_audit_audit_sequence_seq TO trace_trust_merge_runtime;
             COMMIT;",
        )
        .await
        .expect("provision restricted merge login");
    let trust_rights: bool = client
        .query_one(
            "SELECT has_table_privilege('trace_trust_merge_runtime', 'trace_account_trust', 'SELECT')
                 OR has_table_privilege('trace_trust_merge_runtime', 'trace_account_invite_grants', 'SELECT')",
            &[],
        )
        .await
        .expect("inspect merge login")
        .get(0);
    assert!(!trust_rights, "the merge login holds no trust-table rights");
    let base = postgres_test_config().expect("config");
    let mut url = reqwest::Url::parse(secrecy::ExposeSecret::expose_secret(&base.url))
        .expect("test database URL");
    url.set_username("trace_trust_merge_runtime")
        .expect("set user");
    PgBackend::new(&DatabaseConfig {
        url: SecretString::from(url.to_string()),
        ..base
    })
    .await
    .expect("restricted merge backend")
}

/// Seed A and B, merge B into A as the restricted login, and return the
/// tenant, both ids, and the merge audit metadata.
async fn merge_with_trust(
    backend: &PgBackend,
    label: &str,
    survivor: TrustSeed<'_>,
    absorbed: TrustSeed<'_>,
) -> (String, Uuid, Uuid, serde_json::Value) {
    let tenant = unique_tenant(label);
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:trust-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:trust-b")
        .await
        .expect("mint B");
    seed_trust(backend, &tenant, account_a, &survivor).await;
    seed_trust(backend, &tenant, account_b, &absorbed).await;
    let code_hash = unique_code_hash();
    seed_login_link(backend, &tenant, account_b, &code_hash, false, false).await;
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");
    restricted_merge_backend(backend)
        .await
        .execute_merge(&tenant, account_a, staged.proposal_id)
        .await
        .expect("execute ok")
        .expect("executed some");
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let metadata: serde_json::Value = client
        .query_one(
            "SELECT safe_metadata FROM trace_account_audit
              WHERE tenant_id = $1 AND action = 'account_merged'",
            &[&tenant],
        )
        .await
        .expect("merge audit")
        .get(0);
    // The definer function refuses outside the transaction that consumed the
    // proposal, so it cannot be called to move trust on its own.
    let tx = client.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant context");
    let replay = tx
        .query_one(
            "SELECT public.trace_account_trust_merge($1, $2, $3, $4)",
            &[&tenant, &account_a, &account_b, &staged.proposal_id],
        )
        .await
        .expect_err("a proposal consumed by an earlier transaction authorizes nothing");
    assert_eq!(
        replay.as_db_error().map(|e| e.message()),
        Some("account_trust_merge_unauthorized"),
        "refused by the function's own proof, not by its absence"
    );
    tx.rollback().await.expect("rollback");
    (tenant, account_a, account_b, metadata)
}

#[tokio::test]
async fn merge_bounded_into_invited_keeps_survivor_invited() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let (tenant, a, _b, metadata) = merge_with_trust(
        &backend,
        "trust-bounded-into-invited",
        TrustSeed {
            trust: Some(("invited", 3)),
            grants: &[("x", false)],
        },
        TrustSeed {
            trust: Some(("bounded", 9)),
            grants: &[],
        },
    )
    .await;
    assert_eq!(
        trust_state(&backend, &tenant, a).await,
        Some(("invited".into(), 3)),
        "an unchanged authority keeps the survivor's version"
    );
    assert_eq!(
        grant_state(&backend, &tenant, a).await,
        vec![(invite_hash("x"), false)]
    );
    assert_eq!(metadata["invite_grants_carried"], 0);

    // The definer is a NOLOGIN, NOBYPASSRLS, non-superuser guard.
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let guard = client
        .query_one(
            "SELECT r.rolcanlogin, r.rolbypassrls, r.rolsuper,
                    pg_get_userbyid(p.proowner) = r.rolname, p.prosecdef
               FROM pg_proc p, pg_roles r
              WHERE p.proname = 'trace_account_trust_merge'
                AND r.rolname = 'trace_account_trust_merge_guard'",
            &[],
        )
        .await
        .expect("inspect guard");
    assert!(!guard.get::<_, bool>(0), "guard cannot log in");
    assert!(!guard.get::<_, bool>(1), "guard does not bypass RLS");
    assert!(!guard.get::<_, bool>(2), "guard is not a superuser");
    assert!(guard.get::<_, bool>(3), "guard owns the function");
    assert!(guard.get::<_, bool>(4), "function is SECURITY DEFINER");
}

#[tokio::test]
async fn merge_invited_into_bounded_carries_grant_and_elevates() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let (tenant, a, b, metadata) = merge_with_trust(
        &backend,
        "trust-invited-into-bounded",
        TrustSeed {
            trust: Some(("bounded", 2)),
            grants: &[],
        },
        TrustSeed {
            trust: Some(("invited", 5)),
            grants: &[("y", false)],
        },
    )
    .await;
    assert_eq!(
        trust_state(&backend, &tenant, a).await,
        Some(("invited".into(), 6)),
        "elevation moves past both accounts' versions"
    );
    assert_eq!(
        grant_state(&backend, &tenant, a).await,
        vec![(invite_hash("y"), false)]
    );
    // The carried row keeps its provenance.
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let preserved: bool = client
        .query_one(
            "SELECT a.granted_at = b.granted_at AND a.trust_version = b.trust_version
               FROM trace_account_invite_grants a
               JOIN trace_account_invite_grants b
                 ON b.tenant_id = a.tenant_id AND b.invite_subject_hash = a.invite_subject_hash
              WHERE a.tenant_id = $1 AND a.account_id = $2 AND b.account_id = $3",
            &[&tenant, &a, &b],
        )
        .await
        .expect("compare grant rows")
        .get(0);
    assert!(
        preserved,
        "granted_at and grant trust_version carry over unchanged"
    );
    assert_eq!(metadata["invite_grants_carried"], 1);
}

#[tokio::test]
async fn merge_invited_into_untrusted_creates_invited_trust() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let (tenant, a, _b, _metadata) = merge_with_trust(
        &backend,
        "trust-invited-into-none",
        TrustSeed {
            trust: None,
            grants: &[],
        },
        TrustSeed {
            trust: Some(("invited", 4)),
            grants: &[("w", false)],
        },
    )
    .await;
    assert_eq!(
        trust_state(&backend, &tenant, a).await,
        Some(("invited".into(), 5))
    );
}

#[tokio::test]
async fn merge_revoked_grant_carries_as_revoked_and_confers_nothing() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let (tenant, a, _b, metadata) = merge_with_trust(
        &backend,
        "trust-revoked",
        TrustSeed {
            trust: Some(("bounded", 1)),
            grants: &[],
        },
        TrustSeed {
            trust: Some(("invited", 4)),
            grants: &[("z", true)],
        },
    )
    .await;
    assert_eq!(
        trust_state(&backend, &tenant, a).await,
        Some(("bounded".into(), 1)),
        "a revoked grant never elevates the survivor"
    );
    assert_eq!(
        grant_state(&backend, &tenant, a).await,
        vec![(invite_hash("z"), true)]
    );
    assert_eq!(metadata["invite_grants_carried"], 1);
}

#[tokio::test]
async fn merge_revocation_of_same_invite_wins_over_survivor_grant() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let (tenant, a, _b, _metadata) = merge_with_trust(
        &backend,
        "trust-revocation-wins",
        TrustSeed {
            trust: Some(("invited", 2)),
            grants: &[("v", false)],
        },
        TrustSeed {
            trust: Some(("invited", 3)),
            grants: &[("v", true)],
        },
    )
    .await;
    assert_eq!(
        grant_state(&backend, &tenant, a).await,
        vec![(invite_hash("v"), true)]
    );
    assert_eq!(
        trust_state(&backend, &tenant, a).await,
        Some(("bounded".into(), 4)),
        "with no unrevoked grant left, the survivor is bounded at a new version"
    );
}

// ---------------------------------------------------------------------------
// NEAR anchor and provisioned-device carry-over (V82).
//
// Rule: every `trace_near_account_anchors` row of the absorbed account, and
// every `trace_near_provisioned_devices` row hanging off those anchors, is
// re-keyed onto the survivor. Nothing else about the rows changes: a device
// whose key is revoked stays revoked (revocation lives on `device_keys`, which
// the merge does not touch), and a survivor that already holds an anchor ends
// up holding both. The same NEAR account cannot be anchored on both sides, and
// the same device cannot be provisioned to both, because `anchor_hash` and
// `(tenant_id, device_key_id)` are UNIQUE; the re-key is therefore
// collision-free by construction.
// ---------------------------------------------------------------------------

/// A NEAR-shaped tenant id, so the readiness check treats it as one.
fn unique_near_tenant() -> String {
    format!(
        "near-{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

async fn seed_near_anchor(backend: &PgBackend, tenant_id: &str, account_id: Uuid) -> String {
    let anchor_hash = unique_code_hash();
    raw_execute(
        backend,
        tenant_id,
        "INSERT INTO trace_near_account_anchors
            (tenant_id, account_id, anchor_hash, sealed_account_name,
             index_pepper_ref, account_name_key_ref)
         VALUES (trace_current_tenant_id(), $1, $2, $3, 'fixture-pepper', 'fixture-key')",
        &[
            &account_id,
            &anchor_hash,
            &serde_json::json!({"test_fixture": true}),
        ],
    )
    .await;
    anchor_hash
}

/// A NEAR-provisioned device for `principal_ref` on `account_id`, provisioned
/// under `anchor_hash`. The principal is linked if it is not already.
async fn seed_near_device(
    backend: &PgBackend,
    tenant_id: &str,
    account_id: Uuid,
    principal_ref: &str,
    anchor_hash: &str,
    revoked: bool,
) -> String {
    let device_key_id = unique_code_hash();
    let revoked_sql = if revoked { "now()" } else { "NULL" };
    raw_execute(
        backend,
        tenant_id,
        &format!(
            "INSERT INTO device_keys
                (device_key_id, tenant_id, public_key, invite_subject_hash,
                 onboarding_origin, revoked_at)
             VALUES ($1, trace_current_tenant_id(), 'fixture-public-key', NULL,
                 'near', {revoked_sql})"
        ),
        &[&device_key_id],
    )
    .await;
    raw_execute(
        backend,
        tenant_id,
        "INSERT INTO trace_account_principals (tenant_id, account_id, principal_ref)
         VALUES (trace_current_tenant_id(), $1, $2)
         ON CONFLICT (tenant_id, principal_ref) DO NOTHING",
        &[&account_id, &principal_ref],
    )
    .await;
    raw_execute(
        backend,
        tenant_id,
        "INSERT INTO trace_near_provisioned_devices
            (tenant_id, principal_ref, account_id, device_key_id, anchor_hash)
         VALUES (trace_current_tenant_id(), $1, $2, $3, $4)",
        &[&principal_ref, &account_id, &device_key_id, &anchor_hash],
    )
    .await;
    device_key_id
}

/// `trace_account_admission_live_device`, the check admission makes per
/// request, as the tenant.
async fn live_near_device(
    backend: &PgBackend,
    tenant_id: &str,
    account_id: Uuid,
    principal_ref: &str,
) -> bool {
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
    let live: bool = tx
        .query_one(
            "SELECT public.trace_account_admission_live_device($1, $2, $3)",
            &[&tenant_id, &account_id, &principal_ref],
        )
        .await
        .expect("live device")
        .get(0);
    tx.rollback().await.expect("rollback");
    live
}

/// `trace_account_admission_linkage_ready()` as it would answer for a fleet
/// holding only `tenant_id`.
///
/// The function is fleet-wide and this database is shared with every other
/// test in the suite, which CI runs in parallel. Inside one transaction the
/// tables that decide readiness are locked against concurrent writers, every
/// other tenant's devices are revoked, its accounts closed, and every
/// legacy-link conflict (V81) resolved -- then the real function is asked,
/// and the transaction is rolled back. Only other tenants' rows are hidden,
/// so the answer is exactly what the function says about this tenant; the
/// predicate itself is never restated here.
async fn linkage_ready_for_tenant(backend: &PgBackend, tenant_id: &str) -> bool {
    for _ in 0..1000 {
        match linkage_ready_for_tenant_once(backend, tenant_id).await {
            Ok(ready) => return ready,
            // Another test holds a lock the probe needs. Back off and try
            // again rather than wait: see `lock_timeout` below.
            Err(error)
                if error.code() == Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE)
                    || error.code()
                        == Some(&tokio_postgres::error::SqlState::T_R_DEADLOCK_DETECTED) =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => panic!("readiness probe: {error:?}"),
        }
    }
    panic!("readiness probe could not lock the readiness tables");
}

async fn linkage_ready_for_tenant_once(
    backend: &PgBackend,
    tenant_id: &str,
) -> Result<bool, tokio_postgres::Error> {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let tx = client.transaction().await?;
    // Freeze every table whose OTHER-tenant rows can make readiness false.
    // SHARE ROW EXCLUSIVE blocks concurrent writes (and other probes) until
    // the rollback, so a row another test commits between the isolating
    // UPDATEs and the readiness call cannot reach the function. Without it
    // the suite's parallel tests raced this probe in CI.
    //
    // A short `lock_timeout` covers both the table locks and the row locks
    // the UPDATEs can meet (a merge holds its absorbed account `FOR UPDATE`,
    // which the table lock does not exclude). The probe gives up and retries
    // well inside `deadlock_timeout`, so while it holds the table locks it
    // never waits long enough to be half of a deadlock that the server
    // resolves by aborting another test's merge.
    tx.batch_execute(
        "SET LOCAL lock_timeout = '50ms';
         LOCK TABLE trace_accounts, device_keys, trace_legacy_invite_link_conflicts
            IN SHARE ROW EXCLUSIVE MODE",
    )
    .await?;
    tx.execute(
        "UPDATE device_keys SET revoked_at = now()
          WHERE tenant_id <> $1 AND revoked_at IS NULL",
        &[&tenant_id],
    )
    .await?;
    tx.execute(
        "UPDATE trace_accounts SET closed_at = now()
          WHERE tenant_id <> $1 AND closed_at IS NULL",
        &[&tenant_id],
    )
    .await?;
    tx.execute(
        "UPDATE trace_legacy_invite_link_conflicts SET resolved_at = now()
          WHERE resolved_at IS NULL",
        &[],
    )
    .await?;
    let ready: bool = tx
        .query_one("SELECT public.trace_account_admission_linkage_ready()", &[])
        .await?
        .get(0);
    tx.rollback().await?;
    Ok(ready)
}

/// `(account_id, count)` of the tenant's rows in `table`, sorted.
async fn near_rows_by_account(
    backend: &PgBackend,
    tenant_id: &str,
    table: &str,
) -> Vec<(Uuid, i64)> {
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    client
        .query(
            &format!(
                "SELECT account_id, count(*) FROM {table}
                  WHERE tenant_id = $1 GROUP BY account_id ORDER BY account_id"
            ),
            &[&tenant_id],
        )
        .await
        .expect("read near rows")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

#[tokio::test]
async fn merge_carries_near_anchors_and_devices_to_survivor() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_near_tenant();
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:near-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:near-b-live")
        .await
        .expect("mint B");
    // Both sides anchored, each to its own NEAR account.
    let anchor_a = seed_near_anchor(&backend, &tenant, account_a).await;
    let anchor_b = seed_near_anchor(&backend, &tenant, account_b).await;
    seed_near_device(
        &backend,
        &tenant,
        account_a,
        "principal:near-a",
        &anchor_a,
        false,
    )
    .await;
    seed_near_device(
        &backend,
        &tenant,
        account_b,
        "principal:near-b-live",
        &anchor_b,
        false,
    )
    .await;
    let revoked_device = seed_near_device(
        &backend,
        &tenant,
        account_b,
        "principal:near-b-revoked",
        &anchor_b,
        true,
    )
    .await;
    assert!(
        live_near_device(&backend, &tenant, account_b, "principal:near-b-live").await,
        "fixture: B's device is live on B before the merge"
    );
    assert!(
        linkage_ready_for_tenant(&backend, &tenant).await,
        "fixture: the tenant is linkage-ready before the merge"
    );

    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");
    restricted_merge_backend(&backend)
        .await
        .execute_merge(&tenant, account_a, staged.proposal_id)
        .await
        .expect("execute ok")
        .expect("executed some");

    assert_eq!(
        near_rows_by_account(&backend, &tenant, "trace_near_account_anchors").await,
        vec![(account_a, 2)],
        "both anchors end on the survivor; none stay on the closed account"
    );
    assert_eq!(
        near_rows_by_account(&backend, &tenant, "trace_near_provisioned_devices").await,
        vec![(account_a, 3)],
        "every provisioned device follows its anchor"
    );
    assert!(
        live_near_device(&backend, &tenant, account_a, "principal:near-b-live").await,
        "a device provisioned to the absorbed account is a live device of the survivor"
    );
    assert!(
        live_near_device(&backend, &tenant, account_a, "principal:near-a").await,
        "the survivor's own device is untouched"
    );
    assert!(
        !live_near_device(&backend, &tenant, account_a, "principal:near-b-revoked").await,
        "a revoked device stays revoked"
    );
    let still_revoked = raw_scalar_i64(
        &backend,
        &tenant,
        "SELECT count(*) FROM device_keys
          WHERE tenant_id = trace_current_tenant_id() AND device_key_id = $1
            AND revoked_at IS NOT NULL",
        &[&revoked_device],
    )
    .await;
    assert_eq!(still_revoked, 1);
    // A returning sign-in finds its account by anchor among OPEN accounts
    // only; with the anchor left on the closed account it would find none and
    // be refused.
    assert_eq!(
        raw_opt_uuid(
            &backend,
            &tenant,
            "SELECT a.account_id FROM trace_near_account_anchors n
               JOIN trace_accounts a USING (tenant_id, account_id)
              WHERE n.anchor_hash = $1 AND a.closed_at IS NULL",
            &[&anchor_b],
        )
        .await,
        Some(account_a),
        "the absorbed account's NEAR anchor resolves to the open survivor"
    );
    assert!(
        linkage_ready_for_tenant(&backend, &tenant).await,
        "a merge does not strand a live NEAR device on a closed account"
    );

    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let metadata: serde_json::Value = client
        .query_one(
            "SELECT safe_metadata FROM trace_account_audit
              WHERE tenant_id = $1 AND action = 'account_merged'",
            &[&tenant],
        )
        .await
        .expect("merge audit")
        .get(0);
    assert_eq!(metadata["near_anchors_carried"], 1);
    assert_eq!(metadata["near_devices_carried"], 2);

    // Outside the consuming transaction the function authorizes nothing.
    let tx = client.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant context");
    let replay = tx
        .query_one(
            "SELECT * FROM public.trace_near_account_merge($1, $2, $3, $4)",
            &[&tenant, &account_a, &account_b, &staged.proposal_id],
        )
        .await
        .expect_err("a proposal consumed by an earlier transaction authorizes nothing");
    assert_eq!(
        replay.as_db_error().map(|e| e.message()),
        Some("near_account_merge_unauthorized"),
        "refused by the function's own proof, not by its absence"
    );
    tx.rollback().await.expect("rollback");

    let guard = client
        .query_one(
            "SELECT r.rolcanlogin, r.rolbypassrls, r.rolsuper,
                    pg_get_userbyid(p.proowner) = r.rolname, p.prosecdef
               FROM pg_proc p, pg_roles r
              WHERE p.proname = 'trace_near_account_merge'
                AND r.rolname = 'trace_near_account_merge_guard'",
            &[],
        )
        .await
        .expect("inspect guard");
    assert!(!guard.get::<_, bool>(0), "guard cannot log in");
    assert!(!guard.get::<_, bool>(1), "guard does not bypass RLS");
    assert!(!guard.get::<_, bool>(2), "guard is not a superuser");
    assert!(guard.get::<_, bool>(3), "guard owns the function");
    assert!(guard.get::<_, bool>(4), "function is SECURITY DEFINER");
}

#[tokio::test]
async fn merge_without_near_rows_carries_nothing() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let (tenant, _a, _b, metadata) = merge_with_trust(
        &backend,
        "near-none",
        TrustSeed {
            trust: None,
            grants: &[],
        },
        TrustSeed {
            trust: None,
            grants: &[],
        },
    )
    .await;
    assert_eq!(metadata["near_anchors_carried"], 0);
    assert_eq!(metadata["near_devices_carried"], 0);
    assert!(
        near_rows_by_account(&backend, &tenant, "trace_near_account_anchors")
            .await
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Earned trust (V87, M4 of the earned-account-trust spec): a merge carries the
// absorbed account's trust facts to the survivor inside the executing merge,
// never duplicating one, and the union does not count a week twice.
// ---------------------------------------------------------------------------

/// An accepted submission from `principal`, with its `accepted` and passed
/// `gate_evaluation` facts on `account`, at `at`. Returns the submission.
async fn seed_trust_facts(
    backend: &PgBackend,
    tenant: &str,
    account: Uuid,
    principal: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    let submission = Uuid::new_v4();
    let decision = Uuid::new_v4();
    let hash = "a".repeat(64);
    raw_execute(
        backend,
        tenant,
        "INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,
            schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,
            redaction_pipeline_version,redaction_hash,received_at)
         VALUES(trace_current_tenant_id(),$1,$2,$3,'v1','v1','test','accepted','low','test',$4,$5)",
        &[&submission, &Uuid::new_v4(), &principal, &hash, &at],
    )
    .await;
    raw_execute(
        backend,
        tenant,
        "INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,
            gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,
            novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,
            attestation_chain_hash,decided_at,dedup_cluster_id,dedup_signal_version)
         VALUES(trace_current_tenant_id(),$1,$2,'gate-v1',$3,1,1,TRUE,1,$3,TRUE,$3,$3,$4,$5,
            'events.v2+simhash.v2')",
        &[&decision, &submission, &hash, &at, &Uuid::new_v4()],
    )
    .await;
    raw_execute(
        backend,
        tenant,
        "INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,
            submission_id,outcome,evaluator_version,occurred_at)
         VALUES(trace_current_tenant_id(),$1,'accepted_submission',$2,$2,'accepted',NULL,$3),
               (trace_current_tenant_id(),$1,'gate_evaluation',$4,$2,'evaluated_passed','gate-v1',$3)",
        &[&account, &submission, &at, &decision],
    )
    .await;
    submission
}

async fn fact_keys(backend: &PgBackend, tenant: &str, account: Uuid) -> Vec<(String, Uuid)> {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw connection");
    let tx = client.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant context");
    let rows = tx
        .query(
            "SELECT source_kind, source_id FROM trace_account_trust_facts
              WHERE tenant_id=$1 AND account_id=$2 ORDER BY source_kind, source_id",
            &[&tenant, &account],
        )
        .await
        .expect("facts");
    tx.commit().await.expect("commit");
    rows.into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

#[tokio::test]
async fn merge_carries_trust_facts_once_and_does_not_double_count_a_week() {
    use trace_commons_server::account_trust_rule::{evaluate, parse_shadow_growth_policy};
    let Some(backend) = postgres_backend().await else {
        return;
    };
    let tenant = unique_tenant("trust-facts-merge");
    let account_a = backend
        .create_or_reuse_account(&tenant, "principal:facts-a")
        .await
        .expect("mint A");
    let account_b = backend
        .create_or_reuse_account(&tenant, "principal:facts-b")
        .await
        .expect("mint B");
    // One qualified contribution each, in the same week; plus one fact that
    // both accounts already hold for the same source.
    // A day into last week's 7-day bucket, so both contributions (an hour
    // apart) share one bucket whatever the wall clock says.
    let bucket = chrono::Utc::now().timestamp().div_euclid(7 * 86_400) - 1;
    let week = chrono::DateTime::from_timestamp(bucket * 7 * 86_400 + 86_400, 0).expect("time");
    seed_trust_facts(&backend, &tenant, account_a, "principal:facts-a", week).await;
    seed_trust_facts(
        &backend,
        &tenant,
        account_b,
        "principal:facts-b",
        week + chrono::Duration::hours(1),
    )
    .await;
    // A fact both accounts already hold for the same source: a quarantine of
    // a submission with no acceptance, so it nets nothing out.
    let shared = Uuid::new_v4();
    raw_execute(
        &backend,
        &tenant,
        "INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,
            schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,
            redaction_pipeline_version,redaction_hash)
         VALUES(trace_current_tenant_id(),$1,$2,'principal:facts-b','v1','v1','test',
            'quarantined','medium','test',$3)",
        &[&shared, &Uuid::new_v4(), &"b".repeat(64)],
    )
    .await;
    for holder in [account_a, account_b] {
        raw_execute(
            &backend,
            &tenant,
            "INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,
                submission_id,outcome,evaluator_version,occurred_at)
             VALUES(trace_current_tenant_id(),$1,'submission_quarantined',$2,$2,'quarantined',
                NULL,$3)",
            &[&holder, &shared, &(week + chrono::Duration::hours(2))],
        )
        .await;
    }
    let before_a = fact_keys(&backend, &tenant, account_a).await;
    let before_b = fact_keys(&backend, &tenant, account_b).await;
    assert_eq!((before_a.len(), before_b.len()), (3, 3));

    let code_hash = unique_code_hash();
    seed_login_link(&backend, &tenant, account_b, &code_hash, false, false).await;
    let staged = backend
        .stage_merge_proposal(&tenant, account_a, &code_hash)
        .await
        .expect("stage ok")
        .expect("staged some");
    let restricted = restricted_merge_backend(&backend).await;
    let facts_rights: bool = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw")
        .query_one(
            "SELECT has_table_privilege('trace_trust_merge_runtime','trace_account_trust_facts','SELECT')
                 OR has_table_privilege('trace_trust_merge_runtime','trace_account_trust_facts','INSERT')",
            &[],
        )
        .await
        .expect("inspect")
        .get(0);
    assert!(!facts_rights, "the merge login holds no facts rights");
    restricted
        .execute_merge(&tenant, account_a, staged.proposal_id)
        .await
        .expect("execute ok")
        .expect("executed some");

    let mut expected: Vec<(String, Uuid)> = before_a.iter().chain(&before_b).cloned().collect();
    expected.sort();
    expected.dedup();
    assert_eq!(
        fact_keys(&backend, &tenant, account_a).await,
        expected,
        "the survivor holds the union, each source once"
    );
    assert_eq!(
        fact_keys(&backend, &tenant, account_b).await,
        before_b,
        "the absorbed account's rows stay, unreferenced, on the closed account"
    );

    // Two contributions in one week, weekly cap 1: the union counts the week
    // once and caps it, rather than adding each account's allowance of it.
    let policy = parse_shadow_growth_policy(
        &serde_json::json!({
            "version": "merge-test-v1",
            "processing_cost_bound": 1,
            "bounded_allowance": 10,
            "period": {"mode": "lifetime"},
            "growth_rule": "tiered-v1",
            "growth": {
                "window_seconds": 30 * 86400,
                "weekly_cap": 1,
                "q_min_micros": null,
                "penalty_cooldown_seconds": 0,
                "evaluation_max_age_seconds": 86400,
                "allowance_ceiling": 10,
                "evaluator_versions": ["gate-v1"],
                "dedup_signal_versions": ["events.v2+simhash.v2"],
                "tiers": [{"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1}]
            }
        })
        .to_string(),
        &["merge-test-v1"],
    )
    .expect("policy");
    // `unique_tenant` is not an anchored tenant, so the worker enumeration
    // never lists it; read the survivor's inputs directly.
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("raw");
    let tx = client.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant");
    let rows = tx
        .query(
            "SELECT source_kind, source_id, submission_id, outcome, evaluator_version, occurred_at,
                    credit_quality_micros, credit_quality_calibration_version,
                    dedup_signal_version, first_in_cluster
               FROM trace_account_trust_evaluation_inputs($1,$2)",
            &[&tenant, &account_a],
        )
        .await
        .expect("inputs");
    tx.commit().await.expect("commit");
    let facts: Vec<_> = rows
        .into_iter()
        .map(
            |row| trace_commons_server::account_trust_rule::EvaluationFact {
                source_kind: row.get(0),
                source_id: row.get(1),
                submission_id: row.get(2),
                outcome: row.get(3),
                evaluator_version: row.get(4),
                occurred_at: row.get(5),
                credit_quality_micros: row.get(6),
                credit_quality_calibration_version: row.get(7),
                dedup_signal_version: row.get(8),
                first_in_cluster: row.get(9),
            },
        )
        .collect();
    let evaluation = evaluate(&policy, &facts, chrono::Utc::now());
    assert_eq!(evaluation.units, 1, "weekly cap 1 over the union");
    assert_eq!(
        evaluation.units_capped, 1,
        "the second contribution is capped, not added"
    );
    assert_eq!(
        evaluation.active_weeks, 1,
        "the merged week is counted once"
    );
}
