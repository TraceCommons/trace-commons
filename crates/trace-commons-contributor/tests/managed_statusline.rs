//! The hidden `statusline` subcommand, run as Claude Code runs it: the
//! built executable, a running managed route's home, and Claude Code's
//! document on stdin.

use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};
use trace_commons_contributor::{
    config::ConfigStore,
    managed::{ToolId, route::ManagedRoute, secrets::ApiKey},
};

const SERVED: &str = "anthropic/claude-sonnet-5-5";

/// The model Claude Code says it asked for reaches the line from stdin:
/// after a call served as one model, a document naming another shows the
/// substitution.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_status_line_reads_the_model_claude_code_sends_on_stdin() {
    async fn completion(Json(_): Json<Value>) -> Json<Value> {
        Json(json!({
            "id": "fixture", "object": "chat.completion", "created": 1, "model": SERVED,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello"},
                "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        }))
    }
    let models = json!({"data": [
        {"id": SERVED, "input_modalities": ["text"], "output_modalities": ["text"]},
    ]});
    let app = Router::new()
        .route(
            "/v1/models",
            get(move || {
                let models = models.clone();
                async move { Json(models) }
            }),
        )
        .route("/v1/chat/completions", post(completion));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let route = ManagedRoute::start_against(
        &store,
        uuid::Uuid::new_v4(),
        ApiKey::new("fixture-key".into()).unwrap(),
        &endpoint,
        ToolId::Claude,
        None,
    )
    .await
    .unwrap();

    // One call, so the route has a last model to compare against.
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = client
        .post(format!(
            "http://127.0.0.1:{}/anthropic/v1/messages",
            route.port()
        ))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({"model": SERVED, "max_tokens": 16,
            "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert!(result.status().is_success());
    let _ = result.bytes().await.unwrap();

    let home = route.home().to_path_buf();
    let config_dir = dir.path().join("cli-config");
    let run = move |document: &'static str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_near-ai"))
            .arg("--config-dir")
            .arg(&config_dir)
            .arg("statusline")
            .arg("--home")
            .arg(&home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(document.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    let line = tokio::task::spawn_blocking({
        let run = run.clone();
        move || run(r#"{"model":{"id":"fixture/other-model"}}"#)
    })
    .await
    .unwrap();
    assert!(
        line.contains(&format!("({SERVED} for fixture/other-model)")),
        "{line:?}"
    );
    // A document naming the served model: nothing to point out.
    let line = tokio::task::spawn_blocking(move || {
        run(r#"{"model":{"id":"anthropic/claude-sonnet-5-5"}}"#)
    })
    .await
    .unwrap();
    assert!(line.starts_with("ironwire"), "{line:?}");
    assert!(!line.contains(" for "), "{line:?}");

    route.shutdown().await;
    server.abort();
}
