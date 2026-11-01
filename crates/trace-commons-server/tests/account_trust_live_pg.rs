// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust, live (V126, decided 2026-10-11) against a real
//! PostgreSQL: the attested_submission fact verified from V76's witness
//! evidence, the guard's column grants, and a `tiered-v1` admission policy
//! whose in-process `applied` evaluation raises an account's allowance,
//! falling back to the base allowance when the evaluation is stale or its
//! inputs changed. Explicit isolated database only; never falls back to
//! DATABASE_URL.
//!
//! This suite turns external growth on (V114), which no runtime path turns
//! off, so it has a database of its own.

use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;
use trace_commons_server::account_trust::{
    TrustAccount, TrustFactOutcome, TrustFactSource, parse_bounded_policy,
    resolve_contribution_account,
};
use trace_commons_server::account_trust_growth::{
    EvaluationMode, evaluate_account_trust, explain_account_trust, record_account_trust_facts,
};
use trace_commons_server::account_trust_rule::{GrowthPolicy, parse_growth_policy};
use trace_commons_server::admission_ledger::{AccountAdmissionReservation, AdmissionDecision};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

const URL_VAR: &str = "TRACE_COMMONS_ACCOUNT_TRUST_LIVE_PG_TEST_DATABASE_URL";
const DEDUP: &str = "events.v2+simhash.v2";
const VERSION: &str = "live-growth-test-v1";

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: url.into(),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

struct Fixture {
    admin: deadpool_postgres::Object,
    url: String,
    _admin_db: PgBackend,
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

/// The workers enumerate every account in the database, so the tests run
/// one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var(URL_VAR) else {
        eprintln!("SKIPPED: isolated earned-trust live PostgreSQL fixture required ({URL_VAR})");
        return None;
    };
    let serial = SERIAL.lock().await;
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(
        parsed
            .path()
            .starts_with("/admission_test_account_trust_live")
    );
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    Some(Fixture {
        admin,
        url,
        _admin_db: admin_db,
        _serial: serial,
    })
}

/// A NOBYPASSRLS login holding exactly the named NOLOGIN roles.
async fn login_with_roles(fx: &Fixture, login: &str, roles: &[&str]) -> Arc<PgBackend> {
    fx.admin
        .batch_execute(&format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{login}') \
             THEN CREATE ROLE {login} LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;"
        ))
        .await
        .unwrap();
    for role in roles {
        fx.admin
            .batch_execute(&format!("GRANT {role} TO {login};"))
            .await
            .unwrap();
    }
    let mut url = reqwest::Url::parse(&fx.url).unwrap();
    url.set_username(login).unwrap();
    Arc::new(PgBackend::new(&config(url.into())).await.unwrap())
}

fn hash_ref() -> String {
    format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2))
}

fn hex64() -> String {
    Uuid::new_v4().simple().to_string().repeat(2)
}

/// An anchored account with one provisioned device, so admission resolves it.
async fn seed_account(admin: &deadpool_postgres::Object) -> (String, Uuid, String) {
    let tenant = format!("near-{}", hex64());
    let account = Uuid::new_v4();
    let principal = hash_ref();
    let device = hash_ref();
    let anchor = hash_ref();
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,'fixture-pepper','fixture-key')", &[&tenant,&account,&anchor,&serde_json::json!({"fixture":true})]).await.unwrap();
    admin.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,'fixture-key',NULL,'near')", &[&device,&tenant]).await.unwrap();
    admin.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant,&account,&principal]).await.unwrap();
    admin.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)", &[&tenant,&principal,&account,&device,&anchor]).await.unwrap();
    (tenant, account, principal)
}

/// How a seeded submission's gate decision and witness evidence look.
#[derive(Clone, Copy)]
struct Shape {
    status: &'static str,
    gate_passed: bool,
    /// `None`: no witness evidence row at all.
    witness: Option<(i16, &'static str)>,
    /// The witness names an artifact other than the current object.
    artifact_matches: bool,
}

const ATTESTED_FAILED_GATE: Shape = Shape {
    status: "accepted",
    gate_passed: false,
    witness: Some((2, "provider_tee_final_call")),
    artifact_matches: true,
};

/// An accepted submission with its credit event, a first-in-cluster gate
/// decision, its submitted_envelope object and (per `shape`) witness evidence.
async fn seed_submission(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    principal: &str,
    at: DateTime<Utc>,
    shape: Shape,
) -> Uuid {
    let submission = Uuid::new_v4();
    let trace = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,
                schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,
                redaction_pipeline_version,redaction_hash,received_at)
             VALUES($1,$2,$3,$4,'v1','v1','test',$5,'low','test',$6,$7)",
            &[
                &tenant,
                &submission,
                &trace,
                &principal,
                &shape.status,
                &"a".repeat(64),
                &at,
            ],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_credit_ledger(tenant_id,credit_event_id,submission_id,trace_id,
                credit_account_ref,event_type,points_delta,reason,actor_principal_ref,actor_role,
                settlement_state,occurred_at)
             VALUES($1,$2,$3,$4,'fixture','accepted','0','fixture',$5,'system','pending',$6)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &submission,
                &trace,
                &principal,
                &at,
            ],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_gate_decisions(tenant_id,decision_id,submission_id,gate_policy_version,
                gate_version_hash,perplexity_micros,tail_fraction_micros,perplexity_passed,
                novelty_score_micros,nearest_neighbor_hash,novelty_passed,embedding_evidence_hash,
                attestation_chain_hash,dedup_cluster_id,dedup_signal_version,decided_at)
             VALUES($1,$2,$3,'gate-v1',$4,1,1,$5,1,$4,TRUE,$4,$4,$6,$7,$8)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &submission,
                &"a".repeat(64),
                &shape.gate_passed,
                &Uuid::new_v4(),
                &DEDUP,
                &at,
            ],
        )
        .await
        .unwrap();
    let object_sha = hex64();
    admin
        .execute(
            "INSERT INTO trace_object_refs(tenant_id,submission_id,object_ref_id,artifact_kind,
                object_store,object_key,content_sha256,encryption_key_ref,size_bytes)
             VALUES($1,$2,$3,'submitted_envelope','fixture','fixture',$4,'fixture',1)",
            &[&tenant, &submission, &Uuid::new_v4(), &object_sha],
        )
        .await
        .unwrap();
    if let Some((version, class)) = shape.witness {
        let artifact = if shape.artifact_matches {
            object_sha
        } else {
            hex64()
        };
        let (model, signer): (Option<&str>, Option<&str>) = if class == "unattested" {
            (None, None)
        } else {
            (Some("fixture-model"), Some("fixture-signer"))
        };
        admin
            .execute(
                "INSERT INTO trace_witness_certificate_evidence(tenant_id,submission_id,
                    certificate_json,signature_header,raw_body_sha256,artifact_sha256,
                    certificate_version,inference_class,bound_model,receipt_signer,issued_at)
                 VALUES($1,$2,'\\x7b7d','\\x00',$3,$4,$5,$6,$7,$8,$9)",
                &[
                    &tenant,
                    &submission,
                    &hex64(),
                    &artifact,
                    &version,
                    &class,
                    &model,
                    &signer,
                    &(at - Duration::minutes(5)),
                ],
            )
            .await
            .unwrap();
    }
    submission
}

async fn find(worker: &PgBackend, tenant: &str, account: Uuid) -> TrustAccount {
    let mut after: Option<TrustAccount> = None;
    loop {
        let page = worker
            .list_account_trust_worker_accounts(after.as_ref(), 200)
            .await
            .unwrap();
        assert!(!page.is_empty(), "account enumerated");
        if let Some(found) = page
            .iter()
            .find(|a| a.tenant_id() == tenant && a.account_id() == account)
        {
            return found.clone();
        }
        after = page.last().cloned();
    }
}

fn policy_json(base: i64, multiplier: i64, max_age_seconds: i64) -> String {
    serde_json::json!({
        "version": VERSION,
        "processing_cost_bound": 10,
        "bounded_allowance": base,
        "period": {"mode": "fixed", "seconds": 604800},
        "growth_rule": "tiered-v1",
        "growth": {
            "window_seconds": 84 * 86400,
            "weekly_cap": 10,
            "q_min_micros": null,
            "penalty_cooldown_seconds": 90 * 86400,
            "evaluation_max_age_seconds": max_age_seconds,
            "allowance_ceiling": base * multiplier,
            "evaluator_versions": ["gate-v1"],
            "dedup_signal_versions": [DEDUP],
            "tiers": [
                {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                {"units": 2, "active_weeks": 1, "age_seconds": 0, "multiplier": multiplier,
                 "attested_units": 2}
            ]
        }
    })
    .to_string()
}

fn growth_policy(json: &str) -> GrowthPolicy {
    parse_growth_policy(json, &[VERSION]).unwrap()
}

#[tokio::test]
async fn attested_facts_are_verified_from_witness_evidence() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let worker = login_with_roles(
        &fx,
        "tc_live_trust_worker_login",
        &["trace_account_trust_worker"],
    )
    .await;
    let (tenant, account_id, principal) = seed_account(admin).await;
    let at = Utc::now() - Duration::days(3);
    let good = seed_submission(admin, &tenant, &principal, at, ATTESTED_FAILED_GATE).await;
    let gateway = seed_submission(
        admin,
        &tenant,
        &principal,
        at,
        Shape {
            witness: Some((2, "gateway_final_call")),
            ..ATTESTED_FAILED_GATE
        },
    )
    .await;
    let legacy_v1 = seed_submission(
        admin,
        &tenant,
        &principal,
        at,
        Shape {
            witness: Some((1, "unattested")),
            ..ATTESTED_FAILED_GATE
        },
    )
    .await;
    let mismatched = seed_submission(
        admin,
        &tenant,
        &principal,
        at,
        Shape {
            artifact_matches: false,
            ..ATTESTED_FAILED_GATE
        },
    )
    .await;
    let quarantined = seed_submission(
        admin,
        &tenant,
        &principal,
        at,
        Shape {
            status: "quarantined",
            ..ATTESTED_FAILED_GATE
        },
    )
    .await;
    let no_witness = seed_submission(
        admin,
        &tenant,
        &principal,
        at,
        Shape {
            witness: None,
            ..ATTESTED_FAILED_GATE
        },
    )
    .await;
    let account = find(&worker, &tenant, account_id).await;

    // Only the verified provider-TEE submission is a candidate.
    let attested: Vec<Uuid> = worker
        .list_account_trust_fact_candidates(&account, 10_000)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|source| match source {
            TrustFactSource::AttestedSubmission(id) => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(attested, vec![good]);
    // And the recorder declines each of the others even when asked directly.
    for (label, submission) in [
        ("gateway", gateway),
        ("v1", legacy_v1),
        ("artifact mismatch", mismatched),
        ("not accepted", quarantined),
        ("no witness", no_witness),
    ] {
        assert_eq!(
            worker
                .record_account_trust_fact(
                    &account,
                    TrustFactSource::AttestedSubmission(submission)
                )
                .await
                .unwrap(),
            None,
            "{label}"
        );
    }

    let summary = record_account_trust_facts(worker.as_ref(), Some(10_000), false)
        .await
        .unwrap();
    assert_eq!(summary.failed, 0);
    let row = admin
        .query_one(
            "SELECT outcome, occurred_at, evaluator_version FROM trace_account_trust_facts
              WHERE tenant_id=$1 AND account_id=$2 AND source_kind='attested_submission'",
            &[&tenant, &account_id],
        )
        .await
        .unwrap();
    assert_eq!(
        TrustFactOutcome::from_storage(row.get::<_, &str>(0)),
        Some(TrustFactOutcome::AttestedProviderTee)
    );
    let issued: DateTime<Utc> = admin
        .query_one(
            "SELECT issued_at FROM trace_witness_certificate_evidence
              WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &good],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        row.get::<_, DateTime<Utc>>(1),
        issued,
        "occurred_at is issued_at"
    );
    assert_eq!(row.get::<_, Option<String>>(2), None);
    // Idempotent.
    assert_eq!(
        worker
            .list_account_trust_fact_candidates(&account, 10_000)
            .await
            .unwrap()
            .into_iter()
            .filter(|s| matches!(s, TrustFactSource::AttestedSubmission(_)))
            .count(),
        0
    );
    // The evaluator reads it with a first-in-cluster boolean, and it counts
    // although the submission's gate decision failed.
    let facts = worker
        .account_trust_evaluation_inputs(&account)
        .await
        .unwrap();
    let fact = facts
        .iter()
        .find(|f| f.source_kind == "attested_submission")
        .expect("attested fact read");
    assert_eq!(fact.first_in_cluster, Some(true));

    // An unlinked device sheds no adverse fact but earns no new attested one.
    let (tenant2, account2, principal2) = seed_account(admin).await;
    let later = seed_submission(admin, &tenant2, &principal2, at, ATTESTED_FAILED_GATE).await;
    admin
        .execute(
            "UPDATE trace_account_principals SET unlinked_at=clock_timestamp()
              WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant2, &account2],
        )
        .await
        .unwrap();
    let unlinked = find(&worker, &tenant2, account2).await;
    assert_eq!(
        worker
            .record_account_trust_fact(&unlinked, TrustFactSource::AttestedSubmission(later))
            .await
            .unwrap(),
        None
    );

    // The table refuses an attested fact that does not name its submission.
    let refused = admin
        .execute(
            "INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,
                submission_id,outcome,occurred_at)
             VALUES($1,$2,'attested_submission',$3,$4,'attested_provider_tee',now())",
            &[&tenant, &account_id, &Uuid::new_v4(), &good],
        )
        .await;
    assert!(refused.is_err(), "source_id must equal submission_id");
}

#[tokio::test]
async fn the_guard_reads_only_the_witness_columns_it_needs() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    let privilege = |table: &'static str, column: &'static str| async move {
        admin
            .query_one(
                "SELECT has_column_privilege('trace_account_admission_guard',$1,$2,'SELECT')",
                &[&table, &column],
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    };
    for column in [
        "tenant_id",
        "submission_id",
        "certificate_version",
        "inference_class",
        "artifact_sha256",
        "issued_at",
    ] {
        assert!(
            privilege("trace_witness_certificate_evidence", column).await,
            "{column}"
        );
    }
    for column in [
        "certificate_json",
        "signature_header",
        "raw_body_sha256",
        "bound_model",
        "receipt_signer",
    ] {
        assert!(
            !privilege("trace_witness_certificate_evidence", column).await,
            "the guard never reads {column}"
        );
    }
    for column in ["object_key", "encryption_key_ref", "object_store"] {
        assert!(
            !privilege("trace_object_refs", column).await,
            "the guard never reads {column}"
        );
    }
    assert!(privilege("trace_object_refs", "content_sha256").await);
    // The worker login reads witness evidence only through the guard.
    let worker = login_with_roles(
        &fx,
        "tc_live_trust_worker_login",
        &["trace_account_trust_worker"],
    )
    .await;
    let client = worker
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    for table in [
        "trace_witness_certificate_evidence",
        "trace_object_refs",
        "trace_account_trust_facts",
        "trace_account_trust_evaluations",
    ] {
        let writable: bool = client
            .query_one(
                "SELECT has_table_privilege(current_user,$1,'INSERT,UPDATE,DELETE')",
                &[&table],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!writable, "{table}");
    }
    assert!(
        client
            .query(
                "SELECT 1 FROM trace_witness_certificate_evidence LIMIT 1",
                &[]
            )
            .await
            .is_err(),
        "no direct read of witness evidence"
    );
}

#[tokio::test]
async fn an_applied_evaluation_grows_the_allowance_and_falls_back_to_the_base() {
    let Some(fx) = fixture().await else {
        return;
    };
    let admin = &fx.admin;
    // The pilot's shape: one ingest login holds the admission runtime and
    // the trust worker roles.
    let ingest = login_with_roles(
        &fx,
        "tc_live_ingest_login",
        &[
            "trace_account_admission_runtime",
            "trace_account_trust_worker",
        ],
    )
    .await;
    let runtime_only = login_with_roles(
        &fx,
        "tc_live_runtime_only_login",
        &["trace_account_admission_runtime"],
    )
    .await;
    ingest.enable_external_account_trust_growth().await.unwrap();
    assert!(ingest.external_account_trust_runtime_ready().await.unwrap());
    assert!(ingest.tiered_account_trust_runtime_ready().await.unwrap());
    assert!(
        !runtime_only
            .tiered_account_trust_runtime_ready()
            .await
            .unwrap(),
        "a login without the worker role cannot evaluate tiered-v1"
    );

    let base = 10;
    let json = policy_json(base, 5, 3600);
    let admission_policy = parse_bounded_policy(&json, &[VERSION]).unwrap();
    let growth = growth_policy(&json);

    let (tenant, account_id, principal) = seed_account(admin).await;
    let account = resolve_contribution_account(ingest.as_ref(), &tenant, &principal)
        .await
        .unwrap();
    let reserve = |policy: trace_commons_server::account_trust::BoundedPolicy| {
        let account = account.clone();
        let principal = principal.clone();
        let ingest = ingest.clone();
        async move {
            let request = AccountAdmissionReservation {
                account,
                principal_ref: principal,
                expected_trust_version: None,
                submission_id: Uuid::new_v4(),
                body_hash: "a".repeat(64),
                lease_id: Uuid::new_v4(),
                policy,
                lease_seconds: 60,
                attested: false,
            };
            let decision = ingest
                .reserve_account_admission(&request)
                .await
                .unwrap()
                .decision;
            (decision, request.submission_id)
        }
    };
    let none_policy = parse_bounded_policy(
        &format!(
            r#"{{"version":"{VERSION}","processing_cost_bound":10,"bounded_allowance":{base},"period":{{"mode":"fixed","seconds":604800}},"growth_rule":"none"}}"#
        ),
        &[VERSION],
    )
    .unwrap();

    // The client-facing allowance must use the exact validated allowance
    // that decides ready, including fallbacks, without changing period spend.
    let status = || async {
        ingest
            .account_admission_status(&account, &principal, &admission_policy)
            .await
            .unwrap()
            .unwrap()
    };

    // Tier 0: the base allowance of one reservation.
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Reserved
    );
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Exhausted
    );

    // Two attested acceptances whose gate decisions failed: (b) qualifies
    // them, and they meet tier 1's two-attested-unit requirement (c).
    for days in [2, 1] {
        seed_submission(
            admin,
            &tenant,
            &principal,
            Utc::now() - Duration::days(days),
            ATTESTED_FAILED_GATE,
        )
        .await;
    }
    let recorded = record_account_trust_facts(ingest.as_ref(), Some(10_000), false)
        .await
        .unwrap();
    assert_eq!(recorded.failed, 0);
    let trust = find(&ingest, &tenant, account_id).await;

    // A shadow row of the same version never raises the allowance.
    let shadow = evaluate_account_trust(
        ingest.as_ref(),
        &growth,
        EvaluationMode::Shadow,
        Utc::now(),
        Some(10_000),
    )
    .await
    .unwrap();
    assert_eq!(shadow.failed, 0);
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Exhausted,
        "shadow is never read"
    );
    let shadow_status = status().await;
    assert_eq!(shadow_status.effective_allowance, Some(base));
    assert_eq!(shadow_status.growth.unwrap().period_spend, base);

    let applied = evaluate_account_trust(
        ingest.as_ref(),
        &growth,
        EvaluationMode::Applied,
        Utc::now(),
        Some(10_000),
    )
    .await
    .unwrap();
    assert_eq!(applied.mode, "applied");
    assert_eq!(applied.failed, 0, "{applied:?}");
    let stored = ingest
        .latest_account_trust_evaluation(&trust, VERSION, "applied")
        .await
        .unwrap()
        .expect("applied row");
    assert_eq!((stored.tier, stored.attested_units), (1, 2));
    assert_eq!(stored.effective_allowance, base * 5);
    let generation: Option<i64> = admin
        .query_one(
            "SELECT input_generation FROM trace_account_trust_evaluations
              WHERE tenant_id=$1 AND account_id=$2 AND mode='applied'",
            &[&tenant, &account_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(generation.is_some(), "an applied row is generation-bound");
    let explained =
        explain_account_trust(ingest.as_ref(), &growth, EvaluationMode::Applied, &trust)
            .await
            .unwrap();
    assert!(explained.reproduced, "{explained:?}");

    let raised_status = status().await;
    assert!(raised_status.ready);
    assert_eq!(raised_status.effective_allowance, Some(base * 5));
    assert_eq!(
        raised_status.growth.unwrap().period_spend,
        base,
        "growth preserves spend"
    );

    // Tier 1: five times the base. The reservation records what it relied on.
    let (decision, submission) = reserve(admission_policy.clone()).await;
    assert_eq!(decision, AdmissionDecision::Reserved);
    let evidence = admin
        .query_one(
            "SELECT earned_tier, trust_evaluation_digest FROM trace_account_admission_submissions
              WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &submission],
        )
        .await
        .unwrap();
    assert_eq!(evidence.get::<_, Option<i32>>(0), Some(1));
    assert_eq!(
        evidence.get::<_, Option<String>>(1),
        Some(stored.facts_digest.clone())
    );

    // "none" is unchanged: the same fresh applied row raises nothing.
    assert_eq!(
        reserve(none_policy.clone()).await.0,
        AdmissionDecision::Exhausted,
        "growth_rule none reads no evaluation"
    );

    // Stale: older than evaluation_max_age_seconds reads as tier 0.
    admin
        .execute(
            "UPDATE trace_account_trust_evaluations SET as_of=as_of - interval '2 hours'
              WHERE tenant_id=$1 AND account_id=$2 AND mode='applied'",
            &[&tenant, &account_id],
        )
        .await
        .unwrap();
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Exhausted,
        "stale falls to the base"
    );
    let stale_status = status().await;
    assert!(!stale_status.ready);
    assert_eq!(stale_status.effective_allowance, Some(base));
    assert_eq!(stale_status.growth.unwrap().period_spend, base * 2);
    evaluate_account_trust(
        ingest.as_ref(),
        &growth,
        EvaluationMode::Applied,
        Utc::now(),
        Some(10_000),
    )
    .await
    .unwrap();
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Reserved
    );

    // Changed inputs: a new fact advances the generation, so the evaluation
    // stops counting until the next run.
    seed_submission(
        admin,
        &tenant,
        &principal,
        Utc::now() - Duration::hours(1),
        ATTESTED_FAILED_GATE,
    )
    .await;
    record_account_trust_facts(ingest.as_ref(), Some(10_000), false)
        .await
        .unwrap();
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Exhausted,
        "an invalidated evaluation falls to the base"
    );
    let invalidated_status = status().await;
    assert!(!invalidated_status.ready);
    assert_eq!(invalidated_status.effective_allowance, Some(base));
    assert_eq!(invalidated_status.growth.unwrap().period_spend, base * 3);
    evaluate_account_trust(
        ingest.as_ref(),
        &growth,
        EvaluationMode::Applied,
        Utc::now(),
        Some(10_000),
    )
    .await
    .unwrap();
    assert_eq!(
        reserve(admission_policy.clone()).await.0,
        AdmissionDecision::Reserved
    );

    // The writer refuses an applied row outside a snapshot, or over inputs
    // that moved.
    let client = ingest
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let write = "SELECT trace_record_account_trust_evaluation_v2($1,$2,$3,$4,'applied',
        transaction_timestamp(),1,50,2,0,2,1,0,0,0,0,0,FALSE,$5,$6)";
    let digest = format!("sha256:{}", "7".repeat(64));
    client
        .batch_execute(&format!(
            "BEGIN; SELECT set_config('trace_commons.trace_tenant_id','{tenant}',true);"
        ))
        .await
        .unwrap();
    let refused = client
        .query_one(
            write,
            &[
                &tenant,
                &Uuid::new_v4(),
                &account_id,
                &VERSION,
                &digest,
                &Some(0_i64),
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(
        refused.as_db_error().map(|e| e.message()),
        Some("account_trust_evaluation_snapshot_required")
    );
    client.batch_execute("ROLLBACK").await.unwrap();
    client
        .batch_execute(&format!(
            "BEGIN ISOLATION LEVEL REPEATABLE READ; \
             SELECT set_config('trace_commons.trace_tenant_id','{tenant}',true);"
        ))
        .await
        .unwrap();
    let refused = client
        .query_one(
            write,
            &[
                &tenant,
                &Uuid::new_v4(),
                &account_id,
                &VERSION,
                &digest,
                &Some(-1_i64),
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(
        refused.as_db_error().map(|e| e.message()),
        Some("account_trust_evaluation_inputs_changed")
    );
    client.batch_execute("ROLLBACK").await.unwrap();
}
