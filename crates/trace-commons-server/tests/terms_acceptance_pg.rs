// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The terms acceptance record (V158) on a real PostgreSQL, as the
//! least-privilege ingest runtime role: the append-only write, the
//! idempotent repeat, tenant isolation and the shape checks.

use chrono::{DateTime, Duration, Utc};
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, postgres::PgBackend},
};
use uuid::Uuid;

const VERSION: &str = "2026-08-22-draft";

fn hash(byte: &str) -> String {
    byte.repeat(32)
}

/// The migrator's backend (fixtures) and the ingest runtime's (the method
/// under V158's grants, with forced RLS).
async fn backends() -> Option<(PgBackend, PgBackend)> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).unwrap();
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
        .await
        .unwrap();
    admin.run_migrations().await.unwrap();
    let mut runtime_url = parsed.clone();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
        .await
        .unwrap();
    let row = runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert!(!row.get::<_, bool>(0) && !row.get::<_, bool>(1));
    Some((admin, runtime))
}

async fn account(admin: &PgBackend, tenant: &str) -> Uuid {
    admin
        .create_or_reuse_account(tenant, &format!("principal:{}", Uuid::new_v4()))
        .await
        .unwrap()
}

async fn rows_for(runtime: &PgBackend, tenant: &str, account: Uuid) -> i64 {
    let mut client = runtime.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.query_one(
        "SELECT count(*) FROM account_terms_acceptances WHERE account_id = $1",
        &[&account],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn records_and_returns_recorded_at() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("terms-{}", Uuid::new_v4());
    let account = account(&admin, &tenant).await;
    let before = Utc::now() - Duration::seconds(5);
    let recorded = runtime
        .record_terms_acceptance(
            &tenant,
            account,
            "native_token",
            "macos_app",
            VERSION,
            &hash("ab"),
            Utc::now(),
        )
        .await
        .unwrap();
    assert!(recorded > before && recorded < Utc::now() + Duration::seconds(5));
    assert_eq!(rows_for(&runtime, &tenant, account).await, 1);
}

#[tokio::test]
async fn repeat_is_idempotent() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("terms-{}", Uuid::new_v4());
    let account = account(&admin, &tenant).await;
    let first = runtime
        .record_terms_acceptance(
            &tenant,
            account,
            "device_bearer",
            "macos_app",
            VERSION,
            &hash("cd"),
            Utc::now(),
        )
        .await
        .unwrap();
    let second: DateTime<Utc> = runtime
        .record_terms_acceptance(
            &tenant,
            account,
            "session_cookie",
            "macos_app",
            VERSION,
            &hash("cd"),
            Utc::now() + Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(rows_for(&runtime, &tenant, account).await, 1);
}

#[tokio::test]
async fn new_version_is_a_new_row() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("terms-{}", Uuid::new_v4());
    let account = account(&admin, &tenant).await;
    for (version, h) in [(VERSION, "ab"), ("2026-12-01", "ab"), (VERSION, "ef")] {
        runtime
            .record_terms_acceptance(
                &tenant,
                account,
                "native_token",
                "macos_app",
                version,
                &hash(h),
                Utc::now(),
            )
            .await
            .unwrap();
    }
    assert_eq!(rows_for(&runtime, &tenant, account).await, 3);
}

#[tokio::test]
async fn other_tenant_cannot_read() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("terms-{}", Uuid::new_v4());
    let other = format!("terms-{}", Uuid::new_v4());
    let account = account(&admin, &tenant).await;
    runtime
        .record_terms_acceptance(
            &tenant,
            account,
            "native_token",
            "macos_app",
            VERSION,
            &hash("ab"),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(rows_for(&runtime, &other, account).await, 0);
    assert_eq!(rows_for(&runtime, &tenant, account).await, 1);
}

#[tokio::test]
async fn runtime_role_cannot_update_or_delete() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("terms-{}", Uuid::new_v4());
    let account = account(&admin, &tenant).await;
    runtime
        .record_terms_acceptance(
            &tenant,
            account,
            "native_token",
            "macos_app",
            VERSION,
            &hash("ab"),
            Utc::now(),
        )
        .await
        .unwrap();
    for statement in [
        "UPDATE account_terms_acceptances SET terms_version = 'tampered' WHERE account_id = $1",
        "DELETE FROM account_terms_acceptances WHERE account_id = $1",
    ] {
        let mut client = runtime.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        let err = tx.execute(statement, &[&account]).await.unwrap_err();
        assert!(
            err.to_string().contains("permission denied")
                || err
                    .as_db_error()
                    .is_some_and(|e| e.message().contains("permission denied")),
            "{statement}: {err:?}"
        );
    }
    assert_eq!(rows_for(&runtime, &tenant, account).await, 1);
}

#[tokio::test]
async fn check_constraints_refuse_bad_hash() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant = format!("terms-{}", Uuid::new_v4());
    let account = account(&admin, &tenant).await;
    for (version, bad_hash) in [
        (VERSION, "ABCD".repeat(16)),
        (VERSION, "ab".repeat(31)),
        ("has space", hash("ab")),
        ("", hash("ab")),
    ] {
        let result = runtime
            .record_terms_acceptance(
                &tenant,
                account,
                "native_token",
                "macos_app",
                version,
                &bad_hash,
                Utc::now(),
            )
            .await;
        assert!(result.is_err(), "{version:?} {bad_hash:?} must be refused");
    }
    assert_eq!(rows_for(&runtime, &tenant, account).await, 0);
}
