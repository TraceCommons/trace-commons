//! Explicit, bounded native subscription usage probes. Secrets never form IPC
//! values or errors. The native CLI owns authentication and may renew its own
//! credentials, so the native app-server probe refuses while another Codex
//! process is running. The read-only OAuth reader (`codex_oauth`) runs first
//! and writes nothing, so it does not.

use std::io::Read;
use std::path::Path;
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
#[cfg(unix)]
use trace_commons_contributor_platform::native_process_group::ProcessGroupGuard;

use super::{
    Identity, LoginSpec, ProbeFailure, Provider, Reading, Window, codex_oauth,
    native_metadata_revision,
};

const MAX_BYTES: usize = 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg(test)]
pub(super) async fn collect(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    let deps = super::claude::Deps::for_build(Arc::new(
        super::claude_token_cache::TokenCache::for_build(),
    ));
    collect_with(login, interactive, lease, &deps).await
}

pub(super) async fn collect_with(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
    claude: &super::claude::Deps,
) -> Result<Reading, ProbeFailure> {
    match login.provider {
        Provider::Codex => collect_codex(login, interactive, lease).await,
        Provider::Claude => super::claude::collect(login, interactive, lease, claude).await,
    }
}

// Native filesystem/process queries may block and cannot be canceled. Tokio
// waits for spawn_blocking jobs at runtime drop. A leased detached thread bounds concurrency without
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

pub(super) fn failure(reason: &'static str, identity: Option<Identity>) -> ProbeFailure {
    ProbeFailure {
        reason: normalize_failure_reason(reason),
        identity,
    }
}

pub(super) fn normalize_failure_reason(reason: &'static str) -> &'static str {
    match reason {
        "credential_changed" => "auth_changed",
        other => other,
    }
}

/// The fixed client for provider usage requests: HTTPS only, no proxy, no
/// redirects (a redirect could carry the bearer to another host), bounded.
pub(super) fn usage_client() -> Result<reqwest::Client, &'static str> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .no_proxy()
        .https_only(true)
        .user_agent(concat!(
            "TraceCommons/",
            env!("CARGO_PKG_VERSION"),
            " subscription-usage"
        ))
        .build()
        .map_err(|_| "unavailable")
}

pub(super) fn request_error(error: reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request_timeout"
    } else {
        "unavailable"
    }
}

/// A size-capped JSON body. Provider text never reaches an error value.
pub(super) async fn bounded_json(mut response: reqwest::Response) -> Result<Value, &'static str> {
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BYTES as u64)
    {
        return Err("invalid_response");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err("invalid_response");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_response")
}

pub(super) fn fingerprint(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(super) fn read_file(path: &Path) -> Result<Vec<u8>, &'static str> {
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
pub(super) fn codex_file_identity(bytes: &[u8]) -> Result<(String, Option<String>), &'static str> {
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
    // Without a saved account id, prefer the account the JWT claims carry
    // (CodexBar `accountIDFromJWT`) over the token itself, so a renewal for
    // the same account is not mistaken for an account switch.
    let jwt_account = account.filter(|s| !s.is_empty()).is_none().then(|| {
        let id_token = tokens
            .get("id_token")
            .or_else(|| tokens.get("idToken"))
            .and_then(Value::as_str);
        codex_oauth::account_from_jwt([id_token, access])
    });
    let jwt_account = jwt_account.flatten();
    let identity = account
        .filter(|s| !s.is_empty())
        .or(jwt_account.as_deref())
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
pub(super) enum CodexStore {
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
            _ => Err("native_storage_unsupported"),
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
            // No verified credential-free Keychain revision is available. Auto
            // may select Keychain even if auth.json exists, so refuse it too.
            if store != CodexStore::File {
                return Err("native_storage_unsupported");
            }
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

// We deliberately do not try to infer CODEX_HOME from another process's
// environment. Any Codex process may share this home. Unknown process metadata
// fails closed; neither scanning nor refusal signals another process.
fn codex_process_may_share_home(
    name: &str,
    executable: Option<&Path>,
    arguments: &[String],
    native: Option<&Path>,
) -> bool {
    if name.trim().is_empty() {
        return true;
    }
    if executable
        .zip(native)
        .is_some_and(|(running, selected)| running == selected)
    {
        return true;
    }
    fn codex_token(token: &str) -> bool {
        let token = token.to_ascii_lowercase();
        let name = token.rsplit(['/', '\\']).next().unwrap_or(&token);
        matches!(name, "codex" | "codex.exe" | "codex.js" | "codex.mjs")
            || name.starts_with("codex-")
            || token.contains("/codex.app/")
            || token.contains("/@openai/codex/")
    }
    if codex_token(name)
        || executable.is_some_and(|path| codex_token(&path.to_string_lossy()))
        || arguments.iter().any(|argument| codex_token(argument))
    {
        return true;
    }
    // A wrapper whose argv is hidden could be the native Codex launcher.
    arguments.is_empty() && matches!(name, "node" | "nodejs" | "sh" | "bash" | "zsh" | "env")
}

async fn ensure_codex_idle(
    login: &LoginSpec,
    lease: &Arc<OwnedSemaphorePermit>,
) -> Result<(), ProbeFailure> {
    let native = login
        .native_executable
        .clone()
        .or_else(|| crate::managed::profiles::find_native(crate::managed::ToolId::Codex).ok())
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path));
    let check = native_read(Arc::clone(lease), move || {
        let mut system = sysinfo::System::new();
        if system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_exe(sysinfo::UpdateKind::Always)
                .with_cmd(sysinfo::UpdateKind::Always),
        ) == 0
        {
            return false;
        }
        !system.processes().values().any(|process| {
            codex_process_may_share_home(
                &process.name().to_string_lossy(),
                process.exe(),
                &process
                    .cmd()
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                native.as_deref(),
            )
        })
    });
    match tokio::time::timeout(PROBE_TIMEOUT, check).await {
        Ok(Ok(true)) => Ok(()),
        _ => Err(failure("native_tool_running", None)),
    }
}

async fn collect_codex(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    let target = codex_oauth::Target::production().map_err(|r| failure(r, None))?;
    collect_codex_in(login, interactive, lease, &target).await
}

/// A pressed Refresh, the only kind the fixture tests drive.
#[cfg(test)]
pub(super) async fn collect_codex_via(
    login: &LoginSpec,
    lease: Arc<OwnedSemaphorePermit>,
    target: &codex_oauth::Target,
) -> Result<Reading, ProbeFailure> {
    collect_codex_in(login, true, lease, target).await
}

/// CodexBar's preferred order (docs/codex.md at 5f93dc3): read-only OAuth
/// usage first, then the native app-server only when the saved sign-in needs
/// renewal or the provider rejects it. Only the native path can rewrite
/// auth.json, so only that path waits for Codex to be closed. A silent read
/// (opening Insights) stops after the OAuth step; the app-server waits for a
/// pressed Refresh.
pub(super) async fn collect_codex_in(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
    target: &codex_oauth::Target,
) -> Result<Reading, ProbeFailure> {
    // Do not let a usage query create a native profile that does not exist.
    if !login.root.is_dir() {
        return Err(failure("auth_missing", None));
    }
    // Keyring/auto stores and untrusted endpoints are refused here, before
    // any credential body is opened on either path.
    let (_, base_url) = selected_codex_config(&login.root, &lease)
        .await
        .map_err(|r| failure(r, None))?;
    if let Some(reading) = codex_oauth::collect(login, &lease, &base_url, target).await? {
        return Ok(reading);
    }
    if !interactive {
        return Err(failure("not_checked", None));
    }
    // A global single-probe lease serializes our own collectors; external
    // native clients do not share that lease, so recheck before each step.
    ensure_codex_idle(login, &lease).await?;
    collect_codex_when_idle(login, lease).await
}

async fn collect_codex_when_idle(
    login: &LoginSpec,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<Reading, ProbeFailure> {
    // Do not let a usage query create a native profile that does not exist.
    if !login.root.is_dir() {
        return Err(failure("auth_missing", None));
    }
    let (store, base_url) = selected_codex_config(&login.root, &lease)
        .await
        .map_err(|r| failure(r, None))?;
    ensure_codex_idle(login, &lease).await?;
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
    let mut current_identity = file_identity
        .as_ref()
        .and_then(|(_, account)| account.clone())
        .map(|account| Identity {
            account: Some(account),
            email: None,
            organization: None,
        });
    let executable = login
        .native_executable
        .clone()
        .or_else(|| crate::managed::profiles::find_native(crate::managed::ToolId::Codex).ok())
        .ok_or_else(|| failure("native_cli_unavailable", current_identity.clone()))?;
    let mut command = Command::new(executable);
    command.args(["-s", "read-only", "-a", "never"]);
    // Only file-backed native credentials are supported. Neither keyring nor
    // auto is allowed to reach the native executable, even on explicit Refresh.
    command.args(["-c", "cli_auth_credentials_store=\"file\""]);
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
    ensure_codex_idle(login, &lease).await?;
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
    ensure_codex_idle(login, &lease).await?;
    let revision =
        codex_completion_revision(&login.root, &lease, &file_identity, &(store, base_url)).await?;
    result.map(|mut reading| {
        reading.native_revision = Some(revision);
        reading
    })
}

pub(super) async fn codex_completion_revision(
    root: &Path,
    lease: &Arc<OwnedSemaphorePermit>,
    expected_identity: &Option<(String, Option<String>)>,
    expected_config: &(CodexStore, String),
) -> Result<String, ProbeFailure> {
    let revision_before = native_metadata_revision(root);
    // Revalidate the storage selection before opening any credential body.
    let config = selected_codex_config(root, lease)
        .await
        .map_err(|r| failure(r, None))?;
    if &config != expected_config {
        return Err(failure("auth_changed", None));
    }
    let identity = codex_guard(root, lease)
        .await
        .map_err(|r| failure(r, None))?;
    let revision_after = native_metadata_revision(root);
    if &identity != expected_identity || revision_before != revision_after {
        return Err(failure("auth_changed", None));
    }
    Ok(revision_after)
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
    let mut request = json!({"id":id,"method":method});
    if !params.is_null() {
        request["params"] = params;
    }
    let mut bytes = serde_json::to_vec(&request).map_err(|_| "invalid_response")?;
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
            let code = error.get("code").and_then(Value::as_i64);
            let expected_unit = error
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.to_ascii_lowercase().contains("expected unit"));
            return Err(if auth {
                "auth_missing"
            } else if method == "account/rateLimits/read"
                && (code == Some(-32602) || (code == Some(-32600) && expected_unit))
            {
                "legacy_rate_limits_params"
            } else {
                "unavailable"
            });
        }
        return message.get("result").cloned().ok_or("invalid_response");
    }
}

async fn codex_rpc<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    input: &mut W,
    output: &mut R,
    current_identity: &mut Option<Identity>,
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
        .map(|s| Identity {
            account: None,
            email: Some(fingerprint(s.as_bytes())),
            organization: None,
        });
    let limits = rpc(
        input,
        output,
        3,
        "account/rateLimits/read",
        json!({
        "excludeResetCreditDetails":true,"supportsLunaReserve":false}),
    )
    .await;
    let limits = match limits {
        Err("legacy_rate_limits_params") => {
            rpc(input, output, 5, "account/rateLimits/read", Value::Null).await
        }
        result => result,
    }
    .map_err(|reason| {
        failure(
            if reason == "legacy_rate_limits_params" {
                "unavailable"
            } else {
                reason
            },
            current_identity.clone(),
        )
    })?;
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
    if let Some(reported_account) = reported_account {
        current_identity
            .get_or_insert_with(Identity::default)
            .account = Some(reported_account);
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

fn parse_codex(value: &Value, identity: Identity) -> Result<Reading, &'static str> {
    let limits = value
        .get("rateLimitsByLimitId")
        .or_else(|| value.get("rate_limits_by_limit_id"))
        .and_then(|v| v.get("codex"))
        .or_else(|| value.get("rateLimits"))
        .ok_or("invalid_response")?;
    let mut windows = Vec::new();
    for key in ["primary", "secondary"] {
        if let Some(window) = limits.get(key).filter(|v| v.is_object()) {
            // Native Codex may put the only weekly allowance in primary.
            // Duration is authoritative; unknown slots keep their distinct names.
            let duration_mins = window
                .get("windowDurationMins")
                .or_else(|| window.get("window_duration_mins"))
                .and_then(Value::as_u64)
                .filter(|minutes| *minutes > 0 && *minutes <= i64::MAX as u64);
            let lane = match duration_mins {
                Some(300) => "session",
                Some(10080) => "weekly",
                _ => key,
            };
            windows.push(Window {
                lane: lane.into(),
                label: None,
                duration_mins,
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
        native_revision: None,
        source: super::ReadingSource::Api,
    })
}

pub(super) fn percent(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        .map(|n| (n * 10.0).round() as u16)
}
pub(super) fn unix_reset(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .and_then(|n| DateTime::from_timestamp(n, 0))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_guard_refuses_native_clients_wrappers_and_uninspectable_wrappers() {
        for (name, executable, args) in [
            ("codex", None, vec![]),
            (
                "Codex",
                Some("/Applications/Codex.app/Contents/MacOS/Codex"),
                vec![],
            ),
            (
                "node",
                None,
                vec!["/opt/node_modules/@openai/codex/bin/codex.js"],
            ),
            ("sh", None, vec!["/tmp/codex", "app-server"]),
            ("node", None, vec![]),
            ("", None, vec![]),
        ] {
            assert!(codex_process_may_share_home(
                name,
                executable.map(Path::new),
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                None
            ));
        }
        assert!(codex_process_may_share_home(
            "renamed",
            Some(Path::new("/tmp/native")),
            &[],
            Some(Path::new("/tmp/native"))
        ));
        assert!(!codex_process_may_share_home(
            "other",
            Some(Path::new("/bin/other")),
            &["other".into()],
            None
        ));
    }

    // A running Codex no longer blocks a read-only OAuth reading (see
    // codex_oauth tests). It still refuses the native path, which may renew
    // and rewrite auth.json, before that path launches anything.
    #[cfg(unix)]
    #[tokio::test]
    async fn running_codex_refuses_native_path_before_launch() {
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("codex");
        std::os::unix::fs::symlink("/bin/sleep", &executable).unwrap();
        let mut child = OwnedChild(
            std::process::Command::new(&executable)
                .arg("30")
                .spawn()
                .unwrap(),
        );
        // No saved renewal time: the token needs renewal, so only the native
        // client may handle it.
        std::fs::write(
            dir.path().join("auth.json"),
            br#"{"tokens":{"account_id":"a","access_token":"opaque","refresh_token":"r"}}"#,
        )
        .unwrap();
        let mut login = super::super::native_login(
            Provider::Codex,
            None,
            Some(dir.path().to_string_lossy().into_owned()),
        )
        .unwrap();
        login.native_executable = Some(dir.path().join("must-never-launch"));
        let lease = Arc::new(
            Arc::new(tokio::sync::Semaphore::new(1))
                .acquire_owned()
                .await
                .unwrap(),
        );
        assert_eq!(
            collect(&login, true, lease).await.unwrap_err().reason,
            "native_tool_running"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "the user's process must survive refusal"
        );
    }

    #[tokio::test]
    async fn noninteractive_claude_never_queries_keychain() {
        let login = LoginSpec {
            provider: Provider::Claude,
            root: "/does/not/exist".into(),
            id: "gated".into(),
            label: "gated".into(),
            default_login: true,
            native_executable: None,
            config_dir: None,
            secure_storage_dir: None,
            metadata_revision: None,
            native_revision: None,
        };
        let lease = Arc::new(
            Arc::new(tokio::sync::Semaphore::new(1))
                .acquire_owned()
                .await
                .unwrap(),
        );
        assert_eq!(
            collect(&login, false, lease).await.unwrap_err().reason,
            "keychain_needs_permission"
        );
    }

    #[tokio::test]
    async fn older_unit_params_schema_retries_once_without_params_over_native_protocol() {
        for error in [
            json!({"code":-32600,"message":"Invalid request: invalid type: map, expected unit"}),
            json!({"code":-32602,"message":"Invalid params"}),
        ] {
            let (client, server) = tokio::io::duplex(8192);
            let (reader, mut input) = tokio::io::split(client);
            let mut output = BufReader::new(reader);
            let fixture = tokio::spawn(async move {
                let (reader, mut writer) = tokio::io::split(server);
                let mut reader = BufReader::new(reader);
                let account = json!({"account":{"type":"chatgpt","email":"fixture@example.test","planType":"pro"}});
                for (id, method) in [
                    (1, "initialize"),
                    (0, "initialized"),
                    (2, "account/read"),
                    (3, "account/rateLimits/read"),
                    (5, "account/rateLimits/read"),
                    (4, "account/read"),
                ] {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(request["method"], method);
                    if id == 0 {
                        continue;
                    }
                    assert_eq!(request["id"], id);
                    let response = match id {
                        1 => json!({"id":id,"result":{}}),
                        2 | 4 => json!({"id":id,"result":account}),
                        3 => {
                            assert_eq!(request["params"]["excludeResetCreditDetails"], true);
                            json!({"id":id,"error":error})
                        }
                        5 => {
                            assert!(request.get("params").is_none());
                            json!({"id":id,"result":{"rateLimits":{"primary":{"usedPercent":12,"windowDurationMins":540}}}})
                        }
                        _ => unreachable!(),
                    };
                    writer
                        .write_all(format!("{response}\n").as_bytes())
                        .await
                        .unwrap();
                }
            });
            let reading = codex_rpc(&mut input, &mut output, &mut None, None)
                .await
                .unwrap();
            assert_eq!(reading.windows[0].used_permille, Some(120));
            assert_eq!(reading.windows[0].duration_mins, Some(540));
            fixture.await.unwrap();
        }
    }

    #[tokio::test]
    async fn arbitrary_rpc_errors_do_not_retry_legacy_schema() {
        for error in [
            json!({"code":-32600,"message":"Invalid request"}),
            json!({"code":429,"message":"Too many requests"}),
            json!({"code":401,"message":"authentication required"}),
        ] {
            let failure = fixture_rpc(vec![json!({"id":1,"result":{}}), json!({"id":2,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}}), json!({"id":3,"error":error})], &mut None).await.unwrap_err();
            assert!(matches!(failure.reason, "unavailable" | "auth_missing"));
        }
    }

    #[test]
    fn exact_and_unknown_durations_remain_distinct_without_slot_inference() {
        for (primary, secondary, expected) in [
            (json!(540), json!(1440), [Some(540), Some(1440)]),
            (Value::Null, json!(0), [None, None]),
            (json!(u64::MAX), json!(-1), [None, None]),
        ] {
            let reading = parse_codex(&json!({"rateLimits":{"primary":{"usedPercent":12,"windowDurationMins":primary},"secondary":{"usedPercent":43,"windowDurationMins":secondary}}}), "fixture".into()).unwrap();
            assert_eq!(reading.windows.len(), 2);
            assert_eq!(
                (
                    reading.windows[0].lane.as_str(),
                    reading.windows[1].lane.as_str()
                ),
                ("primary", "secondary")
            );
            assert_eq!(
                [
                    reading.windows[0].duration_mins,
                    reading.windows[1].duration_mins
                ],
                expected
            );
        }
    }

    #[tokio::test]
    async fn post_read_witness_accepts_same_account_rotation_and_rejects_account_switch() {
        let dir = tempfile::tempdir().unwrap();
        let auth = dir.path().join("auth.json");
        std::fs::write(
            &auth,
            br#"{"tokens":{"account_id":"account-a","access_token":"old"}}"#,
        )
        .unwrap();
        let lease = Arc::new(
            Arc::new(tokio::sync::Semaphore::new(1))
                .acquire_owned()
                .await
                .unwrap(),
        );
        let original = codex_guard(dir.path(), &lease).await.unwrap();
        let config = selected_codex_config(dir.path(), &lease).await.unwrap();
        let before = native_metadata_revision(dir.path());
        std::fs::write(
            &auth,
            br#"{"tokens":{"account_id":"account-a","access_token":"rotated-token"}}"#,
        )
        .unwrap();
        let witness = codex_completion_revision(dir.path(), &lease, &original, &config)
            .await
            .unwrap();
        assert_ne!(before, witness);
        assert_eq!(witness, native_metadata_revision(dir.path()));
        std::fs::write(
            &auth,
            br#"{"tokens":{"account_id":"account-b","access_token":"another-token"}}"#,
        )
        .unwrap();
        assert_eq!(
            codex_completion_revision(dir.path(), &lease, &original, &config)
                .await
                .unwrap_err()
                .reason,
            "auth_changed"
        );
        std::fs::write(
            dir.path().join("config.toml"),
            "cli_auth_credentials_store='keyring'",
        )
        .unwrap();
        std::fs::write(&auth, b"invalid credentials must not be parsed").unwrap();
        assert_eq!(
            codex_completion_revision(dir.path(), &lease, &original, &config)
                .await
                .unwrap_err()
                .reason,
            "native_storage_unsupported"
        );
    }

    #[tokio::test]
    async fn keyring_and_auto_are_refused_before_auth_body_reads_or_cli_launch() {
        for store in ["keyring", "auto"] {
            for auth in [
                b"invalid credential body".as_slice(),
                br#"{"tokens":{"account_id":"fixture","access_token":"fixture"}}"#.as_slice(),
            ] {
                let dir = tempfile::tempdir().unwrap();
                std::fs::write(
                    dir.path().join("config.toml"),
                    format!("cli_auth_credentials_store='{store}'"),
                )
                .unwrap();
                std::fs::write(dir.path().join("auth.json"), auth).unwrap();
                let mut login = super::super::native_login(
                    Provider::Codex,
                    None,
                    Some(dir.path().to_string_lossy().into_owned()),
                )
                .unwrap();
                login.native_executable = Some(dir.path().join("must-never-launch"));
                let lease = Arc::new(
                    Arc::new(tokio::sync::Semaphore::new(1))
                        .acquire_owned()
                        .await
                        .unwrap(),
                );
                // Enter immediately after the separately-tested idle preflight;
                // neither malformed auth nor an unavailable CLI may win here.
                let failure = collect_codex_when_idle(&login, lease).await.unwrap_err();
                assert_eq!(failure.reason, "native_storage_unsupported");
                assert_eq!(failure.identity, None);
            }
        }
    }

    #[tokio::test]
    async fn native_success_keeps_account_and_email_evidence_for_partial_failures() {
        let mut identity = None;
        let reading = fixture_rpc(vec![json!({"id":1,"result":{}}),
            json!({"id":2,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}}),
            json!({"id":3,"result":{"accountId":"fixture-account","rateLimits":{"primary":{"usedPercent":20}}}}),
            json!({"id":4,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}})], &mut identity).await.unwrap();
        assert_eq!(
            reading.identity.account,
            Some(fingerprint(b"fixture-account"))
        );
        assert_eq!(
            reading.identity.email,
            Some(fingerprint(b"fixture@example.test"))
        );
        let failure = fixture_rpc(vec![json!({"id":1,"result":{}}),
            json!({"id":2,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}}),
            json!({"id":3,"error":{"code":503,"message":"unavailable"}})], &mut None).await.unwrap_err();
        assert!(!reading.identity.conflicts_with(&failure.identity.unwrap()));
    }

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
            "native_storage_unsupported"
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

    async fn fixture_rpc(
        lines: Vec<Value>,
        identity: &mut Option<Identity>,
    ) -> Result<Reading, ProbeFailure> {
        let (client, mut server) = tokio::io::duplex(MAX_BYTES * 2 + 8192);
        let mut transcript = Vec::new();
        for line in lines {
            transcript.extend(serde_json::to_vec(&line).unwrap());
            transcript.push(b'\n');
        }
        server.write_all(&transcript).await.unwrap();
        server.shutdown().await.unwrap();
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader.take((MAX_BYTES + 1) as u64));
        codex_rpc(&mut writer, &mut reader, identity, None).await
    }

    #[tokio::test]
    async fn codex_rate_limit_failure_belongs_to_native_account_not_old_file() {
        let mut identity = Some(Identity::from(fingerprint(b"old-file-account")));
        let error = fixture_rpc(vec![json!({"id":1,"result":{}}),
            json!({"id":2,"result":{"account":{"type":"chatgpt","email":"new@example.test","planType":"pro"}}}),
            json!({"id":3,"error":{"message":"fixture quota unavailable"}})], &mut identity).await.unwrap_err();
        assert_eq!(error.reason, "unavailable");
        assert_eq!(
            error.identity,
            Some(Identity {
                account: None,
                email: Some(fingerprint(b"new@example.test")),
                organization: None
            })
        );
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
            vec!["primary", "secondary"]
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
}
