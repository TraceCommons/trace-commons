//! Bounded, read-only native subscription usage probes. Secrets never form IPC
//! values or errors. Native clients retain ownership of authentication.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, Command};
use tokio::sync::OwnedSemaphorePermit;
use trace_commons_contributor_platform::external_claude_credentials as external;
#[cfg(unix)]
use trace_commons_contributor_platform::native_process_group::ProcessGroupGuard;

use super::{LoginSpec, ProbeFailure, Provider, Reading, Window};

const MAX_BYTES: usize = 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_CURRENT_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage?cedar_ember=1";

pub(super) async fn collect(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    match login.provider {
        Provider::Codex => collect_codex(login, interactive, lease).await,
        Provider::Claude => collect_claude(login, interactive, lease).await,
    }
}

// Native Security queries cannot be canceled and Tokio waits for spawn_blocking
// jobs at runtime drop. A leased detached thread bounds concurrency without
// preventing daemon shutdown; a dropped receiver simply drops its late result.
pub(super) async fn native_read<T: Send + 'static>(
    lease: Arc<OwnedSemaphorePermit>,
    operation: impl FnOnce() -> T + Send + 'static,
) -> Result<T, &'static str> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("tc-subscription-native".into())
        .spawn(move || {
            let _lease = lease;
            let _ = sender.send(operation());
        })
        .map_err(|_| "unavailable")?;
    receiver.await.map_err(|_| "unavailable")
}

fn failure(reason: &'static str, identity: Option<String>) -> ProbeFailure {
    ProbeFailure { reason, identity }
}

fn fingerprint(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn read_file(path: &Path) -> Result<Vec<u8>, &'static str> {
    let file =
        trace_commons_contributor_platform::local_files::open_import_file(path).map_err(|_| {
            if path.exists() {
                "unavailable"
            } else {
                "auth_missing"
            }
        })?;
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "unavailable")?;
    if bytes.len() > MAX_BYTES {
        return Err("invalid_response");
    }
    Ok(bytes)
}

// Stable account evidence rather than file timestamps, so an atomic token
// renewal for the same identified account does not look like a login swap.
fn codex_file_identity(bytes: &[u8]) -> Result<(String, Option<String>), &'static str> {
    let root: Value = serde_json::from_slice(bytes).map_err(|_| "auth_missing")?;
    if root
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
        || root
            .get("auth_mode")
            .and_then(Value::as_str)
            .is_some_and(|s| s == "apikey" || s == "api_key")
    {
        return Err("auth_missing");
    }
    let tokens = root.get("tokens").ok_or("auth_missing")?;
    let account = tokens
        .get("account_id")
        .or_else(|| tokens.get("accountId"))
        .and_then(Value::as_str);
    let access = tokens
        .get("access_token")
        .or_else(|| tokens.get("accessToken"))
        .and_then(Value::as_str);
    let identity = account
        .filter(|s| !s.is_empty())
        .or_else(|| access.filter(|s| !s.is_empty()))
        .ok_or("auth_missing")?;
    Ok((
        fingerprint(identity.as_bytes()),
        account
            .filter(|s| !s.is_empty())
            .map(|s| fingerprint(s.as_bytes())),
    ))
}

async fn codex_guard(
    root: &Path,
    lease: &Arc<OwnedSemaphorePermit>,
) -> Result<Option<(String, Option<String>)>, &'static str> {
    let path = root.join("auth.json");
    if !path.exists() {
        return Ok(None);
    }
    let lease = lease.clone();
    tokio::time::timeout(
        PROBE_TIMEOUT,
        native_read(lease, move || {
            read_file(&path).and_then(|bytes| codex_file_identity(&bytes))
        }),
    )
    .await
    .map_err(|_| "timeout")?
    .map_err(|_| "unavailable")?
    .map(Some)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexStore {
    File,
    Keyring,
    Auto,
}

fn codex_store(bytes: Option<&[u8]>) -> Result<CodexStore, &'static str> {
    let Some(bytes) = bytes else {
        return Ok(CodexStore::File);
    };
    let text = std::str::from_utf8(bytes).map_err(|_| "unavailable")?;
    let config: toml::Value = toml::from_str(text).map_err(|_| "unavailable")?;
    configured_codex_base(&config)?;
    match config.get("cli_auth_credentials_store") {
        None => Ok(CodexStore::File),
        Some(value) => match value.as_str() {
            Some("file") => Ok(CodexStore::File),
            Some("keyring") => Ok(CodexStore::Keyring),
            Some("auto") => Ok(CodexStore::Auto),
            _ => Err("keychain_needs_permission"),
        },
    }
}

fn configured_codex_base(config: &toml::Value) -> Result<String, &'static str> {
    let base = match config.get("chatgpt_base_url") {
        None => "https://chatgpt.com/backend-api/",
        Some(value) => value.as_str().ok_or("base_url_refused")?,
    };
    let url = reqwest::Url::parse(base).map_err(|_| "base_url_refused")?;
    let trusted_host = url.host_str().is_some_and(|host| {
        host == "chatgpt.com"
            || host.ends_with(".chatgpt.com")
            || host == "openai.com"
            || host.ends_with(".openai.com")
    });
    if url.scheme() != "https"
        || !trusted_host
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("base_url_refused");
    }
    // Do not silently override a named-profile endpoint with the top-level
    // default. This small display probe declines endpoint layering it cannot
    // resolve faithfully; the native client can still use that configuration.
    fn nested_override(value: &toml::Value) -> bool {
        match value {
            toml::Value::Table(table) => {
                table.contains_key("chatgpt_base_url") || table.values().any(nested_override)
            }
            toml::Value::Array(values) => values.iter().any(nested_override),
            _ => false,
        }
    }
    if config.as_table().is_some_and(|table| {
        table
            .iter()
            .any(|(key, value)| key != "chatgpt_base_url" && nested_override(value))
    }) {
        return Err("base_url_refused");
    }
    Ok(base.to_owned())
}

async fn selected_codex_config(
    root: &Path,
    lease: &Arc<OwnedSemaphorePermit>,
) -> Result<(CodexStore, String), &'static str> {
    let path = root.join("config.toml");
    let lease = lease.clone();
    tokio::time::timeout(
        PROBE_TIMEOUT,
        native_read(lease, move || {
            let bytes = if path.exists() {
                Some(read_file(&path)?)
            } else {
                None
            };
            let store = codex_store(bytes.as_deref())?;
            let config: toml::Value = toml::from_str(
                std::str::from_utf8(bytes.as_deref().unwrap_or(b"")).map_err(|_| "unavailable")?,
            )
            .map_err(|_| "unavailable")?;
            Ok((store, configured_codex_base(&config)?))
        }),
    )
    .await
    .map_err(|_| "timeout")?
    .map_err(|_| "unavailable")?
}

async fn collect_codex(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    // Do not let a usage query create a native profile that does not exist.
    if !login.root.is_dir() {
        return Err(failure("auth_missing", None));
    }
    let (store, base_url) = selected_codex_config(&login.root, &lease)
        .await
        .map_err(|r| failure(r, None))?;
    if store != CodexStore::File && !interactive {
        return Err(failure("keychain_needs_permission", None));
    }
    let file_identity = if store == CodexStore::File {
        Some(
            codex_guard(&login.root, &lease)
                .await
                .map_err(|r| failure(r, None))?
                .ok_or_else(|| failure("auth_missing", None))?,
        )
    } else {
        None
    };
    let mut current_identity = file_identity.as_ref().map(|(id, _)| id.clone());
    let executable = login
        .native_executable
        .clone()
        .or_else(|| crate::managed::profiles::find_native(crate::managed::ToolId::Codex).ok())
        .ok_or_else(|| failure("native_cli_unavailable", current_identity.clone()))?;
    let mut command = Command::new(executable);
    command.args(["-s", "read-only", "-a", "never"]);
    // Keep the same explicitly selected source in interactive/background
    // probes; a leftover file must never stand in for a configured keyring.
    command.args([
        "-c",
        match store {
            CodexStore::File => "cli_auth_credentials_store=\"file\"",
            CodexStore::Keyring => "cli_auth_credentials_store=\"keyring\"",
            CodexStore::Auto => "cli_auth_credentials_store=\"auto\"",
        },
    ]);
    command.args([
        "-c",
        &format!("chatgpt_base_url={}", toml::Value::String(base_url.clone())),
    ]);
    command
        .arg("app-server")
        .current_dir(&login.root)
        .env("CODEX_HOME", &login.root);
    for name in [
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "CHATGPT_BASE_URL",
        "CODEX_BASE_URL",
        "CODEX_CHATGPT_BASE_URL",
        "OPENAI_API_URL",
        "CHATGPT_API_URL",
        "CHATGPT_API_BASE",
        "OPENAI_API_BASE",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ] {
        command.env_remove(name);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|_| failure("unavailable", current_identity.clone()))?;
    // The command created its own group. Kill that group on cancellation too,
    // so an npm/native wrapper cannot leave a quota probe descendant behind.
    #[cfg(unix)]
    let mut group = match ProcessGroupGuard::new(child.id()) {
        Ok(group) => group,
        Err(reason) => {
            let _ = child.kill().await;
            return Err(failure(reason, current_identity.clone()));
        }
    };
    let expected_account = file_identity
        .as_ref()
        .and_then(|(_, account)| account.as_deref());
    let result = match (child.stdin.take(), child.stdout.take()) {
        (Some(mut input), Some(output)) => {
            let mut output = BufReader::new(output.take((MAX_BYTES + 1) as u64));
            let probe = codex_rpc(
                &mut input,
                &mut output,
                &mut current_identity,
                expected_account,
            );
            let result = match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
                Ok(result) => result,
                Err(_) => Err(failure("timeout", current_identity.clone())),
            };
            drop(input);
            result
        }
        _ => Err(failure("unavailable", current_identity.clone())),
    };
    #[cfg(unix)]
    let _ = group.terminate();
    stop_child(&mut child).await;
    let after = if store == CodexStore::File {
        codex_guard(&login.root, &lease)
            .await
            .map_err(|r| failure(r, None))?
    } else {
        None
    };
    if after != file_identity
        || selected_codex_config(&login.root, &lease)
            .await
            .map_err(|r| failure(r, None))?
            != (store, base_url)
    {
        return Err(failure("auth_changed", None));
    }
    result
}

async fn stop_child(child: &mut Child) {
    // kill() awaits the direct child's exit, including reaping. The separate
    // process-group guard handles descendants and cancellation on Unix.
    let _ = child.kill().await;
}

async fn rpc<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    input: &mut W,
    output: &mut R,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, &'static str> {
    let mut bytes = serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))
        .map_err(|_| "invalid_response")?;
    bytes.push(b'\n');
    input.write_all(&bytes).await.map_err(|_| "unavailable")?;
    loop {
        let mut line = Vec::new();
        let length = output
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| "unavailable")?;
        if length == 0 || line.len() > MAX_BYTES {
            return Err("invalid_response");
        }
        let message: Value = serde_json::from_slice(&line).map_err(|_| "invalid_response")?;
        if message.get("method").is_some() || message.get("id").and_then(Value::as_u64) != Some(id)
        {
            continue;
        }
        if let Some(error) = message.get("error") {
            let auth = error
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|s| {
                    s.contains("authentication required") || s.contains("not logged in")
                });
            return Err(if auth { "auth_missing" } else { "unavailable" });
        }
        return message.get("result").cloned().ok_or("invalid_response");
    }
}

async fn codex_rpc<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    input: &mut W,
    output: &mut R,
    current_identity: &mut Option<String>,
    expected_account: Option<&str>,
) -> Result<Reading, ProbeFailure> {
    rpc(
        input,
        output,
        1,
        "initialize",
        json!({"clientInfo":{
        "name":"trace_commons","version":env!("CARGO_PKG_VERSION")}}),
    )
    .await
    .map_err(|r| failure(r, current_identity.clone()))?;
    input
        .write_all(b"{\"method\":\"initialized\",\"params\":{}}\n")
        .await
        .map_err(|_| failure("unavailable", current_identity.clone()))?;
    let before = rpc(
        input,
        output,
        2,
        "account/read",
        json!({"refreshToken":false}),
    )
    .await
    .map_err(|r| failure(r, current_identity.clone()))?;
    let account = chatgpt_account(&before).map_err(|r| failure(r, None))?;
    // A native app-server may select keyring credentials. Once it reports an
    // account, no earlier file fingerprint is evidence for that account.
    *current_identity = account
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| fingerprint(s.as_bytes()));
    let limits = rpc(
        input,
        output,
        3,
        "account/rateLimits/read",
        json!({
        "excludeResetCreditDetails":true,"supportsLunaReserve":false}),
    )
    .await
    .map_err(|r| failure(r, current_identity.clone()))?;
    let reported_account = limits
        .get("accountId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| fingerprint(s.as_bytes()));
    if expected_account
        .zip(reported_account.as_deref())
        .is_some_and(|(a, b)| a != b)
    {
        return Err(failure("auth_changed", None));
    }
    if reported_account.is_some() {
        *current_identity = reported_account;
    }
    let after = rpc(
        input,
        output,
        4,
        "account/read",
        json!({"refreshToken":false}),
    )
    .await
    .map_err(|r| failure(r, None))?;
    if account != chatgpt_account(&after).map_err(|r| failure(r, None))? {
        return Err(failure("auth_changed", None));
    }
    let identity = current_identity
        .clone()
        .or_else(|| {
            account
                .get("email")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|s| fingerprint(s.as_bytes()))
        })
        .ok_or_else(|| failure("invalid_response", None))?;
    parse_codex(&limits, identity).map_err(|r| failure(r, current_identity.clone()))
}

fn chatgpt_account(response: &Value) -> Result<Value, &'static str> {
    let account = response
        .get("account")
        .filter(|v| v.is_object())
        .ok_or("auth_missing")?;
    if account.get("type").and_then(Value::as_str) != Some("chatgpt") {
        return Err("auth_missing");
    }
    Ok(account.clone())
}

fn parse_codex(value: &Value, identity: String) -> Result<Reading, &'static str> {
    let limits = value
        .get("rateLimitsByLimitId")
        .or_else(|| value.get("rate_limits_by_limit_id"))
        .and_then(|v| v.get("codex"))
        .or_else(|| value.get("rateLimits"))
        .ok_or("invalid_response")?;
    let mut windows = Vec::new();
    for (key, fallback_lane) in [("primary", "session"), ("secondary", "weekly")] {
        if let Some(window) = limits.get(key).filter(|v| v.is_object()) {
            // Native Codex may put the only weekly allowance in primary.
            // Duration is authoritative; slot position is only a fallback.
            let lane = match window
                .get("windowDurationMins")
                .or_else(|| window.get("window_duration_mins"))
                .and_then(Value::as_u64)
            {
                Some(300) => "session",
                Some(10080) => "weekly",
                _ => fallback_lane,
            };
            windows.push(Window {
                lane: lane.into(),
                label: None,
                used_permille: percent(
                    window
                        .get("usedPercent")
                        .or_else(|| window.get("used_percent")),
                ),
                resets_at: unix_reset(window.get("resetsAt").or_else(|| window.get("resets_at"))),
            });
        }
    }
    windows.sort_by_key(|window| if window.lane == "session" { 0 } else { 1 });
    let banked_resets = value
        .get("rateLimitResetCredits")
        .and_then(|v| v.get("availableCount"))
        .and_then(Value::as_u64);
    Ok(Reading {
        windows,
        banked_resets,
        identity,
    })
}

#[derive(Clone)]
enum ClaudeSource {
    File(PathBuf),
    Keychain {
        service: String,
        version: String,
        file_path: PathBuf,
    },
}
struct ClaudeCredential {
    token: String,
    identity: String,
    source: ClaudeSource,
}

fn claude_service(config_dir: Option<&str>) -> Result<String, &'static str> {
    let Some(dir) = config_dir.filter(|s| !s.is_empty()) else {
        return Ok("Claude Code-credentials".into());
    };
    let normalized = external::normalized_profile_directory(dir)?;
    Ok(format!(
        "Claude Code-credentials-{}",
        &fingerprint(normalized.as_bytes())[..8]
    ))
}

fn claude_path(login: &LoginSpec) -> Result<PathBuf, &'static str> {
    let root = match login.secure_storage_dir.as_deref() {
        Some("") => dirs::home_dir().ok_or("unavailable")?.join(".claude"),
        Some(value) => PathBuf::from(value),
        None => login.root.clone(),
    };
    Ok(root.join(".credentials.json"))
}

fn parse_claude_credential(
    bytes: &[u8],
    source: ClaudeSource,
) -> Result<ClaudeCredential, ProbeFailure> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| failure("auth_missing", None))?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or_else(|| failure("auth_missing", None))?;
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 8192 && !s.chars().any(char::is_control))
        .ok_or_else(|| failure("auth_missing", None))?;
    let identity = fingerprint(token.as_bytes());
    if let Some(expiry) = oauth.get("expiresAt").filter(|v| !v.is_null()) {
        let expired = expiry
            .as_i64()
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .is_none_or(|date| date <= Utc::now());
        if expired {
            return Err(failure("auth_expired", Some(identity)));
        }
    }
    if let Some(scopes) = oauth.get("scopes").and_then(Value::as_array) {
        if !scopes.iter().any(|s| s.as_str() == Some("user:profile")) {
            return Err(failure("auth_missing", Some(identity)));
        }
    }
    Ok(ClaudeCredential {
        token: token.into(),
        identity,
        source,
    })
}

fn load_claude(login: &LoginSpec, interactive: bool) -> Result<ClaudeCredential, ProbeFailure> {
    let path = claude_path(login).map_err(|r| failure(r, None))?;
    let file_error = match read_file(&path) {
        Ok(bytes) => match parse_claude_credential(&bytes, ClaudeSource::File(path.clone())) {
            Ok(credential) => return Ok(credential),
            Err(error) => Some(error),
        },
        Err(reason) => Some(failure(reason, None)),
    };
    // CodexBar establishes the secure-file override, but not a per-override
    // Keychain name. Refuse an uncertain fallback instead of reading a login
    // that the selected native profile might not own.
    if login.secure_storage_dir.is_some() {
        return Err(file_error.unwrap_or_else(|| failure("auth_missing", None)));
    }
    let service = claude_service(login.config_dir.as_deref()).map_err(|r| failure(r, None))?;
    let before = external::version(&service).map_err(|r| failure(r, None))?;
    let Some(version) = before else {
        return Err(file_error.unwrap_or_else(|| failure("auth_missing", None)));
    };
    let bytes = external::read(&service, interactive)
        .map_err(|r| failure(r, None))?
        .ok_or_else(|| failure("auth_missing", None))?;
    if external::version(&service)
        .map_err(|r| failure(r, None))?
        .as_deref()
        != Some(&version)
    {
        return Err(failure("auth_changed", None));
    }
    parse_claude_credential(
        &bytes,
        ClaudeSource::Keychain {
            service,
            version,
            file_path: path,
        },
    )
}

fn claude_unchanged(credential: &ClaudeCredential) -> bool {
    match &credential.source {
        ClaudeSource::File(path) => read_file(path)
            .ok()
            .and_then(|bytes| {
                parse_claude_credential(&bytes, ClaudeSource::File(path.clone())).ok()
            })
            .is_some_and(|fresh| fresh.identity == credential.identity),
        ClaudeSource::Keychain {
            service,
            version,
            file_path,
        } => {
            // A newly valid preferred file supersedes the selected legacy item.
            let preferred_file_now_valid = read_file(file_path)
                .ok()
                .and_then(|bytes| {
                    parse_claude_credential(&bytes, ClaudeSource::File(file_path.clone())).ok()
                })
                .is_some();
            !preferred_file_now_valid
                && external::version(service).ok().flatten().as_ref() == Some(version)
        }
    }
}

async fn collect_claude(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    let spec = login.clone();
    let credential_lease = lease.clone();
    let credential = tokio::time::timeout(
        PROBE_TIMEOUT,
        native_read(credential_lease, move || load_claude(&spec, interactive)),
    )
    .await
    .map_err(|_| failure("credential_read_timeout", None))?
    .map_err(|_| failure("unavailable", None))??;
    let identity = credential.identity.clone();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .no_proxy()
        .user_agent(concat!(
            "TraceCommons/",
            env!("CARGO_PKG_VERSION"),
            " subscription-usage"
        ))
        .build()
        .map_err(|_| failure("unavailable", Some(identity.clone())))?;
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut response = claude_request(&client, &credential.token, true)
            .send()
            .await
            .map_err(request_failure)?;
        // Older accounts may reject the current inventory query. Retry once
        // with the established usage endpoint; neither request changes state.
        if matches!(response.status().as_u16(), 400 | 403) {
            response = claude_request(&client, &credential.token, false)
                .send()
                .await
                .map_err(request_failure)?;
        }
        match response.status().as_u16() {
            200 => {}
            401 | 403 => return Err("auth_expired"),
            _ => return Err("unavailable"),
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err("invalid_response");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_failure)? {
            if chunk.len() > MAX_BYTES - bytes.len() {
                return Err("invalid_response");
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_response")?;
        parse_claude(&value, identity.clone())
    })
    .await
    .unwrap_or(Err("request_timeout"));
    let unchanged = tokio::time::timeout(
        PROBE_TIMEOUT,
        native_read(lease, move || claude_unchanged(&credential)),
    )
    .await
    .map_err(|_| failure("credential_check_timeout", None))?
    .map_err(|_| failure("unavailable", None))?;
    if !unchanged {
        return Err(failure("auth_changed", None));
    }
    result.map_err(|reason| failure(reason, Some(identity)))
}

fn request_failure(error: reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request_timeout"
    } else {
        "unavailable"
    }
}

fn claude_request(client: &reqwest::Client, token: &str, current: bool) -> reqwest::RequestBuilder {
    client
        .get(if current {
            CLAUDE_CURRENT_USAGE_URL
        } else {
            CLAUDE_USAGE_URL
        })
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
}

fn parse_claude(value: &Value, identity: String) -> Result<Reading, &'static str> {
    if !value.is_object() {
        return Err("invalid_response");
    }
    let mut windows = Vec::new();
    for (key, lane, label) in [
        ("five_hour", "session", None),
        ("seven_day", "weekly", None),
        ("seven_day_opus", "model", Some("Opus")),
        ("seven_day_sonnet", "model", Some("Sonnet")),
    ] {
        if let Some(window) = value.get(key).filter(|v| v.is_object()) {
            windows.push(Window {
                lane: lane.into(),
                label: label.map(str::to_owned),
                used_permille: percent(window.get("utilization")),
                resets_at: iso_reset(window.get("resets_at")),
            });
        }
    }
    if let Some(limits) = value.get("limits").and_then(Value::as_array) {
        for entry in limits.iter().take(64) {
            if entry.get("kind").and_then(Value::as_str) != Some("weekly_scoped")
                || entry.get("group").and_then(Value::as_str) != Some("weekly")
            {
                continue;
            }
            let Some(label) = entry
                .pointer("/scope/model/display_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control))
            else {
                continue;
            };
            let model_id = entry
                .pointer("/scope/model/id")
                .and_then(Value::as_str)
                .unwrap_or(label);
            if label.eq_ignore_ascii_case("all models") || model_id.ends_with("all-models") {
                continue;
            }
            if windows
                .iter()
                .any(|w| w.lane == "model" && w.label.as_deref() == Some(label))
            {
                continue;
            }
            windows.push(Window {
                lane: "model".into(),
                label: Some(label.into()),
                used_permille: percent(entry.get("percent")),
                resets_at: iso_reset(entry.get("resets_at")),
            });
        }
    }
    Ok(Reading {
        windows,
        banked_resets: None,
        identity,
    })
}

fn percent(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        .map(|n| (n * 10.0).round() as u16)
}
fn unix_reset(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .and_then(|n| DateTime::from_timestamp(n, 0))
}
fn iso_reset(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_source_selection_never_substitutes_a_leftover_file_for_keyring() {
        assert_eq!(codex_store(None).unwrap(), CodexStore::File);
        assert_eq!(
            codex_store(Some(b"cli_auth_credentials_store = 'file'\n")).unwrap(),
            CodexStore::File
        );
        assert_eq!(
            codex_store(Some(b"cli_auth_credentials_store = 'keyring'\n")).unwrap(),
            CodexStore::Keyring
        );
        assert_eq!(
            codex_store(Some(b"cli_auth_credentials_store = 'auto'\n")).unwrap(),
            CodexStore::Auto
        );
        assert_eq!(
            codex_store(Some(b"cli_auth_credentials_store = 2\n")).unwrap_err(),
            "keychain_needs_permission"
        );
    }

    #[test]
    fn codex_subscription_urls_require_provider_https_hosts_and_no_userinfo() {
        for base in [
            "https://chatgpt.com/backend-api/",
            "https://gov.chatgpt.com/backend-api/",
            "https://api.openai.com/custom/",
        ] {
            let config: toml::Value = toml::from_str(&format!(
                "chatgpt_base_url = {}",
                toml::Value::String(base.into())
            ))
            .unwrap();
            assert_eq!(configured_codex_base(&config).unwrap(), base);
        }
        for base in [
            "http://chatgpt.com/",
            "https://chatgpt.com.evil.test/",
            "https://openai.com@evil.test/",
            "https://name@chatgpt.com/",
            "https://chatgpt.com:pw@openai.com/",
            "https://127.0.0.1/",
        ] {
            let config: toml::Value = toml::from_str(&format!(
                "chatgpt_base_url = {}",
                toml::Value::String(base.into())
            ))
            .unwrap();
            assert_eq!(configured_codex_base(&config), Err("base_url_refused"));
        }
        assert_eq!(
            codex_store(Some(
                b"[profiles.custom]\nchatgpt_base_url='https://chatgpt.com/custom'\n"
            )),
            Err("base_url_refused")
        );
    }

    #[test]
    fn claude_usage_request_is_fixed_host_and_read_only_with_legacy_fallback() {
        let client = reqwest::Client::new();
        for current in [true, false] {
            let request = claude_request(&client, "fixture-token", current)
                .build()
                .unwrap();
            assert_eq!(request.method(), reqwest::Method::GET);
            assert_eq!(
                request.url().as_str(),
                if current {
                    CLAUDE_CURRENT_USAGE_URL
                } else {
                    CLAUDE_USAGE_URL
                }
            );
            assert_eq!(request.headers()["anthropic-beta"], "oauth-2025-04-20");
            assert_eq!(request.headers()["authorization"], "Bearer fixture-token");
        }
    }

    async fn fixture_rpc(
        lines: Vec<Value>,
        identity: &mut Option<String>,
    ) -> Result<Reading, ProbeFailure> {
        let (client, mut server) = tokio::io::duplex(MAX_BYTES * 2 + 8192);
        let mut transcript = Vec::new();
        for line in lines {
            transcript.extend(serde_json::to_vec(&line).unwrap());
            transcript.push(b'\n');
        }
        server.write_all(&transcript).await.unwrap();
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader.take((MAX_BYTES + 1) as u64));
        codex_rpc(&mut writer, &mut reader, identity, None).await
    }

    #[tokio::test]
    async fn codex_rate_limit_failure_belongs_to_native_account_not_old_file() {
        let mut identity = Some(fingerprint(b"old-file-account"));
        let error = fixture_rpc(vec![json!({"id":1,"result":{}}),
            json!({"id":2,"result":{"account":{"type":"chatgpt","email":"new@example.test","planType":"pro"}}}),
            json!({"id":3,"error":{"message":"fixture quota unavailable"}})], &mut identity).await.unwrap_err();
        assert_eq!(error.reason, "unavailable");
        assert_eq!(error.identity, Some(fingerprint(b"new@example.test")));
    }

    #[tokio::test]
    async fn codex_account_switch_during_probe_discards_identity() {
        let mut identity = None;
        let error = fixture_rpc(vec![json!({"id":1,"result":{}}),
            json!({"id":2,"result":{"account":{"type":"chatgpt","email":"a@example.test","planType":"pro"}}}),
            json!({"id":3,"result":{"accountId":"fixture-a","rateLimits":{"primary":{"usedPercent":20}}}}),
            json!({"id":4,"result":{"account":{"type":"chatgpt","email":"b@example.test","planType":"pro"}}})], &mut identity).await.unwrap_err();
        assert_eq!(error.reason, "auth_changed");
        assert_eq!(error.identity, None);
    }

    #[tokio::test]
    async fn codex_stdout_cap_is_enforced_before_unbounded_line_allocation() {
        let (client, mut server) = tokio::io::duplex(MAX_BYTES * 2 + 8192);
        server.write_all(&vec![b'x'; MAX_BYTES + 2]).await.unwrap();
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader.take((MAX_BYTES + 1) as u64));
        assert_eq!(
            rpc(&mut writer, &mut reader, 1, "initialize", json!({}))
                .await
                .unwrap_err(),
            "invalid_response"
        );
    }

    #[test]
    fn codex_count_is_authoritative_when_details_are_missing_or_capped() {
        for credits in [Value::Null, json!([]), json!([{"status":"available"}])] {
            let read = parse_codex(
                &json!({"rateLimits":{"secondary":{"usedPercent":42,"resetsAt":1794000000}},
                "rateLimitResetCredits":{"availableCount":4,"credits":credits}}),
                "fixture".into(),
            )
            .unwrap();
            assert_eq!(read.banked_resets, Some(4));
            assert_eq!(read.windows[0].used_permille, Some(420));
        }
    }

    #[test]
    fn codex_weekly_only_primary_uses_duration_without_fabricating_a_session() {
        let reading = parse_codex(
            &json!({"rateLimits":{"primary":{
            "usedPercent":37,"windowDurationMins":10080,"resetsAt":1794000000},"secondary":null},
            "rateLimitResetCredits":{"availableCount":2,"credits":null}}),
            "fixture".into(),
        )
        .unwrap();
        assert_eq!(reading.windows.len(), 1);
        assert_eq!(reading.windows[0].lane, "weekly");
        assert_eq!(reading.windows[0].used_permille, Some(370));
        assert!(reading.windows[0].resets_at.is_some());
        assert_eq!(reading.banked_resets, Some(2));
    }

    #[test]
    fn codex_reversed_windows_follow_duration_and_unknown_duration_keeps_slot_fallback() {
        let reading = parse_codex(
            &json!({"rateLimits":{"primary":{
            "usedPercent":37,"windowDurationMins":10080},"secondary":{
            "usedPercent":12,"windowDurationMins":300}}}),
            "fixture".into(),
        )
        .unwrap();
        assert_eq!(
            reading
                .windows
                .iter()
                .map(|w| (w.lane.as_str(), w.used_permille))
                .collect::<Vec<_>>(),
            vec![("session", Some(120)), ("weekly", Some(370))]
        );
        let unknown = parse_codex(
            &json!({"rateLimits":{"primary":{
            "usedPercent":21,"windowDurationMins":540},"secondary":{"usedPercent":43}}}),
            "fixture".into(),
        )
        .unwrap();
        assert_eq!(
            unknown
                .windows
                .iter()
                .map(|w| w.lane.as_str())
                .collect::<Vec<_>>(),
            vec!["session", "weekly"]
        );
    }

    #[test]
    fn codex_missing_or_malformed_inventory_keeps_valid_windows() {
        let read = parse_codex(
            &json!({"rateLimits":{"primary":{"usedPercent":0},"secondary":{"usedPercent":99.9}},
            "rateLimitResetCredits":{"availableCount":-1}}),
            "fixture".into(),
        )
        .unwrap();
        assert_eq!(read.banked_resets, None);
        assert_eq!(
            read.windows
                .iter()
                .map(|w| w.used_permille)
                .collect::<Vec<_>>(),
            vec![Some(0), Some(999)]
        );
    }

    #[test]
    fn claude_reads_session_weekly_and_generic_fable_scope_without_fabricating_missing_windows() {
        let read = parse_claude(&json!({"five_hour":{"utilization":12.5,"resets_at":"2026-10-27T12:00:00Z"},
            "seven_day":{"utilization":52},"limits":[{"kind":"weekly_scoped","group":"weekly","percent":23,
                "scope":{"model":{"id":"claude/fable.5:promo","display_name":"Fable"}}}]}),"fixture".into()).unwrap();
        assert_eq!(
            read.windows
                .iter()
                .map(|w| (w.lane.as_str(), w.label.as_deref(), w.used_permille))
                .collect::<Vec<_>>(),
            vec![
                ("session", None, Some(125)),
                ("weekly", None, Some(520)),
                ("model", Some("Fable"), Some(230))
            ]
        );
        assert!(
            parse_claude(
                &json!({"five_hour":null,"seven_day":null}),
                "fixture".into()
            )
            .unwrap()
            .windows
            .is_empty()
        );
    }

    #[test]
    fn claude_invalid_fields_do_not_discard_valid_siblings_or_turn_into_full_allowance() {
        let read = parse_claude(&json!({"five_hour":{"utilization":false},"seven_day":{"utilization":100},
            "limits":[false,{"kind":"weekly_scoped","group":"weekly","percent":"20","scope":{"model":{"display_name":"Fable"}}}]}),"fixture".into()).unwrap();
        assert_eq!(read.windows[0].used_permille, None);
        assert_eq!(read.windows[1].used_permille, Some(1000));
        assert_eq!(read.windows[2].used_permille, None);
        for v in [json!(-1), json!(101), json!("0"), json!(false), Value::Null] {
            assert_eq!(percent(Some(&v)), None);
        }
    }

    #[test]
    fn claude_native_profiles_keep_exact_service_names_and_never_use_global_fallback() {
        assert_eq!(claude_service(None).unwrap(), "Claude Code-credentials");
        assert_ne!(
            claude_service(Some("/tmp/a")).unwrap(),
            claude_service(Some("/tmp/a/")).unwrap()
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            claude_service(Some("/tmp/cafe\u{301}")).unwrap(),
            claude_service(Some("/tmp/caf\u{e9}")).unwrap()
        );
    }

    #[test]
    fn native_credential_parser_rejects_api_keys_and_scopeless_inference_tokens() {
        assert_eq!(
            codex_file_identity(br#"{"OPENAI_API_KEY":"DO_NOT_ECHO"}"#),
            Err("auth_missing")
        );
        let error = parse_claude_credential(
            br#"{"claudeAiOauth":{"accessToken":"DO_NOT_ECHO","scopes":["user:inference"]}}"#,
            ClaudeSource::File(PathBuf::from("fixture")),
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, "auth_missing");
        assert!(!format!("{error:?}").contains("DO_NOT_ECHO"));
    }
}
