//! Each login is its own credential.
//!
//! One product can have several logins — Claude Code under several config
//! dirs, Codex under several homes — and each is its own backend. These pin,
//! against a recording mock, that a backend presents its own login's token and
//! account and nothing else (`docs/TRUST.md` I2, `AGENTS.md` rule 4), and that
//! a stale or wrong-mode login is refused before anything is sent, with the
//! command that fixes *that* login rather than some other one.
//!
//! Every Claude login here is read with `without_keychain`: the credential is a
//! planted file, and what this developer's Keychain holds must not decide the
//! outcome.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use ironwire_core::protocol::BackendId;
use ironwire_creds::{ClaudeLogin, CodexLogin, SubscriptionLogin, SubscriptionProduct};
use ironwire_upstream::anthropic::AnthropicBackend;
use ironwire_upstream::openai_responses::ResponsesBackend;
use ironwire_upstream::{Backend, UpstreamError, UpstreamRequest};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One request as the mock upstream saw it.
#[derive(Debug, Clone)]
struct Recorded {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Recorded {
    /// Every value sent for `name`, in order. A header sent twice shows up
    /// twice.
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

type Log = Arc<Mutex<Vec<Recorded>>>;

/// A loopback upstream that records every request and answers each with the
/// same canned response.
async fn spawn_mock(
    status: &'static str,
    content_type: &'static str,
    body: &'static str,
) -> (String, Log) {
    spawn_mock_with_location(status, content_type, body, None).await
}

async fn spawn_mock_with_location(
    status: &'static str,
    content_type: &'static str,
    body: &'static str,
    location: Option<String>,
) -> (String, Log) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let log = Log::default();
    let sink = Arc::clone(&log);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = socket.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(split) = find_head_end(&buf) {
                    let head = String::from_utf8_lossy(&buf[..split]).to_string();
                    let length = content_length(&head).unwrap_or(0);
                    if buf.len() - split >= length {
                        let mut lines = head.lines();
                        let request_line = lines.next().unwrap_or_default().to_string();
                        let headers = lines
                            .filter_map(|line| line.split_once(": "))
                            .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                            .collect();
                        sink.lock().expect("lock").push(Recorded {
                            request_line,
                            headers,
                            body: String::from_utf8_lossy(&buf[split..split + length]).to_string(),
                        });
                        break;
                    }
                }
            }
            let head = format!(
                "HTTP/1.1 {status}\r\n\
                 content-type: {content_type}\r\n\
                 {}\
                 connection: close\r\n\
                 content-length: {}\r\n\r\n",
                location
                    .as_ref()
                    .map_or_else(String::new, |url| format!("location: {url}\r\n")),
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    (format!("http://{addr}"), log)
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
}

fn recorded(log: &Log) -> Vec<Recorded> {
    log.lock().expect("lock").clone()
}

const ANTHROPIC_SSE: &str = concat!(
    "event: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

const CLAUDE_BODY: &str = r#"{"model":"claude-opus-4-6","stream":true,"messages":[{"role":"user","content":"hi"}],"metadata":{"user_id":"{\"account_uuid\":\"work\"}"}}"#;

fn claude_request() -> UpstreamRequest {
    UpstreamRequest {
        path: "/v1/messages".to_string(),
        body: Bytes::from_static(CLAUDE_BODY.as_bytes()),
        headers: vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
        ],
        stream: true,
        expected_account_scope: None,
    }
}

/// Milliseconds since the epoch, `offset` from now — the unit Claude Code
/// writes `expiresAt` in.
fn expires_in(offset: chrono::Duration) -> i64 {
    (chrono::Utc::now() + offset).timestamp_millis()
}

/// A Claude Code login made under `<home>/<name>`, holding `token`, read from
/// its file alone.
fn claude_login(home: &Path, name: &str, token: &str, expires_at_ms: i64) -> ClaudeLogin {
    let dir = home.join(name);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let credentials = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": token,
            "refreshToken": "sk-ant-ort01-EXAMPLE",
            "expiresAt": expires_at_ms,
            "subscriptionType": "max",
        }
    });
    std::fs::write(dir.join(".credentials.json"), credentials.to_string()).expect("write");
    ClaudeLogin::with_home(Some(dir.display().to_string()), Some(home.to_path_buf()))
        .without_keychain()
}

fn claude_backend(id: &str, login: ClaudeLogin, base_url: &str) -> AnthropicBackend {
    let name = login.describe(false).label();
    AnthropicBackend::subscription_for(
        BackendId::from(id),
        name,
        login,
        Some(base_url.to_string()),
        30,
    )
    .expect("client builds")
}

#[tokio::test]
async fn each_claude_login_sends_its_own_token_and_nothing_else() {
    let home = tempfile::tempdir().expect("tempdir");
    let hour = chrono::Duration::hours(1);
    let personal = claude_login(
        home.path(),
        ".claude-personal",
        "sk-ant-oat01-PERSONAL",
        expires_in(hour),
    );
    let work = claude_login(
        home.path(),
        ".claude-work",
        "sk-ant-oat01-WORK",
        expires_in(hour),
    );
    let (personal_url, personal_log) =
        spawn_mock("200 OK", "text/event-stream", ANTHROPIC_SSE).await;
    let (work_url, work_log) = spawn_mock("200 OK", "text/event-stream", ANTHROPIC_SSE).await;

    let personal = claude_backend("claude-personal", personal, &personal_url);
    let work = claude_backend("claude-work", work, &work_url);
    work.send(claude_request())
        .await
        .expect("work login serves");
    personal
        .send(claude_request())
        .await
        .expect("personal login serves");

    for (log, own, other) in [
        (&work_log, "sk-ant-oat01-WORK", "sk-ant-oat01-PERSONAL"),
        (&personal_log, "sk-ant-oat01-PERSONAL", "sk-ant-oat01-WORK"),
    ] {
        let seen = recorded(log);
        assert_eq!(seen.len(), 1);
        let got = &seen[0];
        assert!(
            got.request_line.starts_with("POST /v1/messages "),
            "{}",
            got.request_line
        );
        assert_eq!(
            got.values("authorization"),
            vec![format!("Bearer {own}").as_str()]
        );
        assert!(
            !got.headers.iter().any(|(_, v)| v.contains(other)),
            "another login's token reached this one's provider"
        );
        assert!(
            got.values("anthropic-beta")
                .iter()
                .any(|beta| beta.contains("oauth-2025-04-20")),
            "an OAuth bearer is refused without its beta flag: {:?}",
            got.values("anthropic-beta")
        );
        assert_eq!(got.body, CLAUDE_BODY, "the body is the client's bytes");
    }
}

#[tokio::test]
async fn an_expired_claude_token_is_refused_before_anything_is_sent() {
    // IronWire never refreshes another product's token, so a login whose Claude
    // Code has not run lately stays expired. Sending it would only earn a 401;
    // saying which command refreshes it is the useful answer.
    let home = tempfile::tempdir().expect("tempdir");
    let login = claude_login(
        home.path(),
        ".claude-work",
        "sk-ant-oat01-STALE",
        expires_in(-chrono::Duration::hours(1)),
    );
    let (url, log) = spawn_mock("200 OK", "text/event-stream", ANTHROPIC_SSE).await;
    let backend = claude_backend("claude-work", login, &url);

    let err = backend
        .send(claude_request())
        .await
        .expect_err("an expired token must not be presented");
    let UpstreamError::NeedsAuth { ref detail, .. } = err else {
        panic!("expected NeedsAuth, got {err:?}");
    };
    assert!(detail.contains("expired"), "{detail}");
    assert!(
        detail.contains("`CLAUDE_CONFIG_DIR=~/.claude-work claude`"),
        "{detail}"
    );
    assert!(
        err.is_retryable(),
        "the router must be free to try the next rung"
    );

    let err = backend.probe().await.expect_err("nor probed with");
    assert!(matches!(err, UpstreamError::NeedsAuth { .. }), "{err:?}");

    let status = backend.status().await;
    assert!(!status.authenticated, "an expired login is not usable");
    assert!(
        status
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("`CLAUDE_CONFIG_DIR=~/.claude-work claude`")),
        "{:?}",
        status.detail
    );

    assert!(recorded(&log).is_empty(), "nothing may reach the provider");
}

#[tokio::test]
async fn a_rejected_claude_token_names_the_login_that_holds_it() {
    let home = tempfile::tempdir().expect("tempdir");
    let login = claude_login(
        home.path(),
        ".claude-work",
        "sk-ant-oat01-REVOKED",
        expires_in(chrono::Duration::hours(1)),
    );
    let (url, log) = spawn_mock(
        "401 Unauthorized",
        "application/json",
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid token"}}"#,
    )
    .await;
    let backend = claude_backend("claude-work", login, &url);

    let err = backend.send(claude_request()).await.expect_err("rejected");
    let UpstreamError::NeedsAuth { detail, .. } = err else {
        panic!("expected NeedsAuth, got {err:?}");
    };
    assert!(detail.contains("for ~/.claude-work"), "{detail}");
    assert!(
        detail.contains("`CLAUDE_CONFIG_DIR=~/.claude-work claude`"),
        "{detail}"
    );
    assert_eq!(
        recorded(&log).len(),
        1,
        "re-reading found the same token, so it must not be sent twice"
    );
}

#[tokio::test]
async fn a_claude_login_with_no_credential_says_how_to_log_it_in() {
    // An additional login is registered whether or not anyone has logged in
    // to it yet; its status is where the user learns what to run.
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join(".claude-work")).expect("mkdir");
    let login = ClaudeLogin::with_home(
        Some(home.path().join(".claude-work").display().to_string()),
        Some(home.path().to_path_buf()),
    )
    .without_keychain();
    let backend = claude_backend("claude-work", login, "http://127.0.0.1:9");

    let status = backend.status().await;
    assert!(!status.authenticated);
    let detail = status.detail.unwrap_or_default();
    assert!(detail.contains("not logged in"), "{detail}");
    assert!(
        detail.contains("`CLAUDE_CONFIG_DIR=~/.claude-work claude`"),
        "{detail}"
    );
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

const RESPONSES_SSE: &str = concat!(
    "event: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_1"}}"#,
    "\n\n",
);

const CODEX_BODY: &str = r#"{"model":"gpt-5.6","stream":true,"input":[{"type":"message","role":"user","content":"hi"}]}"#;

/// A ChatGPT access token for `account_id`. Structurally a JWT; nothing in
/// IronWire checks the signature, and nothing should.
fn access_token_for(account_id: &str, exp: i64) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": {"chatgpt_account_id": account_id},
        "exp": exp,
    });
    format!(
        "{}.{}.{}",
        engine.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        engine.encode(payload.to_string()),
        engine.encode(b"inert"),
    )
}

/// A Codex home logged in with ChatGPT to `account_id`.
fn codex_home(account_id: &str, exp: i64) -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    let token = access_token_for(account_id, exp);
    let auth = serde_json::json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": token,
            "access_token": token,
            "refresh_token": "rt.1.EXAMPLE",
            "account_id": account_id,
        },
    });
    std::fs::write(home.path().join("auth.json"), auth.to_string()).expect("write");
    home
}

fn far_future() -> i64 {
    (chrono::Utc::now() + chrono::Duration::days(30)).timestamp()
}

#[tokio::test]
async fn a_refreshed_chatgpt_token_keeps_its_observed_account_scope() {
    let home = codex_home("account-original", far_future());
    let backend = codex_backend("codex-work", home.path(), "http://127.0.0.1:9");
    let first = backend
        .status()
        .await
        .account_scope
        .expect("observed account");
    let refreshed = codex_home("account-original", far_future() + 3600);
    std::fs::copy(
        refreshed.path().join("auth.json"),
        home.path().join("auth.json"),
    )
    .expect("refresh credential");
    assert_eq!(
        backend.status().await.account_scope.as_deref(),
        Some(first.as_str())
    );
    assert!(!first.contains("account-original"));
}

#[tokio::test]
async fn another_chatgpt_account_in_the_same_home_has_another_scope() {
    let home = codex_home("account-original", far_future());
    let backend = codex_backend("codex-work", home.path(), "http://127.0.0.1:9");
    let first = backend
        .status()
        .await
        .account_scope
        .expect("observed account");
    let replaced = codex_home("account-replacement", far_future());
    std::fs::copy(
        replaced.path().join("auth.json"),
        home.path().join("auth.json"),
    )
    .expect("replace credential");
    assert_ne!(backend.status().await.account_scope, Some(first));
    assert_eq!(backend.id(), &BackendId::from("codex-work"));
}

#[tokio::test]
async fn an_account_changed_after_routing_is_refused_before_its_token_is_sent() {
    let (base, log) = spawn_mock("200 OK", "text/event-stream", RESPONSES_SSE).await;
    let home = codex_home("account-original", far_future());
    let backend = codex_backend("codex-work", home.path(), &base);
    let mut request = codex_request(&[]);
    request.expected_account_scope = backend.status().await.account_scope;
    let replaced = codex_home("account-replacement", far_future());
    std::fs::copy(
        replaced.path().join("auth.json"),
        home.path().join("auth.json"),
    )
    .expect("replace credential");
    let error = backend
        .send(request)
        .await
        .expect_err("account drift refused");
    assert!(matches!(error, UpstreamError::NeedsAuth { .. }));
    assert!(error.to_string().contains("start a new session"));
    assert!(recorded(&log).is_empty());
}

fn codex_backend(id: &str, home: &Path, base_url: &str) -> ResponsesBackend {
    ResponsesBackend::codex_subscription_for(
        BackendId::from(id),
        format!("ChatGPT subscription ({id})"),
        Some(home.to_path_buf()),
        Some(base_url.to_string()),
        30,
    )
    .expect("client builds")
}

fn codex_request(extra: &[(&str, &str)]) -> UpstreamRequest {
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("originator".to_string(), "codex_cli_rs".to_string()),
    ];
    headers.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
    );
    UpstreamRequest {
        path: "/v1/responses".to_string(),
        body: Bytes::from_static(CODEX_BODY.as_bytes()),
        headers,
        stream: true,
        expected_account_scope: None,
    }
}

fn login_command(home: &Path) -> String {
    CodexLogin::in_home(home.to_path_buf()).login_command()
}

#[tokio::test]
async fn each_codex_home_sends_its_own_bearer_and_account_id() {
    let exp = far_future();
    let own_home = codex_home("acct-own", exp);
    let work_home = codex_home("acct-work", exp);
    let (own_url, own_log) = spawn_mock("200 OK", "text/event-stream", RESPONSES_SSE).await;
    let (work_url, work_log) = spawn_mock("200 OK", "text/event-stream", RESPONSES_SSE).await;

    let own = codex_backend("codex-own", own_home.path(), &own_url);
    let work = codex_backend("codex-work", work_home.path(), &work_url);
    // The client names an account and a FedRAMP flag of its own. Neither is
    // the backend's credential, so neither may travel with it.
    let client_headers = [
        ("chatgpt-account-id", "acct-own"),
        ("x-openai-fedramp", "true"),
    ];
    own.send(codex_request(&client_headers))
        .await
        .expect("own home serves");
    work.send(codex_request(&client_headers))
        .await
        .expect("work home serves");

    for (log, account) in [(&own_log, "acct-own"), (&work_log, "acct-work")] {
        let seen = recorded(log);
        assert_eq!(seen.len(), 1);
        let got = &seen[0];
        assert!(
            got.request_line.starts_with("POST /responses "),
            "{}",
            got.request_line
        );
        assert_eq!(
            got.values("authorization"),
            vec![format!("Bearer {}", access_token_for(account, exp)).as_str()]
        );
        assert_eq!(
            got.values("chatgpt-account-id"),
            vec![account],
            "the account id is the presented token's own, once"
        );
        assert!(got.values("x-openai-fedramp").is_empty());
        assert_eq!(got.values("originator"), vec!["codex_cli_rs"]);
        assert_eq!(got.body, CODEX_BODY);
    }
}

#[tokio::test]
async fn the_codex_probe_and_model_fetch_name_the_presented_account() {
    // Both are scoped to a workspace by the provider; asking without the
    // account would check, or list, something other than this login.
    const MODELS: &str = r#"{"models":[{"slug":"gpt-5.6"}]}"#;
    let home = codex_home("acct-work", far_future());

    let (probe_url, probe_log) = spawn_mock("200 OK", "application/json", MODELS).await;
    codex_backend("codex-work", home.path(), &probe_url)
        .probe()
        .await
        .expect("probe succeeds");

    let (models_url, models_log) = spawn_mock("200 OK", "application/json", MODELS).await;
    let document = codex_backend("codex-work", home.path(), &models_url)
        .models_document()
        .await
        .expect("the provider's document");
    assert_eq!(document, MODELS.as_bytes());

    for log in [&probe_log, &models_log] {
        let seen = recorded(log);
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].request_line.starts_with("GET /models"),
            "{}",
            seen[0].request_line
        );
        assert_eq!(seen[0].values("chatgpt-account-id"), vec!["acct-work"]);
    }
}

#[tokio::test]
async fn a_codex_home_holding_an_api_key_is_a_login_to_fix_not_a_hard_failure() {
    // Its bearer belongs to the metered host, so presenting it would end in a
    // host mismatch — never retried, which would stop the router from trying
    // anything else for this request.
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        home.path().join("auth.json"),
        r#"{"auth_mode":"apiKey","OPENAI_API_KEY":"sk-proj-EXAMPLE"}"#,
    )
    .expect("write");

    for base in [
        "https://chatgpt.com/backend-api/codex",
        "http://127.0.0.1:9",
    ] {
        let backend = codex_backend("codex-work", home.path(), base);
        let err = backend.send(codex_request(&[])).await.expect_err("refused");
        let UpstreamError::NeedsAuth { ref detail, .. } = err else {
            panic!("expected NeedsAuth against {base}, got {err:?}");
        };
        assert!(err.is_retryable());
        assert!(detail.contains("API key"), "{detail}");
        assert!(detail.contains(&login_command(home.path())), "{detail}");
    }
}

#[tokio::test]
async fn an_expired_codex_token_is_refused_naming_its_home() {
    let home = codex_home(
        "acct-work",
        (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp(),
    );
    let (url, log) = spawn_mock("200 OK", "text/event-stream", RESPONSES_SSE).await;
    let backend = codex_backend("codex-work", home.path(), &url);

    let err = backend.send(codex_request(&[])).await.expect_err("refused");
    let UpstreamError::NeedsAuth { ref detail, .. } = err else {
        panic!("expected NeedsAuth, got {err:?}");
    };
    assert!(err.is_retryable());
    assert!(detail.contains("expired"), "{detail}");
    let command = login_command(home.path());
    assert!(command.starts_with("CODEX_HOME="), "{command}");
    assert!(detail.contains(&command), "{detail}");

    let status = backend.status().await;
    assert!(!status.authenticated);
    assert!(status.detail.is_some_and(|d| d.contains(&command)));
    assert!(recorded(&log).is_empty(), "nothing may reach the provider");
}

#[tokio::test]
async fn a_rejected_codex_probe_names_the_home_to_log_in_again() {
    let home = codex_home("acct-work", far_future());
    let (url, _log) = spawn_mock("401 Unauthorized", "application/json", "{}").await;
    let err = codex_backend("codex-work", home.path(), &url)
        .probe()
        .await
        .expect_err("rejected");
    let UpstreamError::NeedsAuth { detail, .. } = err else {
        panic!("expected NeedsAuth, got {err:?}");
    };
    assert!(detail.contains(&login_command(home.path())), "{detail}");
}

// ---------------------------------------------------------------------------
// How each backend names its login
// ---------------------------------------------------------------------------

#[test]
fn the_default_logins_keep_their_ids_names_and_descriptions() {
    let claude = AnthropicBackend::subscription(None, 60).expect("client builds");
    assert_eq!(claude.id().as_str(), "claude-sub");
    assert_eq!(claude.name(), "Claude subscription");
    assert_eq!(
        claude.login(),
        Some(SubscriptionLogin::default_for(SubscriptionProduct::Claude))
    );

    let codex = ResponsesBackend::codex_subscription(None, 60).expect("client builds");
    assert_eq!(codex.id().as_str(), "codex-sub");
    assert_eq!(codex.name(), "ChatGPT subscription");
    assert_eq!(
        codex.login(),
        Some(SubscriptionLogin::default_for(SubscriptionProduct::ChatGpt))
    );
}

#[test]
fn an_additional_login_is_described_by_its_directory() {
    let home = PathBuf::from("/Users/alice");
    let work = ClaudeLogin::with_home(Some("~/.claude-work".into()), Some(home));
    let backend = AnthropicBackend::subscription_for(
        BackendId::from("claude-work"),
        work.describe(false).label(),
        work,
        None,
        60,
    )
    .expect("client builds");
    assert_eq!(backend.name(), "Claude subscription (~/.claude-work)");
    let login = backend.login().expect("a subscription has a login");
    assert!(!login.default);
    assert_eq!(login.location.as_deref(), Some("~/.claude-work"));
    assert_eq!(
        login.connect_command(),
        "ironwire connect claude --config-dir ~/.claude-work"
    );

    let codex_home = tempfile::tempdir().expect("tempdir");
    let codex = codex_backend("codex-work", codex_home.path(), "http://127.0.0.1:9");
    let login = codex.login().expect("a subscription has a login");
    assert_eq!(login.product, SubscriptionProduct::ChatGpt);
    assert!(!login.default);
    assert!(login.location.is_some());
}

#[test]
fn a_relocated_default_login_is_still_the_default() {
    // `claude-sub` with a config dir is the same backend everything already
    // recorded against `claude-sub` is about — just stored somewhere else.
    let moved = ClaudeLogin::with_home(
        Some("/Users/alice/.claude-personal".into()),
        Some(PathBuf::from("/Users/alice")),
    );
    let backend = AnthropicBackend::subscription_for(
        BackendId::from("claude-sub"),
        "Claude subscription".to_string(),
        moved,
        None,
        60,
    )
    .expect("client builds");
    let login = backend.login().expect("a subscription has a login");
    assert!(login.default);
    assert_eq!(login.location.as_deref(), Some("~/.claude-personal"));

    let codex_home = tempfile::tempdir().expect("tempdir");
    let codex = codex_backend("codex-sub", codex_home.path(), "http://127.0.0.1:9");
    assert!(codex.login().is_some_and(|l| l.default));
}

#[test]
fn a_metered_key_has_no_login() {
    let key =
        AnthropicBackend::api_key(SecretString::from("sk-ant-x"), None, 60).expect("client builds");
    assert_eq!(key.login(), None);
    let key = ResponsesBackend::openai_api_key(SecretString::from("sk-x"), None, 60)
        .expect("client builds");
    assert_eq!(key.login(), None);
}

/// A host check at construction is insufficient if the HTTP client can send
/// the credential or account header to a second origin on its own.
#[tokio::test]
async fn redirects_never_forward_a_logins_request_or_probe_to_another_origin() {
    let home = tempfile::tempdir().expect("home");
    let claude = claude_login(
        home.path(),
        ".claude-work",
        "sk-ant-oat01-SYNTHETIC",
        expires_in(chrono::Duration::hours(1)),
    );
    let codex = codex_home(
        "synthetic-work-account",
        (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
    );
    let (destination, destination_log) =
        spawn_mock("200 OK", "application/json", r#"{"data":[]}"#).await;
    let (source, source_log) = spawn_mock_with_location(
        "307 Temporary Redirect",
        "application/json",
        "{}",
        Some(format!("{destination}/stolen")),
    )
    .await;
    let backends: Vec<Box<dyn Backend>> = vec![
        Box::new(claude_backend("claude-work", claude, &source)),
        Box::new(codex_backend("codex-work", codex.path(), &source)),
        Box::new(
            AnthropicBackend::api_key(
                SecretString::from("sk-ant-SYNTHETIC"),
                Some(source.clone()),
                30,
            )
            .expect("client builds"),
        ),
        Box::new(
            ResponsesBackend::openai_api_key(SecretString::from("sk-SYNTHETIC"), Some(source), 30)
                .expect("client builds"),
        ),
    ];

    for backend in &backends {
        let request = if backend.id().as_str().starts_with("claude")
            || backend.id().as_str() == "anthropic-key"
        {
            claude_request()
        } else {
            codex_request(&[])
        };
        let result = backend.send(request).await;
        assert!(
            recorded(&destination_log).is_empty(),
            "{} forwarded a request to the redirect target",
            backend.id()
        );
        assert!(matches!(result, Err(UpstreamError::Upstream { status, .. })
            if status == http::StatusCode::TEMPORARY_REDIRECT));
        let result = backend.probe().await;
        assert!(
            recorded(&destination_log).is_empty(),
            "{} forwarded a probe to the redirect target",
            backend.id()
        );
        assert!(matches!(result, Err(UpstreamError::Upstream { status, .. })
            if status == http::StatusCode::TEMPORARY_REDIRECT));
        assert!(backend.models_document().await.is_none());
        assert!(recorded(&destination_log).is_empty());
    }
    assert!(recorded(&source_log).len() >= backends.len() * 2);
}
