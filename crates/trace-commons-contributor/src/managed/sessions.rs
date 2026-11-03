//! Durable lifecycle for managed native processes. A UI is only an observer.

use super::{
    AccountId, AccountView, ConnectionKind, Generation, ManagedError, ToolId,
    terminal::{Placement, TerminalHandle},
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
    /// The model for the tool's background work (Claude Code's small, fast
    /// model; OpenCode's title agent), or none for the tool's background
    /// default. Only a NEAR AI session of Claude Code or OpenCode takes
    /// one; the launcher replaces one less private than the main model
    /// (`models::background_for`). Read from a request, never written: the
    /// registry keeps it on the view, where a daemon that predates it skips
    /// it, and a shell sends it only when the snapshot says
    /// `capabilities.background_model`.
    #[serde(default, skip_serializing)]
    pub background_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    /// The model the session starts on: the one its request named, or, on
    /// NEAR AI, its proxy's default once that proxy has started. The tool
    /// may switch to another listed model later; that is not recorded.
    /// None for a session that names no model and has no proxy.
    #[serde(default)]
    pub model: Option<String>,
    /// The background model the request named, for the launcher; none for
    /// the tool's background default. Additive: absent when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_model: Option<String>,
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
    /// The row can be dismissed (with its conversation): no row of the
    /// conversation holds its profile, or the only ones that do are stuck
    /// unknown, their launcher gone before its first report, with no
    /// process to watch (owner, 2026-11-02). Computed on every read.
    /// Additive: absent from an older daemon.
    #[serde(default)]
    pub can_dismiss: bool,
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

/// A quiet row whose tool the daemon watches ([`SessionRegistry::quiet_tools`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietTool {
    pub id: Uuid,
    pub process: super::collection_plan::ProcessIdentity,
    /// The launcher's last report, which named this process while it ran.
    pub named_at: DateTime<Utc>,
}

impl QuietTool {
    /// Whether the tool is gone.
    pub fn gone(&self) -> bool {
        !self.process.is_running_named_at(self.named_at)
    }
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
    /// The id of the Terminal window `terminal`'s tab was in when the
    /// launch read it back. Recorded by owner decision (2026-11-02) for
    /// #1852's window picker; nothing in production reads it yet. A row
    /// written before it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_window: Option<u32>,
    /// The ended row Start again or Resume prepared this one from. Set by
    /// the daemon, never by a request: the app no longer names the row
    /// (#1839 review, finding 11). Kept by owner decision for #1852;
    /// nothing in production reads it yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    again_of: Option<Uuid>,
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
    /// The daemon ended this row when it saw the tool exit while the
    /// launcher was quiet ([`SessionRegistry::tool_exited`]). Cleared by
    /// the launcher's final report, which is then taken as how it ended.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    ended_by_daemon: bool,
    /// The daemon failed this row's pending collection when it ended the
    /// row, because nothing it knows of will collect it. A launcher that
    /// did collect still says so, in a late report or its exit receipt,
    /// and that result is taken over this one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    collection_by_daemon: bool,
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
            // A row with a collection plan this build can read is still
            // collected by the daemon once its tool is gone
            // (`collection_plan`); any other plan is removed by it.
            if entry.view.state == SessionState::Failed
                && entry.view.collection == CollectionState::Pending
                && super::collection_plan::read(store, entry.view.id).is_none()
            {
                // The daemon's guess: a receipt the launcher left, replayed
                // below, still replaces it.
                entry.view.collection = CollectionState::Failed;
                entry.collection_by_daemon = true;
            }
            entry.ticket_hash = None;
            entry.request.keep_saved_model = entry.keep_saved_model;
            entry.request.resume_of = entry.resume_of;
            entry.request.background_model = entry.view.background_model.clone();
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
        // One row per conversation: the latest row that stands for it. A
        // resume is a new row on the same conversation, and listing the
        // rows it continues beside it showed the conversation once per
        // resume, each with its own Resume. A resume stands for the
        // conversation only while it holds it or once its ticket was
        // redeemed, so one never redeemed does not hide the row that ran.
        //
        // Dismissal is the conversation's: [`Self::dismiss`] flags every
        // row of it, so none is left to list. A row flagged alone (#1838
        // and v0.21.0-0.21.2 dismissed one row at a time) is passed over,
        // and the conversation is listed as its latest row the person kept.
        // The two builds wrote the same file, so a #1838 dismissal of a
        // resumed conversation cannot be told from a v0.21.x dismissal of
        // its resume alone; this reading never hides a row they kept.
        let mut listed = std::collections::BTreeMap::new();
        for (index, e) in self.data.entries.iter().enumerate() {
            if !e.dismissed && e.stands_for_conversation() {
                listed.insert(e.root(), index);
            }
        }
        let mut listed: Vec<usize> = listed.into_values().collect();
        listed.sort_unstable();
        let held = self.holders(true);
        Ok(listed
            .into_iter()
            .map(|index| &self.data.entries[index])
            .map(|e| {
                let mut view = e.view.clone();
                let owned = e.owned_running();
                view.can_focus = owned && e.terminal.is_some();
                view.can_stop = owned && !e.stop_requested;
                view.can_start_again = false;
                view.can_resume = false;
                view.can_dismiss = !held.contains(&e.root());
                view
            })
            .collect())
    }

    /// Hold `id`'s launch for the caller about to open its terminal, after
    /// its ticket check: a second launch with the same ticket is refused
    /// from here on, while the terminal opens without the registry held.
    /// Returns the folder it starts in.
    pub fn begin_terminal_launch(
        &mut self,
        id: Uuid,
        ticket: &str,
    ) -> Result<PathBuf, ManagedError> {
        let cwd = self.launch_cwd(id, ticket)?;
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        entry.launched = true;
        self.commit(next)?;
        Ok(cwd)
    }

    /// The terminal for `id`'s held launch was refused before anything ran
    /// (never for `LaunchUnknown`, whose command may be running): the
    /// launch is released as if never begun, so its ticket stands until it
    /// lapses.
    pub fn terminal_launch_failed(&mut self, id: Uuid) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let Some(entry) = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == id && e.terminal.is_none() && e.proof_hash.is_none())
        else {
            return Ok(());
        };
        entry.launched = false;
        self.commit(next)
    }

    /// Record that the daemon opened `id` in a terminal, after its ticket
    /// check and its own launch, with the tab when one was named and the
    /// window that held it when that could be read.
    pub fn opened_in_terminal(
        &mut self,
        id: Uuid,
        terminal: Option<TerminalHandle>,
        window: Option<u32>,
    ) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        entry.launched = true;
        entry.terminal_window = terminal.as_ref().and(window);
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

    /// The Terminal window the row session `id` was started again or
    /// resumed from ran in: only for a row the daemon opened, that started,
    /// and that has ended. None otherwise, and for any session not prepared
    /// by Start again or Resume. Nothing in production reads it: Start
    /// again and Resume open a new window (owner, 2026-11-02). It is how
    /// the tests see what each row records (`again_of`, `terminal_window`).
    #[cfg(test)]
    pub(crate) fn again_window(&self, id: Uuid) -> Option<u32> {
        let entries = &self.data.entries;
        let source_id = entries.iter().find(|e| e.view.id == id)?.again_of?;
        let source = entries.iter().find(|e| e.view.id == source_id)?;
        if !source.launched || source.proof_hash.is_none() || source.view.state.holds_profile() {
            return None;
        }
        source.terminal.as_ref()?;
        source.terminal_window
    }

    /// The running sessions the daemon opened in a Terminal window it can
    /// name, newest first: each one's id, tab and window. Only these can be
    /// offered in the launch dialog's window list, and each only once the
    /// live window is seen to hold its tab still.
    pub fn placeable_sessions(&self) -> Vec<(Uuid, TerminalHandle, u32)> {
        let mut rows: Vec<&Entry> = self
            .data
            .entries
            .iter()
            .filter(|e| e.owned_running() && e.terminal.is_some() && e.terminal_window.is_some())
            .collect();
        rows.sort_by_key(|e| std::cmp::Reverse(e.view.started_at));
        rows.into_iter()
            .filter_map(|e| Some((e.view.id, e.terminal.clone()?, e.terminal_window?)))
            .collect()
    }

    /// Where a launch the person placed beside session `beside` opens: that
    /// session's window and tab, while it is among
    /// [`Self::placeable_sessions`]. None otherwise, and the launch opens a
    /// new window.
    pub fn placement_beside(&self, beside: Uuid) -> Option<Placement> {
        let entry = self.data.entries.iter().find(|e| e.view.id == beside)?;
        if !entry.owned_running() {
            return None;
        }
        Placement::beside(entry.terminal_window?, entry.terminal.as_ref()?)
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
    /// Also a row the app opened that the person dismissed while it looked
    /// stuck, whose launcher turns out to be running its tool
    /// ([`Self::dismiss`]). Like Stop, never a row started from the command
    /// line (owner, 2026-11-02).
    pub fn stop_requested(&self, id: Uuid) -> bool {
        self.data.entries.iter().any(|e| {
            e.view.id == id
                && e.stop_requested
                && (e.owned_running() || (e.launched && e.dismissed && e.ended_by_daemon))
        })
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
            resume_of: Some(entry.root()),
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
        let root = entry.root();
        (self
            .data
            .entries
            .iter()
            .any(|e| e.view.id == root && e.proof_hash.is_some())
            && !self.conversation_in_use(root))
        .then_some(root)
    }

    /// [`Self::resume_root`] for every row at once, in one pass, for a
    /// snapshot: each row whose conversation can be continued now, with
    /// that conversation's root.
    pub fn resume_roots(&self) -> std::collections::BTreeMap<Uuid, Uuid> {
        let in_use = self.holders(false);
        let started: std::collections::BTreeSet<Uuid> = self
            .data
            .entries
            .iter()
            .filter(|e| e.proof_hash.is_some())
            .map(|e| e.view.id)
            .collect();
        self.data
            .entries
            .iter()
            .map(|e| (e.view.id, e.root()))
            .filter(|(_, root)| started.contains(root) && !in_use.contains(root))
            .collect()
    }

    /// The store this registry was opened on.
    pub fn store(&self) -> &ConfigStore {
        &self.store
    }

    /// Whether a row on the conversation `root` started (the root itself,
    /// or a resume of it) still holds its profile. A stuck row does: its
    /// launcher may still be alive, so no other session may resume it.
    fn conversation_in_use(&self, root: Uuid) -> bool {
        self.data
            .entries
            .iter()
            .any(|e| e.root() == root && self.holds(e, false))
    }

    /// Whether row `e` holds its conversation: it holds its profile. When
    /// `dismissing`, a stuck row ([`Self::stuck`]) does not: the person may
    /// let it go. The one rule behind Resume's lock, Dismiss and
    /// `can_dismiss`.
    fn holds(&self, e: &Entry, dismissing: bool) -> bool {
        e.view.state.holds_profile() && !(dismissing && self.stuck(e))
    }

    /// The roots of the conversations a row holds ([`Self::holds`]).
    fn holders(&self, dismissing: bool) -> std::collections::BTreeSet<Uuid> {
        self.data
            .entries
            .iter()
            .filter(|e| self.holds(e, dismissing))
            .map(Entry::root)
            .collect()
    }

    /// Unknown with no process the daemon can watch: its launcher took the
    /// ticket and went quiet before its first report named the tool's
    /// process, and no collection plan file is there either (one this build
    /// cannot read still blocks it until the collection pass removes it). So
    /// nothing will end it but the person (owner, 2026-11-02: dismissed by
    /// hand, never on a timer). A launcher still in its setup is safe to
    /// dismiss from: it checks in before it spawns, and a dismissed row's
    /// check-in is refused ([`Self::report`]).
    fn stuck(&self, e: &Entry) -> bool {
        e.view.state == SessionState::Unknown
            && e.proof_hash.is_some()
            && e.watched_process().is_none()
            && !super::collection_plan::exists(&self.store, e.view.id)
    }

    /// Fail unredeemed launches whose ticket expired, and lapse redeemed
    /// sessions unheard for longer than [`LEASE`], as of `now`.
    pub fn sweep(&mut self, now: DateTime<Utc>) -> Result<(), ManagedError> {
        let expired = |e: &Entry| {
            e.view.state == SessionState::Starting
                && e.proof_hash.is_none()
                && now >= e.ticket_expires
        };
        let lapses = |e: &Entry| {
            matches!(e.view.state, SessionState::Starting | SessionState::Running)
                && e.proof_hash.is_some()
                && now.signed_duration_since(e.last_seen) > LEASE
        };
        // Runs on every lease tick: copy the registry only to change it.
        if !self.data.entries.iter().any(|e| expired(e) || lapses(e)) {
            return Ok(());
        }
        let mut next = self.data.clone();
        let mut changed = false;
        for entry in &mut next.entries {
            if expired(entry) {
                entry.view.state = SessionState::Failed;
                if entry.view.collection == CollectionState::Pending {
                    entry.view.collection = CollectionState::Failed;
                }
                entry.ticket_hash = None;
                changed = true;
            } else if lapses(entry) {
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
        request: LaunchRequest,
        account: &AccountView,
    ) -> Result<PreparedLaunch, ManagedError> {
        self.prepare_again(request, account, None)
    }

    /// [`Self::prepare`], for Start again and Resume: `again_of` is the
    /// ended row the request came from, kept on the new row. A replay
    /// returns the row first prepared, unchanged.
    pub fn prepare_again(
        &mut self,
        mut request: LaunchRequest,
        account: &AccountView,
        again_of: Option<Uuid>,
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
            background_model: request.background_model.clone(),
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
            can_dismiss: false,
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
            terminal_window: None,
            again_of,
            stop_requested: false,
            view,
            keep_saved_model: request.keep_saved_model,
            resume_of: request.resume_of,
            ended_by_daemon: false,
            collection_by_daemon: false,
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
        // A background model is named only where a NEAR AI proxy serves it,
        // and only for a tool with a background slot; its tier is checked
        // when the tool's configuration is built.
        if let Some(background) = request.background_model.as_deref() {
            let ok = super::opencode::valid_model(background)
                && request.purpose == LaunchPurpose::Coding
                && request.tool != ToolId::Codex
                && matches!(
                    request.connection,
                    ConnectionKind::NearAi | ConnectionKind::NearAiSignIn
                );
            if !ok {
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
        // Refused here, before anything is recorded, so a shell sees why;
        // the launcher checks again before the tool starts.
        if request.purpose == LaunchPurpose::Coding {
            super::profiles::project_model_check(
                request.tool,
                request.connection,
                &request.cwd,
                &cli_options,
            )?;
        }
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
            background_model: entry.request.background_model.clone(),
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
            // A row the person dismissed while it looked stuck, whose tool
            // runs after all: its heartbeat is answered, with a stop
            // ([`Self::stop_requested`]); the row stays ended.
            if entry.launched
                && entry.dismissed
                && entry.stop_requested
                && entry.ended_by_daemon
                && report.state == SessionState::Running
            {
                return Ok(());
            }
            // The daemon ended a sign-in session (sign-out, lapsed lease)
            // before its launcher said how it ended. The launcher's final
            // word is accepted and changes nothing, so its persisted exit
            // receipt never blocks the registry from opening.
            let ended_by_daemon = entry.view.connection == ConnectionKind::NearAiSignIn
                && entry.view.state == SessionState::Failed
                && matches!(report.state, SessionState::Exited | SessionState::Failed);
            // The daemon ended this row once it saw the tool exit: the
            // launcher's final word is the first to say how, and is taken.
            let first_word = entry.ended_by_daemon
                && matches!(report.state, SessionState::Exited | SessionState::Failed);
            if !(ended_by_daemon
                || first_word
                || (report.state == entry.view.state && report.exit_code == entry.view.exit_code))
            {
                return Err(ManagedError::Conflict);
            }
            let mut changed = false;
            if first_word {
                if report.state == SessionState::Exited && report.exit_code.is_none() {
                    return Err(ManagedError::InvalidRequest);
                }
                entry.view.state = report.state;
                entry.view.exit_code = report.exit_code;
                entry.ended_by_daemon = false;
                changed = true;
            }
            // Its collection result is still taken while the row waits for
            // one: the launcher collected, and removed the collection plan
            // the daemon would otherwise have collected from.
            if let Some(collection @ (CollectionState::Collected | CollectionState::Failed)) =
                report.collection
            {
                if entry.view.collect_traces
                    && (entry.view.collection == CollectionState::Pending
                        || entry.collection_by_daemon)
                {
                    entry.view.collection = collection;
                    entry.collection_by_daemon = false;
                    changed = true;
                }
            }
            return if changed { self.commit(next) } else { Ok(()) };
        }
        if report.state == SessionState::Starting {
            // A launcher's check-in right before it spawns its tool: it is
            // still in its setup, so the row is starting again, with a
            // fresh lease. A dismissed row was refused above, so its
            // launcher never spawns.
            if entry.process_id.is_some()
                || report.process_id.is_some()
                || report.process_identity.is_some()
                || report.exit_code.is_some()
                || report.collection.is_some()
                || report.model.is_some()
            {
                return Err(ManagedError::InvalidRequest);
            }
            entry.view.state = SessionState::Starting;
            entry.last_seen = Utc::now();
            return self.commit(next);
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

    /// The daemon saw row `id`'s tool exit while its launcher was quiet
    /// (the row's lease lapsed, so it is unknown): the row has ended,
    /// without an exit code from its tool, and no longer holds its
    /// conversation. The launcher's final report, if one still comes, is
    /// taken as how it ended. A row in any other state is left as it is:
    /// a launcher that still reports says so itself.
    pub fn tool_exited(&mut self, id: Uuid) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let entry = next
            .entries
            .iter_mut()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?;
        if entry.view.state != SessionState::Unknown {
            return Ok(());
        }
        entry.end_by_daemon(false);
        self.commit(next)
    }

    /// The quiet rows (unknown: the launcher stopped reporting) whose tool
    /// the daemon can watch, with that tool's process: rows a collection
    /// plan (`plans`) does not cover, whose launcher named the tool's
    /// process. A pid named without its start time counts as running while
    /// any process holds it that started before the launcher last named it
    /// (`named_at`), so a reused pid only delays the end, and only while
    /// that is so.
    pub fn quiet_tools(&self, plans: &std::collections::BTreeSet<Uuid>) -> Vec<QuietTool> {
        self.data
            .entries
            .iter()
            .filter(|e| e.view.state == SessionState::Unknown && !plans.contains(&e.view.id))
            .filter_map(|e| {
                Some(QuietTool {
                    id: e.view.id,
                    process: e.watched_process()?,
                    named_at: e.last_seen,
                })
            })
            .collect()
    }

    /// End the quiet rows in `ids` whose tool the daemon saw exit, as
    /// [`Self::tool_exited`] does, in one write. They have no collection
    /// plan, so the daemon will not collect them: a collection still
    /// pending fails, unless the launcher, which may have collected itself,
    /// still reports otherwise ([`Entry::end_by_daemon`]). A row that is no
    /// longer unknown (its launcher reported again) is left as it is.
    pub fn end_quiet(&mut self, ids: &[Uuid]) -> Result<(), ManagedError> {
        let mut next = self.data.clone();
        let mut changed = false;
        for entry in next
            .entries
            .iter_mut()
            .filter(|e| ids.contains(&e.view.id) && e.view.state == SessionState::Unknown)
        {
            entry.end_by_daemon(true);
            changed = true;
        }
        if changed { self.commit(next) } else { Ok(()) }
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

    /// Dismiss the conversation of row `id`: every row of it, so no
    /// earlier row comes back in its place. Refused while a row of it
    /// holds its profile, unless that row is stuck ([`Self::stuck`]): the
    /// person may let it go (owner, 2026-11-02), and it is ended as
    /// [`Self::tool_exited`] ends a row, so a launcher that does report
    /// later is still taken as how it ended.
    pub fn dismiss(&mut self, id: Uuid) -> Result<(), ManagedError> {
        let root = self
            .data
            .entries
            .iter()
            .find(|e| e.view.id == id)
            .ok_or(ManagedError::NotFound)?
            .root();
        // Refused by the same rule as `can_dismiss` ([`Self::holds`]); the
        // rows that still hold the conversation past it are stuck.
        let mut stuck = Vec::new();
        for e in self.data.entries.iter().filter(|e| e.root() == root) {
            if self.holds(e, true) {
                return Err(ManagedError::Conflict);
            }
            if e.view.state.holds_profile() {
                stuck.push(e.view.id);
            }
        }
        let mut next = self.data.clone();
        for entry in next.entries.iter_mut().filter(|e| e.root() == root) {
            if stuck.contains(&entry.view.id) {
                // No plan (it is stuck), so nothing will collect it. Its
                // launcher may yet be running the tool, if the daemon
                // restarted between its spawn and its first report: if the
                // app opened it, its next heartbeat is told to stop
                // ([`Self::stop_requested`]); a command-line launch is never
                // stopped by the app (owner, 2026-11-02).
                entry.end_by_daemon(true);
                entry.stop_requested = entry.launched;
            }
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
    /// The root row of this row's conversation: the row it resumed, or
    /// itself. Rows sharing a root are one conversation, listed once and
    /// run by one session at a time.
    fn root(&self) -> Uuid {
        self.request.resume_of.unwrap_or(self.view.id)
    }

    /// The tool's process as the launcher named it: its pid, with its
    /// start time when the launcher could read it.
    fn watched_process(&self) -> Option<super::collection_plan::ProcessIdentity> {
        self.process_identity
            .as_deref()
            .and_then(super::collection_plan::ProcessIdentity::parse)
            .or_else(|| {
                Some(super::collection_plan::ProcessIdentity {
                    pid: self.process_id?,
                    start_time: None,
                })
            })
    }

    /// The daemon ends this row: it saw the tool exit, or the person
    /// dismissed it while nothing watched it. Failed, the state for a
    /// session that ended without an exit code from its tool; the
    /// launcher's final report, if one still comes, is taken as how it
    /// ended. With `unclaimed`, nothing the daemon knows of will collect
    /// the row, so a pending collection fails too, until the launcher says
    /// otherwise (`collection_by_daemon`).
    fn end_by_daemon(&mut self, unclaimed: bool) {
        self.view.state = SessionState::Failed;
        self.ended_by_daemon = true;
        if unclaimed && self.view.collection == CollectionState::Pending {
            self.view.collection = CollectionState::Failed;
            self.collection_by_daemon = true;
        }
    }

    /// Whether this row is listed for its conversation when it is the
    /// latest: the root always; a resume while it holds the conversation
    /// or once its ticket was redeemed.
    fn stands_for_conversation(&self) -> bool {
        self.root() == self.view.id || self.view.state.holds_profile() || self.proof_hash.is_some()
    }

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
            background_model: None,
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
                    background_model: None,
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

    /// A NEAR AI Claude Code launch whose project, or a folder its CLI
    /// options add, chooses a model is refused before a session is
    /// recorded, so a shell sees the refusal's own label (owner,
    /// 2026-11-01), and nothing waits in the registry.
    #[test]
    fn a_project_that_chooses_a_model_is_refused_at_prepare() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("config")).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let sign_in = crate::managed::signin::entry(ToolId::Claude).unwrap();
        let project = home.path().join("project");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        let request = |cli_options: Option<&str>| LaunchRequest {
            cli_options: cli_options.map(str::to_string),
            model: None,
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool: ToolId::Claude,
            connection: ConnectionKind::NearAiSignIn,
            account_id: sign_in.id,
            cwd: project.clone(),
            expected_generation: Generation(0),
            save_default: false,
            keep_saved_model: false,
            resume_of: None,
            background_model: None,
        };
        let agent = "---\nname: a\nmodel: anthropic/claude-opus-5-5\n---\n";

        let extra = home.path().join("extra");
        std::fs::create_dir_all(extra.join(".claude/agents")).unwrap();
        std::fs::write(extra.join(".claude/agents/a.md"), agent).unwrap();
        let add_dir = format!(
            "--add-dir {}",
            crate::managed::options::quote(&extra.display().to_string())
        );
        assert!(!registry.would_prepare_new(request(Some(&add_dir)), &sign_in));
        assert_eq!(
            registry
                .prepare(request(Some(&add_dir)), &sign_in)
                .map(|p| p.session_id),
            Err(ManagedError::ProjectChoosesModel)
        );
        // Without the added folder the project is fine.
        assert!(registry.prepare(request(Some("")), &sign_in).is_ok());

        std::fs::create_dir_all(project.join(".claude/commands")).unwrap();
        std::fs::write(project.join(".claude/commands/c.md"), agent).unwrap();
        assert_eq!(
            registry
                .prepare(request(Some("")), &sign_in)
                .map(|p| p.session_id),
            Err(ManagedError::ProjectChoosesModel)
        );
        assert_eq!(
            registry.data.entries.len(),
            1,
            "no refused launch is recorded"
        );
    }

    /// A launch's background model is kept on the session's view, which
    /// its launcher reads and an older daemon skips, never on the stored
    /// request; it survives a restart, a retry is still one session, Start
    /// again keeps it, and the folder settings remember it. Codex and a
    /// connection with no NEAR AI proxy take none.
    #[test]
    fn a_background_model_is_kept_on_the_view_and_remembered() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let sign_in = crate::managed::signin::entry(ToolId::Claude).unwrap();
        let request = LaunchRequest {
            cli_options: Some(String::new()),
            model: Some("Qwen/Qwen3.8-27B".into()),
            collect_traces: false,
            purpose: LaunchPurpose::Coding,
            request_id: Uuid::new_v4(),
            tool: ToolId::Claude,
            connection: ConnectionKind::NearAiSignIn,
            account_id: sign_in.id,
            cwd: home.path().to_path_buf(),
            expected_generation: Generation(0),
            save_default: false,
            keep_saved_model: false,
            resume_of: None,
            background_model: Some("z-ai/glm-5.3-flash".into()),
        };
        let id = registry
            .prepare(request.clone(), &sign_in)
            .unwrap()
            .session_id;
        let raw: serde_json::Value =
            serde_json::from_slice(&store.read_daemon_file(FILE).unwrap().unwrap()).unwrap();
        let entry = &raw["entries"][0];
        assert!(
            entry["request"].get("background_model").is_none(),
            "{entry}"
        );
        assert_eq!(entry["view"]["background_model"], "z-ai/glm-5.3-flash");
        assert_eq!(
            registry
                .folder_settings(id)
                .unwrap()
                .background_model
                .as_deref(),
            Some("z-ai/glm-5.3-flash")
        );
        let mut registry = SessionRegistry::open(&store).unwrap();
        assert_eq!(
            registry
                .prepare(request.clone(), &sign_in)
                .unwrap()
                .session_id,
            id
        );
        let again = registry.start_again_request(id, Uuid::new_v4()).unwrap();
        assert_eq!(
            again.background_model.as_deref(),
            Some("z-ai/glm-5.3-flash")
        );
        // A launch without one stores none.
        let plain = registry
            .prepare(
                LaunchRequest {
                    background_model: None,
                    ..again.clone()
                },
                &sign_in,
            )
            .unwrap()
            .session_id;
        let raw: serde_json::Value =
            serde_json::from_slice(&store.read_daemon_file(FILE).unwrap().unwrap()).unwrap();
        let plain = raw["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["view"]["id"] == serde_json::json!(plain))
            .unwrap();
        assert!(plain["view"].get("background_model").is_none());
        // Codex, and Claude on a subscription, take none.
        for (tool, connection) in [
            (ToolId::Codex, ConnectionKind::NearAiSignIn),
            (ToolId::Claude, ConnectionKind::Subscription),
        ] {
            let refused = LaunchRequest {
                request_id: Uuid::new_v4(),
                tool,
                connection,
                model: None,
                ..request.clone()
            };
            assert_eq!(
                registry.check(&refused, &sign_in).err(),
                Some(ManagedError::InvalidRequest),
                "{tool:?} {connection:?}"
            );
        }
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
            background_model: None,
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
            background_model: None,
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
            background_model: None,
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
            background_model: None,
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
            background_model: None,
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

    /// A conversation is one row, its latest: the rows a resume continues
    /// are not listed beside it, and dismissing the latest does not bring
    /// one back. A fresh launch in the same folder is its own row.
    #[test]
    fn a_resumed_conversation_is_listed_once() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let first = ended(&mut registry, request, &account);
        let request = registry.resume_request(first, Uuid::new_v4()).unwrap();
        let second = ended(&mut registry, request, &account);
        let fresh = ended(&mut registry, coding(&account, home.path()), &account);

        let ids = |registry: &mut SessionRegistry| -> Vec<Uuid> {
            registry.views().unwrap().iter().map(|v| v.id).collect()
        };
        assert_eq!(ids(&mut registry), vec![second, fresh]);

        registry.dismiss(second).unwrap();
        assert_eq!(ids(&mut registry), vec![fresh], "no older row comes back");
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
        // A snapshot's one pass agrees with the row-by-row answer.
        let agrees = |registry: &SessionRegistry| {
            let roots = registry.resume_roots();
            for id in [root, chained, prepared.session_id] {
                assert_eq!(roots.get(&id).copied(), registry.resume_root(id), "{id}");
            }
        };
        agrees(&registry);
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
        agrees(&registry);
    }

    /// Set the stored `dismissed` flag of the rows at `indexes` directly in
    /// the registry file, as a v0.21.0-0.21.2 daemon left it when the person
    /// dismissed those rows one at a time.
    fn dismissed_on_disk(store: &ConfigStore, indexes: &[usize], dismissed: bool) {
        let mut raw: serde_json::Value =
            serde_json::from_slice(&store.read_daemon_file(FILE).unwrap().unwrap()).unwrap();
        for index in indexes {
            raw["entries"][*index]["dismissed"] = serde_json::json!(dismissed);
        }
        store
            .write_daemon_file(FILE, &serde_json::to_vec(&raw).unwrap())
            .unwrap();
    }

    /// A resumed row says when its conversation's root row started; a
    /// fresh row says nothing; a chain points at the root; and the marker
    /// survives the root row alone being dismissed (as a v0.21.x daemon
    /// did) and a restart.
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

        dismissed_on_disk(&store, &[0], true);
        let mut reopened = SessionRegistry::open(&store).unwrap();
        assert!(reopened.data.entries[0].dismissed, "the root is dismissed");
        let views = reopened.views().unwrap();
        let ids: Vec<Uuid> = views.iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![chained], "listed once, as its latest row");
        assert_eq!(views[0].resumed_from, Some(root_started));
        let json = serde_json::to_value(&views[0]).unwrap();
        assert!(json["resumed_from"].is_string(), "{json}");
    }

    /// Dismiss is the conversation's: dismissing any of its rows dismisses
    /// every row of it, the flag is stored on each and survives a restart,
    /// and nothing of it is listed after. A fresh launch is untouched.
    #[test]
    fn dismissing_a_conversation_dismisses_every_row_of_it() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let resumed = ended(&mut registry, request, &account);
        let fresh = ended(&mut registry, coding(&account, home.path()), &account);

        registry.dismiss(resumed).unwrap();
        let mut reopened = SessionRegistry::open(&store).unwrap();
        let flags: Vec<(Uuid, bool)> = reopened
            .data
            .entries
            .iter()
            .map(|e| (e.view.id, e.dismissed))
            .collect();
        assert_eq!(
            flags,
            vec![(root, true), (resumed, true), (fresh, false)],
            "stored on every row of the conversation"
        );
        let ids: Vec<Uuid> = reopened.views().unwrap().iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![fresh]);
    }

    /// A registry where rows were dismissed one at a time, by #1838 (which
    /// flagged only the listed latest row, staging builds from 2026-11-02)
    /// or by v0.21.0-0.21.2 (every row listed). Both wrote the same file,
    /// so the two cannot be told apart, and the reading chosen never hides
    /// a row the person kept: the conversation is listed as its latest row
    /// not dismissed, and is gone once every row that ran is dismissed. A
    /// #1838 dismissal of a resumed conversation therefore shows its
    /// earlier row again.
    #[test]
    fn rows_dismissed_one_at_a_time_never_hide_a_row_the_person_kept() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let resumed = ended(&mut registry, request, &account);
        drop(registry);
        let listed = || -> Vec<Uuid> {
            SessionRegistry::open(&store)
                .unwrap()
                .views()
                .unwrap()
                .iter()
                .map(|v| v.id)
                .collect()
        };

        dismissed_on_disk(&store, &[1], true);
        assert_eq!(listed(), vec![root], "the resume alone was dismissed");
        dismissed_on_disk(&store, &[0], true);
        assert_eq!(listed(), Vec::<Uuid>::new(), "every row was dismissed");
        dismissed_on_disk(&store, &[1], false);
        assert_eq!(listed(), vec![resumed], "the root alone was dismissed");

        // A resume whose ticket expired unredeemed, dismissed alone: the
        // row that ran is listed.
        dismissed_on_disk(&store, &[0], false);
        let mut registry = SessionRegistry::open(&store).unwrap();
        let request = registry.resume_request(resumed, Uuid::new_v4()).unwrap();
        registry.prepare(request, &account).unwrap();
        registry.sweep(Utc::now() + Duration::seconds(121)).unwrap();
        drop(registry);
        dismissed_on_disk(&store, &[2], true);
        assert_eq!(listed(), vec![resumed], "the row that ran");
    }

    /// A resume whose tool never started (its ticket expired) does not
    /// hide the row that ran: the conversation is listed as that row, and
    /// dismissing it dismisses the conversation. While the resume is only
    /// prepared it holds the conversation, and it is the row listed.
    #[test]
    fn a_resume_that_never_started_does_not_hide_the_row_that_ran() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let resumed = ended(&mut registry, request, &account);
        let request = registry.resume_request(resumed, Uuid::new_v4()).unwrap();
        let prepared = registry.prepare(request, &account).unwrap();
        let ids = |registry: &mut SessionRegistry| -> Vec<Uuid> {
            registry.views().unwrap().iter().map(|v| v.id).collect()
        };
        assert_eq!(ids(&mut registry), vec![prepared.session_id], "starting");

        registry.sweep(Utc::now() + Duration::seconds(121)).unwrap();
        assert_eq!(
            registry.describe(prepared.session_id).map(|(_, s)| s),
            Some(SessionState::Failed)
        );
        assert_eq!(ids(&mut registry), vec![resumed], "the row that ran");
        assert_eq!(registry.resume_root(resumed), Some(root));

        registry.dismiss(resumed).unwrap();
        assert_eq!(ids(&mut registry), Vec::<Uuid>::new());
    }

    /// A resume whose launcher took its ticket and died before its first
    /// report is unknown with no process to watch, so nothing ends it. The
    /// person can dismiss it (owner, 2026-11-02): the row says
    /// `can_dismiss`, and dismissing ends it and dismisses its
    /// conversation, freeing the account. A quiet row whose launcher named
    /// its tool's process, or one still starting, cannot be dismissed; an
    /// ended row can. A launcher that does report later is still heard.
    #[test]
    fn a_row_stuck_before_its_first_report_can_be_dismissed() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let can_dismiss = |registry: &mut SessionRegistry, id: Uuid| {
            registry
                .views()
                .unwrap()
                .iter()
                .find(|v| v.id == id)
                .map(|v| v.can_dismiss)
        };
        assert_eq!(can_dismiss(&mut registry, root), Some(true), "ended");

        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let stuck = registry.prepare(request, &account).unwrap();
        assert_eq!(
            can_dismiss(&mut registry, stuck.session_id),
            Some(false),
            "starting"
        );
        let context = registry
            .redeem(stuck.session_id, stuck.ticket.as_deref().unwrap())
            .unwrap();
        let fresh = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        let fresh_context = registry
            .redeem(fresh.session_id, fresh.ticket.as_deref().unwrap())
            .unwrap();
        registry
            .report(LifecycleReport {
                model: None,
                collection: None,
                session_id: fresh.session_id,
                proof: fresh_context.proof,
                state: SessionState::Running,
                process_id: Some(42),
                process_identity: Some("42:1".into()),
                exit_code: None,
            })
            .unwrap();
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        for id in [stuck.session_id, fresh.session_id] {
            assert_eq!(
                registry.describe(id).map(|(_, s)| s),
                Some(SessionState::Unknown)
            );
        }
        assert_eq!(can_dismiss(&mut registry, stuck.session_id), Some(true));
        assert_eq!(
            can_dismiss(&mut registry, fresh.session_id),
            Some(false),
            "its tool's process is watched"
        );
        assert_eq!(
            registry.dismiss(fresh.session_id),
            Err(ManagedError::Conflict)
        );

        registry.dismiss(stuck.session_id).unwrap();
        let listed: Vec<Uuid> = registry.views().unwrap().iter().map(|v| v.id).collect();
        assert_eq!(listed, vec![fresh.session_id], "the conversation is gone");
        assert_eq!(
            registry.describe(stuck.session_id).map(|(_, s)| s),
            Some(SessionState::Failed)
        );
        assert!(registry.resume_root(root).is_some(), "no longer in use");
        registry
            .report(LifecycleReport {
                model: None,
                collection: None,
                session_id: stuck.session_id,
                proof: context.proof,
                state: SessionState::Exited,
                process_id: None,
                process_identity: None,
                exit_code: Some(0),
            })
            .unwrap();
        assert_eq!(
            registry.describe(stuck.session_id).map(|(_, s)| s),
            Some(SessionState::Exited),
            "a late final report is taken"
        );
    }

    /// A quiet row whose launcher named its tool's pid but not its start
    /// time (`<pid>:<uuid>`, the launcher's fallback) is watched like any
    /// other: a pid alone counts as running while any process holds it, so
    /// a reused pid only delays its end, never ends a live tool. It is not
    /// the person's to dismiss while it may run. The quiet rows the daemon
    /// watches leave out the ones a collection plan covers.
    #[test]
    fn a_quiet_row_with_a_pid_only_identity_is_watched() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let mut quiet = |pid: u32, identity: String| {
            let prepared = registry
                .prepare(coding(&account, home.path()), &account)
                .unwrap();
            let context = registry
                .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
                .unwrap();
            registry
                .report(LifecycleReport {
                    model: None,
                    collection: None,
                    session_id: prepared.session_id,
                    proof: context.proof,
                    state: SessionState::Running,
                    process_id: Some(pid),
                    process_identity: Some(identity),
                    exit_code: None,
                })
                .unwrap();
            prepared.session_id
        };
        let pid_only = quiet(42, format!("42:{}", Uuid::new_v4()));
        let watched = quiet(43, "43:1700000000".to_owned());
        let planned = quiet(44, "44:1700000000".to_owned());
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        let views = registry.views().unwrap();
        for id in [pid_only, watched] {
            assert!(!views.iter().find(|v| v.id == id).unwrap().can_dismiss);
        }
        use super::super::collection_plan::ProcessIdentity;
        let plans = std::collections::BTreeSet::from([planned]);
        assert_eq!(
            registry
                .quiet_tools(&plans)
                .into_iter()
                .map(|tool| (tool.id, tool.process))
                .collect::<Vec<_>>(),
            vec![
                (
                    pid_only,
                    ProcessIdentity {
                        pid: 42,
                        start_time: None
                    }
                ),
                (
                    watched,
                    ProcessIdentity {
                        pid: 43,
                        start_time: Some(1_700_000_000),
                    }
                ),
            ]
        );
        assert_eq!(registry.dismiss(pid_only), Err(ManagedError::Conflict));
    }

    /// The daemon fails a quiet row's pending collection when it ends the
    /// row, but the launcher may have collected after all (it wrote no
    /// plan and collected itself, or its final report did not get
    /// through): its late report, or the exit receipt it left, is taken
    /// over the daemon's guess. A result the launcher or a plan recorded
    /// is never overwritten that way.
    #[test]
    fn a_launchers_late_collection_result_wins_over_the_daemons() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let mut quiet = || {
            let prepared = registry
                .prepare(
                    LaunchRequest {
                        collect_traces: true,
                        ..coding(&account, home.path())
                    },
                    &account,
                )
                .unwrap();
            let context = registry
                .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
                .unwrap();
            registry
                .report(LifecycleReport {
                    model: None,
                    collection: None,
                    session_id: prepared.session_id,
                    proof: context.proof.clone(),
                    state: SessionState::Running,
                    process_id: Some(42),
                    process_identity: Some("42:1".into()),
                    exit_code: None,
                })
                .unwrap();
            (prepared.session_id, context.proof)
        };
        let (reported, reported_proof) = quiet();
        let (receipted, receipted_proof) = quiet();
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        registry.end_quiet(&[reported, receipted]).unwrap();
        let collection = |registry: &SessionRegistry, id: Uuid| {
            registry.collection_status(id).unwrap().collection
        };
        assert_eq!(collection(&registry, reported), CollectionState::Failed);
        let late = |id: Uuid, proof: String| LifecycleReport {
            model: None,
            collection: Some(CollectionState::Collected),
            session_id: id,
            proof,
            state: SessionState::Exited,
            process_id: Some(42),
            process_identity: Some("42:1".into()),
            exit_code: Some(0),
        };
        registry
            .report(late(reported, reported_proof.clone()))
            .unwrap();
        assert_eq!(collection(&registry, reported), CollectionState::Collected);
        persist_exit(&store, &late(receipted, receipted_proof)).unwrap();
        registry.views().unwrap();
        assert_eq!(collection(&registry, receipted), CollectionState::Collected);
        assert_eq!(
            registry.report(LifecycleReport {
                collection: Some(CollectionState::Failed),
                ..late(reported, reported_proof)
            }),
            Ok(()),
            "a repeat is accepted"
        );
        assert_eq!(
            collection(&registry, reported),
            CollectionState::Collected,
            "and changes nothing"
        );
    }

    /// A launcher checks in (a `starting` report) right before it spawns
    /// its tool. One that went quiet in a long setup is taken back as
    /// starting, with a fresh lease, so the person cannot dismiss it from
    /// under it; once dismissed, its check-in is refused, so it never
    /// spawns. A check-in from a row that already named its tool's process,
    /// or that names one, is refused as before.
    #[test]
    fn a_launcher_checks_in_before_it_spawns() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let mut redeemed = || {
            let prepared = registry
                .prepare(coding(&account, home.path()), &account)
                .unwrap();
            let context = registry
                .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
                .unwrap();
            (prepared.session_id, context.proof)
        };
        let (slow, slow_proof) = redeemed();
        let (gone, gone_proof) = redeemed();
        let check_in = |id: Uuid, proof: &str| LifecycleReport {
            model: None,
            collection: None,
            session_id: id,
            proof: proof.to_owned(),
            state: SessionState::Starting,
            process_id: None,
            process_identity: None,
            exit_code: None,
        };
        let can_dismiss = |registry: &mut SessionRegistry, id: Uuid| {
            registry
                .views()
                .unwrap()
                .iter()
                .find(|v| v.id == id)
                .unwrap()
                .can_dismiss
        };
        registry.report(check_in(slow, &slow_proof)).unwrap();
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        assert!(can_dismiss(&mut registry, slow), "quiet in its setup");
        registry.report(check_in(slow, &slow_proof)).unwrap();
        assert_eq!(
            registry.describe(slow).map(|(_, s)| s),
            Some(SessionState::Starting)
        );
        assert!(!can_dismiss(&mut registry, slow), "it checked in");
        assert_eq!(registry.dismiss(slow), Err(ManagedError::Conflict));

        assert!(can_dismiss(&mut registry, gone));
        registry.dismiss(gone).unwrap();
        assert_eq!(
            registry.report(check_in(gone, &gone_proof)),
            Err(ManagedError::Conflict),
            "dismissed: it must not spawn"
        );

        registry
            .report(LifecycleReport {
                state: SessionState::Running,
                process_id: Some(42),
                process_identity: Some("42:1".into()),
                ..check_in(slow, &slow_proof)
            })
            .unwrap();
        assert_eq!(
            registry.report(check_in(slow, &slow_proof)),
            Err(ManagedError::InvalidRequest),
            "its tool runs"
        );
        assert_eq!(
            registry.report(LifecycleReport {
                process_id: Some(42),
                ..check_in(slow, &slow_proof)
            }),
            Err(ManagedError::InvalidRequest)
        );
    }

    /// A row can look stuck while its tool runs: the launcher checked in
    /// and spawned, and the daemon restarted before it took a `running`
    /// report. Dismissing a row the app opened asks for a stop too, so its
    /// launcher's next heartbeat is answered with a stop (which every
    /// launcher since `stop` obeys) rather than refused and ignored; the
    /// row stays ended and dismissed, and its final report is still taken.
    /// A row started from the command line is never stopped by the app
    /// (owner, 2026-11-02, as with Stop): its heartbeat is refused as
    /// before and its tool keeps running. A check-in from a dismissed row
    /// is refused either way, so a launcher still in its setup never spawns.
    #[test]
    fn a_dismissed_row_whose_tool_runs_is_told_to_stop() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let prepared = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        let id = prepared.session_id;
        let context = registry
            .redeem(id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        registry.opened_in_terminal(id, None, None).unwrap();
        let report = |state, exit_code| LifecycleReport {
            model: None,
            collection: None,
            session_id: id,
            proof: context.proof.clone(),
            state,
            process_id: Some(42),
            process_identity: Some("42:1".into()),
            exit_code,
        };
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        registry.dismiss(id).unwrap();

        assert_eq!(registry.report(report(SessionState::Running, None)), Ok(()));
        assert!(registry.stop_requested(id), "told to stop");
        assert_eq!(
            registry.describe(id).map(|(_, s)| s),
            Some(SessionState::Failed),
            "still ended"
        );
        assert!(registry.views().unwrap().is_empty(), "still dismissed");
        assert_eq!(
            registry.report(LifecycleReport {
                state: SessionState::Starting,
                process_id: None,
                process_identity: None,
                ..report(SessionState::Starting, None)
            }),
            Err(ManagedError::Conflict),
            "a check-in is refused"
        );
        registry
            .report(report(SessionState::Exited, Some(143)))
            .unwrap();
        assert_eq!(
            registry.describe(id).map(|(_, s)| s),
            Some(SessionState::Exited)
        );
        assert!(!registry.stop_requested(id));
    }

    #[test]
    fn a_dismissed_command_line_row_is_not_stopped_by_the_app() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let prepared = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        let id = prepared.session_id;
        let context = registry
            .redeem(id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        registry.dismiss(id).unwrap();
        assert_eq!(
            registry.report(LifecycleReport {
                model: None,
                collection: None,
                session_id: id,
                proof: context.proof.clone(),
                state: SessionState::Running,
                process_id: Some(42),
                process_identity: Some("42:1".into()),
                exit_code: None,
            }),
            Err(ManagedError::Conflict),
            "refused, as before"
        );
        assert!(!registry.stop_requested(id), "never told to stop");
    }

    /// A restart fails a pending collection on an ended row with no plan,
    /// as its guess; the launcher's exit receipt, replayed in the same
    /// start, still replaces it.
    #[test]
    fn a_restart_keeps_a_late_receipts_collection_result() {
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
        let report = |state, collection, exit_code| LifecycleReport {
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
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        registry.tool_exited(id).unwrap();
        persist_exit(
            &store,
            &report(
                SessionState::Exited,
                Some(CollectionState::Collected),
                Some(0),
            ),
        )
        .unwrap();
        drop(registry);
        let reopened = SessionRegistry::open(&store).unwrap();
        assert_eq!(
            reopened.collection_status(id).unwrap().collection,
            CollectionState::Collected
        );
    }

    /// The list and the one-session-per-conversation lock group rows by the
    /// same conversation: whenever a conversation is in use, the row listed
    /// for it is the one holding it.
    #[test]
    fn the_list_and_the_conversation_lock_group_rows_alike() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let root = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(root, Uuid::new_v4()).unwrap();
        let resumed = ended(&mut registry, request, &account);
        let fresh = ended(&mut registry, coding(&account, home.path()), &account);
        let request = registry.resume_request(resumed, Uuid::new_v4()).unwrap();
        let running = registry.prepare(request, &account).unwrap().session_id;

        let views = registry.views().unwrap();
        assert_eq!(
            views.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![fresh, running],
            "one row per conversation"
        );
        for view in &views {
            let entry = registry
                .data
                .entries
                .iter()
                .find(|e| e.view.id == view.id)
                .unwrap();
            assert_eq!(
                registry.conversation_in_use(entry.root()),
                view.state.holds_profile(),
                "{}",
                view.id
            );
        }
        for e in &registry.data.entries {
            let expected = if e.view.id == fresh { fresh } else { root };
            assert_eq!(e.root(), expected, "{}", e.view.id);
        }
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

    /// A quiet row whose tool the daemon saw exit has ended: it no longer
    /// holds its conversation, so the conversation can be resumed. Its
    /// launcher's late final report, live or from its exit receipt at the
    /// next start, is still taken and says how the tool ended.
    #[test]
    fn a_quiet_row_whose_tool_exited_ends_and_frees_its_conversation() {
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
        let report = |state, exit_code| LifecycleReport {
            model: None,
            collection: None,
            session_id: id,
            proof: context.proof.clone(),
            state,
            process_id: Some(42),
            process_identity: Some("42:1".into()),
            exit_code,
        };
        registry
            .report(report(SessionState::Running, None))
            .unwrap();
        registry.tool_exited(id).unwrap();
        assert_eq!(
            registry.collection_status(id).unwrap().state,
            SessionState::Running,
            "a launcher that still reports says how its tool ended"
        );
        registry
            .sweep(Utc::now() + LEASE + Duration::seconds(1))
            .unwrap();
        assert_eq!(registry.resume_root(id), None, "unknown still holds it");
        registry
            .record_collection(id, CollectionState::Collected)
            .unwrap();
        registry.tool_exited(id).unwrap();
        let status = registry.collection_status(id).unwrap();
        assert_eq!(status.state, SessionState::Failed);
        assert_eq!(status.collection, CollectionState::Collected);
        assert_eq!(registry.resume_root(id), Some(id));

        // The launcher's exit receipt, left on disk, does not stop the
        // registry from opening, and its exit code is taken.
        let late = report(SessionState::Exited, Some(0));
        persist_exit(&store, &late).unwrap();
        drop(registry);
        let mut registry = SessionRegistry::open(&store).unwrap();
        let status = registry.collection_status(id).unwrap();
        assert_eq!(status.state, SessionState::Exited);
        assert_eq!(status.collection, CollectionState::Collected);
        assert_eq!(registry.resume_root(id), Some(id));
        // Taken once: the same word again changes nothing; another is refused.
        registry
            .report(report(SessionState::Exited, Some(0)))
            .unwrap();
        assert_eq!(
            registry.report(report(SessionState::Failed, None)),
            Err(ManagedError::Conflict)
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

    fn tab(n: u8) -> TerminalHandle {
        TerminalHandle::AppleTerminal {
            tty: format!("/dev/ttys{n:03}"),
        }
    }

    /// Prepare `request` (from row `again_of`, when set), open it in
    /// `window`'s tab, in Terminal window `wid`, start it and report it in
    /// `state`. Returns its id.
    fn ran_in(
        registry: &mut SessionRegistry,
        (request, again_of): (LaunchRequest, Option<Uuid>),
        account: &AccountView,
        (window, wid): (TerminalHandle, Option<u32>),
        state: SessionState,
    ) -> Uuid {
        let prepared = registry.prepare_again(request, account, again_of).unwrap();
        registry
            .opened_in_terminal(prepared.session_id, Some(window), wid)
            .unwrap();
        let context = registry
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        registry
            .report(LifecycleReport {
                model: None,
                collection: None,
                session_id: prepared.session_id,
                proof: context.proof,
                state,
                process_id: Some(42),
                process_identity: Some("42:1".into()),
                exit_code: (state == SessionState::Exited).then_some(0),
            })
            .unwrap();
        prepared.session_id
    }

    /// Prepare Start again on `source`, recording where it came from.
    fn again(registry: &mut SessionRegistry, source: Uuid, account: &AccountView) -> Uuid {
        let request = registry
            .start_again_request(source, Uuid::new_v4())
            .unwrap();
        registry
            .prepare_again(request, account, Some(source))
            .unwrap()
            .session_id
    }

    /// #1839 review, finding 5: a launch that never started (its ticket
    /// lapsed unredeemed) says nothing about its window, so Start again on
    /// its row opens a new window.
    #[test]
    fn a_launch_that_never_started_offers_no_window() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let prepared = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        registry
            .opened_in_terminal(prepared.session_id, Some(tab(1)), Some(7))
            .unwrap();
        // Its ticket lapses unredeemed: the row reads Failed.
        registry.sweep(Utc::now() + Duration::seconds(121)).unwrap();
        assert_eq!(
            registry.describe(prepared.session_id).unwrap().1,
            SessionState::Failed
        );
        let next = again(&mut registry, prepared.session_id, &account);
        assert_eq!(registry.again_window(next), None);
    }

    /// #1839 review, findings 1, 8 and 11, and #1853 review: Start again
    /// finds its window from the row it was prepared from, recorded by the
    /// daemon and kept across a restart, and only for a row that ended. A
    /// row written without a window id (#1839's) offers none. A launch that
    /// is not a Start again or Resume has none. Another session running in
    /// the same window does not matter.
    #[test]
    fn start_again_offers_the_ended_rows_window_only() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let source = ran_in(
            &mut registry,
            (coding(&account, home.path()), None),
            &account,
            (tab(1), Some(7)),
            SessionState::Exited,
        );
        let first = again(&mut registry, source, &account);
        assert_eq!(registry.again_window(first), Some(7));
        assert_eq!(
            SessionRegistry::open(&store).unwrap().again_window(first),
            Some(7),
            "kept across a restart"
        );
        assert_eq!(registry.again_window(source), None, "not a Start again");
        let fresh = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        assert_eq!(registry.again_window(fresh.session_id), None);

        let unbound = ran_in(
            &mut registry,
            (coding(&account, home.path()), None),
            &account,
            (tab(2), None),
            SessionState::Exited,
        );
        let from_unbound = again(&mut registry, unbound, &account);
        assert_eq!(registry.again_window(from_unbound), None, "no window id");

        let running = ran_in(
            &mut registry,
            (coding(&account, home.path()), None),
            &account,
            (tab(3), Some(9)),
            SessionState::Running,
        );
        let request = registry
            .start_again_request(source, Uuid::new_v4())
            .unwrap();
        ran_in(
            &mut registry,
            (request, Some(source)),
            &account,
            (tab(4), Some(7)),
            SessionState::Running,
        );
        let second = again(&mut registry, source, &account);
        assert_eq!(
            registry.again_window(second),
            Some(7),
            "beside a running one"
        );
        assert!(registry.describe(running).unwrap().1.holds_profile());
    }

    /// #1853 review: a launch is held while its terminal opens, so a second
    /// launch with the same ticket is refused; a launch whose terminal did
    /// not open is released and its ticket stands.
    #[test]
    fn a_held_launch_refuses_a_second_and_a_failed_one_is_released() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let account = AccountStore::open(&store)
            .unwrap()
            .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
            .unwrap();
        let mut registry = SessionRegistry::open(&store).unwrap();
        let prepared = registry
            .prepare(coding(&account, home.path()), &account)
            .unwrap();
        let (id, ticket) = (prepared.session_id, prepared.ticket.unwrap());
        registry.begin_terminal_launch(id, &ticket).unwrap();
        assert_eq!(
            registry.begin_terminal_launch(id, &ticket),
            Err(ManagedError::Conflict)
        );
        registry.terminal_launch_failed(id).unwrap();
        registry.begin_terminal_launch(id, &ticket).unwrap();
        registry
            .opened_in_terminal(id, Some(tab(5)), Some(3))
            .unwrap();
        registry.terminal_launch_failed(id).unwrap();
        assert_eq!(
            registry.begin_terminal_launch(id, &ticket),
            Err(ManagedError::Conflict),
            "an opened launch is not released"
        );
    }
}
