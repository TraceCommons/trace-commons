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
    set_rollout(&shared, true);
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
fn set_rollout(shared: &DaemonShared, enabled: bool) {
    let snapshot = shared.feature_flags.snapshot();
    let now = chrono::Utc::now();
    let manifest = json!({
        "schema_version":1,
        "environment":snapshot.environment,
        "revision":snapshot.revision.unwrap_or(0) + 1,
        "issued_at":now,
        "expires_at":now + chrono::Duration::hours(1),
        "flags":{"claude_desktop_private_ai":enabled}
    });
    shared
        .feature_flags
        .accept(&serde_json::to_vec(&manifest).unwrap(), now)
        .unwrap();
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
    Arc<std::sync::atomic::AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let reached = Arc::new(tokio::sync::Notify::new());
    let notified = reached.clone();
    let completions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_completions = completions.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let reached = reached.clone();
            let completions = completions.clone();
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
                    if request.starts_with(b"POST /v1/chat/completions ") {
                        completions.fetch_add(1, Ordering::AcqRel);
                    }
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
    (base, notified, observed_completions, server)
}

#[tokio::test]
async fn shutdown_closes_listener_without_waiting_for_an_active_stream_to_drain() {
    let (base, notified, _completions, server) = held_stream_fixture().await;
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

#[tokio::test]
async fn rollout_defaults_off_and_blocks_presets_without_removing_saved_metadata() {
    let (base, calls, server) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store.clone(), &base);
    let configured = configure(&shared).await;
    let saved = std::fs::read(store.daemon_path(FILE)).unwrap();
    stop(&shared, false).await;
    finish_drains(&shared).await;
    // A separate cache directory exercises the same embedded default-off profile.
    let rollout_dir = tempfile::tempdir().unwrap();
    let mut restarted = DaemonShared::load(store.clone()).unwrap();
    restarted.feature_flags = crate::feature_flags::Runtime::open(
        crate::feature_flags::registry().unwrap(),
        crate::feature_flags::embedded_profile().unwrap(),
        rollout_dir.path(),
        chrono::Utc::now(),
    )
    .unwrap();
    restarted.claude_desktop.lock().unwrap().test_base_url = Some(base);
    *restarted.settings.lock().unwrap() = shared.settings.lock().unwrap().clone();
    assert!(!rollout_enabled(&restarted));
    for method in [
        "claude_desktop_status",
        "claude_desktop_preset",
        "claude_desktop_configure",
    ] {
        let params = if method.ends_with("configure") {
            json!({"model":"fixture-main"})
        } else {
            json!({})
        };
        let value = call(&restarted, method, params).await;
        assert_eq!(value["state"], "unavailable");
        assert!(value.get("environment").is_none());
        assert!(
            !value
                .to_string()
                .contains(variables(&configured)["ANTHROPIC_VERTEX_PROJECT_ID"])
        );
    }
    assert!(restarted.claude_desktop.lock().unwrap().route.is_none());
    assert_eq!(std::fs::read(store.daemon_path(FILE)).unwrap(), saved);
    assert!(calls.lock().unwrap().is_empty());
    call(&restarted, "claude_desktop_disconnect", json!({})).await;
    assert!(!store.daemon_path(FILE).exists());
    server.abort();
}

/// Probe the old capability on the same socket that connected after listener
/// acknowledgement. Only a specific closed transport or HTTP 403 proves refusal;
/// timeouts, malformed responses and other HTTP statuses remain failures.
async fn probe_old_capability_on_socket(
    socket: &mut tokio::net::TcpStream,
    project: &str,
) -> (&'static str, u16, bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let exchange = async {
        let body = json!({"anthropic_version":"vertex-2023-10-16","max_tokens":16,"messages":[{"role":"user","content":"hello"}]}).to_string();
        let request = format!(
            "POST /vertex/projects/{project}/locations/global/publishers/anthropic/models/claude-sonnet-current:streamRawPredict HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if let Err(error) = socket.write_all(request.as_bytes()).await {
            return probe_transport_failure(error.kind(), "write-failed");
        }
        let mut header = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let read = match socket.read(&mut buffer).await {
                Ok(0) if header.is_empty() => return ("closed-without-http", 0, true),
                Ok(0) => return ("incomplete-http", 0, false),
                Ok(read) => read,
                Err(error) if header.is_empty() => {
                    return probe_transport_failure(error.kind(), "read-failed");
                }
                Err(_) => return ("incomplete-http", 0, false),
            };
            header.extend_from_slice(&buffer[..read]);
            if let Some(end) = header.windows(4).position(|part| part == b"\r\n\r\n") {
                let status = std::str::from_utf8(&header[..end]).ok().and_then(|header| {
                    let mut lines = header.split("\r\n");
                    let status_line = lines.next()?;
                    if !status_line
                        .bytes()
                        .all(|byte| byte == b'\t' || (byte >= 32 && byte != 127))
                    {
                        return None;
                    }
                    for line in lines {
                        let (name, value) = line.split_once(':')?;
                        reqwest::header::HeaderName::from_bytes(name.as_bytes()).ok()?;
                        reqwest::header::HeaderValue::from_bytes(value.as_bytes()).ok()?;
                    }
                    let mut fields = status_line.split_whitespace();
                    let protocol = fields.next()?;
                    if !matches!(protocol, "HTTP/1.0" | "HTTP/1.1") {
                        return None;
                    }
                    reqwest::StatusCode::from_bytes(fields.next()?.as_bytes())
                        .ok()
                        .map(|status| status.as_u16())
                        .filter(|status| (100..=599).contains(status))
                });
                return match status {
                    Some(403) => ("http-revoked", 403, true),
                    Some(status) if (200..300).contains(&status) => ("http-success", status, false),
                    Some(status) => ("http-other-status", status, false),
                    None => ("non-http-status", 0, false),
                };
            }
            if header.len() >= 8192 {
                return ("header-limit", 0, false);
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), exchange)
        .await
        .unwrap_or(("http-timeout", 0, false))
}
fn probe_transport_failure(
    kind: std::io::ErrorKind,
    other: &'static str,
) -> (&'static str, u16, bool) {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::ConnectionReset => ("connection-reset", 0, true),
        ErrorKind::ConnectionAborted => ("connection-aborted", 0, true),
        ErrorKind::BrokenPipe => ("broken-pipe", 0, true),
        ErrorKind::NotConnected => ("not-connected", 0, true),
        _ => {
            eprintln!("Desktop post-close transport kind: {kind:?}");
            (other, 0, false)
        }
    }
}

#[tokio::test]
async fn rollout_live_disable_drains_and_reenable_preserves_the_same_capability() {
    let (base, notified, completions, server) = held_stream_fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    let configured = configure(&shared).await;
    let env = variables(&configured);
    let response = reqwest::Client::new().post(format!("{}/projects/{}/locations/global/publishers/anthropic/models/claude-sonnet-current:streamRawPredict", env["ANTHROPIC_VERTEX_BASE_URL"], env["ANTHROPIC_VERTEX_PROJECT_ID"]))
        .json(&json!({"anthropic_version":"vertex-2023-10-16","max_tokens":16,"messages":[{"role":"user","content":"hello"}]})).send().await.unwrap();
    notified.notified().await;
    assert_eq!(completions.load(Ordering::Acquire), 1);
    let lifecycle = tokio::spawn({
        let shared = shared.clone();
        async move { run_rollout(&shared).await }
    });
    let mut stopped = listener_stopped(&shared);
    assert!(!*stopped.borrow());
    set_rollout(&shared, false);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !*stopped.borrow_and_update() {
            stopped
                .changed()
                .await
                .expect("listener acknowledgement remains owned");
        }
    })
    .await
    .expect("live disable acknowledges socket release without waiting for stream drain");
    let address = env["ANTHROPIC_VERTEX_BASE_URL"]
        .trim_start_matches("http://")
        .trim_end_matches("/vertex");
    // Windows can complete a TCP handshake after close acknowledgement while
    // the ensuing HTTP exchange is reset. The product contract is that the old
    // capability cannot serve inference; require concrete refusal evidence.
    let closed_or_revoked = match tokio::net::TcpStream::connect(address).await {
        Err(_) => true,
        Ok(mut socket) => {
            let before = completions.load(Ordering::Acquire);
            let (classification, status, refused) =
                probe_old_capability_on_socket(&mut socket, env["ANTHROPIC_VERTEX_PROJECT_ID"])
                    .await;
            let delta = completions.load(Ordering::Acquire).saturating_sub(before);
            eprintln!(
                "Desktop post-close diagnostic: {classification}; http_status={status}; fixture_call_delta={delta}"
            );
            refused
        }
    };
    assert_eq!(
        completions.load(Ordering::Acquire),
        1,
        "closed Desktop route forwarded another completion"
    );
    assert!(
        closed_or_revoked,
        "old Desktop capability did not prove transport closure or HTTP 403 after listener acknowledgement"
    );
    assert!(store.daemon_path(FILE).exists());
    assert!(
        call(&shared, "claude_desktop_preset", json!({}))
            .await
            .get("environment")
            .is_none()
    );
    drop(response);
    finish_drains(&shared).await;
    set_rollout(&shared, true);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if shared.claude_desktop.lock().unwrap().route.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        call(&shared, "claude_desktop_preset", json!({})).await["environment"],
        configured["environment"]
    );
    lifecycle.abort();
    stop(&shared, true).await;
    finish_drains(&shared).await;
    server.abort();
}

#[tokio::test]
async fn rollout_expiry_stops_listener_and_account_changes_revoke_while_off() {
    let (base, _, server) = fixture().await;
    for revoked in ["none", "sign_out", "key_change", "private_ai_off"] {
        let (_dir, store) = crate::config::tests_support::temp_store();
        let shared = Arc::new(shared(store.clone(), &base));
        configure(&shared).await;
        let lifecycle = tokio::spawn({
            let shared = shared.clone();
            async move { run_rollout(&shared).await }
        });
        shared
            .feature_flags
            .tick(chrono::Utc::now() + chrono::Duration::hours(2));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while shared.claude_desktop.lock().unwrap().route.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!rollout_enabled(&shared));
        assert!(store.daemon_path(FILE).exists());
        match revoked {
            "sign_out" => shared.settings.lock().unwrap().near_ai_inference = None,
            "key_change" => {
                shared
                    .settings
                    .lock()
                    .unwrap()
                    .near_ai_inference
                    .as_mut()
                    .unwrap()
                    .key = "sk-another-account".into()
            }
            "private_ai_off" => shared.settings.lock().unwrap().private_inference = false,
            _ => {}
        }
        reconcile(&shared).await;
        assert_eq!(
            store.daemon_path(FILE).exists(),
            revoked == "none",
            "{revoked}"
        );
        lifecycle.abort();
        stop(&shared, true).await;
        finish_drains(&shared).await;
    }
    server.abort();
}

#[tokio::test]
async fn disabling_rollout_cancels_a_pending_start_without_publishing_metadata() {
    let (base, reached, server) = gated_fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    let pending = tokio::spawn({
        let shared = shared.clone();
        async move { configure(&shared).await }
    });
    reached.notified().await;
    let lifecycle = tokio::spawn({
        let shared = shared.clone();
        async move { run_rollout(&shared).await }
    });
    set_rollout(&shared, false);
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["state"], "unavailable");
    assert!(result.get("environment").is_none());
    assert!(!store.daemon_path(FILE).exists());
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    lifecycle.abort();
    server.abort();
}

#[tokio::test]
async fn final_start_publication_rechecks_rollout_without_a_reconcile() {
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let app = Router::new().route("/v1/models", get({
        let reached = reached.clone();
        let release = release.clone();
        move || {
            let reached = reached.clone();
            let release = release.clone();
            let first = first.clone();
            async move {
                if first.swap(false, Ordering::AcqRel) {
                    reached.notify_one();
                    release.notified().await;
                }
                Json(json!({"data":[{"id":"fixture-main","owned_by":"nearai","context_length":262144,"max_output_length":8192},{"id":"fixture-background","owned_by":"nearai","context_length":262144,"max_output_length":8192}]}))
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    let pending = tokio::spawn({
        let shared = shared.clone();
        async move { configure(&shared).await }
    });
    reached.notified().await;
    set_rollout(&shared, false);
    release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["state"], "unavailable");
    assert!(result.get("environment").is_none());
    assert!(!store.daemon_path(FILE).exists());
    assert!(!shared.claude_desktop.lock().unwrap().starting);
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    finish_drains(&shared).await;
    server.abort();
}

pub(in crate::daemon) fn listener_stopped(
    shared: &DaemonShared,
) -> tokio::sync::watch::Receiver<bool> {
    shared
        .claude_desktop
        .lock()
        .unwrap()
        .route
        .as_ref()
        .unwrap()
        .listener_stopped_for_test()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_publication_holds_rollout_guard_until_metadata_and_route_commit() {
    let (base, _, server) = fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(shared(store.clone(), &base));
    let (entered, observed) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    let weak = Arc::downgrade(&shared);
    shared.claude_desktop.lock().unwrap().test_publication = Some(Arc::new(move || {
        let shared = weak.upgrade().unwrap();
        // This assertion is at the exact old race gap, after the enabled
        // decision and before Desktop ownership. It deterministically fails if
        // final publication no longer owns the Runtime state mutex.
        assert!(shared.feature_flags.try_snapshot().is_none());
        entered.lock().unwrap().take().unwrap().send(()).unwrap();
        released.lock().unwrap().recv().unwrap();
    }));
    let pending = tokio::spawn({
        let shared = shared.clone();
        async move { configure(&shared).await }
    });
    observed.await.unwrap();
    // Off may be attempted while publication is held, but its acceptance must
    // observe the completed save. The exact-gap assertion above proves why it
    // cannot overtake this commit; no scheduler timing is used as evidence.
    let (attempt, attempted) = tokio::sync::oneshot::channel();
    let disabled = std::thread::spawn({
        let shared = shared.clone();
        let store = store.clone();
        move || {
            attempt.send(()).unwrap();
            set_rollout(&shared, false);
            assert!(load(&store).unwrap().is_some());
        }
    });
    attempted.await.unwrap();
    release.send(()).unwrap();
    pending.await.unwrap();
    disabled.join().unwrap();
    assert!(!rollout_enabled(&shared));
    shared.claude_desktop.lock().unwrap().test_publication = None;
    reconcile(&shared).await;
    assert!(shared.claude_desktop.lock().unwrap().route.is_none());
    assert!(load(&store).unwrap().is_some());
    stop(&shared, true).await;
    finish_drains(&shared).await;
    server.abort();
}

fn assert_pending<F: std::future::Future>(future: std::pin::Pin<&mut F>) {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(future.poll(&mut context).is_pending());
}

#[tokio::test]
async fn concurrent_stops_all_wait_for_the_same_pending_listener_acknowledgement() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store, "http://127.0.0.1:9/v1");
    let (closed, pending) = tokio::sync::watch::channel(false);
    shared
        .claude_desktop
        .lock()
        .unwrap()
        .start_done
        .push(pending);
    let mut first = Box::pin(stop(&shared, false));
    let mut second = Box::pin(stop(&shared, true));
    let mut third = Box::pin(stop(&shared, false));
    assert_pending(first.as_mut());
    assert_pending(second.as_mut());
    assert_pending(third.as_mut());
    assert_eq!(shared.claude_desktop.lock().unwrap().start_done.len(), 1);
    closed.send_replace(true);
    first.await;
    second.await;
    third.await;
}

#[tokio::test]
async fn cancelled_reconfigure_preserves_prior_listener_barriers_for_every_stop() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store, "http://127.0.0.1:9/v1");
    let (closed, pending) = tokio::sync::watch::channel(false);
    shared
        .claude_desktop
        .lock()
        .unwrap()
        .start_done
        .push(pending);
    let request = Request {
        id: 1,
        method: "claude_desktop_configure".into(),
        params: json!({"model":"fixture-main"}),
    };
    let mut configuring = Box::pin(handle_inline(&shared, &request));
    assert_pending(configuring.as_mut());
    // Cancelling the new start completes its own guard, never the older close.
    drop(configuring);
    let mut first = Box::pin(stop(&shared, false));
    let mut second = Box::pin(stop(&shared, true));
    let mut third = Box::pin(stop(&shared, false));
    assert_pending(first.as_mut());
    assert_pending(second.as_mut());
    assert_pending(third.as_mut());
    assert_eq!(shared.claude_desktop.lock().unwrap().start_done.len(), 1);
    closed.send_replace(true);
    first.await;
    second.await;
    third.await;
}

#[tokio::test]
async fn late_capability_probe_accepts_only_eof_or_http_403() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (reply, expected_status, refused) in [
        ("", 0, true),
        (
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
            403,
            true,
        ),
        ("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", 200, false),
        (
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
            503,
            false,
        ),
        ("not-http\r\n\r\n", 0, false),
        ("HTTP/1.1 0403 Forbidden\r\n\r\n", 0, false),
        ("HTTP/1.1 403 Forbidden\r\nnot-a-header\r\n\r\n", 0, false),
        (
            "HTTP/1.1 403 Forbidden\r\nBad Header: value\r\n\r\n",
            0,
            false,
        ),
        (
            "HTTP/1.1 403 Forbidden\r\nX-Reason: bad\0value\r\n\r\n",
            0,
            false,
        ),
        ("HTTP/1.1 403 Forbidden\r\n", 0, false),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            // Read the complete request so closing a successful fixture response
            // does not itself reset the transport because unread body bytes remain.
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&buffer[..read]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header_end = end + 4;
                    let length = std::str::from_utf8(&request[..end])
                        .unwrap()
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= header_end + length {
                        break;
                    }
                }
            }
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        let (_, status, closed_or_revoked) =
            probe_old_capability_on_socket(&mut socket, "fixture-project").await;
        assert_eq!(status, expected_status);
        assert_eq!(closed_or_revoked, refused);
        server.await.unwrap();
    }
}

#[test]
fn late_capability_probe_rejects_unspecified_transport_failures() {
    use std::io::ErrorKind;
    for kind in [
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::BrokenPipe,
        ErrorKind::NotConnected,
    ] {
        assert!(probe_transport_failure(kind, "read-failed").2);
    }
    for kind in [
        ErrorKind::TimedOut,
        ErrorKind::WouldBlock,
        ErrorKind::UnexpectedEof,
        ErrorKind::Other,
    ] {
        assert!(!probe_transport_failure(kind, "read-failed").2);
    }
}

#[tokio::test]
async fn late_capability_probe_has_a_live_gateway_positive_control() {
    let (base, reached, completions, server) = held_stream_fixture().await;
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = shared(store, &base);
    let configured = configure(&shared).await;
    let env = variables(&configured);
    let address = env["ANTHROPIC_VERTEX_BASE_URL"]
        .trim_start_matches("http://")
        .trim_end_matches("/vertex");
    let mut live = tokio::net::TcpStream::connect(address).await.unwrap();
    let (classification, status, refused) =
        probe_old_capability_on_socket(&mut live, env["ANTHROPIC_VERTEX_PROJECT_ID"]).await;
    assert_eq!(classification, "http-success");
    assert_eq!(status, 200);
    assert!(
        !refused,
        "the valid old-capability request must detect a live route"
    );
    reached.notified().await;
    assert_eq!(completions.load(Ordering::Acquire), 1);
    let stopped = listener_stopped(&shared);
    stop(&shared, false).await;
    assert!(*stopped.borrow());
    let closed_or_revoked = match tokio::net::TcpStream::connect(address).await {
        Err(_) => true,
        Ok(mut socket) => {
            probe_old_capability_on_socket(&mut socket, env["ANTHROPIC_VERTEX_PROJECT_ID"])
                .await
                .2
        }
    };
    assert!(closed_or_revoked);
    assert_eq!(completions.load(Ordering::Acquire), 1);
    drop(live);
    finish_drains(&shared).await;
    stop(&shared, true).await;
    server.abort();
}
