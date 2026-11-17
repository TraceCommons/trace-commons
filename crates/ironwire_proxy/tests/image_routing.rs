//! Synthetic wire regressions for provider-observed, per-model image support.
//! Every provider URL is an ephemeral loopback fixture; no inference or login
//! resources are used by this suite.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use ironwire_core::config::{Config, PrivacyConfig, PrivacyMode};
use ironwire_core::protocol::{BackendId, BackendKind};
use ironwire_creds::ConsentLedger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::Backend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt;

const VISION: &str = "fixture/vision";
const TEXT_ONLY: &str = "fixture/text-only";
const UNDECLARED: &str = "fixture/undeclared";
// A generated one-pixel PNG, never a user image.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aG1sAAAAASUVORK5CYII=";

#[derive(Clone, Debug)]
struct Received {
    path: String,
    body: Bytes,
}

#[derive(Clone)]
struct ProviderState {
    catalogue: Value,
    posts: Arc<Mutex<Vec<Received>>>,
    status: StatusCode,
}

struct Provider {
    base: String,
    posts: Arc<Mutex<Vec<Received>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Provider {
    async fn spawn(status: StatusCode) -> Self {
        let posts = Arc::new(Mutex::new(Vec::new()));
        let state = ProviderState {
            catalogue: json!({"data":[
                {"id":VISION, "input_modalities":["text","image"], "output_modalities":["text"]},
                {"id":TEXT_ONLY, "input_modalities":["text"], "output_modalities":["text"]},
                {"id":UNDECLARED}
            ]}),
            posts: Arc::clone(&posts),
            status,
        };
        let router = Router::new()
            .route(
                "/v1/models",
                get(|State(state): State<ProviderState>| async move { Json(state.catalogue) }),
            )
            .route("/v1/chat/completions", post(record_post))
            .route("/v1/responses", post(record_post))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind synthetic provider");
        let addr = listener.local_addr().expect("provider address");
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve synthetic provider");
        });
        Self {
            base: format!("http://{addr}/v1"),
            posts,
            task,
        }
    }

    async fn nearai(&self) -> Arc<ChatCompletionsBackend> {
        let backend = Arc::new(
            ChatCompletionsBackend::nearai(
                Some(SecretString::from("synthetic-key")),
                Some(self.base.clone()),
                Vec::new(),
                5,
            )
            .expect("NEAR AI fixture backend"),
        );
        probe(backend.as_ref()).await;
        backend
    }

    async fn untrusted(&self) -> Arc<ChatCompletionsBackend> {
        let backend = Arc::new(
            ChatCompletionsBackend::new(
                BackendId::from("untrusted"),
                "synthetic alternative",
                BackendKind::ApiKey,
                Some(SecretString::from("synthetic-other-key")),
                self.base.clone(),
                Vec::new(),
                5,
            )
            .expect("untrusted fixture backend"),
        );
        probe(backend.as_ref()).await;
        backend
    }

    fn received(&self) -> Vec<Received> {
        self.posts.lock().expect("recorded requests").clone()
    }
}

async fn probe(backend: &dyn Backend) {
    tokio::time::timeout(Duration::from_secs(5), backend.probe())
        .await
        .expect("catalogue probe is bounded")
        .expect("fixture catalogue accepted");
    assert!(
        backend.catalogue_from_provider(),
        "routing must use the real probe result"
    );
}

async fn record_post(
    State(state): State<ProviderState>,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    state.posts.lock().expect("record request").push(Received {
        path: uri.path().into(),
        body,
    });
    let response = if !state.status.is_success() {
        json!({"error":{"message":"synthetic provider unavailable"}})
    } else if uri.path().ends_with("/responses") {
        json!({"id":"resp_synthetic", "object":"response", "status":"completed", "model":VISION,
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"pixel"}]}],
            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
    } else {
        json!({"id":"chatcmpl_synthetic", "object":"chat.completion", "model":VISION,
            "choices":[{"index":0,"message":{"role":"assistant","content":"pixel"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
    };
    (state.status, Json(response))
}

fn state(backends: Vec<Arc<ChatCompletionsBackend>>, full: bool) -> AppState {
    let mut registry = BackendRegistry::new();
    for backend in backends {
        registry.push(backend);
    }
    let config = Config {
        privacy: PrivacyConfig {
            mode: Some(if full {
                PrivacyMode::Full
            } else {
                PrivacyMode::Off
            }),
            trusted_backends: vec!["nearai".into()],
            ..PrivacyConfig::default()
        },
        ..Config::default()
    };
    AppState::new(
        registry,
        config,
        ConsentLedger::default(),
        "synthetic-control".into(),
    )
}

fn anthropic_image(model: &str) -> String {
    json!({"model":model, "max_tokens":16, "stream":false,
    "messages":[{"role":"user","content":[
        {"type":"text","text":"Describe this pixel."},
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}}
    ]}]})
    .to_string()
}

fn chat_image(model: &str) -> String {
    // Deliberate whitespace and an unknown field detect re-encoding on native lanes.
    format!(
        "{{ \"model\" : \"{model}\", \"stream\":false, \"future\":{{\"keep\":[3,2,1]}}, \"messages\":[{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"Describe this pixel.\"}},{{\"type\":\"image_url\",\"image_url\":{{\"url\":\"data:image/png;base64,{PNG}\",\"detail\":\"high\"}}}}]}}] }}\n"
    )
}

fn responses_image(model: &str) -> String {
    format!(
        "{{ \"model\" : \"{model}\", \"stream\":false, \"future\":{{\"keep\":[3,2,1]}}, \"input\":[{{\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Describe this pixel.\"}},{{\"type\":\"input_image\",\"image_url\":\"data:image/png;base64,{PNG}\",\"detail\":\"high\"}}]}}] }}\n"
    )
}

async fn send(
    state: AppState,
    path: &str,
    body: String,
    forced_model: Option<&str>,
) -> (StatusCode, Bytes) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("session-id", "synthetic-image-session");
    if let Some(model) = forced_model {
        request = request.header("x-ironwire-route", format!("nearai:{model}"));
    }
    tokio::time::timeout(Duration::from_secs(5), async move {
        let response = app(state)
            .oneshot(request.body(Body::from(body)).expect("image request"))
            .await
            .expect("proxy response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("proxy response body");
        (status, body)
    })
    .await
    .expect("proxy exchange is bounded")
}

#[tokio::test]
async fn a_private_anthropic_image_reaches_the_observed_nearai_vision_model() {
    let provider = Provider::spawn(StatusCode::OK).await;
    let (status, response) = send(
        state(vec![provider.nearai().await], true),
        "/anthropic/v1/messages",
        anthropic_image(VISION),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "private image refused: {}",
        String::from_utf8_lossy(&response)
    );
    let posts = provider.received();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/v1/chat/completions");
    let sent: Value = serde_json::from_slice(&posts[0].body).expect("translated chat request");
    assert_eq!(sent["model"], VISION);
    let content = sent["messages"][0]["content"]
        .as_array()
        .expect("multimodal content");
    assert!(
        content
            .iter()
            .any(|part| part["type"] == "text" && part["text"] == "Describe this pixel.")
    );
    assert!(
        content.iter().any(|part| part["type"] == "image_url"
            && part["image_url"]["url"] == format!("data:image/png;base64,{PNG}")),
        "the complete base64 PNG must survive Anthropic-to-chat translation"
    );
    let response: Value = serde_json::from_slice(&response).expect("Anthropic response");
    assert_eq!(response["content"][0]["text"], "pixel");
}

#[tokio::test]
async fn native_chat_images_preserve_the_original_body_bytes() {
    let provider = Provider::spawn(StatusCode::OK).await;
    let body = chat_image(VISION);
    let (status, response) = send(
        state(vec![provider.nearai().await], false),
        "/openai/v1/chat/completions",
        body.clone(),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "image refused: {}",
        String::from_utf8_lossy(&response)
    );
    let posts = provider.received();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/v1/chat/completions");
    assert_eq!(posts[0].body.as_ref(), body.as_bytes());
}

#[tokio::test]
async fn native_responses_images_preserve_the_original_body_bytes() {
    let provider = Provider::spawn(StatusCode::OK).await;
    let body = responses_image(VISION);
    let (status, response) = send(
        state(vec![provider.nearai().await], false),
        "/openai/v1/responses",
        body.clone(),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "image refused: {}",
        String::from_utf8_lossy(&response)
    );
    let posts = provider.received();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/v1/responses");
    assert_eq!(posts[0].body.as_ref(), body.as_bytes());
}

#[tokio::test]
async fn a_vision_sibling_never_unlocks_a_text_only_or_undeclared_model() {
    let provider = Provider::spawn(StatusCode::OK).await;
    let backend = provider.nearai().await;
    for model in [TEXT_ONLY, UNDECLARED] {
        for (path, body) in [
            ("/anthropic/v1/messages", anthropic_image(model)),
            ("/openai/v1/chat/completions", chat_image(model)),
            ("/openai/v1/responses", responses_image(model)),
        ] {
            let (status, response) = send(
                state(vec![Arc::clone(&backend)], true),
                path,
                body,
                Some(model),
            )
            .await;
            assert!(
                !status.is_success(),
                "{path} must refuse images for {model}: {}",
                String::from_utf8_lossy(&response)
            );
            assert!(
                provider.received().is_empty(),
                "an ineligible model must never receive an inference POST"
            );
        }
    }
}

#[tokio::test]
async fn a_private_image_never_falls_back_to_an_untrusted_vision_provider() {
    let trusted = Provider::spawn(StatusCode::SERVICE_UNAVAILABLE).await;
    let untrusted = Provider::spawn(StatusCode::OK).await;
    let (status, _) = send(
        state(
            vec![trusted.nearai().await, untrusted.untrusted().await],
            true,
        ),
        "/openai/v1/chat/completions",
        chat_image(VISION),
        None,
    )
    .await;
    assert!(
        !status.is_success(),
        "a failed trusted provider must leave the private image refused"
    );
    assert!(
        !trusted.received().is_empty(),
        "the trusted vision model should be attempted"
    );
    assert!(
        untrusted.received().is_empty(),
        "full privacy must prevent an inference POST to the alternative provider"
    );
}

fn anthropic_tool_image() -> String {
    json!({"model":VISION,"max_tokens":16,"stream":false,"messages":[
        {"role":"assistant","content":[{"type":"tool_use","id":"toolu_pixel","name":"Read","input":{}}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_pixel","content":[
            {"type":"text","text":"synthetic screenshot"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}}
        ]}]}
    ]}).to_string()
}

fn responses_tool_image() -> String {
    json!({"model":VISION,"stream":false,"input":[
        {"type":"function_call","call_id":"call_pixel","name":"Read","arguments":"{}"},
        {"type":"function_call_output","call_id":"call_pixel","output":[
            {"type":"input_text","text":"synthetic screenshot"},
            {"type":"input_image","image_url":format!("data:image/png;base64,{PNG}"),"detail":"high"}
        ]}
    ]}).to_string()
}

#[tokio::test]
async fn an_established_private_anthropic_route_refuses_image_tool_outputs_without_forwarding() {
    let provider = Provider::spawn(StatusCode::OK).await;
    let state = state(vec![provider.nearai().await], true);
    let (status, _) = send(
        state.clone(),
        "/anthropic/v1/messages",
        anthropic_image(VISION),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "establish the translated session before entering its tool loop"
    );
    assert_eq!(provider.received().len(), 1);
    let (status, response) = send(
        state,
        "/anthropic/v1/messages",
        anthropic_tool_image(),
        None,
    )
    .await;
    assert!(
        !status.is_success(),
        "a screenshot tool result must never be flattened to text"
    );
    assert!(
        String::from_utf8_lossy(&response).contains("images in tool output"),
        "refuse the actual content loss, not a fresh-session mid-tool-loop switch: {}",
        String::from_utf8_lossy(&response)
    );
    assert_eq!(
        provider.received().len(),
        1,
        "no second inference POST may drop the screenshot"
    );
}

#[tokio::test]
async fn an_established_private_responses_chat_route_refuses_image_tool_outputs_without_forwarding()
{
    let provider = Provider::spawn(StatusCode::OK).await;
    let backend = Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("synthetic-key")),
            Some(provider.base.clone()),
            Vec::new(),
            5,
        )
        .expect("NEAR AI fixture backend")
        .responses_via_chat(true),
    );
    probe(backend.as_ref()).await;
    let state = state(vec![backend], true);
    let (status, _) = send(
        state.clone(),
        "/openai/v1/responses",
        responses_image(VISION),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "establish the translated session before entering its tool loop"
    );
    assert_eq!(provider.received().len(), 1);
    let (status, response) =
        send(state, "/openai/v1/responses", responses_tool_image(), None).await;
    assert!(
        !status.is_success(),
        "a screenshot function output must never be flattened to text"
    );
    assert!(
        String::from_utf8_lossy(&response).contains("images in tool output"),
        "refuse the actual content loss, not a fresh-session mid-tool-loop switch: {}",
        String::from_utf8_lossy(&response)
    );
    assert_eq!(
        provider.received().len(),
        1,
        "no second inference POST may drop the screenshot"
    );
}

#[tokio::test]
async fn native_responses_image_tool_outputs_preserve_the_original_body_bytes() {
    let provider = Provider::spawn(StatusCode::OK).await;
    let body = responses_tool_image();
    let (status, response) = send(
        state(vec![provider.nearai().await], true),
        "/openai/v1/responses",
        body.clone(),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "native tool image refused: {}",
        String::from_utf8_lossy(&response)
    );
    let posts = provider.received();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/v1/responses");
    assert_eq!(posts[0].body.as_ref(), body.as_bytes());
}
