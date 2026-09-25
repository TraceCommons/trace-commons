// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use secrecy::SecretString;
use tokio_postgres::NoTls;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

fn database_url() -> Option<String> {
    std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
}

async fn migrated_client(url: &str) -> tokio_postgres::Client {
    let config = DatabaseConfig {
        url: SecretString::from(url.to_string()),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let backend = PgBackend::new(&config)
        .await
        .expect("connect to PostgreSQL");
    backend.run_migrations().await.expect("apply migrations");
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("direct PostgreSQL connection");
    tokio::spawn(async move {
        connection.await.expect("PostgreSQL connection");
    });
    client
}

#[tokio::test]
async fn session_tables_have_forced_rls_and_mapping_outlives_content() {
    let Some(url) = database_url() else {
        eprintln!("skipping: PostgreSQL test URL not configured");
        return;
    };
    let client = migrated_client(&url).await;
    for table in ["trace_source_sessions", "trace_submission_sessions"] {
        let row = client
            .query_one(
                "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE oid = to_regclass($1::text)",
                &[&table],
            )
            .await
            .expect("table in migration registry");
        assert!(row.get::<_, bool>(0), "RLS enabled for {table}");
        assert!(row.get::<_, bool>(1), "RLS forced for {table}");
    }

    let tenant = format!("z4-{}", Uuid::new_v4());
    let account = Uuid::new_v4();
    let submission = Uuid::new_v4();
    let digest = vec![0x5au8; 32];
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_source_sessions (tenant_id, account_id, session_digest) VALUES ($1, $2, $3)",
            &[&tenant, &account, &digest],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_submission_sessions (tenant_id, submission_id, account_id, session_digest) VALUES ($1, $2, $3, $4)",
            &[&tenant, &submission, &account, &digest],
        )
        .await
        .expect("mapping needs no content row");

    client
        .batch_execute(
            "DO $$ BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_z4_rls_probe') THEN
                    CREATE ROLE trace_z4_rls_probe NOLOGIN NOBYPASSRLS;
                END IF;
             END $$;
             GRANT USAGE ON SCHEMA public TO trace_z4_rls_probe;
             GRANT SELECT ON trace_source_sessions, trace_submission_sessions TO trace_z4_rls_probe;
             BEGIN;
             SET ROLE trace_z4_rls_probe;",
        )
        .await
        .expect("use a non-bypass role for the RLS assertion");
    let invisible = client
        .query_one("SELECT count(*) FROM trace_source_sessions", &[])
        .await
        .unwrap();
    assert_eq!(
        invisible.get::<_, i64>(0),
        0,
        "raw connection has no tenant context"
    );
    client
        .query_one(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
    let visible = client
        .query_one("SELECT count(*) FROM trace_source_sessions", &[])
        .await
        .unwrap();
    assert_eq!(
        visible.get::<_, i64>(0),
        1,
        "tenant-scoped transaction sees its row"
    );
    client.batch_execute("ROLLBACK").await.unwrap();

    let retargeted = client
        .execute(
            "UPDATE trace_submission_sessions SET account_id = account_id WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &submission],
        )
        .await;
    assert!(
        retargeted.is_err(),
        "even a no-op update cannot retarget a mapping"
    );
    client
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .unwrap();
}
