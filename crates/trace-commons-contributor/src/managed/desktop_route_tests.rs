//! The embedded proxy accepts Desktop's Vertex envelope without owning Desktop.
use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Mutex;

type Seen = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

#[tokio::test]
async fn desktop_vertex_calls_use_only_the_routes_near_key_and_never_invent_token_counts() {
    async fn completion(
        State(seen): State<Seen>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        seen.lock().unwrap().push((headers, body));
        Json(json!({
            "id":"desktop-fixture", "object":"chat.completion", "created":1,
            "model":"desktop-model", "choices":[{"index":0,
                "message":{"role":"assistant","content":"READY"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":7,"completion_tokens":1,"total_tokens":8}
        }))
    }
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async {
                Json(json!({"data":[{
                    "id":"desktop-model", "input_modalities":["text"],
                    "output_modalities":["text"], "context_length":131072
                }]}))
            }),
        )
        .route("/v1/chat/completions", post(completion))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let route = ManagedRoute::start_at(
        &store,
        Uuid::new_v4(),
        ApiKey::new("desktop-host-key".into()).unwrap(),
        &endpoint,
        ToolId::Claude,
        Some("desktop-model"),
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    // A managed route's default is a launcher choice, not a model pin. Desktop
    // model selection is a later product gate; configure the fixture's explicit
    // pin through the existing authenticated host control API.
    let control = std::fs::read_to_string(route.home.join("control.token")).unwrap();
    let pinned = client
        .post(format!("http://127.0.0.1:{}/_ironwire/pin", route.port()))
        .bearer_auth(control.trim())
        .json(&json!({"backend":"nearai","model":route.default_model()}))
        .send()
        .await
        .unwrap();
    assert!(pinned.status().is_success());
    let base = format!(
        "http://127.0.0.1:{}/vertex/projects/preset-project/locations/preset-region/publishers/anthropic/models",
        route.port()
    );
    let response = client
        .post(format!("{base}/claude-sonnet-4-6:rawPredict"))
        .header("authorization", "Bearer desktop-must-not-forward")
        .header("x-api-key", "desktop-must-not-forward")
        .header("x-claude-code-session-id", "desktop-session-fixture")
        .json(
            &json!({"anthropic_version":"vertex-2023-10-16", "max_tokens":16,
            "messages":[{"role":"user","content":"Reply READY"}]}),
        )
        .send()
        .await
        .unwrap();
    let status = response.status();
    assert!(status.is_success(), "Vertex endpoint returned {status}");
    let answer = response.json::<Value>().await.unwrap();
    assert_eq!(answer["content"][0]["text"], "READY");
    {
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let (headers, body) = &requests[0];
        assert_eq!(headers["authorization"], "Bearer desktop-host-key");
        assert!(!headers.contains_key("x-api-key"));
        assert_eq!(body["model"], "desktop-model");
        let serialized = body.to_string();
        for absent in [
            "anthropic_version",
            "preset-project",
            "preset-region",
            "desktop-must-not-forward",
        ] {
            assert!(
                !serialized.contains(absent),
                "unexpected upstream field: {absent}"
            );
        }
    }
    let count = client
        .post(format!("{base}/count-tokens:rawPredict"))
        .json(&json!({"anthropic_version":"vertex-2023-10-16", "messages":[]}))
        .send()
        .await
        .unwrap();
    assert!(count.status().is_client_error(), "{}", count.status());
    let count = count.json::<Value>().await.unwrap();
    assert_eq!(count["type"], "error");
    assert!(count.get("input_tokens").is_none());
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "count must not call the model"
    );
    route.shutdown().await;
    server.abort();
}

/// The injected attempt refuses with `PortInUse` for its first `busy` calls,
/// as a port Windows is still releasing does, then binds.
fn released_after(
    busy: usize,
    calls: Arc<std::sync::atomic::AtomicUsize>,
) -> impl FnMut() -> std::future::Ready<Result<u16, embed::EmbedError>> {
    move || {
        let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::ready(if call < busy {
            Err(embed::EmbedError::PortInUse { port: 8471 })
        } else {
            Ok(8471)
        })
    }
}

#[tokio::test(start_paused = true)]
async fn a_restart_waits_for_its_saved_port_while_the_os_releases_it() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let started = start_on_released_port(
        8471,
        RELEASED_PORT_WITHIN,
        RELEASED_PORT_PAUSE,
        released_after(3, calls.clone()),
    )
    .await;
    assert_eq!(started.ok(), Some(8471));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
}

#[tokio::test(start_paused = true)]
async fn a_port_another_listener_keeps_is_refused_after_a_bounded_wait() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let begun = tokio::time::Instant::now();
    let started = start_on_released_port(
        8471,
        RELEASED_PORT_WITHIN,
        RELEASED_PORT_PAUSE,
        released_after(usize::MAX, calls.clone()),
    )
    .await;
    assert!(matches!(
        started,
        Err(embed::EmbedError::PortInUse { port: 8471 })
    ));
    let waited = begun.elapsed();
    assert!(waited >= RELEASED_PORT_WITHIN, "{waited:?}");
    assert!(
        waited < RELEASED_PORT_WITHIN + RELEASED_PORT_PAUSE * 2,
        "{waited:?}"
    );
    let calls = calls.load(std::sync::atomic::Ordering::SeqCst);
    let bound = (RELEASED_PORT_WITHIN.as_millis() / RELEASED_PORT_PAUSE.as_millis()) as usize + 2;
    assert!(calls > 1 && calls <= bound, "{calls}");
}

#[tokio::test(start_paused = true)]
async fn only_a_fixed_port_refused_as_in_use_is_tried_again() {
    // Port 0 lets the OS choose, so an in-use refusal there is not a
    // released port and is returned at once.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let started = start_on_released_port(
        0,
        RELEASED_PORT_WITHIN,
        RELEASED_PORT_PAUSE,
        released_after(1, calls.clone()),
    )
    .await;
    assert!(started.is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Any other refusal is not about the port being released.
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let started: Result<u16, _> =
        start_on_released_port(8471, RELEASED_PORT_WITHIN, RELEASED_PORT_PAUSE, || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(Err(embed::EmbedError::Bind))
        })
        .await;
    assert!(matches!(started, Err(embed::EmbedError::Bind)));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
