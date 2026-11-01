//! Manual, read-only Claude OAuth usage. Native credentials stay on this side
//! of IPC. The access token, never the refresh token, is kept in this app's
//! own Keychain item (`claude_token_cache`) so a Refresh does not reopen
//! Claude Code's item each time. Tokens are never renewed here, and there is
//! no cookie fallback. When OAuth cannot produce a reading, a user-pressed
//! Refresh falls back to the user's own `claude` CLI (`claude_cli`).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::OwnedSemaphorePermit;
use trace_commons_contributor_platform::external_claude_credentials as external;

use super::claude_cli::{self, CliProbe};
use super::claude_token_cache::TokenCache;
use super::providers::{
    bounded_json, failure, fingerprint, native_read, request_error, usage_client,
};
use super::{Identity, LoginSpec, ProbeFailure, Reading, ReadingSource, Window, file_revision};

const MAX_BYTES: usize = 1024 * 1024;
const NATIVE_TIMEOUT: Duration = Duration::from_secs(180);
const USAGE: &str = "https://api.anthropic.com/api/oauth/usage?cedar_ember=1";
const LEGACY_USAGE: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE: &str = "https://api.anthropic.com/api/oauth/profile";

pub(super) fn account_path(login: &LoginSpec) -> PathBuf {
    let config = login.root.join(".config.json");
    if config.is_file() {
        config
    } else if login.config_dir.as_deref().is_some_and(|v| !v.is_empty()) {
        login.root.join(".claude.json")
    } else {
        login
            .root
            .parent()
            .unwrap_or(&login.root)
            .join(".claude.json")
    }
}

fn credential_path(login: &LoginSpec) -> PathBuf {
    let root = match login.secure_storage_dir.as_deref() {
        Some("") => dirs::home_dir()
            .map(|home| home.join(".claude"))
            .unwrap_or_else(|| login.root.clone()),
        Some(dir) => PathBuf::from(dir),
        None => login.root.clone(),
    };
    root.join(".credentials.json")
}

pub(super) fn metadata_revision(login: &LoginSpec) -> String {
    // File attributes only. In particular, status never calls Security.framework.
    format!(
        "{}|{}|{}",
        file_revision(&credential_path(login)),
        file_revision(&account_path(login)),
        file_revision(&login.root.join(".config.json"))
    )
}

/// A CLI probe makes Claude Code rewrite its own `.claude.json` (start count,
/// the probe directory's trust entry), which changes `metadata_revision`.
/// Left alone, that would look like an account change: the service would
/// replace the login's entry mid-probe and cancel the probe that caused it.
/// While a probe runs, and afterwards for as long as the files stay exactly
/// as it left them, the revision published to the service is the one from
/// before the probe. The next real change ends the alias.
enum ProbeAlias {
    Running { before: String },
    Settled { before: String, after: String },
}

fn probe_aliases() -> &'static Mutex<BTreeMap<String, ProbeAlias>> {
    static ALIASES: OnceLock<Mutex<BTreeMap<String, ProbeAlias>>> = OnceLock::new();
    ALIASES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// The revision the service compares: `metadata_revision`, except while a
/// probe's own writes are being masked.
pub(super) fn published_revision(login: &LoginSpec) -> String {
    let actual = metadata_revision(login);
    let Ok(mut aliases) = probe_aliases().lock() else {
        return actual;
    };
    match aliases.get(&login.id) {
        None => actual,
        Some(ProbeAlias::Running { before }) => before.clone(),
        Some(ProbeAlias::Settled { before, after }) if *after == actual => before.clone(),
        Some(ProbeAlias::Settled { .. }) => {
            aliases.remove(&login.id);
            actual
        }
    }
}

/// Masks a running probe's writes until settled or dropped. Dropping it
/// unsettled (failure, cancellation, panic) removes the alias, so a real
/// change made during the probe is seen at once.
struct ProbeRevisionGuard {
    login: String,
    before: String,
    settled: bool,
}

impl ProbeRevisionGuard {
    fn begin(login: &LoginSpec) -> Self {
        let before = published_revision(login);
        if let Ok(mut aliases) = probe_aliases().lock() {
            aliases.insert(
                login.id.clone(),
                ProbeAlias::Running {
                    before: before.clone(),
                },
            );
        }
        Self {
            login: login.id.clone(),
            before,
            settled: false,
        }
    }

    /// Keep masking exactly the files as they are now.
    fn settle(mut self, after: String) -> String {
        if let Ok(mut aliases) = probe_aliases().lock() {
            aliases.insert(
                self.login.clone(),
                ProbeAlias::Settled {
                    before: self.before.clone(),
                    after,
                },
            );
        }
        self.settled = true;
        self.before.clone()
    }
}

impl Drop for ProbeRevisionGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        if let Ok(mut aliases) = probe_aliases().lock() {
            aliases.remove(&self.login);
        }
    }
}

fn service(login: &LoginSpec) -> Result<String, &'static str> {
    let directory = login
        .secure_storage_dir
        .as_deref()
        .or(login.config_dir.as_deref());
    match directory.filter(|dir| !dir.is_empty()) {
        None => Ok("Claude Code-credentials".into()),
        Some(dir) => Ok(format!(
            "Claude Code-credentials-{}",
            &fingerprint(external::normalized_profile_directory(dir)?.as_bytes())[..8]
        )),
    }
}

// Deserialize only the required OAuth fields. Unknown refresh tokens and MCP
// secrets are skipped, never materialized as serde_json::Value or debug output.
#[derive(Deserialize)]
struct CredentialDocument<'a> {
    #[serde(rename = "claudeAiOauth", borrow)]
    oauth: Option<OAuth<'a>>,
}
#[derive(Deserialize)]
struct OAuth<'a> {
    #[serde(rename = "accessToken")]
    access_token: &'a str,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    scopes: Option<Vec<String>>,
}
/// Claude Code's own Keychain item, behind a seam so a test never reaches the
/// real one. Production delegates to `external_claude_credentials`.
pub(super) trait NativeKeychain: Send + Sync {
    /// Attribute-only item revision; never decrypts and never shows UI.
    fn version(&self, service: &str) -> Result<Option<String>, &'static str>;
    /// The item's payload. With `allow_ui` false the query refuses to show a
    /// dialog and reports `external::INTERACTION_REQUIRED` when one is needed.
    fn read(
        &self,
        service: &str,
        allow_ui: bool,
    ) -> Result<Option<external::SecretBytes>, &'static str>;
}

struct ClaudeCodeKeychain;
impl NativeKeychain for ClaudeCodeKeychain {
    fn version(&self, service: &str) -> Result<Option<String>, &'static str> {
        external::version(service)
    }
    fn read(
        &self,
        service: &str,
        allow_ui: bool,
    ) -> Result<Option<external::SecretBytes>, &'static str> {
        external::read(service, allow_ui)
    }
}

/// What a Claude collection reads through.
#[derive(Clone)]
pub(super) struct Deps {
    pub keychain: Arc<dyn NativeKeychain>,
    /// Shared with `UsageService`, which clears it on unenroll and when the
    /// Claude switch is turned off.
    pub cache: Arc<TokenCache>,
    /// The user's `claude` CLI, for the `/usage` fallback.
    pub cli: Arc<dyn CliProbe>,
}

impl Deps {
    pub(super) fn for_build(cache: Arc<TokenCache>) -> Self {
        Self {
            keychain: Arc::new(ClaudeCodeKeychain),
            cache,
            cli: claude_cli::for_build(),
        }
    }
}

/// A fingerprint of the exact login spec; a changed config or secure-storage
/// directory, root or login names a different cache entry.
fn login_key(login: &LoginSpec) -> String {
    fingerprint(
        format!(
            "claude-usage-login\0{}\0{}\0{:?}\0{:?}",
            login.id,
            login.root.to_string_lossy(),
            login.config_dir,
            login.secure_storage_dir
        )
        .as_bytes(),
    )[..32]
        .to_owned()
}

/// What a cached token is bound to: the Keychain service, the local account
/// evidence, and the login spec. An account switch changes it. The item's
/// revision is deliberately not part of it: Claude Code renewing its own
/// token moves the revision without changing the account, and the cache
/// tracks the revision separately (`TokenCache::confirm`).
fn cache_binding(service: &str, local: &Identity, login_key: &str) -> String {
    fingerprint(
        format!(
            "claude-usage-binding-v2\0{service}\0{:?}\0{:?}\0{:?}\0{login_key}",
            local.account, local.email, local.organization
        )
        .as_bytes(),
    )
}

/// Silent first: the Security query itself refuses to show a dialog. Only an
/// interaction-required answer on a user-pressed Refresh earns a second read
/// that may prompt. A Deny or failed authentication is final for this
/// Refresh, so it is never followed by a second dialog.
fn read_keychain(
    keychain: &dyn NativeKeychain,
    service: &str,
    interactive: bool,
) -> Result<Option<external::SecretBytes>, &'static str> {
    match keychain.read(service, false) {
        Err(reason) if reason == external::INTERACTION_REQUIRED => {
            if !interactive {
                return Err("keychain_needs_permission");
            }
            keychain.read(service, true).map_err(|reason| {
                if reason == external::INTERACTION_REQUIRED {
                    "keychain_needs_permission"
                } else {
                    reason
                }
            })
        }
        other => other,
    }
}

struct Credential {
    token: external::SecretBytes,
    source: Source,
    revision: String,
    local_identity: Identity,
}
enum Source {
    File,
    Keychain {
        service: String,
        /// `None` when the item was not consulted: a background load in a
        /// build that serves its in-memory copy without opening the item.
        revision: Option<String>,
        login_key: String,
    },
}

fn parse_credential(
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<external::SecretBytes, &'static str> {
    parse_oauth(bytes, now).map(|(token, _)| token)
}

/// The access token and its `expiresAt` (milliseconds). Only these fields
/// are read; the refresh token is skipped by the deserializer.
fn parse_oauth(
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<(external::SecretBytes, Option<i64>), &'static str> {
    let document: CredentialDocument = serde_json::from_slice(bytes).map_err(|_| "auth_missing")?;
    let oauth = document.oauth.ok_or("auth_missing")?;
    if oauth.access_token.is_empty()
        || oauth.access_token.len() > 8192
        || oauth.access_token.chars().any(char::is_control)
    {
        return Err("auth_missing");
    }
    if oauth
        .expires_at
        .is_some_and(|expiry| DateTime::from_timestamp_millis(expiry).is_none_or(|at| at <= now))
    {
        return Err("auth_expired");
    }
    if oauth
        .scopes
        .is_some_and(|scopes| !scopes.iter().any(|scope| scope == "user:profile"))
    {
        return Err("scope_missing");
    }
    Ok((
        external::SecretBytes::new(oauth.access_token.as_bytes().to_vec()),
        oauth.expires_at,
    ))
}

fn id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.chars().any(char::is_control))
        .map(|v| fingerprint(v.to_ascii_lowercase().as_bytes()))
}

fn organization_id(value: Option<&Value>) -> Option<String> {
    id(value).map(|hash| fingerprint(format!("claude:organization:{hash}").as_bytes()))
}

fn profile_identity(value: &Value) -> Result<Identity, &'static str> {
    let identity = Identity {
        organization: organization_id(
            value
                .pointer("/organization/uuid")
                .or_else(|| value.get("organizationUuid"))
                .or_else(|| value.get("organization_uuid")),
        ),
        account: id(value
            .pointer("/account/uuid")
            .or_else(|| value.get("account_uuid"))
            .or_else(|| value.get("accountUuid"))),
        email: id(value
            .pointer("/account/email_address")
            .or_else(|| value.get("email_address"))
            .or_else(|| value.get("emailAddress"))),
    };
    // Usage without stable provider identity cannot establish the capture owner.
    if identity.account.is_none() {
        return Err("identity_unavailable");
    }
    Ok(identity)
}

fn read_secret_file(path: &Path) -> Result<external::SecretBytes, &'static str> {
    let file = trace_commons_contributor_platform::local_files::open_import_file(path)
        .map_err(|_| "unavailable")?;
    let mut bytes = Vec::new();
    let result = file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes);
    let bytes = external::SecretBytes::new(bytes);
    result.map_err(|_| "unavailable")?;
    if bytes.len() > MAX_BYTES {
        return Err("invalid_response");
    }
    Ok(bytes)
}

fn local_identity(login: &LoginSpec) -> Result<Identity, &'static str> {
    #[derive(Deserialize)]
    struct Config {
        #[serde(rename = "oauthAccount")]
        account: Option<Account>,
    }
    #[derive(Deserialize)]
    struct Account {
        #[serde(rename = "accountUuid")]
        uuid: Option<String>,
        #[serde(rename = "emailAddress")]
        email: Option<String>,
        #[serde(rename = "organizationUuid")]
        organization: Option<String>,
    }
    fn fingerprint_field(value: Option<String>) -> Option<String> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.chars().any(char::is_control))
            .map(|v| fingerprint(v.to_ascii_lowercase().as_bytes()))
    }
    let path = account_path(login);
    if !path.exists() {
        return Ok(Identity::default());
    }
    let bytes = read_secret_file(&path)?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| "identity_unavailable")?;
    let account = config.account.ok_or("auth_missing")?;
    Ok(Identity {
        account: fingerprint_field(account.uuid),
        email: fingerprint_field(account.email),
        organization: fingerprint_field(account.organization)
            .map(|hash| fingerprint(format!("claude:organization:{hash}").as_bytes())),
    })
}

fn corroborate_profile(local: &Identity, remote: &Identity) -> Result<(), &'static str> {
    if local.conflicts_with(remote) {
        return Err("auth_changed");
    }
    // Missing organization evidence is unknown, not proof that an identified
    // local organization matches the selected OAuth credential.
    if local.organization.is_some() && remote.organization.is_none() {
        return Err("identity_unavailable");
    }
    Ok(())
}

fn load(login: &LoginSpec, interactive: bool, deps: &Deps) -> Result<Credential, &'static str> {
    let revision = published_revision(login);
    let local_identity = local_identity(login)?;
    let path = credential_path(login);
    let (token, source) = if path.exists() {
        // A present selected-profile file is authoritative, even when expired.
        // Never substitute a different legacy credential on parse/auth failure.
        (
            parse_credential(&read_secret_file(&path)?, Utc::now())?,
            Source::File,
        )
    } else {
        load_keychain(login, &local_identity, interactive, deps)?
    };
    if published_revision(login) != revision {
        return Err("auth_changed");
    }
    Ok(Credential {
        token,
        source,
        revision,
        local_identity,
    })
}

/// The token for a Keychain-backed login. A cached token is served while it
/// is valid. When Claude Code has written its item since (it renews its own
/// token in place, which keeps the item's access list), one read that cannot
/// show a dialog decides whether the item now holds a different token. Any
/// failure of that read keeps the cached token: only expiry, Anthropic
/// rejecting it, or an account or login change ends it. A build without the
/// keychain entitlement keeps its cache in memory only and does not open
/// Claude Code's item at all off a pressed Refresh.
fn load_keychain(
    login: &LoginSpec,
    local_identity: &Identity,
    interactive: bool,
    deps: &Deps,
) -> Result<(external::SecretBytes, Source), &'static str> {
    let service = service(login)?;
    let key = login_key(login);
    let binding = cache_binding(&service, local_identity, &key);
    let now = Utc::now();
    // The first store operation, so `unentitled()` is known after it.
    let cached = deps.cache.lookup(&key, &binding, now);
    if !interactive && deps.cache.unentitled() {
        let cached = cached.ok_or("keychain_needs_permission")?;
        return Ok((
            cached.token,
            Source::Keychain {
                service,
                revision: None,
                login_key: key,
            },
        ));
    }
    // Attribute-only and prompt-free.
    let before = deps.keychain.version(&service)?.ok_or("auth_missing")?;
    let source = |service: String| Source::Keychain {
        service,
        revision: Some(before.clone()),
        login_key: key.clone(),
    };
    if let Some(cached) = cached {
        if cached.revision == before {
            return Ok((cached.token, source(service)));
        }
        let epoch = deps.cache.epoch();
        let token = match reread_silently(deps.keychain.as_ref(), &service, &before, now) {
            Some((token, _)) if token.as_ref() == cached.token.as_ref() => {
                deps.cache.confirm(epoch, &key, &binding, &before);
                cached.token
            }
            Some((token, Some(expires_at))) => {
                deps.cache
                    .remember(epoch, &key, &binding, &before, expires_at, &token);
                token
            }
            // A different token without a stated expiry has no safe reuse
            // window; serve it and drop the old one.
            Some((token, None)) => {
                deps.cache.forget(&key);
                token
            }
            None => cached.token,
        };
        return Ok((token, source(service)));
    }
    let epoch = deps.cache.epoch();
    let bytes =
        read_keychain(deps.keychain.as_ref(), &service, interactive)?.ok_or("auth_missing")?;
    if deps.keychain.version(&service)?.as_ref() != Some(&before) {
        return Err("auth_changed");
    }
    let (token, expires_at) = parse_oauth(&bytes, now)?;
    // Without a stated expiry there is no safe reuse window.
    if let Some(expires_at) = expires_at {
        deps.cache
            .remember(epoch, &key, &binding, &before, expires_at, &token);
    }
    Ok((token, source(service)))
}

/// The item's current token, read without any possibility of a dialog, or
/// `None` when that read fails, the item moved again during it, or what it
/// holds is not a usable token. Never escalates to a UI read.
fn reread_silently(
    keychain: &dyn NativeKeychain,
    service: &str,
    before: &str,
    now: DateTime<Utc>,
) -> Option<(external::SecretBytes, Option<i64>)> {
    let bytes = keychain.read(service, false).ok()??;
    if keychain.version(service).ok()?.as_deref() != Some(before) {
        return None;
    }
    parse_oauth(&bytes, now).ok()
}

fn unchanged(
    login: &LoginSpec,
    credential: &Credential,
    deps: &Deps,
) -> Result<bool, &'static str> {
    if published_revision(login) != credential.revision {
        return Ok(false);
    }
    match &credential.source {
        Source::File => Ok(true),
        Source::Keychain { revision: None, .. } => Ok(true),
        Source::Keychain {
            service,
            revision: Some(revision),
            ..
        } => Ok(deps.keychain.version(service)?.as_ref() == Some(revision)),
    }
}

#[derive(Clone, Copy)]
enum Endpoint {
    Usage,
    LegacyUsage,
    Profile,
}
fn request(client: &reqwest::Client, token: &str, endpoint: Endpoint) -> reqwest::RequestBuilder {
    client
        .get(match endpoint {
            Endpoint::Usage => USAGE,
            Endpoint::LegacyUsage => LEGACY_USAGE,
            Endpoint::Profile => PROFILE,
        })
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
}
fn status(status: u16) -> Result<(), &'static str> {
    match status {
        200 => Ok(()),
        401 | 403 => Err("auth_expired"),
        429 => Err("rate_limited"),
        _ => Err("unavailable"),
    }
}
async fn body(response: reqwest::Response) -> Result<Value, &'static str> {
    status(response.status().as_u16())?;
    bounded_json(response).await
}

/// OAuth failures the `claude` CLI can still answer: no readable credential,
/// a Keychain permission need or Deny, or a token Anthropic refused. Not
/// rate limits, timeouts or network failures, which the CLI would hit too.
const CLI_FALLBACK: &[&str] = &[
    "auth_missing",
    "auth_expired",
    "keychain_needs_permission",
    "scope_missing",
];
const CLI_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) async fn collect(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
    deps: &Deps,
) -> Result<Reading, ProbeFailure> {
    let failure = match collect_oauth(login, interactive, Arc::clone(&lease), deps).await {
        Ok(reading) => return Ok(reading),
        Err(failure) => failure,
    };
    // Only a user-pressed Refresh may launch the CLI.
    if !interactive || !CLI_FALLBACK.contains(&failure.reason) {
        return Err(failure);
    }
    // The CLI's own failure labels are internal; the OAuth reason still says
    // what the person can do about it.
    collect_cli(login, lease, deps)
        .await
        .map_err(|reason| match reason {
            "auth_changed" => super::providers::failure(reason, None),
            _ => failure,
        })
}

async fn collect_cli(
    login: &LoginSpec,
    lease: Arc<OwnedSemaphorePermit>,
    deps: &Deps,
) -> Result<Reading, &'static str> {
    let spec = login.clone();
    let cli = Arc::clone(&deps.cli);
    // The whole probe, including the child's lifetime, belongs to the leased
    // native thread: dropping this future cannot orphan the process.
    let (screen, revision, identity) = tokio::time::timeout(
        CLI_TIMEOUT,
        native_read(lease, move || {
            let identity = local_identity(&spec)?;
            let guard = ProbeRevisionGuard::begin(&spec);
            let screen = cli.usage_screen(&spec)?;
            // An account switch while the probe ran is not masked.
            if local_identity(&spec)? != identity {
                return Err("auth_changed");
            }
            let revision = guard.settle(metadata_revision(&spec));
            Ok::<_, &'static str>((screen, revision, identity))
        }),
    )
    .await
    .map_err(|_| "cli_timeout")???;
    let windows = claude_cli::parse_usage(
        &screen,
        chrono::Local::now(),
        claude_cli::local_zone_name().as_deref(),
    )?;
    Ok(Reading {
        windows,
        banked_resets: None,
        identity,
        native_revision: Some(revision),
        source: ReadingSource::Cli,
    })
}

async fn collect_oauth(
    login: &LoginSpec,
    interactive: bool,
    lease: Arc<OwnedSemaphorePermit>,
    deps: &Deps,
) -> Result<Reading, ProbeFailure> {
    let spec = login.clone();
    let load_deps = deps.clone();
    let credential = tokio::time::timeout(
        NATIVE_TIMEOUT,
        native_read(Arc::clone(&lease), move || {
            load(&spec, interactive, &load_deps)
        }),
    )
    .await
    .map_err(|_| failure("credential_read_timeout", None))?
    .map_err(|reason| failure(reason, None))?
    .map_err(|reason| failure(reason, None))?;
    let client = usage_client().map_err(|reason| failure(reason, None))?;
    // Requests use one token and a required stable profile identity. The local
    // config is corroboration only and never substitutes for provider evidence.
    let mut observed_identity = None;
    let result = async {
        let profile = body(
            request(
                &client,
                std::str::from_utf8(&credential.token).map_err(|_| "auth_missing")?,
                Endpoint::Profile,
            )
            .send()
            .await
            .map_err(request_error)?,
        )
        .await?;
        let identity = profile_identity(&profile)?;
        observed_identity = Some(identity.clone());
        corroborate_profile(&credential.local_identity, &identity)?;
        let mut response = request(
            &client,
            std::str::from_utf8(&credential.token).map_err(|_| "auth_missing")?,
            Endpoint::Usage,
        )
        .send()
        .await
        .map_err(request_error)?;
        if matches!(response.status().as_u16(), 400 | 403) {
            response = request(
                &client,
                std::str::from_utf8(&credential.token).map_err(|_| "auth_missing")?,
                Endpoint::LegacyUsage,
            )
            .send()
            .await
            .map_err(request_error)?;
        }
        parse_usage(&body(response).await?, identity)
    }
    .await;
    let spec = login.clone();
    let revision = credential.revision.clone();
    let check_deps = deps.clone();
    let outcome = result.as_ref().map(|_| ()).map_err(|reason| *reason);
    let same = tokio::time::timeout(
        Duration::from_secs(10),
        native_read(lease, move || {
            let same = unchanged(&spec, &credential, &check_deps);
            if let Source::Keychain { login_key, .. } = &credential.source {
                if forget_after_request(&outcome, &same) {
                    check_deps.cache.forget(login_key);
                }
            }
            same
        }),
    )
    .await
    .map_err(|_| failure("credential_check_timeout", None))?
    .map_err(|reason| failure(reason, None))?
    .map_err(|reason| failure(reason, None))?;
    if !same {
        return Err(failure("auth_changed", None));
    }
    let mut reading = result.map_err(|reason| failure(reason, observed_identity))?;
    reading.native_revision = Some(revision);
    Ok(reading)
}

/// Whether a request's outcome drops the cached token: only a token
/// Anthropic refused (401 and 403 are `auth_expired`), or one whose account
/// no longer matches. Claude Code writing its item during the request
/// (`same` not `Ok(true)`) still fails this reading as `auth_changed`, but
/// leaves the cache to the next load's silent check, so a renewal Claude Code
/// made in place does not discard a token Anthropic just accepted.
fn forget_after_request<T>(
    result: &Result<T, &'static str>,
    _same: &Result<bool, &'static str>,
) -> bool {
    matches!(
        result,
        Err("auth_expired" | "auth_changed" | "scope_missing" | "identity_unavailable")
    )
}

fn percent(value: Option<&Value>) -> Option<u16> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        .map(|n| (n * 10.0).round() as u16)
}
fn reset(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|date| date.with_timezone(&Utc))
}
fn parse_usage(value: &Value, identity: Identity) -> Result<Reading, &'static str> {
    if !value.is_object() {
        return Err("invalid_response");
    }
    let mut windows = Vec::new();
    for (key, lane, label, duration) in [
        ("five_hour", "session", None, 300),
        ("seven_day", "weekly", None, 10080),
        ("seven_day_opus", "model", Some("Opus"), 10080),
        ("seven_day_sonnet", "model", Some("Sonnet"), 10080),
    ] {
        if let Some(window) = value.get(key).filter(|v| v.is_object()) {
            windows.push(Window {
                lane: lane.into(),
                label: label.map(str::to_owned),
                duration_mins: Some(duration),
                used_permille: percent(window.get("utilization")),
                resets_at: reset(window.get("resets_at")),
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
            let model = entry
                .pointer("/scope/model/id")
                .and_then(Value::as_str)
                .unwrap_or(label);
            if label.eq_ignore_ascii_case("all models")
                || model.ends_with("all-models")
                || windows
                    .iter()
                    .any(|w| w.lane == "model" && w.label.as_deref() == Some(label))
            {
                continue;
            }
            windows.push(Window {
                lane: "model".into(),
                label: Some(label.into()),
                duration_mins: Some(10080),
                used_permille: percent(entry.get("percent")),
                resets_at: reset(entry.get("resets_at")),
            });
        }
    }
    windows.truncate(16);
    Ok(Reading {
        windows,
        banked_resets: None,
        identity,
        native_revision: None,
        source: ReadingSource::Api,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn test_deps() -> Deps {
        Deps::for_build(Arc::new(TokenCache::for_build()))
    }
    fn login(root: &std::path::Path) -> LoginSpec {
        LoginSpec {
            provider: super::super::Provider::Claude,
            root: root.into(),
            id: "claude-fixture".into(),
            label: "Claude".into(),
            default_login: false,
            native_executable: None,
            config_dir: Some(root.to_string_lossy().into()),
            secure_storage_dir: None,
            metadata_revision: None,
            native_revision: None,
        }
    }
    /// A stand-in for Claude Code's Keychain item. Records whether each
    /// payload read allowed UI; never reaches Security.framework.
    pub(in super::super) struct FakeKeychain {
        pub version: std::sync::Mutex<Option<String>>,
        pub silent: std::sync::Mutex<Result<Option<Vec<u8>>, &'static str>>,
        pub with_ui: std::sync::Mutex<Result<Option<Vec<u8>>, &'static str>>,
        pub reads: std::sync::Mutex<Vec<bool>>,
        pub versions: std::sync::Mutex<usize>,
    }
    impl FakeKeychain {
        pub(in super::super) fn new(
            silent: Result<Option<Vec<u8>>, &'static str>,
            with_ui: Result<Option<Vec<u8>>, &'static str>,
        ) -> Arc<Self> {
            Arc::new(Self {
                version: std::sync::Mutex::new(Some("item-1".into())),
                silent: std::sync::Mutex::new(silent),
                with_ui: std::sync::Mutex::new(with_ui),
                reads: std::sync::Mutex::new(Vec::new()),
                versions: std::sync::Mutex::new(0),
            })
        }
        pub(in super::super) fn reads(&self) -> Vec<bool> {
            self.reads.lock().unwrap().clone()
        }
        pub(in super::super) fn versions(&self) -> usize {
            *self.versions.lock().unwrap()
        }
    }
    impl NativeKeychain for FakeKeychain {
        fn version(&self, _service: &str) -> Result<Option<String>, &'static str> {
            *self.versions.lock().unwrap() += 1;
            Ok(self.version.lock().unwrap().clone())
        }
        fn read(
            &self,
            _service: &str,
            allow_ui: bool,
        ) -> Result<Option<external::SecretBytes>, &'static str> {
            self.reads.lock().unwrap().push(allow_ui);
            let answer = if allow_ui {
                self.with_ui.lock().unwrap().clone()
            } else {
                self.silent.lock().unwrap().clone()
            };
            answer.map(|bytes| bytes.map(external::SecretBytes::new))
        }
    }
    fn deps_with(keychain: Arc<FakeKeychain>) -> Deps {
        Deps {
            keychain,
            cache: Arc::new(TokenCache::for_build()),
            cli: Arc::new(claude_cli::NoCli),
        }
    }
    const KEYCHAIN_DOC: &[u8] = br#"{"claudeAiOauth":{"accessToken":"fixture-token","expiresAt":4102444800000,"refreshToken":"never-used"}}"#;

    #[test]
    fn a_silent_keychain_read_needs_no_dialog_even_off_a_user_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(Ok(Some(KEYCHAIN_DOC.to_vec())), Err("unexpected"));
        let credential = load(&spec, false, &deps_with(Arc::clone(&keychain)))
            .ok()
            .unwrap();
        assert_eq!(credential.token.as_ref(), b"fixture-token");
        assert_eq!(keychain.reads(), [false]);
    }

    #[test]
    fn only_an_interaction_need_on_a_user_refresh_earns_a_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Ok(Some(KEYCHAIN_DOC.to_vec())),
        );
        assert!(load(&spec, true, &deps_with(Arc::clone(&keychain))).is_ok());
        assert_eq!(keychain.reads(), [false, true]);

        // Not a user-pressed Refresh: never a dialog.
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Ok(Some(KEYCHAIN_DOC.to_vec())),
        );
        assert!(matches!(
            load(&spec, false, &deps_with(Arc::clone(&keychain))),
            Err("keychain_needs_permission")
        ));
        assert_eq!(keychain.reads(), [false]);

        // A Deny or failed authentication is final: no second dialog.
        let keychain = FakeKeychain::new(
            Err("keychain_needs_permission"),
            Ok(Some(KEYCHAIN_DOC.to_vec())),
        );
        assert!(matches!(
            load(&spec, true, &deps_with(Arc::clone(&keychain))),
            Err("keychain_needs_permission")
        ));
        assert_eq!(keychain.reads(), [false]);

        // The dialog itself refused: still a permission need, never the
        // internal label.
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Err(external::INTERACTION_REQUIRED),
        );
        assert!(matches!(
            load(&spec, true, &deps_with(keychain)),
            Err("keychain_needs_permission")
        ));
    }

    #[test]
    fn a_cached_token_spares_the_next_refresh_any_keychain_payload_read() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Ok(Some(KEYCHAIN_DOC.to_vec())),
        );
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, true, &deps).is_ok());
        assert_eq!(keychain.reads(), [false, true]);
        // Next Refresh, even a background one: served from the app's own
        // item, so Claude Code's item is not opened and nothing prompts.
        let again = load(&spec, false, &deps).ok().unwrap();
        assert_eq!(again.token.as_ref(), b"fixture-token");
        assert_eq!(keychain.reads(), [false, true]);
    }

    #[test]
    fn a_switched_account_or_login_invalidates_the_cached_token() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(Ok(Some(KEYCHAIN_DOC.to_vec())), Err("unexpected"));
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 1);

        // Claude Code renewed its item in place: one silent read finds the
        // same token, which is confirmed at the new revision and not read
        // again.
        *keychain.version.lock().unwrap() = Some("item-2".into());
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 2);
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 2);

        // A different local account under the same item revision.
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"account-b"}}"#,
        )
        .unwrap();
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 3);

        // A different login spec never shares the entry.
        let mut other = spec.clone();
        other.secure_storage_dir = Some(dir.path().join("other").to_string_lossy().into());
        assert!(load(&other, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 4);
    }

    const RENEWED_DOC: &[u8] = br#"{"claudeAiOauth":{"accessToken":"fixture-token-2","expiresAt":4102444800000,"refreshToken":"never-used"}}"#;

    #[test]
    fn a_rewritten_item_keeps_the_cached_token_when_no_silent_read_is_possible() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Ok(Some(KEYCHAIN_DOC.to_vec())),
        );
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, true, &deps).is_ok());
        assert_eq!(keychain.reads(), [false, true]);

        // Claude Code renewed its token in place: the revision moved, and a
        // read without a dialog is refused.
        *keychain.version.lock().unwrap() = Some("item-2".into());
        let again = load(&spec, false, &deps).ok().unwrap();
        assert_eq!(again.token.as_ref(), b"fixture-token");
        assert_eq!(keychain.reads(), [false, true, false]);

        // Nor does a pressed Refresh ask with UI while the cached token is
        // still good.
        let again = load(&spec, true, &deps).ok().unwrap();
        assert_eq!(again.token.as_ref(), b"fixture-token");
        assert_eq!(keychain.reads(), [false, true, false, false]);

        // Any other failure of that read keeps the cached token too.
        *keychain.silent.lock().unwrap() = Err("unavailable");
        let again = load(&spec, false, &deps).ok().unwrap();
        assert_eq!(again.token.as_ref(), b"fixture-token");
    }

    #[test]
    fn a_rewritten_item_replaces_the_cached_token_only_when_it_holds_a_different_one() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(Ok(Some(KEYCHAIN_DOC.to_vec())), Err("unexpected"));
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, false, &deps).is_ok());
        let key = login_key(&spec);
        let binding = cache_binding(&service(&spec).unwrap(), &Identity::default(), &key);

        // Same token under a new revision: the entry is confirmed, not
        // rewritten, and later loads do not read the item.
        *keychain.version.lock().unwrap() = Some("item-2".into());
        assert_eq!(
            load(&spec, false, &deps).ok().unwrap().token.as_ref(),
            b"fixture-token"
        );
        assert_eq!(
            deps.cache
                .lookup(&key, &binding, Utc::now())
                .unwrap()
                .revision,
            "item-2"
        );
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 2);

        // A different token: it replaces the cached one.
        *keychain.version.lock().unwrap() = Some("item-3".into());
        *keychain.silent.lock().unwrap() = Ok(Some(RENEWED_DOC.to_vec()));
        assert_eq!(
            load(&spec, false, &deps).ok().unwrap().token.as_ref(),
            b"fixture-token-2"
        );
        let cached = deps.cache.lookup(&key, &binding, Utc::now()).unwrap();
        assert_eq!(cached.token.as_ref(), b"fixture-token-2");
        assert_eq!(cached.revision, "item-3");
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 3);
    }

    #[test]
    fn a_rejected_token_is_forgotten_and_only_a_pressed_refresh_may_ask_again() {
        // Anthropic answering 401 (or 403) is `auth_expired`.
        let rejected: Result<(), &'static str> = Err(status(401).unwrap_err());
        assert!(forget_after_request(&rejected, &Ok(true)));
        for reason in ["auth_changed", "scope_missing", "identity_unavailable"] {
            assert!(forget_after_request::<()>(&Err(reason), &Ok(true)));
        }
        for reason in ["rate_limited", "unavailable", "invalid_response"] {
            assert!(!forget_after_request::<()>(&Err(reason), &Ok(true)));
        }
        // Claude Code writing its item during the request is not a reason to
        // drop a token Anthropic accepted.
        assert!(!forget_after_request(&Ok(()), &Ok(false)));
        assert!(!forget_after_request(&Ok(()), &Err("unavailable")));

        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Ok(Some(KEYCHAIN_DOC.to_vec())),
        );
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, true, &deps).is_ok());
        deps.cache.forget(&login_key(&spec));
        // In the background: a silent attempt only, never a dialog.
        assert!(matches!(
            load(&spec, false, &deps),
            Err("keychain_needs_permission")
        ));
        assert_eq!(keychain.reads(), [false, true, false]);
        assert!(load(&spec, true, &deps).is_ok());
        assert_eq!(keychain.reads(), [false, true, false, false, true]);
    }

    /// The OS store of a build without the keychain entitlement.
    struct UnentitledStore;
    impl super::super::claude_token_cache::TokenStore for UnentitledStore {
        fn read(&self) -> Result<Option<external::SecretBytes>, &'static str> {
            Err(super::super::claude_token_cache::UNENTITLED)
        }
        fn write(&self, _bytes: &[u8]) -> Result<(), &'static str> {
            Err(super::super::claude_token_cache::UNENTITLED)
        }
        fn clear(&self) -> Result<(), &'static str> {
            Err(super::super::claude_token_cache::UNENTITLED)
        }
    }

    #[test]
    fn an_unentitled_build_never_opens_claudes_item_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(Ok(Some(KEYCHAIN_DOC.to_vec())), Err("unexpected"));
        let deps = Deps {
            keychain: Arc::clone(&keychain) as Arc<dyn NativeKeychain>,
            cache: Arc::new(TokenCache::new(Box::new(UnentitledStore))),
            cli: Arc::new(claude_cli::NoCli),
        };
        assert!(matches!(
            load(&spec, false, &deps),
            Err("keychain_needs_permission")
        ));
        assert!(deps.cache.unentitled());
        assert_eq!(keychain.versions(), 0);
        assert!(keychain.reads().is_empty());

        // A pressed Refresh reads it, and the token is kept in memory.
        assert!(load(&spec, true, &deps).is_ok());
        assert_eq!(keychain.reads(), [false]);
        let versions = keychain.versions();

        // Later background loads use that copy and touch nothing, even after
        // the item moved.
        *keychain.version.lock().unwrap() = Some("item-2".into());
        assert_eq!(
            load(&spec, false, &deps).ok().unwrap().token.as_ref(),
            b"fixture-token"
        );
        assert_eq!(keychain.versions(), versions);
        assert_eq!(keychain.reads(), [false]);
    }

    #[test]
    fn a_token_without_a_stated_expiry_is_never_cached() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        let keychain = FakeKeychain::new(
            Ok(Some(
                br#"{"claudeAiOauth":{"accessToken":"no-expiry"}}"#.to_vec(),
            )),
            Err("unexpected"),
        );
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, false, &deps).is_ok());
        assert!(load(&spec, false, &deps).is_ok());
        assert_eq!(keychain.reads().len(), 2);
    }

    #[test]
    fn a_file_credential_is_never_copied_into_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(credential_path(&spec), KEYCHAIN_DOC).unwrap();
        let keychain = FakeKeychain::new(Err("unexpected"), Err("unexpected"));
        let deps = deps_with(Arc::clone(&keychain));
        assert!(load(&spec, false, &deps).is_ok());
        let key = login_key(&spec);
        assert!(
            deps.cache
                .lookup(
                    &key,
                    &cache_binding(&service(&spec).unwrap(), &Identity::default(), &key),
                    Utc::now()
                )
                .is_none()
        );
        assert!(keychain.reads().is_empty());
    }

    /// A stand-in for the `claude` CLI: returns a fixed screen and can
    /// change the profile's files the way a real session does.
    struct FakeCli {
        screen: Result<String, &'static str>,
        rewrite: Option<(PathBuf, &'static [u8])>,
        calls: std::sync::Mutex<usize>,
    }
    impl CliProbe for FakeCli {
        fn usage_screen(&self, _login: &LoginSpec) -> Result<String, &'static str> {
            *self.calls.lock().unwrap() += 1;
            if let Some((path, bytes)) = &self.rewrite {
                std::fs::write(path, bytes).unwrap();
            }
            self.screen.clone()
        }
    }
    const CLI_PANEL: &str = "Current session\n 23% used\nCurrent week (all models)\n 70% left\n";
    fn cli_deps(
        keychain: Arc<FakeKeychain>,
        screen: Result<String, &'static str>,
        rewrite: Option<(PathBuf, &'static [u8])>,
    ) -> (Deps, Arc<FakeCli>) {
        let cli = Arc::new(FakeCli {
            screen,
            rewrite,
            calls: std::sync::Mutex::new(0),
        });
        let deps = Deps {
            keychain,
            cache: Arc::new(TokenCache::for_build()),
            cli: Arc::clone(&cli) as Arc<dyn CliProbe>,
        };
        (deps, cli)
    }
    async fn lease() -> Arc<OwnedSemaphorePermit> {
        Arc::new(
            Arc::new(tokio::sync::Semaphore::new(1))
                .acquire_owned()
                .await
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn a_denied_keychain_on_a_user_refresh_falls_back_to_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = login(dir.path());
        spec.id = "claude-cli-fallback".into();
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"account-a"}}"#,
        )
        .unwrap();
        let keychain = FakeKeychain::new(
            Err(external::INTERACTION_REQUIRED),
            Err("keychain_needs_permission"),
        );
        // Claude Code rewrites its config while the probe runs.
        let (deps, cli) = cli_deps(
            keychain,
            Ok(CLI_PANEL.into()),
            Some((
                account_path(&spec),
                br#"{"oauthAccount":{"accountUuid":"account-a"},"numStartups":2}"#,
            )),
        );
        let before = published_revision(&spec);
        let reading = collect(&spec, true, lease().await, &deps).await.unwrap();
        assert_eq!(*cli.calls.lock().unwrap(), 1);
        assert_eq!(reading.source, ReadingSource::Cli);
        assert_eq!(reading.windows[0].used_permille, Some(230));
        assert_eq!(reading.windows[1].used_permille, Some(300));
        assert!(reading.identity.account.is_some());
        // The probe's own write is not an account change for the service...
        assert_ne!(metadata_revision(&spec), before);
        assert_eq!(published_revision(&spec), before);
        assert_eq!(reading.native_revision.as_deref(), Some(before.as_str()));
        // ...until the files change again.
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"account-b"}}"#,
        )
        .unwrap();
        assert_eq!(published_revision(&spec), metadata_revision(&spec));
    }

    #[tokio::test]
    async fn the_cli_never_runs_off_a_background_refresh_or_for_other_failures() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = login(dir.path());
        spec.id = "claude-cli-gated".into();
        let keychain = FakeKeychain::new(Err(external::INTERACTION_REQUIRED), Err("unused"));
        let (deps, cli) = cli_deps(keychain, Ok(CLI_PANEL.into()), None);
        assert_eq!(
            collect(&spec, false, lease().await, &deps)
                .await
                .unwrap_err()
                .reason,
            "keychain_needs_permission"
        );
        assert_eq!(*cli.calls.lock().unwrap(), 0);

        // A Keychain failure that is not a permission need.
        let keychain = FakeKeychain::new(Err("unavailable"), Err("unused"));
        let (deps, cli) = cli_deps(keychain, Ok(CLI_PANEL.into()), None);
        assert_eq!(
            collect(&spec, true, lease().await, &deps)
                .await
                .unwrap_err()
                .reason,
            "unavailable"
        );
        assert_eq!(*cli.calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_failed_cli_reports_the_original_reason_and_unmasks_its_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = login(dir.path());
        spec.id = "claude-cli-failed".into();
        let keychain = FakeKeychain::new(Ok(None), Err("unused"));
        let (deps, cli) = cli_deps(
            keychain,
            Err("cli_signed_out"),
            Some((account_path(&spec), br#"{"oauthAccount":{}}"#)),
        );
        let failure = collect(&spec, true, lease().await, &deps)
            .await
            .unwrap_err();
        assert_eq!(failure.reason, "auth_missing");
        assert_eq!(*cli.calls.lock().unwrap(), 1);
        assert_eq!(published_revision(&spec), metadata_revision(&spec));

        // An account switch during the probe is reported as one.
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"account-a"}}"#,
        )
        .unwrap();
        let keychain = FakeKeychain::new(Ok(None), Err("unused"));
        let (deps, _) = cli_deps(
            keychain,
            Ok(CLI_PANEL.into()),
            Some((
                account_path(&spec),
                br#"{"oauthAccount":{"accountUuid":"account-b"}}"#,
            )),
        );
        let failure = collect(&spec, true, lease().await, &deps)
            .await
            .unwrap_err();
        assert_eq!(failure.reason, "auth_changed");
        assert_eq!(published_revision(&spec), metadata_revision(&spec));
    }

    #[test]
    fn credential_expiry_scope_and_mcp_only_are_not_false_sign_outs() {
        assert_eq!(parse_credential(br#"{"claudeAiOauth":{"accessToken":"fixture","expiresAt":1,"refreshToken":"ignored"}}"#, Utc::now()).unwrap_err(), "auth_expired");
        assert_eq!(
            parse_credential(
                br#"{"claudeAiOauth":{"accessToken":"fixture","scopes":["user:inference"]}}"#,
                Utc::now()
            )
            .unwrap_err(),
            "scope_missing"
        );
        assert_eq!(
            parse_credential(br#"{"mcpOAuth":{"secret":"ignored"}}"#, Utc::now()).unwrap_err(),
            "auth_missing"
        );
        assert_eq!(
            parse_credential(
                br#"{"claudeAiOauth":{"accessToken":"fixture","scopes":["user:profile"]}}"#,
                Utc::now()
            )
            .unwrap()
            .as_ref(),
            b"fixture"
        );
    }
    #[test]
    fn stable_profile_identity_survives_token_rotation_and_detects_account_switch() {
        let first = profile_identity(
            &json!({"account":{"uuid":"account-a","email_address":"a@example.invalid"}}),
        )
        .unwrap();
        let same = profile_identity(
            &json!({"accountUuid":"account-a", "emailAddress":"A@example.invalid"}),
        )
        .unwrap();
        let changed = profile_identity(&json!({"account_uuid":"account-b"})).unwrap();
        assert_eq!(first, same);
        assert!(first.conflicts_with(&changed));
        assert!(profile_identity(&json!({"emailAddress":"a@example.invalid"})).is_err());
    }
    #[test]
    fn same_account_different_organization_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"account-a","organizationUuid":"org-a"}}"#,
        )
        .unwrap();
        let local = local_identity(&spec).unwrap();
        let wrong = profile_identity(
            &json!({"account":{"uuid":"account-a"},"organization":{"uuid":"org-b"}}),
        )
        .unwrap();
        assert!(local.conflicts_with(&wrong));
        let missing = profile_identity(&json!({"account":{"uuid":"account-a"}})).unwrap();
        assert!(!local.conflicts_with(&missing));
        assert_eq!(
            corroborate_profile(&local, &missing),
            Err("identity_unavailable")
        );
        assert_eq!(corroborate_profile(&local, &wrong), Err("auth_changed"));
        let matching =
            profile_identity(&json!({"accountUuid":"account-a","organizationUuid":"org-a"}))
                .unwrap();
        assert_eq!(local, matching);
        assert!(corroborate_profile(&local, &matching).is_ok());
    }

    #[test]
    fn fixed_https_get_endpoints_never_put_tokens_in_urls() {
        let client = reqwest::Client::new();
        for endpoint in [Endpoint::Usage, Endpoint::LegacyUsage, Endpoint::Profile] {
            let req = request(&client, "fixture-secret", endpoint)
                .build()
                .unwrap();
            assert_eq!(req.method(), reqwest::Method::GET);
            assert_eq!(req.url().scheme(), "https");
            assert_eq!(req.url().host_str(), Some("api.anthropic.com"));
            assert!(!req.url().as_str().contains("fixture-secret"));
            assert_eq!(req.headers()["anthropic-beta"], "oauth-2025-04-20");
            assert!(req.body().is_none());
        }
        assert_eq!(status(401), Err("auth_expired"));
        assert_eq!(status(429), Err("rate_limited"));
        assert_eq!(status(302), Err("unavailable"));
    }
    #[tokio::test]
    async fn response_bodies_are_bounded_and_errors_never_echo_provider_content() {
        fn response(status: u16, bytes: Vec<u8>) -> reqwest::Response {
            axum::http::Response::builder()
                .status(status)
                .body(bytes)
                .unwrap()
                .into()
        }
        assert_eq!(
            body(response(
                200,
                br#"{"five_hour":{"utilization":15}}"#.to_vec()
            ))
            .await
            .unwrap()["five_hour"]["utilization"],
            15
        );
        assert_eq!(
            body(response(200, vec![b' '; MAX_BYTES + 1]))
                .await
                .unwrap_err(),
            "invalid_response"
        );
        assert_eq!(
            body(response(401, b"PRIVATE_PROVIDER_ERROR_TOKEN".to_vec()))
                .await
                .unwrap_err(),
            "auth_expired"
        );
        assert_eq!(
            body(response(200, b"PRIVATE_PROVIDER_NON_JSON".to_vec()))
                .await
                .unwrap_err(),
            "invalid_response"
        );
        assert_eq!(
            body(response(302, b"https://untrusted.invalid/SECRET".to_vec()))
                .await
                .unwrap_err(),
            "unavailable"
        );
    }

    #[test]
    fn usage_parses_reported_fable_and_preserves_unknown_values() {
        let reading = parse_usage(&json!({"five_hour":{"utilization":12.5},"seven_day":{"utilization":false}, "seven_day_sonnet":null,
            "limits":[{"kind":"weekly_scoped","group":"weekly","percent":40,"scope":{"model":{"id":"claude/fable.5:promo","display_name":"Fable"}}}]}), Identity::default()).unwrap();
        assert_eq!(reading.windows.len(), 3);
        assert_eq!(reading.windows[0].used_permille, Some(125));
        assert_eq!(reading.windows[1].used_permille, None);
        assert_eq!(reading.windows[2].label.as_deref(), Some("Fable"));
        assert_eq!(reading.windows[2].used_permille, Some(400));
        assert!(
            parse_usage(&json!({}), Identity::default())
                .unwrap()
                .windows
                .is_empty()
        );
        assert!(parse_usage(&json!([]), Identity::default()).is_err());
    }
    /// Moves `path`'s modified time a second past what it was, as a later
    /// write would.
    fn later(path: &std::path::Path) {
        let modified = std::fs::metadata(path).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(modified + std::time::Duration::from_secs(1)),
            )
            .unwrap();
    }

    #[test]
    fn file_profile_changes_and_signout_invalidate_capture_witness() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"a"}}"#,
        )
        .unwrap();
        std::fs::write(
            credential_path(&spec),
            br#"{"claudeAiOauth":{"accessToken":"one"}}"#,
        )
        .unwrap();
        let original = load(&spec, false, &test_deps()).ok().unwrap();
        assert!(unchanged(&spec, &original, &test_deps()).unwrap());
        std::fs::write(
            credential_path(&spec),
            br#"{"claudeAiOauth":{"accessToken":"two"}}"#,
        )
        .unwrap();
        // A rotation lands after the clock has moved on; written microseconds
        // apart, a same-length rewrite shares its timestamps on a coarse clock
        // (Windows, about 15 ms) and nothing else tells it apart, since the
        // revision reads attributes only (`file_revision`).
        later(&credential_path(&spec));
        assert!(!unchanged(&spec, &original, &test_deps()).unwrap());
        let rotated = load(&spec, false, &test_deps()).ok().unwrap();
        assert_eq!(original.local_identity, rotated.local_identity);
        std::fs::write(
            account_path(&spec),
            br#"{"oauthAccount":{"accountUuid":"b"}}"#,
        )
        .unwrap();
        later(&account_path(&spec));
        assert!(!unchanged(&spec, &rotated, &test_deps()).unwrap());
        std::fs::write(account_path(&spec), br#"{}"#).unwrap();
        assert!(matches!(
            load(&spec, false, &test_deps()),
            Err("auth_missing")
        ));
    }
    #[test]
    fn preferred_expired_file_never_falls_back_to_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let spec = login(dir.path());
        std::fs::write(
            credential_path(&spec),
            br#"{"claudeAiOauth":{"accessToken":"one","expiresAt":1}}"#,
        )
        .unwrap();
        assert!(matches!(
            load(&spec, true, &test_deps()),
            Err("auth_expired")
        ));
    }
    #[test]
    fn secure_override_selects_file_and_service_without_changing_account_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = login(dir.path());
        let original = account_path(&spec);
        spec.secure_storage_dir = Some("/fixture/secure".into());
        assert_eq!(
            credential_path(&spec),
            PathBuf::from("/fixture/secure/.credentials.json")
        );
        assert_eq!(account_path(&spec), original);
        assert_eq!(
            service(&spec).unwrap(),
            format!(
                "Claude Code-credentials-{}",
                &fingerprint(b"/fixture/secure")[..8]
            )
        );
        spec.secure_storage_dir = Some(String::new());
        assert_eq!(service(&spec).unwrap(), "Claude Code-credentials");
    }
}
