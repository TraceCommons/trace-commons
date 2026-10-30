// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral operator routes and boot (referral spec sections 6 and 7).
//!
//! The refusals need no database. The flows are PostgreSQL-backed and
//! self-skip without `TRACE_COMMONS_PG_TEST_DATABASE_URL`; each uses tenants
//! of its own. The list and report are deployment-wide, so their assertions
//! read this test's own tenant out of them.

use super::*;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_server::db::{PayoutStore, payout_account_id_hash};
use trace_commons_server::payout_rate::PayoutMode;
use trace_commons_server::payout_worker::PayoutRails;
use trace_commons_server::referral_config::ReferralBonusSettings;

const OPERATOR_TOKEN: &str = "referral-operator-test-token-0123456789abcdef";

async fn call(
    state: Arc<AppState>,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut request = axum::http::Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let body = match body {
        Some(body) => Body::from(body.to_string()),
        None => Body::empty(),
    };
    let response = app(state)
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn with_operator_token(mut state: Arc<AppState>) -> Arc<AppState> {
    Arc::make_mut(&mut state).payout_control_token_hash = Some(
        super::super::payout_rails::payout_control_token_hash(OPERATOR_TOKEN),
    );
    state
}

fn dry_run() -> ReferralBonusSettings {
    ReferralBonusSettings {
        mode: PayoutMode::DryRun,
        ..ReferralBonusSettings::default()
    }
}

fn hash(n: u128) -> String {
    format!("sha256:{n:064x}")
}

/// Every route with a well-formed body.
fn routes() -> Vec<(&'static str, &'static str, Option<serde_json::Value>)> {
    vec![
        ("GET", "/v1/admin/referrals/suspended", None),
        (
            "POST",
            "/v1/admin/referrals/codes/clear",
            Some(serde_json::json!({"tenant_id": "t", "code_index": hash(1)})),
        ),
        (
            "POST",
            "/v1/admin/referrals/codes/void",
            Some(serde_json::json!({"tenant_id": "t", "code_index": hash(1)})),
        ),
        (
            "POST",
            "/v1/admin/referrals/void",
            Some(serde_json::json!({"tenant_id": "t", "account_id_hash": hash(2)})),
        ),
        (
            "POST",
            "/v1/admin/referrals/dry-run-report",
            Some(serde_json::json!({})),
        ),
    ]
}

#[tokio::test]
async fn every_referral_route_takes_the_operator_token_only() {
    let dir = tempfile::tempdir().unwrap();
    let unconfigured = test_state(dir.path().to_path_buf());
    let state = with_operator_token(test_state(dir.path().to_path_buf()));
    for (method, uri, body) in routes() {
        // No token configured: every caller is refused.
        for bearer in [Some("admin-token-a"), Some(OPERATOR_TOKEN)] {
            let (status, reply) =
                call(unconfigured.clone(), method, uri, bearer, body.clone()).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri} {reply}");
            assert_eq!(reply["error"], "payout_control_unconfigured", "{reply}");
        }
        let (status, _) = call(state.clone(), method, uri, None, body.clone()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        // Tenant tokens, an admin's included, never reach these routes.
        for bearer in ["token-a", "admin-token-a"] {
            let (status, reply) =
                call(state.clone(), method, uri, Some(bearer), body.clone()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri} as {bearer} {reply}");
            assert_eq!(reply["error"], "payout_control_token_required");
        }
        // The operator, with referral mode disabled: nothing to reach.
        let (status, reply) = call(
            state.clone(),
            method,
            uri,
            Some(OPERATOR_TOKEN),
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri} {reply}");
        assert_eq!(reply["error"], "referral_store_unavailable", "{reply}");
    }
}

#[test]
fn boot_attaches_nothing_when_disabled_and_refuses_dry_run_without_a_database() {
    let rails = super::super::referral_admin::with_referral_bonus(
        PayoutRails::disabled(),
        None,
        ReferralBonusSettings::default(),
    )
    .unwrap();
    assert!(rails.referral_bonus().is_none());
    assert_eq!(rails.readiness().len(), 2, "no referral readiness entry");

    let error =
        super::super::referral_admin::with_referral_bonus(PayoutRails::disabled(), None, dry_run())
            .unwrap_err();
    assert!(
        error.to_string().contains("referral_store_unavailable"),
        "{error}"
    );
}

/// A state whose rails carry the bonus work over `backend`.
fn referral_state(dir: &std::path::Path, backend: &Arc<PgBackend>) -> Arc<AppState> {
    let mut state = test_state(dir.to_path_buf());
    let rails = super::super::referral_admin::with_referral_bonus(
        PayoutRails::disabled(),
        Some(backend.clone()),
        dry_run(),
    )
    .unwrap();
    let inner = Arc::make_mut(&mut state);
    inner.payout_rails = Arc::new(rails);
    inner.payout_store = Some(backend.clone() as Arc<dyn PayoutStore>);
    with_operator_token(state)
}

async fn owner_exec(
    backend: &PgBackend,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.execute(sql, params).await.unwrap();
    tx.commit().await.unwrap();
}

struct Seeded {
    referrer_tenant: String,
    referrer: Uuid,
    code_index: String,
    referee_tenant: String,
    referee: Uuid,
    referee_ref: String,
}

/// A referrer with a suspended code, one referee who applied it, and a
/// bonus of 100 units owed to the referrer for that referee.
async fn seed(backend: &PgBackend) -> Seeded {
    let referrer_tenant = format!("referral-admin-{}", Uuid::new_v4().simple());
    let referee_tenant = format!("referral-admin-{}", Uuid::new_v4().simple());
    let referrer = Uuid::new_v4();
    let referee = Uuid::new_v4();
    for (tenant, account) in [(&referrer_tenant, referrer), (&referee_tenant, referee)] {
        owner_exec(
            backend,
            tenant,
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[tenant],
        )
        .await;
        owner_exec(
            backend,
            tenant,
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[tenant, &account],
        )
        .await;
    }
    owner_exec(
        backend,
        &referrer_tenant,
        "INSERT INTO trace_payout_rate_epochs (rail_id, version, numerator, denominator)
         VALUES ('near_ai', 'v1', 1, 1) ON CONFLICT DO NOTHING",
        &[],
    )
    .await;
    let code_index = hash(Uuid::new_v4().as_u128());
    let referee_ref = hash(Uuid::new_v4().as_u128());
    owner_exec(
        backend,
        &referrer_tenant,
        "INSERT INTO trace_referral_codes
             (tenant_id, code_id, account_id, code_index, sealed_code, code_key_ref,
              index_pepper_ref, suspended_at, suspension_labels)
         VALUES ($1, $2, $3, $4, '{\"v\":1}'::jsonb, 'key-test-1', 'pepper-test-1',
                 clock_timestamp(), ARRAY['referral_burst'])",
        &[&referrer_tenant, &Uuid::new_v4(), &referrer, &code_index],
    )
    .await;
    owner_exec(
        backend,
        &referee_tenant,
        "INSERT INTO trace_referral_attributions
             (tenant_id, account_id, code_index, referrer_tenant_id, referrer_account_id,
              referee_ref_hash, referee_account_created_at, applied_via, earn_until)
         SELECT $1, $2, $3, $4, $5, $6, a.created_at, 'later', a.created_at + interval '92 days'
           FROM trace_accounts a WHERE a.tenant_id = $1 AND a.account_id = $2",
        &[
            &referee_tenant,
            &referee,
            &code_index,
            &referrer_tenant,
            &referrer,
            &referee_ref,
        ],
    )
    .await;
    owner_exec(
        backend,
        &referrer_tenant,
        "INSERT INTO trace_referral_referees
             (tenant_id, referee_ref_hash, account_id, code_index,
              referee_account_created_at, applied_at, signal_labels)
         VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp(),
                 ARRAY['referral_refused_share'])",
        &[&referrer_tenant, &referee_ref, &referrer, &code_index],
    )
    .await;
    owner_exec(
        backend,
        &referrer_tenant,
        "INSERT INTO trace_referral_bonus_accruals
             (tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
              source_accrual_id, bps, base_units, bonus_units, rate_version, instrument_id)
         VALUES ($1, $2, 'near_ai', $3, $4, 'bonus', $5, 1000, 1000, 100, 'v1', 'near_ai_usd')",
        &[
            &referrer_tenant,
            &Uuid::new_v4(),
            &referrer,
            &referee_ref,
            &Uuid::new_v4(),
        ],
    )
    .await;
    Seeded {
        referrer_tenant,
        referrer,
        code_index,
        referee_tenant,
        referee,
        referee_ref,
    }
}

#[tokio::test]
async fn the_operator_lists_clears_and_reports_suspended_codes() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let state = referral_state(dir.path(), &backend);
    let seeded = seed(&backend).await;
    backend
        .open_payout_hold(
            &seeded.referrer_tenant,
            seeded.referrer,
            &trace_commons_gate_api::RailId::near_ai(),
            "referral_code_suspended",
            &hash(3),
        )
        .await
        .unwrap();

    let (status, body) = call(
        state.clone(),
        "GET",
        "/v1/admin/referrals/suspended",
        Some(OPERATOR_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mine: Vec<&serde_json::Value> = body["codes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|code| code["tenant_id"] == seeded.referrer_tenant.as_str())
        .collect();
    assert_eq!(mine.len(), 1, "{body}");
    assert_eq!(mine[0]["code_index"], seeded.code_index.as_str());
    assert_eq!(
        mine[0]["suspension_labels"],
        serde_json::json!(["referral_burst"])
    );
    assert_eq!(mine[0]["referees"], 1);
    assert_eq!(mine[0]["held_units"], "100");
    // Hash-only: no account id in the list.
    assert!(!body.to_string().contains(&seeded.referrer.to_string()));

    // Shape refusals before any write.
    for (request, label) in [
        (
            serde_json::json!({"tenant_id": "bad tenant", "code_index": seeded.code_index}),
            "referral_tenant_id_invalid",
        ),
        (
            serde_json::json!({"tenant_id": seeded.referrer_tenant, "code_index": "TC-ABCDE"}),
            "referral_code_index_invalid",
        ),
        (
            serde_json::json!({"tenant_id": seeded.referrer_tenant,
                               "code_index": seeded.code_index,
                               "exempt_labels": ["payout_over_ceiling"]}),
            "referral_exempt_label_invalid",
        ),
    ] {
        let (status, body) = call(
            state.clone(),
            "POST",
            "/v1/admin/referrals/codes/clear",
            Some(OPERATOR_TOKEN),
            Some(request),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], label);
    }

    let (status, body) = call(
        state.clone(),
        "POST",
        "/v1/admin/referrals/codes/clear",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({
            "tenant_id": seeded.referrer_tenant,
            "code_index": seeded.code_index,
            "exempt_labels": ["referral_burst"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["cleared"], true);
    // The exemption is recorded, and the code-suspended hold lifted.
    let codes = trace_commons_server::db::ReferralBonusStore::referral_codes(
        backend.as_ref(),
        &seeded.referrer_tenant,
    )
    .await
    .unwrap();
    assert_eq!(codes.len(), 1);
    assert!(codes[0].suspended_at.is_none());
    assert!(codes[0].exempt_labels.contains("referral_burst"));
    let holds = backend
        .open_payout_hold_labels(
            &seeded.referrer_tenant,
            seeded.referrer,
            &trace_commons_gate_api::RailId::near_ai(),
        )
        .await
        .unwrap();
    assert!(holds.is_empty(), "{holds:?}");
    // A second clear finds nothing suspended.
    let (status, body) = call(
        state.clone(),
        "POST",
        "/v1/admin/referrals/codes/clear",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({
            "tenant_id": seeded.referrer_tenant,
            "code_index": seeded.code_index,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "referral_code_not_suspended");

    let (status, body) = call(
        state.clone(),
        "POST",
        "/v1/admin/referrals/dry-run-report",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rail_id"], "near_ai");
    assert_eq!(body["mode"], "dry_run");
    assert!(body["tenants"].as_u64().unwrap() >= 1);
    assert!(
        body["codes_by_state"]["live"].as_u64().unwrap() >= 1,
        "{body}"
    );
    assert!(
        body["referees_by_signal"]["referral_refused_share"]
            .as_u64()
            .unwrap()
            >= 1
    );
    let owed: u128 = body["owed_units"].as_str().unwrap().parse().unwrap();
    let sendable: u128 = body["sendable_units"].as_str().unwrap().parse().unwrap();
    assert!(owed >= 100 && sendable >= 100, "{body}");
}

#[tokio::test]
async fn the_operator_voids_one_referral_in_both_tenants() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let state = referral_state(dir.path(), &backend);
    let seeded = seed(&backend).await;

    let (status, body) = call(
        state.clone(),
        "POST",
        "/v1/admin/referrals/void",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({
            "tenant_id": seeded.referee_tenant,
            "account_id_hash": payout_account_id_hash(Uuid::new_v4()),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "referral_not_found");

    let (status, body) = call(
        state.clone(),
        "POST",
        "/v1/admin/referrals/void",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({
            "tenant_id": seeded.referee_tenant,
            "account_id_hash": payout_account_id_hash(seeded.referee),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["referees_voided"], 1);
    assert_eq!(body["taken_back_units"], "100");

    let client = backend.trace_pool_for_test().get().await.unwrap();
    let voided: bool = client
        .query_one(
            "SELECT voided_at IS NOT NULL FROM trace_referral_attributions
              WHERE tenant_id = $1 AND account_id = $2",
            &[&seeded.referee_tenant, &seeded.referee],
        )
        .await
        .unwrap()
        .get(0);
    assert!(voided);
    let row = client
        .query_one(
            "SELECT bonus_units::TEXT FROM trace_referral_bonus_accruals
              WHERE tenant_id = $1 AND referee_ref_hash = $2 AND kind = 'void'",
            &[&seeded.referrer_tenant, &seeded.referee_ref],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "-100");

    // Voiding the code afterwards finds no live referee left to take back.
    let (status, body) = call(
        state.clone(),
        "POST",
        "/v1/admin/referrals/codes/void",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({
            "tenant_id": seeded.referrer_tenant,
            "code_index": seeded.code_index,
            "reason": "operator_fraud",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["referees_voided"], 0);
    assert_eq!(body["taken_back_units"], "0");
    let (status, body) = call(
        state,
        "POST",
        "/v1/admin/referrals/codes/void",
        Some(OPERATOR_TOKEN),
        Some(serde_json::json!({
            "tenant_id": seeded.referrer_tenant,
            "code_index": seeded.code_index,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "referral_code_not_found");
}

/// A tripped referral stream (spec section 7) runs as `dry_run` until the
/// operator resumes it through the payout rail control route, with the
/// payout control token and an audit row, as a rail's halt is lifted.
#[tokio::test]
async fn the_operator_resumes_a_tripped_referral_stream() {
    use trace_commons_server::referral_bonus::{
        NEAR_AI_REFERRAL_CONTROL_RAIL, near_ai_referral_control_rail,
    };
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let state = referral_state(dir.path(), &backend);
    let seeded = seed(&backend).await;
    let control = near_ai_referral_control_rail();
    let uri = format!("/v1/admin/payout-rails/{NEAR_AI_REFERRAL_CONTROL_RAIL}/resume");
    let referral_readiness = |state: &Arc<AppState>| {
        state
            .payout_rails
            .readiness()
            .into_iter()
            .find(|entry| entry.rail_id == NEAR_AI_REFERRAL_CONTROL_RAIL)
            .expect("the referral stream's readiness entry")
    };

    // The pass trips it, and readiness reports the trip.
    backend
        .set_payout_control(
            &control,
            true,
            Some(trace_commons_server::referral_abuse::REFERRAL_TRIPPED),
            &hash(9),
        )
        .await
        .unwrap();
    let hook = state.payout_rails.referral_bonus().unwrap().clone();
    hook.run_tenant(&seeded.referrer_tenant, hook.rail_id())
        .await;
    assert!(referral_readiness(&state).halted);

    // Tenant tokens never reach it.
    for bearer in [None, Some("admin-token-a")] {
        let (status, _) = call(state.clone(), "POST", &uri, bearer, None).await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
            "{status}"
        );
    }
    assert!(backend.payout_control(&control).await.unwrap().halted);

    let (status, body) = call(state.clone(), "POST", &uri, Some(OPERATOR_TOKEN), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rail_id"], NEAR_AI_REFERRAL_CONTROL_RAIL);
    assert_eq!(body["halted"], false);
    let now = backend.payout_control(&control).await.unwrap();
    assert!(!now.halted);
    assert!(!referral_readiness(&state).halted, "readiness at once");

    // The control row carries the operator's hash and a label, nothing else.
    let client = backend.trace_pool_for_test().get().await.unwrap();
    let row = client
        .query_one(
            "SELECT label, actor_ref_hash FROM trace_payout_control
              WHERE rail_id = $1 ORDER BY seq DESC LIMIT 1",
            &[&control.as_str()],
        )
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, Option<String>>(0).as_deref(),
        Some("operator_resume")
    );
    assert!(row.get::<_, String>(1).starts_with("sha256:"));

    let (status, body) = call(
        state.clone(),
        "POST",
        &format!("/v1/admin/payout-rails/{NEAR_AI_REFERRAL_CONTROL_RAIL}/halt"),
        Some(OPERATOR_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(backend.payout_control(&control).await.unwrap().halted);
    let (status, _) = call(state, "POST", &uri, Some(OPERATOR_TOKEN), None).await;
    assert_eq!(status, StatusCode::OK);
}

/// Without the referral stream attached there is nothing to resume.
#[tokio::test]
async fn the_referral_control_is_unknown_without_the_referral_stream() {
    let dir = tempfile::tempdir().unwrap();
    let state = with_operator_token(test_state(dir.path().to_path_buf()));
    let (status, body) = call(
        state,
        "POST",
        "/v1/admin/payout-rails/near_ai.referral/resume",
        Some(OPERATOR_TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "payout_rail_unknown");
}
