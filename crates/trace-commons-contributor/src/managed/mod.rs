//! Local saved accounts and explicitly managed native-tool sessions.

pub mod accounts;
pub mod cli;
pub mod codex;
pub mod codex_hooks;
pub mod collection_plan;
pub mod folder;
pub mod models;
pub mod native_collection;
pub mod opencode;
pub mod options;
pub mod profiles;
pub mod secrets;
pub mod session_binding;
pub mod sessions;
pub mod signin;
pub mod statusline;
pub mod supervisor;
pub mod tool_setup;

/// Stdin, read for at most `budget` and at most `max` bytes, for the hidden
/// commands a coding tool runs (hooks, the status line). A terminal, a
/// read error, or a writer that does not close stdin in time yields none.
/// The reader is a detached thread, so one blocked on stdin never holds the
/// process open.
pub fn read_stdin_bounded(budget: std::time::Duration, max: u64) -> Option<Vec<u8>> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = std::io::stdin().take(max).read_to_end(&mut bytes).ok();
        let _ = sender.send(read.map(|_| bytes));
    });
    receiver.recv_timeout(budget).ok().flatten()
}

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    Claude,
    Codex,
    Opencode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    NearAi,
    Subscription,
    ApiKey,
    /// Runs on the app's NEAR AI sign-in. Derived, never saved: see
    /// [`signin`]. Not offered on the command line's `--connection`.
    #[serde(rename = "near_ai_signin")]
    #[value(skip)]
    NearAiSignIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub Uuid);

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Generation(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    SignInRequired,
    Checking,
    Ready,
    Unavailable,
}

/// This projection is safe for local UI IPC. It never holds credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountView {
    pub id: AccountId,
    pub tool: ToolId,
    pub connection: ConnectionKind,
    pub label: String,
    pub auth_state: AuthState,
    pub verified_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub tool: ToolId,
    pub connection: ConnectionKind,
    pub account_id: AccountId,
    pub generation: Generation,
}

/// Fixed error vocabulary: paths, account labels and credentials stay out of logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ManagedError {
    #[error("managed selection conflict; refresh and try again")]
    Conflict,
    #[error("trace collection requires exclusive access to this managed account")]
    CollectionAccountBusy,
    #[error("managed account or session not found")]
    NotFound,
    #[error("account label is ambiguous; select an account ID")]
    AmbiguousAccount,
    #[error("managed storage unavailable")]
    StorageUnavailable,
    #[error("native tool is not installed or is unavailable")]
    ToolUnavailable,
    #[error("unsupported native tool version")]
    UnsupportedVersion,
    #[error("OpenCode system settings prevent an isolated managed session")]
    SystemConfiguration,
    #[error("sign in to this account before launching")]
    SignInRequired,
    #[error("sign in to NEAR AI before launching")]
    NearAiSignInRequired,
    #[error("start the {} daemon before launching", crate::brand::APP_NAME)]
    DaemonUnavailable,
    #[error("launch status unknown; reconcile before retrying")]
    LaunchUnknown,
    #[error("invalid managed request")]
    InvalidRequest,
    #[error("invalid managed CLI options")]
    InvalidCliOptions,
    #[error("the chosen terminal app was not found")]
    TerminalNotFound,
    #[error("the chosen model is not available on NEAR AI")]
    ModelUnavailable,
    #[error("add funds to NEAR AI before launching")]
    NearAiBalanceTooLow,
    /// A NEAR AI Claude Code launch whose project, or a folder its CLI
    /// options add, chooses a model by id (owner's wording, 2026-11-01).
    #[error(
        "This project's Claude Code settings, agents, commands or skills choose a model. NEAR AI sessions use the session's models."
    )]
    ProjectChoosesModel,
}

/// The daemon's label for [`ManagedError::ProjectChoosesModel`], which the
/// CLI turns back into its sentence and the shells map to the shared
/// `project_chooses_model` line.
pub const PROJECT_CHOOSES_MODEL_LABEL: &str = "managed-project-chooses-model";

pub mod copy;
pub mod route;
pub mod terminal;
