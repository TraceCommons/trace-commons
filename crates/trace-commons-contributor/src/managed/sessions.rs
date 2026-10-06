//! Durable lifecycle for managed native processes. A UI is only an observer.

use super::{AccountId, AccountView, ConnectionKind, Generation, ManagedError, ToolId};
use crate::config::ConfigStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

const FILE: &str = "managed-sessions.json";

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
    pub can_focus: bool,
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
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleReport {
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
                entry.view.state = if entry.proof_hash.is_none() {
                    SessionState::Failed
                } else {
                    SessionState::Unknown
                };
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
        let now = Utc::now();
        let mut next = self.data.clone();
        let mut changed = false;
        for entry in &mut next.entries {
            if entry.view.state == SessionState::Starting
                && entry.proof_hash.is_none()
                && now >= entry.ticket_expires
            {
                entry.view.state = SessionState::Failed;
                entry.ticket_hash = None;
                changed = true;
            } else if matches!(
                entry.view.state,
                SessionState::Starting | SessionState::Running
            ) && entry.proof_hash.is_some()
                && now.signed_duration_since(entry.last_seen) > Duration::seconds(45)
            {
                entry.view.state = SessionState::Unknown;
                changed = true;
            }
        }
        if changed {
            self.commit(next)?;
        }
        Ok(self
            .data
            .entries
            .iter()
            .filter(|e| !e.dismissed)
            .map(|e| e.view.clone())
            .collect())
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
        };
        let entry = Entry {
            dismissed: false,
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
        Ok(LaunchContext { session, proof })
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
            return if report.state == entry.view.state && report.exit_code == entry.view.exit_code {
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
        entry.process_id = report.process_id;
        entry.process_identity = report.process_identity;
        entry.view.state = report.state;
        entry.view.exit_code = report.exit_code;
        entry.last_seen = Utc::now();
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
