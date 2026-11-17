//! Conformance: several subscriptions to one product.
//!
//! A user with a personal and a work Claude login runs Claude Code twice, under
//! two `CLAUDE_CONFIG_DIR`s, and each is pointed at its own login-bound URL.
//! These tests pin what that buys:
//!
//! - a client is served by its own login, with that login's token, and the
//!   request that leaves is byte-for-byte the one the plain façade would have
//!   sent — the account segment is IronWire's and is never forwarded;
//! - when its own login is spent it descends the ordinary ladder, never onto
//!   the user's other subscription, unless the user turned on cross-account
//!   fallback — and then the move is recorded as the credential change it is;
//! - a URL naming a login IronWire does not have is refused, in the façade's
//!   own error shape, before anything reaches a provider;
//! - a default login moved with `config_dir` is reached only through its bound
//!   URL — the plain façade belongs to the client still on the default
//!   directory;
//! - each login has its own consent, prompt and connect command, and consent
//!   answers the directory it was given for, not the id;
//! - a Codex session's encrypted reasoning goes only to the ChatGPT account
//!   that minted it, through that account's failures and whatever other
//!   sessions on the same login do.
//!
//! Credentials are fixture files in temporary directories, read without the
//! macOS Keychain, so nothing here depends on how the machine running it is
//! logged in.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::{Config, PathsConfig};
use ironwire_core::policy::Rung;
use ironwire_core::protocol::BackendId;
use ironwire_core::quota::QuotaSnapshot;
use ironwire_creds::consent::CONSENT_PROMPT_VERSION;
use ironwire_creds::{ClaudeLogin, ConsentLedger};
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use ironwire_upstream::openai_responses::ResponsesBackend;
use ironwire_upstream::{Backend, Observation, UpstreamError};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const TOKEN: &str = "test-token";

const PERSONAL_TOKEN: &str = "sk-ant-oat01-personal";
const WORK_TOKEN: &str = "sk-ant-oat01-work";

// ------------------------------------------------------------------ upstream

/// One request a mock upstream saw.
#[derive(Debug, Clone)]
struct Received {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Received {
    /// Every value of one header, in order. A header sent twice shows up twice.
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The headers, minus the ones a credential decides.
    fn headers_without(&self, names: &[&str]) -> Vec<(String, String)> {
        self.headers
            .iter()
            .filter(|(k, _)| !names.contains(&k.as_str()))
            .cloned()
            .collect()
    }
}

/// What a mock answers with.
#[derive(Clone)]
struct Reply {
    status: u16,
    /// Header lines beyond the framing, each ending `\r\n`.
    extra_headers: String,
    content_type: &'static str,
    body: String,
    /// Answer to a `GET`, which is only ever a models fetch here.
    models: String,
}

/// A refusal with `status`, carrying `extra_headers` (each line ending
/// `\r\n`).
fn failing(status: u16, extra_headers: &str) -> Reply {
    Reply {
        status,
        extra_headers: extra_headers.to_string(),
        content_type: "application/json",
        body: r#"{"error":{"message":"no"}}"#.to_string(),
        models: r#"{"data":[]}"#.to_string(),
    }
}

fn anthropic_reply() -> Reply {
    Reply {
        status: 200,
        extra_headers: String::new(),
        content_type: "application/json",
        body: serde_json::json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-6",
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 3, "output_tokens": 1},
        })
        .to_string(),
        models: r#"{"data":[]}"#.to_string(),
    }
}

/// A Responses stream in the framing ChatGPT sends, and a models document
/// naming `model`, so two logins' catalogues can be told apart.
fn chatgpt_reply(model: &str) -> Reply {
    Reply {
        status: 200,
        extra_headers: String::new(),
        content_type: "text/event-stream",
        body: concat!(
            "event: response.created\n",
            r#"data: {"type":"response.created","response":{"id":"resp_1","model":"gpt-5.6"}}"#,
            "\n\n",
            "event: response.completed\n",
            r#"data: {"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":11,"output_tokens":3}}}"#,
            "\n\n",
        )
        .to_string(),
        models: serde_json::json!({"models": [{"slug": model}]}).to_string(),
    }
}

/// A hand-rolled HTTP/1.1 upstream that records every request it is sent.
///
/// Hand-rolled so the byte comparisons below are about what IronWire sent, not
/// about what a server framework chose to normalise on the way in.
async fn spawn_mock(reply: Reply) -> (String, Arc<Mutex<Vec<Received>>>) {
    let (url, seen, _) = spawn_switchable_mock(reply).await;
    (url, seen)
}

/// [`spawn_mock`], with a handle that changes what it answers from the next
/// request on — an upstream that starts failing mid-conversation, and later
/// recovers.
async fn spawn_switchable_mock(
    reply: Reply,
) -> (String, Arc<Mutex<Vec<Received>>>, Arc<Mutex<Reply>>) {
    let current = Arc::new(Mutex::new(reply));
    let answer = Arc::clone(&current);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let sink = Arc::clone(&sink);
            let reply = answer.lock().expect("lock").clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let received = loop {
                    let Ok(n) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let split = split + 4;
                    let head = String::from_utf8_lossy(&buf[..split]).to_string();
                    let length = head
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if buf.len() - split < length {
                        continue;
                    }
                    let mut lines = head.lines();
                    let request_line = lines.next().unwrap_or_default().to_string();
                    let headers = lines
                        .filter_map(|line| line.split_once(": "))
                        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                        .collect();
                    break Received {
                        request_line,
                        headers,
                        body: String::from_utf8_lossy(&buf[split..split + length]).to_string(),
                    };
                };
                let is_get = received.request_line.starts_with("GET ");
                sink.lock().expect("lock").push(received);

                let (status, extra, content_type, body) = if is_get {
                    (200, String::new(), "application/json", reply.models)
                } else {
                    (
                        reply.status,
                        reply.extra_headers,
                        reply.content_type,
                        reply.body,
                    )
                };
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\n{extra}\
                     content-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (format!("http://{addr}"), seen, current)
}

fn seen(mock: &Arc<Mutex<Vec<Received>>>) -> Vec<Received> {
    mock.lock().expect("lock").clone()
}

// --------------------------------------------------------------- credentials

/// A temporary home holding a default Claude login and a second one in
/// `~/.claude-work`, each a credential file of the shape Claude Code writes.
struct ClaudeLogins {
    home: tempfile::TempDir,
}

impl ClaudeLogins {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("tempdir");
        let logins = Self { home };
        write_claude_credentials(&logins.home.path().join(".claude"), PERSONAL_TOKEN);
        write_claude_credentials(&logins.work_dir(), WORK_TOKEN);
        logins
    }

    fn work_dir(&self) -> PathBuf {
        self.home.path().join(".claude-work")
    }

    /// The backend for the default login, `claude-sub`.
    fn personal(&self, base_url: &str) -> AnthropicBackend {
        claude_backend("claude-sub", self.home.path(), None, base_url)
    }

    /// The backend for the login in `~/.claude-work`, `claude-work`.
    fn work(&self, base_url: &str) -> AnthropicBackend {
        claude_backend(
            "claude-work",
            self.home.path(),
            Some(&self.work_dir()),
            base_url,
        )
    }
}

fn write_claude_credentials(dir: &Path, token: &str) {
    std::fs::create_dir_all(dir).expect("mkdir");
    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(8)).timestamp_millis();
    std::fs::write(
        dir.join(".credentials.json"),
        serde_json::json!({
            "claudeAiOauth": {
                "accessToken": token,
                "refreshToken": "sk-ant-ort01-unused",
                "expiresAt": expires_at,
                "subscriptionType": "max",
            }
        })
        .to_string(),
    )
    .expect("write credentials");
}

/// A Claude subscription backend for one login, named the way registration
/// names it, reading its file and never this machine's Keychain.
fn claude_backend(id: &str, home: &Path, dir: Option<&Path>, base_url: &str) -> AnthropicBackend {
    let login = ClaudeLogin::with_home(
        dir.map(|dir| dir.display().to_string()),
        Some(home.to_path_buf()),
    )
    .without_keychain();
    let name = login.describe(id == "claude-sub").label();
    AnthropicBackend::subscription_for(
        BackendId::from(id),
        name,
        login,
        Some(base_url.to_string()),
        30,
    )
    .expect("client builds")
}

fn metered_key(base_url: &str) -> AnthropicBackend {
    AnthropicBackend::api_key(
        SecretString::from("sk-ant-api03-metered"),
        Some(base_url.to_string()),
        30,
    )
    .expect("client builds")
}

/// NEAR AI as a daemon registers it on a machine with no key for it.
fn nearai_without_a_key() -> ChatCompletionsBackend {
    ChatCompletionsBackend::nearai(None, Some("http://127.0.0.1:9".to_string()), Vec::new(), 30)
        .expect("client builds")
}

fn exhausted<B: Backend>(backend: B) -> B {
    backend.record(&Observation {
        retry_after_secs: Some(3600),
        ..Observation::default()
    });
    backend
}

/// A ChatGPT access token carrying the account claim. Structurally a JWT,
/// cryptographically meaningless: the issuer verifies it, not IronWire.
fn chatgpt_token(account_id: &str) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": {"chatgpt_account_id": account_id},
        "exp": 4_000_000_000u64,
    });
    format!(
        "{}.{}.{}",
        engine.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        engine.encode(payload.to_string()),
        engine.encode(b"inert"),
    )
}

/// A Codex home logged in to `account_id`, as Codex writes it.
fn codex_home(account_id: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("auth.json"),
        serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": chatgpt_token(account_id),
                "access_token": chatgpt_token(account_id),
                "refresh_token": "rt.1.EXAMPLE",
                "account_id": account_id,
            },
            "last_refresh": "2026-07-30T04:20:00Z",
        })
        .to_string(),
    )
    .expect("write auth.json");
    dir
}

fn codex_default(home: &Path, base_url: &str) -> ResponsesBackend {
    ResponsesBackend::codex_subscription_at(
        Some(home.join("auth.json")),
        Some(base_url.to_string()),
        30,
    )
    .expect("client builds")
}

fn codex_work(home: &Path, base_url: &str) -> ResponsesBackend {
    ResponsesBackend::codex_subscription_for(
        BackendId::from("codex-work"),
        "ChatGPT subscription (codex-work)".to_string(),
        Some(home.to_path_buf()),
        Some(base_url.to_string()),
        30,
    )
    .expect("client builds")
}

// --------------------------------------------------------------------- state

fn state_with(backends: Vec<Arc<dyn Backend>>, config: Config, consented: &[&str]) -> AppState {
    let mut registry = BackendRegistry::new();
    for backend in backends {
        registry.push(backend);
    }
    // A subscription is off until the user says yes (TRUST.md §2); these tests
    // are about what happens after they did — to the login each id reads.
    let mut consent = ConsentLedger::default();
    for id in consented {
        let login = registry
            .get(&BackendId::from(*id))
            .and_then(|backend| backend.login());
        consent.grant_for(
            id,
            login.as_ref().and_then(|login| login.consent_location()),
            chrono::Utc::now(),
        );
    }
    AppState::new(registry, config, consent, TOKEN.to_string())
}

fn with_fallback() -> Config {
    let mut config = Config::default();
    config.subscriptions.cross_account_fallback = true;
    config
}

// ------------------------------------------------------------------ requests

/// A Claude Code turn: its own system block, its own user agent, and a
/// `metadata.user_id` naming the account it is logged in as — which is why a
/// request belongs on its own login's token.
const CLAUDE_CODE_BODY: &str = concat!(
    r#"{"model":"claude-opus-4-6","max_tokens":64,"#,
    r#""system":[{"type":"text","text":"You are Claude Code, Anthropic's official CLI for Claude."}],"#,
    r#""messages":[{"role":"user","content":"hi"}],"#,
    r#""metadata":{"user_id":"{\"account_uuid\":\"7d0e0000-work\"}"}}"#,
);

fn claude_code(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("user-agent", "claude-cli/2.1.286 (external, cli)")
        .header("anthropic-version", "2023-06-01")
        .header(
            "x-claude-code-session-id",
            "5db811ed-ce4a-45a7-ab00-56890e111668",
        )
        .body(Body::from(CLAUDE_CODE_BODY))
        .expect("request builds")
}

/// A Codex turn that replays encrypted reasoning from an earlier one.
const CODEX_BODY: &str = concat!(
    r#"{"model":"gpt-5.6","stream":true,"#,
    r#""tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}],"#,
    r#""input":[{"type":"reasoning","id":"rs_1","encrypted_content":"gAAAAA"},"#,
    r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
);

/// The first turn of a Codex conversation: nothing replayed yet, and the same
/// tools — so the same conversation key — as [`CODEX_BODY`].
const CODEX_FIRST_TURN_BODY: &str = concat!(
    r#"{"model":"gpt-5.6","stream":true,"#,
    r#""tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}],"#,
    r#""input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
);

const CODEX_SESSION: &str = "01998f6e-0000-7000-8000-000000000000";

fn codex(uri: &str) -> Request<Body> {
    codex_turn(uri, CODEX_SESSION, CODEX_BODY)
}

/// A Codex request in `session`, which Codex names in its `session-id`
/// header.
fn codex_turn(uri: &str, session: &str, body: &'static str) -> Request<Body> {
    codex_turn_naming(uri, "session-id", session, body)
}

/// [`codex_turn`], naming the session in `header` — `session_id` is how older
/// Codex spells it.
fn codex_turn_naming(
    uri: &str,
    header: &'static str,
    session: &str,
    body: &'static str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("originator", "codex_cli_rs")
        .header(header, session)
        .body(Body::from(body))
        .expect("request builds")
}

/// Send, drain, and hand back the status, the backend that served it and the
/// body.
async fn send(state: &AppState, request: Request<Body>) -> (StatusCode, String, serde_json::Value) {
    let response = app(state.clone()).oneshot(request).await.expect("served");
    let status = response.status();
    let backend = response
        .headers()
        .get("x-ironwire-backend")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, backend, body)
}

async fn control(
    state: &AppState,
    method: &str,
    path: &str,
    body: Option<String>,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(method)
        .uri(format!("/_ironwire{path}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, Body::from))
        .expect("request builds");
    let response = app(state.clone()).oneshot(request).await.expect("handled");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

// ------------------------------------------------------------- Claude routing

/// The binding in one test: the plain façade is the default login's, the bound
/// URL is the work login's, and the two requests that leave differ in the
/// token and nothing else — same path, same headers, same body bytes.
#[tokio::test]
async fn a_bound_url_changes_which_login_pays_and_nothing_else_on_the_wire() {
    let logins = ClaudeLogins::new();
    // One upstream for both, so even the `host` header has to match.
    let (url, upstream) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![Arc::new(logins.personal(&url)), Arc::new(logins.work(&url))],
        Config::default(),
        &["claude-sub", "claude-work"],
    );

    let (status, backend, _) = send(&state, claude_code("/anthropic/v1/messages")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        backend, "claude-sub",
        "the plain façade is the default login's"
    );

    let (status, backend, _) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "claude-work", "the bound URL is its own login's");

    let requests = seen(&upstream);
    assert_eq!(requests.len(), 2);
    let (plain, bound) = (&requests[0], &requests[1]);
    assert_eq!(
        plain.values("authorization"),
        vec![format!("Bearer {PERSONAL_TOKEN}").as_str()]
    );
    assert_eq!(
        bound.values("authorization"),
        vec![format!("Bearer {WORK_TOKEN}").as_str()]
    );
    assert_eq!(
        bound.request_line, plain.request_line,
        "the account segment is IronWire's and is never forwarded"
    );
    assert_eq!(bound.request_line, "POST /v1/messages HTTP/1.1");
    assert_eq!(
        bound.body, CLAUDE_CODE_BODY,
        "the body is the client's bytes"
    );
    assert_eq!(bound.body, plain.body);
    assert_eq!(
        bound.headers_without(&["authorization"]),
        plain.headers_without(&["authorization"]),
        "only the credential may differ"
    );
    // Two sessions of one Claude Code version share a system prompt and tool
    // set, so on two logins they are still two conversations, each with its
    // own sticky route — not one route the two keep taking from each other.
    assert_eq!(
        state.policy.lock().expect("policy").tracked_conversations(),
        2
    );
}

/// `count_tokens` is how Claude Code decides when to compact, so a bound Claude
/// Code needs it at its own URL — counted by its own login.
#[tokio::test]
async fn count_tokens_is_served_at_the_bound_url_by_that_login() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(logins.work(&work_url)),
        ],
        Config::default(),
        &["claude-sub", "claude-work"],
    );

    let (status, backend, _) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages/count_tokens"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "claude-work");
    assert!(seen(&personal).is_empty());
    let got = seen(&work);
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].request_line,
        "POST /v1/messages/count_tokens HTTP/1.1"
    );
}

/// By default a spent login descends the ordinary ladder. The user's other
/// subscription is right there and is not used: switching accounts after a 429
/// is what both providers' terms describe as circumventing a limit.
#[tokio::test]
async fn a_spent_login_falls_to_the_metered_key_not_to_the_other_subscription() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let (key_url, key) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(exhausted(logins.work(&work_url))),
            Arc::new(metered_key(&key_url)),
        ],
        Config::default(),
        &["claude-sub", "claude-work"],
    );

    let (status, backend, _) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "anthropic-key");
    assert!(
        seen(&personal).is_empty(),
        "the other subscription must not have been dialled"
    );
    assert!(seen(&work).is_empty(), "the spent login is not retried");
    let got = seen(&key);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].values("x-api-key"), vec!["sk-ant-api03-metered"]);
    assert_eq!(got[0].body, CLAUDE_CODE_BODY);
    assert_eq!(
        state.last_route().expect("routed").rung,
        Rung::AlternateCredential,
        "falling to a metered key is still a credential change"
    );
}

/// With nothing but the other subscription left, the client is told its own is
/// spent — in the shape its back-off already handles — rather than being moved.
#[tokio::test]
async fn a_spent_login_with_only_the_other_subscription_left_is_told_it_is_spent() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(exhausted(logins.work(&work_url))),
        ],
        Config::default(),
        &["claude-sub", "claude-work"],
    );

    let (status, _, body) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(seen(&personal).is_empty());
    assert!(seen(&work).is_empty());
}

/// Whether the other login was ever consented to makes no difference: it could
/// not have served this client either way, so the answer is still that the
/// client's own subscription is spent — not a 503 telling the user to enable a
/// login that would then be refused as someone else's.
#[tokio::test]
async fn a_spent_login_beside_an_unconsented_one_is_still_told_it_is_spent() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(exhausted(logins.work(&work_url))),
            Arc::new(nearai_without_a_key()),
        ],
        Config::default(),
        &["claude-work"],
    );

    let (status, _, body) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(seen(&personal).is_empty());
    assert!(seen(&work).is_empty());
}

/// Open `id`'s circuit the way failing requests do.
fn trip(state: &AppState, id: &str) {
    let id = BackendId::from(id);
    while state.breakers.allows(&id, chrono::Utc::now()) {
        state.breakers.record_failure(
            &id,
            &UpstreamError::Transport {
                backend: id.clone(),
                detail: "connection reset".to_string(),
            },
            chrono::Utc::now(),
        );
    }
}

/// A login whose circuit is open is still the client's own. Skipped as though
/// it were absent, the client on its bound URL read as one pointed at the wrong
/// login, and was told to rebind itself for as long as the circuit stayed
/// open — when all it has to do is wait.
#[tokio::test]
async fn a_login_whose_circuit_is_open_is_still_its_clients_and_the_client_is_told_to_wait() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(logins.work(&work_url)),
            Arc::new(nearai_without_a_key()),
        ],
        Config::default(),
        &["claude-sub", "claude-work"],
    );
    trip(&state, "claude-work");

    let (status, _, body) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(seen(&personal).is_empty());
    assert!(seen(&work).is_empty(), "an open circuit is not dialled");
}

/// Skipping a subscription for its open circuit is a credential change like
/// any other fall to a metered key. Recorded as rung 0, promotion never looked
/// again, and the conversation stayed on the key after the login recovered.
#[tokio::test]
async fn a_login_skipped_for_its_open_circuit_still_records_the_fall_as_rung_two() {
    let logins = ClaudeLogins::new();
    let (personal_url, _personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let (key_url, key) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(logins.work(&work_url)),
            Arc::new(metered_key(&key_url)),
        ],
        Config::default(),
        &["claude-sub", "claude-work"],
    );
    trip(&state, "claude-work");

    let (status, backend, _) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "anthropic-key");
    assert!(seen(&work).is_empty(), "an open circuit is not dialled");
    assert_eq!(seen(&key).len(), 1);
    assert_eq!(
        state.last_route().expect("routed").rung,
        Rung::AlternateCredential
    );
}

/// The opt-in. With `[subscriptions] cross_account_fallback = true` — read from
/// the config, not poked into the policy — the spent login's client is served
/// by the other subscription ahead of the metered key, on that login's own
/// token, and the ledger's rung says a credential changed.
#[tokio::test]
async fn with_cross_account_fallback_a_spent_login_is_served_by_the_other_at_rung_two() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let (key_url, key) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(exhausted(logins.work(&work_url))),
            Arc::new(metered_key(&key_url)),
        ],
        with_fallback(),
        &["claude-sub", "claude-work"],
    );

    let (status, backend, _) = send(
        &state,
        claude_code("/anthropic/account/claude-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "claude-sub");
    assert!(seen(&work).is_empty());
    assert!(
        seen(&key).is_empty(),
        "a subscription is still preferred to a key"
    );
    let got = seen(&personal);
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].values("authorization"),
        vec![format!("Bearer {PERSONAL_TOKEN}").as_str()],
        "the serving login's own token, never the client's"
    );
    assert_eq!(got[0].body, CLAUDE_CODE_BODY);
    assert_eq!(
        state.last_route().expect("routed").rung,
        Rung::AlternateCredential
    );
}

/// A client served on another of the user's logins is a credential change
/// even when its own login could never have served it — not yet consented
/// to, or not registered at all. Nothing was passed over, and the route is
/// still recorded and announced at rung 2: rung 0 would hide the very move the
/// opt-in makes, and stop promotion from ever looking for the way back.
#[tokio::test]
async fn a_request_served_by_another_login_is_recorded_at_rung_two_whatever_became_of_its_own() {
    let logins = ClaudeLogins::new();

    // Its own login is registered, and nobody said yes to it.
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let unconsented = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(logins.work(&work_url)),
        ],
        with_fallback(),
        &["claude-sub"],
    );
    // Its own login is not registered at all: the plain façade's client, on a
    // machine where only the work login is.
    let (only_url, only) = spawn_mock(anthropic_reply()).await;
    let absent = state_with(
        vec![Arc::new(logins.work(&only_url))],
        with_fallback(),
        &["claude-work"],
    );

    for (state, uri, served_by, what) in [
        (
            &unconsented,
            "/anthropic/account/claude-work/v1/messages",
            "claude-sub",
            "its own login unconsented",
        ),
        (
            &absent,
            "/anthropic/v1/messages",
            "claude-work",
            "its own login absent",
        ),
    ] {
        let mut events = state.events.subscribe();
        let (status, backend, body) = send(state, claude_code(uri)).await;
        assert_eq!(status, StatusCode::OK, "{what}: {body}");
        assert_eq!(backend, served_by, "{what}");
        assert_eq!(
            state.last_route().expect("routed").rung,
            Rung::AlternateCredential,
            "{what}: recorded as though the client were on its own login"
        );
        let announced =
            std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event {
                ironwire_proxy::events::Event::Routed { to, rung, .. } => Some((to, rung)),
                _ => None,
            });
        assert_eq!(
            announced,
            Some((served_by.to_string(), Rung::AlternateCredential)),
            "{what}: announced as though the client were on its own login"
        );
    }
    assert!(
        seen(&work).is_empty(),
        "an unconsented login is not dialled"
    );
    assert_eq!(seen(&personal).len(), 1);
    assert_eq!(seen(&only).len(), 1);
}

// ------------------------------------------------------------ bad bound URLs

/// A URL naming no login IronWire has is refused — never quietly served as the
/// default login, which would put one account's request on another's token —
/// and the refusal says what does exist and what to run. Nothing is sent.
#[tokio::test]
async fn a_bound_url_naming_no_such_login_is_refused_naming_the_fix() {
    let logins = ClaudeLogins::new();
    let chatgpt_home = codex_home("work-account");
    let (url, upstream) = spawn_mock(anthropic_reply()).await;
    let (chatgpt_url, chatgpt) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&url)),
            Arc::new(logins.work(&url)),
            Arc::new(metered_key(&url)),
            Arc::new(codex_work(chatgpt_home.path(), &chatgpt_url)),
        ],
        Config::default(),
        &["claude-sub", "claude-work", "codex-work"],
    );

    // A typo: the Anthropic envelope, 404, naming the logins that exist.
    let (status, _, body) = send(
        &state,
        claude_code("/anthropic/account/claude-wrok/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "not_found_error");
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("`claude-wrok`"), "{message}");
    assert!(
        message.contains("/anthropic/account/claude-sub")
            && message.contains("/anthropic/account/claude-work"),
        "it names what exists: {message}"
    );
    assert!(
        message.contains("ironwire connect claude --config-dir"),
        "and what to run: {message}"
    );

    // Another product's login, and a backend that is no login at all.
    let (status, _, body) = send(
        &state,
        claude_code("/anthropic/account/codex-work/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("ChatGPT login")),
        "{body}"
    );
    let (status, _, _) = send(
        &state,
        claude_code("/anthropic/account/anthropic-key/v1/messages/count_tokens"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The OpenAI façade refuses in OpenAI's envelope, on every route.
    let (status, _, body) = send(&state, codex("/openai/account/claude-work/v1/responses")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.get("type").is_none(), "not Anthropic's shape: {body}");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Claude login") && m.contains("/openai/account/codex-work")),
        "{body}"
    );
    let chat = Request::builder()
        .method("POST")
        .uri("/openai/account/nobody/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"gpt-5.6","messages":[]}"#))
        .expect("request builds");
    let (status, _, _) = send(&state, chat).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A segment that is not an identifier is described, not echoed into a
    // terminal.
    let (status, _, body) = send(
        &state,
        claude_code("/anthropic/account/%1B%5B2J/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        !body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains('\u{1b}'),
        "{body}"
    );

    // One that is not even UTF-8 once decoded never reaches the handler as a
    // string. It still gets the façade's own refusal, not the framework's
    // plain-text 400 — on both façades.
    let (status, _, body) = send(&state, claude_code("/anthropic/account/%FF/v1/messages")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["type"], "not_found_error", "{body}");
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("an account"), "{message}");
    assert!(
        message.contains("/anthropic/account/claude-work")
            && message.contains("ironwire connect claude --config-dir"),
        "{message}"
    );
    let (status, _, body) = send(&state, codex("/openai/account/%FF/v1/responses")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["type"], "invalid_request_error", "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("/openai/account/codex-work")),
        "{body}"
    );

    assert!(seen(&upstream).is_empty(), "nothing reached Anthropic");
    assert!(seen(&chatgpt).is_empty(), "nothing reached ChatGPT");
}

/// Every bound route, not only the two a client sends turns to, answers an
/// account segment that is not UTF-8 once decoded with its own façade's
/// refusal — never the framework's plain-text 400, which no client parses
/// and which names no login and no command.
#[tokio::test]
async fn every_bound_route_refuses_an_undecodable_account_in_its_facades_own_envelope() {
    let logins = ClaudeLogins::new();
    let chatgpt_home = codex_home("work-account");
    let (url, upstream) = spawn_mock(anthropic_reply()).await;
    let (chatgpt_url, chatgpt) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
    let state = state_with(
        vec![
            Arc::new(logins.work(&url)),
            Arc::new(codex_work(chatgpt_home.path(), &chatgpt_url)),
        ],
        Config::default(),
        &["claude-work", "codex-work"],
    );
    let get = |uri: &str| {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request builds")
    };
    let chat = Request::builder()
        .method("POST")
        .uri("/openai/account/%FF/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"gpt-5.6","messages":[]}"#))
        .expect("request builds");

    for (request, anthropic) in [
        (
            claude_code("/anthropic/account/%FF/v1/messages/count_tokens"),
            true,
        ),
        (get("/anthropic/account/%FF/v1/models"), true),
        (get("/openai/account/%FF/v1/models"), false),
        (chat, false),
    ] {
        let uri = request.uri().to_string();
        let (status, _, body) = send(&state, request).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
        let message = body["error"]["message"].as_str().unwrap_or_default();
        if anthropic {
            assert_eq!(body["type"], "error", "{uri}: {body}");
            assert_eq!(body["error"]["type"], "not_found_error", "{uri}: {body}");
            assert!(
                message.contains("/anthropic/account/claude-work")
                    && message.contains("ironwire connect claude --config-dir"),
                "{uri}: {message}"
            );
        } else {
            assert_eq!(
                body["error"]["type"], "invalid_request_error",
                "{uri}: {body}"
            );
            assert!(
                message.contains("/openai/account/codex-work")
                    && message.contains("ironwire connect codex --config-dir"),
                "{uri}: {message}"
            );
        }
    }
    assert!(seen(&upstream).is_empty(), "nothing reached Anthropic");
    assert!(seen(&chatgpt).is_empty(), "nothing reached ChatGPT");
}

/// The model routes are bound too: a bad account is refused there as well, and
/// a good one lists what that login's client could actually be served.
#[tokio::test]
async fn a_bound_model_list_offers_only_what_that_login_could_serve() {
    let logins = ClaudeLogins::new();
    let (url, _) = spawn_mock(anthropic_reply()).await;
    // Only the work login is consented: the plain façade's client — the
    // default login's — has nothing it could be served.
    let state = state_with(
        vec![Arc::new(logins.personal(&url)), Arc::new(logins.work(&url))],
        Config::default(),
        &["claude-work"],
    );
    let list = |uri: &str| {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request builds")
    };

    let (status, _, body) = send(&state, list("/anthropic/account/claude-work/v1/models")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body["data"].as_array().expect("a list").is_empty(),
        "the work login's models: {body}"
    );

    let (status, _, body) = send(&state, list("/anthropic/v1/models")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["data"].as_array().expect("a list").is_empty(),
        "another login's models would be offered and then refused: {body}"
    );

    let (status, _, body) = send(&state, list("/anthropic/account/claude-wrok/v1/models")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["type"], "not_found_error");
}

/// A Claude Code left on the plain URL while the only Claude login IronWire has
/// is another one is refused, and told the exact command that would bind it —
/// with that login's directory, which the router alone could not name.
///
/// NEAR AI is registered with no key, as every real daemon registers it. It is
/// unavailable, but it is no reason to tell this client to back off and retry:
/// nothing that could serve it is coming back.
#[tokio::test]
async fn a_client_on_the_wrong_url_is_told_the_command_that_binds_it() {
    let logins = ClaudeLogins::new();
    let (url, upstream) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.work(&url)),
            Arc::new(nearai_without_a_key()),
        ],
        Config::default(),
        &["claude-work"],
    );

    let (status, _, body) = send(&state, claude_code("/anthropic/v1/messages")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(message.contains("claude-work"), "{message}");
    assert!(
        message.contains("`ironwire connect claude --config-dir ~/.claude-work`"),
        "{message}"
    );
    assert!(seen(&upstream).is_empty());
}

// --------------------------------------------------------- a moved default

/// `config_dir` on `claude-sub` moves the default login to another directory —
/// here the one holding the work account's token. The Claude Code still using
/// `~/.claude` (the personal account) keeps calling the plain façade, so the
/// plain façade cannot be the moved login's: that would send the personal
/// account's request on the work account's token with fallback off. The moved
/// login is reached through its bound URL instead, like any additional login.
#[tokio::test]
async fn a_moved_default_login_serves_its_bound_url_and_never_the_plain_one() {
    let logins = ClaudeLogins::new();
    let (sub_url, sub) = spawn_mock(anthropic_reply()).await;
    let (key_url, key) = spawn_mock(anthropic_reply()).await;
    let moved = || {
        claude_backend(
            "claude-sub",
            logins.home.path(),
            Some(&logins.work_dir()),
            &sub_url,
        )
    };
    let state = state_with(
        vec![Arc::new(moved()), Arc::new(metered_key(&key_url))],
        Config::default(),
        &["claude-sub"],
    );

    let (status, backend, _) = send(&state, claude_code("/anthropic/v1/messages")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        backend, "anthropic-key",
        "the plain façade's client has no subscription of its own any more"
    );
    assert!(
        seen(&sub).is_empty(),
        "the moved login's token carried the plain façade's request"
    );
    assert_eq!(seen(&key).len(), 1);

    let (status, backend, _) = send(
        &state,
        claude_code("/anthropic/account/claude-sub/v1/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "claude-sub", "its bound URL is its own");
    let got = seen(&sub);
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].values("authorization"),
        vec![format!("Bearer {WORK_TOKEN}").as_str()]
    );

    // With nothing else to fall to, the plain façade's client is told what
    // binds it — not served by the moved login, and not told to back off.
    let alone = state_with(
        vec![Arc::new(moved()), Arc::new(nearai_without_a_key())],
        Config::default(),
        &["claude-sub"],
    );
    let (status, _, body) = send(&alone, claude_code("/anthropic/v1/messages")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let message = body["error"]["message"].as_str().expect("a message");
    assert!(
        message.contains("`ironwire connect claude --config-dir ~/.claude-work`"),
        "{message}"
    );
    assert_eq!(seen(&sub).len(), 1, "still only the bound request");

    // And its models are offered where it would serve them, not elsewhere.
    let list = |uri: &str| {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request builds")
    };
    let (_, _, body) = send(&alone, list("/anthropic/v1/models")).await;
    assert!(
        body["data"].as_array().expect("a list").is_empty(),
        "the plain façade would be refused what it offers: {body}"
    );
    let (_, _, body) = send(&alone, list("/anthropic/account/claude-sub/v1/models")).await;
    assert!(
        !body["data"].as_array().expect("a list").is_empty(),
        "{body}"
    );
}

/// The same for Codex: a `codex-sub` moved to another `CODEX_HOME` serves the
/// Codex bound to it, and never the one still using `~/.codex` at the plain
/// façade.
#[tokio::test]
async fn a_moved_default_codex_login_serves_only_its_bound_url() {
    let moved_home = codex_home("moved-account");
    let (url, upstream) = spawn_mock(chatgpt_reply("gpt-moved-plan")).await;
    let state = state_with(
        vec![
            Arc::new(
                ResponsesBackend::codex_subscription_for(
                    BackendId::from("codex-sub"),
                    "ChatGPT subscription".to_string(),
                    Some(moved_home.path().to_path_buf()),
                    Some(url.clone()),
                    30,
                )
                .expect("client builds"),
            ),
            Arc::new(nearai_without_a_key()),
        ],
        Config::default(),
        &["codex-sub"],
    );

    let (status, _, body) = send(&state, codex("/openai/v1/responses")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("ironwire connect codex --config-dir")),
        "{body}"
    );
    assert!(seen(&upstream).is_empty());

    let (status, _, body) = send(
        &state,
        codex_turn(
            "/openai/account/codex-sub/v1/responses",
            CODEX_SESSION,
            CODEX_FIRST_TURN_BODY,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, backend, _) = send(&state, codex("/openai/account/codex-sub/v1/responses")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "codex-sub");
    let got = seen(&upstream);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].values("chatgpt-account-id"), vec!["moved-account"]);

    // Its catalogue is not offered to the plain façade's Codex either.
    let models = |uri: &str| {
        Request::builder()
            .uri(uri)
            .header("originator", "codex_cli_rs")
            .body(Body::empty())
            .expect("request builds")
    };
    let (_, _, body) = send(&state, models("/openai/v1/models")).await;
    assert!(body.get("models").is_none(), "{body}");
    let (_, _, body) = send(&state, models("/openai/account/codex-sub/v1/models")).await;
    assert_eq!(body["models"][0]["slug"], "gpt-moved-plan", "{body}");
}

// ------------------------------------------------------------------- Codex

/// Two `CODEX_HOME`s. The bound URL's request carries that home's bearer and
/// that home's account id — never the other's — and is otherwise the request
/// the plain façade sends for the default home.
#[tokio::test]
async fn a_bound_codex_request_carries_its_own_homes_bearer_and_account() {
    let personal = codex_home("personal-account");
    let work = codex_home("work-account");
    let (url, upstream) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
    let state = state_with(
        vec![
            Arc::new(codex_default(personal.path(), &url)),
            Arc::new(codex_work(work.path(), &url)),
        ],
        Config::default(),
        &["codex-sub", "codex-work"],
    );

    for uri in [
        "/openai/v1/responses",
        "/openai/account/codex-work/v1/responses",
    ] {
        let (status, _, body) = send(
            &state,
            codex_turn(uri, CODEX_SESSION, CODEX_FIRST_TURN_BODY),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, backend, _) = send(&state, codex("/openai/v1/responses")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "codex-sub");
    let (status, backend, _) = send(&state, codex("/openai/account/codex-work/v1/responses")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        backend, "codex-work",
        "its own login, encrypted reasoning and all"
    );

    let requests = seen(&upstream);
    assert_eq!(requests.len(), 4);
    let (plain, bound) = (&requests[2], &requests[3]);
    assert_eq!(plain.values("chatgpt-account-id"), vec!["personal-account"]);
    assert_eq!(
        plain.values("authorization"),
        vec![format!("Bearer {}", chatgpt_token("personal-account")).as_str()]
    );
    assert_eq!(bound.values("chatgpt-account-id"), vec!["work-account"]);
    assert_eq!(
        bound.values("authorization"),
        vec![format!("Bearer {}", chatgpt_token("work-account")).as_str()]
    );
    assert_eq!(bound.request_line, plain.request_line);
    assert!(
        !bound.request_line.contains("account"),
        "{}",
        bound.request_line
    );
    assert_eq!(bound.body, CODEX_BODY);
    assert_eq!(bound.body, plain.body);
    assert_eq!(
        bound.headers_without(&["authorization", "chatgpt-account-id"]),
        plain.headers_without(&["authorization", "chatgpt-account-id"]),
    );
}

/// Codex reads its model catalogue from the provider's own document, and two
/// ChatGPT plans can carry different models. Each login's Codex is shown its
/// own login's.
#[tokio::test]
async fn each_codex_login_is_shown_its_own_model_catalogue() {
    let personal = codex_home("personal-account");
    let work = codex_home("work-account");
    let (personal_url, _) = spawn_mock(chatgpt_reply("gpt-personal-plan")).await;
    let (work_url, _) = spawn_mock(chatgpt_reply("gpt-work-plan")).await;
    let state = state_with(
        vec![
            Arc::new(codex_default(personal.path(), &personal_url)),
            Arc::new(codex_work(work.path(), &work_url)),
        ],
        Config::default(),
        &["codex-sub", "codex-work"],
    );
    let models = |uri: &str| {
        Request::builder()
            .uri(uri)
            .header("originator", "codex_cli_rs")
            .body(Body::empty())
            .expect("request builds")
    };

    let (status, _, body) = send(&state, models("/openai/account/codex-work/v1/models")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"][0]["slug"], "gpt-work-plan", "{body}");

    let (status, _, body) = send(&state, models("/openai/v1/models")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"][0]["slug"], "gpt-personal-plan", "{body}");
}

/// Which ChatGPT account minted a conversation's encrypted reasoning is not the
/// route the pipeline forgets when a backend fails. With fallback on and the
/// client's own login spent, a conversation starts on the sibling; the own
/// login's window then resets, and the sibling answers a reasoning-carrying
/// turn with a 429, a 5xx or a 401. The own login must not be handed reasoning
/// it never minted — OpenAI would refuse it, and the conversation would then be
/// stuck there — and once the sibling recovers it serves the conversation
/// again.
#[tokio::test]
async fn a_failing_sibling_never_hands_its_reasoning_to_the_clients_own_login() {
    for (failure, what) in [
        (failing(429, "retry-after: 3600\r\n"), "a 429"),
        (failing(500, ""), "a 500"),
        (failing(401, ""), "a 401"),
    ] {
        let own_home = codex_home("own-account");
        let work_home = codex_home("work-account");
        let (own_url, own) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
        let (work_url, work, answer) = spawn_switchable_mock(chatgpt_reply("gpt-5.6")).await;
        let state = state_with(
            vec![
                Arc::new(exhausted(codex_default(own_home.path(), &own_url))),
                Arc::new(codex_work(work_home.path(), &work_url)),
            ],
            with_fallback(),
            &["codex-sub", "codex-work"],
        );
        let get = |id: &str| {
            state
                .backends
                .get(&BackendId::from(id))
                .expect("registered")
                .clone()
        };

        let (status, backend, _) = send(
            &state,
            codex_turn("/openai/v1/responses", CODEX_SESSION, CODEX_FIRST_TURN_BODY),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(backend, "codex-work", "the own login is spent");
        assert_eq!(
            state.last_route().expect("routed").rung,
            Rung::AlternateCredential
        );

        get("codex-sub").restore_quota(QuotaSnapshot::default());
        *answer.lock().expect("lock") = failure;
        let (status, backend, body) = send(&state, codex("/openai/v1/responses")).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "{what}: served by {backend}: {body}"
        );
        assert!(
            seen(&own).is_empty(),
            "on {what}, the own login was handed the sibling's reasoning"
        );

        *answer.lock().expect("lock") = chatgpt_reply("gpt-5.6");
        get("codex-work").restore_quota(QuotaSnapshot::default());
        state
            .breakers
            .record_success(&BackendId::from("codex-work"));
        let (status, backend, _) = send(&state, codex("/openai/v1/responses")).await;
        assert_eq!(status, StatusCode::OK, "after {what}");
        assert_eq!(
            backend, "codex-work",
            "after {what}, the account that minted the reasoning serves again"
        );
        assert!(seen(&own).is_empty(), "after {what}");
        assert!(seen(&work).len() >= 3, "{what}: the sibling saw every turn");
    }
}

/// Every Codex session of one version shares a conversation key, so which
/// account minted the reasoning is kept per session, by the id Codex sends in
/// `session-id` — or, from an older Codex, `session_id`. One session landing
/// on the sibling login moves no other session's reasoning there.
#[tokio::test]
async fn one_codex_session_moving_to_another_login_takes_no_other_sessions_reasoning_with_it() {
    const FIRST: &str = "01998f6e-0000-7000-8000-00000000000a";
    const SECOND: &str = "01998f6e-0000-7000-8000-00000000000b";
    for header in ["session-id", "session_id"] {
        let own_home = codex_home("own-account");
        let work_home = codex_home("work-account");
        let (own_url, own) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
        let (work_url, work) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
        let state = state_with(
            vec![
                Arc::new(codex_default(own_home.path(), &own_url)),
                Arc::new(codex_work(work_home.path(), &work_url)),
            ],
            with_fallback(),
            &["codex-sub", "codex-work"],
        );
        let turn = |session: &str, body: &'static str| {
            codex_turn_naming("/openai/v1/responses", header, session, body)
        };

        let (status, backend, _) = send(&state, turn(FIRST, CODEX_FIRST_TURN_BODY)).await;
        assert_eq!(status, StatusCode::OK, "{header}");
        assert_eq!(backend, "codex-sub", "{header}");

        // The own login runs out, and the second session starts on the sibling.
        state
            .backends
            .get(&BackendId::from("codex-sub"))
            .expect("registered")
            .record(&Observation {
                retry_after_secs: Some(3600),
                ..Observation::default()
            });
        let (status, backend, _) = send(&state, turn(SECOND, CODEX_FIRST_TURN_BODY)).await;
        assert_eq!(status, StatusCode::OK, "{header}");
        assert_eq!(backend, "codex-work", "{header}");

        // The first session's reasoning is the own login's, wherever the
        // shared key's route went.
        let (status, backend, body) = send(&state, turn(FIRST, CODEX_BODY)).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "{header}: served by {backend}: {body}"
        );
        assert_eq!(
            seen(&work).len(),
            1,
            "{header}: the first session's reasoning reached the sibling"
        );

        // The second session's is the sibling's.
        let (status, backend, _) = send(&state, turn(SECOND, CODEX_BODY)).await;
        assert_eq!(status, StatusCode::OK, "{header}");
        assert_eq!(backend, "codex-work", "{header}");
        assert_eq!(
            seen(&own).len(),
            1,
            "{header}: only the first session's first turn"
        );
    }
}

/// Pins obey the same encrypted-origin gate as automatic routing.
#[tokio::test]
async fn a_pin_cannot_send_encrypted_reasoning_to_another_account() {
    let own_home = codex_home("own-account");
    let work_home = codex_home("work-account");
    let (own_url, own) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
    let (work_url, work) = spawn_mock(Reply {
        body: r#"{"error":{"code":"invalid_encrypted_content","message":"no"}}"#.to_string(),
        ..failing(400, "")
    })
    .await;
    let state = state_with(
        vec![
            Arc::new(codex_default(own_home.path(), &own_url)),
            Arc::new(codex_work(work_home.path(), &work_url)),
        ],
        with_fallback(),
        &["codex-sub", "codex-work"],
    );

    let (status, backend, _) = send(
        &state,
        codex_turn("/openai/v1/responses", CODEX_SESSION, CODEX_FIRST_TURN_BODY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "codex-sub");

    let (status, _) = control(
        &state,
        "POST",
        "/pin",
        Some(r#"{"backend":"codex-work"}"#.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(&state, codex("/openai/v1/responses")).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "the proxy refuses before another account sees encrypted history"
    );
    assert!(
        seen(&work).is_empty(),
        "the other account never receives the replay"
    );

    let (status, _) = control(
        &state,
        "POST",
        "/pin",
        Some(r#"{"backend":null}"#.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, backend, body) = send(&state, codex("/openai/v1/responses")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        backend, "codex-sub",
        "the pinned turn that failed was taken as having moved the reasoning"
    );
    assert_eq!(seen(&own).len(), 2);
    assert!(seen(&work).is_empty());
}

// ------------------------------------------- requests IronWire makes itself

/// `doctor`'s probe sends each backend's credential, a subscription's OAuth
/// token included. One nobody has said yes to is reported unprobed, with the
/// command that grants it, and its provider never hears from IronWire.
#[tokio::test]
async fn a_probe_never_sends_the_token_of_a_login_awaiting_consent() {
    let logins = ClaudeLogins::new();
    let (personal_url, personal) = spawn_mock(anthropic_reply()).await;
    let (work_url, work) = spawn_mock(anthropic_reply()).await;
    let state = state_with(
        vec![
            Arc::new(logins.personal(&personal_url)),
            Arc::new(logins.work(&work_url)),
        ],
        Config::default(),
        &["claude-sub"],
    );

    let (status, body) = control(&state, "POST", "/probe", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        seen(&work).is_empty(),
        "the unconsented login's token was sent"
    );
    let probed = seen(&personal);
    assert_eq!(probed.len(), 1, "the consented login is still probed");
    assert!(probed[0].request_line.starts_with("GET /v1/models "));

    let view = |id: &str| {
        body.as_array()
            .expect("a list")
            .iter()
            .find(|view| view["id"] == id)
            .unwrap_or_else(|| panic!("{id} is reported: {body}"))
            .clone()
    };
    let sub = view("claude-sub");
    assert_eq!(sub["ok"], true, "{sub}");
    assert_eq!(sub["probed"], true, "{sub}");
    let waiting = view("claude-work");
    assert_eq!(waiting["ok"], false, "{waiting}");
    assert_eq!(waiting["probed"], false, "{waiting}");
    assert_eq!(waiting["awaiting_consent"], true, "{waiting}");
    let command = waiting["connect_command"].as_str().expect("a command");
    assert!(
        command.starts_with("ironwire connect claude --config-dir "),
        "{command}"
    );
    let error = waiting["error"].as_str().expect("says why");
    assert!(
        error.contains("consent") && error.contains(&format!("`{command} --subscription`")),
        "{error}"
    );

    // A daemon that predates the field probed everything it reported.
    let old: ironwire_proxy::control::ProbeView = serde_json::from_value(serde_json::json!({
        "id": "claude-sub", "name": "Claude subscription", "ok": true,
        "latency_ms": 12, "error": null,
    }))
    .expect("an older daemon's answer still reads");
    assert!(old.probed && !old.awaiting_consent && old.connect_command.is_none());
}

/// Learning catalogues at startup is a request per backend that no user
/// request asked for. A login awaiting consent is skipped; a consented one is
/// still asked.
///
/// Registered in config order with the unconsented login first, so by the
/// time the consented one's provider hears anything, the decision about the
/// other has been made.
#[tokio::test]
async fn startup_discovery_never_sends_the_token_of_a_login_awaiting_consent() {
    use ironwire_proxy::embed::{EmbedOptions, StartupProbes, start_with_options};
    let waiting_home = codex_home("waiting-account");
    let allowed_home = codex_home("allowed-account");
    let (waiting_url, waiting) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
    let (allowed_url, allowed) = spawn_mock(chatgpt_reply("gpt-5.6")).await;
    let dir = |home: &tempfile::TempDir| home.path().display().to_string();

    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "[updates]\ncheck = false\n\
             [[backends]]\nid = 'nearai'\nkind = 'nearai'\nenabled = false\n\
             [[backends]]\nid = 'claude-sub'\nkind = 'claude-subscription'\nenabled = false\n\
             [[backends]]\nid = 'codex-sub'\nkind = 'codex-subscription'\nenabled = false\n\
             [[backends]]\nid = 'anthropic-key'\nkind = 'anthropic-api'\nenabled = false\n\
             [[backends]]\nid = 'openai-key'\nkind = 'openai-api'\nenabled = false\n\
             [[backends]]\nid = 'codex-waiting'\nkind = 'codex-subscription'\n\
             config_dir = '{}'\nbase_url = '{waiting_url}'\n\
             [[backends]]\nid = 'codex-allowed'\nkind = 'codex-subscription'\n\
             config_dir = '{}'\nbase_url = '{allowed_url}'\n",
            dir(&waiting_home),
            dir(&allowed_home),
        ),
    )
    .expect("write config");
    let mut consent = ConsentLedger::default();
    consent.grant_for(
        "codex-allowed",
        Some(&dir(&allowed_home)),
        chrono::Utc::now(),
    );
    consent
        .save(&PathsConfig::rooted_at(home.path().to_path_buf()).consent_file())
        .expect("write consent");

    let proxy = start_with_options(
        home.path(),
        Some(0),
        EmbedOptions::default().with_startup_probes(StartupProbes::All),
        |_, _| {},
    )
    .await
    .expect("starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while seen(&allowed).is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    proxy.shutdown().await;

    assert!(
        !seen(&allowed).is_empty(),
        "the consented login is still asked for its catalogue"
    );
    assert!(
        seen(&waiting).is_empty(),
        "the login awaiting consent was sent a request with its token"
    );
}

// ------------------------------------------------------------------ control

fn control_state(logins: &ClaudeLogins, home: &Path, config: Config) -> AppState {
    let url = "http://127.0.0.1:9";
    state_with(
        vec![Arc::new(logins.personal(url)), Arc::new(logins.work(url))],
        config,
        &[],
    )
    .with_paths(PathsConfig::rooted_at(home))
}

/// The second login is asked about by name. "Enable the Claude subscription"
/// does not say which account is being handed over once there are two.
#[tokio::test]
async fn a_second_login_has_its_own_consent_question_naming_its_directory() {
    let logins = ClaudeLogins::new();
    let daemon_home = tempfile::tempdir().expect("tempdir");
    let state = control_state(&logins, daemon_home.path(), Config::default());

    let (status, body) = control(&state, "GET", "/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    let service = |id: &str| {
        body["services"]
            .as_array()
            .expect("services")
            .iter()
            .find(|s| s["id"] == id)
            .unwrap_or_else(|| panic!("{id} is listed: {body}"))
            .clone()
    };

    let work = service("claude-work");
    assert_eq!(work["requires_consent"], true);
    assert_eq!(work["consent_prompt"]["backend_id"], "claude-work");
    assert_eq!(work["consent_prompt"]["version"], CONSENT_PROMPT_VERSION);
    for field in ["summary", "question"] {
        assert!(
            work["consent_prompt"][field]
                .as_str()
                .is_some_and(|text| text.contains("~/.claude-work")),
            "the {field} names the login: {work}"
        );
    }
    assert_eq!(
        work["connect_command"],
        "ironwire connect claude --config-dir ~/.claude-work"
    );
    assert_eq!(work["name"], "Claude subscription (~/.claude-work)");

    let personal = service("claude-sub");
    let default_login = state
        .backends
        .get(&BackendId::from("claude-sub"))
        .and_then(|b| b.login())
        .expect("login");
    let default_prompt =
        ironwire_creds::consent::ConsentPrompt::for_login(&default_login, "claude-sub");
    assert_eq!(
        personal["consent_prompt"],
        serde_json::to_value(&default_prompt).expect("serialises"),
        "the default login keeps the question its users already answered"
    );
    assert_eq!(personal["connect_command"], "ironwire connect claude");
}

/// Consent is per login. Saying yes to the work account says nothing about the
/// personal one, in either direction.
#[tokio::test]
async fn consent_for_a_second_login_leaves_the_first_alone() {
    let logins = ClaudeLogins::new();
    let daemon_home = tempfile::tempdir().expect("tempdir");
    let state = control_state(&logins, daemon_home.path(), Config::default());
    let grant = |id: &str, granted: bool| Some(consent_answer(&state, id, granted));

    // Asked the way routing asks: of the login each id reads.
    let consented = |id: &str| {
        state
            .backends
            .consented(&BackendId::from(id), &state.consent_snapshot())
    };

    let (status, body) = control(&state, "POST", "/consent", grant("claude-work", true)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(consented("claude-work"));
    assert!(
        !consented("claude-sub"),
        "granting one login granted the other"
    );

    let (status, _) = control(&state, "POST", "/consent", grant("claude-sub", true)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = control(&state, "POST", "/consent", grant("claude-work", false)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!consented("claude-work"));
    assert!(
        consented("claude-sub"),
        "withdrawing one login withdrew the other"
    );

    // And written down per login.
    let saved = std::fs::read_to_string(daemon_home.path().join("consent.json")).expect("reads");
    assert!(
        saved.contains("claude-sub") && !saved.contains("claude-work"),
        "{saved}"
    );
}

/// Consent answers the login it was given for, not the backend id. `config_dir`
/// keeps `claude-sub`'s id and changes its account, so a yes recorded while it
/// read `~/.claude` — every consent.json written before logins could move — is
/// no yes to the account it reads now. Until the new directory is asked about,
/// the login is not routed to, not offered, and shown as needing consent; the
/// answer is then recorded for that directory, and it serves.
#[tokio::test]
async fn moving_a_login_withdraws_its_consent_until_its_new_directory_is_asked_about() {
    let logins = ClaudeLogins::new();
    let daemon_home = tempfile::tempdir().expect("tempdir");
    let (url, upstream) = spawn_mock(anthropic_reply()).await;
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(claude_backend(
        "claude-sub",
        logins.home.path(),
        Some(&logins.work_dir()),
        &url,
    )));
    let mut before_the_move = ConsentLedger::default();
    before_the_move.grant("claude-sub", chrono::Utc::now());
    let state = AppState::new(
        registry,
        Config::default(),
        before_the_move,
        TOKEN.to_string(),
    )
    .with_paths(PathsConfig::rooted_at(daemon_home.path()));
    let bound = "/anthropic/account/claude-sub/v1/messages";

    let (status, _, body) = send(&state, claude_code(bound)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("claude-sub: not enabled")),
        "{body}"
    );
    assert!(seen(&upstream).is_empty());

    let models = Request::builder()
        .uri("/anthropic/account/claude-sub/v1/models")
        .body(Body::empty())
        .expect("request builds");
    let (_, _, body) = send(&state, models).await;
    assert!(
        body["data"].as_array().expect("a list").is_empty(),
        "{body}"
    );

    let (_, body) = control(&state, "GET", "/settings", None).await;
    let service = body["services"]
        .as_array()
        .expect("services")
        .iter()
        .find(|s| s["id"] == "claude-sub")
        .expect("listed")
        .clone();
    assert_eq!(service["consented"], false, "{service}");
    assert!(
        service["consent_prompt"]["question"]
            .as_str()
            .is_some_and(|q| q.contains("~/.claude-work")),
        "the question names the directory it is about: {service}"
    );
    let (_, body) = control(&state, "GET", "/status", None).await;
    let listed = body["backends"]
        .as_array()
        .expect("backends")
        .iter()
        .find(|b| b["id"] == "claude-sub")
        .expect("listed")
        .clone();
    assert_eq!(listed["consented"], false, "{listed}");
    assert_eq!(
        body["balance"]["subscription_used"],
        serde_json::json!([]),
        "{body}"
    );

    let (status, body) = control(
        &state,
        "POST",
        "/consent",
        Some(consent_answer(&state, "claude-sub", true)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ledger = state.consent_snapshot();
    let work_dir = logins.work_dir().display().to_string();
    assert!(ledger.is_granted_for("claude-sub", Some(&work_dir)));
    assert!(
        !ledger.is_granted("claude-sub"),
        "a yes to the moved login is not a yes to ~/.claude"
    );

    let (status, backend, _) = send(&state, claude_code(bound)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(backend, "claude-sub");
    assert_eq!(seen(&upstream).len(), 1);
}

/// For Codex the same, including the provider's own model document: a moved
/// `codex-sub` whose consent answered `~/.codex` hands its catalogue to no one.
#[tokio::test]
async fn a_moved_codex_login_with_an_old_consent_is_neither_routed_to_nor_offered() {
    let moved_home = codex_home("moved-account");
    let (url, upstream) = spawn_mock(chatgpt_reply("gpt-moved-plan")).await;
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        ResponsesBackend::codex_subscription_for(
            BackendId::from("codex-sub"),
            "ChatGPT subscription".to_string(),
            Some(moved_home.path().to_path_buf()),
            Some(url.clone()),
            30,
        )
        .expect("client builds"),
    ));
    let mut before_the_move = ConsentLedger::default();
    before_the_move.grant("codex-sub", chrono::Utc::now());
    let state = AppState::new(
        registry,
        Config::default(),
        before_the_move,
        TOKEN.to_string(),
    );

    let (status, _, body) = send(&state, codex("/openai/account/codex-sub/v1/responses")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let models = Request::builder()
        .uri("/openai/account/codex-sub/v1/models")
        .header("originator", "codex_cli_rs")
        .body(Body::empty())
        .expect("request builds");
    let (_, _, body) = send(&state, models).await;
    assert!(body.get("models").is_none(), "{body}");
    assert_eq!(body["data"], serde_json::json!([]), "{body}");
    assert!(seen(&upstream).is_empty(), "not even for its catalogue");
}

/// An id that is no registered login has no question to answer, so there is
/// nothing to record consent to.
#[tokio::test]
async fn consent_for_a_login_that_is_not_registered_is_refused() {
    let logins = ClaudeLogins::new();
    let daemon_home = tempfile::tempdir().expect("tempdir");
    let state = control_state(&logins, daemon_home.path(), Config::default());
    let (status, body) = control(
        &state,
        "POST",
        "/consent",
        Some(format!(
            r#"{{"backend":"claude-play","granted":true,"prompt_version":{CONSENT_PROMPT_VERSION}}}"#
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = body["error"].as_str().expect("an error");
    assert!(error.contains("claude-play"), "{error}");
    assert!(
        error.contains("claude-sub") && error.contains("claude-work"),
        "it names the logins consent does apply to: {error}"
    );
    assert!(!state.consent_snapshot().is_granted("claude-play"));
}

/// Two logins of one product are two rows everywhere status shows them, told
/// apart by id rather than by a display name they might share.
#[tokio::test]
async fn status_tells_two_logins_apart_and_says_when_fallback_is_on() {
    let logins = ClaudeLogins::new();
    let daemon_home = tempfile::tempdir().expect("tempdir");
    let state = control_state(&logins, daemon_home.path(), Config::default());
    state.set_consent("claude-sub", true).expect("granted");
    state.set_consent("claude-work", true).expect("granted");

    let (status, body) = control(&state, "GET", "/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["cross_account_fallback"], false);

    let used = body["balance"]["subscription_used"]
        .as_array()
        .expect("rows");
    let ids: Vec<&str> = used
        .iter()
        .map(|row| row["backend"].as_str().expect("an id"))
        .collect();
    assert_eq!(ids, vec!["claude-sub", "claude-work"]);
    assert_ne!(used[0]["name"], used[1]["name"], "{used:?}");

    let work = body["backends"]
        .as_array()
        .expect("backends")
        .iter()
        .find(|b| b["id"] == "claude-work")
        .expect("listed")
        .clone();
    assert_eq!(
        work["connect_command"],
        "ironwire connect claude --config-dir ~/.claude-work"
    );

    let on = control_state(&logins, daemon_home.path(), with_fallback());
    let (_, body) = control(&on, "GET", "/status", None).await;
    assert_eq!(
        body["cross_account_fallback"], true,
        "a risk the user took on is never off the status screen"
    );
}

fn consent_answer(state: &AppState, id: &str, granted: bool) -> String {
    let login = state
        .backends
        .get(&BackendId::from(id))
        .and_then(|backend| backend.login())
        .expect("login");
    let prompt = ironwire_creds::consent::ConsentPrompt::for_login(&login, id);
    serde_json::json!({"backend":id,"granted":granted,"prompt_version":prompt.version,"binding":prompt.binding}).to_string()
}

#[tokio::test]
async fn a_prompt_from_before_a_login_moved_cannot_authorize_its_new_directory() {
    let logins = ClaudeLogins::new();
    let daemon_home = tempfile::tempdir().expect("home");
    let old = control_state(&logins, daemon_home.path(), Config::default());
    let answer = consent_answer(&old, "claude-sub", true);
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(claude_backend(
        "claude-sub",
        logins.home.path(),
        Some(&logins.work_dir()),
        "http://127.0.0.1:9",
    )));
    let restarted = AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        TOKEN.to_string(),
    )
    .with_paths(PathsConfig::rooted_at(daemon_home.path()));
    let (status, _) = control(&restarted, "POST", "/consent", Some(answer)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(restarted.consent_snapshot().granted().is_empty());
    assert!(!daemon_home.path().join("consent.json").exists());
    let (status, _) = control(
        &restarted,
        "POST",
        "/consent",
        Some(consent_answer(&restarted, "claude-sub", true)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(restarted.backends.consented(
        &BackendId::from("claude-sub"),
        &restarted.consent_snapshot()
    ));
    let (status, _) = control(
        &restarted,
        "POST",
        "/consent",
        Some(r#"{"backend":"claude-sub","granted":false,"prompt_version":0}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(restarted.consent_snapshot().granted().is_empty());
}
