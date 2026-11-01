//! One local source of truth for CLI and desktop managed sessions.

use super::ipc::{DaemonShared, Request, Response};
use crate::daemon::nearai_credential::balance::{self, BalanceReport};
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
            CollectionState, LaunchContext, LaunchPurpose, LaunchRequest, LifecycleReport,
            SessionRegistry, SessionState, SessionView,
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

    /// Prepare the session a `managed_launch_prepare`,
    /// `managed_session_start_again` or `managed_session_resume` request
    /// resolves to.
    fn prepare_launch(&mut self, request: &Request, sign_in: bool) -> Result<Value, ManagedError> {
        match self.resolve_launch(request, sign_in, true)? {
            Resolved::Replayed(existing) => Ok(json!(existing)),
            Resolved::Launch(launch, account) => {
                Ok(json!(self.sessions.prepare(launch, &account)?))
            }
        }
    }

    /// The launch a prepare or start-again request resolves to, after every
    /// check made before [`crate::managed::sessions::SessionRegistry::prepare`].
    /// Without `verify`, a dry run: the tool's version is not run. Any other
    /// method is `InvalidRequest`.
    fn resolve_launch(
        &self,
        request: &Request,
        sign_in: bool,
        verify: bool,
    ) -> Result<Resolved, ManagedError> {
        match request.method.as_str() {
            "managed_launch_prepare" => {
                // No folder named: the person's home directory.
                let params = crate::managed::folder::with_default_cwd(
                    request.params.clone(),
                    dirs::home_dir(),
                )?;
                let p: LaunchRequest =
                    serde_json::from_value(params).map_err(|_| ManagedError::InvalidRequest)?;
                // Only `managed_session_resume` continues a conversation,
                // from a row it checks.
                if p.resume_of.is_some() {
                    return Err(ManagedError::InvalidRequest);
                }
                let account = self.launch_account(&p, sign_in)?;
                let generation = self.accounts.generation(p.tool);
                if p.expected_generation != generation {
                    return Err(ManagedError::Conflict);
                }
                // Default changes are separate until the cross-file transaction is available.
                if p.save_default {
                    return Err(ManagedError::InvalidRequest);
                }
                // The dry run skips the tool's `--version`: it runs under
                // the managed lock, and the launch makes the check anyway
                // before anything commits. A version failure only changes
                // which refusal the person sees.
                if verify {
                    verify_visible_version(&self.profiles, &account)?;
                }
                Ok(Resolved::Launch(p, account))
            }
            "managed_session_start_again" | "managed_session_resume" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    session_id: Uuid,
                    request_id: Uuid,
                }
                let p: Params = parse(request)?;
                let mut again = if request.method == "managed_session_resume" {
                    self.sessions.resume_request(p.session_id, p.request_id)?
                } else {
                    self.sessions
                        .start_again_request(p.session_id, p.request_id)?
                };
                if let Some(existing) = self.sessions.replayed(&again) {
                    return Ok(Resolved::Replayed(existing));
                }
                if again.resume_of.is_some_and(|root| {
                    !self.knows_conversation(again.tool, again.account_id, root)
                }) {
                    return Err(ManagedError::Conflict);
                }
                let account = self.launch_account(&again, sign_in)?;
                if !again.cwd.is_absolute() || !again.cwd.is_dir() {
                    return Err(ManagedError::InvalidRequest);
                }
                again.expected_generation = self.accounts.generation(again.tool);
                Ok(Resolved::Launch(again, account))
            }
            _ => Err(ManagedError::InvalidRequest),
        }
    }

    /// Whether `request` would prepare a new session on the NEAR AI
    /// sign-in: it passes every check [`Self::prepare_launch`] makes and is
    /// not a replay. Commits nothing; the request is resolved and checked
    /// again when it is handled.
    fn prepares_on_sign_in(&self, request: &Request, sign_in: bool) -> bool {
        match self.resolve_launch(request, sign_in, false) {
            Ok(Resolved::Launch(launch, account)) => {
                launch.connection == ConnectionKind::NearAiSignIn
                    && self.sessions.would_prepare_new(launch, &account)
            }
            _ => false,
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
                        keep_saved_model: false,
                        resume_of: None,
                    },
                    sign_in,
                )
                .is_ok_and(|a| a.tool == view.tool && a.connection == view.connection)
    }

    /// Whether the conversation `root` started is there to continue in
    /// `account`'s profile. Claude Code's id is the root row's, but Claude
    /// writes `<root>.jsonl` only once the conversation has a turn, and a
    /// launch can fail before its tool ever starts. OpenCode keeps the
    /// session's own directory. Codex's id is known only from the root's
    /// binding, which also requires its transcript.
    fn knows_conversation(&self, tool: ToolId, account: AccountId, root: Uuid) -> bool {
        match tool {
            ToolId::Codex => {
                crate::managed::session_binding::read(self.sessions.store(), root).is_some()
            }
            ToolId::Claude => self
                .profiles
                .existing_profile_root(account)
                .is_some_and(|profile| claude_conversation_written(&profile, root)),
            ToolId::Opencode => {
                self.profiles
                    .existing_profile_root(account)
                    .is_some_and(|profile| {
                        std::fs::symlink_metadata(profile.join(root.to_string()))
                            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                    })
            }
        }
    }

    /// Whether an ended row's conversation can be continued in a new
    /// session: it can be started again, its conversation's root started
    /// and is not in use (`roots`, from `SessionRegistry::resume_roots`),
    /// and the tool's id for that conversation is known. `known` keeps
    /// that last answer for the snapshot, since every row of a chain asks
    /// it of the same conversation.
    fn can_resume(
        &self,
        view: &SessionView,
        roots: &std::collections::BTreeMap<Uuid, Uuid>,
        known: &mut std::collections::BTreeMap<(ToolId, Uuid, Uuid), bool>,
    ) -> bool {
        view.can_start_again
            && roots.get(&view.id).is_some_and(|root| {
                *known
                    .entry((view.tool, view.account_id.0, *root))
                    .or_insert_with(|| self.knows_conversation(view.tool, view.account_id, *root))
            })
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
    /// `terminal` is the terminal the settings name, and whether the person
    /// chose it.
    fn handle(
        &mut self,
        request: &Request,
        sign_in: bool,
        terminal: TerminalChoice,
    ) -> Result<Value, ManagedError> {
        let choice = terminal.setting;
        match request.method.as_str() {
            "managed_snapshot" => {
                let mut sessions = self.sessions.views()?;
                let roots = self.sessions.resume_roots();
                let mut known = std::collections::BTreeMap::new();
                for view in &mut sessions {
                    view.can_start_again = self.can_start_again(view, sign_in);
                    view.can_resume = self.can_resume(view, &roots, &mut known);
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
                    json!({"copy":crate::managed::copy::copy(),"tools":crate::managed::profiles::tools(),"cli_options":crate::managed::options::snapshot(&ConfigStore::open(self.config_dir.clone()).map_err(|_| ManagedError::StorageUnavailable)?)?,"revision":self.sessions.revision(),"has_launched":self.sessions.has_launched(),"accounts":self.accounts.list(),"sign_in_accounts":signin::entries(sign_in),"generations":{"claude":self.accounts.generation(ToolId::Claude),"codex":self.accounts.generation(ToolId::Codex),"opencode":self.accounts.generation(ToolId::Opencode)},"defaults":defaults,"sessions":sessions,"capabilities":{"tool_setup":terminal::tool_setup_available(),"managed_launch":true,"folder_settings":true,"keep_saved_model":true,"resume":true,"near_ai_signin":sign_in,"terminal_launch":terminal_launch,"terminal_destination":terminal::destination(choice),"terminal_current":choice.app().map(TerminalApp::id),"terminal_chosen":terminal.chosen,"terminal_choices":choices}}),
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
            "managed_launch_prepare" => self.prepare_launch(request, sign_in),
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
            "managed_session_start_again" | "managed_session_resume" => {
                self.prepare_launch(request, sign_in)
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
                    keep_saved_model: false,
                    resume_of: None,
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

/// The terminal the settings name, and whether the person chose it
/// (`managed_terminal_chosen`). A shell asks for a terminal in the launch
/// dialog until they have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalChoice {
    setting: TerminalSetting,
    chosen: bool,
}

/// The terminal choice, read per request like the sign-in. Unreadable
/// settings name no app, and count as chosen so nothing asks over them.
fn terminal_choice(shared: &DaemonShared) -> TerminalChoice {
    shared
        .settings
        .lock()
        .map(|s| TerminalChoice {
            setting: s.managed_terminal,
            chosen: s.managed_terminal_chosen,
        })
        .unwrap_or(TerminalChoice {
            setting: TerminalSetting::Unrecognized,
            chosen: true,
        })
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
    if prepares_a_session(&request.method) && value.get("ticket").is_some_and(Value::is_string) {
        let id: Uuid = serde_json::from_value(value["session_id"].clone())
            .map_err(|_| ManagedError::InvalidRequest)?;
        let (prefs, keep) = with_service(shared, |service| {
            Ok((
                service.sessions.folder_settings(id),
                service.sessions.keeps_saved_model(id),
            ))
        })?;
        if let Some(mut prefs) = prefs {
            // A launch that fell back to the default because NEAR AI no
            // longer lists the saved model remembers its other choices but
            // not the fallback: the folder keeps the model it had.
            // When the saved model cannot be read, nothing is written: a
            // default must never replace it (#1754 review).
            if keep {
                match saved_folder_model(shared, &prefs) {
                    Ok(model) => prefs.model = model,
                    Err(()) => {
                        with_service(shared, |service| service.sessions.rollback_prepared(id))?;
                        return Err(Refusal::Unavailable("managed-folder-settings-save-failed"));
                    }
                }
            }
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

/// What a prepare or start-again request resolves to.
enum Resolved {
    /// A new launch, checked and ready to prepare.
    Launch(LaunchRequest, AccountView),
    /// The session this request id already prepared.
    Replayed(crate::managed::sessions::PreparedLaunch),
}

/// The model the folder's saved settings name for `prefs`' tool: `None`
/// when they name none, `Err` when they could not be read (the folder is
/// gone, or the settings lock is poisoned).
fn saved_folder_model(
    shared: &DaemonShared,
    prefs: &crate::managed::folder::FolderSettings,
) -> Result<Option<String>, ()> {
    let cwd = crate::managed::folder::canonical(&prefs.cwd).map_err(|_| ())?;
    let settings = shared.settings.lock().map_err(|_| ())?;
    Ok(settings
        .managed_folder_settings
        .iter()
        .find(|entry| entry.tool == prefs.tool && entry.cwd == cwd)
        .and_then(|entry| entry.model.clone()))
}

fn refusal_label(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::Unavailable(value) => value,
        Refusal::Managed(error) => label(error),
    }
}

/// The stored sign-in key's id, naming which NEAR AI key a balance read
/// was for.
fn sign_in_key_id(shared: &DaemonShared) -> Option<String> {
    shared
        .settings
        .lock()
        .ok()?
        .near_ai_inference
        .as_ref()
        .map(|c| c.key_id.clone())
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
    handle_async_with(shared, request, |fresh| read_balance(shared, fresh)).await
}

/// [`handle_async`] with the sign-in's balance read through `read`, so a
/// test can answer it without NEAR AI.
pub(crate) async fn handle_async_with<R, F>(
    shared: &DaemonShared,
    request: &Request,
    read: R,
) -> Response
where
    R: Fn(bool) -> F,
    F: std::future::Future<Output = BalanceReport>,
{
    if request.method == "managed_launch_redeem" {
        return redeem_async(shared, request).await;
    }
    if request.method == "managed_model_catalogue" {
        return model_catalogue(shared, request).await;
    }
    if request.method == "managed_model_favorite_set" {
        return favorite_set(shared, request).await;
    }
    // A launch that would prepare a new session on the sign-in waits for
    // the balance before anything is committed: a refusal, a cancelled
    // request or a sign-out during the wait leaves no session behind, and
    // `handle` below resolves and checks the request again from scratch.
    //
    // The sign-in is read once, and the dry run and `handle` both use it.
    // The balance read is the sign-in's at the time of each read, so a
    // switch to another NEAR AI key during the wait, which would let one
    // account's balance pass a launch on the other, is refused as if
    // signed out, whatever the balance said.
    let key = sign_in_key_id(shared);
    let sign_in = key.is_some();
    if prepares_on_sign_in(shared, request, sign_in) {
        let funds = funds_for_first_calls(read).await;
        if sign_in_key_id(shared) != key {
            return respond(request, Err(ManagedError::NearAiSignInRequired.into()));
        }
        if let Err(error) = funds {
            return respond(request, Err(error.into()));
        }
    }
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

/// Whether `method` prepares a session: the requests
/// [`ManagedService::resolve_launch`] resolves.
fn prepares_a_session(method: &str) -> bool {
    matches!(
        method,
        "managed_launch_prepare" | "managed_session_start_again" | "managed_session_resume"
    )
}

/// [`ManagedService::prepares_on_sign_in`] for a prepare or start-again
/// request, with the service locked directly: a read that publishes
/// nothing.
fn prepares_on_sign_in(shared: &DaemonShared, request: &Request, sign_in: bool) -> bool {
    // Every other request skips the lock; `resolve_launch` refuses them too.
    if !prepares_a_session(&request.method) {
        return false;
    }
    let Ok(held) = shared.managed.lock() else {
        return false;
    };
    held.as_ref()
        .is_ok_and(|service| service.prepares_on_sign_in(request, sign_in))
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

/// The least a NEAR AI balance must hold for a launch on the sign-in, in
/// nano-dollars: $0.10. Enough for the first few calls of a coding session
/// (a Claude Code turn sends a system prompt and tool list of tens of
/// thousands of tokens), so a session does not open only to fail on its
/// first request.
const MIN_LAUNCH_BALANCE_NANOS: i64 = 100_000_000;

/// How long a launch waits for the sign-in's balance, across every read,
/// before going ahead without it.
const FUNDS_CHECK_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

/// The sign-in's balance. The read runs detached on the daemon's runtime,
/// so when [`funds_for_first_calls`] stops waiting for it, a refresh-token
/// rotation in flight is never cut short: it finishes and fills the cache
/// for the next look. A sender dropped without an answer is
/// [`BalanceReport::Unavailable`].
///
/// A `DaemonShared` that never adopted a runtime reads inline, and there
/// the deadline can cut a rotation short. Only unit tests reach that: the
/// daemon serving the socket adopts its runtime at start, and managed
/// launches reach it over IPC.
pub(crate) async fn read_balance(shared: &DaemonShared, fresh: bool) -> BalanceReport {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let detached = shared.spawn_detached(move |this| async move {
        let _ = sender.send(balance::read(&this, fresh).await);
    });
    if detached.is_none() {
        return balance::read(shared, fresh).await;
    }
    receiver.await.unwrap_or(BalanceReport::Unavailable)
}

/// Refuse a launch only on a balance known to be below
/// [`MIN_LAUNCH_BALANCE_NANOS`], and only after a fresh read has said so:
/// a cached figure may predate a payment. Every other state lets it
/// through: no session, an expired one or an unreachable service says
/// nothing about the money, and a null remaining means no limit was ever
/// set. NEAR AI still refuses calls it will not pay for; this only stops a
/// session that would fail at once.
///
/// Both reads share one [`FUNDS_CHECK_WITHIN`] deadline; a read still
/// pending when it passes counts as [`BalanceReport::Unavailable`].
async fn funds_for_first_calls<R, F>(read: R) -> Result<(), ManagedError>
where
    R: Fn(bool) -> F,
    F: std::future::Future<Output = BalanceReport>,
{
    let deadline = tokio::time::Instant::now() + FUNDS_CHECK_WITHIN;
    let within = |future: F| async move {
        tokio::time::timeout_at(deadline, future)
            .await
            .unwrap_or(BalanceReport::Unavailable)
    };
    if !too_low(&within(read(false)).await) || !too_low(&within(read(true)).await) {
        return Ok(());
    }
    Err(ManagedError::NearAiBalanceTooLow)
}

/// Whether `report` is a known balance below [`MIN_LAUNCH_BALANCE_NANOS`].
fn too_low(report: &BalanceReport) -> bool {
    matches!(
        report,
        BalanceReport::Known(reading)
            if reading
                .remaining_nanos
                .is_some_and(|remaining| remaining < MIN_LAUNCH_BALANCE_NANOS)
    )
}

/// The NEAR AI chat models a launch can name, and the one `tool` gets
/// when it names none (`managed::models`). The catalogue is public: no
/// key is read or sent. Answers `{"models":[{"id","name"?,"tier"?}],"default",
/// "favorites","unavailable_favorites"}`: `favorites` is the starred models
/// among `models`, in NEAR AI's order; `unavailable_favorites` is the
/// starred ids NEAR AI no longer lists for any tool. Neither prunes the
/// stored list, so a model NEAR AI offers again is a favorite again. When
/// NEAR AI cannot be read there is no answer, so nothing is marked.
/// `tier` is NEAR AI's privacy tier (`models::ModelTier`); `tier_order` is
/// the tiers a picker groups by, most private first (`ModelTier::ORDER`).
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
    let listed = fetch_chat_models().await;
    let offered = listed.clone().map(|models| models::offered(p.tool, models));
    let result = match (listed, offered) {
        (Some(listed), Some(models)) if !models.is_empty() => {
            let default = models::default_for(p.tool, &models).map(|m| m.id.clone());
            let starred = favorite_models(shared);
            let favorites: Vec<&str> = models
                .iter()
                .map(|m| m.id.as_str())
                .filter(|id| starred.iter().any(|s| s == id))
                .collect();
            let unavailable: Vec<&str> = starred
                .iter()
                .map(String::as_str)
                .filter(|id| !listed.iter().any(|m| m.id == *id))
                .collect();
            Ok(json!({
                "models":models,"default":default,"favorites":favorites,
                "unavailable_favorites":unavailable,
                "tier_order":models::ModelTier::ORDER,
            }))
        }
        _ => Err(ManagedError::ModelUnavailable.into()),
    };
    respond(request, result)
}

/// `managed_model_favorite_set {model, favorite}`: star or unstar one model
/// for the Launch sheet's picker. Starring reads NEAR AI's chat models and
/// refuses one it does not list, so only an offered model is ever added;
/// unstarring reads nothing and takes any stored string, so a malformed id
/// from an older or hand-edited file can still be removed. Writes through
/// `set_settings`, whose checks (id shape for a star, list bound) apply
/// too. Answers `{"favorites"}`, the stored list, and publishes
/// `status_changed` when that list moved, so every window re-reads its
/// settings. No lock is held across the read.
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
    if p.favorite && !crate::daemon::settings::valid_favorite_model_id(&p.model) {
        return respond(request, Err(ManagedError::InvalidRequest.into()));
    }
    if p.favorite {
        match fetch_chat_models().await {
            Some(models) if models.iter().any(|m| m.id == p.model) => {}
            _ => return respond(request, Err(ManagedError::ModelUnavailable.into())),
        }
    }
    let before = favorite_models(shared);
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
    let favorites = favorite_models(shared);
    if favorites != before {
        shared.publish(super::ipc::EVENT_STATUS_CHANGED, json!({}));
    }
    respond(request, Ok(json!({ "favorites": favorites })))
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

/// Collect the trace of a collecting session whose launcher went quiet
/// (terminal window closed, force quit, logout), once its tool has exited,
/// from the collection plan the launcher left (`managed::collection_plan`).
/// The daemon's loop runs this on its lease tick, whose first tick is at
/// startup, so rows a restart found unknown or failed are picked up too.
/// A row whose tool the daemon saw exit has ended, so its conversation can
/// be resumed (`SessionRegistry::tool_exited`).
///
/// Never while the launcher still reports, never while the tool's process
/// runs (its pid and start time), never for a row that does not collect,
/// and never when the launcher took the row's claim first. A plan is read
/// only for a quiet row still waiting for its result, and all file,
/// process and collection work runs on a blocking task, off the daemon's
/// loop and off the managed lock, which is taken only to read rows and
/// record results. Past `collection_plan::GIVE_UP_AFTER` a running tool's
/// collection is recorded as failed and its local session is left as it
/// is. Returns how many rows it collected.
pub async fn collect_abandoned(
    shared: &std::sync::Arc<DaemonShared>,
    now: chrono::DateTime<chrono::Utc>,
) -> usize {
    let shared = std::sync::Arc::clone(shared);
    tokio::task::spawn_blocking(move || collect_abandoned_now(&shared, now))
        .await
        .unwrap_or(0)
}

fn collect_abandoned_now(shared: &DaemonShared, now: chrono::DateTime<chrono::Utc>) -> usize {
    use crate::managed::collection_plan::{self, Next, ProcessIdentity};
    let store = &shared.store;
    let ids = collection_plan::list(store);
    if ids.is_empty() {
        return 0;
    }
    // Rows lapse lazily (a shell's snapshot, or a sweep while the daemon
    // hosts a route); lapse them here so a quiet launcher is seen.
    let Ok(rows) = with_service(shared, |service| {
        service.sessions.sweep(now)?;
        Ok(ids
            .iter()
            .map(|id| service.sessions.collection_status(*id))
            .collect::<Vec<_>>())
    }) else {
        return 0;
    };
    let record = |id, collection, tool_exited: bool| {
        let _ = with_service(shared, |service| {
            service.sessions.record_collection(id, collection)?;
            if tool_exited {
                service.sessions.tool_exited(id)?;
            }
            Ok(())
        });
    };
    let mut me = None;
    let mut collected = 0;
    for (id, row) in ids.into_iter().zip(rows) {
        let (next, plan) = match collection_plan::before_reading(row) {
            Some(decided) => (decided, None),
            None => {
                let claim = collection_plan::claimant(store, id)
                    .map(|holder| holder.is_some_and(|holder| holder.is_running()));
                match collection_plan::read(store, id) {
                    None => (collection_plan::unreadable(row, claim), None),
                    Some(plan) => {
                        let tool_running = plan.process.is_running();
                        let staged =
                            claim == Some(false) && collection_plan::staged(store, &plan);
                        let next =
                            collection_plan::next(&plan, row, tool_running, claim, staged, now);
                        (next, Some((plan, tool_running)))
                    }
                }
            }
        };
        // Whether the tool is known to be gone: only a plan names it.
        let tool_exited = plan.as_ref().is_some_and(|(_, running)| !running);
        match next {
            Next::Wait => {}
            Next::Discard => collection_plan::finish(store, id),
            Next::Fail => {
                record(id, CollectionState::Failed, tool_exited);
                collection_plan::finish(store, id);
            }
            Next::Collected => {
                record(id, CollectionState::Collected, tool_exited);
                collection_plan::finish(store, id);
            }
            Next::Collect => {
                let Some((plan, _)) = plan else {
                    continue;
                };
                let me = me.get_or_insert_with(ProcessIdentity::current);
                if collection_plan::claim(store, id, me) != Ok(true) {
                    continue;
                }
                let result = match collection_plan::collect(store, &plan) {
                    Ok(()) => CollectionState::Collected,
                    Err(_) => {
                        tracing::warn!(pass = "managed-collection", "collection failed");
                        CollectionState::Failed
                    }
                };
                record(id, result, true);
                collection_plan::finish(store, id);
                collected += 1;
            }
        }
    }
    collected
}

/// Whether Claude Code wrote the conversation `id` in the profile at
/// `profile`: a regular `projects/<project>/<id>.jsonl`, links refused.
/// Only names are read, since a snapshot asks this of every ended row;
/// collection verifies the transcript's own identity when it stages it.
fn claude_conversation_written(profile: &std::path::Path, id: Uuid) -> bool {
    let real = |path: &std::path::Path, dir: bool| {
        std::fs::symlink_metadata(path).is_ok_and(|m| {
            !m.file_type().is_symlink() && if dir { m.is_dir() } else { m.is_file() }
        })
    };
    let projects = profile.join("projects");
    if !real(&projects, true) {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return false;
    };
    let name = format!("{id}.jsonl");
    entries
        .flatten()
        .map(|entry| entry.path())
        .any(|project| real(&project, true) && real(&project.join(&name), false))
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
        ManagedError::NearAiBalanceTooLow => "managed-near-ai-balance-too-low",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::{sessions::PreparedLaunch, terminal::TerminalHandle};

    fn known(remaining_nanos: Option<i64>) -> BalanceReport {
        BalanceReport::Known(balance::BalanceReading {
            remaining_nanos,
            spend_limit_nanos: None,
            total_spent_nanos: 0,
            total_requests: 0,
            total_tokens: 0,
            observed_at: chrono::Utc::now(),
        })
    }

    #[test]
    fn only_a_known_balance_under_the_floor_is_too_low() {
        for low in [Some(0), Some(-5), Some(MIN_LAUNCH_BALANCE_NANOS - 1)] {
            assert!(too_low(&known(low)));
        }
        assert!(!too_low(&known(Some(MIN_LAUNCH_BALANCE_NANOS))));
        assert!(!too_low(&known(Some(12_500_000_000))));
        // No limit set, or nothing known: the launch goes ahead.
        assert!(!too_low(&known(None)));
        for unknown in [
            BalanceReport::NoSession,
            BalanceReport::SessionExpired,
            BalanceReport::NoOrganization,
            BalanceReport::Unavailable,
        ] {
            assert!(!too_low(&unknown));
        }
        assert_eq!(
            label(ManagedError::NearAiBalanceTooLow),
            "managed-near-ai-balance-too-low"
        );
    }

    /// A cached low figure may predate a payment: the launch is refused
    /// only when a fresh read agrees, and a fine cached figure is never
    /// read again.
    #[tokio::test]
    async fn a_low_cached_balance_is_read_fresh_before_refusing() {
        let reads = std::sync::Mutex::new(Vec::new());
        let read = |low_when_fresh: bool| {
            let reads = &reads;
            move |fresh: bool| {
                reads.lock().unwrap().push(fresh);
                std::future::ready(if fresh && !low_when_fresh {
                    known(Some(MIN_LAUNCH_BALANCE_NANOS))
                } else {
                    known(Some(0))
                })
            }
        };
        assert!(funds_for_first_calls(read(false)).await.is_ok());
        assert_eq!(*reads.lock().unwrap(), [false, true]);
        reads.lock().unwrap().clear();
        assert!(matches!(
            funds_for_first_calls(read(true)).await,
            Err(ManagedError::NearAiBalanceTooLow)
        ));
        assert_eq!(*reads.lock().unwrap(), [false, true]);
        reads.lock().unwrap().clear();
        let fine = |fresh: bool| {
            reads.lock().unwrap().push(fresh);
            std::future::ready(known(Some(12_500_000_000)))
        };
        assert!(funds_for_first_calls(fine).await.is_ok());
        assert_eq!(*reads.lock().unwrap(), [false]);
    }

    /// NEAR AI that never answers holds a launch for the deadline once, not
    /// once per read, and then lets it through.
    #[tokio::test(start_paused = true)]
    async fn an_unanswered_balance_lets_the_launch_through_after_one_deadline() {
        let started = tokio::time::Instant::now();
        let never = |_fresh: bool| std::future::pending::<BalanceReport>();
        assert!(funds_for_first_calls(never).await.is_ok());
        assert_eq!(started.elapsed(), FUNDS_CHECK_WITHIN);

        // A cached low figure, then a fresh read that never comes back:
        // still one deadline, and the launch goes ahead.
        let started = tokio::time::Instant::now();
        let low_then_never = |fresh: bool| async move {
            if fresh {
                std::future::pending().await
            } else {
                known(Some(0))
            }
        };
        assert!(funds_for_first_calls(low_then_never).await.is_ok());
        assert_eq!(started.elapsed(), FUNDS_CHECK_WITHIN);
    }

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
        call_choosing(service, method, params, sign_in, choice, false)
    }

    fn call_choosing(
        service: &mut ManagedService,
        method: &str,
        params: Value,
        sign_in: bool,
        setting: TerminalSetting,
        chosen: bool,
    ) -> Result<Value, ManagedError> {
        service.handle(
            &Request {
                id: 1,
                method: method.into(),
                params,
            },
            sign_in,
            TerminalChoice { setting, chosen },
        )
    }

    /// #1754 review: a shell sends `keep_saved_model` only to a daemon that
    /// says it reads it; a daemon that predates the field refuses it.
    #[test]
    fn the_snapshot_says_it_reads_keep_saved_model() {
        let (_dir, _store, mut service) = service();
        let snapshot = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        assert_eq!(snapshot["capabilities"]["keep_saved_model"], true);
    }

    /// #1754 review: "no saved model" and "could not tell" are different
    /// answers, so a failed lookup never writes `model: None` over the
    /// folder's saved model.
    #[test]
    fn saved_folder_model_tells_none_from_could_not_tell() {
        let (home, store, _service) = service();
        let shared = DaemonShared::load(store).unwrap();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let prefs = |cwd: &std::path::Path| crate::managed::folder::FolderSettings {
            tool: ToolId::Codex,
            cwd: cwd.to_path_buf(),
            model: None,
            collect_traces: false,
            cli_options: String::new(),
        };
        assert_eq!(saved_folder_model(&shared, &prefs(&folder)), Ok(None));
        shared
            .settings
            .lock()
            .unwrap()
            .managed_folder_settings
            .push(crate::managed::folder::FolderSettings {
                model: Some("provider/saved".into()),
                ..prefs(&crate::managed::folder::canonical(&folder).unwrap())
            });
        assert_eq!(
            saved_folder_model(&shared, &prefs(&folder)),
            Ok(Some("provider/saved".into()))
        );
        // The folder went away between prepare and this write.
        assert!(saved_folder_model(&shared, &prefs(&home.path().join("gone"))).is_err());
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
            keep_saved_model: false,
            resume_of: None,
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
                    keep_saved_model: false,
                    resume_of: None,
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

    /// The snapshot says whether the person has chosen a terminal, so a
    /// shell asks in the launch dialog until they have.
    #[test]
    fn the_snapshot_says_whether_the_terminal_was_chosen() {
        let (_home, _store, mut service) = service();
        for chosen in [false, true] {
            let snapshot = call_choosing(
                &mut service,
                "managed_snapshot",
                json!({}),
                false,
                TerminalSetting::default(),
                chosen,
            )
            .unwrap();
            assert_eq!(snapshot["capabilities"]["terminal_chosen"], chosen);
        }
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

    /// A start-again on a saved account never waits for the sign-in's
    /// balance, and no other method is a launch.
    #[test]
    fn only_a_sign_in_launch_waits_for_the_balance() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let (id, proof) = running(&mut service, &folder);
        report(&mut service, id, &proof, "exited", Some(0)).unwrap();
        let request = |method: &str, params: Value| Request {
            id: 1,
            method: method.into(),
            params,
        };
        let again = request(
            "managed_session_start_again",
            json!({"session_id":id,"request_id":Uuid::new_v4()}),
        );
        assert!(matches!(
            service.resolve_launch(&again, true, false),
            Ok(Resolved::Launch(..))
        ));
        assert!(!service.prepares_on_sign_in(&again, true));
        let snapshot = request("managed_snapshot", json!({}));
        assert!(matches!(
            service.resolve_launch(&snapshot, true, false),
            Err(ManagedError::InvalidRequest)
        ));
        assert!(!service.prepares_on_sign_in(&snapshot, true));
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
    /// A shell offers Resume only to a daemon that says it has it.
    #[test]
    fn the_snapshot_says_it_resumes_sessions() {
        let (_dir, _store, mut service) = service();
        let snapshot = call(&mut service, "managed_snapshot", json!({}), false).unwrap();
        assert_eq!(snapshot["capabilities"]["resume"], true);
    }

    /// Only `managed_session_resume` continues a conversation; a prepare
    /// that names one is refused, so no shell can resume an arbitrary row.
    #[test]
    fn a_prepare_cannot_name_a_conversation_to_continue() {
        let (home, _store, mut service) = service();
        let mut params = launch(ToolId::Claude, home.path());
        params["resume_of"] = json!(Uuid::new_v4());
        assert_eq!(
            call(&mut service, "managed_launch_prepare", params, true),
            Err(ManagedError::InvalidRequest)
        );
    }

    /// An ended coding row on `tool`'s saved subscription, in `cwd`.
    fn ended_on(service: &mut ManagedService, tool: ToolId, cwd: &std::path::Path) -> Uuid {
        let account = service
            .accounts
            .add(tool, ConnectionKind::Subscription, "Work")
            .unwrap();
        let mut request: LaunchRequest = serde_json::from_value(launch(tool, cwd)).unwrap();
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
        report(
            service,
            prepared.session_id,
            &context.proof,
            "exited",
            Some(0),
        )
        .unwrap();
        prepared.session_id
    }

    /// Claude Code's transcript of row `id`'s conversation in its account's
    /// profile, which Claude writes once the conversation has a turn.
    fn claude_transcript(service: &mut ManagedService, id: Uuid) {
        let account: AccountId =
            serde_json::from_value(row(service, id, true)["account_id"].clone()).unwrap();
        let project = service
            .profiles
            .profile_root(account)
            .unwrap()
            .join("projects/-project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), b"{}\n").unwrap();
    }

    /// A Claude Code row whose tool never wrote its conversation (a launch
    /// refused before the tool started, or quit before the first turn) can
    /// be started again but not resumed.
    #[test]
    fn a_claude_row_resumes_only_once_its_conversation_is_written() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let id = ended_on(&mut service, ToolId::Claude, &folder);
        let row_now = row(&mut service, id, false);
        assert_eq!(row_now["can_start_again"], true);
        assert_eq!(row_now["can_resume"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::Conflict)
        );
        claude_transcript(&mut service, id);
        assert_eq!(row(&mut service, id, false)["can_resume"], true);
    }

    /// An OpenCode row resumes only while its session's own directory is
    /// still in the account's profile.
    #[test]
    fn an_opencode_row_resumes_only_with_its_session_directory() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let account = service
            .accounts
            .add(ToolId::Opencode, ConnectionKind::ApiKey, "Work")
            .unwrap();
        let mut request: LaunchRequest =
            serde_json::from_value(launch(ToolId::Opencode, &folder)).unwrap();
        request.connection = ConnectionKind::ApiKey;
        request.account_id = account.id;
        request.model = Some("gpt-test".into());
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
        let id = prepared.session_id;
        let context = service
            .sessions
            .redeem(id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        report(&mut service, id, &context.proof, "exited", Some(0)).unwrap();
        assert_eq!(row(&mut service, id, false)["can_start_again"], true);
        assert_eq!(row(&mut service, id, false)["can_resume"], false);
        std::fs::create_dir(
            service
                .profiles
                .profile_root(account.id)
                .unwrap()
                .join(id.to_string()),
        )
        .unwrap();
        assert_eq!(row(&mut service, id, false)["can_resume"], true);
    }

    /// Start again on a resumed row starts a fresh conversation, so its
    /// launcher is told no root.
    #[test]
    fn starting_a_resumed_row_again_continues_nothing() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let root = ended_on(&mut service, ToolId::Claude, &folder);
        claude_transcript(&mut service, root);
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":root,"request_id":Uuid::new_v4()}),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let resumed = prepared.session_id;
        let context = service
            .sessions
            .redeem(resumed, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(context.resume_of, Some(root));
        report(&mut service, resumed, &context.proof, "exited", Some(0)).unwrap();
        let again: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_session_start_again",
                json!({"session_id":resumed,"request_id":Uuid::new_v4()}),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let context = service
            .sessions
            .redeem(again.session_id, again.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(context.resume_of, None);
    }

    /// While a resume of a conversation runs, no row on that conversation
    /// offers Resume.
    #[test]
    fn resume_is_withheld_while_the_conversation_runs_elsewhere() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let root = ended_on(&mut service, ToolId::Claude, &folder);
        claude_transcript(&mut service, root);
        assert_eq!(row(&mut service, root, false)["can_resume"], true);
        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":root,"request_id":Uuid::new_v4()}),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(row(&mut service, root, false)["can_resume"], false);
        assert_eq!(row(&mut service, root, false)["can_start_again"], true);
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":root,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::Conflict)
        );
        let context = service
            .sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        report(
            &mut service,
            prepared.session_id,
            &context.proof,
            "exited",
            Some(0),
        )
        .unwrap();
        assert_eq!(row(&mut service, root, false)["can_resume"], true);
        assert_eq!(
            row(&mut service, prepared.session_id, false)["can_resume"],
            true
        );
    }

    #[test]
    fn resume_continues_the_root_and_replays_on_retry() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let (id, proof) = running(&mut service, &folder);
        let resume = json!({"session_id":id,"request_id":Uuid::new_v4()});
        // Running: nothing to resume.
        assert_eq!(row(&mut service, id, false)["can_resume"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                resume.clone(),
                false
            ),
            Err(ManagedError::Conflict)
        );
        report(&mut service, id, &proof, "exited", Some(0)).unwrap();
        assert_eq!(
            row(&mut service, id, false)["can_resume"],
            false,
            "no turn yet"
        );
        claude_transcript(&mut service, id);
        assert_eq!(row(&mut service, id, false)["can_resume"], true);

        let prepared: PreparedLaunch = serde_json::from_value(
            call(
                &mut service,
                "managed_session_resume",
                resume.clone(),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(prepared.session_id, id, "a resumed launch is a new row");
        assert!(prepared.ticket.is_some());
        let fresh = row(&mut service, prepared.session_id, false);
        let old = row(&mut service, id, false);
        for field in ["tool", "connection", "account_id", "cwd", "purpose"] {
            assert_eq!(fresh[field], old[field], "{field}");
        }
        // The same request twice is one session.
        let repeated: PreparedLaunch = serde_json::from_value(
            call(&mut service, "managed_session_resume", resume, false).unwrap(),
        )
        .unwrap();
        assert_eq!(repeated.session_id, prepared.session_id);
        assert!(repeated.ticket.is_none());
        let context = service
            .sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(context.resume_of, Some(id));
    }

    #[test]
    fn resume_is_withheld_when_the_folder_or_account_is_gone() {
        let (home, _store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let id = ended_on(&mut service, ToolId::Claude, &folder);
        claude_transcript(&mut service, id);
        assert_eq!(row(&mut service, id, false)["can_resume"], true);
        std::fs::remove_dir(&folder).unwrap();
        assert_eq!(row(&mut service, id, false)["can_resume"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::InvalidRequest)
        );
        std::fs::create_dir(&folder).unwrap();
        let account = service.accounts.list()[0].id;
        service.accounts.remove(account).unwrap();
        assert_eq!(row(&mut service, id, false)["can_resume"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::NotFound)
        );
    }

    /// Codex's own thread id is known only from the root's binding. Without
    /// one, the row can be started again but not resumed.
    #[test]
    fn a_codex_row_resumes_only_with_its_recorded_thread() {
        let (home, store, mut service) = service();
        let folder = home.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let id = ended_on(&mut service, ToolId::Codex, &folder);
        let row_now = row(&mut service, id, false);
        assert_eq!(row_now["can_start_again"], true);
        assert_eq!(row_now["can_resume"], false);
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            ),
            Err(ManagedError::Conflict)
        );

        let binding = crate::managed::session_binding::prepare(&store, id).unwrap();
        let transcript = std::fs::canonicalize(home.path())
            .unwrap()
            .join("rollout.jsonl");
        std::fs::write(&transcript, b"{}\n").unwrap();
        let nonce = std::fs::read_to_string(binding.directory().join("nonce")).unwrap();
        crate::managed::session_binding::record(
            binding.directory(),
            &nonce,
            std::io::Cursor::new(
                serde_json::to_vec(&json!({
                    "session_id": Uuid::new_v4(), "transcript_path": transcript,
                    "hook_event_name": "SessionStart",
                }))
                .unwrap(),
            ),
        )
        .unwrap();
        assert_eq!(row(&mut service, id, false)["can_resume"], true);
        assert!(
            call(
                &mut service,
                "managed_session_resume",
                json!({"session_id":id,"request_id":Uuid::new_v4()}),
                false
            )
            .unwrap()["ticket"]
                .is_string()
        );
    }

    /// A sign-in row resumes only while signed in, and its resume waits for
    /// the sign-in's balance like any new session on it.
    #[test]
    fn a_sign_in_session_resumes_only_while_signed_in() {
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
        claude_transcript(&mut service, id);
        assert_eq!(row(&mut service, id, true)["can_resume"], true);
        assert_eq!(row(&mut service, id, false)["can_resume"], false);
        let params = json!({"session_id":id,"request_id":Uuid::new_v4()});
        assert_eq!(
            call(
                &mut service,
                "managed_session_resume",
                params.clone(),
                false
            ),
            Err(ManagedError::NearAiSignInRequired)
        );
        let request = Request {
            id: 1,
            method: "managed_session_resume".into(),
            params: params.clone(),
        };
        assert!(service.prepares_on_sign_in(&request, true));
        assert!(
            call(&mut service, "managed_session_resume", params, true).unwrap()["ticket"]
                .is_string()
        );
    }

    /// Collection after the launcher is gone (`collection_plan`). Unix:
    /// the fixtures start `true` and `sleep`.
    #[cfg(unix)]
    mod abandoned {
        use super::*;
        use crate::managed::{
            collection_plan::{self, CollectionPlan, ProcessIdentity},
            native_collection::NativeCollection,
            sessions::CollectionState,
        };
        use std::sync::Arc;

        struct Fixture {
            _home: tempfile::TempDir,
            store: ConfigStore,
            account: AccountView,
        }

        fn fixture() -> Fixture {
            let home = tempfile::tempdir().unwrap();
            let store = ConfigStore::open(home.path().join("state")).unwrap();
            let account = AccountStore::open(&store)
                .unwrap()
                .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
                .unwrap();
            Fixture {
                _home: home,
                store,
                account,
            }
        }

        fn transcript(f: &Fixture, id: Uuid) {
            let root = NativeProfiles::new(&f.store)
                .profile_root(f.account.id)
                .unwrap();
            let path = root
                .join("projects/-Users-test-project")
                .join(format!("{id}.jsonl"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!(r#"{{"type":"user","message":{{"role":"user","content":"Please fix the test"}},"cwd":"/Users/test/project","timestamp":"2026-08-08T10:00:00Z","version":"2.0.1","sessionId":"{id}","uuid":"u1"}}
"#)).unwrap();
        }

        fn staged(f: &Fixture, id: Uuid) -> bool {
            f.store
                .dir()
                .join(crate::managed::native_collection::STAGING)
                .join("claude")
                .join(id.to_string())
                .is_dir()
        }

        /// A collecting Claude Code row, redeemed and reported running by
        /// `process`, its plan written and its transcript there, as a
        /// launcher leaves it when it dies.
        fn running(
            f: &Fixture,
            sessions: &mut SessionRegistry,
            process: ProcessIdentity,
            collect_traces: bool,
        ) -> Uuid {
            let prepared = sessions
                .prepare(
                    LaunchRequest {
                        cli_options: None,
                        model: None,
                        collect_traces,
                        purpose: LaunchPurpose::Coding,
                        request_id: Uuid::new_v4(),
                        tool: ToolId::Claude,
                        connection: ConnectionKind::Subscription,
                        account_id: f.account.id,
                        cwd: f.store.dir().to_path_buf(),
                        expected_generation: Generation(0),
                        save_default: false,
                        keep_saved_model: false,
                        resume_of: None,
                    },
                    &f.account,
                )
                .unwrap();
            let id = prepared.session_id;
            let context = sessions
                .redeem(id, prepared.ticket.as_deref().unwrap())
                .unwrap();
            sessions
                .report(LifecycleReport {
                    model: None,
                    collection: None,
                    session_id: id,
                    proof: context.proof,
                    state: SessionState::Running,
                    process_id: Some(process.pid),
                    process_identity: Some(
                        process
                            .label()
                            .unwrap_or_else(|| format!("{}:0", process.pid)),
                    ),
                    exit_code: None,
                })
                .unwrap();
            let collection =
                NativeCollection::snapshot(&f.store, ToolId::Claude, f.account.id, id, None)
                    .unwrap();
            let plan = CollectionPlan {
                schema_version: 1,
                session_id: id,
                tool: ToolId::Claude,
                account_id: f.account.id,
                resume_root: None,
                cwd: f.store.dir().to_path_buf(),
                collect_traces: true,
                created_at: chrono::Utc::now(),
                native: Some(collection.plan()),
                opencode: None,
                process,
            };
            if collect_traces {
                collection_plan::write(&f.store, &plan).unwrap();
            } else {
                // A plan no launcher writes for such a row.
                let dir = f.store.dir().join(collection_plan::DIRECTORY);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join(format!("{id}.json")),
                    serde_json::to_vec(&plan).unwrap(),
                )
                .unwrap();
            }
            transcript(f, id);
            id
        }

        fn status(shared: &DaemonShared, id: Uuid) -> crate::managed::collection_plan::RowStatus {
            sessions_of(shared, |sessions| sessions.collection_status(id).unwrap())
        }

        fn sessions_of<T>(shared: &DaemonShared, f: impl FnOnce(&mut SessionRegistry) -> T) -> T {
            let mut held = shared.managed.lock().unwrap();
            f(&mut held.as_mut().unwrap().sessions)
        }

        fn exited() -> ProcessIdentity {
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let process = ProcessIdentity::of(child.id());
            child.wait().unwrap();
            process
        }

        fn lapsed() -> chrono::DateTime<chrono::Utc> {
            chrono::Utc::now() + crate::managed::sessions::LEASE + chrono::Duration::seconds(1)
        }

        #[tokio::test]
        async fn a_lapsed_rows_exited_tool_is_collected_once() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let id = sessions_of(&shared, |s| running(&f, s, exited(), true));
            assert_eq!(
                collect_abandoned(&shared, chrono::Utc::now()).await,
                0,
                "a launcher inside its lease collects for itself"
            );
            assert!(!staged(&f, id));
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 1);
            assert!(staged(&f, id));
            let row = status(&shared, id);
            assert_eq!(row.collection, CollectionState::Collected);
            assert_eq!(
                row.state,
                SessionState::Failed,
                "its tool exited: the row has ended"
            );
            assert!(!collection_plan::exists(&f.store, id), "removed once final");
            assert!(collection_plan::claimant(&f.store, id).is_none());
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
        }

        #[tokio::test]
        async fn nothing_is_collected_while_the_tool_runs() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let mut tool = std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .unwrap();
            let process = ProcessIdentity::of(tool.id());
            let id = sessions_of(&shared, |s| running(&f, s, process, true));
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
            assert!(!staged(&f, id));
            assert_eq!(status(&shared, id).collection, CollectionState::Pending);
            assert!(collection_plan::exists(&f.store, id));
            tool.kill().unwrap();
            tool.wait().unwrap();
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 1);
            assert!(staged(&f, id));
            assert_eq!(status(&shared, id).collection, CollectionState::Collected);
        }

        #[tokio::test]
        async fn a_row_that_does_not_collect_is_never_collected() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let id = sessions_of(&shared, |s| running(&f, s, exited(), false));
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
            assert!(!staged(&f, id));
            assert_eq!(status(&shared, id).collection, CollectionState::Off);
            assert!(
                !collection_plan::exists(&f.store, id),
                "the stray plan is dropped"
            );
        }

        #[tokio::test]
        async fn rows_a_restart_found_unknown_are_collected_at_startup() {
            let f = fixture();
            let id = {
                let mut sessions = SessionRegistry::open(&f.store).unwrap();
                running(&f, &mut sessions, exited(), true)
            };
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            assert_eq!(status(&shared, id).state, SessionState::Unknown);
            assert_eq!(
                collect_abandoned(&shared, chrono::Utc::now()).await,
                1
            );
            assert!(staged(&f, id));
            assert_eq!(status(&shared, id).collection, CollectionState::Collected);
            assert!(!collection_plan::exists(&f.store, id));
        }

        /// A collected row whose launcher was killed has ended once its
        /// tool is gone, so its conversation can be resumed.
        #[tokio::test]
        async fn a_killed_rows_conversation_can_be_resumed_once_collected() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let id = sessions_of(&shared, |s| running(&f, s, exited(), true));
            let can_resume = |shared: &DaemonShared| {
                let mut held = shared.managed.lock().unwrap();
                let service = held.as_mut().unwrap();
                service.sessions.sweep(lapsed()).unwrap();
                row(service, id, false)["can_resume"].clone()
            };
            assert_eq!(can_resume(&shared), false, "unknown: it may still run");
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 1);
            assert_eq!(status(&shared, id).state, SessionState::Failed);
            assert_eq!(can_resume(&shared), true);
        }

        /// A plan this build cannot read fails its row's collection and is
        /// removed, so the row does not stay pending.
        #[tokio::test]
        async fn an_unreadable_plan_fails_its_rows_collection_and_is_removed() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let id = sessions_of(&shared, |s| running(&f, s, exited(), true));
            let path = f
                .store
                .dir()
                .join(collection_plan::DIRECTORY)
                .join(format!("{id}.json"));
            let mut plan: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            plan["schema_version"] = json!(2);
            std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
            assert_eq!(collect_abandoned(&shared, chrono::Utc::now()).await, 0);
            assert!(path.exists(), "its launcher still reports");
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
            assert!(!staged(&f, id));
            assert_eq!(status(&shared, id).collection, CollectionState::Failed);
            assert!(!path.exists());
        }

        /// A holder that died after staging the row's trace, before it
        /// recorded the result, leaves the row collected, not failed.
        #[tokio::test]
        async fn a_dead_holder_that_staged_the_trace_leaves_the_row_collected() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let id = sessions_of(&shared, |s| running(&f, s, exited(), true));
            let plan = collection_plan::read(&f.store, id).unwrap();
            assert_eq!(collection_plan::claim(&f.store, id, &exited()), Ok(true));
            collection_plan::collect(&f.store, &plan).unwrap();
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
            assert!(staged(&f, id));
            let row = status(&shared, id);
            assert_eq!(row.collection, CollectionState::Collected);
            assert_eq!(row.state, SessionState::Failed);
            assert!(!collection_plan::exists(&f.store, id));
        }

        /// The launcher took the claim: the daemon does nothing while that
        /// launcher lives, and records a failure once it died holding it.
        #[tokio::test]
        async fn the_daemon_never_collects_a_row_the_launcher_claimed() {
            let f = fixture();
            let shared = Arc::new(DaemonShared::load(f.store.clone()).unwrap());
            let id = sessions_of(&shared, |s| running(&f, s, exited(), true));
            let mut launcher = std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .unwrap();
            let holder = ProcessIdentity::of(launcher.id());
            assert_eq!(collection_plan::claim(&f.store, id, &holder), Ok(true));
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
            assert!(!staged(&f, id));
            assert_eq!(status(&shared, id).collection, CollectionState::Pending);
            assert_eq!(collection_plan::claimant(&f.store, id), Some(Some(holder)));
            launcher.kill().unwrap();
            launcher.wait().unwrap();
            assert_eq!(collect_abandoned(&shared, lapsed()).await, 0);
            assert!(!staged(&f, id), "a dead holder's claim is not taken over");
            assert_eq!(status(&shared, id).collection, CollectionState::Failed);
            assert!(!collection_plan::exists(&f.store, id));
        }
    }
}
