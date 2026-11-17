//! Actual provider backends at scale, using only fixture credentials and loopback.
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use bytes::Bytes;
use futures_util::{StreamExt, stream};
use ironwire_core::config::Config;
use ironwire_core::protocol::BackendId;
use ironwire_creds::{ClaudeLogin, ConsentLedger};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::Backend;
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_responses::ResponsesBackend;
use tower::ServiceExt;

const EACH_PRODUCT: usize = 512;
const CLAUDE_BODY: &[u8] = br#"{"model":"claude-opus-4-6","stream":true,"max_tokens":64,"system":[{"type":"text","text":"You are Claude Code, Anthropic's official CLI for Claude."}],"messages":[{"role":"user","content":"hi"}],"unknown_provider_field":{"preserve":true}}"#;
const CODEX_FIRST: &[u8] = br#"{"model":"gpt-5.6","stream":true,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}],"unknown_provider_field":{"preserve":true}}"#;
const CODEX_REPLAY: &[u8] = br#"{"model":"gpt-5.6","stream":true,"input":[{"type":"reasoning","id":"rs_1","encrypted_content":"gAAAAA-synthetic"},{"type":"message","role":"user","content":[{"type":"input_text","text":"again"}]}],"unknown_provider_field":{"preserve":true}}"#;

#[derive(Clone)]
struct Recorded {
    authorization: String,
    account: Option<String>,
    session: String,
    body: Bytes,
}
type Recordings = Arc<Mutex<Vec<Recorded>>>;
async fn upstream(State(records): State<Recordings>, headers: HeaderMap, body: Bytes) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let account = header("chatgpt-account-id");
    let session = header("session-id")
        .or_else(|| header("x-claude-code-session-id"))
        .expect("client session forwarded");
    let response = if account.is_some() {
        "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_scale\"}}\n\n"
    } else {
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    };
    records.lock().unwrap().push(Recorded {
        authorization: header("authorization").expect("provider auth"),
        account,
        session,
        body,
    });
    ([("content-type", "text/event-stream")], response).into_response()
}

fn codex_token(account: &str) -> String {
    let payload = serde_json::json!({"exp": (chrono::Utc::now() + chrono::Duration::days(30)).timestamp(),
        "https://api.openai.com/auth": {"chatgpt_account_id": account}});
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "{}.{}.{}",
        engine.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        engine.encode(payload.to_string()),
        engine.encode(b"inert")
    )
}
fn write_claude(dir: &Path, token: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let data = serde_json::json!({"claudeAiOauth": {"accessToken": token,
        "refreshToken": "synthetic-refresh", "expiresAt": (chrono::Utc::now() + chrono::Duration::days(30)).timestamp_millis(), "subscriptionType": "max"}});
    std::fs::write(dir.join(".credentials.json"), data.to_string()).unwrap();
}
fn write_codex(dir: &Path, account: &str, token: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let data = serde_json::json!({"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
        "tokens": {"id_token": token, "access_token": token, "refresh_token": "synthetic-refresh", "account_id": account}});
    std::fs::write(dir.join("auth.json"), data.to_string()).unwrap();
}

async fn turn(state: AppState, codex: bool, n: usize, replay: bool) {
    let (product, prefix, endpoint, body) = if codex {
        (
            "codex",
            "openai",
            "responses",
            if replay { CODEX_REPLAY } else { CODEX_FIRST },
        )
    } else {
        ("claude", "anthropic", "messages", CLAUDE_BODY)
    };
    let id = format!("{product}-{n}");
    let session = format!("scale-{product}-{n}");
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/{prefix}/account/{id}/v1/{endpoint}"))
        .header("content-type", "application/json");
    if codex {
        request = request
            .header("originator", "codex_cli_rs")
            .header("session-id", session);
    } else {
        request = request
            .header("user-agent", "claude-cli/2.1.286 (external, cli)")
            .header("anthropic-version", "2023-06-01")
            .header("x-claude-code-session-id", session);
    }
    let response = app(state)
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{id}, replay={replay}");
    assert_eq!(
        response
            .headers()
            .get("x-ironwire-backend")
            .unwrap()
            .to_str()
            .unwrap(),
        id
    );
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(!body.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thousand_real_login_backends_keep_credentials_and_encrypted_sessions_separate() {
    let start = Instant::now();
    let records = Recordings::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(
        axum::serve(
            listener,
            axum::Router::new()
                .fallback(upstream)
                .with_state(records.clone()),
        )
        .into_future(),
    );
    let fixture = tempfile::tempdir().unwrap();
    let mut registry = BackendRegistry::new();
    let mut consent = ConsentLedger::default();
    let mut expected_auth = HashMap::new();
    for n in 0..EACH_PRODUCT {
        let claude_id = format!("claude-{n}");
        let claude_dir = fixture.path().join(&claude_id);
        let token = format!("sk-ant-oat01-fake-{n}");
        write_claude(&claude_dir, &token);
        let login = ClaudeLogin::with_home(
            Some(claude_dir.display().to_string()),
            Some(fixture.path().to_path_buf()),
        )
        .without_keychain();
        let backend = AnthropicBackend::subscription_for(
            BackendId::from(claude_id.as_str()),
            claude_id.clone(),
            login,
            Some(base.clone()),
            30,
        )
        .unwrap();
        consent.grant_for(
            &claude_id,
            backend.login().unwrap().consent_location(),
            chrono::Utc::now(),
        );
        expected_auth.insert(format!("scale-claude-{n}"), format!("Bearer {token}"));
        registry.push(Arc::new(backend));
        let codex_id = format!("codex-{n}");
        let codex_dir = fixture.path().join(&codex_id);
        let account = format!("synthetic-account-{n}");
        let token = codex_token(&account);
        write_codex(&codex_dir, &account, &token);
        let backend = ResponsesBackend::codex_subscription_for(
            BackendId::from(codex_id.as_str()),
            codex_id.clone(),
            Some(codex_dir),
            Some(base.clone()),
            30,
        )
        .unwrap();
        consent.grant_for(
            &codex_id,
            backend.login().unwrap().consent_location(),
            chrono::Utc::now(),
        );
        expected_auth.insert(format!("scale-codex-{n}"), format!("Bearer {token}"));
        registry.push(Arc::new(backend));
    }
    eprintln!(
        "1024 actual subscription backends and fixture files built in {:?}",
        start.elapsed()
    );
    let statuses = registry.statuses().await;
    assert_eq!(statuses.len(), 2 * EACH_PRODUCT);
    assert!(
        statuses.iter().all(|status| status.authenticated),
        "unusable fixture logins: {:?}",
        statuses
            .iter()
            .filter(|status| !status.authenticated)
            .take(4)
            .map(|status| (&status.id, &status.detail))
            .collect::<Vec<_>>()
    );
    let scopes: HashSet<_> = statuses
        .iter()
        .filter_map(|status| status.account_scope.as_ref())
        .collect();
    assert_eq!(
        scopes.len(),
        EACH_PRODUCT,
        "each account has its own observed scope"
    );
    let candidates = registry.candidates(&statuses, &consent);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.consented && candidate.healthy)
    );
    let mut wrong_location = ConsentLedger::default();
    wrong_location.grant_for("claude-0", Some("/wrong-login"), chrono::Utc::now());
    assert!(!registry.consented(&BackendId::from("claude-0"), &wrong_location));
    // Diagnostic replay of the same acceptance fixture, without maintaining
    // a second giant test: IRONWIRE_SCALE_FULLSCAN=1 benchmarks the opt-in.
    let mut config = Config::default();
    config.subscriptions.cross_account_fallback =
        std::env::var_os("IRONWIRE_SCALE_FULLSCAN").is_some();
    let state = AppState::new(registry, config, consent, "scale-control-token".into());
    stream::iter((0..EACH_PRODUCT).flat_map(|n| [(false, n), (true, n)]))
        .map(|(codex, n)| turn(state.clone(), codex, n, false))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
    eprintln!("1024 first facade turns completed at {:?}", start.elapsed());
    stream::iter(0..EACH_PRODUCT)
        .map(|n| turn(state.clone(), true, n, true))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
    let records = records.lock().unwrap();
    assert_eq!(records.len(), EACH_PRODUCT * 3);
    let mut counts = HashMap::new();
    for record in records.iter() {
        assert_eq!(record.authorization, expected_auth[&record.session]);
        let n: usize = record.session.rsplit('-').next().unwrap().parse().unwrap();
        if record.session.starts_with("scale-codex-") {
            assert_eq!(
                record.account.as_deref(),
                Some(format!("synthetic-account-{n}").as_str())
            );
            assert!(
                record.body.as_ref() == CODEX_FIRST || record.body.as_ref() == CODEX_REPLAY,
                "native Codex body changed"
            );
        } else {
            assert!(record.account.is_none());
            assert_eq!(record.body.as_ref(), CLAUDE_BODY);
        }
        *counts.entry(record.session.clone()).or_insert(0usize) += 1;
    }
    for n in 0..EACH_PRODUCT {
        assert_eq!(counts[&format!("scale-claude-{n}")], 1);
        assert_eq!(counts[&format!("scale-codex-{n}")], 2);
    }
    eprintln!(
        "all 1536 real-backend turns including512 encrypted replays completed in {:?}",
        start.elapsed()
    );
    server.abort();
}
