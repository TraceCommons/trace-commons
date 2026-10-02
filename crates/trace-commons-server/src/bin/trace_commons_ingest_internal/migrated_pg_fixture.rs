// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The migrated PostgreSQL both ignored ceremony suites start from.
//!
//! `nearai_ceremony_pg_tests` and `wallet_v2_pg_tests` each carried their own
//! copy of this setup, line for line. One copy changing without the other is
//! how a fixture silently stops modelling the system, so both call this.

use super::*;

/// A `DatabaseConfig` for `url`, with the login-resolver pool on its own role.
///
/// Aliasing the runtime and resolver pools is not a shortcut, it is #727: the
/// resolver resolves a blind index while holding no tenant context, which V61
/// permits only through a permissive policy scoped `TO trace_login_resolver`.
/// Any other non-superuser role is left with `trace_corpus_tenant_isolation`,
/// whose predicate compares `tenant_id` against a `trace_current_tenant_id()`
/// that is NULL there, so it matches nothing and a returning contributor looks
/// new.
pub(super) fn pg_config(url: &str, resolver_url: Option<&str>) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url.to_owned()),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url: resolver_url.map(|value| SecretString::from(value.to_owned())),
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

/// A database migrated to this build, and the URL it was reached on.
pub(super) struct MigratedPg {
    url: reqwest::Url,
    /// Connects as the URL's own role (the migration owner, a superuser in CI)
    /// for the runtime pool, and as `trace_login_resolver` for the resolver.
    pub(super) admin: Arc<PgBackend>,
}

/// Migrate the isolated database named by `env_var` and wire the resolver.
///
/// The URL must be on the literal loopback host, and when `db_prefix` is given
/// its database name must start with it: CI's committed-transaction guard reads
/// the database by that name, so a suite pointed anywhere else would pass
/// without that guard ever seeing it.
pub(super) async fn migrated_pg(env_var: &str, db_prefix: Option<&str>) -> MigratedPg {
    let url = std::env::var(env_var)
        .unwrap_or_else(|_| panic!("{env_var}: explicit isolated PostgreSQL URL required"));
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    if let Some(prefix) = db_prefix {
        let database = parsed.path().trim_start_matches('/');
        assert!(
            database.starts_with(prefix),
            "{env_var} must name a database starting {prefix}, got {database}"
        );
    }
    // Migrate first, with no resolver: V30 is what creates the role the
    // resolver pool connects as, so it cannot be wired before it exists.
    let migrator = PgBackend::new(&pg_config(&url, None)).await.unwrap();
    migrator.run_migrations().await.unwrap();

    // V30 creates `trace_login_resolver` NOLOGIN, so a pool cannot connect as
    // it until a test says so. The grant is the narrow one the resolver needs
    // and nothing more: it must never see `sealed_account_name`.
    superuser_ddl(
        &url,
        "ALTER ROLE trace_login_resolver LOGIN; \
         GRANT USAGE ON SCHEMA public TO trace_login_resolver; \
         GRANT SELECT (tenant_id, anchor_hash) \
           ON trace_near_account_anchors TO trace_login_resolver;",
    )
    .await;

    let admin = PgBackend::new(&pg_config(&url, Some(&resolver_url(&parsed))))
        .await
        .unwrap();
    MigratedPg {
        url: parsed,
        admin: Arc::new(admin),
    }
}

impl MigratedPg {
    /// A backend whose runtime pool connects as `role`, created LOGIN
    /// NOSUPERUSER NOBYPASSRLS with ordinary table grants, as
    /// `tests/account_onboarding_pg.rs` does with `tc_near_runtime`.
    ///
    /// The migration owner is a superuser in CI, and a superuser bypasses every
    /// policy even on FORCE ROW LEVEL SECURITY tables, so a runtime pool on that
    /// role cannot notice a write or a read made outside its tenant context.
    /// This one can. The role is checked after it is provisioned rather than
    /// trusted, because a pre-existing role of the same name keeps whatever
    /// attributes it already had.
    pub(super) async fn narrow_runtime(&self, role: &str) -> Arc<PgBackend> {
        assert!(
            role.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "fixture role names are interpolated into DDL"
        );
        superuser_ddl(
            self.url.as_str(),
            &format!(
                "DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='{role}') \
                 THEN CREATE ROLE {role} LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; \
                 GRANT USAGE ON SCHEMA public TO {role}; \
                 GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO {role}; \
                 GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO {role};"
            ),
        )
        .await;
        let admin = self
            .admin
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let row = admin
            .query_one(
                "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname=$1",
                &[&role],
            )
            .await
            .unwrap();
        let (superuser, bypass): (bool, bool) = (row.get(0), row.get(1));
        assert!(
            !superuser && !bypass,
            "{role} must be subject to row-level security"
        );
        let mut runtime = self.url.clone();
        runtime.set_username(role).unwrap();
        Arc::new(
            PgBackend::new(&pg_config(runtime.as_str(), Some(&resolver_url(&self.url))))
                .await
                .unwrap(),
        )
    }
}

fn resolver_url(url: &reqwest::Url) -> String {
    let mut resolver = url.clone();
    resolver.set_username("trace_login_resolver").unwrap();
    resolver.into()
}

async fn superuser_ddl(url: &str, sql: &str) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .unwrap();
    let handle = tokio::spawn(connection);
    client.batch_execute(sql).await.unwrap();
    drop(client);
    handle.abort();
}
