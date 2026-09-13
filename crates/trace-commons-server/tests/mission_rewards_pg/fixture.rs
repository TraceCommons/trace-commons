// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: exercises V69 through distinct PostgreSQL login roles and the public PgBackend API.

use std::process::Command;

use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use trace_commons_server::{
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
    mission_rewards::{RewardActivityKind, RewardError, RewardProgramTerms},
};
use uuid::Uuid;

static MIGRATED_DATABASE_URL: OnceCell<String> = OnceCell::const_new();

pub(crate) struct RewardPgFixture {
    pub(crate) url: String,
    pub(crate) tenant: String,
    pub(crate) suffix: String,
    pub(crate) issuer: String,
    pub(crate) reviewer: String,
    pub(crate) reviewer_actor: String,
    pub(crate) admin: tokio_postgres::Client,
    pub(crate) admin_db: PgBackend,
    pub(crate) issuer_db: PgBackend,
    pub(crate) reviewer_db: PgBackend,
    pub(crate) no_grant_db: PgBackend,
}

pub(crate) fn digest(seed: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(seed.as_bytes())))
}

pub(crate) fn terms(
    kind: RewardActivityKind,
    units: i64,
    capacity: i64,
    cap: i64,
) -> RewardProgramTerms {
    RewardProgramTerms {
        schema_version: 1,
        activity_kind: kind,
        definition_hash: digest("definition"),
        rubric_hash: digest("rubric"),
        evaluator_policy_hash: digest("evaluator"),
        required_evidence_hash: digest("evidence-rules"),
        rights_hash: digest("rights"),
        challenge_policy_hash: digest("challenge"),
        sponsor_hash: digest("sponsor"),
        award_units: units,
        capacity_units: capacity,
        participant_cap_units: cap,
        closes_at: Utc::now() + Duration::hours(1),
        reservation_ttl_seconds: 60,
    }
}

fn config(url: String) -> DatabaseConfig {
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

pub(crate) fn role_url(url: &str, role: &str) -> String {
    let mut parsed = reqwest::Url::parse(url).expect("parse explicit test URL");
    parsed.set_username(role).expect("set test login role");
    parsed.into()
}

async fn backend(url: &str, role: &str) -> PgBackend {
    PgBackend::new(&config(role_url(url, role)))
        .await
        .expect("connect as test login")
}

async fn migrated_database_url() -> String {
    MIGRATED_DATABASE_URL
        .get_or_init(|| async {
            let url = std::env::var("TRACE_COMMONS_REWARDS_PG_TEST_URL")
                .expect("TRACE_COMMONS_REWARDS_PG_TEST_URL is required; no fallback is permitted");
            let target = url
                .parse::<tokio_postgres::Config>()
                .expect("parse test URL");
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
            let hostaddrs = target.get_hostaddrs();
            assert!(
                hostaddrs.iter().all(std::net::IpAddr::is_loopback),
                "every PostgreSQL hostaddr override must be loopback"
            );
            assert!(
                target
                    .get_dbname()
                    .is_some_and(|name| name.starts_with("reward_test_")),
                "database name must start reward_test_"
            );

            let backend = PgBackend::new(&config(url.clone()))
                .await
                .expect("connect administrative migration backend");
            backend
                .run_migrations()
                .await
                .expect("apply the complete production migration chain");
            url
        })
        .await
        .clone()
}

impl RewardPgFixture {
    pub(crate) async fn new() -> Self {
        let url = migrated_database_url().await;
        let suffix = Uuid::new_v4().simple().to_string();
        let tenant = format!("reward-pg-{suffix}");
        let issuer = format!("reward_issuer_{suffix}");
        let reviewer = format!("reward_reviewer_{suffix}");
        let nobody = format!("reward_nogrant_{suffix}");
        let issuer_actor = digest(&format!("issuer-{suffix}"));
        let reviewer_actor = digest(&format!("reviewer-{suffix}"));

        let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect admin fixture");
        tokio::spawn(async move { connection.await.expect("admin connection") });
        admin
            .execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
                &[&tenant],
            )
            .await
            .expect("insert unique tenant");
        for role in [&issuer, &reviewer, &nobody] {
            admin.batch_execute(&format!("CREATE ROLE {role} LOGIN NOSUPERUSER NOBYPASSRLS INHERIT; GRANT trace_reward_runtime TO {role}; GRANT USAGE ON SCHEMA public TO {role};"))
                .await
                .expect("create narrow login");
        }
        admin.execute("INSERT INTO trace_reward_operators (tenant_id, login_role, actor_hash, operator_role) VALUES ($1, $2::name, $3, 'issuer'), ($1, $4::name, $5, 'reviewer')", &[&tenant, &issuer, &issuer_actor, &reviewer, &reviewer_actor])
            .await
            .expect("provision operator grants");

        Self {
            admin_db: PgBackend::new(&config(url.clone()))
                .await
                .expect("admin backend"),
            issuer_db: backend(&url, &issuer).await,
            reviewer_db: backend(&url, &reviewer).await,
            no_grant_db: backend(&url, &nobody).await,
            url,
            tenant,
            suffix,
            issuer,
            reviewer,
            reviewer_actor,
            admin,
        }
    }

    pub(crate) async fn runtime(&self) -> tokio_postgres::Client {
        let (runtime, connection) =
            tokio_postgres::connect(&role_url(&self.url, &self.issuer), tokio_postgres::NoTls)
                .await
                .expect("connect direct runtime verifier");
        tokio::spawn(async move { connection.await.expect("runtime connection") });
        runtime
            .execute(
                "SELECT set_config('trace_commons.trace_tenant_id', $1, false)",
                &[&self.tenant],
            )
            .await
            .expect("bind raw runtime verifier to tenant");
        runtime
    }
}

pub(crate) fn assert_refusal(result: Result<Value, RewardError>, expected: RewardError) {
    assert_eq!(result.expect_err("operation must be refused"), expected);
}

pub(crate) fn history_entry(history: &Value, reservation: Uuid) -> &Value {
    history["entries"]
        .as_array()
        .expect("history entries array")
        .iter()
        .find(|entry| entry["reservation_id"] == reservation.to_string())
        .expect("history contains reservation")
}

pub(crate) async fn reserve_and_submit(
    db: &PgBackend,
    tenant: &str,
    program: Uuid,
    participant: &str,
    seed: &str,
) -> (Uuid, String) {
    let reservation = Uuid::new_v4();
    let evidence = digest(&format!("{seed}-evidence"));
    db.reward_reserve(
        tenant,
        program,
        reservation,
        participant,
        &digest(&format!("{seed}-work")),
        &digest(&format!("{seed}-consent")),
    )
    .await
    .expect("reserve fixture");
    db.reward_claim_submit(
        tenant,
        reservation,
        &evidence,
        &digest(&format!("{seed}-evaluation")),
    )
    .await
    .expect("submit fixture");
    (reservation, evidence)
}

pub(crate) async fn award_submission(
    issuer: &PgBackend,
    reviewer: &PgBackend,
    tenant: &str,
    program: Uuid,
    participant: &str,
    seed: &str,
) -> Uuid {
    let (reservation, _) = reserve_and_submit(issuer, tenant, program, participant, seed).await;
    reviewer
        .reward_review(
            tenant,
            reservation,
            Uuid::new_v4(),
            true,
            "completion_verified",
        )
        .await
        .expect("award fixture submission");
    reservation
}

pub(crate) async fn create_program(
    db: &PgBackend,
    tenant: &str,
    kind: RewardActivityKind,
    units: i64,
    capacity: i64,
    participant_cap: i64,
) -> Uuid {
    let program = Uuid::new_v4();
    db.reward_program_create(
        tenant,
        program,
        &terms(kind, units, capacity, participant_cap),
    )
    .await
    .expect("create fixture program");
    program
}

pub(crate) async fn race_for_slot(
    db: &PgBackend,
    tenant: &str,
    program: Uuid,
    seed: &str,
) -> Result<Value, RewardError> {
    let participant = digest(seed);
    let work = digest(&format!("{seed}-work"));
    let consent = digest(&format!("{seed}-consent"));
    db.reward_reserve(
        tenant,
        program,
        Uuid::new_v4(),
        &participant,
        &work,
        &consent,
    )
    .await
}

pub(crate) async fn cli(url: &str, tenant: &str, arguments: &[String], success: bool) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-reward-operator"))
        .env("TRACE_COMMONS_REWARDS_DATABASE_URL", url)
        .args(["--tenant", tenant, "--json"])
        .args(arguments)
        .output()
        .expect("execute reward operator");
    assert_eq!(
        output.status.success(),
        success,
        "CLI stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if !success {
        assert!(
            output.stdout.is_empty(),
            "denied CLI request emits no JSON result"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("reward_unauthorized"),
            "safe denial label: {stderr}"
        );
        assert!(
            stderr.contains("verify the database login has the required tenant grant"),
            "denial has a next action: {stderr}"
        );
        return Value::Null;
    }
    serde_json::from_slice(&output.stdout).expect("CLI stdout is safe JSON")
}
