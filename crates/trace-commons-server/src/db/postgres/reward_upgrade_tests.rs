// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: migrates a real V69 database through V70 before reading pagination.

use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::db::postgres::{MIGRATIONS, PgBackend, apply_and_record_migration};
use crate::{
    config::{DatabaseConfig, SslMode},
    db::Database,
    mission_rewards::{RewardActivityKind, RewardProgramTerms},
};

fn digest(seed: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(seed.as_bytes())))
}

fn reward_terms() -> RewardProgramTerms {
    RewardProgramTerms {
        schema_version: 1,
        activity_kind: RewardActivityKind::MissionCompletion,
        definition_hash: digest("upgrade-definition"),
        rubric_hash: digest("upgrade-rubric"),
        evaluator_policy_hash: digest("upgrade-evaluator"),
        required_evidence_hash: digest("upgrade-evidence-rules"),
        rights_hash: digest("upgrade-rights"),
        challenge_policy_hash: digest("upgrade-challenge"),
        sponsor_hash: digest("upgrade-sponsor"),
        award_units: 7,
        capacity_units: 14,
        participant_cap_units: 14,
        closes_at: Utc::now() + Duration::hours(1),
        reservation_ttl_seconds: 60,
    }
}

fn database_config(url: String) -> DatabaseConfig {
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

fn role_url(url: &str, role: &str) -> String {
    let mut parsed = reqwest::Url::parse(url).expect("parse explicit upgrade test URL");
    parsed.set_username(role).expect("set upgrade login role");
    parsed.into()
}

async fn reward_backend(url: &str, role: &str) -> PgBackend {
    PgBackend::new(&database_config(role_url(url, role)))
        .await
        .expect("connect upgrade test role")
}

fn isolated_upgrade_database_url() -> String {
    let url = std::env::var("TRACE_COMMONS_REWARDS_PG_UPGRADE_TEST_URL")
        .expect("TRACE_COMMONS_REWARDS_PG_UPGRADE_TEST_URL is required; no fallback is permitted");
    let target = url
        .parse::<tokio_postgres::Config>()
        .expect("parse explicit upgrade test URL");
    let hosts = target.get_hosts();
    let hosts_are_local = hosts.iter().all(|host| match host {
        tokio_postgres::config::Host::Tcp(host) => host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        #[cfg(unix)]
        tokio_postgres::config::Host::Unix(_) => true,
    });
    assert!(
        !hosts.is_empty() && hosts_are_local,
        "every explicit PostgreSQL host must be loopback or a local Unix socket"
    );
    assert!(
        target
            .get_hostaddrs()
            .iter()
            .all(std::net::IpAddr::is_loopback),
        "every PostgreSQL hostaddr override must be loopback"
    );
    assert!(
        target
            .get_dbname()
            .is_some_and(|name| name.starts_with("reward_test_")),
        "database name must start reward_test_"
    );
    url
}

async fn apply_real_migrations_through_v69(client: &mut Client) {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS _trace_commons_migrations (\
                version INTEGER PRIMARY KEY,\
                name TEXT NOT NULL,\
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
            );",
        )
        .await
        .expect("create normal migration history table");
    for (version, name, sql) in MIGRATIONS.iter().filter(|(version, _, _)| *version <= 69) {
        assert!(
            client
                .query_opt(
                    "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                    &[version],
                )
                .await
                .expect("check real migration record")
                .is_none(),
            "fresh upgrade database must not pre-record V{version}"
        );
        apply_and_record_migration(client, *version, name, sql)
            .await
            .unwrap_or_else(|error| panic!("apply real V{version} ({name}): {error}"));
    }
}

async fn legacy_history(backend: &PgBackend, tenant: &str, participant: &str) -> Value {
    let mut client = backend
        .trace_pool()
        .get()
        .await
        .expect("get legacy runtime pool");
    let transaction = client
        .transaction()
        .await
        .expect("start legacy history transaction");
    transaction
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .expect("bind legacy history tenant");
    let history = transaction
        .query_one(
            "SELECT public.trace_reward_history($1, $2, $3)",
            &[&tenant, &participant, &10_i32],
        )
        .await
        .expect("read V69 history")
        .get(0);
    transaction
        .commit()
        .await
        .expect("commit legacy history read");
    history
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_UPGRADE_TEST_URL"]
async fn reward_v69_upgrade_preserves_awards_and_installs_paginated_history() {
    let url = isolated_upgrade_database_url();
    let (mut admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect upgrade migration admin");
    tokio::spawn(async move { connection.await.expect("upgrade migration connection") });
    apply_real_migrations_through_v69(&mut admin).await;

    assert!(
        admin
            .query_one(
                "SELECT to_regprocedure('public.trace_reward_history(text,text,integer,uuid)') IS NULL",
                &[],
            )
            .await
            .expect("inspect predecessor history function")
            .get::<_, bool>(0),
        "V69 must expose only the predecessor three-argument history function"
    );
    assert!(
        admin
            .query_one(
                "SELECT NOT EXISTS (SELECT 1 FROM information_schema.columns \
                 WHERE table_schema = 'public' AND table_name = 'trace_reward_programs' \
                   AND column_name = 'identity_mode')",
                &[],
            )
            .await
            .expect("inspect predecessor program columns")
            .get::<_, bool>(0),
        "V69 must not already carry V70 identity storage"
    );

    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("reward-upgrade-{suffix}");
    let issuer = format!("reward_upgrade_issuer_{suffix}");
    let reviewer = format!("reward_upgrade_reviewer_{suffix}");
    let issuer_actor = digest(&format!("issuer-{suffix}"));
    let reviewer_actor = digest(&format!("reviewer-{suffix}"));
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .expect("insert upgrade tenant");
    for role in [&issuer, &reviewer] {
        admin
            .batch_execute(&format!(
                "CREATE ROLE {role} LOGIN NOSUPERUSER NOBYPASSRLS INHERIT; \
                 GRANT trace_reward_runtime TO {role}; GRANT USAGE ON SCHEMA public TO {role};"
            ))
            .await
            .expect("create narrow upgrade login");
    }
    admin
        .execute(
            "INSERT INTO trace_reward_operators \
             (tenant_id, login_role, actor_hash, operator_role) \
             VALUES ($1, $2::name, $3, 'issuer'), ($1, $4::name, $5, 'reviewer')",
            &[&tenant, &issuer, &issuer_actor, &reviewer, &reviewer_actor],
        )
        .await
        .expect("provision upgrade operators");

    let issuer_db = reward_backend(&url, &issuer).await;
    let reviewer_db = reward_backend(&url, &reviewer).await;
    let participant = digest("upgrade-participant");
    let program = Uuid::new_v4();
    issuer_db
        .reward_program_create(&tenant, program, &reward_terms())
        .await
        .expect("create actual V69 program");
    let mut reservations = Vec::new();
    for seed in ["upgrade-first", "upgrade-second"] {
        let reservation = Uuid::new_v4();
        issuer_db
            .reward_reserve(
                &tenant,
                program,
                reservation,
                &participant,
                &digest(&format!("{seed}-work")),
                &digest(&format!("{seed}-consent")),
            )
            .await
            .expect("reserve actual V69 award");
        issuer_db
            .reward_claim_submit(
                &tenant,
                reservation,
                &digest(&format!("{seed}-evidence")),
                &digest(&format!("{seed}-evaluation")),
            )
            .await
            .expect("submit actual V69 award");
        reviewer_db
            .reward_review(
                &tenant,
                reservation,
                Uuid::new_v4(),
                true,
                "completion_verified",
            )
            .await
            .expect("award actual V69 submission");
        reservations.push(reservation);
    }
    let before_upgrade = legacy_history(&issuer_db, &tenant, &participant).await;
    assert_eq!(before_upgrade["entries"].as_array().map(Vec::len), Some(2));
    assert_eq!(before_upgrade["programs"][0]["awarded_units"], json!(14));

    let migrator = PgBackend::new(&database_config(url.clone()))
        .await
        .expect("connect current production migrator");
    migrator
        .run_migrations()
        .await
        .expect("upgrade real V69 database through V70");

    assert_eq!(
        admin
            .query_one(
                "SELECT count(*) FROM _trace_commons_migrations \
                 WHERE version = 70 AND name = 'reward_history_pagination'",
                &[],
            )
            .await
            .expect("read V70 migration record")
            .get::<_, i64>(0),
        1,
        "the production migrator records V70 exactly once"
    );
    assert_eq!(
        admin
            .query_one(
                "SELECT identity_mode FROM trace_reward_programs \
                 WHERE tenant_id = $1 AND program_id = $2",
                &[&tenant, &program],
            )
            .await
            .expect("read V70 default identity mode")
            .get::<_, String>(0),
        "operator_asserted"
    );

    let first = issuer_db
        .reward_history_page(&tenant, &participant, 1, None)
        .await
        .expect("read first upgraded history page");
    assert_eq!(first["entries"].as_array().map(Vec::len), Some(1));
    assert_eq!(first["truncated"], true);
    assert_eq!(first["identity_mode"], "operator_asserted");
    assert_eq!(first["programs"][0]["identity_mode"], "operator_asserted");
    assert_eq!(first["programs"][0]["awarded_units"], json!(14));
    let cursor = Uuid::parse_str(first["next_cursor"].as_str().expect("first page cursor"))
        .expect("first page cursor UUID");
    let first_reservation = Uuid::parse_str(
        first["entries"][0]["reservation_id"]
            .as_str()
            .expect("first reservation UUID"),
    )
    .expect("parse first reservation UUID");

    let second = issuer_db
        .reward_history_page(&tenant, &participant, 1, Some(cursor))
        .await
        .expect("read second upgraded history page");
    assert_eq!(second["entries"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["truncated"], false);
    assert!(second["next_cursor"].is_null());
    assert_eq!(second["programs"][0]["awarded_units"], json!(14));
    let second_reservation = Uuid::parse_str(
        second["entries"][0]["reservation_id"]
            .as_str()
            .expect("second reservation UUID"),
    )
    .expect("parse second reservation UUID");
    assert_ne!(
        first_reservation, second_reservation,
        "pages must not overlap"
    );
    assert!(reservations.contains(&first_reservation));
    assert!(reservations.contains(&second_reservation));

    migrator
        .run_migrations()
        .await
        .expect("repeat current migration run");
    assert_eq!(
        issuer_db
            .reward_history_page(&tenant, &participant, 1, Some(cursor))
            .await
            .expect("read history after repeat migration"),
        second,
        "repeat migration preserves upgraded history"
    );
}
