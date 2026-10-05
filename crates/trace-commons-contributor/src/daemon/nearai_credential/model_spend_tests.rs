//! Synthetic deployed-shape provider spend fixtures; all transport is loopback.
use super::{cache, read_model_spend_with, read_with};
use crate::daemon::ipc::DaemonShared;
use crate::daemon::nearai_credential::api::CloudApi;
use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};
use axum::{
    Json, Router,
    routing::{get, post},
};
use chrono::Utc;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn shared() -> (tempfile::TempDir, DaemonShared) {
    let (dir, store) = crate::config::tests_support::temp_store();
    let shared = DaemonShared::load(store).unwrap();
    {
        let mut settings = shared.settings.lock().unwrap();
        settings.near_ai_session = Some(NearAiSession {
            refresh_token: "rt-first".into(),
            refresh_token_expires_at: None,
            stored_at: Utc::now(),
            user_agent: "Test".into(),
        });
        settings.near_ai_inference = Some(NearAiInferenceCredential {
            key: "sk-secret".into(),
            key_id: "key-secret".into(),
            key_prefix: "sk-".into(),
            organization_id: "org-selected".into(),
            workspace_id: "workspace-secret".into(),
            minted_at: Utc::now(),
        });
        settings.save_for_test(&shared.store).unwrap();
    }
    (dir, shared)
}

fn usage() -> serde_json::Value {
    serde_json::json!({"period":"day","start_date":(Utc::now()-chrono::Duration::hours(24)).to_rfc3339(),"data":[{"model":"example-model","total_cost":1501,"request_count":3,"input_tokens":4,"output_tokens":5,"total_tokens":9,"total_cost_display":"SECRET prose"}]})
}

async fn serve(
    body: serde_json::Value,
) -> (CloudApi, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let count = Arc::new(AtomicUsize::new(0));
    let record = count.clone();
    let router = Router::new()
        .route("/v1/users/me/access-tokens", post(move |headers: axum::http::HeaderMap| { let count=record.clone(); async move {
            assert_eq!(headers["authorization"], "Bearer rt-first"); count.fetch_add(1,Ordering::SeqCst);
            Json(serde_json::json!({"access_token":"jwt-first","refresh_token":"rt-second","refresh_token_expiration":"2027-01-01T00:00:00Z"}))
        }}))
        .route("/v1/organizations/org-selected/usage/by-model", get(move |headers: axum::http::HeaderMap, uri: axum::http::Uri| {let body=body.clone();async move {
            assert_eq!(headers["authorization"], "Bearer jwt-first"); assert_eq!(uri.query(), Some("period=day")); Json(body)
        }}))
        .route("/v1/organizations/org-selected/usage/balance", get(|| async {Json(serde_json::json!({"remaining":1000,"spend_limit":2000,"total_spent":1000,"total_requests":3,"total_tokens":9}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (CloudApi::for_test(&origin).unwrap(), count, task)
}

#[tokio::test]
async fn model_spend_reads_provider_nanos_with_account_scope_and_explicit_rounding() {
    let (_dir, s) = shared();
    let (api, count, task) = serve(usage()).await;
    let result = read_model_spend_with(&s, &api).await;
    assert_eq!(result["known"], true);
    assert_eq!(result["scope"], "near_ai_organization");
    assert_eq!(result["source"], "near_ai_usage_by_model");
    assert_eq!(result["window_hours"], 24);
    assert_eq!(result["scale"], 9);
    assert_eq!(result["currency"], "USD");
    assert!(result["since"].is_string());
    assert!(result["observed_at"].is_string());
    assert_eq!(result["models"][0]["billed_nanos"], 1501);
    assert_eq!(result["models"][0]["billed_micros"], 2);
    assert_eq!(result["models"][0]["rounding"], "nearest_micro_half_up");
    assert_eq!(result["models"][0]["calls"], 3);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(!result.to_string().contains("secret"));
    assert!(!result.to_string().contains("SECRET"));
    task.abort();
}

#[tokio::test]
async fn model_spend_and_balance_share_rotation_without_double_refresh() {
    let (_dir, s) = shared();
    let (api, count, task) = serve(usage()).await;
    let (spend, balance) = tokio::join!(read_model_spend_with(&s, &api), read_with(&s, &api));
    assert_eq!(spend["known"], true);
    assert_eq!(balance.state(), "known");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        s.settings
            .lock()
            .unwrap()
            .near_ai_session
            .as_ref()
            .unwrap()
            .refresh_token,
        "rt-second"
    );
    assert!(cache().lock().await.contains_key(s.store.dir()));
    task.abort();
}

#[tokio::test]
async fn model_spend_invalid_source_counts_costs_and_timestamps_remain_unknown() {
    for (field, value) in [
        ("total_cost", serde_json::json!(-1)),
        ("total_cost", serde_json::json!(1e100)),
        ("request_count", serde_json::json!(-1)),
        ("total_cost", serde_json::Value::Null),
    ] {
        let (_dir, s) = shared();
        let mut body = usage();
        body["data"][0][field] = value;
        let (api, _, task) = serve(body).await;
        let result = read_model_spend_with(&s, &api).await;
        assert_eq!(result["known"], false, "{field}");
        assert_eq!(result["models"], serde_json::json!([]));
        task.abort();
    }
    for start in ["SECRET", "2026-10-01", "2026-10-01T00:00:00+02:00"] {
        let (_dir, s) = shared();
        let mut body = usage();
        body["start_date"] = serde_json::json!(start);
        let (api, _, task) = serve(body).await;
        assert_eq!(read_model_spend_with(&s, &api).await["known"], false);
        task.abort();
    }
}

#[tokio::test]
async fn model_spend_no_session_is_unknown_and_empty_success_is_known_zero() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let no_session = DaemonShared::load(store).unwrap();
    let (api, count, task) = serve(usage()).await;
    let unknown = read_model_spend_with(&no_session, &api).await;
    assert_eq!(unknown["known"], false);
    assert_eq!(unknown["state"], "no_session");
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(unknown["since"].is_null());
    task.abort();
    let (_dir, s) = shared();
    let mut body = usage();
    body["data"] = serde_json::json!([]);
    let (api, _, task) = serve(body).await;
    let known = read_model_spend_with(&s, &api).await;
    assert_eq!(known["known"], true);
    assert_eq!(known["models"], serde_json::json!([]));
    task.abort();
}

#[tokio::test]
async fn model_spend_native_maximum_rounding_does_not_overflow() {
    let (_dir, s) = shared();
    let mut body = usage();
    body["data"][0]["total_cost"] = serde_json::json!(i64::MAX);
    let (api, _, task) = serve(body).await;
    let known = read_model_spend_with(&s, &api).await;
    assert_eq!(known["models"][0]["billed_nanos"], i64::MAX);
    assert_eq!(
        known["models"][0]["billed_micros"],
        9_223_372_036_854_776_i64
    );
    task.abort();
}

#[tokio::test]
async fn model_spend_forget_during_http_read_discards_old_account_costs() {
    let (_dir, s) = shared();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let arrived = entered.clone();
    let released = release.clone();
    let router = Router::new()
        .route(
            "/v1/users/me/access-tokens",
            post(|| async {
                Json(serde_json::json!({
            "access_token":"jwt-first","refresh_token":"rt-second",
            "refresh_token_expiration":"2027-01-01T00:00:00Z"}))
            }),
        )
        .route(
            "/v1/organizations/org-selected/usage/by-model",
            get(move || {
                let arrived = arrived.clone();
                let released = released.clone();
                async move {
                    arrived.notify_one();
                    released.notified().await;
                    Json(usage())
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = CloudApi::for_test(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mutate = async {
        entered.notified().await;
        let result = crate::daemon::nearai_credential::handle_forget(
            &s,
            &crate::daemon::ipc::Request {
                id: 1,
                method: "near_ai_credential_forget".into(),
                params: serde_json::json!({}),
            },
        );
        assert!(result.error.is_none());
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(read_model_spend_with(&s, &api), mutate)
    })
    .await
    .unwrap();
    assert_eq!(result["known"], false);
    assert_eq!(result["state"], "no_session");
    assert_eq!(result["models"], serde_json::json!([]));
    server.abort();
}
