use super::*;
use std::sync::Mutex;

use serde_json::json;

use super::super::providers::{collect_codex_in, collect_codex_via, fingerprint};
use super::super::{Provider, native_metadata_revision};

const TOKEN: &str = "fixture-access-token-never-logged";

fn jwt(payload: Value) -> String {
    let encode = |value: &Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap())
    };
    format!(
        "{}.{}.signature",
        encode(&json!({"alg":"none"})),
        encode(&payload)
    )
}

fn fresh_auth(access_token: &str) -> Value {
    json!({
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": jwt(json!({"email":"fixture@example.test"})),
            "access_token": access_token,
            "refresh_token": "fixture-refresh-token",
            "account_id": "account-fixture"
        },
        "last_refresh": Utc::now().to_rfc3339()
    })
}

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    beta: Option<String>,
    originator: Option<String>,
    authorization: Option<String>,
    account: Option<String>,
    accept: Option<String>,
}

struct FakeServer {
    base: reqwest::Url,
    seen: Arc<Mutex<Vec<Seen>>>,
}

/// The reset-credit inventory endpoint's reply: status, body, delay.
type Credits = (u16, &'static str, Duration);

const NO_CREDITS_ENDPOINT: Credits = (404, "", Duration::ZERO);

async fn fake_server(status: u16, body: &'static str) -> FakeServer {
    fake_server_with_credits(status, body, NO_CREDITS_ENDPOINT).await
}

async fn fake_server_with_credits(status: u16, body: &'static str, credits: Credits) -> FakeServer {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let app = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, headers: axum::http::HeaderMap| {
            let record = Arc::clone(&record);
            async move {
                let header = |name: &str| {
                    headers
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned)
                };
                record.lock().unwrap().push(Seen {
                    method: method.to_string(),
                    path: uri.path().to_owned(),
                    beta: header("openai-beta"),
                    originator: header("originator"),
                    authorization: header("authorization"),
                    account: header("chatgpt-account-id"),
                    accept: header("accept"),
                });
                let (status, body) = if uri.path().ends_with("/wham/rate-limit-reset-credits") {
                    tokio::time::sleep(credits.2).await;
                    (credits.0, credits.1)
                } else {
                    (status, body)
                };
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    body.to_owned(),
                )
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FakeServer {
        base: format!("http://{address}/backend-api/").parse().unwrap(),
        seen,
    }
}

fn target(server: &FakeServer) -> Target {
    Target {
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .unwrap(),
        base: Some(server.base.clone()),
        reset_credits_timeout: Duration::from_millis(300),
    }
}

fn lease() -> Arc<OwnedSemaphorePermit> {
    Arc::new(
        Arc::new(tokio::sync::Semaphore::new(1))
            .try_acquire_owned()
            .unwrap(),
    )
}

fn login(root: &std::path::Path) -> LoginSpec {
    let mut login = super::super::native_login(
        Provider::Codex,
        None,
        Some(root.to_string_lossy().into_owned()),
    )
    .unwrap();
    login.native_executable = Some(root.join("must-never-launch"));
    login
}

/// A process the Codex process guard recognizes, alive for the test's duration.
/// Unix only, like every test that needs one (it symlinks `/bin/sleep`).
#[cfg(unix)]
struct RunningCodex(std::process::Child);
#[cfg(unix)]
impl Drop for RunningCodex {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(unix)]
fn running_codex(dir: &std::path::Path) -> RunningCodex {
    let executable = dir.join("codex");
    std::os::unix::fs::symlink("/bin/sleep", &executable).unwrap();
    RunningCodex(
        std::process::Command::new(&executable)
            .arg("30")
            .spawn()
            .unwrap(),
    )
}

const USAGE: &str = r#"{
    "account_id": "account-fixture",
    "plan_type": "pro",
    "rate_limit": {
        "primary_window": {"used_percent": 12, "reset_at": 1794000000, "limit_window_seconds": 18000},
        "secondary_window": {"used_percent": 43, "reset_at": 1794500000, "limit_window_seconds": 604800}
    },
    "credits": {"has_credits": false, "unlimited": false, "balance": "0"},
    "additional_rate_limits": [
        {"limit_name": "GPT-5.3-Codex-Spark", "metered_feature": "codex_bengalfox",
         "rate_limit": {
            "primary_window": {"used_percent": 5, "reset_at": 1794000000, "limit_window_seconds": 18000},
            "secondary_window": {"used_percent": 7, "reset_at": 1794500000, "limit_window_seconds": 604800}}},
        {"limit_name": "Review", "metered_feature": "code_review",
         "rate_limit": {"primary_window": {"used_percent": 30, "reset_at": 1794500000, "limit_window_seconds": 604800}}}
    ]
}"#;

#[cfg(unix)]
#[tokio::test]
async fn fresh_token_reads_usage_while_codex_runs_and_never_writes_auth() {
    let dir = tempfile::tempdir().unwrap();
    let mut codex = running_codex(dir.path());
    let token = jwt(json!({"exp": Utc::now().timestamp() + 3600}));
    let auth = serde_json::to_vec(&fresh_auth(&token)).unwrap();
    std::fs::write(dir.path().join("auth.json"), &auth).unwrap();
    let server = fake_server(200, USAGE).await;
    let login = login(dir.path());
    let revision = native_metadata_revision(dir.path());

    let reading = collect_codex_via(&login, lease(), &target(&server))
        .await
        .unwrap();

    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "usage, then the reset-credit inventory");
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/backend-api/wham/usage");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some(format!("Bearer {token}").as_str())
    );
    assert_eq!(seen[0].account.as_deref(), Some("account-fixture"));
    assert_eq!(seen[0].accept.as_deref(), Some("application/json"));
    assert_eq!(
        reading
            .windows
            .iter()
            .map(|w| (
                w.lane.as_str(),
                w.label.as_deref(),
                w.duration_mins,
                w.used_permille
            ))
            .collect::<Vec<_>>(),
        vec![
            ("session", None, Some(300), Some(120)),
            ("weekly", None, Some(10080), Some(430)),
            ("model", Some("Codex Spark 5-hour"), Some(300), Some(50)),
            ("model", Some("Codex Spark Weekly"), Some(10080), Some(70)),
            ("model", Some("Review"), Some(10080), Some(300)),
        ]
    );
    assert_eq!(
        reading.windows[0].resets_at,
        DateTime::from_timestamp(1794000000, 0)
    );
    assert_eq!(
        reading.identity.account,
        Some(fingerprint(b"account-fixture"))
    );
    assert_eq!(reading.banked_resets, None);
    assert_eq!(reading.native_revision, Some(revision.clone()));
    // Read-only: the native owner's file is untouched.
    assert_eq!(std::fs::read(dir.path().join("auth.json")).unwrap(), auth);
    assert_eq!(native_metadata_revision(dir.path()), revision);
    assert!(
        codex.0.try_wait().unwrap().is_none(),
        "the user's Codex keeps running"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_silent_read_never_falls_back_to_the_native_app_server() {
    let near_expiry = jwt(json!({"exp": Utc::now().timestamp() + 120}));
    let dir = tempfile::tempdir().unwrap();
    let auth = json!({"tokens":{"access_token":near_expiry,"refresh_token":"r","account_id":"a"}});
    let bytes = serde_json::to_vec(&auth).unwrap();
    std::fs::write(dir.path().join("auth.json"), &bytes).unwrap();
    let server = fake_server(200, USAGE).await;
    let login = login(dir.path());
    let failure = collect_codex_in(&login, false, lease(), &target(&server))
        .await
        .unwrap_err();
    assert_eq!(failure.reason, "not_checked");
    assert!(server.seen.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(dir.path().join("auth.json")).unwrap(), bytes);
}

#[cfg(unix)]
#[tokio::test]
async fn stale_token_falls_back_to_native_and_is_refused_while_codex_runs() {
    let near_expiry = jwt(json!({"exp": Utc::now().timestamp() + 120}));
    for auth in [
        // Opaque token: the saved renewal time is the signal.
        json!({"tokens":{"access_token":"opaque","refresh_token":"r","account_id":"a"},
            "last_refresh": (Utc::now() - chrono::Duration::days(9)).to_rfc3339()}),
        json!({"tokens":{"access_token":"opaque","refresh_token":"r","account_id":"a"}}),
        // A recognized expiry wins over a recent renewal time.
        json!({"tokens":{"access_token":near_expiry,"refresh_token":"r","account_id":"a"},
            "last_refresh": Utc::now().to_rfc3339()}),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let _codex = running_codex(dir.path());
        let bytes = serde_json::to_vec(&auth).unwrap();
        std::fs::write(dir.path().join("auth.json"), &bytes).unwrap();
        let server = fake_server(200, USAGE).await;
        let failure = collect_codex_via(&login(dir.path()), lease(), &target(&server))
            .await
            .unwrap_err();
        assert_eq!(failure.reason, "native_tool_running");
        assert!(
            server.seen.lock().unwrap().is_empty(),
            "a stale token is never sent"
        );
        assert_eq!(std::fs::read(dir.path().join("auth.json")).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn unauthorized_falls_back_but_server_and_decode_failures_keep_their_labels() {
    for (status, body, expected) in [
        (401, "{}", "native_tool_running"),
        (403, "{}", "unavailable"),
        (429, "{}", "rate_limited"),
        (500, "PRIVATE_PROVIDER_ERROR", "unavailable"),
        (302, "https://untrusted.invalid/", "unavailable"),
        (200, "PRIVATE_PROVIDER_NON_JSON", "invalid_response"),
        (200, "[]", "invalid_response"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let _codex = running_codex(dir.path());
        let token = jwt(json!({"exp": Utc::now().timestamp() + 3600}));
        std::fs::write(
            dir.path().join("auth.json"),
            serde_json::to_vec(&fresh_auth(&token)).unwrap(),
        )
        .unwrap();
        let server = fake_server(status, body).await;
        let failure = collect_codex_via(&login(dir.path()), lease(), &target(&server))
            .await
            .unwrap_err();
        assert_eq!(failure.reason, expected, "status {status}");
        // A failed usage read never asks for the reset-credit inventory.
        assert_eq!(server.seen.lock().unwrap().len(), 1, "status {status}");
        if expected != "native_tool_running" {
            assert_eq!(
                failure.identity.and_then(|identity| identity.account),
                Some(fingerprint(b"account-fixture"))
            );
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn api_key_missing_tokens_and_unsupported_stores_never_reach_the_network_or_native() {
    for (config, auth, expected) in [
        (
            None,
            Some(json!({"OPENAI_API_KEY":"sk-fixture"})),
            "auth_missing",
        ),
        (
            None,
            Some(json!({"auth_mode":"apikey","tokens":null})),
            "auth_missing",
        ),
        (
            None,
            Some(json!({"tokens":{"account_id":"a"}})),
            "auth_missing",
        ),
        (None, None, "auth_missing"),
        (
            Some("cli_auth_credentials_store='keyring'"),
            Some(fresh_auth("fixture")),
            "native_storage_unsupported",
        ),
        (
            Some("cli_auth_credentials_store='auto'"),
            Some(fresh_auth("fixture")),
            "native_storage_unsupported",
        ),
        (
            Some("chatgpt_base_url='https://chatgpt.com.evil.test/'"),
            Some(fresh_auth("fixture")),
            "base_url_refused",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let _codex = running_codex(dir.path());
        if let Some(config) = config {
            std::fs::write(dir.path().join("config.toml"), config).unwrap();
        }
        if let Some(auth) = auth {
            std::fs::write(
                dir.path().join("auth.json"),
                serde_json::to_vec(&auth).unwrap(),
            )
            .unwrap();
        }
        let server = fake_server(200, USAGE).await;
        let failure = collect_codex_via(&login(dir.path()), lease(), &target(&server))
            .await
            .unwrap_err();
        assert_eq!(failure.reason, expected);
        assert!(server.seen.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn account_switch_reported_by_the_provider_discards_the_reading() {
    let dir = tempfile::tempdir().unwrap();
    let token = jwt(json!({"exp": Utc::now().timestamp() + 3600}));
    std::fs::write(
        dir.path().join("auth.json"),
        serde_json::to_vec(&fresh_auth(&token)).unwrap(),
    )
    .unwrap();
    let server = fake_server(200, r#"{"account_id":"someone-else","rate_limit":{}}"#).await;
    let failure = collect_codex_via(&login(dir.path()), lease(), &target(&server))
        .await
        .unwrap_err();
    assert_eq!(failure.reason, "auth_changed");
    assert_eq!(failure.identity, None);
}

#[test]
fn freshness_follows_expiry_then_saved_renewal_time_never_mtime() {
    let now = Utc::now();
    let auth = |access: String, last_refresh: Option<String>| {
        let mut value =
            json!({"tokens":{"access_token":access,"refresh_token":"r","account_id":"a"}});
        if let Some(at) = last_refresh {
            value["last_refresh"] = json!(at);
        }
        serde_json::to_vec(&value).unwrap()
    };
    let exp = |seconds: i64| jwt(json!({"exp": now.timestamp() + seconds}));
    let stale = |bytes: Vec<u8>| parse_credential(&bytes, now).unwrap().stale;
    assert!(!stale(auth(exp(302), None)));
    assert!(stale(auth(exp(300), Some(now.to_rfc3339()))));
    assert!(stale(auth(exp(-10), None)));
    assert!(!stale(auth(
        "opaque".into(),
        Some((now - chrono::Duration::days(7)).to_rfc3339())
    )));
    assert!(stale(auth(
        "opaque".into(),
        Some((now - chrono::Duration::days(8) - chrono::Duration::minutes(1)).to_rfc3339())
    )));
    assert!(stale(auth("opaque".into(), None)));
    assert!(stale(auth("opaque".into(), Some("not a date".into()))));
    // A non-integer expiry is not a recognized expiry; fall back to renewal time.
    assert!(!stale(auth(
        jwt(json!({"exp": "soon"})),
        Some(now.to_rfc3339())
    )));
}

#[test]
fn account_id_comes_from_tokens_then_jwt_claims() {
    let now = Utc::now();
    let account = |tokens: Value| {
        parse_credential(
            &serde_json::to_vec(&json!({"tokens":tokens,"last_refresh":now.to_rfc3339()})).unwrap(),
            now,
        )
        .unwrap()
        .account_id
    };
    assert_eq!(
        account(json!({"access_token":"t","refresh_token":"r","accountId":"camel"})),
        Some("camel".into())
    );
    assert_eq!(
        account(json!({"access_token":"t","refresh_token":"r",
            "id_token": jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"claim"}}))})),
        Some("claim".into())
    );
    assert_eq!(
        account(
            json!({"access_token": jwt(json!({"organizations":[{"id":""},{"id":"org"}]})),
            "refresh_token":"r"})
        ),
        Some("org".into())
    );
    assert_eq!(
        account(json!({"access_token":"t","refresh_token":"r"})),
        None
    );
}

#[test]
fn missing_refresh_token_is_not_an_oauth_credential() {
    let now = Utc::now();
    assert_eq!(
        parse_credential(br#"{"tokens":{"access_token":"t","account_id":"a"}}"#, now).err(),
        Some("oauth_unavailable")
    );
}

#[test]
fn usage_url_follows_backend_api_rule() {
    for (base, expected) in [
        (
            "https://chatgpt.com/backend-api/",
            "https://chatgpt.com/backend-api/wham/usage",
        ),
        (
            "https://chatgpt.com",
            "https://chatgpt.com/backend-api/wham/usage",
        ),
        (
            "https://gov.chatgpt.com/backend-api//",
            "https://gov.chatgpt.com/backend-api/wham/usage",
        ),
        (
            "https://api.openai.com/custom/",
            "https://api.openai.com/custom/api/codex/usage",
        ),
    ] {
        assert_eq!(usage_url(base).unwrap().as_str(), expected);
    }
}

#[test]
fn production_client_sends_token_only_in_a_header_over_https() {
    let target = Target::production().unwrap();
    let request = request(
        &target.client,
        &usage_url("https://chatgpt.com/backend-api/").unwrap(),
        TOKEN,
        Some("acct"),
    )
    .build()
    .unwrap();
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(request.url().scheme(), "https");
    assert!(!request.url().as_str().contains(TOKEN));
    assert_eq!(request.headers()["chatgpt-account-id"], "acct");
    assert!(request.body().is_none());
}

#[test]
fn usage_parse_keeps_unknown_values_unknown() {
    let reading = parse_usage(
        &json!({"rate_limit":{"primary_window":{"used_percent":"x","limit_window_seconds":32400},
            "secondary_window":null},
            "additional_rate_limits":[{"limit_name":"","metered_feature":""},"junk",
                {"limit_name":"Review","rate_limit":{"secondary_window":{"used_percent":101}}}]}),
        None,
    )
    .unwrap();
    assert_eq!(
        reading
            .windows
            .iter()
            .map(|w| (
                w.lane.as_str(),
                w.label.as_deref(),
                w.duration_mins,
                w.used_permille
            ))
            .collect::<Vec<_>>(),
        vec![
            ("primary", None, Some(540), None),
            ("model", Some("Review"), None, None),
        ]
    );
    assert_eq!(reading.identity, Default::default());
    assert!(
        parse_usage(&json!({"rate_limit":{}}), None)
            .unwrap()
            .windows
            .is_empty()
    );
    assert_eq!(
        parse_usage(&json!({"accountId":"b"}), Some("a")).err(),
        Some("auth_changed")
    );
}

const CREDITS: &str = r#"{"credits":[
    {"id":"c1","reset_type":"weekly","status":"available","granted_at":"2026-10-01T00:00:00Z"},
    {"id":"c2","reset_type":"weekly","status":"available","granted_at":"2026-10-02T00:00:00Z"}],
    "available_count": 2}"#;

async fn fresh_read(server: &FakeServer) -> Result<Reading, ProbeFailure> {
    let dir = tempfile::tempdir().unwrap();
    let token = jwt(json!({"exp": Utc::now().timestamp() + 3600}));
    std::fs::write(
        dir.path().join("auth.json"),
        serde_json::to_vec(&fresh_auth(&token)).unwrap(),
    )
    .unwrap();
    collect_codex_via(&login(dir.path()), lease(), &target(server)).await
}

#[tokio::test]
async fn reset_credit_inventory_is_read_with_codexbar_headers_and_only_a_get() {
    let server = fake_server_with_credits(200, USAGE, (200, CREDITS, Duration::ZERO)).await;
    let reading = fresh_read(&server).await.unwrap();
    assert_eq!(reading.banked_resets, Some(2));
    assert_eq!(reading.windows.len(), 5);
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    let credits = &seen[1];
    assert_eq!(credits.method, "GET");
    assert_eq!(credits.path, "/backend-api/wham/rate-limit-reset-credits");
    assert!(
        credits
            .authorization
            .as_deref()
            .is_some_and(|value| value.starts_with("Bearer "))
    );
    assert_eq!(credits.account.as_deref(), Some("account-fixture"));
    assert_eq!(credits.accept.as_deref(), Some("application/json"));
    assert_eq!(credits.beta.as_deref(), Some("codex-1"));
    assert_eq!(credits.originator.as_deref(), Some("Codex Desktop"));
    // The usage request itself carries neither inventory-only header.
    assert_eq!(seen[0].beta, None);
    assert_eq!(seen[0].originator, None);
}

#[tokio::test]
async fn reset_credit_failures_leave_banked_resets_unknown_and_keep_the_reading() {
    for credits in [
        (404, "", Duration::ZERO),
        (500, "PRIVATE_PROVIDER_ERROR", Duration::ZERO),
        (401, "{}", Duration::ZERO),
        (200, "PRIVATE_PROVIDER_NON_JSON", Duration::ZERO),
        (
            200,
            r#"{"credits":[],"available_count":-1}"#,
            Duration::ZERO,
        ),
        (200, r#"{"credits":[]}"#, Duration::ZERO),
        (200, CREDITS, Duration::from_secs(5)),
    ] {
        let server = fake_server_with_credits(200, USAGE, credits).await;
        let started = std::time::Instant::now();
        let reading = fresh_read(&server).await.unwrap();
        assert_eq!(reading.banked_resets, None, "{credits:?}");
        assert_eq!(reading.windows.len(), 5, "{credits:?}");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the inventory request is bounded"
        );
        assert_eq!(server.seen.lock().unwrap().len(), 2);
    }
}

#[test]
fn reset_credit_url_follows_codexbar_normalization() {
    for (base, expected) in [
        (
            "https://chatgpt.com/backend-api/",
            "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits",
        ),
        (
            "https://chatgpt.com",
            "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits",
        ),
        (
            "https://api.openai.com/custom/",
            "https://api.openai.com/custom/wham/rate-limit-reset-credits",
        ),
    ] {
        assert_eq!(reset_credits_url(base).unwrap().as_str(), expected);
    }
}

#[test]
fn reset_credit_count_is_authoritative_like_the_native_path() {
    for (body, expected) in [
        (json!({"credits":[],"available_count":3}), Some(3)),
        (json!({"credits":null,"available_count":4}), Some(4)),
        (json!({"available_count":-1}), None),
        (json!({"available_count":"2"}), None),
        (json!([]), None),
    ] {
        assert_eq!(parse_reset_credits(&body), expected, "{body}");
    }
}

const USAGE_WITHOUT_ACCOUNT: &str =
    r#"{"rate_limit":{"primary_window":{"used_percent":1,"limit_window_seconds":18000}}}"#;

#[tokio::test]
async fn fresh_token_without_any_account_id_hands_off_to_the_native_client() {
    let dir = tempfile::tempdir().unwrap();
    let token = jwt(json!({"exp": Utc::now().timestamp() + 3600}));
    std::fs::write(
        dir.path().join("auth.json"),
        serde_json::to_vec(&json!({"tokens":{"access_token":token,"refresh_token":"r",
            "id_token": jwt(json!({"email":"fixture@example.test"}))}}))
        .unwrap(),
    )
    .unwrap();
    let server = fake_server(200, USAGE_WITHOUT_ACCOUNT).await;
    // The native app-server identifies the account through account/read, so
    // missing account evidence here is a hand-off, not a failure.
    let handed_off = collect(
        &login(dir.path()),
        &lease(),
        "https://chatgpt.com/backend-api/",
        &target(&server),
    )
    .await
    .unwrap();
    assert!(handed_off.is_none());
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "no inventory request without an account");
    assert_eq!(seen[0].account, None);
}

/// Codex renews its own token while the usage request is in flight.
async fn renewing_server(auth: std::path::PathBuf, renewed: Vec<u8>) -> reqwest::Url {
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let (auth, renewed) = (auth.clone(), renewed.clone());
        async move {
            if uri.path().ends_with("/wham/usage") {
                // Atomic replacement, as the native client publishes it.
                let staging = auth.with_extension("renewing");
                std::fs::write(&staging, &renewed).unwrap();
                std::fs::rename(&staging, &auth).unwrap();
                (axum::http::StatusCode::OK, USAGE_WITHOUT_ACCOUNT.to_owned())
            } else {
                (axum::http::StatusCode::NOT_FOUND, String::new())
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}/backend-api/").parse().unwrap()
}

#[tokio::test]
async fn renewal_for_the_same_jwt_account_keeps_the_reading_and_a_switch_does_not() {
    let auth_for = |account: &str, exp_offset: i64| {
        let token = jwt(json!({"exp": Utc::now().timestamp() + exp_offset,
            "https://api.openai.com/auth": {"chatgpt_account_id": account}}));
        // No tokens.account_id: the account is known only from the JWT.
        serde_json::to_vec(&json!({"tokens":{"access_token":token,"refresh_token":"r"}})).unwrap()
    };
    for (renewed_account, expected) in [
        ("account-jwt", None),
        ("someone-else", Some("auth_changed")),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, auth_for("account-jwt", 3600)).unwrap();
        let renewed = auth_for(renewed_account, 7200);
        let mut target = target(&FakeServer {
            base: renewing_server(path.clone(), renewed.clone()).await,
            seen: Arc::default(),
        });
        target.reset_credits_timeout = Duration::from_millis(300);
        let result = collect(
            &login(dir.path()),
            &lease(),
            "https://chatgpt.com/backend-api/",
            &target,
        )
        .await;
        assert_eq!(
            std::fs::read(&path).unwrap(),
            renewed,
            "the renewal happened"
        );
        match expected {
            None => {
                let reading = result.unwrap().expect("a reading, not a hand-off");
                assert_eq!(reading.identity.account, Some(fingerprint(b"account-jwt")));
                assert_eq!(
                    reading.native_revision,
                    Some(native_metadata_revision(dir.path()))
                );
            }
            Some(reason) => assert_eq!(result.unwrap_err().reason, reason),
        }
    }
}
