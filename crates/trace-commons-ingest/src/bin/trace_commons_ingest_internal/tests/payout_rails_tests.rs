// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout rails in ingest: readiness per rail, and the operator routes
//! that halt, resume and release (payout spec section 4, "Modes and the kill
//! switch" and "Fail-closed rules"; section 5, "Operator release").
//!
//! The refusals need no database. The writes are PostgreSQL-backed and
//! self-skip without `TRACE_COMMONS_PG_TEST_DATABASE_URL`; like the rest of
//! the PostgreSQL suite they assume `--test-threads=1`, because a rail's
//! kill switch is deployment-wide. Each write test uses a tenant of its own,
//! never the shared `tenant-a`, so its payout rows cannot block another
//! test's tenant cleanup.

use super::*;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_gate_api::{RailId, RecipientKind};
use trace_commons_server::db::{
    PayoutAuditAction, PayoutLinkInsert, PayoutLinkState, PayoutLinkWrite, PayoutStore,
    SealedRecipient, payout_account_id_hash, payout_link_id_hash,
};

const ACTOR: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

async fn post(
    state: Arc<AppState>,
    uri: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let response = app(state)
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn get(
    state: Arc<AppState>,
    uri: &str,
    bearer: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut request = axum::http::Request::builder().method("GET").uri(uri);
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let response = app(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn readiness(state: Arc<AppState>) -> serde_json::Value {
    let response = super::super::pipeline_runtime::build_pipeline_app(state)
        .oneshot(
            axum::http::Request::get("/v1/pipeline/readiness")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn readiness_reports_each_rail_separately() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path().to_path_buf());
    let body = readiness(state.clone()).await;
    assert_eq!(
        body["payout_rails"],
        serde_json::json!([
            {"rail_id": "trace_credit", "mode": "internal", "halted": false,
             "rate_epoch": null, "qualified": true},
            {"rail_id": "near_ai", "mode": "disabled", "halted": false,
             "rate_epoch": null, "qualified": false},
        ]),
        "{body}"
    );

    // One rail halted: only that entry says so.
    state
        .payout_rails
        .get("near_ai")
        .unwrap()
        .record_halted(true);
    let body = readiness(state).await;
    let rails = body["payout_rails"].as_array().unwrap();
    assert_eq!(rails[0]["rail_id"], "trace_credit");
    assert_eq!(rails[0]["halted"], false);
    assert_eq!(rails[1]["rail_id"], "near_ai");
    assert_eq!(rails[1]["halted"], true);
    assert_eq!(rails[1]["reason"], "payout_halted");
    assert_eq!(rails[1]["qualified"], false);
}

const OPERATOR_TOKEN: &str = "payout-operator-test-token-0123456789abcdef";

fn with_operator_token(mut state: Arc<AppState>, token: &str) -> Arc<AppState> {
    Arc::make_mut(&mut state).payout_control_token_hash =
        Some(super::super::payout_rails::payout_control_token_hash(token));
    state
}

#[tokio::test]
async fn halt_and_resume_refuse_without_the_operator_token() {
    let dir = tempfile::tempdir().unwrap();
    // No operator token configured: every caller is refused, a tenant
    // admin included, and nothing falls back to tenant auth.
    let unconfigured = test_state(dir.path().to_path_buf());
    for action in ["halt", "resume"] {
        let uri = format!("/v1/admin/payout-rails/near_ai/{action}");
        for bearer in [Some("admin-token-a"), Some(OPERATOR_TOKEN)] {
            let (status, body) =
                post(unconfigured.clone(), &uri, bearer, serde_json::json!({})).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{action} {body}");
            assert_eq!(body["error"], "payout_control_unconfigured", "{body}");
        }
    }

    let state = with_operator_token(test_state(dir.path().to_path_buf()), OPERATOR_TOKEN);
    for action in ["halt", "resume"] {
        let uri = format!("/v1/admin/payout-rails/near_ai/{action}");
        let (status, _) = post(state.clone(), &uri, None, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{action} with no bearer");
        // Tenant tokens never reach a rail-wide control, an admin's included.
        for bearer in ["token-a", "admin-token-a", "payout-operator-test-token"] {
            let (status, body) =
                post(state.clone(), &uri, Some(bearer), serde_json::json!({})).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{action} as {bearer} {body}");
            assert_eq!(body["error"], "payout_control_token_required", "{body}");
        }
        // An unknown rail is refused by name, never defaulted.
        let (status, body) = post(
            state.clone(),
            &format!("/v1/admin/payout-rails/test_keyed/{action}"),
            Some(OPERATOR_TOKEN),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{action} {body}");
        assert_eq!(body["error"], "payout_rail_unknown", "{body}");
        // No payout store: refused, nothing set.
        let (status, body) = post(
            state.clone(),
            &uri,
            Some(OPERATOR_TOKEN),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{action} {body}");
        assert_eq!(body["error"], "payout_store_unavailable", "{body}");
    }
    assert_eq!(
        state.payout_rails.get("near_ai").unwrap().status().halted,
        None
    );
}

/// A test state over `backend`, with an admin token for a tenant of its own.
fn payout_state(
    dir: &std::path::Path,
    backend: &Arc<PgBackend>,
    tenant: &str,
    admin_token: &str,
) -> Arc<AppState> {
    let mut state = test_state(dir.to_path_buf());
    let mut tokens = (*state.tokens).clone();
    insert_token(&mut tokens, tenant, admin_token, TokenRole::Admin);
    let inner = Arc::make_mut(&mut state);
    inner.tokens = Arc::new(tokens);
    inner.payout_store = Some(backend.clone() as Arc<dyn PayoutStore>);
    with_operator_token(state, OPERATOR_TOKEN)
}

async fn control_rows(backend: &PgBackend) -> i64 {
    backend
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT count(*) FROM trace_payout_control WHERE rail_id = 'near_ai'",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn halt_and_resume_require_the_operator_token_and_write_audit_rows() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("payout-admin-{}", Uuid::new_v4());
    let token = format!("payout-admin-token-{}", Uuid::new_v4());
    let state = payout_state(dir.path(), &backend, &tenant, &token);
    let rail = RailId::near_ai();
    let before = control_rows(&backend).await;

    // A contributor and this tenant's own admin are both refused: the kill
    // switch is rail-wide, so no tenant token reaches it.
    for bearer in ["token-a", token.as_str()] {
        let (status, _) = post(
            state.clone(),
            "/v1/admin/payout-rails/near_ai/halt",
            Some(bearer),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert_eq!(
        control_rows(&backend).await,
        before,
        "a refusal writes nothing"
    );

    let (status, body) = post(
        state.clone(),
        "/v1/admin/payout-rails/near_ai/halt",
        Some(OPERATOR_TOKEN),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({"rail_id": "near_ai", "halted": true})
    );
    let control = backend.payout_control(&rail).await.unwrap();
    assert!(control.halted);
    assert_eq!(control.label.as_deref(), Some("operator_halt"));
    assert_eq!(control_rows(&backend).await, before + 1);
    assert_eq!(
        readiness(state.clone()).await["payout_rails"][1]["halted"],
        true
    );
    // The control row names the operator by hash only.
    let actor: String = backend
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT actor_ref_hash FROM trace_payout_control
              WHERE rail_id = 'near_ai' ORDER BY seq DESC LIMIT 1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(actor.starts_with("sha256:") && actor.len() == 71, "{actor}");
    assert!(!actor.contains(OPERATOR_TOKEN));

    let (status, body) = post(
        state.clone(),
        "/v1/admin/payout-rails/near_ai/resume",
        Some(OPERATOR_TOKEN),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({"rail_id": "near_ai", "halted": false})
    );
    let control = backend.payout_control(&rail).await.unwrap();
    assert!(!control.halted);
    assert_eq!(control.label.as_deref(), Some("operator_resume"));
    assert_eq!(control_rows(&backend).await, before + 2);
    assert_eq!(readiness(state).await["payout_rails"][1]["halted"], false);
}

#[tokio::test]
async fn release_moves_an_active_link_to_closing_with_an_audit_row() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("payout-release-{}", Uuid::new_v4());
    let token = format!("payout-release-token-{}", Uuid::new_v4());
    let state = payout_state(dir.path(), &backend, &tenant, &token);
    let account = Uuid::new_v4();
    {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let link_id = Uuid::new_v4();
    let digest = |value: &str| sha256_prefixed(value);
    assert_eq!(
        backend
            .insert_payout_link(
                &tenant,
                &PayoutLinkWrite {
                    account_id: account,
                    rail_id: RailId::near_ai(),
                    link_id,
                    recipient_kind: RecipientKind::near_ai_org(),
                    recipient: SealedRecipient {
                        sealed: serde_json::json!({"v": 1, "ciphertext": digest("ct")}),
                        recipient_index: digest("index"),
                        recipient_key_ref: "key-test-1".into(),
                        index_pepper_ref: "pepper-test-1".into(),
                        subject_index: digest("subject"),
                        recipient_ref: None,
                    },
                    program: format!("tc1-{}", &digest(&link_id.to_string())[7..39]),
                    state: PayoutLinkState::Active,
                    actor_ref_hash: ACTOR.into(),
                },
            )
            .await
            .unwrap(),
        PayoutLinkInsert::Inserted
    );
    let hash = payout_link_id_hash(link_id);
    let uri = |hash: &str| format!("/v1/admin/payout-rails/near_ai/links/{hash}/release");

    // Refusals first: each changes nothing.
    let (status, _) = post(
        state.clone(),
        &uri(&hash),
        Some("token-a"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for (bad_hash, want) in [
        ("not-a-hash", StatusCode::BAD_REQUEST),
        (&format!("sha256:{}", "0".repeat(64)), StatusCode::NOT_FOUND),
    ] {
        let (status, body) = post(
            state.clone(),
            &uri(bad_hash),
            Some(&token),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, want, "{bad_hash} {body}");
    }
    let (status, body) = post(
        state.clone(),
        &uri(&hash),
        Some(&token),
        serde_json::json!({"reason": "Lost Access!"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        backend
            .payout_links(&tenant, &RailId::near_ai(), &[PayoutLinkState::Active])
            .await
            .unwrap()
            .len(),
        1
    );

    let (status, body) = post(
        state.clone(),
        &uri(&hash),
        Some(&token),
        serde_json::json!({"reason": "contributor_lost_org_access"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "closing", "{body}");
    assert_eq!(body["link_id_hash"], hash, "{body}");
    assert!(!body.to_string().contains(&link_id.to_string()), "{body}");
    let closing = backend
        .payout_links(&tenant, &RailId::near_ai(), &[PayoutLinkState::Closing])
        .await
        .unwrap();
    assert_eq!(closing.len(), 1);
    assert_eq!(closing[0].link_id, link_id);
    assert!(
        backend
            .payout_audit_actions(&tenant, &RailId::near_ai())
            .await
            .unwrap()
            .contains(&PayoutAuditAction::LinkReleased)
    );
    // A second release finds no active link.
    let (status, _) = post(state, &uri(&hash), Some(&token), serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hold_routes_refuse_every_tenant_token() {
    let dir = tempfile::tempdir().unwrap();
    let list = "/v1/admin/payout-rails/near_ai/tenants/tenant-a/holds";
    let clear = "/v1/admin/payout-rails/near_ai/tenants/tenant-a/holds/clear";
    let body = serde_json::json!({
        "account_id_hash": format!("sha256:{}", "ab".repeat(32)),
        "label": "payout_recovery_unmatched",
    });
    let unconfigured = test_state(dir.path().to_path_buf());
    let (status, _) = get(unconfigured.clone(), list, Some("admin-token-a")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (status, _) = post(unconfigured, clear, Some("admin-token-a"), body.clone()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let state = with_operator_token(test_state(dir.path().to_path_buf()), OPERATOR_TOKEN);
    for bearer in ["token-a", "admin-token-a"] {
        let (status, _) = get(state.clone(), list, Some(bearer)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bearer}");
        let (status, _) = post(state.clone(), clear, Some(bearer), body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bearer}");
    }
    let (status, _) = get(state.clone(), list, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Shapes are checked before the store is reached.
    let (status, body) = get(
        state.clone(),
        "/v1/admin/payout-rails/near_ai/tenants/a%20b/holds",
        Some(OPERATOR_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = get(
        state,
        "/v1/admin/payout-rails/test_keyed/tenants/tenant-a/holds",
        Some(OPERATOR_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "payout_rail_unknown");
}

#[tokio::test]
async fn an_operator_lists_and_clears_a_hold_with_an_audit_row() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tenant = format!("payout-holds-{}", Uuid::new_v4());
    let token = format!("payout-holds-token-{}", Uuid::new_v4());
    let state = payout_state(dir.path(), &backend, &tenant, &token);
    let account = Uuid::new_v4();
    {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let rail = RailId::near_ai();
    for label in ["payout_recovery_unmatched", "payout_program_total_mismatch"] {
        assert!(
            backend
                .open_payout_hold(&tenant, account, &rail, label, ACTOR)
                .await
                .unwrap()
        );
    }
    let list = format!("/v1/admin/payout-rails/near_ai/tenants/{tenant}/holds");
    let clear = format!("{list}/clear");
    let hash = payout_account_id_hash(account);

    // The tenant's own admin is refused.
    let (status, _) = get(state.clone(), &list, Some(&token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = get(state.clone(), &list, Some(OPERATOR_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({
            "rail_id": "near_ai",
            "holds": [
                {"account_id_hash": hash, "label": "payout_program_total_mismatch"},
                {"account_id_hash": hash, "label": "payout_recovery_unmatched"},
            ],
        })
    );
    assert!(!body.to_string().contains(&account.to_string()));

    for (request, want) in [
        (
            serde_json::json!({"account_id_hash": "nope", "label": "payout_recovery_unmatched"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"account_id_hash": hash, "label": "Not A Label"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({
                "account_id_hash": format!("sha256:{}", "0".repeat(64)),
                "label": "payout_recovery_unmatched",
            }),
            StatusCode::NOT_FOUND,
        ),
        (
            serde_json::json!({"account_id_hash": hash, "label": "payout_over_ceiling"}),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (status, body) = post(state.clone(), &clear, Some(OPERATOR_TOKEN), request).await;
        assert_eq!(status, want, "{body}");
    }
    let request =
        serde_json::json!({"account_id_hash": hash, "label": "payout_recovery_unmatched"});
    let (status, _) = post(state.clone(), &clear, Some(&token), request.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a tenant admin cannot clear");
    let (status, body) = post(state.clone(), &clear, Some(OPERATOR_TOKEN), request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({
            "rail_id": "near_ai",
            "account_id_hash": hash,
            "label": "payout_recovery_unmatched",
            "cleared": true,
        })
    );
    // Only that hold cleared; the other stays until it is cleared too.
    assert_eq!(
        backend
            .open_payout_hold_labels(&tenant, account, &rail)
            .await
            .unwrap(),
        vec!["payout_program_total_mismatch".to_string()]
    );
    assert!(
        backend
            .payout_audit_actions(&tenant, &rail)
            .await
            .unwrap()
            .contains(&PayoutAuditAction::HoldCleared)
    );
    // The audit row names the operator by hash, never the token.
    let actor: String = {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .unwrap();
        let row = tx
            .query_one(
                "SELECT actor_ref_hash FROM trace_payout_audit
                  WHERE tenant_id = $1 AND action = 'hold_cleared'",
                &[&tenant],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        row.get(0)
    };
    assert!(actor.starts_with("sha256:") && actor.len() == 71, "{actor}");
    assert_ne!(actor, ACTOR);
    assert!(!actor.contains(OPERATOR_TOKEN));
    // A second clear finds nothing open.
    let (status, _) = post(state, &clear, Some(OPERATOR_TOKEN), request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
