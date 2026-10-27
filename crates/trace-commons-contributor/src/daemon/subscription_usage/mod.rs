//! Current native subscription limits. Credentials and account fingerprints
//! stay in memory and never cross IPC; the display reads a bounded snapshot.

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

const POLL_SECONDS: i64 = 15 * 60;
const MANUAL_FLOOR_SECONDS: i64 = 60;
const CACHE_SECONDS: i64 = 60 * 60;
const MAX_LOGINS: usize = 128;
const MAX_PROBES: usize = 4;

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
    /// Claude may separately scope its native secure-storage item.
    pub secure_storage_dir: Option<String>,
    /// Native-account metadata revision, never a provider account identifier.
    pub metadata_revision: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Window {
    pub lane: String,
    pub label: Option<String>,
    pub used_permille: Option<u16>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub(super) struct Reading {
    pub windows: Vec<Window>,
    pub banked_resets: Option<u64>,
    /// Memory-only account fingerprint. Never derive a display label from it.
    pub identity: String,
}

#[derive(Clone, Debug)]
pub(super) struct ProbeFailure {
    pub reason: &'static str,
    pub identity: Option<String>,
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
    timeout: std::time::Duration,
}

fn global_probe_permits() -> Arc<Semaphore> {
    static PERMITS: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
    Arc::clone(PERMITS.get_or_init(|| Arc::new(Semaphore::new(MAX_PROBES))))
}

impl Default for UsageService {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            permits: global_probe_permits(),
            collector: Arc::new(|login, interactive, lease| {
                Box::pin(async move { providers::collect(&login, interactive, lease).await })
            }),
            timeout: std::time::Duration::from_secs(45),
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
                        reason: None,
                    },
                );
            } else if let Some(entry) = self.entries.get_mut(&login.id) {
                entry.login = login;
            }
        }
    }

    fn claims(
        &mut self,
        now: DateTime<Utc>,
        manual: bool,
        provider: Option<Provider>,
    ) -> Vec<Claim> {
        self.entries
            .values_mut()
            .filter_map(|entry| {
                if provider.is_some_and(|provider| entry.login.provider != provider) {
                    return None;
                }
                let interval = if manual {
                    MANUAL_FLOOR_SECONDS
                } else {
                    POLL_SECONDS
                };
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
                reading.windows.truncate(16);
                reading.windows.retain(|window| {
                    matches!(window.lane.as_str(), "session" | "weekly" | "model")
                });
                for window in &mut reading.windows {
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
                entry.reason = None;
            }
            Err(failure) => {
                let same_identity = failure
                    .identity
                    .as_ref()
                    .filter(|identity| !identity.is_empty())
                    .is_some_and(|identity| {
                        entry
                            .reading
                            .as_ref()
                            .is_some_and(|old| &old.identity == identity)
                    });
                let authentication = matches!(
                    failure.reason,
                    "auth_missing" | "auth_expired" | "auth_changed"
                );
                if authentication || !same_identity {
                    entry.reading = None;
                    entry.measured_at = None;
                }
                entry.reason = Some(failure.reason);
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
                    || entry.measured_at.is_some_and(|at| now >= at + Duration::seconds(POLL_SECONDS)));
                let windows: Vec<_> = reading.into_iter().flat_map(|reading| &reading.windows).map(|window| {
                    let used = window.used_permille.filter(|_| window.resets_at.is_none_or(|reset| reset > now));
                    json!({"lane":window.lane,"label":window.label,"used_permille":used,
                        "remaining_text":remaining_text(used),"resets_at":window.resets_at})
                }).collect();
                json!({"id":entry.login.id,"label":entry.login.label,"default_login":entry.login.default_login,
                    "state":if reading.is_some() {"observed"} else {"unknown"},"reason":entry.reason,
                    "updated_at":entry.measured_at,"stale":stale,"windows":windows,
                    "banked_resets":reading.and_then(|reading| reading.banked_resets)})
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
    secure_storage_dir: Option<String>,
) -> Option<LoginSpec> {
    let (folder, id, label) = match provider {
        Provider::Codex => (".codex", "codex-default", "Codex"),
        Provider::Claude => (".claude", "claude-default", "Claude"),
    };
    let root = match &config_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        Some(_) => return None,
        None => home?.join(folder),
    };
    Some(LoginSpec {
        provider,
        root,
        id: id.into(),
        label: label.into(),
        default_login: true,
        config_dir,
        secure_storage_dir,
        metadata_revision: None,
        native_executable: None,
    })
}

fn inputs(shared: &DaemonShared) -> ([bool; 2], Vec<LoginSpec>) {
    let enabled = shared
        .settings
        .lock()
        .map(|s| {
            [
                s.subscription_usage_codex_enabled,
                s.subscription_usage_claude_enabled,
            ]
        })
        .unwrap_or([false, false]);
    if !enabled.iter().any(|on| *on) {
        return (enabled, Vec::new());
    }
    let mut logins = Vec::new();
    let executables = [
        enabled[0]
            .then(|| crate::managed::profiles::find_native(ToolId::Codex).ok())
            .flatten(),
        enabled[1]
            .then(|| crate::managed::profiles::find_native(ToolId::Claude).ok())
            .flatten(),
    ];
    for (provider, _tool, override_name) in [
        (Provider::Codex, ToolId::Codex, "CODEX_HOME"),
        (Provider::Claude, ToolId::Claude, "CLAUDE_CONFIG_DIR"),
    ] {
        if enabled[provider.index()] {
            let secure = (provider == Provider::Claude)
                .then(|| std::env::var("CLAUDE_SECURESTORAGE_CONFIG_DIR").ok())
                .flatten();
            if let Some(mut login) = native_login(
                provider,
                dirs::home_dir(),
                std::env::var(override_name).ok(),
                secure,
            ) {
                login.native_executable = executables[provider.index()].clone();
                logins.push(login);
            }
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
        logins.push(LoginSpec {
            provider,
            config_dir: Some(root.to_string_lossy().into_owned()),
            root,
            id: account.id.to_string(),
            label: account.label,
            default_login: false,
            secure_storage_dir: None,
            metadata_revision: Some(format!(
                "{:?}:{:?}",
                account.auth_state, account.verified_at
            )),
            native_executable: executables[provider.index()].clone(),
        });
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
    }
}

fn schedule(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    manual: bool,
    interactive: bool,
    provider: Option<Provider>,
) {
    synchronize(shared);
    if shared.dev_dry_run {
        return;
    }
    let service = Arc::clone(&shared.subscription_usage);
    let claims = service
        .state
        .lock()
        .map(|mut state| {
            if manual && interactive {
                // The explicit permission-recovery action should not wait a minute
                // behind the background probe that discovered it needs a prompt.
                for entry in state.entries.values_mut().filter(|entry| {
                    entry.reason == Some("keychain_needs_permission")
                        && provider.is_none_or(|provider| entry.login.provider == provider)
                }) {
                    entry.last_started = None;
                }
            }
            state.claims(now, manual, provider)
        })
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
                    let result = tokio::time::timeout(
                        service.timeout,
                        (service.collector)(claim.login.clone(), interactive, Arc::clone(&lease)),
                    )
                    .await
                    .unwrap_or({
                        Err(ProbeFailure {
                            reason: "timeout",
                            identity: None,
                        })
                    });
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
pub(super) fn settings_changed(shared: &DaemonShared) {
    synchronize(shared);
    shared.publish(EVENT_STATUS_CHANGED, json!({}));
}

/// Cheap when disabled or not due. Provider I/O is detached from the pass.
pub(super) fn tick(shared: &DaemonShared, now: DateTime<Utc>) {
    schedule(shared, now, false, false, None);
}

pub(super) fn handle(shared: &DaemonShared, req: &Request) -> Response {
    let Some(params) = req.params.as_object() else {
        return Response::err(req.id, ERR_BAD_PARAMS, "subscription-usage-params-invalid");
    };
    if params
        .keys()
        .any(|key| !matches!(key.as_str(), "refresh" | "interactive" | "provider"))
        || ["refresh", "interactive"]
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
    let now = Utc::now();
    if refresh {
        schedule(shared, now, true, interactive, provider);
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
            provider: Provider::Claude,
            root: PathBuf::from(format!("/fake/{id}")),
            id: id.into(),
            label: id.into(),
            default_login: id == "default",
            config_dir: None,
            secure_storage_dir: None,
            metadata_revision: None,
            native_executable: None,
        }
    }
    fn reading(identity: &str, reset: DateTime<Utc>) -> Reading {
        Reading {
            identity: identity.into(),
            banked_resets: Some(2),
            windows: vec![Window {
                lane: "weekly".into(),
                label: None,
                used_permille: Some(420),
                resets_at: Some(reset),
            }],
        }
    }
    fn prepared() -> (State, Claim) {
        let mut state = State::default();
        state.synchronize([false, true], vec![login("default")]);
        let claim = state.claims(now(), true, None).pop().unwrap();
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
        assert!(state.claims(now(), true, None).is_empty());
        assert!(state.entries.is_empty());
        assert_eq!(state.value(now())["providers"][1]["enabled"], false);
    }
    #[test]
    fn disabling_or_removing_a_login_invalidates_its_inflight_completion() {
        let (mut state, claim) = prepared();
        state.synchronize([false, false], vec![]);
        state.synchronize([false, true], vec![login("default")]);
        state.complete(&claim, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries["default"].reading.is_none());
        let next = state.claims(now(), true, None).pop().unwrap();
        state.synchronize([false, true], vec![]);
        state.complete(&next, Ok(reading("old", now() + Duration::days(1))), now());
        assert!(state.entries.is_empty());
    }
    #[test]
    fn reconnect_metadata_invalidates_cache_and_inflight_but_renaming_does_not() {
        let (mut state, claim) = prepared();
        state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
        let mut same = login("default");
        same.label = "renamed".into();
        state.synchronize([false, true], vec![same.clone()]);
        assert!(state.entries["default"].reading.is_some());
        same.metadata_revision = Some("ready-new-verification".into());
        state.synchronize([false, true], vec![same]);
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
        let row = &value["providers"][1]["logins"][0];
        assert_eq!(row["stale"], true);
        assert_eq!(row["windows"][0]["remaining_text"], "58%");
        assert_eq!(
            state.value(reset)["providers"][1]["logins"][0]["windows"][0]["used_permille"],
            Value::Null
        );
        assert_eq!(
            state.value(now() + Duration::hours(1))["providers"][1]["logins"][0]["banked_resets"],
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
    fn unknown_failure_identity_and_auth_loss_clear_prior_percentages() {
        for error in [
            ProbeFailure {
                reason: "timeout",
                identity: None,
            },
            ProbeFailure {
                reason: "auth_expired",
                identity: Some("a".into()),
            },
        ] {
            let (mut state, claim) = prepared();
            state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
            state.complete(&claim, Err(error), now());
            assert!(state.entries["default"].reading.is_none());
        }
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
        assert!(state.claims(now(), true, None).is_empty());
        state.complete(&claim, Ok(reading("a", now() + Duration::days(1))), now());
        assert!(
            state
                .claims(now() + Duration::seconds(59), true, None)
                .is_empty()
        );
        assert!(
            state
                .claims(now() + Duration::minutes(14), false, None)
                .is_empty()
        );
        assert_eq!(
            state
                .claims(now() + Duration::minutes(15), false, None)
                .len(),
            1
        );
    }
    #[test]
    fn a_focused_refresh_claims_only_the_selected_provider() {
        let mut state = State::default();
        let mut codex = login("codex-default");
        codex.provider = Provider::Codex;
        state.synchronize([true, true], vec![codex, login("default")]);
        let claims = state.claims(now(), true, Some(Provider::Codex));
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].login.provider, Provider::Codex);
        assert!(!state.entries["default"].refreshing);
    }

    #[test]
    fn separate_logins_do_not_share_readings_and_cache_size_is_bounded() {
        let mut state = State::default();
        state.synchronize(
            [false, true],
            (0..MAX_LOGINS + 8).map(|i| login(&i.to_string())).collect(),
        );
        assert_eq!(state.entries.len(), MAX_LOGINS);
        let claims = state.claims(now(), true, None);
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
    fn native_overrides_keep_the_exact_profile_and_secure_storage_scope() {
        let home = Some(PathBuf::from("/fake/home"));
        let normal = native_login(Provider::Claude, home.clone(), None, None).unwrap();
        assert_eq!(normal.root, PathBuf::from("/fake/home/.claude"));
        assert!(normal.config_dir.is_none());
        let overridden = native_login(
            Provider::Claude,
            home.clone(),
            Some("/fake/home/.claude/".into()),
            Some(String::new()),
        )
        .unwrap();
        assert_eq!(overridden.root, PathBuf::from("/fake/home/.claude/"));
        assert_eq!(
            overridden.config_dir.as_deref(),
            Some("/fake/home/.claude/")
        );
        assert_eq!(overridden.secure_storage_dir.as_deref(), Some(""));
        let codex =
            native_login(Provider::Codex, None, Some("/fake/codex-work".into()), None).unwrap();
        assert_eq!(codex.root, PathBuf::from("/fake/codex-work"));
        assert!(codex.default_login);
        assert!(native_login(Provider::Claude, None, None, None).is_none());
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
            timeout,
        });
        Arc::get_mut(&mut shared).unwrap().dev_dry_run = dev_dry_run;
        shared.adopt_runtime();
        shared.adopt_self();
        (dir, shared)
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
                        .add(ToolId::Claude, ConnectionKind::Subscription, "fixture")
                        .unwrap();
                    accounts.update_auth(account.id, AuthState::Ready).unwrap();
                }
                shared
                    .settings
                    .lock()
                    .unwrap()
                    .subscription_usage_claude_enabled = true;
                tick(&shared, Utc::now());
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
                    ToolId::Claude,
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
            .subscription_usage_claude_enabled = true;
        tick(&shared, Utc::now());
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
            .subscription_usage_claude_enabled = false;
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
                p.fetch_max(running, Ordering::SeqCst);
                n.fetch_add(1, Ordering::SeqCst);
                g.acquire().await.unwrap().forget();
                a.fetch_sub(1, Ordering::SeqCst);
                Ok(Reading {
                    windows: vec![],
                    banked_resets: Some(3),
                    identity: "fixture".into(),
                })
            })
        }));
        for i in 0..8 {
            let mut managed = shared.managed.lock().unwrap();
            let accounts = &mut managed.as_mut().unwrap().accounts;
            let account = accounts
                .add(
                    ToolId::Claude,
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
            .subscription_usage_claude_enabled = true;
        tick(&shared, Utc::now());
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
            .subscription_usage_claude_enabled = false;
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
                .is_empty()
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
            .add(ToolId::Claude, ConnectionKind::Subscription, "saved")
            .unwrap();
        shared
            .settings
            .lock()
            .unwrap()
            .subscription_usage_claude_enabled = true;
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
        assert!(managed.secure_storage_dir.is_none());
        assert!(!shared.store.dir().join("managed-profiles").exists());
    }

    #[test]
    fn defaults_are_off_and_invalid_setting_types_are_refused() {
        let mut settings = super::super::settings::DaemonSettings::default();
        assert!(
            !settings.subscription_usage_codex_enabled
                && !settings.subscription_usage_claude_enabled
        );
        let patch = json!({"subscription_usage_codex_enabled":true,"subscription_usage_claude_enabled":true});
        assert!(super::super::settings::apply_settings_object(&mut settings, &patch).unwrap());
        assert!(
            settings.subscription_usage_codex_enabled && settings.subscription_usage_claude_enabled
        );
        assert!(
            super::super::settings::apply_settings_object(
                &mut settings,
                &json!({"subscription_usage_codex_enabled":"yes"})
            )
            .is_err()
        );
    }
}
