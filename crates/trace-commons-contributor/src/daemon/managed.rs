//! One local source of truth for CLI and desktop managed sessions.

use super::ipc::{DaemonShared, Request, Response};
use crate::{
    config::ConfigStore,
    managed::{
        AccountId, AccountView, AuthState, ConnectionKind, Generation, ManagedError, Selection,
        ToolId,
        accounts::AccountStore,
        models,
        profiles::{NativeProfiles, find_native},
        route::{ManagedRoute, near_ai_base_url},
        secrets::{ApiKey, SecretStore},
        sessions::{
            LaunchContext, LaunchPurpose, LaunchRequest, LifecycleReport, SessionRegistry,
            SessionState, SessionView,
        },
        signin,
        terminal::{self, TerminalApp, TerminalHandle, TerminalSetting},
        tool_setup::{self, ToolSetupAction},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub struct ManagedService {
    pub accounts: AccountStore,
    #[cfg(test)]
    pub(super) inspect_profile_override:
        Option<fn(&AccountView) -> Result<AuthState, ManagedError>>,
    pub sessions: SessionRegistry,
    profiles: NativeProfiles,
    config_dir: std::path::PathBuf,
    secrets: SecretStore,
    /// Proxies the daemon hosts for sessions on the NEAR AI sign-in, by
    /// session id. Only the daemon holds the sign-in credential.
    routes: std::collections::HashMap<Uuid, ManagedRoute>,
    /// Which terminal apps are installed. A field so tests answer it
    /// without asking the OS; asked through `installed_cache`.
    installed_terminals: fn() -> Vec<TerminalApp>,
    /// The last answer, reused for a short window so a snapshot does not
    /// start a probe process each time a shell asks.
    installed_cache: terminal::InstalledCache,
    /// Brings a session's window to the front. A field so tests can record
    /// the call rather than drive a real terminal.
    focus: fn(&TerminalHandle) -> Result<(), ManagedError>,
    setup_terminal: fn(TerminalSetting, &[TerminalApp], &str) -> Result<(), ManagedError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionParams {
    session_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RedeemParams {
    session_id: Uuid,
    ticket: String,
}

impl ManagedService {
    pub fn open(store: &ConfigStore) -> Result<Self, ManagedError> {
        Ok(Self {
            accounts: AccountStore::open(store)?,
            #[cfg(test)]
            inspect_profile_override: None,
            sessions: SessionRegistry::open(store)?,
            profiles: NativeProfiles::new(store),
            config_dir: store.dir().to_path_buf(),
            secrets: SecretStore::new(store),
            routes: std::collections::HashMap::new(),
            installed_terminals: terminal::installed,
            installed_cache: terminal::InstalledCache::default(),
            focus: crate::managed::terminal::focus,
            setup_terminal: terminal::launch_tool_setup,
        })
    }

    /// The installed terminal apps, from the cache when it is fresh and
    /// holds `launching`.
    fn installed_terminals(&mut self, launching: Option<TerminalApp>) -> Vec<TerminalApp> {
        let probe = self.installed_terminals;
        self.installed_cache
            .get(std::time::Instant::now(), launching, probe)
    }

    fn inspect_profile(&self, account: &AccountView) -> Result<AuthState, ManagedError> {
        #[cfg(test)]
        if let Some(inspect) = self.inspect_profile_override {
            return inspect(account);
        }
        self.profiles.inspect(account, &find_native(account.tool)?)
    }

    /// The account a launch request names: the NEAR AI sign-in's entry
    /// for its tool, only while signed in and only for coding, or a saved
    /// account.
    fn launch_account(
        &self,
        p: &LaunchRequest,
        sign_in: bool,
    ) -> Result<AccountView, ManagedError> {
        if p.connection == ConnectionKind::NearAiSignIn {
            if !sign_in {
                return Err(ManagedError::NearAiSignInRequired);
            }
            if signin::tool_for(p.account_id) != Some(p.tool) || p.purpose != LaunchPurpose::Coding
            {
                return Err(ManagedError::InvalidRequest);
            }
            signin::entry(p.tool).ok_or(ManagedError::InvalidRequest)
        } else {
            self.account(p.account_id)
        }
    }

    /// Whether an ended coding row can be launched again as it was: same
    /// account (still saved, or still signed in) and same folder (still
    /// there). A shell also needs `capabilities.terminal_launch` to offer it,
    /// as it does for any launch.
    fn can_start_again(&self, view: &SessionView, sign_in: bool) -> bool {
        !view.state.holds_profile()
            && view.purpose == LaunchPurpose::Coding
            && view.cwd.is_absolute()
            && view.cwd.is_dir()
            && self
                .launch_account(
                    &LaunchRequest {
                        cli_options: None,
                        model: None,
                        collect_traces: false,
                        purpose: view.purpose,
                        request_id: view.id,
                        tool: view.tool,
                        connection: view.connection,
                        account_id: view.account_id,
                        cwd: view.cwd.clone(),
                        expected_generation: Generation(0),
                        save_default: false,
                    },
                    sign_in,
                )
                .is_ok_and(|a| a.tool == view.tool && a.connection == view.connection)
    }

    /// Redeem a ticket. A session on the NEAR AI sign-in is refused without
    /// a sign-in, and is failed so it never waits out its ticket. The bool
    /// says whether the caller must host the session's proxy.
    fn redeem(
        &mut self,
        session_id: Uuid,
        ticket: &str,
        sign_in: bool,
    ) -> Result<(LaunchContext, bool), ManagedError> {
        let on_sign_in = matches!(
            self.sessions.describe(session_id),
            Some((ConnectionKind::NearAiSignIn, _))
        );
        if on_sign_in && !sign_in {
            self.sessions.fail_unredeemed(session_id, ticket)?;
            return Err(ManagedError::NearAiSignInRequired);
        }
        Ok((self.sessions.redeem(session_id, ticket)?, on_sign_in))
    }

    #[cfg(test)]
    pub(crate) fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Take the proxies whose sessions no longer run, or every proxy.
    fn take_routes(&mut self, all: bool) -> Vec<ManagedRoute> {
        let ids: Vec<Uuid> = self
            .routes
            .keys()
            .copied()
            .filter(|id| {
                all || !self
                    .sessions
                    .describe(*id)
                    .is_some_and(|(_, state)| state.holds_profile())
            })
            .collect();
        ids.into_iter()
            .filter_map(|id| self.routes.remove(&id))
            .collect()
    }

    fn account(&self, id: AccountId) -> Result<AccountView, ManagedError> {
        self.accounts
            .list()
            .into_iter()
            .find(|a| a.id == id)
            .ok_or(ManagedError::NotFound)
    }

    /// `sign_in` says only whether the daemon holds a NEAR AI sign-in;
    /// `choice` is the terminal the settings name.
    fn handle(
        &mut self,
        request: &Request,
        sign_in: bool,
        choice: TerminalSetting,
    ) -> Result<Value, ManagedError> {
        match request.method.as_str() {
            "managed_snapshot" => {
                let mut sessions = self.sessions.views()?;
                for view in &mut sessions {
                    view.can_start_again = self.can_start_again(view, sign_in);
                }
                let saved = [
                    self.accounts.selection(ToolId::Claude),
                    self.accounts.selection(ToolId::Codex),
                    self.accounts.selection(ToolId::Opencode),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
                let defaults =
                    signin::defaults(saved, sign_in, |tool| self.accounts.generation(tool));
                // The offered apps found here, for a shell's picker.
                let choices: Vec<Value> = self
                    .installed_terminals(None)
                    .into_iter()
                    .map(|app| json!({"id":app.id(),"name":app.name()}))
                    .collect();
                // Off macOS, with no offered terminal found there is nothing
                // to choose and nothing to open: Launch stays off and the
                // shells show their "unavailable" line, rather than a
                // refusal pointing at a picker that is not drawn. macOS
                // always has Terminal, so a failed probe never dims it.
                let terminal_launch = terminal::launch_available()
                    && (cfg!(target_os = "macos") || !choices.is_empty());
                Ok(
                    json!({"copy":crate::managed::copy::copy(),"tools":crate::managed::profiles::tools(),"cli_options":crate::managed::options::snapshot(&ConfigStore::open(self.config_dir.clone()).map_err(|_| ManagedError::StorageUnavailable)?)?,"revision":self.sessions.revision(),"has_launched":self.sessions.has_launched(),"accounts":self.accounts.list(),"sign_in_accounts":signin::entries(sign_in),"generations":{"claude":self.accounts.generation(ToolId::Claude),"codex":self.accounts.generation(ToolId::Codex),"opencode":self.accounts.generation(ToolId::Opencode)},"defaults":defaults,"sessions":sessions,"capabilities":{"tool_setup":terminal::tool_setup_available(),"managed_launch":true,"folder_settings":true,"near_ai_signin":sign_in,"terminal_launch":terminal_launch,"terminal_destination":terminal::destination(choice),"terminal_current":choice.app().map(TerminalApp::id),"terminal_choices":choices}}),
                )
            }
            "managed_tool_setup" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    tool: ToolId,
                    action: ToolSetupAction,
                }
                let params: Params = parse(request)?;
                let native = find_native(params.tool).ok();
                let command = tool_setup::command(params.tool, params.action, native.as_deref())?;
                let installed = self.installed_terminals(choice.app());
                (self.setup_terminal)(choice, &installed, &command)?;
                // This acknowledges only the visible terminal handoff. The
                // installer runs there; the next snapshot re-discovers tools.
                Ok(json!({"opened": true}))
            }
            "managed_account_add" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    tool: ToolId,
                    connection: ConnectionKind,
                    label: String,
                }
                let p: Params = parse(request)?;
                let account = self.accounts.add(p.tool, p.connection, &p.label)?;
                self.sessions.changed()?;
                Ok(json!(account))
            }
            "managed_select" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    selection: Selection,
                    expected_generation: Generation,
                }
                let p: Params = parse(request)?;
                let selection = self.accounts.select(p.selection, p.expected_generation)?;
                self.sessions.changed()?;
                Ok(json!(selection))
            }
            "managed_launch_prepare" => {
                // No folder named: the person's home directory.
                let params = crate::managed::folder::with_default_cwd(
                    request.params.clone(),
                    dirs::home_dir(),
                )?;
                let p: LaunchRequest =
                    serde_json::from_value(params).map_err(|_| ManagedError::InvalidRequest)?;
                let account = self.launch_account(&p, sign_in)?;
                let generation = self.accounts.generation(p.tool);
                if p.expected_generation != generation {
                    return Err(ManagedError::Conflict);
                }
                // Default changes are separate until the cross-file transaction is available.
                if p.save_default {
                    return Err(ManagedError::InvalidRequest);
                }
                verify_visible_version(&self.profiles, &account)?;
                Ok(json!(self.sessions.prepare(p, &account)?))
            }
            "managed_terminal_launch" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    session_id: Uuid,
                    ticket: String,
                }
                let p: Params = parse(request)?;
                let cwd = self.sessions.launch_cwd(p.session_id, &p.ticket)?;
                let installed = self.installed_terminals(choice.app());
                let window = terminal::launch(
                    choice,
                    &installed,
                    &self.config_dir,
                    &cwd,
                    p.session_id,
                    &p.ticket,
                )?;
                self.sessions.opened_in_terminal(p.session_id, window)?;
                Ok(json!({"opened":true,"destination":terminal::destination(choice)}))
            }
            "managed_launch_redeem" => {
                let p: RedeemParams = parse(request)?;
                // A sign-in session needs a daemon-hosted proxy, which only
                // the asynchronous path can start. Refuse before redeeming.
                if matches!(
                    self.sessions.describe(p.session_id),
                    Some((ConnectionKind::NearAiSignIn, _))
                ) {
                    return Err(ManagedError::LaunchUnknown);
                }
                Ok(json!(self.redeem(p.session_id, &p.ticket, sign_in)?.0))
            }
            "managed_session_report" => {
                let report: LifecycleReport = parse(request)?;
                let id = report.session_id;
                self.sessions.report(report)?;
                // Additive: an older launcher reads only `accepted`.
                Ok(json!({"accepted":true,"stop":self.sessions.stop_requested(id)}))
            }
            "managed_session_focus" => {
                let p: SessionParams = parse(request)?;
                let window = self.sessions.terminal_for(p.session_id)?;
                (self.focus)(&window)?;
                Ok(json!({"focused":true}))
            }
            "managed_session_stop" => {
                let p: SessionParams = parse(request)?;
                self.sessions.request_stop(p.session_id)?;
                Ok(json!({"stopping":true}))
            }
            "managed_session_start_again" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    session_id: Uuid,
                    request_id: Uuid,
                }
                let p: Params = parse(request)?;
                let mut again = self
                    .sessions
                    .start_again_request(p.session_id, p.request_id)?;
                if let Some(existing) = self.sessions.replayed(&again) {
                    return Ok(json!(existing));
                }
                let account = self.launch_account(&again, sign_in)?;
                if !again.cwd.is_absolute() || !again.cwd.is_dir() {
                    return Err(ManagedError::InvalidRequest);
                }
                again.expected_generation = self.accounts.generation(again.tool);
                Ok(json!(self.sessions.prepare(again, &account)?))
            }
            "managed_session_dismiss" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    session_id: Uuid,
                }
                let p: Params = parse(request)?;
                self.sessions.dismiss(p.session_id)?;
                Ok(json!({"dismissed":true}))
            }
            "managed_account_rename" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    account_id: AccountId,
                    label: String,
                }
                let p: Params = parse(request)?;
                let account = self.accounts.rename(p.account_id, &p.label)?;
                self.sessions.changed()?;
                Ok(json!(account))
            }
            "managed_account_set_key" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    account_id: AccountId,
                    key: String,
                }
                let p: Params = parse(request)?;
                let account = self.account(p.account_id)?;
                if account.connection == ConnectionKind::Subscription {
                    return Err(ManagedError::InvalidRequest);
                }
                if self.sessions.holds_profile(p.account_id) {
                    return Err(ManagedError::Conflict);
                }
                self.secrets.put(p.account_id, &ApiKey::new(p.key)?)?;
                let account = self.accounts.update_auth(p.account_id, AuthState::Ready)?;
                self.sessions.changed()?;
                Ok(json!(account))
            }
            "managed_account_verify" => {
                let p: AccountParams = parse(request)?;
                let account = self.account(p.account_id)?;
                let state = match account.connection {
                    ConnectionKind::Subscription => self.inspect_profile(&account)?,
                    _ => {
                        self.secrets.get(p.account_id)?;
                        AuthState::Ready
                    }
                };
                let account = self.accounts.update_auth(p.account_id, state)?;
                self.sessions.changed()?;
                Ok(json!(account))
            }
            "managed_account_reconnect" => {
                let p: AccountParams = parse(request)?;
                let account = self.account(p.account_id)?;
                if account.connection != ConnectionKind::Subscription {
                    return Err(ManagedError::InvalidRequest);
                }
                if self.sessions.holds_profile(p.account_id) {
                    return Err(ManagedError::Conflict);
                }
                verify_visible_version(&self.profiles, &account)?;
                let request = LaunchRequest {
                    cli_options: None,
                    model: None,
                    collect_traces: false,
                    request_id: Uuid::new_v4(),
                    purpose: LaunchPurpose::Login,
                    tool: account.tool,
                    connection: account.connection,
                    account_id: account.id,
                    cwd: self.profiles.profile_root(account.id)?,
                    expected_generation: self.accounts.generation(account.tool),
                    save_default: false,
                };
                Ok(json!(self.sessions.prepare(request, &account)?))
            }
            "managed_account_remove" => {
                let p: AccountParams = parse(request)?;
                let account = self.account(p.account_id)?;
                if self.sessions.holds_profile(p.account_id) {
                    return Err(ManagedError::Conflict);
                }
                if account.connection == ConnectionKind::Subscription {
                    self.profiles
                        .remove(&account, &find_native(account.tool)?)?;
                } else {
                    self.secrets.delete(account.id)?;
                    self.profiles.remove_files(account.id)?;
                }
                self.accounts.remove(account.id)?;
                self.sessions.changed()?;
                Ok(json!({"removed":true}))
            }
            _ => Err(ManagedError::InvalidRequest),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountParams {
    account_id: AccountId,
}

fn parse<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, ManagedError> {
    serde_json::from_value(request.params.clone()).map_err(|_| ManagedError::InvalidRequest)
}

/// The sign-in credential, read at the moment it is needed. Every other
/// path is told a bool.
fn sign_in_key(shared: &DaemonShared) -> Option<String> {
    shared
        .settings
        .lock()
        .ok()?
        .near_ai_inference
        .as_ref()
        .map(|c| c.key.clone())
}

/// The terminal the settings name, read per request like the sign-in.
fn terminal_choice(shared: &DaemonShared) -> TerminalSetting {
    shared
        .settings
        .lock()
        .map(|s| s.managed_terminal)
        .unwrap_or(TerminalSetting::Unrecognized)
}

/// The terminal choice changed: move the revision so every shell re-reads
/// the snapshot, whose destination and picker follow it.
pub fn terminal_changed(shared: &DaemonShared) {
    let _ = with_service(shared, |service| service.sessions.changed());
}

/// Canonical launch options changed through the shared settings writer.
/// Called only after the settings and commit locks have been released.
pub fn options_changed(shared: &DaemonShared) -> Result<u64, &'static str> {
    with_service(shared, |service| {
        service.sessions.changed()?;
        Ok(service.sessions.revision())
    })
    .map_err(refusal_label)
}

/// Read acknowledgment state together, so a concurrent later save cannot be
/// represented by the old value carrying a newer managed revision.
pub fn options_acknowledgment(shared: &DaemonShared, tool: ToolId) -> Result<Value, &'static str> {
    with_service(shared, |service| {
        let store = ConfigStore::open(service.config_dir.clone()).map_err(|_| ManagedError::StorageUnavailable)?;
        let options = crate::managed::options::snapshot(&store)?;
        Ok(json!({"tool":tool,"options":options.get(&tool).cloned().unwrap_or_default(),"revision":service.sessions.revision()}))
    }).map_err(refusal_label)
}

pub fn folder_snapshot(
    shared: &DaemonShared,
    tool: ToolId,
    cwd: &std::path::Path,
) -> Result<Value, &'static str> {
    with_service(shared, |service| {
        let store = ConfigStore::open(service.config_dir.clone())
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let mut value = serde_json::to_value(crate::managed::folder::lookup(&store, tool, cwd)?)
            .map_err(|_| ManagedError::StorageUnavailable)?;
        value["revision"] = service.sessions.revision().into();
        Ok(value)
    })
    .map_err(refusal_label)
}

fn remember_prepared(
    shared: &DaemonShared,
    request: &Request,
    result: Result<Value, Refusal>,
) -> Result<Value, Refusal> {
    let value = result?;
    if matches!(
        request.method.as_str(),
        "managed_launch_prepare" | "managed_session_start_again"
    ) && value.get("ticket").is_some_and(Value::is_string)
    {
        let id: Uuid = serde_json::from_value(value["session_id"].clone())
            .map_err(|_| ManagedError::InvalidRequest)?;
        let prefs = with_service(shared, |service| Ok(service.sessions.folder_settings(id)))?;
        if let Some(prefs) = prefs {
            let write = Request {
                id: request.id,
                method: "set_settings".into(),
                params: json!({"managed_folder_settings":[prefs]}),
            };
            let response = super::ipc::handle_set_settings(shared, &write);
            if response.error.is_some() {
                with_service(shared, |service| service.sessions.rollback_prepared(id))?;
                return Err(Refusal::Unavailable("managed-folder-settings-save-failed"));
            }
        }
    }
    Ok(value)
}

fn refusal_label(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::Unavailable(value) => value,
        Refusal::Managed(error) => label(error),
    }
}

fn sign_in_present(shared: &DaemonShared) -> bool {
    shared
        .settings
        .lock()
        .is_ok_and(|s| s.near_ai_inference.is_some())
}

/// Why a managed request got no answer from the service.
enum Refusal {
    /// The service itself could not be reached.
    Unavailable(&'static str),
    Managed(ManagedError),
}

impl From<ManagedError> for Refusal {
    fn from(error: ManagedError) -> Self {
        Self::Managed(error)
    }
}

/// Run `f` on the service, publishing a change if the registry moved.
fn with_service<T>(
    shared: &DaemonShared,
    f: impl FnOnce(&mut ManagedService) -> Result<T, ManagedError>,
) -> Result<T, Refusal> {
    let Ok(mut held) = shared.managed.lock() else {
        return Err(Refusal::Unavailable("managed-service-unavailable"));
    };
    let service = match held.as_mut() {
        Ok(service) => service,
        Err(_) => return Err(Refusal::Unavailable("managed-storage-unavailable")),
    };
    let before = service.sessions.revision();
    let before_accounts = service.accounts.list();
    let result = f(service);
    let after = service.sessions.revision();
    let accounts_changed = service.accounts.list() != before_accounts;
    // Usage synchronization reads this registry too. Release the mutex first,
    // and synchronize even after an error: an account write may have committed
    // before persisting the session revision failed. Pending native permission
    // results must not authorize HTTP for a removed or rebound account.
    drop(held);
    if before != after || accounts_changed {
        super::subscription_usage::settings_changed(shared);
    }
    if before != after {
        shared.publish("managed_changed", json!({"revision":after}));
    }
    Ok(result?)
}

fn respond(request: &Request, result: Result<Value, Refusal>) -> Response {
    match result {
        Ok(value) => Response::ok(request.id, value),
        Err(Refusal::Unavailable(code)) => Response::err(request.id, "unavailable", code),
        Err(Refusal::Managed(error)) => {
            let code = label(error);
            Response::err(request.id, code, code)
        }
    }
}

/// Synchronous entry. A proxy whose session ended is dropped, which
/// requests its shutdown; the asynchronous entry also awaits it.
pub fn handle(shared: &DaemonShared, request: &Request) -> Response {
    let sign_in = sign_in_present(shared);
    let choice = terminal_choice(shared);
    let mut ended = vec![];
    let result = with_service(shared, |service| {
        let value = service.handle(request, sign_in, choice);
        ended = service.take_routes(false);
        value
    });
    drop(ended);
    respond(request, remember_prepared(shared, request, result))
}

/// Every managed method on the socket. `managed_launch_redeem` is where a
/// session on the NEAR AI sign-in gets its proxy; the others are the
/// synchronous handler, then a shutdown of proxies whose sessions ended.
pub async fn handle_async(shared: &DaemonShared, request: &Request) -> Response {
    if request.method == "managed_launch_redeem" {
        return redeem_async(shared, request).await;
    }
    if request.method == "managed_model_catalogue" {
        return model_catalogue(shared, request).await;
    }
    if request.method == "managed_model_favorite_set" {
        return favorite_set(shared, request).await;
    }
    let sign_in = sign_in_present(shared);
    let choice = terminal_choice(shared);
    let mut ended = vec![];
    let result = with_service(shared, |service| {
        let value = service.handle(request, sign_in, choice);
        ended = service.take_routes(false);
        value
    });
    for route in ended {
        route.shutdown().await;
    }
    respond(request, remember_prepared(shared, request, result))
}

/// NEAR AI's chat models, read without a key. `None` when NEAR AI cannot
/// be read.
async fn fetch_chat_models() -> Option<Vec<models::ChatModel>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    models::fetch(&client, near_ai_base_url()).await
}

/// The stored favorite model ids.
fn favorite_models(shared: &DaemonShared) -> Vec<String> {
    shared
        .settings
        .lock()
        .map(|s| s.managed_favorite_models.clone())
        .unwrap_or_default()
}

/// The NEAR AI chat models a launch can name, and the one `tool` gets
/// when it names none (`managed::models`). The catalogue is public: no
/// key is read or sent. Answers `{"models":[{"id","name"?}],"default",
/// "favorites"}`, where `favorites` is the starred models among `models`,
/// in NEAR AI's order.
async fn model_catalogue(shared: &DaemonShared, request: &Request) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        tool: ToolId,
    }
    let p: Params = match parse(request) {
        Ok(p) => p,
        Err(error) => return respond(request, Err(error.into())),
    };
    let offered = fetch_chat_models()
        .await
        .map(|models| models::offered(p.tool, models));
    let result = match offered {
        Some(models) if !models.is_empty() => {
            let default = models::default_for(p.tool, &models).map(|m| m.id.clone());
            let starred = favorite_models(shared);
            let favorites: Vec<&str> = models
                .iter()
                .map(|m| m.id.as_str())
                .filter(|id| starred.iter().any(|s| s == id))
                .collect();
            Ok(json!({"models":models,"default":default,"favorites":favorites}))
        }
        _ => Err(ManagedError::ModelUnavailable.into()),
    };
    respond(request, result)
}

/// `managed_model_favorite_set {model, favorite}`: star or unstar one model
/// for the Launch sheet's picker. Starring reads NEAR AI's chat models and
/// refuses one it does not list, so only an offered model is ever added;
/// unstarring reads nothing. Writes through `set_settings`, whose checks
/// (id shape, list bound) apply too. Answers `{"favorites"}`, the stored
/// list. No lock is held across the read.
async fn favorite_set(shared: &DaemonShared, request: &Request) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        model: String,
        favorite: bool,
    }
    let p: Params = match parse(request) {
        Ok(p) => p,
        Err(error) => return respond(request, Err(error.into())),
    };
    if !crate::daemon::settings::valid_favorite_model_id(&p.model) {
        return respond(request, Err(ManagedError::InvalidRequest.into()));
    }
    if p.favorite {
        match fetch_chat_models().await {
            Some(models) if models.iter().any(|m| m.id == p.model) => {}
            _ => return respond(request, Err(ManagedError::ModelUnavailable.into())),
        }
    }
    let write = Request {
        id: request.id,
        method: "set_settings".into(),
        params: json!({"managed_favorite_models":{ p.model: p.favorite }}),
    };
    let response = super::ipc::handle_set_settings(shared, &write);
    if let Some(error) = &response.error {
        // A full list is the only refusal a well-formed id can meet here;
        // a failed write keeps its own answer.
        if error.message == crate::daemon::settings::ERR_SETTINGS_INVALID_VALUE {
            return respond(request, Err(ManagedError::InvalidRequest.into()));
        }
        return response;
    }
    respond(request, Ok(json!({"favorites": favorite_models(shared)})))
}

/// The daemon reads the sign-in here, at launch time, and gives it only to
/// the session's proxy. The launcher is told a loopback port and the model
/// the proxy pinned, nothing else.
async fn redeem_async(shared: &DaemonShared, request: &Request) -> Response {
    let p: RedeemParams = match parse(request) {
        Ok(p) => p,
        Err(error) => return respond(request, Err(error.into())),
    };
    let key = sign_in_key(shared);
    let redeemed = with_service(shared, |service| {
        service.redeem(p.session_id, &p.ticket, key.is_some())
    });
    let (mut context, on_sign_in) = match redeemed {
        Ok(redeemed) => redeemed,
        Err(refusal) => return respond(request, Err(refusal)),
    };
    if !on_sign_in {
        return respond(request, Ok(json!(context)));
    }
    // No lock is held across the start: it waits on the network.
    let started = match key.map(ApiKey::new) {
        Some(Ok(key)) => ManagedRoute::start(
            &shared.store,
            p.session_id,
            key,
            context.session.tool,
            context.session.model.as_deref(),
        )
        .await
        .map_err(|error| match error {
            // The proxy reached no model with the sign-in.
            ManagedError::SignInRequired => ManagedError::NearAiSignInRequired,
            other => other,
        }),
        _ => Err(ManagedError::NearAiSignInRequired),
    };
    let result = with_service(shared, |service| match started {
        Ok(route) => {
            context.route_port = Some(route.port());
            context.route_model = Some(route.default_model().to_owned());
            context.route_models = Some(route.models().to_vec());
            // The row shows the model the session starts on. Only the row
            // misses it if this cannot be saved; the launch goes ahead.
            let _ = service
                .sessions
                .record_model(p.session_id, route.default_model());
            context.session.model = Some(route.default_model().to_owned());
            service.routes.insert(p.session_id, route);
            Ok(json!(context))
        }
        Err(error) => {
            // Nothing will run: say so now, not after a heartbeat gap.
            let _ = service.sessions.report(LifecycleReport {
                model: None,
                collection: None,
                session_id: p.session_id,
                proof: context.proof.clone(),
                state: SessionState::Failed,
                process_id: None,
                process_identity: None,
                exit_code: None,
            });
            Err(error)
        }
    });
    respond(request, result)
}

/// Once the sign-in is gone, shut down every proxy it was lent to and mark
/// their sessions ended. They stop reaching NEAR AI; nothing falls back to
/// another key.
pub async fn close_routes_without_sign_in(shared: &DaemonShared) {
    if sign_in_present(shared) {
        return;
    }
    let ended = with_service(shared, |service| {
        let routes = service.take_routes(true);
        service.sessions.end_sign_in_sessions()?;
        Ok(routes)
    })
    .unwrap_or_default();
    for route in ended {
        route.shutdown().await;
    }
}

/// Stop the proxy of a sign-in session whose launcher has gone quiet: one
/// unheard for longer than the lease (`sessions::LEASE`) is failed, and its
/// proxy shut down. The daemon's loop calls this on its own tick, so a
/// launcher that dies is noticed with no shell open.
pub async fn sweep(shared: &DaemonShared, now: chrono::DateTime<chrono::Utc>) {
    let ended = with_service(shared, |service| {
        if service.routes.is_empty() {
            return Ok(vec![]);
        }
        service.sessions.sweep(now)?;
        Ok(service.take_routes(false))
    })
    .unwrap_or_default();
    for route in ended {
        route.shutdown().await;
    }
}

/// The check before a ticket is issued, against the executable this
/// process can see. The tool runs where the helper runs: a CLI launch from
/// a terminal whose PATH holds a tool the daemon cannot see is not refused
/// here, because the helper resolves and version-checks it there before
/// starting it (`supervisor::redeem_and_run`). One the daemon does find is
/// checked now, so an unsupported version is refused before a ticket.
fn verify_visible_version(
    profiles: &NativeProfiles,
    account: &AccountView,
) -> Result<(), ManagedError> {
    match find_native(account.tool) {
        Ok(native) => profiles.verify_version(account, &native),
        Err(ManagedError::ToolUnavailable) => Ok(()),
        Err(other) => Err(other),
    }
}

fn label(error: ManagedError) -> &'static str {
    match error {
        ManagedError::Conflict => "managed-conflict",
        ManagedError::CollectionAccountBusy => "managed-collection-account-busy",
        ManagedError::NotFound => "managed-not-found",
        ManagedError::AmbiguousAccount => "managed-ambiguous-account",
        ManagedError::StorageUnavailable => "managed-storage-unavailable",
        ManagedError::ToolUnavailable => "managed-tool-unavailable",
        ManagedError::UnsupportedVersion => "managed-unsupported-version",
        ManagedError::SystemConfiguration => "managed-system-configuration",
        ManagedError::SignInRequired => "managed-sign-in-required",
        ManagedError::NearAiSignInRequired => "managed-near-ai-sign-in-required",
        ManagedError::DaemonUnavailable => "managed-daemon-unavailable",
        ManagedError::LaunchUnknown => "managed-launch-unknown",
        ManagedError::InvalidRequest => "managed-invalid-request",
        ManagedError::InvalidCliOptions => "managed-cli-options-invalid",
        ManagedError::TerminalNotFound => "managed-terminal-not-found",
        ManagedError::ModelUnavailable => "managed-model-unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::{sessions::PreparedLaunch, terminal::TerminalHandle};

    fn service() -> (tempfile::TempDir, ConfigStore, ManagedService) {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut service = ManagedService::open(&store).unwrap();
        service.installed_terminals = || TerminalApp::offered().to_vec();
        (home, store, service)
    }

    fn call(
        service: &mut ManagedService,
        method: &str,
        params: Value,
        sign_in: bool,
    ) -> Result<Value, ManagedError> {
        call_with(service, method, params, sign_in, TerminalSetting::default())
    }

    fn call_with(
        service: &mut ManagedService,
        method: &str,
        params: Value,
        sign_in: bool,
        choice: TerminalSetting,
    ) -> Result<Value, ManagedError> {
        service.handle(
            &Request {
                id: 1,
                method: method.into(),
                params,
            },
            sign_in,
            choice,
        )
    }

    #[test]
    fn tool_setup_needs_no_sign_in_account_profile_or_session() {
        let (_dir, _store, mut service) = service();
        let before = service.sessions.revision();
        service.setup_terminal = |_, _, command| {
            assert!(command.contains("https://chatgpt.com/codex/install.sh"));
            assert!(!command.contains("--config-dir"));
            assert!(!command.contains("--ticket"));
            Ok(())
        };
        let result = call(
            &mut service,
            "managed_tool_setup",
            json!({"tool":"codex","action":"install"}),
            false,
        )
        .unwrap();
        assert_eq!(result, json!({"opened":true}));
        assert_eq!(service.sessions.revision(), before);
        assert!(service.sessions.views().unwrap().is_empty());
        assert!(service.accounts.list().is_empty());
    }

    #[test]
    fn tool_setup_refuses_arbitrary_commands_and_does_not_claim_install_completion() {
        let (_dir, _store, mut service) = service();
        service.setup_terminal = |_, _, _| panic!("invalid input reached the terminal");
        for params in [
            json!({"tool":"codex;echo bad","action":"install"}),
            json!({"tool":"codex","action":"install;echo bad"}),
            json!({"tool":"codex","action":"install","command":"echo bad"}),
            json!({"tool":"codex","action":"install","cwd":"/tmp"}),
        ] {
            assert!(call(&mut service, "managed_tool_setup", params, false).is_err());
        }
        service.setup_terminal = |_, _, _| Err(ManagedError::TerminalNotFound);
        assert_eq!(
            call(
                &mut service,
                "managed_tool_setup",
                json!({"tool":"codex","action":"install"}),
                false
            ),
            Err(ManagedError::TerminalNotFound)
        );
    }

    fn launch(tool: ToolId, cwd: &std::path::Path) -> Value {
        json!(LaunchRequest {
            cli_options: None,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool,
            connection: ConnectionKind::NearAiSignIn,
            account_id: signin::account_id(tool).unwrap(),
            cwd: cwd.to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
            model: None,
            collect_traces: false,
        })
    }

    fn prepare(
        service: &mut ManagedService,
        tool: ToolId,
        cwd: &std::path::Path,
    ) -> PreparedLaunch {
        serde_json::from_value(
            call(service, "managed_launch_prepare", launch(tool, cwd), true).unwrap(),
        )
        .unwrap()
    }

    /// A launch that names no folder runs in the person's home directory,
    /// resolved by the daemon, and the session records it like a picked one.
    #[test]
    fn a_launch_without_a_folder_runs_in_the_home_directory() {
        let (_home, _store, mut service) = service();
        let mut params = launch(ToolId::Claude, std::path::Path::new("/unused"));
        params.as_object_mut().unwrap().remove("cwd");
        let prepared: PreparedLaunch = serde_json::from_value(
            call(&mut service, "managed_launch_prepare", params, true).unwrap(),
        )
        .unwrap();
        let sessions = service.sessions.views().unwrap();
        let view = sessions
            .iter()
            .find(|v| v.id == prepared.session_id)
            .unwrap();
        let home = std::fs::canonicalize(dirs::home_dir().unwrap()).unwrap();
        assert_eq!(view.cwd, home);
    }

    /// A folder that is named is still checked: a missing one is refused,
    /// never swapped for home.
    #[test]
    fn a_missing_named_folder_is_refused_not_replaced_by_home() {
        let (home, _store, mut service) = service();
        let gone = home.path().join("missing");
        assert!(
            call(
                &mut service,
                "managed_launch_prepare",
                launch(ToolId::Claude, &gone),
                true
            )
            .is_err()
        );
    }

    /// #1408: the snapshot says whether a coding session has started on
    /// this computer, and the first one started flips it.
    #[test]
    fn the_snapshot_says_once_a_session_has_started() {
        let (home, _store, mut service) = service();
        let before = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        assert_eq!(before["has_launched"], false);
        let account = service
            .accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_launch_prepare",
                json!(LaunchRequest {
                    cli_options: None,
                    purpose: LaunchPurpose::Coding,
                    request_id: Uuid::new_v4(),
                    tool: ToolId::Claude,
                    connection: ConnectionKind::Subscription,
                    account_id: account.id,
                    cwd: home.path().to_path_buf(),
                    expected_generation: Generation(0),
                    save_default: false,
                    model: None,
                    collect_traces: false,
                }),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let prepared_only = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        assert_eq!(prepared_only["has_launched"], false);
        call(
            &mut service,
            "managed_launch_redeem",
            json!({"session_id":prepared.session_id,"ticket":prepared.ticket.unwrap()}),
            false,
        )
        .unwrap();
        let after = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        assert_eq!(after["has_launched"], true);
    }

    #[test]
    fn the_snapshot_derives_sign_in_entries_and_defaults_from_the_sign_in() {
        let (_home, _store, mut service) = service();
        let without = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        let tools = without["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[2]["tool"], "opencode");
        assert_eq!(without["generations"].as_object().unwrap().len(), 3);
        assert_eq!(without["capabilities"]["near_ai_signin"], false);
        assert_eq!(without["sign_in_accounts"], json!([]));
        assert_eq!(without["defaults"], json!([]));

        let with = call(&mut service, "managed_snapshot", json!({}), true).unwrap();
        assert_eq!(with["capabilities"]["near_ai_signin"], true);
        let entries = with["sign_in_accounts"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        for (entry, tool) in entries.iter().zip(["claude", "codex", "opencode"]) {
            assert_eq!(entry["tool"], tool);
            assert_eq!(entry["connection"], "near_ai_signin");
            assert_eq!(entry["auth_state"], "ready");
        }
        let defaults = with["defaults"].as_array().unwrap();
        assert_eq!(defaults.len(), 3);
        assert!(defaults.iter().all(|d| d["connection"] == "near_ai_signin"));
        // Derived, not saved.
        assert_eq!(with["accounts"], json!([]));
        assert!(service.accounts.list().is_empty());
    }

    #[test]
    fn a_saved_default_wins_over_the_sign_in() {
        let (_home, _store, mut service) = service();
        let saved = service
            .accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let selection = Selection {
            tool: ToolId::Claude,
            connection: ConnectionKind::Subscription,
            account_id: saved.id,
            generation: Generation(0),
        };
        service.accounts.select(selection, Generation(0)).unwrap();
        let snapshot = call(&mut service, "managed_snapshot", json!({}), true).unwrap();
        let defaults = snapshot["defaults"].as_array().unwrap();
        assert_eq!(defaults.len(), 3);
        let claude = defaults.iter().find(|d| d["tool"] == "claude").unwrap();
        assert_eq!(claude["account_id"], json!(saved.id));
        let codex = defaults.iter().find(|d| d["tool"] == "codex").unwrap();
        assert_eq!(
            codex["account_id"],
            json!(signin::account_id(ToolId::Codex))
        );
        let opencode = defaults.iter().find(|d| d["tool"] == "opencode").unwrap();
        assert_eq!(
            opencode["account_id"],
            json!(signin::account_id(ToolId::Opencode))
        );
        // Signed out, the saved default stands alone.
        let snapshot = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        assert_eq!(snapshot["defaults"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn opencode_saved_default_survives_sign_in_snapshot() {
        let (_home, _store, mut service) = service();
        let account = service
            .accounts
            .add(ToolId::Opencode, ConnectionKind::ApiKey, "OpenAI")
            .unwrap();
        service
            .accounts
            .select(
                Selection {
                    tool: ToolId::Opencode,
                    connection: ConnectionKind::ApiKey,
                    account_id: account.id,
                    generation: Generation(0),
                },
                Generation(0),
            )
            .unwrap();
        let snapshot = call(&mut service, "managed_snapshot", json!({}), true).unwrap();
        assert_eq!(snapshot["sign_in_accounts"].as_array().unwrap().len(), 3);
        let defaults = snapshot["defaults"].as_array().unwrap();
        assert_eq!(defaults.len(), 3);
        let opencode = defaults.iter().find(|d| d["tool"] == "opencode").unwrap();
        assert_eq!(opencode["account_id"], json!(account.id));
        assert_eq!(opencode["connection"], "api_key");
        assert_eq!(snapshot["generations"]["opencode"], 1);
    }

    #[test]
    fn a_sign_in_entry_can_be_neither_saved_nor_chosen_as_a_default() {
        let (home, store, mut service) = service();
        assert_eq!(
            call(
                &mut service,
                "managed_account_add",
                json!({"tool":"claude","connection":"near_ai_signin","label":"NEAR AI"}),
                true,
            ),
            Err(ManagedError::InvalidRequest)
        );
        let selection = json!({"selection":{"tool":"claude","connection":"near_ai_signin","account_id":signin::account_id(ToolId::Claude),"generation":0},"expected_generation":0});
        assert_eq!(
            call(&mut service, "managed_select", selection, true),
            Err(ManagedError::Conflict)
        );
        for method in [
            "managed_account_remove",
            "managed_account_verify",
            "managed_account_reconnect",
            "managed_account_rename",
        ] {
            let mut params = json!({"account_id":signin::account_id(ToolId::Codex)});
            if method == "managed_account_rename" {
                params["label"] = json!("Mine");
            }
            assert!(
                call(&mut service, method, params, true).is_err(),
                "{method}"
            );
        }
        // A launch writes a session, never an account.
        let prepared = prepare(&mut service, ToolId::Claude, home.path());
        assert!(prepared.ticket.is_some());
        assert!(service.accounts.list().is_empty());
        assert!(AccountStore::open(&store).unwrap().list().is_empty());
        assert!(
            !home.path().join("managed-accounts.json").exists(),
            "nothing is written to the accounts store"
        );
    }

    #[test]
    fn a_sign_in_launch_without_a_sign_in_is_refused_and_fails_closed() {
        let (home, _store, mut service) = service();
        assert_eq!(
            call(
                &mut service,
                "managed_launch_prepare",
                launch(ToolId::Codex, home.path()),
                false,
            ),
            Err(ManagedError::NearAiSignInRequired)
        );
        // The entry id must match its tool.
        let mut crossed = launch(ToolId::Codex, home.path());
        crossed["account_id"] = json!(signin::account_id(ToolId::Claude));
        assert_eq!(
            call(&mut service, "managed_launch_prepare", crossed, true),
            Err(ManagedError::InvalidRequest)
        );

        let prepared = prepare(&mut service, ToolId::Codex, home.path());
        let ticket = prepared.ticket.clone().unwrap();
        let redeem = json!({"session_id":prepared.session_id,"ticket":ticket});
        // The synchronous path cannot host the proxy, so it refuses without
        // spending the ticket.
        assert_eq!(
            call(&mut service, "managed_launch_redeem", redeem, true),
            Err(ManagedError::LaunchUnknown)
        );
        assert!(
            service
                .sessions
                .validate_ticket(prepared.session_id, &ticket)
                .is_ok()
        );
        // Signed out between prepare and redeem: refused, and the session
        // fails now rather than when its ticket expires.
        assert_eq!(
            service.redeem(prepared.session_id, &ticket, false).err(),
            Some(ManagedError::NearAiSignInRequired)
        );
        assert_eq!(
            service.sessions.describe(prepared.session_id),
            Some((ConnectionKind::NearAiSignIn, SessionState::Failed))
        );
    }

    /// OpenCode accepts either an explicit model or the sign-in proxy default.
    #[test]
    fn opencode_starts_on_the_sign_in_with_a_chosen_or_default_model() {
        let (home, _store, mut service) = service();
        let mut named = launch(ToolId::Opencode, home.path());
        named["model"] = json!("gpt-5");
        let chosen: PreparedLaunch = serde_json::from_value(
            call(&mut service, "managed_launch_prepare", named, true).unwrap(),
        )
        .unwrap();
        let (context, host) = service
            .redeem(chosen.session_id, chosen.ticket.as_deref().unwrap(), true)
            .unwrap();
        assert!(host);
        assert_eq!(context.session.model.as_deref(), Some("gpt-5"));
        let prepared = prepare(&mut service, ToolId::Opencode, home.path());
        let (context, host) = service
            .redeem(
                prepared.session_id,
                prepared.ticket.as_deref().unwrap(),
                true,
            )
            .unwrap();
        assert!(host);
        assert_eq!(context.session.tool, ToolId::Opencode);
        assert_eq!(context.session.model, None);
    }

    #[test]
    fn a_signed_in_redeem_asks_its_caller_to_host_the_proxy() {
        let (home, _store, mut service) = service();
        let prepared = prepare(&mut service, ToolId::Claude, home.path());
        let (context, host) = service
            .redeem(
                prepared.session_id,
                prepared.ticket.as_deref().unwrap(),
                true,
            )
            .unwrap();
        assert!(host);
        assert_eq!(context.route_port, None);
        assert_eq!(context.route_model, None);
        assert_eq!(context.route_models, None);
        // Absent, not null, until a route is started.
        let wire = serde_json::to_value(&context).unwrap();
        for field in ["route_port", "route_model", "route_models"] {
            assert!(wire.get(field).is_none(), "{field}");
        }
        assert_eq!(context.session.connection, ConnectionKind::NearAiSignIn);
        assert_eq!(context.session.account_label, "NEAR AI");
        assert!(service.take_routes(true).is_empty());
    }

    /// The snapshot names the chosen app and offers the installed ones, by
    /// id and name; an unknown stored choice names none.
    #[test]
    fn the_snapshot_carries_the_terminal_choice_and_the_installed_apps() {
        let (_home, _store, mut service) = service();
        // This platform's last app, so the choice is not the default.
        let chosen = *TerminalApp::offered().last().expect("an offered app");
        let snapshot = call_with(
            &mut service,
            "managed_snapshot",
            json!({}),
            false,
            TerminalSetting::from(chosen),
        )
        .unwrap();
        let capabilities = &snapshot["capabilities"];
        assert_eq!(capabilities["terminal_current"], chosen.id());
        let offered: Vec<Value> = TerminalApp::offered()
            .iter()
            .map(|app| json!({"id":app.id(),"name":app.name()}))
            .collect();
        assert_eq!(capabilities["terminal_choices"], json!(offered));
        // Named only where a helper ships beside the daemon; where one
        // does, the name follows the choice.
        if terminal::helper_path().is_ok() && terminal::launch_available() {
            assert_eq!(capabilities["terminal_destination"], chosen.name());
        }

        let unknown = call_with(
            &mut service,
            "managed_snapshot",
            json!({}),
            false,
            TerminalSetting::Unrecognized,
        )
        .unwrap();
        assert_eq!(unknown["capabilities"]["terminal_current"], Value::Null);
        assert_eq!(unknown["capabilities"]["terminal_destination"], Value::Null);
    }

    /// Off macOS, no offered terminal found means no Launch, rather than a
    /// refusal pointing at a picker that is not drawn.
    #[test]
    fn no_terminal_found_turns_launch_off_except_on_macos() {
        let (_home, _store, mut service) = service();
        service.installed_terminals = Vec::new;
        let snapshot = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        let capabilities = &snapshot["capabilities"];
        assert_eq!(capabilities["terminal_choices"], json!([]));
        let expected = cfg!(target_os = "macos") && terminal::launch_available();
        assert_eq!(capabilities["terminal_launch"], expected);
    }

    #[test]
    fn the_terminal_refusal_has_its_own_label() {
        assert_eq!(
            label(ManagedError::TerminalNotFound),
            "managed-terminal-not-found"
        );
    }

    static FOCUSED: std::sync::Mutex<Vec<TerminalHandle>> = std::sync::Mutex::new(vec![]);

    /// Records the window it was asked to raise. Tests never drive a real
    /// terminal.
    fn record_focus(handle: &TerminalHandle) -> Result<(), ManagedError> {
        FOCUSED.lock().unwrap().push(handle.clone());
        Ok(())
    }

    /// A saved-account session, redeemed and reported running, as the
    /// launcher leaves it. Returns its id and the launcher's proof.
    fn running(service: &mut ManagedService, cwd: &std::path::Path) -> (Uuid, String) {
        let account = match service.accounts.list().into_iter().next() {
            Some(account) => account,
            None => service
                .accounts
                .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
                .unwrap(),
        };
        let mut request: LaunchRequest =
            serde_json::from_value(launch(ToolId::Claude, cwd)).unwrap();
        request.connection = ConnectionKind::Subscription;
        request.account_id = account.id;
        let prepared: PreparedLaunch = serde_json::from_value(
            call(service, "managed_launch_prepare", json!(request), false).unwrap(),
        )
        .unwrap();
        let context = service
            .sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        report(
            service,
            prepared.session_id,
            &context.proof,
            "running",
            None,
        )
        .unwrap();
        (prepared.session_id, context.proof)
    }

    fn report(
        service: &mut ManagedService,
        id: Uuid,
        proof: &str,
        state: &str,
        exit_code: Option<i32>,
    ) -> Result<Value, ManagedError> {
        call(
            service,
            "managed_session_report",
            json!({"session_id":id,"proof":proof,"state":state,"process_id":42,"process_identity":"42:1","exit_code":exit_code}),
            false,
        )
    }

    fn row(service: &mut ManagedService, id: Uuid, sign_in: bool) -> Value {
        let snapshot = call(service, "managed_snapshot", json!({}), sign_in).unwrap();
        snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == json!(id))
            .unwrap()
            .clone()
    }

    #[test]
    fn a_session_started_elsewhere_can_be_neither_shown_nor_stopped() {
        let (home, _store, mut service) = service();
        service.focus = record_focus;
        let (id, proof) = running(&mut service, home.path());
        let row = row(&mut service, id, false);
        assert_eq!(row["can_focus"], false);
        assert_eq!(row["can_stop"], false);
        for method in ["managed_session_focus", "managed_session_stop"] {
            assert_eq!(
                call(&mut service, method, json!({"session_id":id}), false),
                Err(ManagedError::Conflict),
                "{method}"
            );
        }
        assert_eq!(
            call(
                &mut service,
                "managed_session_stop",
                json!({"session_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::NotFound)
        );
        // The launcher is never told to stop a session nobody could stop.
        assert_eq!(
            report(&mut service, id, &proof, "running", None).unwrap()["stop"],
            false
        );
    }

    #[test]
    fn a_session_the_daemon_opened_in_terminal_can_be_shown_and_stopped() {
        let (home, _store, mut service) = service();
        service.focus = record_focus;
        let account = service
            .accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
            .unwrap();
        let mut request: LaunchRequest =
            serde_json::from_value(launch(ToolId::Claude, home.path())).unwrap();
        request.connection = ConnectionKind::Subscription;
        request.account_id = account.id;
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_launch_prepare",
                json!(request),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let handle = TerminalHandle::AppleTerminal {
            tty: "/dev/ttys042".into(),
        };
        service
            .sessions
            .opened_in_terminal(prepared.session_id, Some(handle.clone()))
            .unwrap();
        // Not running yet: nothing to show or stop.
        let starting = row(&mut service, prepared.session_id, false);
        assert_eq!(starting["can_focus"], false);
        assert_eq!(starting["can_stop"], false);
        let context = service
            .sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        let id = prepared.session_id;
        assert_eq!(
            report(&mut service, id, &context.proof, "running", None).unwrap()["stop"],
            false
        );
        let running = row(&mut service, id, false);
        assert_eq!(running["can_focus"], true);
        assert_eq!(running["can_stop"], true);

        call(
            &mut service,
            "managed_session_focus",
            json!({"session_id":id}),
            false,
        )
        .unwrap();
        assert!(FOCUSED.lock().unwrap().contains(&handle));

        call(
            &mut service,
            "managed_session_stop",
            json!({"session_id":id}),
            false,
        )
        .unwrap();
        // Asked once: the row offers no second Stop, still shows its window,
        // and the launcher hears it on its next report.
        let stopping = row(&mut service, id, false);
        assert_eq!(stopping["can_stop"], false);
        assert_eq!(stopping["can_focus"], true);
        assert_eq!(stopping["state"], "running");
        call(
            &mut service,
            "managed_session_stop",
            json!({"session_id":id}),
            false,
        )
        .unwrap();
        assert_eq!(
            report(&mut service, id, &context.proof, "running", None).unwrap()["stop"],
            true
        );

        // The exit is recorded as any other.
        report(&mut service, id, &context.proof, "exited", Some(143)).unwrap();
        let ended = row(&mut service, id, false);
        assert_eq!(ended["state"], "exited");
        assert_eq!(ended["exit_code"], 143);
        assert_eq!(ended["can_focus"], false);
        assert_eq!(ended["can_stop"], false);
        for method in ["managed_session_focus", "managed_session_stop"] {
            assert_eq!(
                call(&mut service, method, json!({"session_id":id}), false),
                Err(ManagedError::Conflict),
                "{method}"
            );
        }
    }

    #[test]
    fn a_session_opened_where_no_window_can_be_named_can_be_stopped_but_not_shown() {
        let (home, _store, mut service) = service();
        service.focus = record_focus;
        let account = service
            .accounts
            .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
            .unwrap();
        let mut request: LaunchRequest =
            serde_json::from_value(launch(ToolId::Codex, home.path())).unwrap();
        request.connection = ConnectionKind::Subscription;
        request.account_id = account.id;
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_launch_prepare",
                json!(request),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        service
            .sessions
            .opened_in_terminal(prepared.session_id, None)
            .unwrap();
        let context = service
            .sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        report(
            &mut service,
            prepared.session_id,
            &context.proof,
            "running",
            None,
        )
        .unwrap();
        let row = row(&mut service, prepared.session_id, false);
        assert_eq!(row["can_focus"], false);
        assert_eq!(row["can_stop"], true);
        assert_eq!(
            call(
                &mut service,
                "managed_session_focus",
                json!({"session_id":prepared.session_id}),
                false
            ),
            Err(ManagedError::Conflict)
        );
    }

    #[test]
    fn start_again_replays_an_ended_launch_in_the_same_folder() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let (id, proof) = running(&mut service, &folder);
        let again = json!({"session_id":id,"request_id":Uuid::new_v4()});
        // Running: nothing to start again.
        assert_eq!(row(&mut service, id, false)["can_start_again"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_start_again",
                again.clone(),
                false
            ),
            Err(ManagedError::Conflict)
        );
        report(&mut service, id, &proof, "exited", Some(0)).unwrap();
        assert_eq!(row(&mut service, id, false)["can_start_again"], true);

        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_session_start_again",
                again.clone(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(prepared.session_id, id);
        assert!(prepared.ticket.is_some());
        let fresh = row(&mut service, prepared.session_id, false);
        let old = row(&mut service, id, false);
        for field in ["tool", "connection", "account_id", "cwd", "purpose"] {
            assert_eq!(fresh[field], old[field], "{field}");
        }
        assert_eq!(fresh["state"], "starting");
        // The same request twice is one session.
        let repeated: PreparedLaunch = serde_json::from_value(
            call(&mut service, "managed_session_start_again", again, false).unwrap(),
        )
        .unwrap();
        assert_eq!(repeated.session_id, prepared.session_id);
        assert!(repeated.ticket.is_none());
    }

    #[test]
    fn start_again_retried_after_a_selection_change_is_still_one_session() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let (id, proof) = running(&mut service, &folder);
        report(&mut service, id, &proof, "exited", Some(0)).unwrap();
        let again = json!({"session_id":id,"request_id":Uuid::new_v4()});
        let first: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_session_start_again",
                again.clone(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        // The selection moves between the request and its retry.
        let account = service.accounts.list().into_iter().next().unwrap();
        let generation = service.accounts.generation(ToolId::Claude);
        service
            .accounts
            .select(
                Selection {
                    tool: ToolId::Claude,
                    connection: ConnectionKind::Subscription,
                    account_id: account.id,
                    generation,
                },
                generation,
            )
            .unwrap();
        let retried: PreparedLaunch = serde_json::from_value(
            call(&mut service, "managed_session_start_again", again, false).unwrap(),
        )
        .unwrap();
        assert_eq!(retried.session_id, first.session_id);
        assert!(retried.ticket.is_none());
    }

    #[test]
    fn a_second_terminal_launch_for_one_session_is_refused() {
        let (home, _store, mut service) = service();
        let account = service
            .accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
            .unwrap();
        let mut request: LaunchRequest =
            serde_json::from_value(launch(ToolId::Claude, home.path())).unwrap();
        request.connection = ConnectionKind::Subscription;
        request.account_id = account.id;
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_launch_prepare",
                json!(request),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let ticket = prepared.ticket.unwrap();
        service
            .sessions
            .validate_ticket(prepared.session_id, &ticket)
            .unwrap();
        service
            .sessions
            .opened_in_terminal(prepared.session_id, None)
            .unwrap();
        assert_eq!(
            service
                .sessions
                .validate_ticket(prepared.session_id, &ticket),
            Err(ManagedError::Conflict)
        );
    }

    #[test]
    fn start_again_is_withheld_when_the_folder_or_account_is_gone() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let (id, proof) = running(&mut service, &folder);
        report(&mut service, id, &proof, "exited", Some(0)).unwrap();
        std::fs::remove_dir(&folder).unwrap();
        assert_eq!(row(&mut service, id, false)["can_start_again"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_start_again",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::InvalidRequest)
        );

        std::fs::create_dir(&folder).unwrap();
        let account = service.accounts.list()[0].id;
        call(
            &mut service,
            "managed_account_remove",
            json!({"account_id":account}),
            false,
        )
        .ok();
        // Removing a subscription account needs the native tool; drop it
        // from the store directly when that is absent here.
        if !service.accounts.list().is_empty() {
            service.accounts.remove(account).unwrap();
        }
        assert_eq!(row(&mut service, id, false)["can_start_again"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_start_again",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::NotFound)
        );
    }

    #[test]
    fn a_sign_in_session_starts_again_only_while_signed_in() {
        let (home, _store, mut service) = service();
        let prepared = prepare(&mut service, ToolId::Claude, home.path());
        let (context, _) = service
            .redeem(
                prepared.session_id,
                prepared.ticket.as_deref().unwrap(),
                true,
            )
            .unwrap();
        let id = prepared.session_id;
        report(&mut service, id, &context.proof, "running", None).unwrap();
        report(&mut service, id, &context.proof, "exited", Some(0)).unwrap();
        assert_eq!(row(&mut service, id, true)["can_start_again"], true);
        assert_eq!(row(&mut service, id, false)["can_start_again"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_start_again",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::NearAiSignInRequired)
        );
        assert!(
            call(
                &mut service,
                "managed_session_start_again",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                true
            )
            .unwrap()["ticket"]
                .is_string()
        );
    }

    #[test]
    fn a_sign_in_row_never_starts_again() {
        let (_home, _store, mut service) = service();
        let account = service
            .accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
            .unwrap();
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_account_reconnect",
                json!({"account_id":account.id}),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let context = service
            .sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        let id = prepared.session_id;
        report(&mut service, id, &context.proof, "exited", Some(0)).unwrap();
        assert_eq!(row(&mut service, id, false)["can_start_again"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_start_again",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::InvalidRequest)
        );
    }

    #[test]
    fn the_near_ai_sign_in_refusal_has_its_own_label() {
        assert_eq!(
            label(ManagedError::NearAiSignInRequired),
            "managed-near-ai-sign-in-required"
        );
        assert_ne!(
            label(ManagedError::NearAiSignInRequired),
            label(ManagedError::SignInRequired)
        );
    }
}
