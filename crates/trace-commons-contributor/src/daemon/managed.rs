//! One local source of truth for CLI and desktop managed sessions.

use super::ipc::{DaemonShared, Request, Response};
use crate::{
    config::ConfigStore,
    managed::{
        AccountId, AccountView, AuthState, ConnectionKind, Generation, ManagedError, Selection,
        ToolId,
        accounts::AccountStore,
        profiles::{NativeProfiles, find_native},
        secrets::{ApiKey, SecretStore},
        sessions::{LaunchPurpose, LaunchRequest, LifecycleReport, SessionRegistry},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub struct ManagedService {
    pub accounts: AccountStore,
    pub sessions: SessionRegistry,
    profiles: NativeProfiles,
    config_dir: std::path::PathBuf,
    secrets: SecretStore,
}

impl ManagedService {
    pub fn open(store: &ConfigStore) -> Result<Self, ManagedError> {
        Ok(Self {
            accounts: AccountStore::open(store)?,
            sessions: SessionRegistry::open(store)?,
            profiles: NativeProfiles::new(store),
            config_dir: store.dir().to_path_buf(),
            secrets: SecretStore::new(store),
        })
    }

    fn account(&self, id: AccountId) -> Result<AccountView, ManagedError> {
        self.accounts
            .list()
            .into_iter()
            .find(|a| a.id == id)
            .ok_or(ManagedError::NotFound)
    }

    fn handle(&mut self, request: &Request) -> Result<Value, ManagedError> {
        match request.method.as_str() {
            "managed_snapshot" => {
                let sessions = self.sessions.views()?;
                Ok(
                    json!({"copy":crate::managed::copy::copy(),"revision":self.sessions.revision(),"accounts":self.accounts.list(),"generations":{"claude":self.accounts.generation(ToolId::Claude),"codex":self.accounts.generation(ToolId::Codex)},"defaults":([self.accounts.selection(ToolId::Claude),self.accounts.selection(ToolId::Codex)].into_iter().flatten().collect::<Vec<_>>()),"sessions":sessions,"capabilities":{"managed_launch":true,"terminal_launch":crate::managed::terminal::destination().is_some(),"terminal_destination":crate::managed::terminal::destination()}}),
                )
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
                let p: LaunchRequest = parse(request)?;
                let account = self
                    .accounts
                    .list()
                    .into_iter()
                    .find(|a| a.id == p.account_id)
                    .ok_or(ManagedError::NotFound)?;
                let generation = self.accounts.generation(p.tool);
                if p.expected_generation != generation {
                    return Err(ManagedError::Conflict);
                }
                // Default changes are separate until the cross-file transaction is available.
                if p.save_default {
                    return Err(ManagedError::InvalidRequest);
                }
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
                self.sessions.validate_ticket(p.session_id, &p.ticket)?;
                crate::managed::terminal::launch(&self.config_dir, p.session_id, &p.ticket)?;
                Ok(json!({"opened":true,"destination":crate::managed::terminal::destination()}))
            }
            "managed_launch_redeem" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params {
                    session_id: Uuid,
                    ticket: String,
                }
                let p: Params = parse(request)?;
                Ok(json!(self.sessions.redeem(p.session_id, &p.ticket)?))
            }
            "managed_session_report" => {
                let report: LifecycleReport = parse(request)?;
                self.sessions.report(report)?;
                Ok(json!({"accepted":true}))
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
                    ConnectionKind::Subscription => self
                        .profiles
                        .inspect(&account, &find_native(account.tool)?)?,
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
                let request = LaunchRequest {
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

pub fn handle(shared: &DaemonShared, request: &Request) -> Response {
    let Ok(mut held) = shared.managed.lock() else {
        return Response::err(request.id, "unavailable", "managed-service-unavailable");
    };
    let service = match held.as_mut() {
        Ok(service) => service,
        Err(_) => return Response::err(request.id, "unavailable", "managed-storage-unavailable"),
    };
    let before = service.sessions.revision();
    let result = service.handle(request);
    let after = service.sessions.revision();
    if before != after {
        shared.publish("managed_changed", json!({"revision":after}));
    }
    match result {
        Ok(value) => Response::ok(request.id, value),
        Err(error) => {
            let code = match error {
                ManagedError::Conflict => "managed-conflict",
                ManagedError::NotFound => "managed-not-found",
                ManagedError::AmbiguousAccount => "managed-ambiguous-account",
                ManagedError::StorageUnavailable => "managed-storage-unavailable",
                ManagedError::UnsupportedVersion => "managed-unsupported-version",
                ManagedError::SignInRequired => "managed-sign-in-required",
                ManagedError::DaemonUnavailable => "managed-daemon-unavailable",
                ManagedError::LaunchUnknown => "managed-launch-unknown",
                ManagedError::InvalidRequest => "managed-invalid-request",
            };
            Response::err(request.id, code, code)
        }
    }
}
