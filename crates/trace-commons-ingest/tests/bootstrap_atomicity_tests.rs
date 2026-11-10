// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Real PostgreSQL regressions for commit-time first-authenticator admission.
//! Run with the ingest PG suite's disposable database and --test-threads=1.
use trace_commons_ingest::test_support::*;
use trace_commons_server::db::Database;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::HeaderValue;
use axum::http::HeaderMap;
use uuid::Uuid;
use trace_commons_server::db::postgres::PgBackend;
use std::sync::Arc;

#[tokio::test]
async fn database_default_cannot_bypass_atomic_enrollment() {
    let db = NativeAuthTestDb::default();
    for strong in [false, true] {
        assert!(
            db.enroll_near_identity(
                "tenant",
                Uuid::new_v4(),
                "key",
                "account.near",
                None,
                strong
            )
            .await
            .is_err()
        );
    }
}

struct ConcurrentWalletProofs(tokio::sync::Barrier);

#[async_trait::async_trait]
impl trace_commons_server::account_near::NearAccessKeyChecker for ConcurrentWalletProofs {
    async fn has_full_access_key(
        &self,
        _cfg: &trace_commons_server::config::NearConfig,
        _account_id: &str,
        _public_key: &str,
    ) -> anyhow::Result<bool> {
        // Both valid requests have passed the early count gate before either
        // is allowed to persist. This is the historical count0/count0 race.
        self.0.wait().await;
        Ok(true)
    }
}

#[tokio::test]
async fn concurrent_weak_enrollment_finishes_admit_exactly_one() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let tenant = "tenant-a";
    cleanup_pg_trace_tenant(backend.as_ref(), tenant).await;
    let temp = tempfile::tempdir().unwrap();
    let state = test_state_with_near(
        temp.path().to_path_buf(),
        Some(backend.clone()),
        Some(Arc::new(ConcurrentWalletProofs(tokio::sync::Barrier::new(
            2,
        )))),
    );
    let cookie = mint_redeem_session_cookie_value(&state, "token-a").await;
    let mut requests = Vec::new();
    for _ in 0..2 {
        let (message, nonce, recipient, ceremony) = near_enroll_start(&state, &cookie).await;
        let key = near_test_keypair();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!("{ACCOUNT_SESSION_COOKIE}={cookie}; {ceremony}"))
                .unwrap(),
        );
        let ctx = account_ctx_ext(&state, &headers).await;
        assert!(!ctx.0.is_strong_session());
        requests.push((
            ctx,
            headers,
            AccountNearEnrollFinishBody {
                account_id: "bootstrap.testnet".into(),
                public_key: near_test_pubkey_string(&key),
                signature: near_test_sign(&key, &message, &nonce, &recipient),
                label: None,
            },
        ));
    }
    let (ctx_a, headers_a, body_a) = requests.pop().unwrap();
    let account = ctx_a.0.account_id.as_uuid();
    let (ctx_b, headers_b, body_b) = requests.pop().unwrap();
    let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            account_near_enroll_finish_handler(
                State(state.clone()),
                ctx_a,
                headers_a,
                Json(body_a)
            ),
            account_near_enroll_finish_handler(
                State(state.clone()),
                ctx_b,
                headers_b,
                Json(body_b)
            ),
        )
    })
    .await
    .expect("concurrent finishes complete");
    let results = [a, b];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let denied = results.into_iter().find_map(Result::err).unwrap();
    assert_eq!(denied.0, StatusCode::FORBIDDEN);
    assert_eq!(
        backend
            .count_active_strong_authenticators(tenant, account)
            .await
            .unwrap(),
        1
    );
    assert_eq!(near_enrolled_audit_count(backend.as_ref(), tenant).await, 1);
    cleanup_pg_trace_tenant(backend.as_ref(), tenant).await;
}

#[tokio::test]
async fn atomic_enrollment_preserves_first_weak_and_strong_controls() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let tenant = "tenant-bootstrap-controls";
    cleanup_pg_trace_tenant(backend.as_ref(), tenant).await;
    let account = backend
        .create_or_reuse_account(tenant, "principal-a")
        .await
        .unwrap();
    assert!(
        backend
            .enroll_near_identity(
                tenant,
                account,
                "ed25519:bootstrap-one",
                "one.near",
                None,
                false
            )
            .await
            .unwrap()
    );
    assert!(
        !backend
            .enroll_near_identity(
                tenant,
                account,
                "ed25519:bootstrap-two",
                "two.near",
                None,
                false
            )
            .await
            .unwrap()
    );
    assert!(
        backend
            .enroll_near_identity(
                tenant,
                account,
                "ed25519:bootstrap-two",
                "two.near",
                None,
                true
            )
            .await
            .unwrap()
    );
    assert!(
        backend
            .enroll_near_identity(
                tenant,
                account,
                "ed25519:bootstrap-two",
                "two.near",
                None,
                true
            )
            .await
            .is_err(),
        "duplicate key stays a uniqueness refusal"
    );
    assert!(
        !backend
            .enroll_near_identity(
                "other-tenant",
                account,
                "ed25519:wrong-tenant",
                "other.near",
                None,
                false
            )
            .await
            .unwrap()
    );
    for key in ["ed25519:bootstrap-one", "ed25519:bootstrap-two"] {
        assert!(
            backend
                .revoke_account_near_identity(tenant, account, key)
                .await
                .unwrap()
                .removed
        );
    }
    assert!(
        backend
            .enroll_near_identity(
                tenant,
                account,
                "ed25519:bootstrap-three",
                "three.near",
                None,
                false
            )
            .await
            .unwrap()
    );
    cleanup_pg_trace_tenant(backend.as_ref(), tenant).await;
}

/// Observe actual PostgreSQL blocking, rather than guessing that a sleep means
/// the competing API has reached its admission transaction.
async fn wait_for_account_lock_waiter(backend: &PgBackend, blocker: i32) {
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = client.query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
                &[&blocker],
            ).await.unwrap().get(0);
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("enrollment must wait for the strong writer's account FK lock");
}

#[tokio::test]
async fn weak_admission_waits_for_provisioning_and_merge_foreign_keys() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    // These statements are the actual write shapes used by provisioning and
    // merge. They intentionally use only the schema's FK locking protocol.
    for kind in ["wallet", "anchor", "wallet_move", "anchor_move"] {
        let tenant = format!("tenant-bootstrap-{kind}");
        cleanup_bootstrap_tenant(backend.as_ref(), &tenant).await;
        let target = backend
            .create_or_reuse_account(&tenant, "target")
            .await
            .unwrap();
        let source = backend
            .create_or_reuse_account(&tenant, "source")
            .await
            .unwrap();
        let key = format!("ed25519:bootstrap-{kind}");
        let pool = backend.raw_pool_for_tests_and_diagnostics();
        let mut client = pool.get().await.unwrap();
        let blocker: i32 = client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        let first_account = if kind.ends_with("move") {
            source
        } else {
            target
        };
        if kind.starts_with("wallet") {
            tx.execute("INSERT INTO trace_near_identities(tenant_id,public_key,near_account_id,account_id) VALUES($1,$2,'strong.near',$3)", &[&tenant, &key, &first_account]).await.unwrap();
        } else {
            let anchor = format!("sha256:{}", "a".repeat(64));
            tx.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref,identity_source,auth_provider) VALUES($1,$2,$3,'{}','test-pepper','test-key','near_ai_login','google')", &[&tenant, &anchor, &first_account]).await.unwrap();
        }
        if kind.ends_with("move") {
            // Commit the source row, then start a destination move whose new
            // account FK must serialize with the destination bootstrap gate.
            tx.commit().await.unwrap();
        } else {
            // Keep the new strong authenticator uncommitted below.
            run_blocked_weak_admission(&backend, &tenant, target, blocker, tx).await;
            cleanup_bootstrap_tenant(backend.as_ref(), &tenant).await;
            continue;
        }
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        let sql = if kind == "wallet_move" {
            "UPDATE trace_near_identities SET account_id=$1 WHERE tenant_id=$2 AND account_id=$3"
        } else {
            "UPDATE trace_near_account_anchors SET account_id=$1 WHERE tenant_id=$2 AND account_id=$3"
        };
        tx.execute(sql, &[&target, &tenant, &source]).await.unwrap();
        run_blocked_weak_admission(&backend, &tenant, target, blocker, tx).await;
        cleanup_bootstrap_tenant(backend.as_ref(), &tenant).await;
    }
}

async fn cleanup_bootstrap_tenant(backend: &PgBackend, tenant: &str) {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    // Anchors deliberately have no tenant-delete cascade.
    tx.execute(
        "DELETE FROM trace_near_account_anchors WHERE tenant_id=$1",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    drop(client);
    cleanup_pg_trace_tenant(backend, tenant).await;
}

async fn run_blocked_weak_admission(
    backend: &Arc<PgBackend>,
    tenant: &str,
    account: Uuid,
    blocker: i32,
    writer: deadpool_postgres::Transaction<'_>,
) {
    assert_eq!(
        backend
            .count_active_strong_authenticators(tenant, account)
            .await
            .unwrap(),
        0,
        "an early gate cannot see the uncommitted strong authenticator"
    );
    let db = backend.clone();
    let owned_tenant = tenant.to_owned();
    let weak = tokio::spawn(async move {
        db.enroll_near_identity(
            &owned_tenant,
            account,
            "ed25519:blocked-weak",
            "weak.near",
            None,
            false,
        )
        .await
    });
    wait_for_account_lock_waiter(backend, blocker).await;
    assert!(!weak.is_finished());
    writer.commit().await.unwrap();
    assert!(
        !tokio::time::timeout(std::time::Duration::from_secs(5), weak)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "the post-lock count must see the committed strong authenticator"
    );
    assert_eq!(
        backend
            .count_active_strong_authenticators(tenant, account)
            .await
            .unwrap(),
        1
    );
}
