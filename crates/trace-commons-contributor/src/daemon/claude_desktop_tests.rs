use super::*;
use crate::daemon::settings::NearAiInferenceCredential;
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use std::sync::{Arc, Mutex};
type Calls = Arc<Mutex<Vec<(String, Value)>>>;
async fn fixture() -> (String, Calls, tokio::task::JoinHandle<()>) {
    async fn completion(
        State(calls): State<Calls>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        calls.lock().unwrap().push((
            headers["authorization"].to_str().unwrap().into(),
            body.clone(),
        ));
        Json(
            json!({"id":"fixture-response","object":"chat.completion","model":body["model"],"choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}),
        )
    }
    let calls: Calls = Default::default();
    let app = Router::new().route("/v1/models", get(|| async {Json(json!({"data":[
        {"id":"fixture-main","owned_by":"nearai","context_length":262144,"max_output_length":8192},
        {"id":"fixture-background","owned_by":"nearai","context_length":131072,"max_output_length":8192},
        {"id":"fixture-external","owned_by":"vendor","context_length":262144,"max_output_length":8192},
        {"id":"fixture-small","owned_by":"nearai","context_length":32768,"max_output_length":8192},
        {"id":"fixture-unsized","owned_by":"nearai"},
        {"id":"fixture-wide-output","owned_by":"nearai","context_length":1048576,"max_output_length":1048576}
    ]}))})).route("/v1/chat/completions", post(completion)).with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, calls, task)
}
pub(in crate::daemon) fn shared(store: ConfigStore, base: &str) -> DaemonShared {
    let shared = DaemonShared::load(store).unwrap();
    shared.claude_desktop.lock().unwrap().test_base_url = Some(base.into());
    shared.settings.lock().unwrap().private_inference = true;
    shared.settings.lock().unwrap().near_ai_inference = Some(NearAiInferenceCredential {
        key: "sk-desktop-fixture".into(),
        key_id: "fixture-key".into(),
        key_prefix: "sk-fi".into(),
        organization_id: "org".into(),
        workspace_id: "workspace".into(),
        minted_at: chrono::Utc::now(),
    });
    shared
}
async fn call(shared: &DaemonShared, method: &str, params: Value) -> Value {
    let req = Request {
        id: 1,
        method: method.into(),
        params,
    };
    let response = handle(shared, &req).await;
    assert!(response.error.is_none(), "{response:?}");
    response.result.unwrap()
}
async fn configure(shared: &DaemonShared) -> Value {
    call(
        shared,
        "claude_desktop_configure",
        json!({"model":"fixture-main","background_model":"fixture-background"}),
    )
    .await
}
fn variables(value: &Value) -> std::collections::BTreeMap<&str, &str> {
    value["environment"]
        .as_str()
        .unwrap()
        .lines()
        .map(|line| line.split_once('=').unwrap())
        .collect()
}
async fn infer(shared: &DaemonShared, model: &str) -> reqwest::Response {
    let value = call(shared, "claude_desktop_preset", json!({})).await;
    let env = variables(&value);
    reqwest::Client::new().post(format!("{}/projects/{}/locations/global/publishers/anthropic/models/{model}:rawPredict",env["ANTHROPIC_VERTEX_BASE_URL"],env["ANTHROPIC_VERTEX_PROJECT_ID"]))
        .bearer_auth("must-not-forward").json(&json!({"anthropic_version":"vertex-2023-10-16","max_tokens":16,"messages":[{"role":"user","content":"hello"}]})).send().await.unwrap()
}
#[tokio::test]
async fn persistent_binding_is_safe_observed_only_after_completion_and_revocable() {
    let (base, calls, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store.clone(), &base);
    let first = configure(&shared).await;
    assert_eq!(first["state"], "ready");
    assert_eq!(first["observed"], false);
    let env = variables(&first);
    assert_eq!(env["CLAUDE_CODE_USE_VERTEX"], "1");
    assert_eq!(env["CLAUDE_CODE_SKIP_VERTEX_AUTH"], "1");
    assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
    assert_eq!(env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "122880");
    let safe = call(&shared, "claude_desktop_status", json!({})).await;
    assert!(safe.get("environment").is_none());
    assert!(
        !safe
            .to_string()
            .contains(env["ANTHROPIC_VERTEX_PROJECT_ID"])
    );
    assert!(
        !std::fs::read_to_string(store.daemon_path(FILE))
            .unwrap()
            .contains("sk-desktop-fixture")
    );
    let main = infer(&shared, "claude-sonnet-current").await;
    assert!(main.status().is_success());
    assert_eq!(main.json::<Value>().await.unwrap()["model"], "fixture-main");
    let background = infer(&shared, "claude-haiku-current").await;
    assert!(background.status().is_success());
    assert_eq!(
        background.json::<Value>().await.unwrap()["model"],
        "fixture-background"
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(calls.lock().unwrap()[0].0, "Bearer sk-desktop-fixture");
    assert_eq!(calls.lock().unwrap()[0].1["model"], "fixture-main");
    assert_eq!(calls.lock().unwrap()[1].1["model"], "fixture-background");
    assert_eq!(
        call(&shared, "claude_desktop_status", json!({})).await["state"],
        "observed"
    );
    stop(&shared, false).await;
    finish_drains(&shared).await;
    let restarted = self::shared(store.clone(), &base);
    reconcile(&restarted).await;
    let again = call(&restarted, "claude_desktop_preset", json!({})).await;
    assert_eq!(first["environment"], again["environment"]);
    assert_eq!(again["state"], "observed");
    let disconnected = call(&restarted, "claude_desktop_disconnect", json!({})).await;
    assert_eq!(disconnected["state"], "disconnected");
    assert!(disconnected.get("environment").is_none());
    assert!(
        tokio::net::TcpStream::connect(
            env["ANTHROPIC_VERTEX_BASE_URL"]
                .trim_start_matches("http://")
                .trim_end_matches("/vertex")
        )
        .await
        .is_err()
    );
    reconcile(&restarted).await;
    assert_eq!(
        call(&restarted, "claude_desktop_status", json!({})).await["state"],
        "disconnected"
    );
    assert!(!store.daemon_path(FILE).exists());
    task.abort();
}
#[tokio::test]
async fn disabled_missing_key_changed_key_and_dry_run_revoke_without_reassigning() {
    let (base, _, task) = fixture().await;
    for condition in ["disabled", "missing", "changed", "dryrun", "shutdown"] {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let mut shared = shared(store.clone(), &base);
        configure(&shared).await;
        match condition {
            "disabled" => shared.settings.lock().unwrap().private_inference = false,
            "missing" => shared.settings.lock().unwrap().near_ai_inference = None,
            "changed" => {
                shared
                    .settings
                    .lock()
                    .unwrap()
                    .near_ai_inference
                    .as_mut()
                    .unwrap()
                    .key = "sk-other-account".into()
            }
            "dryrun" => shared.dev_dry_run = true,
            _ => shared
                .shutdown
                .store(true, std::sync::atomic::Ordering::Release),
        }
        reconcile(&shared).await;
        assert_eq!(
            store.daemon_path(FILE).exists(),
            condition == "shutdown",
            "{condition}"
        );
        assert!(shared.claude_desktop.lock().unwrap().route.is_none());
        assert!(
            call(&shared, "claude_desktop_status", json!({}))
                .await
                .get("environment")
                .is_none()
        );
    }
    task.abort();
}
#[tokio::test]
async fn no_enable_invalid_model_privacy_and_context_fail_closed() {
    let (base, _, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store.clone(), &base);
    shared.settings.lock().unwrap().private_inference = false;
    assert_eq!(configure(&shared).await["state"], "disabled");
    assert!(!shared.settings.lock().unwrap().private_inference);
    shared.settings.lock().unwrap().private_inference = true;
    for (main, background) in [
        ("not-listed", "fixture-background"),
        ("fixture-main", "fixture-external"),
        ("fixture-small", "fixture-background"),
        ("fixture-unsized", "fixture-background"),
    ] {
        let value = call(
            &shared,
            "claude_desktop_configure",
            json!({"model":main,"background_model":background}),
        )
        .await;
        assert_eq!(value["state"], "unavailable");
        assert!(value.get("environment").is_none());
        assert!(!store.daemon_path(FILE).exists());
    }
    task.abort();
}
#[tokio::test]
async fn corrupt_saved_binding_and_port_conflict_do_not_pick_another_port() {
    let (base, _, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    store.write_daemon_file(FILE, b"{bad").unwrap();
    let shared = shared(store.clone(), &base);
    reconcile(&shared).await;
    assert_eq!(
        call(&shared, "claude_desktop_status", json!({})).await["state"],
        "unavailable"
    );
    call(&shared, "claude_desktop_disconnect", json!({})).await;
    configure(&shared).await;
    let binding = shared
        .claude_desktop
        .lock()
        .unwrap()
        .binding
        .clone()
        .unwrap();
    stop(&shared, false).await;
    finish_drains(&shared).await;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, binding.port))
        .await
        .unwrap();
    reconcile(&shared).await;
    assert_eq!(
        call(&shared, "claude_desktop_status", json!({})).await["state"],
        "unavailable"
    );
    assert_eq!(load(&store).unwrap().unwrap().port, binding.port);
    drop(listener);
    call(&shared, "claude_desktop_disconnect", json!({})).await;
    task.abort();
}
#[cfg(unix)]
#[tokio::test]
async fn symlink_saved_binding_is_refused_and_atomic_metadata_is_private() {
    use std::os::unix::{fs::PermissionsExt, fs::symlink};
    let (base, _, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let target = store.daemon_path("outside");
    std::fs::write(&target, b"unchanged").unwrap();
    symlink(&target, store.daemon_path(FILE)).unwrap();
    let shared = shared(store.clone(), &base);
    assert_eq!(configure(&shared).await["state"], "unavailable");
    assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
    call(&shared, "claude_desktop_disconnect", json!({})).await;
    configure(&shared).await;
    assert_eq!(
        std::fs::metadata(store.daemon_path(FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    stop(&shared, true).await;
    task.abort();
}

#[tokio::test]
async fn count_overrides_and_wrong_capabilities_do_not_observe_or_forward() {
    let (base, calls, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store.clone(), &base);
    let configured = configure(&shared).await;
    let env = variables(&configured);
    let root = format!(
        "{}/projects/{}/locations/global/publishers/anthropic/models",
        env["ANTHROPIC_VERTEX_BASE_URL"], env["ANTHROPIC_VERTEX_PROJECT_ID"]
    );
    let body = json!({"anthropic_version":"vertex-2023-10-16","max_tokens":16,"messages":[{"role":"user","content":"hello"}]} );
    let client = reqwest::Client::new();
    for (url, override_route) in [
        (format!("{root}/count-tokens:rawPredict"), false),
        (format!("{root}/claude-sonnet-current:rawPredict"), true),
        (
            format!(
                "{}/projects/wrong-project/locations/global/publishers/anthropic/models/claude-sonnet-current:rawPredict",
                env["ANTHROPIC_VERTEX_BASE_URL"]
            ),
            false,
        ),
    ] {
        let mut request = client.post(url).json(&body).bearer_auth("never-forward");
        if override_route {
            request = request.header("x-ironwire-route", "subscription/other-model");
        }
        assert!(request.send().await.unwrap().status().is_client_error());
    }
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        call(&shared, "claude_desktop_status", json!({})).await["state"],
        "ready"
    );
    let old = load(&store).unwrap().unwrap();
    let replaced = configure(&shared).await;
    assert_ne!(
        variables(&replaced)["ANTHROPIC_VERTEX_PROJECT_ID"],
        old.project
    );
    let route_home = crate::managed::route::home_for(&store, old.id);
    assert!(
        route_home.join("ledger.sqlite").exists(),
        "disconnect preserves local observation ledger"
    );
    let config = std::fs::read_to_string(route_home.join("config.toml")).unwrap();
    assert!(config.contains("bodies = false"));
    assert!(!config.contains("sk-desktop-fixture"));
    stop(&shared, true).await;
    task.abort();
}

async fn gated_fixture() -> (
    String,
    Arc<tokio::sync::Notify>,
    tokio::task::JoinHandle<()>,
) {
    let reached = Arc::new(tokio::sync::Notify::new());
    let gate = reached.clone();
    let app = Router::new().route(
        "/v1/models",
        get(move || {
            let gate = gate.clone();
            async move {
                gate.notify_one();
                std::future::pending::<()>().await;
                Json(json!({"data":[]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, reached, task)
}
#[tokio::test]
async fn disconnect_and_key_change_cancel_pending_start_without_resurrection() {
    for disconnect in [true, false] {
        let (base, reached, server) = gated_fixture().await;
        let (_dir, store) = crate::config::tests_support::temp_store();
        let shared = Arc::new(shared(store.clone(), &base));
        let pending = tokio::spawn({
            let shared = shared.clone();
            async move { configure(&shared).await }
        });
        reached.notified().await;
        let revoke = async {
            if disconnect {
                call(&shared, "claude_desktop_disconnect", json!({})).await;
            } else {
                shared
                    .settings
                    .lock()
                    .unwrap()
                    .near_ai_inference
                    .as_mut()
                    .unwrap()
                    .key = "sk-another-account".into();
                reconcile(&shared).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), revoke)
            .await
            .expect("withdrawal cancels catalogue wait");
        pending.await.unwrap();
        assert!(!store.daemon_path(FILE).exists());
        assert!(shared.claude_desktop.lock().unwrap().route.is_none());
        server.abort();
    }
}
#[tokio::test]
async fn cancelled_configure_never_saves_or_restarts_a_connection() {
    let (base, reached, server) = gated_fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    let pending = tokio::spawn({
        let shared = shared.clone();
        async move { configure(&shared).await }
    });
    reached.notified().await;
    pending.abort();
    let _ = pending.await;
    assert!(!store.daemon_path(FILE).exists());
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    reconcile(&shared).await;
    assert!(!store.daemon_path(FILE).exists());
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    server.abort();
}

pub(in crate::daemon) async fn held_stream_fixture() -> (
    String,
    Arc<tokio::sync::Notify>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let reached = Arc::new(tokio::sync::Notify::new());
    let notified = reached.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let reached = reached.clone();
            tokio::spawn(async move {
                let mut request = vec![];
                let mut buffer = [0; 8192];
                loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..n]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                let headers_end = request
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                let content_length = std::str::from_utf8(&request[..headers_end])
                    .unwrap()
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while request.len() < headers_end + content_length {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                if request.starts_with(b"GET /v1/models ") {
                    let body=json!({"data":[{"id":"fixture-main","owned_by":"nearai","context_length":262144,"max_output_length":8192},{"id":"fixture-background","owned_by":"nearai","context_length":262144,"max_output_length":8192}]}).to_string();
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    socket.write_all(reply.as_bytes()).await.unwrap();
                } else {
                    let chunk = "data: {\"id\":\"hanging\",\"object\":\"chat.completion.chunk\",\"model\":\"fixture-main\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\n";
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n",
                        chunk.len(),
                        chunk
                    );
                    socket.write_all(reply.as_bytes()).await.unwrap();
                    reached.notify_one();
                    let _ = socket.read(&mut buffer).await;
                }
            });
        }
    });
    (base, notified, server)
}

#[tokio::test]
async fn shutdown_closes_listener_without_waiting_for_an_active_stream_to_drain() {
    let (base, notified, server) = held_stream_fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store.clone(), &base);
    let configured = configure(&shared).await;
    let env = variables(&configured);
    let address = env["ANTHROPIC_VERTEX_BASE_URL"]
        .trim_start_matches("http://")
        .trim_end_matches("/vertex");
    let request=reqwest::Client::new().post(format!("{}/projects/{}/locations/global/publishers/anthropic/models/claude-sonnet-current:streamRawPredict",env["ANTHROPIC_VERTEX_BASE_URL"],env["ANTHROPIC_VERTEX_PROJECT_ID"]))
        .json(&json!({"anthropic_version":"vertex-2023-10-16","max_tokens":16,"messages":[{"role":"user","content":"hello"}]})).send();
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap();
    assert!(response.status().is_success());
    notified.notified().await;
    assert_eq!(
        call(&shared, "claude_desktop_status", json!({})).await["state"],
        "ready"
    );
    shared
        .shutdown
        .store(true, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        shared.stop_private_inference(),
    )
    .await
    .expect("quit remains bounded while stream is active");
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(
        store.daemon_path(FILE).exists(),
        "normal restart retains binding"
    );
    reconcile(&shared).await;
    assert!(
        store.daemon_path(FILE).exists(),
        "status during shutdown retains restart binding"
    );
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    drop(response);
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn redirected_route_files_and_readable_capabilities_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (base, _, task) = fixture().await;
    for redirected_route in [true, false] {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let shared = shared(store.clone(), &base);
        configure(&shared).await;
        let binding = load(&store).unwrap().unwrap();
        stop(&shared, false).await;
        finish_drains(&shared).await;
        if redirected_route {
            let home = crate::managed::route::home_for(&store, binding.id);
            let outside = store.daemon_path("outside-ledger");
            std::fs::write(&outside, b"do-not-open").unwrap();
            std::fs::remove_file(home.join("ledger.sqlite")).unwrap();
            symlink(&outside, home.join("ledger.sqlite")).unwrap();
            reconcile(&shared).await;
            assert_eq!(std::fs::read(&outside).unwrap(), b"do-not-open");
        } else {
            std::fs::set_permissions(
                store.daemon_path(FILE),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            reconcile(&shared).await;
        }
        assert_eq!(
            call(&shared, "claude_desktop_status", json!({})).await["state"],
            "unavailable"
        );
        assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    }
    task.abort();
}
async fn finish_drains(shared: &DaemonShared) {
    let drains = std::mem::take(&mut shared.claude_desktop.lock().unwrap().drains);
    for drain in drains {
        drain.await.unwrap();
    }
}

#[tokio::test]
async fn restart_refuses_a_preset_larger_than_the_current_model_budget() {
    let (base, _, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store.clone(), &base);
    configure(&shared).await;
    stop(&shared, false).await;
    finish_drains(&shared).await;
    let mut binding = load(&store).unwrap().unwrap();
    binding.compact_window = 1_000_000;
    store
        .write_daemon_file(FILE, &serde_json::to_vec(&binding).unwrap())
        .unwrap();
    let restarted = self::shared(store.clone(), &base);
    reconcile(&restarted).await;
    assert_eq!(
        call(&restarted, "claude_desktop_status", json!({})).await["state"],
        "unavailable"
    );
    assert!(restarted.claude_desktop.lock().unwrap().route.is_none());
    assert_eq!(load(&store).unwrap().unwrap().project, binding.project);
    task.abort();
}

#[tokio::test]
async fn large_advertised_output_does_not_reserve_more_than_the_emitted_client_cap() {
    let (base, _, task) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store, &base);
    let configured = call(
        &shared,
        "claude_desktop_configure",
        json!({"model":"fixture-main","background_model":"fixture-wide-output"}),
    )
    .await;
    assert_eq!(configured["state"], "ready");
    let env = variables(&configured);
    assert_eq!(env["CLAUDE_CODE_MAX_OUTPUT_TOKENS"], "8192");
    assert_eq!(env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "253952");
    stop(&shared, true).await;
    task.abort();
}

#[test]
fn ffi_handle_local_connection_outlives_its_throwaway_request_runtime() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (base, calls, server) = runtime.block_on(fixture());
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    runtime.block_on(async {
        shared.adopt_runtime();
        shared.adopt_self();
    });
    let configured = super::super::ipc::handle_local(
        &shared,
        "claude_desktop_configure",
        json!({"model":"fixture-main","background_model":"fixture-background"}),
    );
    assert!(configured.error.is_none(), "{configured:?}");
    assert_eq!(configured.result.unwrap()["state"], "ready");
    // Each tc_call uses handle_local's temporary runtime, then drops it.
    // The actual listener and all housekeeping must remain on the daemon.
    let response = runtime.block_on(infer(&shared, "claude-sonnet-current"));
    assert!(response.status().is_success());
    assert_eq!(
        runtime.block_on(response.json::<Value>()).unwrap()["model"],
        "fixture-main"
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    let status = super::super::ipc::handle_local(&shared, "claude_desktop_status", json!({}));
    assert_eq!(status.result.unwrap()["state"], "observed");
    let disconnected =
        super::super::ipc::handle_local(&shared, "claude_desktop_disconnect", json!({}));
    assert_eq!(disconnected.result.unwrap()["state"], "disconnected");
    assert!(!store.daemon_path(FILE).exists());
    runtime.block_on(finish_drains(&shared));
    server.abort();
}

#[tokio::test]
async fn ffi_runtime_caller_cancellation_cancels_its_pending_start() {
    let (base, reached, server) = gated_fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    shared.adopt_runtime();
    shared.adopt_self();
    let pending = tokio::spawn({
        let shared = shared.clone();
        async move { configure(&shared).await }
    });
    reached.notified().await;
    pending.abort();
    let _ = pending.await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if !shared.claude_desktop.lock().unwrap().starting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled caller releases daemon startup generation");
    assert!(!store.daemon_path(FILE).exists());
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    server.abort();
}

#[tokio::test]
async fn ipc_developer_dry_run_refuses_without_mutating_the_saved_binding() {
    let (base, _, server) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut shared = shared(store.clone(), &base);
    configure(&shared).await;
    let saved = std::fs::read(store.daemon_path(FILE)).unwrap();
    shared.dev_dry_run = true;
    for method in [
        "claude_desktop_status",
        "claude_desktop_configure",
        "claude_desktop_preset",
        "claude_desktop_disconnect",
    ] {
        let response = super::super::ipc::handle_request_async(
            &shared,
            &Request {
                id: 1,
                method: method.into(),
                params: json!({}),
            },
        )
        .await;
        assert!(response.error.is_some(), "{method}");
        assert_eq!(std::fs::read(store.daemon_path(FILE)).unwrap(), saved);
    }
    stop(&shared, true).await;
    server.abort();
}
