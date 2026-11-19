//! Conformance for the Vertex envelope, before the existing Messages pipeline.
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, Uri};
use ironwire_core::config::Config;
use ironwire_core::protocol::{BackendId, BackendKind, ModelTier};
use ironwire_creds::ConsentLedger;
use ironwire_proxy::embed::{EmbedOptions, VertexGateway};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tower::ServiceExt;

const PREFIX: &str =
    "/vertex/projects/private-project/locations/private-region/publishers/anthropic/models/";
const JSON: &str = r#"{"id":"msg_vertex","type":"message","role":"assistant","model":"served-model","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":2,"output_tokens":1}}"#;
const SSE: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_vertex","type":"message","role":"assistant","model":"served-model","content":[],"usage":{"input_tokens":2,"output_tokens":0}}}"#,
    "\n\nevent: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}"#,
    "\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
);
type Seen = Arc<Mutex<Vec<(Uri, HeaderMap, Bytes)>>>;

async fn setup() -> (AppState, Seen) {
    setup_with_status(StatusCode::OK).await
}

async fn setup_with_status(status: StatusCode) -> (AppState, Seen) {
    let seen: Seen = Arc::default();
    let received = seen.clone();
    let upstream =
        axum::Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: Bytes| {
            let received = received.clone();
            async move {
                let streaming =
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["stream"] == true;
                let model = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["model"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                received.lock().unwrap().push((uri, headers, body));
                (
                    status,
                    [
                        ("retry-after", "60"),
                        (
                            "content-type",
                            if streaming {
                                "text/event-stream"
                            } else {
                                "application/json"
                            },
                        ),
                    ],
                    if streaming {
                        SSE.to_owned()
                    } else {
                        JSON.replace("served-model", &model)
                    },
                )
            }
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(SecretString::from("host-key"), Some(base.clone()), 5).unwrap(),
    ));
    registry.push(Arc::new(
        ChatCompletionsBackend::new(
            BackendId::from("spare"),
            "Spare",
            BackendKind::ApiKey,
            Some(SecretString::from("spare-key")),
            base,
            vec![("escape-model".into(), ModelTier::Frontier)],
            5,
        )
        .unwrap(),
    ));
    let state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "control-token".into(),
    )
    .with_ledger(Some(ironwire_ledger::Ledger::in_memory().unwrap()));
    state.policy.lock().unwrap().set_pin(
        Some(BackendId::from("anthropic-key")),
        Some("served-model".into()),
    );
    (state, seen)
}

fn request(suffix: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{PREFIX}{suffix}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn vertex_uses_messages_routing_and_retains_requested_model_separately_from_pin() {
    for stream in [false, true] {
        let (state, seen) = setup().await;
        let action = if stream {
            "streamRawPredict"
        } else {
            "rawPredict"
        };
        let mut request = request(
            &format!("claude-client-alias@20261001:{action}"),
            r#"{"anthropic_version":"vertex-2023-10-16","max_tokens":16,"messages":[{"role":"user","content":"hello"}],"future":{"preserve":true}}"#,
        );
        request.headers_mut().insert(
            "authorization",
            "Bearer vertex-client-secret".parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("x-goog-api-key", "google-secret".parse().unwrap());
        request.headers_mut().insert(
            "x-goog-request-params",
            "projects=private-project&locations=private-region"
                .parse()
                .unwrap(),
        );
        request.headers_mut().insert(
            "x-claude-code-session-id",
            "desktop-session".parse().unwrap(),
        );
        let response = app(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(
            response.as_ref(),
            if stream {
                SSE.as_bytes()
            } else {
                JSON.as_bytes()
            }
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.path(), "/v1/messages");
        let sent: serde_json::Value = serde_json::from_slice(&seen[0].2).unwrap();
        assert_eq!(sent["model"], "served-model");
        assert_eq!(sent["stream"], stream);
        assert_eq!(sent["future"], serde_json::json!({"preserve":true}));
        assert!(sent.get("anthropic_version").is_none());
        assert!(
            sent.get("system").is_none(),
            "a facade must never manufacture client identity"
        );
        let rendered = format!("{:?}", seen[0]);
        for secret in [
            "vertex-client-secret",
            "google-secret",
            "private-project",
            "private-region",
        ] {
            assert!(!rendered.contains(secret), "leaked {secret}");
        }
        assert_eq!(seen[0].1["x-api-key"], "host-key");
        let rows = state.ledger.as_ref().unwrap().recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/v1/messages");
        assert_eq!(
            rows[0].requested_model.as_deref(),
            Some("claude-client-alias@20261001")
        );
        assert_eq!(rows[0].served_model.as_deref(), Some("served-model"));
        assert_eq!(
            rows[0].client_session_id.as_deref(),
            Some("desktop-session")
        );
    }
}

#[tokio::test]
async fn vertex_counting_is_terminal_without_an_upstream_request_or_fabricated_count() {
    let (state, seen) = setup().await;
    let response = app(state.clone())
        .oneshot(request(
            "count-tokens:rawPredict",
            r#"{"anthropic_version":"vertex-2023-10-16","messages":[]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get("retry-after").is_none());
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(body.get("input_tokens").is_none());
    assert!(seen.lock().unwrap().is_empty());
    let rows = state.ledger.as_ref().unwrap().recent(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, "/v1/messages/count_tokens");
}

#[tokio::test]
async fn malformed_vertex_requests_are_refused_without_echoing_path_or_body_values() {
    for (suffix, body) in [
        ("claude:rawPredict", "not JSON private-project"),
        ("claude:rawPredict", "[]"),
        ("claude:rawPredict", "null"),
        ("claude:rawPredict", r#"{"model":"other"}"#),
        ("claude:rawPredict", r#"{"stream":true}"#),
        ("claude:streamRawPredict", r#"{"stream":false}"#),
        ("claude:streamRawPredict", r#"{"stream":"true"}"#),
        (
            "claude:rawPredict",
            r#"{"anthropic_version":"unsupported"}"#,
        ),
        ("claude:rawPredict", r#"{"model":"other","model":"claude"}"#),
        ("claude:rawPredict", r#"{"stream":true,"stream":false}"#),
        ("claude:unknownAction", "{}"),
        ("claude%2Fescape:rawPredict", "{}"),
        ("claude:rawPredict/extra", "{}"),
        ("count-tokens:streamRawPredict", "{}"),
    ] {
        let (state, seen) = setup().await;
        let response = app(state.clone())
            .oneshot(request(suffix, body))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{suffix} {body}"
        );
        let error = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&error).unwrap();
        assert_eq!(error["error"]["type"], "invalid_request_error");
        assert!(!error.to_string().contains("private-project"));
        assert!(seen.lock().unwrap().is_empty());
        let rows = state.ledger.as_ref().unwrap().recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].path == "/v1/messages" || rows[0].path == "/v1/messages/count_tokens");
        assert!(rows[0].requested_model.is_none());
    }
}

#[tokio::test]
async fn a_vertex_route_override_cannot_escape_the_existing_pin() {
    let (state, seen) = setup().await;
    let mut request = request("claude:rawPredict", r#"{"messages":[]}"#);
    request
        .headers_mut()
        .insert("x-ironwire-route", "other:escape".parse().unwrap());
    let response = app(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(
        state.ledger.as_ref().unwrap().recent(1).unwrap()[0].path,
        "/v1/messages"
    );
}

#[tokio::test]
async fn oversized_vertex_uploads_keep_anthropic_errors_and_canonical_ledger_paths() {
    let (state, seen) = setup().await;
    let response = app(state.clone())
        .oneshot(request(
            "claude:rawPredict",
            &"x".repeat(ironwire_proxy::facade::MAX_REQUEST_BODY_BYTES + 1),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"]["type"],
        "invalid_request_error"
    );
    assert!(seen.lock().unwrap().is_empty());
    let rows = state.ledger.as_ref().unwrap().recent(1).unwrap();
    assert_eq!(rows[0].path, "/v1/messages");
    assert_eq!(rows[0].failure.as_deref(), Some("body_rejected"));
}

const CAPABILITY: &str = "nearai-0123456789abcdef0123456789abcdef";

fn gateway(background: Option<&str>) -> VertexGateway {
    VertexGateway::new(
        CAPABILITY.into(),
        "anthropic-key".into(),
        "served-model".into(),
        background.map(str::to_owned),
    )
    .unwrap()
}

fn bound_request(project: &str, model: &str, body: &str) -> Request<Body> {
    Request::builder().method("POST")
        .uri(format!("/vertex/projects/{project}/locations/private-region/publishers/anthropic/models/{model}:rawPredict"))
        .header("content-type", "application/json").body(Body::from(body.to_owned())).unwrap()
}

#[test]
fn bound_gateway_validation_and_debug_never_echo_the_capability() {
    let options = EmbedOptions::default().with_vertex_gateway(gateway(None));
    assert!(!format!("{options:?}").contains(CAPABILITY));
    assert!(!format!("{:?}", gateway(None)).contains(CAPABILITY));
    for (project, backend, model, background, label) in [
        (
            "private-secret",
            "anthropic-key",
            "model",
            None,
            "invalid_vertex_project",
        ),
        (CAPABILITY, "", "model", None, "invalid_vertex_backend"),
        (
            CAPABILITY,
            "anthropic-key",
            "",
            None,
            "invalid_vertex_model",
        ),
        (
            CAPABILITY,
            "anthropic-key",
            "model",
            Some(""),
            "invalid_vertex_background_model",
        ),
    ] {
        assert_eq!(
            VertexGateway::new(
                project.into(),
                backend.into(),
                model.into(),
                background.map(str::to_owned)
            )
            .unwrap_err(),
            label
        );
    }
}

#[tokio::test]
async fn bound_gateway_authenticates_before_parsing_or_answering_counts() {
    let (state, seen) = setup().await;
    let state = state.with_vertex_gateway(gateway(None));
    for project in [
        "wrong",
        "",
        "nearai-0123456789abcdef0123456789abcdeF",
        "nearai-0123456789abcdef0123456789abcdef%2F",
    ] {
        for model in ["claude", "count-tokens"] {
            let response = app(state.clone())
                .oneshot(bound_request(project, model, "not JSON"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let error = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&error).unwrap()["error"]["message"],
                "Invalid Vertex capability."
            );
        }
    }
    let response = app(state.clone())
        .oneshot(bound_request(
            "wrong",
            "claude",
            &"x".repeat(ironwire_proxy::facade::MAX_REQUEST_BODY_BYTES + 1),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app(state)
        .oneshot(bound_request(CAPABILITY, "count-tokens", "{}"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bound_gateway_hides_other_inference_facades_and_preserves_control_auth() {
    let (state, seen) = setup().await;
    let state = state.with_vertex_gateway(gateway(None));
    for path in [
        "/anthropic/v1/messages",
        "/anthropic/account/anthropic-key/v1/messages",
        "/openai/v1/chat/completions",
        "/openai/v1/responses",
        "/anthropic/v1/models",
        "/openai/v1/models",
    ] {
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    for (path, expected) in [
        ("/_ironwire/health", StatusCode::OK),
        ("/_ironwire/status", StatusCode::UNAUTHORIZED),
    ] {
        let response = app(state.clone())
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/_ironwire/status")
                .header("authorization", "Bearer control-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&status).contains(CAPABILITY));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bound_gateway_maps_main_and_haiku_without_losing_requested_model() {
    for background in [None, Some("served-fast")] {
        let (state, seen) = setup().await;
        let state = state.with_vertex_gateway(gateway(background));
        // Even authenticated mutable control pins cannot widen the host binding.
        state
            .policy
            .lock()
            .unwrap()
            .set_pin(Some(BackendId::from("spare")), Some("escape-model".into()));
        for requested in ["claude-opus", "claude-HaIkU@20261001"] {
            let response = app(state.clone())
                .oneshot(bound_request(
                    CAPABILITY,
                    requested,
                    r#"{"max_tokens":16,"messages":[{"role":"user","content":"hello"}]}"#,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let _ = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            let served = if requested.contains("HaIkU") {
                background.unwrap_or("served-model")
            } else {
                "served-model"
            };
            let rows = state.ledger.as_ref().unwrap().recent(1).unwrap();
            assert_eq!(rows[0].requested_model.as_deref(), Some(requested));
            assert_eq!(rows[0].served_model.as_deref(), Some(served));
            let seen = seen.lock().unwrap();
            let last = seen.last().unwrap();
            assert_eq!(last.0.path(), "/v1/messages");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&last.2).unwrap()["model"],
                served
            );
            let wire = format!("{last:?}");
            assert!(!wire.contains(CAPABILITY));
            assert!(!wire.contains("private-region"));
        }
    }
}

#[tokio::test]
async fn bound_gateway_never_falls_back_to_a_second_backend_or_model() {
    let (state, seen) = setup_with_status(StatusCode::TOO_MANY_REQUESTS).await;
    let state = state.with_vertex_gateway(gateway(Some("served-fast")));
    let response = app(state.clone())
        .oneshot(bound_request(
            CAPABILITY,
            "claude-opus",
            r#"{"messages":[]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0.path(), "/v1/messages");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&seen[0].2).unwrap()["model"],
        "served-model"
    );
}

#[tokio::test]
async fn bound_gateway_refuses_route_headers_and_keeps_model_eligibility() {
    let (state, seen) = setup().await;
    let state = state.with_vertex_gateway(gateway(None));
    let mut override_request = bound_request(CAPABILITY, "claude", r#"{"messages":[]}"#);
    override_request
        .headers_mut()
        .insert("x-ironwire-route", "spare:escape-model".parse().unwrap());
    assert_eq!(
        app(state.clone())
            .oneshot(override_request)
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    // A configured text-only Chat model must still refuse image input, even
    // when the bound route forces its backend and model.
    let state = state.with_vertex_gateway(
        VertexGateway::new(
            CAPABILITY.into(),
            "spare".into(),
            "escape-model".into(),
            None,
        )
        .unwrap(),
    );
    let response = app(state).oneshot(bound_request(CAPABILITY, "claude", r#"{"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}}]}]}"#)).await.unwrap();
    assert!(!response.status().is_success());
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bound_background_models_keep_their_provider_stated_context_limit() {
    use ironwire_upstream::backend::Backend;
    let inference = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received = inference.clone();
    let upstream = axum::Router::new()
        .route(
            "/models",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"data": [
                    {"id": "wide-model", "context_length": 200000},
                    {"id": "small-model", "context_length": 1024}
                ]}))
            }),
        )
        .fallback(move || {
            received.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { StatusCode::INTERNAL_SERVER_ERROR }
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let backend = ChatCompletionsBackend::new(
        BackendId::from("sized"),
        "Sized",
        BackendKind::ApiKey,
        Some(SecretString::from("host-key")),
        base,
        vec![],
        5,
    )
    .unwrap();
    backend.probe().await.unwrap();
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(backend));
    let state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "control".into(),
    )
    .with_vertex_gateway(
        VertexGateway::new(
            CAPABILITY.into(),
            "sized".into(),
            "wide-model".into(),
            Some("small-model".into()),
        )
        .unwrap(),
    );
    let response = app(state)
        .oneshot(bound_request(
            CAPABILITY,
            "claude-haiku",
            r#"{"max_tokens":4096,"messages":[]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(inference.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bound_translated_responses_report_provider_model_while_retaining_client_alias() {
    for streaming in [false, true] {
        for provider_model in ["configured-qwen", "canonical-qwen-provider"] {
            for bound in [false, true] {
                let seen = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
                let received = seen.clone();
                let upstream = axum::Router::new().fallback(move |body: Bytes| {
                    let received = received.clone();
                    async move {
                        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let stream = body["stream"] == true;
                        received.lock().unwrap().push(body);
                        if stream {
                            let event = serde_json::json!({"id":"translated-id", "model":provider_model, "choices":[{"index":0,"delta":{"role":"assistant","content":"READY"},"finish_reason":null}]});
                            ([ ("content-type", "text/event-stream") ], format!("data: {event}\n\ndata: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":2,\"completion_tokens\":1}}}}\n\ndata: [DONE]\n\n"))
                        } else {
                            ([ ("content-type", "application/json") ], serde_json::json!({"id":"translated-id","model":provider_model,"choices":[{"index":0,"message":{"role":"assistant","content":"READY"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1}}).to_string())
                        }
                    }
                });
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let task =
                    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
                let mut registry = BackendRegistry::new();
                registry.push(Arc::new(
                    ChatCompletionsBackend::new(
                        BackendId::from("qwen"),
                        "Qwen",
                        BackendKind::ApiKey,
                        Some(SecretString::from("host-key")),
                        base,
                        vec![("configured-qwen".into(), ModelTier::Frontier)],
                        5,
                    )
                    .unwrap(),
                ));
                let state = AppState::new(
                    registry,
                    Config::default(),
                    ConsentLedger::default(),
                    "control".into(),
                )
                .with_ledger(Some(ironwire_ledger::Ledger::in_memory().unwrap()));
                state.policy.lock().unwrap().set_pin(
                    Some(BackendId::from("qwen")),
                    Some("configured-qwen".into()),
                );
                let state = if bound {
                    state.with_vertex_gateway(
                        VertexGateway::new(
                            CAPABILITY.into(),
                            "qwen".into(),
                            "configured-qwen".into(),
                            None,
                        )
                        .unwrap(),
                    )
                } else {
                    state
                };
                let action = if streaming {
                    "streamRawPredict"
                } else {
                    "rawPredict"
                };
                let request = Request::builder().method("POST").uri(format!("/vertex/projects/{CAPABILITY}/locations/region/publishers/anthropic/models/claude-sonnet-current:{action}"))
                    .body(Body::from(r#"{"max_tokens":17,"messages":[{"role":"user","content":"hi"}]}"#)).unwrap();
                let response = app(state.clone()).oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                    .await
                    .unwrap();
                let expected = if bound {
                    provider_model
                } else {
                    "claude-sonnet-current"
                };
                if streaming {
                    let frames: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
                        .unwrap()
                        .lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect();
                    let start = frames
                        .iter()
                        .find(|frame| frame["type"] == "message_start")
                        .unwrap();
                    assert_eq!(start["message"]["model"], expected, "bound={bound}");
                } else {
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["model"],
                        expected,
                        "bound={bound}"
                    );
                }
                let rows = state.ledger.as_ref().unwrap().recent(1).unwrap();
                assert_eq!(
                    rows[0].requested_model.as_deref(),
                    Some("claude-sonnet-current")
                );
                assert_eq!(rows[0].served_model.as_deref(), Some(expected));
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0]["model"], "configured-qwen");
                assert_eq!(seen[0]["max_tokens"], 17);
                task.abort();
            }
        }
    }
}

#[tokio::test]
async fn stopping_a_bound_gateway_revokes_new_requests_on_idle_and_live_connections() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (state, seen) = setup().await;
    let state = state.with_vertex_gateway(gateway(None));
    let listener = ironwire_proxy::server::bind(0).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = state.clone();
    // Keep the listener alive while announcing the stop flag. This makes a
    // new handler's refusal observable independently of OS socket closure or
    // the scheduling of graceful shutdown on already-accepted connections.
    let server =
        tokio::spawn(async move { axum::serve(listener, app(server_state)).await.unwrap() });
    let path = format!(
        "/vertex/projects/{CAPABILITY}/locations/region/publishers/anthropic/models/claude:rawPredict"
    );
    let url = format!("http://{address}{path}");
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .body(r#"{"messages":[]}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.bytes().await.unwrap();
    let before = seen.lock().unwrap().len();
    assert_eq!(before, 1);
    let mut idle = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = r#"{"messages":[]}"#;
    idle.write_all(format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n", body.len()).as_bytes()).await.unwrap();
    state.shutdown.begin();
    // The pooled client can reuse its live connection; the incomplete idle
    // request becomes a complete request only after capability revocation.
    let response = client.post(&url).body(body).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["message"],
        "Invalid Vertex capability."
    );
    idle.write_all(format!("\r\n{body}").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        idle.read_to_end(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 403"));
    let response = reqwest::Client::new()
        .post(&url)
        .body("not JSON")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let health = reqwest::get(format!("http://{address}/_ironwire/health"))
        .await
        .unwrap();
    assert_eq!(
        health.status(),
        StatusCode::OK,
        "the listener still served health, so refusal came from the capability gate"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        before,
        "revoked requests must never reach inference"
    );
    server.abort();
}

#[tokio::test]
async fn generic_vertex_requests_keep_existing_behavior_during_shutdown() {
    let (state, seen) = setup().await;
    state.shutdown.begin();
    let response = app(state)
        .oneshot(request("claude:rawPredict", r#"{"messages":[]}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}
