// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;
use trace_commons_protocol::activity_missions::ActivityPolicy;

fn configured(state: &mut Arc<AppState>) {
    let today = Utc::now().date_naive();
    let raw = serde_json::json!({"schema_version":1,"policy_id":"test-policy","starts_on":today,"ends_before":today+Duration::days(30),"qualification":"accepted","missions":[{"id":"one","title":"One contribution","required_contributions":1}],"daily":{"kind":"fixed","mission_id":"one"},"levels":null,"badges":null});
    Arc::get_mut(state).unwrap().activity_missions_policy = Some(Arc::new(
        ActivityPolicy::parse(&serde_json::to_vec(&raw).unwrap()).unwrap(),
    ));
}
async fn read(state: Arc<AppState>, path: &str) -> axum::response::Response {
    app(state)
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}
#[tokio::test]
async fn activity_missions_public_catalogue_preserves_unknown_credit_and_rejects_profile() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state(temp.path().into());
    let response = read(state.clone(), "/v1/activity-missions").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let unconfigured = body(response).await;
    assert_eq!(unconfigured["state"], "unconfigured");
    assert!(unconfigured["policy"].is_null());
    assert!(unconfigured["credit_points_pending"].is_null());
    configured(&mut state);
    let live = body(read(state.clone(), "/v1/activity-missions").await).await;
    assert_eq!(live["state"], "configured");
    assert_eq!(live["kind"], "trace_activity");
    assert_eq!(live["rewards_enabled"], false);
    assert!(live["credit_points_pending"].is_null());
    for query in [
        "profile=codex",
        "mission_id=one",
        "complete=true",
        "tenant_id=other",
    ] {
        let response = read(state.clone(), &format!("/v1/activity-missions?{query}")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    assert_eq!(
        read(state.clone(), "/v1/account/activity-missions/status")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(read(state, "/v1/source").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn activity_missions_status_never_substitutes_zero_for_missing_policy_or_database() {
    use trace_commons_server::account_session::{AccountId, AccountPrincipalSet};
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state(temp.path().into());
    let ctx = AccountCtx {
        account_id: AccountId::from_uuid(Uuid::new_v4()),
        principal_set: AccountPrincipalSet::from_iter_for_test_only(vec!["owned".into()]),
        auth_method: AccountAuthMethod::NativeToken,
        tenant_id: "tenant-a".into(),
        actor_ref: "test".into(),
        auth_credential_id: None,
        client_kind: "native".into(),
        session_token_hash: None,
    };
    let response = activity_missions::status(
        State(state.clone()),
        Extension(ctx.clone()),
        axum::extract::RawQuery(None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body(response).await["error"],
        "activity_missions_unconfigured"
    );
    configured(&mut state);
    let response =
        activity_missions::status(State(state), Extension(ctx), axum::extract::RawQuery(None))
            .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let result = body(response).await;
    assert!(result.get("monthly_contributions").is_none());
    assert!(result.get("credit_points_pending").is_none());
}

#[tokio::test]
async fn activity_missions_authenticated_http_progress_is_read_from_account_records() {
    use trace_commons_server::config::DatabaseConfig;
    use trace_commons_server::db::{NewSession, RedeemAudit};
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return;
    };
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test_"));
    let backend = Arc::new(
        PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
            .await
            .unwrap(),
    );
    backend.run_migrations().await.unwrap();
    let tenant = format!("activity-http-{}", Uuid::new_v4());
    let principal = "mission-owned";
    let account = backend
        .create_or_reuse_account(&tenant, principal)
        .await
        .unwrap();
    let secret = generate_session_secret();
    backend
        .issue_native_session(
            &tenant,
            account,
            NewSession {
                token_hash: &hash_secret(&secret),
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(1),
            },
            RedeemAudit {
                action: "test_session".into(),
                outcome: "success".into(),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
    // Force the existing account middleware's rotation boundary, so this
    // endpoint is shown to preserve native session headers too.
    backend.trace_pool_for_test().get().await.unwrap().execute(
        "UPDATE trace_sessions SET token_issued_at=now()-interval '13 hours' WHERE tenant_id=$1 AND token_hash=$2",
        &[&tenant,&hash_secret(&secret)],
    ).await.unwrap();
    let submission = insert_account_test_submission_with_status(
        backend.as_ref(),
        &tenant,
        principal,
        StorageTraceCorpusStatus::Accepted,
    )
    .await;
    insert_account_test_submission_with_status(
        backend.as_ref(),
        &tenant,
        "foreign",
        StorageTraceCorpusStatus::Accepted,
    )
    .await;
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state_with_db(temp.path().into(), backend.clone());
    configured(&mut state);
    let bearer = format!("Bearer {}", native_token_value(&tenant, &secret));
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/v1/account/activity-missions/status")
                .header("authorization", &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let rotated = response
        .headers()
        .get(trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
        .expect("existing account middleware retains rotated token header")
        .to_str()
        .unwrap()
        .to_owned();
    let bearer = format!("Bearer {rotated}");
    let result = body(response).await;
    assert_eq!(result["monthly_contributions"], 1);
    assert_eq!(result["daily"]["complete"], true);
    assert_eq!(result["rewards_enabled"], false);
    assert!(result["credit_points_pending"].is_null());
    // A withdrawal marker invalidates the view even before status cleanup.
    let client = backend.trace_pool_for_test().get().await.unwrap();
    client.execute("INSERT INTO trace_withdrawals(tenant_id,submission_id,withdrawn_at,prior_status,distribution_reach) VALUES ($1,$2,now(),'accepted','not_distributed')",&[&tenant,&submission]).await.unwrap();
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/v1/account/activity-missions/status")
                .header("authorization", &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result = body(response).await;
    assert_eq!(result["monthly_contributions"], 0);
    assert_eq!(result["daily"]["complete"], false);
    assert!(result["credit_points_pending"].is_null());
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/v1/account/activity-missions/status?complete=true")
                .header("authorization", &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let row = client
        .query_one(
            "SELECT count(*) FROM trace_credit_ledger WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, i64>(0),
        0,
        "mission reads never manufacture ledger awards"
    );
}
