use super::*;

#[tokio::test]
async fn mission_catalogue_is_discoverable_and_missing_config_is_unavailable() {
    let s = shared();
    let hello = handle_request_async(&s, &req("hello", serde_json::json!({}))).await;
    assert!(
        hello.result.unwrap()["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m == "mission_catalogue")
    );
    let response = handle_request_async(&s, &req("mission_catalogue", serde_json::json!({}))).await;
    assert!(response.result.is_none());
    let error = response.error.unwrap();
    assert_eq!(error.code, ERR_UNAVAILABLE);
    assert_eq!(error.message, "mission-catalogue-unavailable");
}

use axum::{
    Json, Router,
    extract::Query,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use std::{collections::BTreeMap, path::PathBuf, sync::atomic::AtomicUsize};

async fn mission_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), task)
}

fn configure_catalogue(s: &DaemonShared, base: &str) {
    let mut cfg = crate::commands::unenrolled_preview_config();
    cfg.ingest_url = format!("{base}/v1/traces");
    cfg.allowed_hosts = Some("127.0.0.1".into());
    cfg.display_handle = Some("private-local-handle".into());
    cfg.user_subject = "private-subject".into();
    s.store.save_config(&cfg).unwrap();
}

fn mission_page() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "entries": [{
            "mission_id": "00000000-0000-0000-0000-000000000003",
            "program_id": "00000000-0000-0000-0000-000000000002",
            "package_sha256": "c".repeat(64),
            "offer_version_hash": format!("sha256:{}", "d".repeat(64)),
            "task_preview": "Repair the generated manifest.",
            "published_at": "2026-10-01T00:00:00Z"
        }],
        "next_cursor": "00000000-0000-0000-0000-000000000003"
    })
}

fn persisted_files(dir: &std::path::Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(persisted_files(&path));
        } else {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}

#[tokio::test]
async fn mission_catalogue_relays_exact_page_anonymously_without_mutating_consent() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = Router::new().route(
        "/v1/missions",
        get(
            move |headers: HeaderMap, Query(query): Query<BTreeMap<String, String>>| {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(
                        query,
                        BTreeMap::from([
                            ("limit".into(), "1".into()),
                            (
                                "before".into(),
                                "00000000-0000-0000-0000-000000000004".into()
                            )
                        ])
                    );
                    for header in [
                        "authorization",
                        "cookie",
                        "x-api-key",
                        "x-tenant-id",
                        "x-user-subject",
                        "x-profile",
                        "x-matched",
                    ] {
                        assert!(!headers.contains_key(header), "{header} must not be sent");
                    }
                    Json(mission_page())
                }
            },
        ),
    );
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let entry = QueueEntry {
        entry_id: Uuid::new_v4(),
        project_key: "/private-project".into(),
        project_label: "private-project".into(),
        session_hash: "sha256:private-session".into(),
        ..Default::default()
    };
    s.queue.lock().unwrap().upsert(entry, 500).unwrap();
    s.queue.lock().unwrap().save(&s.store).unwrap();
    s.policy
        .lock()
        .unwrap()
        .set_mode("/private-project", ProjectMode::NotifyOnly, Utc::now())
        .unwrap();
    s.policy
        .lock()
        .unwrap()
        .set_mode("/ignored-project", ProjectMode::Ignore, Utc::now())
        .unwrap();
    s.policy.lock().unwrap().save(&s.store).unwrap();
    s.state.lock().unwrap().paused = true;
    s.paused.store(true, Ordering::SeqCst);
    HistoryCache::save(&s.store, &[]).unwrap();
    s.state.lock().unwrap().save(&s.store).unwrap();
    // Even a saved native session must not become a discovery credential.
    s.store
        .write_daemon_file(
            crate::config::ACCOUNT_SESSION_FILE,
            b"private-session-not-needed-for-discovery",
        )
        .unwrap();
    let before_files = persisted_files(s.store.dir());
    let before_policy = s.policy.lock().unwrap().clone();
    let before_state = s.state.lock().unwrap().clone();
    let before_settings = serde_json::to_value(&*s.settings.lock().unwrap()).unwrap();
    let before_queue = serde_json::to_value(s.queue.lock().unwrap().all()).unwrap();
    let response = handle_request_async(
        &s,
        &req(
            "mission_catalogue",
            serde_json::json!({"limit":1, "before":"00000000-0000-0000-0000-000000000004"}),
        ),
    )
    .await;
    assert!(response.error.is_none());
    let result = response.result.unwrap();
    assert_eq!(result["kind"], "skill_evaluation");
    assert_eq!(
        result["disclosure"],
        "These are published skill-evaluation tasks. Matching stays on this Mac; no activity profile or match result is sent. Viewing or selecting a mission changes no capture or contribution permissions and sends no sessions. Contributions still require your existing consent. Corpus credit remains pending and conditional until settlement. Skill-evaluation awards are separate from corpus credit."
    );
    assert_eq!(result["catalogue"], mission_page());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(s.paused.load(Ordering::SeqCst));
    assert_eq!(persisted_files(s.store.dir()), before_files);
    assert_eq!(*s.policy.lock().unwrap(), before_policy);
    assert_eq!(*s.state.lock().unwrap(), before_state);
    assert_eq!(
        serde_json::to_value(&*s.settings.lock().unwrap()).unwrap(),
        before_settings
    );
    assert_eq!(
        serde_json::to_value(s.queue.lock().unwrap().all()).unwrap(),
        before_queue
    );
    server.abort();
}

#[tokio::test]
async fn mission_catalogue_rejects_profile_and_invalid_params_before_http() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = Router::new().fallback(move || {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            StatusCode::OK
        }
    });
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    for params in [
        serde_json::json!({"limit":0}),
        serde_json::json!({"limit":-1}),
        serde_json::json!({"limit":51}),
        serde_json::json!({"limit":null}),
        serde_json::json!({"before":"00000000-0000-0000-0000-000000000000"}),
        serde_json::json!({"before":"invalid"}),
        serde_json::json!({"before":12}),
        serde_json::json!({"profile":{}}),
        serde_json::json!({"matched":true}),
        serde_json::json!({"activity":[]}),
        serde_json::json!({"tags":[]}),
        serde_json::json!({"tool":"private-tool"}),
        serde_json::json!({"session_id":"private"}),
        serde_json::json!({"ingest_url":base}),
        serde_json::json!([]),
        serde_json::Value::Null,
    ] {
        let response = handle_request_async(&s, &req("mission_catalogue", params)).await;
        assert!(response.result.is_none());
        let error = response.error.unwrap();
        assert_eq!(error.code, ERR_BAD_PARAMS);
        assert_eq!(error.message, "mission-catalog-request-invalid");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn mission_catalogue_default_fetch_can_return_verified_empty_catalogue() {
    let empty = serde_json::json!({"schema_version":1, "entries":[], "next_cursor":null});
    let app = Router::new().route(
        "/v1/missions",
        get(|Query(query): Query<BTreeMap<String, String>>| async move {
            assert_eq!(query, BTreeMap::from([("limit".into(), "20".into())]));
            Json(serde_json::json!({"schema_version":1, "entries":[], "next_cursor":null}))
        }),
    );
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    for params in [serde_json::json!({}), serde_json::json!({"before":null})] {
        let response = handle_request_async(&s, &req("mission_catalogue", params)).await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["kind"], "skill_evaluation");
        assert_eq!(
            result["disclosure"],
            "These are published skill-evaluation tasks. Matching stays on this Mac; no activity profile or match result is sent. Viewing or selecting a mission changes no capture or contribution permissions and sends no sessions. Contributions still require your existing consent. Corpus credit remains pending and conditional until settlement. Skill-evaluation awards are separate from corpus credit."
        );
        assert_eq!(result["catalogue"], empty);
    }
    server.abort();
}

#[tokio::test]
async fn mission_catalogue_failures_are_fixed_errors_never_empty_success() {
    for (status, body, label) in [
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "private remote failure",
            "mission-catalogue-unavailable",
        ),
        (
            StatusCode::OK,
            "invalid private body",
            "mission-catalogue-unavailable",
        ),
    ] {
        let app = Router::new().route("/v1/missions", get(move || async move { (status, body) }));
        let (base, server) = mission_server(app).await;
        let s = shared();
        configure_catalogue(&s, &base);
        let response =
            handle_request_async(&s, &req("mission_catalogue", serde_json::json!({}))).await;
        assert!(response.result.is_none());
        let error = response.error.unwrap();
        assert_eq!(error.code, ERR_UNAVAILABLE);
        assert_eq!(error.message, label);
        server.abort();
    }
    let s = shared();
    configure_catalogue(&s, "https://unapproved.example");
    let response = handle_request_async(&s, &req("mission_catalogue", serde_json::json!({}))).await;
    assert!(response.result.is_none());
    let error = response.error.unwrap();
    assert_eq!(error.code, ERR_UNAVAILABLE);
    assert_eq!(error.message, "mission-catalogue-unavailable");
}

#[tokio::test]
async fn history_rollup_uses_cached_standing_rank_with_null_and_stale_semantics() {
    let s = shared();
    let response = handle_request_async(&s, &req("history_rollup", serde_json::json!({}))).await;
    assert!(response.result.unwrap().get("community").is_none());
    let standing = crate::daemon::community::CommunityStanding {
        rank: Some(7),
        novelty_credit: 12.5,
        accepted_in_window: 4,
        accept_rate: None,
        window_label: "7d".into(),
        public_since: None,
        snapshot_at: Some(Utc::now()),
        analytics_withheld: true,
    };
    s.state.lock().unwrap().community = Some(standing.clone());
    let response = handle_request_async(&s, &req("history_rollup", serde_json::json!({}))).await;
    let community = response.result.unwrap()["community"].clone();
    assert_eq!(community["rank"], 7);
    assert!(community["accept_rate"].is_null());
    assert!(community.get("top_percentile").is_none());
    s.state.lock().unwrap().community.as_mut().unwrap().rank = None;
    let response = handle_request_async(&s, &req("history_rollup", serde_json::json!({}))).await;
    assert!(response.result.unwrap()["community"]["rank"].is_null());
    s.state
        .lock()
        .unwrap()
        .community
        .as_mut()
        .unwrap()
        .snapshot_at = Some(Utc::now() - chrono::Duration::minutes(31));
    let response = handle_request_async(&s, &req("history_rollup", serde_json::json!({}))).await;
    assert!(response.result.unwrap().get("community").is_none());
}
