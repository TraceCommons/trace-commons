//! A Responses client must receive executable calls and replay their results
//! through NEAR AI's Chat Completions compatibility lane.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::post,
};
use ironwire_core::{
    config::Config,
    protocol::{BackendId, ModelTier},
};
use ironwire_creds::ConsentLedger;
use ironwire_proxy::{
    server::app,
    state::{AppState, BackendRegistry},
};
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

async fn chat(
    State(seen): State<Arc<Mutex<Vec<Value>>>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let second = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["role"] == "tool");
    let function_name = body["tools"][0]["function"]["name"].clone();
    seen.lock().unwrap().push(body);
    let chunks = if second {
        vec![
            json!({"id":"chat_2","choices":[{"index":0,"delta":{"role":"assistant","content":"verified"},"finish_reason":null}]}),
            json!({"id":"chat_2","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":2}}),
        ]
    } else {
        vec![
            json!({"id":"chat_1","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_probe","type":"function","function":{"name":function_name,"arguments":"{\"marker\":"}}]},"finish_reason":null}]}),
            json!({"id":"chat_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"synthetic\"}"}}]},"finish_reason":null}]}),
            json!({"id":"chat_1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":3}}),
        ]
    };
    let mut sse = chunks
        .iter()
        .map(|v| format!("data: {v}\n\n"))
        .collect::<String>();
    sse.push_str("data: [DONE]\n\n");
    ([("content-type", "text/event-stream")], sse)
}
fn request(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("session-id", "synthetic-tool-roundtrip")
        .body(Body::from(body.to_string()))
        .unwrap()
}
fn events(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
#[tokio::test]
async fn a_pinned_codex_tool_call_executes_and_replays_on_the_same_chat_route() {
    roundtrip(false).await;
}

#[tokio::test]
async fn a_pinned_namespaced_tool_restores_identity_and_replays_on_the_same_chat_route() {
    roundtrip(true).await;
}

async fn roundtrip(namespaced: bool) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(chat))
                .with_state(seen.clone()),
        )
        .into_future(),
    );
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("synthetic-key")),
            Some(base),
            vec![("abliteration/abliterated-model".into(), ModelTier::Frontier)],
            30,
        )
        .unwrap()
        .responses_via_chat(true),
    ));
    let state = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "synthetic-control".into(),
    );
    state
        .policy
        .lock()
        .unwrap()
        .set_pin(Some(BackendId::from("nearai")), None);
    let proxy = app(state);
    let mut tools = json!([{"type":"function","name":"probe_echo","description":"Return a marker","parameters":{"type":"object","properties":{"marker":{"type":"string"}},"required":["marker"],"additionalProperties":false},"strict":true}]);
    if namespaced {
        tools = json!([{"type":"namespace", "name":"multi_agent_v1", "description":"Synthetic agent tools", "tools":tools}]);
    }
    let user = json!({"role":"user","content":"Call probe_echo with synthetic"});
    let choice = if namespaced {
        json!({"type":"function","namespace":"multi_agent_v1","name":"probe_echo"})
    } else {
        json!("required")
    };
    let first = json!({"model":"abliteration/abliterated-model","stream":true,"input":[user.clone()],"tools":tools.clone(),"tool_choice":choice,"parallel_tool_calls":false});
    let response = proxy.clone().oneshot(request(first)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let events = events(&text);
    let call = events
        .iter()
        .find(|e| e["type"] == "response.output_item.done" && e["item"]["type"] == "function_call")
        .expect("an executable function call")["item"]
        .clone();
    assert_eq!(call["name"], "probe_echo");
    if namespaced {
        assert_eq!(call["namespace"], "multi_agent_v1");
    } else {
        assert!(call.get("namespace").is_none());
    }
    let args: Value = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args, json!({"marker":"synthetic"}));
    // Execute the harmless declared tool; feed its actual result back.
    let output = args["marker"].as_str().unwrap().to_owned();
    let second = json!({"model":"abliteration/abliterated-model","stream":true,"input":[user,call.clone(),{"type":"function_call_output","call_id":call["call_id"],"output":output}],"tools":tools,"tool_choice":"auto","parallel_tool_calls":false});
    let response = proxy.oneshot(request(second)).await.unwrap();
    let status = response.status();
    let text = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("verified"), "{text}");
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    if namespaced {
        assert_eq!(
            requests[0]["tool_choice"],
            json!({"type":"function","function":{"name":"iw_namespace_0"}})
        );
    } else {
        assert_eq!(requests[0]["tool_choice"], "required");
    }
    assert_eq!(requests[0]["tools"][0]["function"]["strict"], true);
    assert_eq!(requests[0]["parallel_tool_calls"], false);
    let upstream_name = if namespaced {
        "iw_namespace_0"
    } else {
        "probe_echo"
    };
    assert_eq!(requests[0]["tools"][0]["function"]["name"], upstream_name);
    let messages = requests[1]["messages"].as_array().unwrap();
    assert!(messages.iter().any(
        |m| m["role"] == "assistant" && m["tool_calls"][0]["function"]["name"] == upstream_name
    ));
    assert!(messages.iter().any(|m| m["role"] == "tool"
        && m["tool_call_id"] == "call_probe"
        && m["content"] == "synthetic"));
    server.abort();
}
