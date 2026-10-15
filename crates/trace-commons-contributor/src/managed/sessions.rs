//! Durable lifecycle for managed native processes. A UI is only an observer.

use super::{
    AccountId, AccountView, ConnectionKind, Generation, ManagedError, ToolId,
    terminal::TerminalHandle,
};
use crate::config::ConfigStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

const FILE: &str = "managed-sessions.json";

/// How long a redeemed session may go unheard before it is no longer taken
/// to be running. The launcher reports every 5 seconds, so this is nine
/// missed reports; it is also inside the 120-second launch ticket.
pub const LEASE: Duration = Duration::seconds(45);

/// The state a session reaches when its lease runs out. A session on the
/// NEAR AI sign-in has lost its daemon-hosted proxy by then, so it has
/// ended whatever its tool is doing; any other session is unknown.
fn lapsed(connection: ConnectionKind) -> SessionState {
    if connection == ConnectionKind::NearAiSignIn {
        SessionState::Failed
    } else {
        SessionState::Unknown
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPurpose {
    #[default]
    Coding,
    Login,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Running,
    Exited,
    Failed,
    Unknown,
}

impl SessionState {
    pub fn holds_profile(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRequest {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub collect_traces: bool,
    #[serde(default)]
    pub purpose: LaunchPurpose,
    pub request_id: Uuid,
    pub tool: ToolId,
    pub connection: ConnectionKind,
    pub account_id: AccountId,
    pub cwd: PathBuf,
    pub expected_generation: Generation,
    #[serde(default)]
    pub save_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub collect_traces: bool,
    #[serde(default)]
    pub collection: CollectionState,
    #[serde(default)]
    pub purpose: LaunchPurpose,
    pub id: Uuid,
    pub tool: ToolId,
    pub connection: ConnectionKind,
    pub account_id: AccountId,
    pub account_label: String,
    pub selection_generation: Generation,
    pub project_label: String,
    pub cwd: PathBuf,
    pub started_at: DateTime<Utc>,
    pub state: SessionState,
    pub exit_code: Option<i32>,
    /// The daemon opened this session in a terminal it can script, named
    /// its window, and the session runs: a shell may bring that window to
    /// the front. Computed on every read, never trusted from storage.
    pub can_focus: bool,
    /// The daemon opened this session and its launcher reports a running
    /// tool, so a stop can be handed to that launcher, which owns the
    /// process. False for a launch from the command line.
    #[serde(default)]
    pub can_stop: bool,
    /// The session ended and the same launch can be made again: its
    /// account and folder are still there. Set by the daemon's snapshot.
    #[serde(default)]
    pub can_start_again: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PreparedLaunch {
    pub session_id: Uuid,
    /// Only the first prepare returns a ticket. Repeated requests reconcile.
    pub ticket: Option<String>,
}

impl std::fmt::Debug for PreparedLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedLaunch")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
pub struct LaunchContext {
    pub session: SessionView,
    pub proof: String,
    /// The loopback port of a proxy the daemon hosts for this session. Set
    /// only for a session on the NEAR AI sign-in, whose credential never
    /// leaves the daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionState {
    #[default]
    Off,
    Pending,
    Collected,
    Failed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleReport {
    #[serde(default)]
    pub collection: Option<CollectionState>,
    pub session_id: Uuid,
    pub proof: String,
    pub state: SessionState,
    pub process_id: Option<u32>,
    pub process_identity: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    dismissed: bool,
    /// The daemon opened this session's terminal itself, from its own
    /// ticket check. A launch from the command line never sets it.
    #[serde(default)]
    launched: bool,
    /// The window that launch opened, when the terminal could name it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal: Option<TerminalHandle>,
    /// A stop was asked for; the launcher is told on its next report.
    #[serde(default)]
    stop_requested: bool,
    view: SessionView,
    request: LaunchRequest,
    ticket_hash: Option<String>,
    ticket_expires: DateTime<Utc>,
    proof_hash: Option<String>,
    process_id: Option<u32>,
    process_identity: Option<String>,
    last_seen: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    schema_version: u32,
    revision: u64,
    entries: Vec<Entry>,
}

pub struct SessionRegistry {
    store: ConfigStore,
    data: Data,
}

fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

impl SessionRegistry {
    pub fn open(store: &ConfigStore) -> Result<Self, ManagedError> {
        let mut data: Data = match store
            .read_daemon_file(FILE)
            .map_err(|_| ManagedError::StorageUnavailable)?
        {
            Some(bytes) => {
                serde_json::from_slice(&bytes).map_err(|_| ManagedError::StorageUnavailable)?
            }
            None => Data {
                schema_version: 1,
                revision: 0,
                entries: vec![],
            },
        };
        if data.schema_version != 1 {
            return Err(ManagedError::StorageUnavailable);
        }
        // A recovered PID alone is not evidence that the same native tool lives.
        for entry in &mut data.entries {
            if entry.view.state.holds_profile() {
                // A restarted daemon hosts no proxy for a sign-in session.
                entry.view.state = if entry.proof_hash.is_none() {
                    SessionState::Failed
                } else {
                    lapsed(entry.view.connection)
                };
            }
            if entry.view.state == SessionState::Failed
                && entry.view.collection == CollectionState::Pending
            {
                entry.view.collection = CollectionState::Failed;
            }
            entry.ticket_hash = None;
        }
        let mut registry = Self {
            store: ConfigStore::open(store.dir().to_path_buf())
                .map_err(|_| ManagedError::StorageUnavailable)?,
            data,
        };
        registry.commit(registry.data.clone())?;
        registry.reconcile_receipts()?;
        Ok(registry)
    }

    fn reconcile_receipts(&mut self) -> Result<(), ManagedError> {
        let ids: Vec<_> = self.data.entries.iter().map(|e| e.view.id).collect();
        for id in ids {
            let name = receipt_name(id);
            if let Some(bytes) = self
                .store
                .read_daemon_file(&name)
                .map_err(|_| ManagedError::StorageUnavailable)?
            {
                let report: LifecycleReport =
                    serde_json::from_slice(&bytes).map_err(|_| ManagedError::StorageUnavailable)?;
                if report.session_id != id
                    || !matches!(report.state, SessionState::Exited | SessionState::Failed)
                {
                    return Err(ManagedError::InvalidRequest);
                }
                self.report(report)?;
                let _ = std::fs::remove_file(self.store.dir().join(name));
            }
        }
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.data.revision
    }

    pub fn views(&mut self) -> Result<Vec<SessionView>, ManagedError> {
        self.reconcile_receipts()?;
        self.sweep(Utc::now())?;
        Ok(self
            .data
            .entries
            .iter()
            .filter(|e| !e.dismissed)
            .map(|e| {
                let mut view = e.view.clone();
                let owned = e.owned_running();
                view.can_focus = owned && e.terminal.is_some();
                view.can_stop = owned && !e.stop_requested;
                view.can_start_again = false;
                view
            })
            .collect())
    }

    /// Record that the daemon opened `id` in a terminal, after its ticket
    /// check and its own launch, with the window when one was named.
    pub fn opened_in_terminal(
        &mut self,
        id: Uuid,
        terminal: Option<TerminalHandle>,
    ) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        entry.launched = true;
        entry.terminal = terminal;
        self.commit(next)
    }

    fn owned(&self, id: Uuid) -> Result<&Entry, ManagedError> {
        let entry = self
            .data
            .entries
            .iter()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        if entry.owned_running() {
            Ok(entry)
        } else {
            Err(ManagedError::Conflict)
        }
    }

    /// The window of a running session the daemon opened, or a refusal.
    pub fn terminal_for(&self, id: Uuid) -> Result<TerminalHandle, ManagedError> {
        self.owned(id)?
            .terminal
            .clone()
            .ok_or(ManagedError::Conflict)
    }

    /// Ask the launcher of a running session the daemon opened to stop its
    /// tool. Nothing is signalled here: the launcher owns the process and
    /// hears this on its next report. Asking twice changes nothing.
    pub fn request_stop(&mut self, id: Uuid) -> Result<(), ManagedError> {
        if self.owned(id)?.stop_requested {
            return Ok(());
        }
        let mut next = self.data.clone();
        if let Some(entry) = next.entries.iter_mut().find(|e| e.view.id == id) {
            entry.stop_requested = true;
        }
        self.commit(next)
    }

    /// Whether the launcher of `id` is to stop its tool now.
    pub fn stop_requested(&self, id: Uuid) -> bool {
        self.data
            .entries
            .iter()
            .any(|e| e.view.id == id && e.stop_requested && e.owned_running())
    }

    /// The launch request that started an ended coding session, under a
    /// new request id. Its account and folder are checked by the caller.
    pub fn start_again_request(
        &self,
        id: Uuid,
        request_id: Uuid,
    ) -> Result<LaunchRequest, ManagedError> {
        let entry = self
            .data
            .entries
            .iter()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        if entry.view.state.holds_profile() {
            return Err(ManagedError::Conflict);
        }
        if entry.request.purpose != LaunchPurpose::Coding {
            return Err(ManagedError::InvalidRequest);
        }
        Ok(LaunchRequest {
            request_id,
            save_default: false,
            ..entry.request.clone()
        })
    }

    /// Fail unredeemed launches whose ticket expired, and lapse redeemed
    /// sessions unheard for longer than [`LEASE`], as of `now`.
    pub fn sweep(&mut self, now: DateTime<Utc>) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let mut changed = false;
        for entry in &mut next.entries {
            if entry.view.state == SessionState::Starting
                && entry.proof_hash.is_none()
                && now >= entry.ticket_expires
            {
                entry.view.state = SessionState::Failed;
                if entry.view.collection == CollectionState::Pending {
                    entry.view.collection = CollectionState::Failed;
                }
                entry.ticket_hash = None;
                changed = true;
            } else if matches!(
                entry.view.state,
                SessionState::Starting | SessionState::Running
            ) && entry.proof_hash.is_some()
                && now.signed_duration_since(entry.last_seen) > LEASE
            {
                entry.view.state = lapsed(entry.view.connection);
                changed = true;
            }
        }
        if changed {
            self.commit(next)?;
        }
        Ok(())
    }

    /// The NEAR AI sign-in is gone: every session on it has lost its proxy,
    /// so none is running any more. Failed, the state for a session that
    /// ended without an exit code from its tool.
    pub fn end_sign_in_sessions(&mut self) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let mut changed = false;
        for entry in &mut next.entries {
            if entry.view.connection == ConnectionKind::NearAiSignIn
                && entry.view.state.holds_profile()
            {
                entry.view.state = SessionState::Failed;
                entry.ticket_hash = None;
                changed = true;
            }
        }
        if changed {
            self.commit(next)?;
        }
        Ok(())
    }

    pub fn holds_profile(&self, id: AccountId) -> bool {
        self.data
            .entries
            .iter()
            .any(|e| e.view.account_id == id && e.view.state.holds_profile())
    }

    pub fn prepare(
        &mut self,
        request: LaunchRequest,
        account: &AccountView,
    ) -> Result<PreparedLaunch, ManagedError> {
        if request.tool == ToolId::Opencode {
            if request.purpose != LaunchPurpose::Coding
                || matches!(
                    request.connection,
                    ConnectionKind::Subscription | ConnectionKind::NearAiSignIn
                )
                || request
                    .model
                    .as_deref()
                    .is_none_or(|model| !super::opencode::valid_model(model))
            {
                return Err(ManagedError::InvalidRequest);
            }
        } else if request.model.is_some() || request.collect_traces {
            return Err(ManagedError::InvalidRequest);
        }
        if request.account_id != account.id
            || request.tool != account.tool
            || request.connection != account.connection
        {
            return Err(ManagedError::Conflict);
        }
        if let Some(entry) = self
            .data
            .entries
            .iter()
            .find(|e| e.request.request_id == request.request_id)
        {
            if entry.request != request {
                return Err(ManagedError::Conflict);
            }
            return Ok(PreparedLaunch {
                session_id: entry.view.id,
                ticket: None,
            });
        }
        if !request.cwd.is_absolute() || !request.cwd.is_dir() {
            return Err(ManagedError::InvalidRequest);
        }
        if self.data.entries.iter().any(|e| {
            e.view.account_id == account.id
                && e.view.state.holds_profile()
                && (request.purpose == LaunchPurpose::Login
                    || e.view.purpose == LaunchPurpose::Login)
        }) {
            return Err(ManagedError::Conflict);
        }
        let now = Utc::now();
        let ticket = secret();
        let id = Uuid::new_v4();
        let view = SessionView {
            model: request.model.clone(),
            collect_traces: request.collect_traces,
            collection: if request.collect_traces {
                CollectionState::Pending
            } else {
                CollectionState::Off
            },
            purpose: request.purpose,
            id,
            tool: request.tool,
            connection: request.connection,
            account_id: account.id,
            account_label: account.label.clone(),
            selection_generation: request.expected_generation,
            project_label: request
                .cwd
                .file_name()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            cwd: request.cwd.clone(),
            started_at: now,
            state: SessionState::Starting,
            exit_code: None,
            can_focus: false,
            can_stop: false,
            can_start_again: false,
        };
        let entry = Entry {
            dismissed: false,
            launched: false,
            terminal: None,
            stop_requested: false,
            view,
            request,
            ticket_hash: Some(digest(&ticket)),
            ticket_expires: now + Duration::seconds(120),
            proof_hash: None,
            process_id: None,
            process_identity: None,
            last_seen: now,
        };
        let mut next = self.data.clone();
        next.entries.push(entry);
        self.commit(next)?;
        Ok(PreparedLaunch {
            session_id: id,
            ticket: Some(ticket),
        })
    }

    pub fn validate_ticket(&self, session_id: Uuid, ticket: &str) -> Result<(), ManagedError> {
        let entry = self
            .data
            .entries
            .iter()
            .find(|e| e.view.id == session_id)
            .ok_or(ManagedError::NotFound)?;
        if entry.ticket_hash.as_deref() != Some(&digest(ticket))
            || Utc::now() >= entry.ticket_expires
            || entry.view.state != SessionState::Starting
        {
            return Err(ManagedError::Conflict);
        }
        Ok(())
    }

    pub fn redeem(
        &mut self,
        session_id: Uuid,
        ticket: &str,
    ) -> Result<LaunchContext, ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == session_id)
            .ok_or(ManagedError::NotFound)?;
        if entry.ticket_hash.as_deref() != Some(&digest(ticket))
            || Utc::now() >= entry.ticket_expires
            || entry.view.state != SessionState::Starting
        {
            return Err(ManagedError::Conflict);
        }
        let proof = secret();
        entry.ticket_hash = None;
        entry.proof_hash = Some(digest(&proof));
        entry.last_seen = Utc::now();
        let session = entry.view.clone();
        self.commit(next)?;
        Ok(LaunchContext {
            session,
            proof,
            route_port: None,
        })
    }

    pub fn report(&mut self, report: LifecycleReport) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == report.session_id)
            .ok_or(ManagedError::NotFound)?;
        if entry.proof_hash.as_deref() != Some(&digest(&report.proof)) {
            return Err(ManagedError::Conflict);
        }
        if !entry.view.state.holds_profile() {
            // The daemon ended a sign-in session (sign-out, lapsed lease)
            // before its launcher said how it ended. The launcher's final
            // word is accepted and changes nothing, so its persisted exit
            // receipt never blocks the registry from opening.
            let ended_by_daemon = entry.view.connection == ConnectionKind::NearAiSignIn
                && entry.view.state == SessionState::Failed
                && matches!(report.state, SessionState::Exited | SessionState::Failed);
            return if ended_by_daemon
                || (report.state == entry.view.state && report.exit_code == entry.view.exit_code)
            {
                Ok(())
            } else {
                Err(ManagedError::Conflict)
            };
        }
        if report.state == SessionState::Starting {
            return Err(ManagedError::InvalidRequest);
        }
        if report.state == SessionState::Running
            && (report.process_id.is_none()
                || report
                    .process_identity
                    .as_ref()
                    .is_none_or(|s| s.is_empty() || s.len() > 128)
                || report.exit_code.is_some())
        {
            return Err(ManagedError::InvalidRequest);
        }
        if report.state == SessionState::Exited && report.exit_code.is_none() {
            return Err(ManagedError::InvalidRequest);
        }
        if entry.process_id.is_some()
            && (entry.process_id != report.process_id
                || entry.process_identity != report.process_identity)
        {
            return Err(ManagedError::Conflict);
        }
        if let Some(collection) = report.collection {
            if !entry.view.collect_traces
                || !matches!(report.state, SessionState::Exited | SessionState::Failed)
                || !matches!(
                    collection,
                    CollectionState::Collected | CollectionState::Failed
                )
            {
                return Err(ManagedError::InvalidRequest);
            }
            entry.view.collection = collection;
        }
        entry.process_id = report.process_id;
        entry.process_identity = report.process_identity;
        entry.view.state = report.state;
        entry.view.exit_code = report.exit_code;
        entry.last_seen = Utc::now();
        self.commit(next)
    }

    /// The connection and state of a session, without consuming anything.
    pub fn describe(&self, id: Uuid) -> Option<(ConnectionKind, SessionState)> {
        self.data
            .entries
            .iter()
            .find(|e| e.view.id == id)
            .map(|e| (e.view.connection, e.view.state))
    }

    /// Fail a session that was never redeemed, for the ticket holder only.
    pub fn fail_unredeemed(&mut self, session_id: Uuid, ticket: &str) -> Result<(), ManagedError> {
        self.validate_ticket(session_id, ticket)?;
        let mut next = self.data.clone();
        if let Some(entry) = next.entries.iter_mut().find(|e| e.view.id == session_id) {
            entry.view.state = SessionState::Failed;
            entry.ticket_hash = None;
        }
        self.commit(next)
    }

    pub fn dismiss(&mut self, id: Uuid) -> Result<(), ManagedError> {
        let entry = self
            .data
            .entries
            .iter()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        if entry.view.state.holds_profile() {
            return Err(ManagedError::Conflict);
        }
        let mut next = self.data.clone();
        if let Some(entry) = next.entries.iter_mut().find(|e| e.view.id == id) {
            entry.dismissed = true;
        }
        self.commit(next)
    }

    pub fn changed(&mut self) -> Result<(), ManagedError> {
        self.commit(self.data.clone())
    }

    fn commit(&mut self, mut next: Data) -> Result<(), ManagedError> {
        next.revision = self
            .data
            .revision
            .checked_add(1)
            .ok_or(ManagedError::StorageUnavailable)?;
        self.store
            .write_daemon_file(
                FILE,
                &serde_json::to_vec(&next).map_err(|_| ManagedError::StorageUnavailable)?,
            )
            .map_err(|_| ManagedError::StorageUnavailable)?;
        self.data = next;
        Ok(())
    }
}
impl Entry {
    /// The daemon opened this session itself and its launcher reports the
    /// tool running, under the proof only that launcher holds.
    fn owned_running(&self) -> bool {
        self.launched
            && self.view.state == SessionState::Running
            && self.view.purpose == LaunchPurpose::Coding
            && self.proof_hash.is_some()
            && self.process_id.is_some()
    }
}

fn receipt_name(id: Uuid) -> String {
    format!("managed-exit-{id}.json")
}

/// Write before the final RPC so a daemon outage cannot lose the native exit.
pub fn persist_exit(store: &ConfigStore, report: &LifecycleReport) -> Result<(), ManagedError> {
    if !matches!(report.state, SessionState::Exited | SessionState::Failed) {
        return Err(ManagedError::InvalidRequest);
    }
    let bytes = serde_json::to_vec(report).map_err(|_| ManagedError::StorageUnavailable)?;
    store
        .write_daemon_file(&receipt_name(report.session_id), &bytes)
        .map_err(|_| ManagedError::StorageUnavailable)
}

pub fn clear_exit(store: &ConfigStore, id: Uuid) {
    let _ = std::fs::remove_file(store.dir().join(receipt_name(id)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::accounts::AccountStore;

    #[test]
    fn expired_ticket_cannot_start_a_tool_and_restart_invalidates_unredeemed_ticket() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut accounts = AccountStore::open(&store).unwrap();
        let account = accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let request = LaunchRequest {
            model: None,
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool: ToolId::Claude,
            connection: ConnectionKind::Subscription,
            account_id: account.id,
            cwd: home.path().to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
        };
        let launch = registry.prepare(request.clone(), &account).unwrap();
        registry.data.entries[0].ticket_expires = Utc::now() - Duration::seconds(1);
        assert!(
            registry
                .redeem(launch.session_id, launch.ticket.as_deref().unwrap())
                .is_err()
        );
        assert_eq!(registry.views().unwrap()[0].state, SessionState::Failed);
        let second = registry
            .prepare(
                LaunchRequest {
                    request_id: Uuid::new_v4(),
                    ..request
                },
                &account,
            )
            .unwrap();
        let mut reopened = SessionRegistry::open(&store).unwrap();
        assert!(
            reopened
                .redeem(second.session_id, second.ticket.as_deref().unwrap())
                .is_err()
        );
        assert_eq!(reopened.views().unwrap()[1].state, SessionState::Failed);
        assert!(!reopened.holds_profile(account.id));
    }
}
