//! Local saved accounts and explicitly managed native-tool sessions.

pub mod accounts;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    NearAi,
    Subscription,
    ApiKey,
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
    #[error("managed account or session not found")]
    NotFound,
    #[error("account label is ambiguous; select an account ID")]
    AmbiguousAccount,
    #[error("managed storage unavailable")]
    StorageUnavailable,
    #[error("unsupported native tool version")]
    UnsupportedVersion,
    #[error("sign in to this account before launching")]
    SignInRequired,
    #[error("start the Trace Commons daemon before launching")]
    DaemonUnavailable,
    #[error("launch status unknown; reconcile before retrying")]
    LaunchUnknown,
    #[error("invalid managed request")]
    InvalidRequest,
}
