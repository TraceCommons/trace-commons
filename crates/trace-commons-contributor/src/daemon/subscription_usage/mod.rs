//! Current native subscription limits. Credentials and account fingerprints
//! stay in memory and never cross IPC; the display reads a bounded snapshot.

mod claude;
mod claude_cli;
mod claude_token_cache;
mod codex_oauth;
pub(super) mod providers;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, EVENT_STATUS_CHANGED, Request, Response};
use crate::managed::{AuthState, ConnectionKind, ToolId};

const STALE_SECONDS: i64 = 20 * 60;
const MANUAL_FLOOR_SECONDS: i64 = 60;
const CACHE_SECONDS: i64 = 60 * 60;
const MAX_LOGINS: usize = 128;
const MAX_PROBES: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    Codex,
    Claude,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LoginSpec {
    pub provider: Provider,
    pub root: PathBuf,
    pub id: String,
    pub label: String,
    pub default_login: bool,
    pub native_executable: Option<PathBuf>,
    /// The exact native override string; None names the native default login.
    pub config_dir: Option<String>,
    /// Exact native secure-storage override, independent of the account config.
    pub secure_storage_dir: Option<String>,
    /// Native-account metadata revision, never a provider account identifier.
    pub metadata_revision: Option<String>,
    /// Credential-free native file revision, separate from the managed binding.
    pub native_revision: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Window {
    pub lane: String,
    pub label: Option<String>,
    /// Exact provider duration, when supplied; never inferred from a slot.
    pub duration_mins: Option<u64>,
    pub used_permille: Option<u16>,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Comparable account evidence only. Missing evidence is not an account change;
/// an email fingerprint must never be compared with an account-ID fingerprint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Identity {
    pub account: Option<String>,
    pub email: Option<String>,
    /// Independent namespace: a same-account organization switch is a conflict.
    pub organization: Option<String>,
}

impl Identity {
    fn conflicts_with(&self, other: &Self) -> bool {
        self.account
            .as_ref()
            .zip(other.account.as_ref())
            .is_some_and(|(a, b)| a != b)
            || self
                .email
                .as_ref()
                .zip(other.email.as_ref())
                .is_some_and(|(a, b)| a != b)
            || self
                .organization
                .as_ref()
                .zip(other.organization.as_ref())
                .is_some_and(|(a, b)| a != b)
    }
}

#[cfg(test)]
impl From<&str> for Identity {
    fn from(value: &str) -> Self {
        Self {
            account: Some(value.into()),
            email: None,
            organization: None,
        }
    }
}
#[cfg(test)]
impl From<String> for Identity {
    fn from(value: String) -> Self {
        Self {
            account: Some(value),
            email: None,
            organization: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Reading {
    pub windows: Vec<Window>,
    pub banked_resets: Option<u64>,
    /// Memory-only account fingerprint. Never derive a display label from it.
    pub identity: Identity,
    /// Post-validation file revision witnessed by the native collector.
    pub native_revision: Option<String>,
    /// Where the figures came from; shown as the login's `source`.
    pub source: ReadingSource,
}

/// `api`: a provider usage endpoint or native app-server. `cli`: read off
/// the Claude Code CLI's `/usage` screen, whole percentages and resets only
/// when stated in the local zone.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReadingSource {
    Api,
    Cli,
}

#[derive(Clone, Debug)]
pub(super) struct ProbeFailure {
    pub reason: &'static str,
    pub identity: Option<Identity>,
}

type ProbeFuture = Pin<Box<dyn Future<Output = Result<Reading, ProbeFailure>> + Send>>;
type Collector =
    Arc<dyn Fn(LoginSpec, bool, Arc<OwnedSemaphorePermit>) -> ProbeFuture + Send + Sync>;

struct Entry {
    login: LoginSpec,
    generation: u64,
    refreshing: bool,
    last_started: Option<DateTime<Utc>>,
    measured_at: Option<DateTime<Utc>>,
    reading: Option<Reading>,
    reason: Option<&'static str>,
}

#[derive(Default)]
struct State {
    enabled: [bool; 2],
    entries: BTreeMap<String, Entry>,
    next_generation: u64,
    truncated: bool,
}

#[derive(Clone)]
struct Claim {
    login: LoginSpec,
    generation: u64,
}

pub(crate) struct UsageService {
    state: Mutex<State>,
    permits: Arc<Semaphore>,
    collector: Collector,
    changed: tokio::sync::Notify,
    timeout: std::time::Duration,
    /// The app's own copy of Claude usage access tokens; see
    /// `claude_token_cache`. Cleared on unenroll and when Claude is off.
    token_cache: Arc<claude_token_cache::TokenCache>,
}

fn global_probe_permits() -> Arc<Semaphore> {
    static PERMITS: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
    Arc::clone(PERMITS.get_or_init(|| Arc::new(Semaphore::new(MAX_PROBES))))
}

impl Default for UsageService {
    fn default() -> Self {
        let token_cache = Arc::new(claude_token_cache::TokenCache::for_build());
        let deps = claude::Deps::for_build(Arc::clone(&token_cache));
        Self {
            state: Mutex::new(State::default()),
            permits: global_probe_permits(),
            changed: tokio::sync::Notify::new(),
            collector: Arc::new(move |login, interactive, lease| {
                let deps = deps.clone();
                Box::pin(
                    async move { providers::collect_with(&login, interactive, lease, &deps).await },
                )
            }),
            timeout: std::time::Duration::from_secs(240),
            token_cache,
        }
    }
}

impl Provider {
    fn index(self) -> usize {
        match self {
            Self::Codex => 0,
            Self::Claude => 1,
        }
    }
}

impl State {
    fn synchronize(&mut self, enabled: [bool; 2], logins: Vec<LoginSpec>) {
        self.enabled = enabled;
        self.truncated = logins
            .iter()
            .filter(|login| enabled[login.provider.index()])
            .count()
            > MAX_LOGINS;
        let logins: Vec<_> = logins
            .into_iter()
            .filter(|login| enabled[login.provider.index()])
            .take(MAX_LOGINS)
            .collect();
        let live: BTreeSet<_> = logins.iter().map(|login| login.id.clone()).collect();
        self.entries.retain(|id, _| live.contains(id));
        for login in logins {
            let replace = self.entries.get(&login.id).is_none_or(|entry| {
                entry.login.provider != login.provider
                    || entry.login.root != login.root
                    || entry.login.config_dir != login.config_dir
                    || entry.login.secure_storage_dir != login.secure_storage_dir
                    || entry.login.metadata_revision != login.metadata_revision
                    || (login.provider == Provider::Claude
                        && entry.login.native_revision != login.native_revision)
            });
            if replace {
                self.next_generation = self.next_generation.wrapping_add(1);
                self.entries.insert(
                    login.id.clone(),
                    Entry {
                        login,
                        generation: self.next_generation,
                        refreshing: false,
                        last_started: None,
                        measured_at: None,
                        reading: None,
                        reason: Some("not_checked"),
                    },
                );
            } else if let Some(entry) = self.entries.get_mut(&login.id) {
                if entry.login.native_revision != login.native_revision {
                    // Invalidate displayed data immediately. An already-running
                    // probe may finish only with a witness of this new revision.
                    // Keep its generation; consent/binding changes above never do.
                    entry.reading = None;
                    entry.measured_at = None;
                    entry.reason = Some("not_checked");
                }
                entry.login = login;
            }
        }
    }

    fn claims(
        &mut self,
        now: DateTime<Utc>,
        manual: bool,
        provider: Option<Provider>,
        login: Option<&str>,
    ) -> Vec<Claim> {
        if !manual {
            return Vec::new();
        }
        self.entries
            .values_mut()
            .filter_map(|entry| {
                if entry.login.provider != provider.unwrap_or(Provider::Codex)
                    || login.is_some_and(|id| entry.login.id != id)
                    || (entry.login.provider == Provider::Claude && login.is_none())
                {
                    return None;
                }
                let interval = MANUAL_FLOOR_SECONDS;
                if entry.refreshing
                    || entry
                        .last_started
                        .is_some_and(|at| now < at + Duration::seconds(interval))
                {
                    return None;
                }
                entry.refreshing = true;
                entry.last_started = Some(now);
                Some(Claim {
                    login: entry.login.clone(),
                    generation: entry.generation,
                })
            })
            .collect()
    }

    fn current(&self, claim: &Claim) -> bool {
        self.entries
            .get(&claim.login.id)
            .is_some_and(|entry| entry.generation == claim.generation && entry.refreshing)
    }

    fn complete(
        &mut self,
        claim: &Claim,
        answer: Result<Reading, ProbeFailure>,
        now: DateTime<Utc>,
    ) {
        let Some(entry) = self
            .entries
            .get_mut(&claim.login.id)
            .filter(|entry| entry.generation == claim.generation)
        else {
            return;
        };
        entry.refreshing = false;
        match answer {
            Ok(mut reading) => {
                let witnessed_revision = reading
                    .native_revision
                    .as_ref()
                    .or(claim.login.native_revision.as_ref());
                if witnessed_revision != entry.login.native_revision.as_ref() {
                    entry.reading = None;
                    entry.measured_at = None;
                    entry.reason = Some("auth_changed");
                    return;
                }
                reading.windows.truncate(16);
                reading.windows.retain(|window| {
                    matches!(
                        window.lane.as_str(),
                        "session" | "weekly" | "model" | "primary" | "secondary"
                    )
                });
                for window in &mut reading.windows {
                    window.duration_mins = window
                        .duration_mins
                        .filter(|minutes| *minutes > 0 && *minutes <= i64::MAX as u64);
                    window.used_permille = window.used_permille.filter(|used| *used <= 1000);
                    if window
                        .resets_at
                        .is_some_and(|at| at > now + Duration::days(31))
                    {
                        window.resets_at = None;
                    }
                    if let Some(label) = &mut window.label {
                        *label = label.chars().filter(|c| !c.is_control()).take(80).collect();
                    }
                }
                reading.banked_resets = reading
                    .banked_resets
                    .filter(|count| *count <= i64::MAX as u64);
                entry.reading = Some(reading);
                entry.measured_at = Some(now);
                entry.reason =
                    (entry.login.provider == Provider::Claude).then_some("captured_on_refresh");
            }
            Err(mut failure) => {
                failure.reason = providers::normalize_failure_reason(failure.reason);
                let changed_identity = failure.identity.as_ref().is_some_and(|identity| {
                    entry
                        .reading
                        .as_ref()
                        .is_some_and(|old| old.identity.conflicts_with(identity))
                });
                let authentication = matches!(
                    failure.reason,
                    "auth_missing" | "auth_expired" | "auth_changed"
                );
                if authentication || changed_identity {
                    entry.reading = None;
                    entry.measured_at = None;
                }
                if matches!(
                    failure.reason,
                    "credential_read_timeout"
                        | "credential_check_timeout"
                        | "keychain_needs_permission"
                        | "timeout"
                ) {
                    entry.last_started = None;
                }
                entry.reason = Some(failure.reason);
            }
        }
    }

    fn expire(&mut self, now: DateTime<Utc>) {
        for entry in self.entries.values_mut() {
            if entry
                .measured_at
                .is_some_and(|at| now >= at + Duration::seconds(CACHE_SECONDS))
            {
                entry.reading = None;
            }
        }
    }

    fn value(&self, now: DateTime<Utc>) -> Value {
        let providers: Vec<_> = [Provider::Codex, Provider::Claude].into_iter().map(|provider| {
            let mut entries: Vec<_> = self.entries.values().filter(|entry| entry.login.provider == provider).collect();
            entries.sort_by(|a,b| (!a.login.default_login, &a.login.id).cmp(&(!b.login.default_login, &b.login.id)));
            let logins: Vec<_> = entries.into_iter().map(|entry| {
                let fresh_enough = entry.measured_at.is_some_and(|at| now < at + Duration::seconds(CACHE_SECONDS));
                let reading = entry.reading.as_ref().filter(|_| fresh_enough);
                let stale = reading.is_some() && (entry.reason.is_some()
                    || entry.measured_at.is_some_and(|at| now >= at + Duration::seconds(STALE_SECONDS)));
                let windows: Vec<_> = reading.into_iter().flat_map(|reading| &reading.windows).map(|window| {
                    let used = window.used_permille.filter(|_| window.resets_at.is_none_or(|reset| reset > now));
                    json!({"lane":window.lane,"label":window.label,"used_permille":used,
                        "remaining_text":remaining_text(used),"resets_at":window.resets_at,"duration_mins":window.duration_mins})
                }).collect();
                json!({"id":entry.login.id,"label":entry.login.label,"default_login":entry.login.default_login,
                    "state":if reading.is_some() {"observed"} else {"unknown"},"reason":entry.reason,
                    "updated_at":entry.measured_at,"stale":stale,"windows":windows,"refreshing":entry.refreshing,
                    "banked_resets":reading.and_then(|reading| reading.banked_resets),
                    "source":reading.map(|reading| reading.source)})
            }).collect();
            json!({"provider":provider,"enabled":self.enabled[provider.index()],
                "refreshing":self.entries.values().any(|entry| entry.login.provider == provider && entry.refreshing),"logins":logins})
        }).collect();
        json!({"schema_version":"trace_commons.subscription_usage.v1","providers":providers,"truncated":self.truncated})
    }
}

fn remaining_text(used: Option<u16>) -> String {
    match used.filter(|used| *used <= 1000).map(|used| 1000 - used) {
        None => "—".into(),
        Some(left) if left % 10 == 0 => format!("{}%", left / 10),
        Some(left) => format!("{}.{:01}%", left / 10, left % 10),
    }
}

fn native_login(
    provider: Provider,
    home: Option<PathBuf>,
    config_dir: Option<String>,
) -> Option<LoginSpec> {
    let (folder, id, label) = match provider {
        Provider::Codex => (".codex", "codex-default", "Codex"),
        Provider::Claude => (".claude", "claude-default", "Claude"),
    };
    let root = match &config_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        Some(_) if provider == Provider::Claude => home?.join(folder),
        Some(_) => return None,
        None => home?.join(folder),
    };
    let mut login = LoginSpec {
        provider,
        metadata_revision: None,
        native_revision: None,
        root,
        id: id.into(),
        label: label.into(),
        default_login: true,
        config_dir,
        secure_storage_dir: None,
        native_executable: None,
    };
    login.native_revision = Some(login_revision(&login));
    Some(login)
}

/// A credential-free invalidation revision: file attributes only, never
/// the contents. Atomic replacement and signout are always changes; a
/// same-length rewrite is one once the file system's clock has moved on
/// (a rewrite within one tick of the last write, about 15 ms on Windows,
/// has nothing to tell it apart by). Keychain contents are never queried.
fn file_revision(path: &std::path::Path) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    metadata.len(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                    metadata.ino()
                )
            }
            #[cfg(windows)]
            {
                format!(
                    "{}:{:?}:{:?}:{}",
                    metadata.len(),
                    metadata.modified(),
                    metadata.created(),
                    windows_file_identity(path).unwrap_or_default()
                )
            }
            #[cfg(not(any(unix, windows)))]
            {
                format!(
                    "{}:{:?}:{:?}",
                    metadata.len(),
                    metadata.modified(),
                    metadata.created()
                )
            }
        }
        Err(error) => format!("{:?}", error.kind()),
    }
}

/// Windows has no inode, and NTFS "tunnels" a file's creation time onto a
/// file renamed over it within seconds, so an atomic replacement with the
/// same length and modified time looks unchanged by those attributes. The
/// volume serial and file index tell the two files apart. The file is
/// opened with no access rights (access mode 0), which reads attributes
/// only, never contents, and shares read, write and delete so the tool
/// that owns it is never blocked. `MetadataExt::file_index` is unstable,
/// so the call is made directly, as `trace_commons_contributor_insights`
/// does for its file identity.
#[cfg(windows)]
fn windows_file_identity(path: &std::path::Path) -> Option<String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = std::fs::OpenOptions::new().access_mode(0).open(path).ok()?;
    // SAFETY: the structure is plain integers, so a zeroed value is valid,
    // and the handle stays live for the duration of the call.
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let read = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if read == 0 {
        return None;
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Some(format!("{}:{index}", information.dwVolumeSerialNumber))
}

fn native_metadata_revision(root: &std::path::Path) -> String {
    format!(
        "{}|{}",
        file_revision(&root.join("auth.json")),
        file_revision(&root.join("config.toml"))
    )
}

fn login_revision(login: &LoginSpec) -> String {
    match login.provider {
        Provider::Codex => native_metadata_revision(&login.root),
        Provider::Claude => claude::published_revision(login),
    }
}

fn inputs(shared: &DaemonShared) -> ([bool; 2], Vec<LoginSpec>) {
    let enabled = shared
        .settings
        .lock()
        .map(|s| {
            [
                s.subscription_usage_codex_enabled,
                s.subscription_usage_claude_on,
            ]
        })
        .unwrap_or([false, false]);
    let mut logins = Vec::new();
    // Executable discovery is deferred to explicit refresh. Background work
    // only stats native files; it never reads auth/config contents or Keychain.
    if let Some(login) = native_login(
        Provider::Codex,
        dirs::home_dir(),
        std::env::var("CODEX_HOME").ok(),
    ) {
        logins.push(login);
    }
    if let Some(mut login) = native_login(
        Provider::Claude,
        dirs::home_dir(),
        std::env::var("CLAUDE_CONFIG_DIR").ok(),
    ) {
        login.secure_storage_dir = std::env::var("CLAUDE_SECURESTORAGE_CONFIG_DIR").ok();
        login.native_revision = Some(login_revision(&login));
        // Detection only: no credential contents or Keychain queries on status.
        if login.root.is_dir() || claude::account_path(&login).is_file() {
            logins.push(login);
        }
    }
    let accounts = shared
        .managed
        .lock()
        .ok()
        .and_then(|service| service.as_ref().ok().map(|service| service.accounts.list()))
        .unwrap_or_default();
    for account in accounts {
        if account.connection != ConnectionKind::Subscription
            || account.auth_state == AuthState::SignInRequired
        {
            continue;
        }
        let provider = match account.tool {
            ToolId::Codex => Provider::Codex,
            ToolId::Claude => Provider::Claude,
            ToolId::Opencode => continue,
        };
        if !enabled[provider.index()] {
            continue;
        }
        let root = shared
            .store
            .dir()
            .join("managed-profiles")
            .join(account.id.to_string());
        let mut login = LoginSpec {
            provider,
            config_dir: Some(root.to_string_lossy().into_owned()),
            root: root.clone(),
            id: account.id.to_string(),
            label: account.label,
            default_login: false,
            metadata_revision: Some(format!(
                "{:?}:{:?}",
                account.auth_state, account.verified_at
            )),
            native_revision: None,
            secure_storage_dir: None,
            native_executable: None,
        };
        login.native_revision = Some(login_revision(&login));
        logins.push(login);
    }
    (enabled, logins)
}

fn synchronize(shared: &DaemonShared) {
    let (enabled, logins) = if shared.dev_dry_run {
        ([false, false], Vec::new())
    } else {
        inputs(shared)
    };
    if let Ok(mut state) = shared.subscription_usage.state.lock() {
        state.synchronize(enabled, logins);
        state.expire(Utc::now());
    }
    shared.subscription_usage.changed.notify_waiters();
}

fn schedule(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    manual: bool,
    interactive: bool,
    provider: Option<Provider>,
    login: Option<&str>,
) {
    synchronize(shared);
    if shared.dev_dry_run {
        return;
    }
    let service = Arc::clone(&shared.subscription_usage);
    let claims = service
        .state
        .lock()
        .map(|mut state| state.claims(now, manual, provider, login))
        .unwrap_or_default();
    if claims.is_empty() {
        return;
    }
    let abandoned = claims.clone();
    if shared
        .spawn_detached(move |shared| async move {
            let mut tasks = tokio::task::JoinSet::new();
            for claim in claims {
                let shared = Arc::clone(&shared);
                let service = Arc::clone(&service);
                tasks.spawn(async move {
                    let Ok(permit) = Arc::clone(&service.permits).acquire_owned().await else {
                        return;
                    };
                    let lease = Arc::new(permit);
                    synchronize(&shared);
                    if !service
                        .state
                        .lock()
                        .map(|state| state.current(&claim))
                        .unwrap_or(false)
                    {
                        return;
                    }
                    let canceled = async {
                        loop {
                            let changed = service.changed.notified();
                            tokio::pin!(changed);
                            changed.as_mut().enable();
                            if !service.state.lock().map(|state| state.current(&claim)).unwrap_or(false) {
                                return;
                            }
                            changed.await;
                        }
                    };
                    // Cancellation wins if a late native permission result and
                    // unenroll/rebinding arrive together. Dropping the collector
                    // stops before HTTP; its native thread retains the lease.
                    let result = tokio::select! {
                        biased;
                        () = canceled => return,
                        answer = tokio::time::timeout(
                            if claim.login.provider == Provider::Codex { service.timeout.min(std::time::Duration::from_secs(45)) } else { service.timeout },
                            (service.collector)(claim.login.clone(), interactive, Arc::clone(&lease)),
                        ) => answer.unwrap_or(Err(ProbeFailure { reason: "timeout", identity: None })),
                    };
                    // An account may have been removed or a switch turned off while
                    // the provider read was in flight. Synchronize before publish.
                    synchronize(&shared);
                    if let Ok(mut state) = service.state.lock() {
                        state.complete(&claim, result, Utc::now());
                    }
                    shared.publish(EVENT_STATUS_CHANGED, json!({}));
                });
            }
            while tasks.join_next().await.is_some() {}
        })
        .is_none()
    {
        if let Ok(mut state) = shared.subscription_usage.state.lock() {
            for claim in abandoned {
                if let Some(entry) = state
                    .entries
                    .get_mut(&claim.login.id)
                    .filter(|entry| entry.generation == claim.generation)
                {
                    entry.refreshing = false;
                    entry.last_started = None;
                    entry.reason = Some("unavailable");
                }
            }
        }
    }
}

/// Called after a committed setting change; no provider I/O from this path.
/// Turning Claude off deletes the cached Claude usage token, on its own
/// thread because the OS store call may block.
pub(super) fn settings_changed(shared: &DaemonShared) {
    synchronize(shared);
    let claude_on = shared
        .settings
        .lock()
        .map(|s| s.subscription_usage_claude_on)
        .unwrap_or(false);
    if !claude_on {
        clear_token_cache(&shared.subscription_usage);
    }
    shared.publish(EVENT_STATUS_CHANGED, json!({}));
}

/// Unenroll discards every cached reading and invalidates all in-flight generations.
/// Available native Claude profiles may reappear later, with no retained reading.
/// The cached Claude usage token goes too.
pub(super) fn forget_readings(shared: &DaemonShared) {
    if let Ok(mut state) = shared.subscription_usage.state.lock() {
        state.entries.clear();
        state.next_generation = state.next_generation.wrapping_add(1);
    }
    clear_token_cache(&shared.subscription_usage);
    shared.subscription_usage.changed.notify_waiters();
    shared.publish(EVENT_STATUS_CHANGED, json!({}));
}

fn clear_token_cache(service: &UsageService) {
    let cache = Arc::clone(&service.token_cache);
    // The epoch moves now, so a Refresh still in flight cannot write its
    // token back after this; the store delete itself runs off this thread.
    cache.begin_clear();
    let spawned = std::thread::Builder::new()
        .name("tc-claude-usage-cache-clear".into())
        .spawn(move || cache.clear());
    if spawned.is_err() {
        tracing::warn!("claude usage token cache clear could not start");
    }
}

/// Background ticks synchronize metadata only; explicit Refresh owns all probes.
pub(super) fn tick(shared: &DaemonShared, _now: DateTime<Utc>) {
    synchronize(shared);
}

pub(super) fn handle(shared: &DaemonShared, req: &Request) -> Response {
    let Some(params) = req.params.as_object() else {
        return Response::err(req.id, ERR_BAD_PARAMS, "subscription-usage-params-invalid");
    };
    if params.keys().any(|key| {
        !matches!(
            key.as_str(),
            "refresh" | "interactive" | "provider" | "login"
        )
    }) || ["refresh", "interactive"]
        .iter()
        .any(|key| params.get(*key).is_some_and(|value| !value.is_boolean()))
    {
        return Response::err(req.id, ERR_BAD_PARAMS, "subscription-usage-params-invalid");
    }
    let provider = match params.get("provider") {
        None => None,
        Some(Value::String(value)) if value == "codex" => Some(Provider::Codex),
        Some(Value::String(value)) if value == "claude" => Some(Provider::Claude),
        Some(_) => {
            return Response::err(
                req.id,
                ERR_BAD_PARAMS,
                "subscription-usage-provider-invalid",
            );
        }
    };
    let refresh = params
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let interactive = params
        .get("interactive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if interactive && !refresh {
        return Response::err(
            req.id,
            ERR_BAD_PARAMS,
            "subscription-usage-interactive-needs-refresh",
        );
    }
    let login = match params.get("login") {
        None => None,
        Some(Value::String(id))
            if !id.is_empty()
                && id.len() <= 128
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') =>
        {
            Some(id.as_str())
        }
        Some(_) => {
            return Response::err(req.id, ERR_BAD_PARAMS, "subscription-usage-login-invalid");
        }
    };
    synchronize(shared);
    if login.is_some_and(|id| {
        provider.is_none()
            || !shared
                .subscription_usage
                .state
                .lock()
                .map(|state| {
                    state
                        .entries
                        .get(id)
                        .is_some_and(|entry| Some(entry.login.provider) == provider)
                })
                .unwrap_or(false)
    }) || (refresh && provider == Some(Provider::Claude) && login.is_none())
    {
        return Response::err(req.id, ERR_BAD_PARAMS, "subscription-usage-login-invalid");
    }
    let now = Utc::now();
    if refresh {
        schedule(shared, now, true, interactive, provider, login);
    } else {
        synchronize(shared);
    }
    let result = shared
        .subscription_usage
        .state
        .lock()
        .map(|state| state.value(now))
        .unwrap_or_else(
            |_| json!({"schema_version":"trace_commons.subscription_usage.v1","providers":[]}),
        );
    Response::ok(req.id, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap()
    }
    fn login(id: &str) -> LoginSpec {
        LoginSpec {
            provider: Provider::Codex,
            root: PathBuf::from(format!("/fake/{id}")),
            id: id.into(),
            label: id.into(),
            default_login: id == "default",
            config_dir: None,
            secure_storage_dir: None,
            metadata_revision: None,
            native_revision: None,
            native_executable: None,
        }
    }
    fn reading(identity: &str, reset: DateTime<Utc>) -> Reading {
        Reading {
            identity: identity.into(),
            native_revision: None,
            source: ReadingSource::Api,
            banked_resets: Some(2),
            windows: vec![Window {
                lane: "weekly".into(),
                label: None,
                duration_mins: Some(10080),
                used_permille: Some(420),
                resets_at: Some(reset),
            }],
        }
    }
    #[test]
    fn a_reading_names_its_source_and_no_reading_names_none() {
        let (mut state, claim) = prepared();
        let logins = |state: &State| state.value(now())["providers"][0]["logins"][0].clone();
        assert!(logins(&state)["source"].is_null());
        let mut cli = reading("account", now() + Duration::days(1));
        cli.source = ReadingSource::Cli;
        state.complete(&claim, Ok(cli), now());
        assert_eq!(logins(&state)["source"], "cli");
        let claim = state
            .claims(now() + Duration::hours(1), true, None, None)
            .pop()
            .unwrap();
        state.complete(
            &claim,
            Ok(reading("account", now() + Duration::days(1))),
            now() + Duration::hours(1),
        );
        assert_eq!(logins(&state)["source"], "api");
    }
    fn prepared() -> (State, Claim) {
        let mut state = State::default();
        state.synchronize([true, false], vec![login("default")]);
        let claim = state.claims(now(), true, None, None).pop().unwrap();
        (state, claim)
    }

    #[test]
    fn daemon_restarts_share_the_same_native_probe_budget() {
        let first = UsageService::default();
        let next = UsageService::default();
        assert!(Arc::ptr_eq(&first.permits, &next.permits));
    }

    #[test]
    fn off_is_empty_and_does_not_claim_or_retain_a_reading() {
        let (mut state, claim) = prepared();
        state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
        state.synchronize([false, false], vec![login("default")]);
        assert!(state.claims(now(), true, None, None).is_empty());
        assert!(state.entries.is_empty());
        assert_eq!(state.value(now())["providers"][0]["enabled"], false);
    }
    #[test]
    fn disabling_or_removing_a_login_invalidates_its_inflight_completion() {
        let (mut state, claim) = prepared();
        state.synchronize([false, false], vec![]);
        state.synchronize([true, false], vec![login("default")]);
        state.complete(&claim, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries["default"].reading.is_none());
        let next = state.claims(now(), true, None, None).pop().unwrap();
        state.synchronize([true, false], vec![]);
        state.complete(&next, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries.is_empty());
    }
    #[test]
    fn reconnect_metadata_invalidates_cache_and_inflight_but_renaming_does_not() {
        let (mut state, claim) = prepared();
        state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
        let mut same = login("default");
        same.label = "renamed".into();
        state.synchronize([true, false], vec![same.clone()]);
        assert!(state.entries["default"].reading.is_some());
        same.metadata_revision = Some("ready-new-verification".into());
        state.synchronize([true, false], vec![same]);
        assert!(state.entries["default"].reading.is_none());
        state.complete(&claim, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries["default"].reading.is_none());
    }

    #[test]
    fn cache_staleness_is_bound_to_identity_and_window_reset() {
        let (mut state, claim) = prepared();
        let reset = now() + Duration::minutes(20);
        state.complete(&claim, Ok(reading("a", reset)), now());
        state.complete(
            &claim,
            Err(ProbeFailure {
                reason: "unavailable",
                identity: Some("a".into()),
            }),
            now() + Duration::minutes(1),
        );
        let value = state.value(now() + Duration::minutes(2));
        let row = &value["providers"][0]["logins"][0];
        assert_eq!(row["stale"], true);
        assert_eq!(row["windows"][0]["remaining_text"], "58%");
        assert_eq!(
            state.value(reset)["providers"][0]["logins"][0]["windows"][0]["used_permille"],
            Value::Null
        );
        assert_eq!(
            state.value(now() + Duration::hours(1))["providers"][0]["logins"][0]["banked_resets"],
            Value::Null
        );
        state.complete(
            &claim,
            Err(ProbeFailure {
                reason: "unavailable",
                identity: Some("different".into()),
            }),
            now(),
        );
        assert!(state.entries["default"].reading.is_none());
    }
    #[test]
    fn unknown_failure_identity_keeps_bounded_stale_data_but_auth_loss_clears_it() {
        for reason in ["timeout", "native_tool_running", "unavailable"] {
            let (mut state, claim) = prepared();
            state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
            state.complete(
                &claim,
                Err(ProbeFailure {
                    reason,
                    identity: None,
                }),
                now(),
            );
            assert!(state.entries["default"].reading.is_some());
            assert_eq!(
                state.value(now())["providers"][0]["logins"][0]["stale"],
                true
            );
            assert_eq!(
                state.value(now() + Duration::hours(1))["providers"][0]["logins"][0]["windows"],
                json!([])
            );
            state.complete(
                &claim,
                Err(ProbeFailure {
                    reason: "auth_missing",
                    identity: None,
                }),
                now(),
            );
            assert!(state.entries["default"].reading.is_none());
        }
    }

    #[test]
    fn owned_rotation_publishes_only_with_matching_witness_and_unchanged_binding() {
        let mut original = login("default");
        original.native_revision = Some("before".into());
        let mut state = State::default();
        state.synchronize([true, false], vec![original.clone()]);
        let claim = state.claims(now(), true, None, None).pop().unwrap();
        let mut rotated = original.clone();
        rotated.native_revision = Some("after".into());
        state.synchronize([true, false], vec![rotated.clone()]);
        assert!(state.current(&claim));
        assert!(state.entries["default"].reading.is_none());
        let mut answer = reading("same-account", now() + Duration::days(1));
        answer.native_revision = Some("after".into());
        state.complete(&claim, Ok(answer.clone()), now());
        assert!(state.entries["default"].reading.is_some());

        // Later file changes immediately remove the old display; the earlier
        // witness cannot authorize republishing it for another native revision.
        rotated.native_revision = Some("new-account".into());
        state.synchronize([true, false], vec![rotated.clone()]);
        assert!(state.entries["default"].reading.is_none());
        state.complete(&claim, Ok(answer.clone()), now());
        assert!(state.entries["default"].reading.is_none());

        // Neither disabling/re-enabling nor reconnecting may revive an old claim.
        for changed_binding in [false, true] {
            let mut state = State::default();
            state.synchronize([true, false], vec![original.clone()]);
            let claim = state.claims(now(), true, None, None).pop().unwrap();
            let mut current = original.clone();
            current.native_revision = Some("after".into());
            if changed_binding {
                current.metadata_revision = Some("reconnected".into());
            } else {
                state.synchronize([false, false], vec![]);
            }
            state.synchronize([true, false], vec![current]);
            state.complete(&claim, Ok(answer.clone()), now());
            assert!(state.entries["default"].reading.is_none());
        }
    }

    #[test]
    fn changed_native_revision_rejects_an_unwitnessed_completion() {
        let (mut state, claim) = prepared();
        let mut changed = login("default");
        changed.native_revision = Some("changed".into());
        state.synchronize([true, false], vec![changed]);
        state.complete(&claim, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries["default"].reading.is_none());
    }

    #[test]
    fn partial_account_evidence_preserves_stale_values_until_comparable_identity_changes() {
        let (mut state, claim) = prepared();
        let mut answer = reading("account-id", now() + Duration::days(1));
        answer.identity.email = Some("email-a".into());
        state.complete(&claim, Ok(answer), now());
        state.complete(
            &claim,
            Err(ProbeFailure {
                reason: "unavailable",
                identity: Some(Identity {
                    account: None,
                    email: Some("email-a".into()),
                    organization: None,
                }),
            }),
            now(),
        );
        assert!(state.entries["default"].reading.is_some());
        state.complete(
            &claim,
            Err(ProbeFailure {
                reason: "unavailable",
                identity: Some(Identity {
                    account: None,
                    email: Some("email-b".into()),
                    organization: None,
                }),
            }),
            now(),
        );
        assert!(state.entries["default"].reading.is_none());
    }

    #[test]
    fn freshness_has_cadence_margin_and_preserves_exact_duration() {
        let (mut state, claim) = prepared();
        state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
        assert_eq!(
            state.value(now() + Duration::minutes(15))["providers"][0]["logins"][0]["stale"],
            false
        );
        assert_eq!(
            state.value(now() + Duration::minutes(20))["providers"][0]["logins"][0]["stale"],
            true
        );
        assert_eq!(
            state.value(now())["providers"][0]["logins"][0]["windows"][0]["duration_mins"],
            10080
        );
    }

    #[test]
    fn snapshot_never_serializes_native_roots_or_provider_identity() {
        let (mut state, claim) = prepared();
        state.complete(
            &claim,
            Ok(reading("PRIVATE_PROVIDER_ID", now() + Duration::days(1))),
            now(),
        );
        let wire = state.value(now()).to_string();
        assert!(!wire.contains("/fake/"));
        assert!(!wire.contains("PRIVATE_PROVIDER_ID"));
    }

    #[test]
    fn manual_floor_poll_interval_and_parallel_requests_coalesce() {
        let (mut state, claim) = prepared();
        assert!(state.claims(now(), true, None, None).is_empty());
        state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
        assert!(
            state
                .claims(now() + Duration::seconds(59), true, None, None)
                .is_empty()
        );
        assert!(
            state
                .claims(now() + Duration::minutes(14), false, None, None)
                .is_empty()
        );
        assert_eq!(
            state
                .claims(now() + Duration::minutes(15), true, None, None)
                .len(),
            1
        );
    }
    #[test]
    fn claude_is_available_without_a_legacy_opt_in() {
        let mut state = State::default();
        let mut claude = login("claude-default");
        claude.provider = Provider::Claude;
        state.synchronize([true, true], vec![login("default"), claude]);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.value(now())["providers"][1]["enabled"], true);
    }

    #[test]
    fn claude_refresh_is_scoped_and_permission_retry_is_immediate() {
        let mut state = State::default();
        let mut first = login("claude-default");
        first.provider = Provider::Claude;
        let mut second = first.clone();
        second.id = "saved-claude".into();
        state.synchronize([true, true], vec![login("default"), first, second]);
        assert_eq!(state.claims(now(), true, None, None).len(), 1);
        assert!(
            state
                .claims(now(), false, Some(Provider::Claude), Some("claude-default"))
                .is_empty()
        );
        assert!(
            state
                .claims(now(), true, Some(Provider::Claude), None)
                .is_empty()
        );
        let claims = state.claims(now(), true, Some(Provider::Claude), Some("saved-claude"));
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].login.id, "saved-claude");
        state.complete(
            &claims[0],
            Err(ProbeFailure {
                reason: "credential_read_timeout",
                identity: None,
            }),
            now(),
        );
        assert_eq!(
            state
                .claims(now(), true, Some(Provider::Claude), Some("saved-claude"))
                .len(),
            1
        );
    }

    #[test]
    fn claude_metadata_change_cancels_generation_and_allows_immediate_refresh() {
        let mut original = login("claude-default");
        original.provider = Provider::Claude;
        original.native_revision = Some("before".into());
        let mut state = State::default();
        state.synchronize([false, true], vec![original.clone()]);
        let claim = state
            .claims(now(), true, Some(Provider::Claude), Some("claude-default"))
            .pop()
            .unwrap();
        original.native_revision = Some("new-account".into());
        state.synchronize([false, true], vec![original]);
        assert!(!state.current(&claim));
        state.complete(&claim, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries["claude-default"].reading.is_none());
        assert_eq!(
            state
                .claims(now(), true, Some(Provider::Claude), Some("claude-default"))
                .len(),
            1
        );
    }

    #[test]
    fn organization_switch_invalidates_same_account_stale_reading() {
        let (mut state, claim) = prepared();
        let mut answer = reading("same-account", now() + Duration::days(1));
        answer.identity.organization = Some("organization-a".into());
        state.complete(&claim, Ok(answer), now());
        state.complete(
            &claim,
            Err(ProbeFailure {
                reason: "unavailable",
                identity: Some(Identity {
                    account: Some("same-account".into()),
                    email: None,
                    organization: Some("organization-b".into()),
                }),
            }),
            now(),
        );
        assert!(state.entries["default"].reading.is_none());
    }

    #[test]
    fn known_credential_change_discards_retained_reading() {
        let (mut state, claim) = prepared();
        state.complete(
            &claim,
            Ok(reading("account-a", now() + Duration::days(1))),
            now(),
        );
        state.complete(
            &claim,
            Err(ProbeFailure {
                reason: "credential_changed",
                identity: None,
            }),
            now(),
        );
        assert!(state.entries["default"].reading.is_none());
        assert_eq!(
            state.value(now())["providers"][0]["logins"][0]["windows"],
            json!([])
        );
    }

    #[test]
    fn separate_logins_do_not_share_readings_and_cache_size_is_bounded() {
        let mut state = State::default();
        state.synchronize(
            [true, false],
            (0..MAX_LOGINS + 8).map(|i| login(&i.to_string())).collect(),
        );
        assert_eq!(state.entries.len(), MAX_LOGINS);
        let claims = state.claims(now(), true, None, None);
        state.complete(
            &claims[0],
            Ok(reading("a", now() + Duration::days(1))),
            now(),
        );
        assert_eq!(
            state
                .entries
                .values()
                .filter(|entry| entry.reading.is_some())
                .count(),
            1
        );
        assert_eq!(remaining_text(None), "—");
        assert_eq!(remaining_text(Some(1001)), "—");
        assert_eq!(remaining_text(Some(1000)), "0%");
        assert_eq!(remaining_text(Some(1)), "99.9%");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn native_overrides_keep_the_exact_codex_home() {
        let login = native_login(Provider::Codex, None, Some("/fake/codex-work".into())).unwrap();
        assert_eq!(login.root, PathBuf::from("/fake/codex-work"));
        assert!(login.default_login);
        assert_eq!(
            native_login(Provider::Claude, Some(PathBuf::from("/fake")), None)
                .unwrap()
                .root,
            PathBuf::from("/fake/.claude")
        );
    }

    #[test]
    fn native_metadata_revision_detects_atomic_switch_config_change_and_signout() {
        let dir = tempfile::tempdir().unwrap();
        let auth = dir.path().join("auth.json");
        std::fs::write(&auth, b"account-a").unwrap();
        let before = native_metadata_revision(dir.path());
        let replacement = dir.path().join("replacement");
        std::fs::write(&replacement, b"account-b").unwrap();
        // Atomic replacement must invalidate even with identical length/mtime.
        let modified = std::fs::metadata(&auth).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        std::fs::rename(replacement, &auth).unwrap();
        let switched = native_metadata_revision(dir.path());
        assert_ne!(before, switched);
        std::fs::write(
            dir.path().join("config.toml"),
            b"cli_auth_credentials_store='keyring'",
        )
        .unwrap();
        let configured = native_metadata_revision(dir.path());
        assert_ne!(switched, configured);
        std::fs::remove_file(auth).unwrap();
        assert_ne!(configured, native_metadata_revision(dir.path()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn enabled_background_status_and_settings_never_probe_and_claude_requires_login() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let (_dir, shared) = shared(Arc::new(move |_, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { panic!("metadata-only routes must not collect") })
        }));
        {
            let mut settings = shared.settings.lock().unwrap();
            settings.subscription_usage_codex_enabled = true;
            settings.subscription_usage_claude_enabled = true;
        }
        settings_changed(&shared);
        for minutes in [0, 15, 60, 120] {
            tick(&shared, Utc::now() + Duration::minutes(minutes));
        }
        let status = handle(
            &shared,
            &Request {
                id: 1,
                method: "subscription_usage".into(),
                params: json!({}),
            },
        )
        .result
        .unwrap();
        assert_eq!(status["providers"].as_array().unwrap().len(), 2);
        assert_eq!(status["providers"][0]["enabled"], true);
        assert_eq!(status["providers"][0]["refreshing"], false);
        let response = handle(
            &shared,
            &Request {
                id: 2,
                method: "subscription_usage".into(),
                params: json!({"provider":"claude","refresh":true,"interactive":true}),
            },
        );
        assert!(response.error.is_some());
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    fn shared(collector: Collector) -> (tempfile::TempDir, Arc<DaemonShared>) {
        shared_options(collector, false, std::time::Duration::from_secs(45))
    }

    fn shared_options(
        collector: Collector,
        dev_dry_run: bool,
        timeout: std::time::Duration,
    ) -> (tempfile::TempDir, Arc<DaemonShared>) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().into()).unwrap();
        let mut shared = Arc::new(DaemonShared::load(store).unwrap());
        Arc::get_mut(&mut shared).unwrap().subscription_usage = Arc::new(UsageService {
            state: Mutex::new(State::default()),
            permits: Arc::new(Semaphore::new(MAX_PROBES)),
            collector,
            changed: tokio::sync::Notify::new(),
            timeout,
            token_cache: Arc::new(claude_token_cache::TokenCache::for_build()),
        });
        Arc::get_mut(&mut shared).unwrap().dev_dry_run = dev_dry_run;
        shared.adopt_runtime();
        shared.adopt_self();
        (dir, shared)
    }

    fn cached(shared: &DaemonShared) -> bool {
        shared
            .subscription_usage
            .token_cache
            .lookup("login", "binding", Utc::now())
            .is_some()
    }
    fn wait_until_uncached(shared: &DaemonShared) -> bool {
        for _ in 0..200 {
            if !cached(shared) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }
    fn remember(shared: &DaemonShared) {
        let cache = &shared.subscription_usage.token_cache;
        cache.remember(
            cache.epoch(),
            "login",
            "binding",
            (Utc::now() + Duration::hours(1)).timestamp_millis(),
            b"fixture-token",
        );
        assert!(cached(shared));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unenroll_and_turning_claude_off_delete_the_cached_claude_token() {
        let (_dir, shared) = shared(Arc::new(|_, _, _| {
            Box::pin(async {
                Err(ProbeFailure {
                    reason: "unavailable",
                    identity: None,
                })
            })
        }));
        remember(&shared);
        // Another setting changing while Claude stays on keeps it.
        settings_changed(&shared);
        assert!(cached(&shared));

        forget_readings(&shared);
        assert!(wait_until_uncached(&shared));

        remember(&shared);
        shared.settings.lock().unwrap().subscription_usage_claude_on = false;
        settings_changed(&shared);
        assert!(wait_until_uncached(&shared));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disabled_provider_never_invokes_an_injected_collector() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let (_dir, shared) = shared(Arc::new(move |_, _, _lease| {
            observed.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Err(ProbeFailure {
                    reason: "unavailable",
                    identity: None,
                })
            })
        }));
        tick(&shared, Utc::now());
        let request = Request {
            id: 1,
            method: "subscription_usage".into(),
            params: json!({"refresh":true,"interactive":true}),
        };
        assert!(handle(&shared, &request).error.is_none());
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!shared.store.dir().join("managed-profiles").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn developer_dry_run_refuses_probes_even_with_both_provider_switches_enabled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let (_dir, shared) = shared_options(
            Arc::new(move |_, _, _lease| {
                observed.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {
                    Err(ProbeFailure {
                        reason: "unavailable",
                        identity: None,
                    })
                })
            }),
            true,
            std::time::Duration::from_secs(45),
        );
        {
            let mut settings = shared.settings.lock().unwrap();
            settings.subscription_usage_codex_enabled = true;
            settings.subscription_usage_claude_enabled = true;
        }
        tick(&shared, Utc::now());
        let req = Request {
            id: 1,
            method: "subscription_usage".into(),
            params: json!({"refresh":true,"interactive":true}),
        };
        let value = handle(&shared, &req).result.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for provider in value["providers"].as_array().unwrap() {
            assert_eq!(provider["enabled"], false);
            assert_eq!(provider["logins"], json!([]));
        }
        assert!(!shared.store.dir().join("managed-profiles").exists());
    }

    #[test]
    fn daemon_runtime_shutdown_does_not_wait_for_an_unanswered_native_query() {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
        let (release, observed) = (Arc::clone(&gate), Arc::clone(&entered));
        let daemon = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let (_dir, shared) = shared(Arc::new(move |_, _, lease| {
                    let (release, observed, started) = (
                        Arc::clone(&release),
                        Arc::clone(&observed),
                        started_tx.clone(),
                    );
                    Box::pin(async move {
                        providers::native_read(lease, move || {
                            observed.fetch_add(1, Ordering::SeqCst);
                            let _ = started.send(());
                            let (lock, wake) = &*release;
                            let mut done = lock.lock().unwrap();
                            while !*done {
                                done = wake.wait(done).unwrap();
                            }
                            Err(ProbeFailure {
                                reason: "unavailable",
                                identity: None,
                            })
                        })
                        .await
                        .unwrap()
                    })
                }));
                // A saved ready fixture guarantees a probe even without a home directory.
                {
                    let mut managed = shared.managed.lock().unwrap();
                    let accounts = &mut managed.as_mut().unwrap().accounts;
                    let account = accounts
                        .add(ToolId::Codex, ConnectionKind::Subscription, "fixture")
                        .unwrap();
                    accounts.update_auth(account.id, AuthState::Ready).unwrap();
                }
                shared
                    .settings
                    .lock()
                    .unwrap()
                    .subscription_usage_codex_enabled = true;
                schedule(&shared, Utc::now(), true, true, None, None);
                tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    while entered.load(Ordering::SeqCst) == 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            });
            drop(runtime);
            let _ = stopped_tx.send(());
        });
        let started = started_rx.recv_timeout(std::time::Duration::from_secs(4));
        let stopped = stopped_rx.recv_timeout(std::time::Duration::from_secs(1));
        // Always unblock the fake OS query before assertions/join, including
        // the old spawn_blocking regression that held runtime shutdown here.
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let joined = daemon.join();
        assert!(
            started.is_ok(),
            "the fake native query must actually have started"
        );
        assert!(joined.is_ok(), "the daemon fixture failed");
        assert!(
            stopped.is_ok(),
            "shutdown waited for the unanswered native query"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn timed_out_collectors_keep_global_leases_until_native_blocking_jobs_finish() {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (seen, release) = (Arc::clone(&calls), Arc::clone(&gate));
        let (_dir, shared) = shared_options(
            Arc::new(move |_, _, lease| {
                let (seen, release) = (Arc::clone(&seen), Arc::clone(&release));
                Box::pin(async move {
                    providers::native_read(lease, move || {
                        seen.fetch_add(1, Ordering::SeqCst);
                        let (lock, wake) = &*release;
                        let mut done = lock.lock().unwrap();
                        while !*done {
                            done = wake.wait(done).unwrap();
                        }
                        Ok(Reading {
                            windows: vec![],
                            banked_resets: None,
                            identity: "fixture".into(),
                            native_revision: None,
                            source: ReadingSource::Api,
                        })
                    })
                    .await
                    .unwrap()
                })
            }),
            false,
            std::time::Duration::from_millis(30),
        );
        for i in 0..5 {
            let mut managed = shared.managed.lock().unwrap();
            let accounts = &mut managed.as_mut().unwrap().accounts;
            let account = accounts
                .add(
                    ToolId::Codex,
                    ConnectionKind::Subscription,
                    &format!("saved-{i}"),
                )
                .unwrap();
            accounts.update_auth(account.id, AuthState::Ready).unwrap();
        }
        shared
            .settings
            .lock()
            .unwrap()
            .subscription_usage_codex_enabled = true;
        schedule(&shared, Utc::now(), true, true, None, None);
        let timed_out = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let count = shared
                    .subscription_usage
                    .state
                    .lock()
                    .unwrap()
                    .entries
                    .values()
                    .filter(|entry| entry.reason == Some("timeout"))
                    .count();
                if count == MAX_PROBES {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        // Always release the native workers before asserting, including on failure.
        let started = calls.load(Ordering::SeqCst);
        let held = shared.subscription_usage.permits.available_permits();
        shared
            .settings
            .lock()
            .unwrap()
            .subscription_usage_codex_enabled = false;
        settings_changed(&shared);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        timed_out.unwrap();
        assert_eq!(
            started, MAX_PROBES,
            "async timeout must not admit queued native work"
        );
        assert_eq!(
            held, 0,
            "each still-running OS query must hold its global probe lease"
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while shared.subscription_usage.permits.available_permits() != MAX_PROBES {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_io_is_detached_bounded_and_disabling_discards_inflight_data() {
        struct ActiveGuard(Arc<AtomicUsize>);
        impl Drop for ActiveGuard {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));
        let (n, a, p, g) = (
            Arc::clone(&calls),
            Arc::clone(&active),
            Arc::clone(&peak),
            Arc::clone(&gate),
        );
        let (_dir, shared) = shared(Arc::new(move |_, _, _lease| {
            let (n, a, p, g) = (
                Arc::clone(&n),
                Arc::clone(&a),
                Arc::clone(&p),
                Arc::clone(&g),
            );
            Box::pin(async move {
                let running = a.fetch_add(1, Ordering::SeqCst) + 1;
                let _active = ActiveGuard(Arc::clone(&a));
                p.fetch_max(running, Ordering::SeqCst);
                n.fetch_add(1, Ordering::SeqCst);
                g.acquire().await.unwrap().forget();
                Ok(Reading {
                    windows: vec![],
                    banked_resets: Some(3),
                    identity: "fixture".into(),
                    native_revision: None,
                    source: ReadingSource::Api,
                })
            })
        }));
        for i in 0..8 {
            let mut managed = shared.managed.lock().unwrap();
            let accounts = &mut managed.as_mut().unwrap().accounts;
            let account = accounts
                .add(
                    ToolId::Codex,
                    ConnectionKind::Subscription,
                    &format!("saved-{i}"),
                )
                .unwrap();
            accounts.update_auth(account.id, AuthState::Ready).unwrap();
        }
        shared
            .settings
            .lock()
            .unwrap()
            .subscription_usage_codex_enabled = true;
        schedule(&shared, Utc::now(), true, true, None, None);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while calls.load(Ordering::SeqCst) < MAX_PROBES {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), MAX_PROBES);
        assert_eq!(calls.load(Ordering::SeqCst), MAX_PROBES);
        assert!(!shared.store.dir().join("managed-profiles").exists());
        shared
            .settings
            .lock()
            .unwrap()
            .subscription_usage_codex_enabled = false;
        settings_changed(&shared);
        gate.add_permits(MAX_PROBES);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while active.load(Ordering::SeqCst) > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert!(
            shared
                .subscription_usage
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .all(|entry| entry.login.provider != Provider::Codex)
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            MAX_PROBES,
            "queued probes must not read after disable"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_scoped_interactive_read_never_probes_the_other_enabled_provider() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::clone(&observed);
        let (_dir, shared) = shared(Arc::new(move |login, interactive, _lease| {
            calls.lock().unwrap().push((login.provider, interactive));
            Box::pin(async {
                Err(ProbeFailure {
                    reason: "unavailable",
                    identity: None,
                })
            })
        }));
        {
            let mut settings = shared.settings.lock().unwrap();
            settings.subscription_usage_codex_enabled = true;
            settings.subscription_usage_claude_enabled = true;
        }
        let req = Request {
            id: 1,
            method: "subscription_usage".into(),
            params: json!({"refresh":true,"interactive":true,"provider":"codex"}),
        };
        let value = handle(&shared, &req).result.unwrap();
        assert_eq!(value["providers"].as_array().unwrap().len(), 2);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while observed.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(*observed.lock().unwrap(), vec![(Provider::Codex, true)]);
        assert!(
            handle(
                &shared,
                &Request {
                    id: 2,
                    method: "subscription_usage".into(),
                    params: json!({"provider":"SECRET"})
                }
            )
            .error
            .is_some()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claude_is_default_available_and_only_selected_login_refreshes() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let (_dir, shared) = shared(Arc::new(move |login, interactive, _| {
            seen.lock().unwrap().push((login.id, interactive));
            Box::pin(async {
                Ok(Reading {
                    windows: vec![],
                    banked_resets: None,
                    identity: "fixture-owner".into(),
                    native_revision: None,
                    source: ReadingSource::Api,
                })
            })
        }));
        let mut ids = Vec::new();
        for label in ["one", "two"] {
            let mut managed = shared.managed.lock().unwrap();
            let accounts = &mut managed.as_mut().unwrap().accounts;
            let account = accounts
                .add(ToolId::Claude, ConnectionKind::Subscription, label)
                .unwrap();
            accounts.update_auth(account.id, AuthState::Ready).unwrap();
            ids.push(account.id.to_string());
        }
        assert!(
            !shared
                .settings
                .lock()
                .unwrap()
                .subscription_usage_claude_enabled
        );
        let snapshot = handle(
            &shared,
            &Request {
                id: 1,
                method: "subscription_usage".into(),
                params: json!({}),
            },
        )
        .result
        .unwrap();
        assert_eq!(snapshot["providers"][1]["enabled"], true);
        assert!(calls.lock().unwrap().is_empty());
        for params in [
            json!({"refresh":true,"provider":"claude"}),
            json!({"refresh":true,"provider":"claude","login":"../../other"}),
            json!({"refresh":true,"provider":"claude","login":"unknown"}),
            json!({"refresh":true,"login":ids[0]}),
            json!({"refresh":true,"provider":"codex","login":ids[0]}),
        ] {
            assert!(
                handle(
                    &shared,
                    &Request {
                        id: 2,
                        method: "subscription_usage".into(),
                        params
                    }
                )
                .error
                .is_some()
            );
        }
        assert!(handle(&shared, &Request { id: 3, method: "subscription_usage".into(), params: json!({"refresh":true,"interactive":true,"provider":"claude","login":ids[1]}) }).error.is_none());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if shared
                    .subscription_usage
                    .state
                    .lock()
                    .unwrap()
                    .entries
                    .get(&ids[1])
                    .is_some_and(|entry| entry.reading.is_some())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![(ids[1].clone(), true)]);
        let state = shared.subscription_usage.state.lock().unwrap();
        assert_eq!(state.entries[&ids[1]].reason, Some("captured_on_refresh"));
        assert!(state.entries[&ids[0]].reading.is_none());
        drop(state);
        forget_readings(&shared);
        synchronize(&shared);
        assert!(
            shared
                .subscription_usage
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .all(|entry| entry.reading.is_none())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forget_during_permission_wait_cancels_before_network_and_retains_lease() {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new(AtomicUsize::new(0));
        let network = Arc::new(AtomicUsize::new(0));
        let (release, started, sent) = (
            Arc::clone(&gate),
            Arc::clone(&entered),
            Arc::clone(&network),
        );
        let (_dir, shared) = shared(Arc::new(move |_, _, lease| {
            let (release, started, sent) = (
                Arc::clone(&release),
                Arc::clone(&started),
                Arc::clone(&sent),
            );
            Box::pin(async move {
                providers::native_read(lease, move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let mut done = release.0.lock().unwrap();
                    while !*done {
                        done = release.1.wait(done).unwrap();
                    }
                })
                .await
                .unwrap();
                sent.fetch_add(1, Ordering::SeqCst);
                Err(ProbeFailure {
                    reason: "unavailable",
                    identity: None,
                })
            })
        }));
        let id = {
            let mut managed = shared.managed.lock().unwrap();
            let accounts = &mut managed.as_mut().unwrap().accounts;
            let account = accounts
                .add(ToolId::Claude, ConnectionKind::Subscription, "saved")
                .unwrap();
            accounts.update_auth(account.id, AuthState::Ready).unwrap();
            account.id.to_string()
        };
        schedule(
            &shared,
            Utc::now(),
            true,
            true,
            Some(Provider::Claude),
            Some(&id),
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while entered.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        forget_readings(&shared);
        synchronize(&shared);
        assert_eq!(shared.subscription_usage.permits.available_permits(), 0);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while shared.subscription_usage.permits.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(network.load(Ordering::SeqCst), 0);
        assert!(
            shared
                .subscription_usage
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .all(|entry| entry.reading.is_none())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_verify_during_permission_wait_cancels_without_a_usage_tick() {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new(AtomicUsize::new(0));
        let network = Arc::new(AtomicUsize::new(0));
        let (release, started, sent) = (
            Arc::clone(&gate),
            Arc::clone(&entered),
            Arc::clone(&network),
        );
        let (_dir, shared) = shared(Arc::new(move |_, _, lease| {
            let (release, started, sent) = (
                Arc::clone(&release),
                Arc::clone(&started),
                Arc::clone(&sent),
            );
            Box::pin(async move {
                providers::native_read(lease, move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let mut done = release.0.lock().unwrap();
                    while !*done {
                        done = release.1.wait(done).unwrap();
                    }
                })
                .await
                .unwrap();
                sent.fetch_add(1, Ordering::SeqCst);
                Err(ProbeFailure {
                    reason: "unavailable",
                    identity: None,
                })
            })
        }));
        let id = {
            let mut managed = shared.managed.lock().unwrap();
            let accounts = &mut managed.as_mut().unwrap().accounts;
            let account = accounts
                .add(ToolId::Claude, ConnectionKind::Subscription, "saved")
                .unwrap();
            accounts.update_auth(account.id, AuthState::Ready).unwrap();
            account.id.to_string()
        };
        schedule(
            &shared,
            Utc::now(),
            true,
            true,
            Some(Provider::Claude),
            Some(&id),
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while entered.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shared
            .managed
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .inspect_profile_override = Some(|_| Ok(AuthState::SignInRequired));
        let response = super::super::managed::handle(
            &shared,
            &Request {
                id: 1,
                method: "managed_account_verify".into(),
                params: json!({"account_id":id}),
            },
        );
        assert!(response.error.is_none());

        assert_eq!(shared.subscription_usage.permits.available_permits(), 0);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while shared.subscription_usage.permits.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(network.load(Ordering::SeqCst), 0);
        assert!(
            shared
                .subscription_usage
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .all(|entry| entry.reading.is_none())
        );
    }

    #[tokio::test]
    async fn signed_out_saved_profiles_are_not_read_and_ready_profiles_have_revisions() {
        let (_dir, shared) = shared(Arc::new(|_, _, _lease| {
            Box::pin(async { panic!("enumeration must not collect") })
        }));
        let account = shared
            .managed
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .accounts
            .add(ToolId::Codex, ConnectionKind::Subscription, "saved")
            .unwrap();
        shared
            .settings
            .lock()
            .unwrap()
            .subscription_usage_codex_enabled = true;
        let (_, logins) = inputs(&shared);
        assert!(
            !logins
                .iter()
                .any(|login| login.id == account.id.to_string())
        );
        shared
            .managed
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .accounts
            .update_auth(account.id, AuthState::Ready)
            .unwrap();
        let (_, logins) = inputs(&shared);
        let managed = logins
            .iter()
            .find(|login| login.id == account.id.to_string())
            .unwrap();
        assert!(managed.metadata_revision.is_some());
        assert!(!shared.store.dir().join("managed-profiles").exists());
    }
}
