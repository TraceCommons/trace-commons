//! Process-level coverage for contributor content-control flags.

use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::post};
use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::identity::DeviceIdentity;

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

fn issuer() -> Router {
    Router::new().route(
        "/v1/trace-upload-claim",
        post(|| async {
            Json(serde_json::json!({
                "access_token": "compiled-cli-test-claim",
                "token_type": "Bearer",
                "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                "expires_in": 300,
                "consent_scopes": ["debugging_evaluation"],
                "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"]
            }))
        }),
    )
}

fn ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
    Router::new().route(
        "/v1/traces",
        post(move |Json(body): Json<serde_json::Value>| {
            let received = received.clone();
            async move {
                received.lock().unwrap().push(body);
                Json(serde_json::json!({
                    "status": "accepted",
                    "credit_points_pending": 0.0,
                    "explanation": []
                }))
            }
        }),
    )
}

fn trajectory_body() -> &'static str {
    r#"[
      {"role":"meta","source":"openhands"},
      {"role":"user","content":"message text","timestamp":"2026-07-31T12:00:00Z"},
      {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","name":"send-private-payroll-to-vendor","args":"{\"vendor\":\"acme\"}"}],"timestamp":"2026-07-31T12:00:01Z"},
      {"role":"tool","tool_call_id":"call_1","content":"tool output","timestamp":"2026-07-31T12:00:02Z"}
    ]"#
}

fn run_compiled_cli(flag: &str, issuer_url: &str, ingest_url: &str) {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let device = DeviceIdentity::load_or_generate(&store).unwrap();
    store
        .save_config(&ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: issuer_url.to_string(),
            ingest_url: ingest_url.to_string(),
            audience: "trace-commons-upload".to_string(),
            tenant_id: "tenant-cli-test".to_string(),
            instance_id: "instance-cli-test".to_string(),
            user_subject: "cli-test-user".to_string(),
            device_key_id: device.device_key_id,
            consent_scopes: vec!["debugging_evaluation".to_string()],
            pii_filter: None,
            allowed_hosts: Some("127.0.0.1".to_string()),
            include_message_text: true,
            include_tool_payloads: true,
        })
        .unwrap();
    let trajectory = dir.path().join("session.json");
    std::fs::write(&trajectory, trajectory_body()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"))
        .arg("--config-dir")
        .arg(store.dir())
        .arg("submit")
        .arg("--all")
        .arg("--yes")
        .arg("--source")
        .arg("trajectory")
        .arg("--trajectory")
        .arg(&trajectory)
        .arg(flag)
        .env_remove("TRACE_COMMONS_ALLOWED_HOSTS")
        .env_remove("TRACE_PRIVACY_FILTER_BACKEND")
        .output()
        .expect("run compiled contributor CLI");
    assert!(
        output.status.success(),
        "compiled CLI failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compiled_cli_content_flags_reach_wire_envelope() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let issuer_url = spawn(issuer()).await;
    let ingest_url = spawn(ingest(received.clone())).await;

    run_compiled_cli("--no-message-text", &issuer_url, &ingest_url);
    run_compiled_cli("--no-tool-payloads", &issuer_url, &ingest_url);

    let envelopes = received.lock().unwrap();
    assert_eq!(envelopes.len(), 2);
    let message_opt_out = &envelopes[0];
    assert_eq!(message_opt_out["consent"]["message_text_included"], false);
    assert_eq!(message_opt_out["consent"]["tool_payloads_included"], true);
    assert!(
        message_opt_out["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event_type"] == "tool_call" && !event["tool_name"].is_null())
    );

    let tool_opt_out = &envelopes[1];
    assert_eq!(tool_opt_out["consent"]["message_text_included"], true);
    assert_eq!(tool_opt_out["consent"]["tool_payloads_included"], false);
    assert!(
        tool_opt_out["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| {
                event["tool_name"].is_null()
                    && event["tool_category"].is_null()
                    && event["side_effect"]
                        == if event["event_type"] == "tool_call" {
                            "unknown"
                        } else {
                            "none"
                        }
            })
    );
}
