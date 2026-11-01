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
/// Present once a coding session has started on this computer, from the
/// app or the command line (#1408). Its own file, not a field of
/// [`FILE`], so a downgraded daemon still opens the registry.
const FIRST_LAUNCH_FILE: &str = "managed-first-launch.json";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_options: Option<String>,
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
    /// The launcher fell back to the default because NEAR AI no longer
    /// lists the folder's saved model: the launch's other choices are
    /// remembered, the saved model is kept. Absent means false. Read from a
    /// request, never written: the registry keeps it on the entry, where a
    /// daemon that predates it can skip it, and a shell sends it only when
    /// the snapshot says `capabilities.keep_saved_model`.
    #[serde(default, skip_serializing)]
    pub keep_saved_model: bool,
    /// The root row whose native conversation this launch continues, set
    /// only by [`SessionRegistry::resume_request`]. Resuming a resumed row
    /// names that row's root, so a chain always points at the first row
    /// and the native id never drifts. Absent means a fresh conversation.
    /// Read from a request, never written, like `keep_saved_model`: the
    /// registry keeps it on the entry, so a daemon that predates it can
    /// still open the registry. `managed_launch_prepare` refuses it.
    #[serde(default, skip_serializing)]
    pub resume_of: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    /// The model the session starts on: the one its request named, or, on
    /// NEAR AI, its proxy's default once that proxy has started. The tool
    /// may switch to another listed model later; that is not recorded.
    /// None for a session that names no model and has no proxy.
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
    /// The session ended and its conversation can be continued in a new
    /// session: every condition of `can_start_again`, and the tool's own
    /// conversation id is known. Set by the daemon's snapshot.
    #[serde(default)]
    pub can_resume: bool,
    /// For a resumed row: when its conversation's root row started, so a
    /// shell can say "Continues 2:14 PM". Recorded when the row is
    /// prepared, so it survives the root being dismissed. Absent for a
    /// fresh row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PreparedLaunch {
    pub session_id: Uuid,
    /// Only the first prepare returns a ticket. Repeated requests reconcile.
    pub ticket: Option<String>,
}

/// What [`SessionRegistry::check`] found a launch to be.
enum Checked {
    /// The session this request id already prepared.
    Replayed(PreparedLaunch),
    /// A new session, with the CLI options it would carry.
    New {
        cli_options_text: Option<String>,
        cli_options: Vec<String>,
    },
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
    /// Saved argv captured when this launch was first prepared.
    #[serde(default)]
    pub cli_options: Vec<String>,
    pub session: SessionView,
    pub proof: String,
    /// The loopback port of a proxy the daemon hosts for this session. Set
    /// only for a session on the NEAR AI sign-in, whose credential never
    /// leaves the daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_port: Option<u16>,
    /// The model the tool starts on: the one the request named, or the
    /// tool's default (`models::select`). Not a pin: the proxy serves any
    /// of `route_models` as named. Set with `route_port`. Additive: an
    /// older launcher ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_model: Option<String>,
    /// NEAR AI's chat models, in its order, as that proxy read them: the
    /// ids the launcher offers in the tool's own model picker. Set with
    /// `route_port`. Additive: an older launcher ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_models: Option<Vec<super::models::ChatModel>>,
    /// The root row whose conversation this session continues
    /// ([`LaunchRequest::resume_of`]). The launcher derives the tool's own
    /// conversation id from it. Additive: absent for a fresh conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_of: Option<Uuid>,
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
    /// The default model of the launcher's own proxy, for a session on a saved
    /// NEAR AI key. Additive: absent from every other report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub collection: Option<CollectionState>,
    pub session_id: Uuid,
    pub proof: String,
    pub state: SessionState,
    pub process_id: Option<u32>,
    pub process_identity: Option<String>,
    pub exit_code: Option<i32>,
}

/// A handle this build cannot read (a terminal it does not script, or a field
/// a newer build added) reads as none: that session cannot be shown, and the
/// registry still opens.
fn known_terminal<'de, D>(d: D) -> Result<Option<TerminalHandle>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    cli_options_text: Option<String>,
    #[serde(default)]
    cli_options: Vec<String>,
    #[serde(default)]
    dismissed: bool,
    /// The daemon opened this session's terminal itself, from its own
    /// ticket check. A launch from the command line never sets it.
    #[serde(default)]
    launched: bool,
    /// The window that launch opened, when the terminal could name it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "known_terminal"
    )]
    terminal: Option<TerminalHandle>,
    /// A stop was asked for; the launcher is told on its next report.
    #[serde(default)]
    stop_requested: bool,
    view: SessionView,
    request: LaunchRequest,
    /// `request.keep_saved_model`, stored here and copied back on open.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    keep_saved_model: bool,
    /// `request.resume_of`, stored here and copied back on open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resume_of: Option<Uuid>,
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
    launched: bool,
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
            // A row with a collection plan is still collected by the
            // daemon once its tool is gone (`collection_plan`).
            if entry.view.state == SessionState::Failed
                && entry.view.collection == CollectionState::Pending
                && !super::collection_plan::exists(store, entry.view.id)
            {
                entry.view.collection = CollectionState::Failed;
            }
            entry.ticket_hash = None;
            entry.request.keep_saved_model = entry.keep_saved_model;
            entry.request.resume_of = entry.resume_of;
        }
        let launched = store
            .read_daemon_file(FIRST_LAUNCH_FILE)
            .map_err(|_| ManagedError::StorageUnavailable)?
            .is_some();
        let mut registry = Self {
            store: ConfigStore::open(store.dir().to_path_buf())
                .map_err(|_| ManagedError::StorageUnavailable)?,
            data,
            launched,
        };
        registry.commit(registry.data.clone())?;
        registry.reconcile_receipts()?;
        // A session started before the fact was recorded still counts.
        if !registry.launched
            && registry
                .data
                .entries
                .iter()
                .any(|e| e.request.purpose == LaunchPurpose::Coding && e.proof_hash.is_some())
        {
            registry.record_launch()?;
        }
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

    /// Whether a coding session has ever started on this computer: a
    /// redeemed launch, from the app or the command line. A sign-in launch
    /// does not count; it never asked for a folder.
    pub fn has_launched(&self) -> bool {
        self.launched
    }

    fn record_launch(&mut self) -> Result<(), ManagedError> {
        self.store
            .write_daemon_file(FIRST_LAUNCH_FILE, br#"{"schema_version":1}"#)
            .map_err(|_| ManagedError::StorageUnavailable)?;
        self.launched = true;
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
                view.can_resume = false;
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
        // A fallback launch's restart runs the same fallback, so it keeps
        // the folder's saved model too. Starting a resumed row again starts
        // a fresh conversation: only [`Self::resume_request`] continues one.
        Ok(LaunchRequest {
            request_id,
            save_default: false,
            resume_of: None,
            ..entry.request.clone()
        })
    }

    /// The launch request that continues an ended coding session's
    /// conversation, under a new request id: the same launch, with
    /// `resume_of` naming the conversation's root row. Trace collection is
    /// the entry's own choice, never turned on: that would upload earlier
    /// turns the person did not agree to share. Its account and folder are
    /// checked by the caller.
    pub fn resume_request(
        &self,
        id: Uuid,
        request_id: Uuid,
    ) -> Result<LaunchRequest, ManagedError> {
        let again = self.start_again_request(id, request_id)?;
        let entry = self
            .data
            .entries
            .iter()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        let resumed = LaunchRequest {
            resume_of: Some(entry.request.resume_of.unwrap_or(id)),
            ..again
        };
        // A retry of a resume already prepared is that same session, even
        // while it runs; any other resume waits until the conversation is
        // free.
        if self.replayed(&resumed).is_none() && self.resume_root(id).is_none() {
            return Err(ManagedError::Conflict);
        }
        Ok(resumed)
    }

    /// The root row of `id`'s conversation: `id` itself, or the row it
    /// resumed. None when either row is unknown, the root never started,
    /// so it has no conversation to continue, or another row on the same
    /// conversation still holds its profile: two tools writing one
    /// conversation would interleave its transcript.
    pub fn resume_root(&self, id: Uuid) -> Option<Uuid> {
        let entry = self.data.entries.iter().find(|e| e.view.id == id)?;
        let root = entry.request.resume_of.unwrap_or(id);
        (self
            .data
            .entries
            .iter()
            .any(|e| e.view.id == root && e.proof_hash.is_some())
            && !self.conversation_in_use(root))
        .then_some(root)
    }

    /// Whether a row on the conversation `root` started (the root itself,
    /// or a resume of it) still holds its profile.
    fn conversation_in_use(&self, root: Uuid) -> bool {
        self.data.entries.iter().any(|e| {
            e.request.resume_of.unwrap_or(e.view.id) == root && e.view.state.holds_profile()
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

    /// Whether [`Self::prepare`] would accept `request` and record a new
    /// session for it, rather than refuse it or answer it as a replay.
    /// Commits nothing.
    pub fn would_prepare_new(&self, mut request: LaunchRequest, account: &AccountView) -> bool {
        let Ok(cwd) = super::folder::canonical(&request.cwd) else {
            return false;
        };
        request.cwd = cwd;
        matches!(self.check(&request, account), Ok(Checked::New { .. }))
    }

    pub fn prepare(
        &mut self,
        mut request: LaunchRequest,
        account: &AccountView,
    ) -> Result<PreparedLaunch, ManagedError> {
        request.cwd = super::folder::canonical(&request.cwd)?;
        let (cli_options_text, cli_options) = match self.check(&request, account)? {
            Checked::Replayed(existing) => return Ok(existing),
            Checked::New {
                cli_options_text,
                cli_options,
            } => (cli_options_text, cli_options),
        };
        let now = Utc::now();
        let ticket = secret();
        let id = Uuid::new_v4();
        let view = SessionView {
            model: request.model.clone(),
            // A resumed session's request carries its root's choice
            // (`resume_request` copies it, and a prepare naming
            // `resume_of` is refused), so a resume never turns collection
            // on; one that collects stages the root's continued transcript.
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
            can_resume: false,
            resumed_from: request.resume_of.and_then(|root| {
                self.data
                    .entries
                    .iter()
                    .find(|e| e.view.id == root)
                    .map(|e| e.view.started_at)
            }),
        };
        let entry = Entry {
            cli_options_text,
            cli_options,
            dismissed: false,
            launched: false,
            terminal: None,
            stop_requested: false,
            view,
            keep_saved_model: request.keep_saved_model,
            resume_of: request.resume_of,
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

    /// Every check [`Self::prepare`] makes on an already-canonical
    /// `request`, and the CLI options a new session would carry.
    fn check(
        &self,
        request: &LaunchRequest,
        account: &AccountView,
    ) -> Result<Checked, ManagedError> {
        let named = request
            .model
            .as_deref()
            .is_some_and(super::opencode::valid_model);
        if request.tool == ToolId::Opencode {
            // The NEAR AI sign-in can choose a model or use its proxy's
            // default. A saved key needs a model OpenCode can be told.
            let model_ok = match request.connection {
                ConnectionKind::Subscription => false,
                ConnectionKind::NearAiSignIn => request.model.is_none() || named,
                ConnectionKind::ApiKey | ConnectionKind::NearAi => named,
            };
            if request.purpose != LaunchPurpose::Coding || !model_ok {
                return Err(ManagedError::InvalidRequest);
            }
        } else {
            // A model can be named only where a NEAR AI proxy serves it; its
            // availability is checked when that proxy starts.
            let model_ok = request.model.is_none()
                || (named
                    && request.purpose == LaunchPurpose::Coding
                    && matches!(
                        request.connection,
                        ConnectionKind::NearAi | ConnectionKind::NearAiSignIn
                    ));
            if !model_ok || (request.collect_traces && request.purpose != LaunchPurpose::Coding) {
                return Err(ManagedError::InvalidRequest);
            }
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
            if entry.request != *request {
                return Err(ManagedError::Conflict);
            }
            return Ok(Checked::Replayed(PreparedLaunch {
                session_id: entry.view.id,
                ticket: None,
            }));
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
        let cli_options_text = if request.purpose == LaunchPurpose::Coding {
            Some(match request.cli_options.clone() {
                Some(text) => text,
                None => super::folder::lookup(&self.store, request.tool, &request.cwd)?.cli_options,
            })
        } else {
            None
        };
        let cli_options = match cli_options_text.as_deref() {
            Some(text) => super::options::parse(request.tool, text)?,
            None => vec![],
        };
        Ok(Checked::New {
            cli_options_text,
            cli_options,
        })
    }

    /// A failed preference write must not strand a ticket the caller never saw.
    pub fn rollback_prepared(&mut self, id: Uuid) -> Result<(), ManagedError> {
        let entry = self
            .data
            .entries
            .iter()
            .find(|entry| entry.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        if entry.view.state != SessionState::Starting
            || entry.proof_hash.is_some()
            || entry.launched
        {
            return Err(ManagedError::Conflict);
        }
        let mut next = self.data.clone();
        next.entries.retain(|entry| entry.view.id != id);
        self.commit(next)
    }

    pub fn folder_settings(&self, id: Uuid) -> Option<super::folder::FolderSettings> {
        let entry = self.data.entries.iter().find(|entry| entry.view.id == id)?;
        if entry.request.purpose != LaunchPurpose::Coding {
            return None;
        }
        Some(super::folder::FolderSettings {
            tool: entry.request.tool,
            cwd: entry.request.cwd.clone(),
            model: entry.request.model.clone(),
            collect_traces: entry.request.collect_traces,
            cli_options: entry.cli_options_text.clone()?,
        })
    }

    /// Whether the launch asked to keep the folder's saved model.
    pub fn keeps_saved_model(&self, id: Uuid) -> bool {
        self.data
            .entries
            .iter()
            .any(|entry| entry.view.id == id && entry.request.keep_saved_model)
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
            || entry.launched
        {
            return Err(ManagedError::Conflict);
        }
        Ok(())
    }

    /// The folder a session with a valid ticket starts in, for a terminal
    /// that takes a working directory. Validates the ticket as
    /// [`Self::validate_ticket`] does.
    pub fn launch_cwd(&self, session_id: Uuid, ticket: &str) -> Result<PathBuf, ManagedError> {
        self.validate_ticket(session_id, ticket)?;
        self.data
            .entries
            .iter()
            .find(|e| e.view.id == session_id)
            .map(|e| e.view.cwd.clone())
            .ok_or(ManagedError::NotFound)
    }

    /// The session an earlier start-again with this request id made, when
    /// `request` is that same request apart from the selection generation it
    /// was stamped with. A retry then finds its session, whatever the
    /// selection has done since.
    pub fn replayed(&self, request: &LaunchRequest) -> Option<PreparedLaunch> {
        let entry = self
            .data
            .entries
            .iter()
            .find(|e| e.request.request_id == request.request_id)?;
        let same = LaunchRequest {
            expected_generation: entry.request.expected_generation,
            ..request.clone()
        } == entry.request;
        same.then_some(PreparedLaunch {
            session_id: entry.view.id,
            ticket: None,
        })
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
        let cli_options = entry.cli_options.clone();
        let session = entry.view.clone();
        let coding = entry.request.purpose == LaunchPurpose::Coding;
        let resume_of = entry.request.resume_of;
        self.commit(next)?;
        // The launch has happened; failing to record it only shows the
        // first-launch explanation again, so it never fails the launch.
        if coding && !self.launched {
            let _ = self.record_launch();
        }
        Ok(LaunchContext {
            cli_options,
            session,
            proof,
            route_port: None,
            route_model: None,
            route_models: None,
            resume_of,
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
            if !(ended_by_daemon
                || (report.state == entry.view.state && report.exit_code == entry.view.exit_code))
            {
                return Err(ManagedError::Conflict);
            }
            // Its collection result is still taken while the row waits for
            // one: the launcher collected, and removed the collection plan
            // the daemon would otherwise have collected from.
            return match report.collection {
                Some(collection @ (CollectionState::Collected | CollectionState::Failed))
                    if entry.view.collect_traces
                        && entry.view.collection == CollectionState::Pending =>
                {
                    entry.view.collection = collection;
                    self.commit(next)
                }
                _ => Ok(()),
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
        if let Some(model) = report.model {
            if entry.view.connection != ConnectionKind::NearAi
                || !super::opencode::valid_model(&model)
            {
                return Err(ManagedError::InvalidRequest);
            }
            if entry.view.model.as_ref().is_some_and(|m| *m != model) {
                return Err(ManagedError::Conflict);
            }
            entry.view.model = Some(model);
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

    /// What a collection plan's decision needs from row `id`.
    pub fn collection_status(&self, id: Uuid) -> Option<super::collection_plan::RowStatus> {
        self.data.entries.iter().find(|e| e.view.id == id).map(|e| {
            super::collection_plan::RowStatus {
                state: e.view.state,
                collect_traces: e.view.collect_traces,
                collection: e.view.collection,
                tool: e.view.tool,
                account_id: e.view.account_id,
                resume_of: e.request.resume_of,
            }
        })
    }

    /// Record the result of collecting row `id` from its collection plan,
    /// after its launcher went quiet and its tool exited. Only `Collected`
    /// or `Failed`, only for a row that collects and is still pending, and
    /// never while its launcher reports (the launcher collects then). The
    /// row's state and exit code stay the launcher's: a launcher that comes
    /// back still reports how the tool ended, and its report leaves this
    /// result alone unless it carries one of its own.
    pub fn record_collection(
        &mut self,
        id: Uuid,
        collection: CollectionState,
    ) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        if !entry.view.collect_traces
            || !matches!(
                collection,
                CollectionState::Collected | CollectionState::Failed
            )
        {
            return Err(ManagedError::InvalidRequest);
        }
        if matches!(
            entry.view.state,
            SessionState::Starting | SessionState::Running
        ) || entry.view.collection != CollectionState::Pending
        {
            return Err(ManagedError::Conflict);
        }
        entry.view.collection = collection;
        self.commit(next)
    }

    /// Record the default model of a daemon-hosted proxy for a session on the
    /// NEAR AI sign-in, so its row can show it. A session that asked for a
    /// model keeps it: a different one is a conflict.
    pub fn record_model(&mut self, session_id: Uuid, model: &str) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == session_id)
            .ok_or(ManagedError::NotFound)?;
        if entry.view.connection != ConnectionKind::NearAiSignIn
            || !super::opencode::valid_model(model)
        {
            return Err(ManagedError::InvalidRequest);
        }
        match entry.view.model.as_deref() {
            Some(existing) if existing == model => return Ok(()),
            Some(_) => return Err(ManagedError::Conflict),
            None => entry.view.model = Some(model.to_owned()),
        }
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

    /// #1754 review: a fallback launch's `keep_saved_model` is kept on the
    /// registry entry, not in its stored request, so a daemon that predates
    /// the field (its `LaunchRequest` refuses unknown fields) can still read
    /// the registry. It survives a restart, a retry of the same request is
    /// still one session, and Start again keeps it.
    #[test]
    fn keep_saved_model_survives_a_restart_and_start_again() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let sign_in = crate::managed::signin::entry(ToolId::Opencode).unwrap();
        let request = LaunchRequest {
            cli_options: None,
            model: None,
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool: ToolId::Opencode,
            connection: ConnectionKind::NearAiSignIn,
            account_id: sign_in.id,
            cwd: home.path().to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
            keep_saved_model: true,
            resume_of: None,
        };
        let id = registry
            .prepare(request.clone(), &sign_in)
            .unwrap()
            .session_id;
        assert!(registry.keeps_saved_model(id));

        let raw: serde_json::Value =
            serde_json::from_slice(&store.read_daemon_file(FILE).unwrap().unwrap()).unwrap();
        let entry = &raw["entries"][0];
        assert!(
            entry["request"].get("keep_saved_model").is_none(),
            "{entry}"
        );
        assert_eq!(entry["keep_saved_model"], true);

        // A restart fails the unredeemed launch, which ends it.
        let mut registry = SessionRegistry::open(&store).unwrap();
        assert!(registry.keeps_saved_model(id));
        assert_eq!(
            registry.prepare(request, &sign_in).unwrap().session_id,
            id,
            "a retry of the same request is the same session"
        );
        let again = registry.start_again_request(id, Uuid::new_v4()).unwrap();
        assert!(again.keep_saved_model, "Start again keeps the saved model");
        assert_eq!(again.model, None);
        // A launch without the flag stores none.
        let other = registry
            .prepare(
                LaunchRequest {
                    keep_saved_model: false,
                    ..again
                },
                &sign_in,
            )
            .unwrap()
            .session_id;
        assert!(!registry.keeps_saved_model(other));
        let raw: serde_json::Value =
            serde_json::from_slice(&store.read_daemon_file(FILE).unwrap().unwrap()).unwrap();
        assert!(raw["entries"][1].get("keep_saved_model").is_none());
    }

    /// OpenCode can use the sign-in proxy's default or select a valid model.
    /// A saved key still needs a model; subscriptions remain unavailable.
    #[test]
    fn opencode_sign_in_can_choose_a_model_and_a_key_still_needs_one() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let request = |connection, account_id, model: Option<&str>| LaunchRequest {
            cli_options: None,
            model: model.map(str::to_owned),
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool: ToolId::Opencode,
            connection,
            account_id,
            cwd: home.path().to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
            keep_saved_model: false,
            resume_of: None,
        };
        let sign_in = crate::managed::signin::entry(ToolId::Opencode).unwrap();
        assert_eq!(
            registry
                .prepare(
                    request(ConnectionKind::NearAiSignIn, sign_in.id, Some("bad model")),
                    &sign_in
                )
                .err(),
            Some(ManagedError::InvalidRequest)
        );
        let prepared = registry
            .prepare(
                request(ConnectionKind::NearAiSignIn, sign_in.id, None),
                &sign_in,
            )
            .unwrap();
        assert!(prepared.ticket.is_some());

        let mut accounts = AccountStore::open(&store).unwrap();
        let key = accounts
            .add(ToolId::Opencode, ConnectionKind::NearAi, "Work")
            .unwrap();
        for model in [None, Some(""), Some("bad model")] {
            assert_eq!(
                registry
                    .prepare(request(ConnectionKind::NearAi, key.id, model), &key)
                    .err(),
                Some(ManagedError::InvalidRequest),
                "{model:?}"
            );
        }
        assert!(
            registry
                .prepare(
                    request(
                        ConnectionKind::NearAi,
                        key.id,
                        Some("deepseek-ai/DeepSeek-V3.1")
                    ),
                    &key
                )
                .is_ok()
        );
        let mut subscription = key.clone();
        subscription.connection = ConnectionKind::Subscription;
        assert_eq!(
            registry
                .prepare(
                    request(ConnectionKind::Subscription, key.id, Some("gpt-5")),
                    &subscription
                )
                .err(),
            Some(ManagedError::InvalidRequest)
        );
    }

    fn coding_with(
        tool: ToolId,
        account: &AccountView,
        cwd: &std::path::Path,
        model: Option<&str>,
    ) -> LaunchRequest {
        LaunchRequest {
            cli_options: None,
            model: model.map(str::to_owned),
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool,
            connection: account.connection,
            account_id: account.id,
            cwd: cwd.to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
            keep_saved_model: false,
            resume_of: None,
        }
    }

    /// Claude Code and Codex may name a model only where a NEAR AI proxy
    /// pins it: on the sign-in or a saved NEAR AI key. Naming none is
    /// always allowed and means the tool's default.
    #[test]
    fn claude_and_codex_name_a_model_only_on_near_ai() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let mut accounts = AccountStore::open(&store).unwrap();
        for tool in [ToolId::Claude, ToolId::Codex] {
            let sign_in = crate::managed::signin::entry(tool).unwrap();
            let near_ai = accounts.add(tool, ConnectionKind::NearAi, "NEAR").unwrap();
            let subscription = accounts
                .add(tool, ConnectionKind::Subscription, "Plan")
                .unwrap();
            let api_key = accounts.add(tool, ConnectionKind::ApiKey, "Key").unwrap();
            for account in [&subscription, &api_key] {
                assert_eq!(
                    registry
                        .prepare(
                            coding_with(tool, account, home.path(), Some("openai/gpt-5.5")),
                            account
                        )
                        .err(),
                    Some(ManagedError::InvalidRequest),
                    "{tool:?} {:?}",
                    account.connection
                );
                assert!(
                    registry
                        .prepare(coding_with(tool, account, home.path(), None), account)
                        .is_ok()
                );
            }
            for account in [&sign_in, &near_ai] {
                for bad in ["", "bad model"] {
                    assert_eq!(
                        registry
                            .prepare(coding_with(tool, account, home.path(), Some(bad)), account)
                            .err(),
                        Some(ManagedError::InvalidRequest),
                        "{tool:?} {bad:?}"
                    );
                }
                for model in [Some("openai/gpt-5.5"), None] {
                    let prepared = registry
                        .prepare(coding_with(tool, account, home.path(), model), account)
                        .unwrap();
                    let view = registry
                        .views()
                        .unwrap()
                        .into_iter()
                        .find(|v| v.id == prepared.session_id)
                        .unwrap();
                    assert_eq!(view.model.as_deref(), model);
                }
            }
        }
    }

    /// The row says which default model the proxy chose: the daemon records it
    /// for a sign-in session, the launcher reports it for a saved NEAR AI
    /// key. A model the request named is never replaced.
    #[test]
    fn the_default_model_is_recorded_on_the_row() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let model_of = |registry: &mut SessionRegistry, id| {
            registry
                .views()
                .unwrap()
                .into_iter()
                .find(|v| v.id == id)
                .unwrap()
                .model
        };
        let sign_in = crate::managed::signin::entry(ToolId::Claude).unwrap();
        let default = registry
            .prepare(
                coding_with(ToolId::Claude, &sign_in, home.path(), None),
                &sign_in,
            )
            .unwrap()
            .session_id;
        assert_eq!(model_of(&mut registry, default), None);
        registry
            .record_model(default, "anthropic/claude-sonnet-5-5")
            .unwrap();
        assert_eq!(
            model_of(&mut registry, default).as_deref(),
            Some("anthropic/claude-sonnet-5-5")
        );
        registry
            .record_model(default, "anthropic/claude-sonnet-5-5")
            .unwrap();
        assert_eq!(
            registry.record_model(default, "openai/gpt-5.5"),
            Err(ManagedError::Conflict)
        );
        assert_eq!(
            registry.record_model(default, "bad model"),
            Err(ManagedError::InvalidRequest)
        );
        let named = registry
            .prepare(
                coding_with(
                    ToolId::Claude,
                    &sign_in,
                    home.path(),
                    Some("openai/gpt-5.5"),
                ),
                &sign_in,
            )
            .unwrap()
            .session_id;
        assert_eq!(
            registry.record_model(named, "anthropic/claude-sonnet-5-5"),
            Err(ManagedError::Conflict)
        );

        let mut accounts = AccountStore::open(&store).unwrap();
        let near_ai = accounts
            .add(ToolId::Codex, ConnectionKind::NearAi, "NEAR")
            .unwrap();
        let prepared = registry
            .prepare(
                coding_with(ToolId::Codex, &near_ai, home.path(), None),
                &near_ai,
            )
            .unwrap();
        let id = prepared.session_id;
        // Only a daemon-hosted proxy is recorded by the daemon.
        assert_eq!(
            registry.record_model(id, "openai/gpt-6.1-sol"),
            Err(ManagedError::InvalidRequest)
        );
        let context = registry
            .redeem(id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        let running = |model: Option<&str>| LifecycleReport {
            model: model.map(str::to_owned),
            collection: None,
            session_id: id,
            proof: context.proof.clone(),
            state: SessionState::Running,
            process_id: Some(42),
            process_identity: Some("42:1".into()),
            exit_code: None,
        };
        registry
            .report(running(Some("openai/gpt-6.1-sol")))
            .unwrap();
        assert_eq!(
            model_of(&mut registry, id).as_deref(),
            Some("openai/gpt-6.1-sol")
        );
        // Later reports without the field leave it; a different one is refused.
        registry.report(running(None)).unwrap();
        assert_eq!(
            registry.report(running(Some("openai/gpt-5.5"))),
            Err(ManagedError::Conflict)
        );
        assert_eq!(
            model_of(&mut registry, id).as_deref(),
            Some("openai/gpt-6.1-sol")
        );
    }

    #[test]
    fn a_registry_holding_a_terminal_handle_this_build_does_not_know_still_opens() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut accounts = AccountStore::open(&store).unwrap();
        let account = accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let request = LaunchRequest {
            cli_options: None,
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
            keep_saved_model: false,
            resume_of: None,
        };
        let launch = registry.prepare(request, &account).unwrap();
        let mut value = serde_json::to_value(&registry.data).unwrap();
        for unknown in [
            serde_json::json!({"kind":"ghostty","tty":"/dev/ttys007"}),
            serde_json::json!({"kind":"apple_terminal","tty":"/dev/ttys007","extra":"x"}),
            serde_json::json!("garbage"),
        ] {
            value["entries"][0]["terminal"] = unknown;
            let parsed: Data = serde_json::from_value(value.clone()).unwrap();
            assert!(parsed.entries[0].terminal.is_none());
            assert_eq!(parsed.entries[0].view.id, launch.session_id);
        }
    }

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
            cli_options: None,
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
            keep_saved_model: false,
            resume_of: None,
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

    fn coding(account: &AccountView, cwd: &std::path::Path) -> LaunchRequest {
        LaunchRequest {
            cli_options: None,
            model: None,
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool: account.tool,
            connection: account.connection,
            account_id: account.id,
            cwd: cwd.to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
            keep_saved_model: false,
            resume_of: None,
        }
    }

    /// #1408: the first started coding session is recorded once, on this
    /// computer, whichever caller redeemed it, and a restart keeps it. A
    /// prepared launch that never started, and a sign-in launch, do not
    /// count.
    #[test]
    fn the_first_started_coding_session_is_recorded_and_survives_a_restart() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut accounts = AccountStore::open(&store).unwrap();
        let account = accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        assert!(!registry.has_launched());

        let login = registry
            .prepare(
                LaunchRequest {
                    purpose: LaunchPurpose::Login,
                    ..coding(&account, home.path())
                },
                &account,
            )
            .unwrap();
        registry
            .redeem(login.session_id, login.ticket.as_deref().unwrap())
            .unwrap();
        assert!(
            !registry.has_launched(),
            "a sign-in is not a coding session"
        );

        let other = accounts
            .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
            .unwrap();
        let prepared = registry
            .prepare(coding(&other, home.path()), &other)
            .unwrap();
        assert!(!registry.has_launched(), "prepared is not started");
        assert!(!SessionRegistry::open(&store).unwrap().has_launched());

        registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert!(registry.has_launched());
        assert!(SessionRegistry::open(&store).unwrap().has_launched());
    }

    /// A computer that started a session before the fact was recorded is
    /// not shown the first-launch explanation again.
    #[test]
    fn a_session_started_before_the_record_existed_counts() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut accounts = AccountStore::open(&store).unwrap();
        let account = accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let prepared = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        std::fs::remove_file(store.dir().join(FIRST_LAUNCH_FILE)).unwrap();
        assert!(SessionRegistry::open(&store).unwrap().has_launched());
        assert!(store.dir().join(FIRST_LAUNCH_FILE).is_file());
    }
    /// Prepare, start and end a coding session, collecting when it asked
    /// to. Returns its id.
    fn ended(
        registry: &mut SessionRegistry,
        request: LaunchRequest,
        account: &AccountView,
    ) -> Uuid {
        let prepared = registry.prepare(request, account).unwrap();
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        registry
            .report(LifecycleReport {
                model: None,
                collection: context
                    .session
                    .collect_traces
                    .then_some(CollectionState::Collected),
                session_id: prepared.session_id,
                proof: context.proof,
                state: SessionState::Exited,
                process_id: None,
                process_identity: None,
                exit_code: Some(0),
            })
            .unwrap();
        prepared.session_id
    }

    /// Resume continues the root row's conversation: the request names the
    /// root, and resuming a resumed row names the same root, so the native
    /// id never drifts. The root survives a restart, the launcher is told
    /// it, and a retry of the same request is the same session.
    #[test]
    fn resume_request_names_the_root_and_a_chain_keeps_it() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let original = coding(&account, home.path());
        let root = ended(&mut registry, original.clone(), &account);

        let request_id = Uuid::new_v4();
        let first = registry.resume_request(root, request_id).unwrap();
        assert_eq!(first.resume_of, Some(root));
        assert_eq!(first.request_id, request_id);
        assert!(!first.save_default);
        assert_eq!(
            LaunchRequest {
                request_id: original.request_id,
                resume_of: None,
                ..first.clone()
            },
            LaunchRequest {
                cwd: crate::managed::folder::canonical(&original.cwd).unwrap(),
                ..original
            },
            "the same launch, continuing the root"
        );

        let prepared = registry.prepare(first.clone(), &account).unwrap();
        assert_eq!(
            registry
                .prepare(first.clone(), &account)
                .unwrap()
                .session_id,
            prepared.session_id,
            "a retry of the same request is the same session"
        );
        assert_eq!(
            registry.replayed(&first).unwrap().session_id,
            prepared.session_id
        );
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            context.resume_of,
            Some(root),
            "the launcher is told the root"
        );
        let resumed = prepared.session_id;
        assert_ne!(resumed, root, "a resumed launch is a new row");
        registry
            .report(LifecycleReport {
                model: None,
                collection: None,
                session_id: resumed,
                proof: context.proof,
                state: SessionState::Exited,
                process_id: None,
                process_identity: None,
                exit_code: Some(0),
            })
            .unwrap();

        let chained = registry.resume_request(resumed, Uuid::new_v4()).unwrap();
        assert_eq!(chained.resume_of, Some(root), "a chain points at the root");

        // Stored on the entry, not in the request, so a daemon that
        // predates the field still opens the registry.
        let raw: serde_json::Value =
            serde_json::from_slice(&store.read_daemon_file(FILE).unwrap().unwrap()).unwrap();
        let entry = &raw["entries"][1];
        assert!(entry["request"].get("resume_of").is_none(), "{entry}");
        assert_eq!(entry["resume_of"], serde_json::json!(root));
        assert!(raw["entries"][0].get("resume_of").is_none());

        let reopened = SessionRegistry::open(&store).unwrap();
        assert_eq!(
            reopened
                .resume_request(resumed, Uuid::new_v4())
                .unwrap()
                .resume_of,
            Some(root)
        );
        assert_eq!(reopened.resume_root(resumed), Some(root));
        assert_eq!(reopened.resume_root(root), Some(root));
    }

    /// Start again on a resumed row starts a fresh conversation; only
    /// Resume continues the root's.
    #[test]
    fn starting_a_resumed_row_again_starts_a_fresh_conversation() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let resume = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let resumed = ended(&mut registry, resume, &account);

        let again = registry
            .start_again_request(resumed, Uuid::new_v4())
            .unwrap();
        assert_eq!(again.resume_of, None, "a fresh conversation");
        let prepared = registry.prepare(again, &account).unwrap();
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(context.resume_of, None);
        assert_eq!(
            registry
                .views()
                .unwrap()
                .iter()
                .find(|v| v.id == prepared.session_id)
                .unwrap()
                .resumed_from,
            None
        );
        // Resume on the same row still continues the root.
        assert_eq!(
            registry
                .resume_request(resumed, Uuid::new_v4())
                .unwrap()
                .resume_of,
            Some(root)
        );
    }

    /// One conversation runs in one session at a time: while a row on it
    /// holds its profile, no other resume of it starts, from the root or
    /// from any row of its chain. A retry of the running resume is still
    /// that same session.
    #[test]
    fn a_conversation_is_resumed_by_one_session_at_a_time() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let earlier = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let chained = ended(&mut registry, earlier, &account);

        let first = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let prepared = registry.prepare(first.clone(), &account).unwrap();
        for id in [root, chained] {
            assert_eq!(registry.resume_root(id), None, "starting");
            assert_eq!(
                registry.resume_request(id, Uuid::new_v4()),
                Err(ManagedError::Conflict)
            );
        }
        assert_eq!(
            registry.resume_request(root, first.request_id),
            Ok(first.clone()),
            "a retry is the same request"
        );
        assert_eq!(
            registry.replayed(&first).unwrap().session_id,
            prepared.session_id
        );
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        let report = |state, exit_code: Option<i32>| LifecycleReport {
            model: None,
            collection: None,
            session_id: prepared.session_id,
            proof: context.proof.clone(),
            state,
            process_id: Some(42),
            process_identity: Some("42:1".to_owned()),
            exit_code,
        };
        registry
            .report(report(SessionState::Running, None))
            .unwrap();
        assert_eq!(
            registry.resume_request(chained, Uuid::new_v4()),
            Err(ManagedError::Conflict),
            "running"
        );
        // Starting the root again is a fresh conversation, so it may run.
        assert!(registry.start_again_request(root, Uuid::new_v4()).is_ok());
        registry
            .report(report(SessionState::Exited, Some(0)))
            .unwrap();
        for id in [root, chained, prepared.session_id] {
            assert_eq!(registry.resume_root(id), Some(root));
            assert!(registry.resume_request(id, Uuid::new_v4()).is_ok());
        }
    }

    /// A resumed row says when its conversation's root row started; a
    /// fresh row says nothing; a chain points at the root; and the marker
    /// survives the root being dismissed and a restart.
    #[test]
    fn a_resumed_row_says_when_its_root_started() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let views = registry.views().unwrap();
        assert_eq!(views[0].resumed_from, None, "a fresh row");
        let root_started = views[0].started_at;

        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let resumed = ended(&mut registry, request, &account);
        let request = registry.resume_request(resumed, Uuid::new_v4()).unwrap();
        let chained = ended(&mut registry, request, &account);

        registry.dismiss(root).unwrap();
        let reopened = SessionRegistry::open(&store).unwrap();
        let mut reopened = reopened;
        let views = reopened.views().unwrap();
        let find = |id: Uuid| views.iter().find(|v| v.id == id).unwrap();
        assert!(views.iter().all(|v| v.id != root), "the root is dismissed");
        assert_eq!(find(resumed).resumed_from, Some(root_started));
        assert_eq!(find(chained).resumed_from, Some(root_started));
        let json = serde_json::to_value(find(resumed)).unwrap();
        assert!(json["resumed_from"].is_string(), "{json}");
    }

    /// A row that still holds its profile, a sign-in row, and a row whose
    /// tool never started (no conversation exists) cannot be resumed.
    #[test]
    fn resume_is_refused_while_running_for_a_login_and_without_a_conversation() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut accounts = AccountStore::open(&store).unwrap();
        let account = accounts
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();

        let running = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        assert_eq!(
            registry
                .resume_request(running.session_id, Uuid::new_v4())
                .err(),
            Some(ManagedError::Conflict)
        );
        registry
            .fail_unredeemed(running.session_id, running.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(registry.resume_root(running.session_id), None);
        assert_eq!(
            registry
                .resume_request(running.session_id, Uuid::new_v4())
                .err(),
            Some(ManagedError::Conflict),
            "a session that never started has no conversation"
        );

        let other = accounts
            .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
            .unwrap();
        let login = ended(
            &mut registry,
            LaunchRequest {
                purpose: LaunchPurpose::Login,
                ..coding(&other, home.path())
            },
            &other,
        );
        assert_eq!(
            registry.resume_request(login, Uuid::new_v4()).err(),
            Some(ManagedError::InvalidRequest)
        );
        assert_eq!(
            registry
                .resume_request(Uuid::new_v4(), Uuid::new_v4())
                .err(),
            Some(ManagedError::NotFound)
        );
    }

    /// A resumed row inherits the root's collection choice and can never
    /// turn it on: earlier turns would be uploaded without agreement. A
    /// root that collected gives a resumed row that collects, reports its
    /// own result, and passes the choice down a chain; one that did not
    /// gives a row whose collection is off.
    #[test]
    fn resume_inherits_collect_traces_and_cannot_turn_it_on() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();

        let off = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(off, Uuid::new_v4()).unwrap();
        assert!(!request.collect_traces);
        let prepared = registry.prepare(request, &account).unwrap();
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert!(!context.session.collect_traces, "never turned on");
        assert_eq!(context.session.collection, CollectionState::Off);
        registry
            .report(LifecycleReport {
                model: None,
                collection: None,
                session_id: prepared.session_id,
                proof: context.proof,
                state: SessionState::Exited,
                process_id: None,
                process_identity: None,
                exit_code: Some(0),
            })
            .unwrap();

        let on = ended(
            &mut registry,
            LaunchRequest {
                collect_traces: true,
                ..coding(&account, home.path())
            },
            &account,
        );
        let request = registry.resume_request(on, Uuid::new_v4()).unwrap();
        assert!(request.collect_traces, "the root's choice is kept");
        let prepared = registry.prepare(request, &account).unwrap();
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert!(context.session.collect_traces, "inherited from the root");
        assert_eq!(context.session.collection, CollectionState::Pending);
        registry
            .report(LifecycleReport {
                model: None,
                collection: Some(CollectionState::Collected),
                session_id: prepared.session_id,
                proof: context.proof,
                state: SessionState::Exited,
                process_id: None,
                process_identity: None,
                exit_code: Some(0),
            })
            .unwrap();
        assert_eq!(
            registry
                .data
                .entries
                .iter()
                .find(|e| e.view.id == prepared.session_id)
                .unwrap()
                .view
                .collection,
            CollectionState::Collected,
            "a resumed row reports its own collection"
        );
        assert!(
            registry
                .resume_request(prepared.session_id, Uuid::new_v4())
                .unwrap()
                .collect_traces,
            "a chain keeps the root's choice"
        );
    }

    /// The daemon records a collection plan's result only for a quiet,
    /// collecting, still pending row, and leaves the row's state to its
    /// launcher, whose later report is still accepted. A plan keeps a
    /// failed row's collection pending across a restart.
    #[test]
    fn a_plans_collection_is_recorded_only_for_a_quiet_pending_row() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let prepared = registry
            .prepare(
                LaunchRequest {
                    collect_traces: true,
                    ..coding(&account, home.path())
                },
                &account,
            )
            .unwrap();
        let id = prepared.session_id;
        let context = registry
            .redeem(id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        let report = |state, exit_code, collection| LifecycleReport {
            model: None,
            collection,
            session_id: id,
            proof: context.proof.clone(),
            state,
            process_id: Some(42),
            process_identity: Some("42:1".into()),
            exit_code,
        };
        registry
            .report(report(SessionState::Running, None, None))
            .unwrap();
        assert_eq!(
            registry.record_collection(id, CollectionState::Collected),
            Err(ManagedError::Conflict),
            "never while the launcher reports"
        );
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        let status = registry.collection_status(id).unwrap();
        assert_eq!(status.state, SessionState::Unknown);
        assert_eq!(status.collection, CollectionState::Pending);
        assert_eq!(
            registry.record_collection(id, CollectionState::Pending),
            Err(ManagedError::InvalidRequest)
        );
        registry
            .record_collection(id, CollectionState::Collected)
            .unwrap();
        assert_eq!(
            registry.record_collection(id, CollectionState::Failed),
            Err(ManagedError::Conflict),
            "a result is final"
        );
        let status = registry.collection_status(id).unwrap();
        assert_eq!(
            status.state,
            SessionState::Unknown,
            "the state is the launcher's"
        );
        assert_eq!(status.collection, CollectionState::Collected);
        // A launcher that lost the claim comes back and says how the tool ended.
        registry
            .report(report(SessionState::Exited, Some(129), None))
            .unwrap();
        let status = registry.collection_status(id).unwrap();
        assert_eq!(status.state, SessionState::Exited);
        assert_eq!(status.collection, CollectionState::Collected);

        let off = ended(&mut registry, coding(&account, home.path()), &account);
        assert_eq!(
            registry.record_collection(off, CollectionState::Collected),
            Err(ManagedError::InvalidRequest),
            "never for a row that does not collect"
        );
        assert_eq!(
            registry.record_collection(Uuid::new_v4(), CollectionState::Collected),
            Err(ManagedError::NotFound)
        );

        // A sign-in row ends Failed when its daemon restarts. With a plan
        // its collection stays pending for the daemon; without, it fails.
        let sign_in = crate::managed::signin::entry(ToolId::Claude).unwrap();
        let started = |registry: &mut SessionRegistry| {
            let prepared = registry
                .prepare(
                    LaunchRequest {
                        collect_traces: true,
                        ..coding_with(ToolId::Claude, &sign_in, home.path(), None)
                    },
                    &sign_in,
                )
                .unwrap();
            registry
                .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
                .unwrap();
            prepared.session_id
        };
        let planned = started(&mut registry);
        let unplanned = started(&mut registry);
        crate::managed::collection_plan::write(
            &store,
            &crate::managed::collection_plan::CollectionPlan {
                schema_version: 1,
                session_id: planned,
                tool: ToolId::Claude,
                account_id: sign_in.id,
                resume_root: None,
                cwd: home.path().to_path_buf(),
                collect_traces: true,
                created_at: Utc::now(),
                native: None,
                opencode: None,
                process: crate::managed::collection_plan::ProcessIdentity {
                    pid: 1,
                    start_time: None,
                },
            },
        )
        .unwrap();
        drop(registry);
        let registry = SessionRegistry::open(&store).unwrap();
        let planned = registry.collection_status(planned).unwrap();
        assert_eq!(planned.state, SessionState::Failed);
        assert_eq!(planned.collection, CollectionState::Pending);
        let unplanned = registry.collection_status(unplanned).unwrap();
        assert_eq!(unplanned.state, SessionState::Failed);
        assert_eq!(unplanned.collection, CollectionState::Failed);
    }

    /// A sign-in row the daemon failed (its launcher went quiet) still
    /// takes the collection result its launcher reports when it comes
    /// back, while it waits for one; nothing else in that report changes it.
    #[test]
    fn a_late_launcher_still_reports_a_failed_sign_in_rows_collection() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let sign_in = crate::managed::signin::entry(ToolId::Claude).unwrap();
        let prepared = registry
            .prepare(
                LaunchRequest {
                    collect_traces: true,
                    ..coding_with(ToolId::Claude, &sign_in, home.path(), None)
                },
                &sign_in,
            )
            .unwrap();
        let id = prepared.session_id;
        let context = registry
            .redeem(id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        let status = registry.collection_status(id).unwrap();
        assert_eq!(status.state, SessionState::Failed);
        assert_eq!(status.collection, CollectionState::Pending);
        let exited = |collection| LifecycleReport {
            model: None,
            collection,
            session_id: id,
            proof: context.proof.clone(),
            state: SessionState::Exited,
            process_id: Some(42),
            process_identity: Some("42:1".into()),
            exit_code: Some(0),
        };
        registry.report(exited(None)).unwrap();
        assert_eq!(
            registry.collection_status(id).unwrap().collection,
            CollectionState::Pending
        );
        registry
            .report(exited(Some(CollectionState::Collected)))
            .unwrap();
        let status = registry.collection_status(id).unwrap();
        assert_eq!(status.state, SessionState::Failed, "the state stays");
        assert_eq!(status.collection, CollectionState::Collected);
        registry
            .report(exited(Some(CollectionState::Failed)))
            .unwrap();
        assert_eq!(
            registry.collection_status(id).unwrap().collection,
            CollectionState::Collected,
            "a result is final"
        );
    }
}
