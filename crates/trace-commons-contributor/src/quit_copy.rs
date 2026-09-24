//! What quitting says, in one place, for every shell.
//!
//! Quitting must say what keeps running, and the true sentence depends on
//! which process is doing the watching (the shared shell design spec,
//! "Quit is explicit about consequences"):
//!
//! - A shell that HOSTS the watcher in-process stops it by quitting, because
//!   the app *is* it.
//! - A shell ATTACHED to a watcher some other process runs (a CLI, a service
//!   manager, another shell) leaves that watcher running.
//! - A shell with no watcher at all -- none started yet, or an attached one
//!   whose connection dropped -- cannot claim either. It says only what is
//!   true in both cases: quitting it stops nothing.
//!
//! Getting it wrong is a lie about whether the machine is still watching,
//! so the choice lives here with the sentences rather than in each shell.
//!
//! Stopping an attached watcher from the quit prompt is deliberately not a
//! shared action: a shell that talks to the watcher through
//! [`crate::daemon::attached::AttachedDaemon`] cannot send `shutdown` (it is
//! refused before it reaches the wire), so offering it there would be a
//! button that does nothing.

use serde::Serialize;

/// The prompt's heading.
pub const QUIT_TITLE: &str = concat!("Quit ", crate::app_name!(), "?");

/// This process is the watcher.
pub const QUIT_HOSTING_BODY: &str = concat!(
    "Quitting stops ",
    crate::app_name!(),
    " watching for finished sessions. Nothing is queued or sent until you open it again. \
     Anything already waiting stays waiting."
);

/// This process is attached to a watcher another process runs.
pub const QUIT_ATTACHED_BODY: &str = "The background watcher keeps running and will keep queuing \
     sessions. Nothing will be sent while nobody's approving.";

/// This process has no watcher to stop.
pub const QUIT_UNAVAILABLE_BODY: &str = "This app isn't connected to a watcher right now, so \
     quitting doesn't stop one. Anything already waiting stays waiting.";

pub const QUIT_CONFIRM: &str = "Quit";
pub const QUIT_CANCEL: &str = "Cancel";

/// Which process is doing the watching, from the quitting shell's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuitRole {
    /// This process runs the watcher; quitting stops it.
    Hosting,
    /// This process is a client of a watcher that outlives it.
    Attached,
    /// No watcher is reachable from this process.
    Unavailable,
}

/// Every word of one quit prompt, already chosen for the role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuitPrompt {
    pub role: QuitRole,
    pub title: &'static str,
    pub body: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
}

/// The quit prompt for a shell in `role`.
pub fn quit_prompt(role: QuitRole) -> QuitPrompt {
    QuitPrompt {
        role,
        title: QUIT_TITLE,
        body: match role {
            QuitRole::Hosting => QUIT_HOSTING_BODY,
            QuitRole::Attached => QUIT_ATTACHED_BODY,
            QuitRole::Unavailable => QUIT_UNAVAILABLE_BODY,
        },
        confirm: QUIT_CONFIRM,
        cancel: QUIT_CANCEL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_role_gets_its_own_sentence() {
        assert_eq!(quit_prompt(QuitRole::Hosting).body, QUIT_HOSTING_BODY);
        assert_eq!(quit_prompt(QuitRole::Attached).body, QUIT_ATTACHED_BODY);
        assert_eq!(
            quit_prompt(QuitRole::Unavailable).body,
            QUIT_UNAVAILABLE_BODY
        );
    }

    /// Only the hosting process may say quitting stops the watcher.
    #[test]
    fn only_hosting_claims_quitting_stops_watching() {
        assert!(QUIT_HOSTING_BODY.starts_with("Quitting stops Trace Commons watching"));
        for role in [QuitRole::Attached, QuitRole::Unavailable] {
            let body = quit_prompt(role).body;
            assert!(
                !body.contains("Quitting stops"),
                "{role:?} must not claim quitting stops the watcher: {body}"
            );
        }
        assert!(QUIT_ATTACHED_BODY.contains("keeps running"));
    }

    #[test]
    fn role_serializes_as_a_fixed_label() {
        let value = serde_json::to_value(quit_prompt(QuitRole::Unavailable)).unwrap();
        assert_eq!(value["role"], "unavailable");
        assert_eq!(value["title"], "Quit Trace Commons?");
        assert_eq!(value["confirm"], "Quit");
        assert_eq!(value["cancel"], "Cancel");
        assert_eq!(serde_json::to_value(QuitRole::Hosting).unwrap(), "hosting");
        assert_eq!(
            serde_json::to_value(QuitRole::Attached).unwrap(),
            "attached"
        );
    }
}
