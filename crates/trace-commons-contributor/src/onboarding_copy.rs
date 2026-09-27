//! Shared onboarding and digest-permission words for every native shell.

pub const WELCOME_BODY: &str = "This app finds finished coding-agent sessions on this machine according to your source settings. Review those settings to see which folders are read.";
pub const DONE_BODY: &str = "Review waiting sessions in the queue. Notifications summarize sessions to review and recent contributions.";
pub const NOTIFICATION_PURPOSE: &str = "Notifications tell you about sessions waiting for review and recent contributions. They never submit a session for you.";
pub const NOTIFICATION_HEADING: &str = "Notifications";
pub const NOTIFICATION_OFFER: &str = "Let Trace Commons notify you?";
pub const NOTIFICATION_ALLOWED: &str = "Notifications allowed";
pub const NOTIFICATION_DENIED: &str = "Notifications turned off in System Settings";
pub const NOTIFICATION_UNKNOWN: &str = "Notification permission could not be determined";
pub const NOTIFICATION_NOT_ASKED: &str = "Not asked yet";
pub const NOTIFICATION_ALLOW: &str = "Allow notifications";
pub const NOT_NOW: &str = "Not now";
pub const SYSTEM_SETTINGS: &str = "Open System Settings";
/// Names the two rows that gate the watcher (`daemon::settings::roots_declared`).
/// Not "each": a blank Gemini, Cline, or OpenCode row never blocks it.
pub const ROOTS_REQUIRED: &str = "Answer for Claude Code and Codex before continuing.";
pub const WATCHER_STARTING: &str = "Starting the watcher…";
pub const WATCHER_START_FAILED: &str = "The watcher didn't start, so nothing is being watched yet. Your folder answers are saved; retry to start it.";

#[derive(serde::Serialize)]
pub struct OnboardingCopy {
    pub welcome_body: &'static str,
    pub done_body: &'static str,
    pub notification_purpose: &'static str,
    pub notification_heading: &'static str,
    pub notification_offer: &'static str,
    pub notification_allowed: &'static str,
    pub notification_denied: &'static str,
    pub notification_unknown: &'static str,
    pub notification_not_asked: &'static str,
    pub notification_allow: &'static str,
    pub not_now: &'static str,
    pub system_settings: &'static str,
    pub roots_required: &'static str,
    pub watcher_starting: &'static str,
    pub watcher_start_failed: &'static str,
}

#[must_use]
pub fn onboarding_copy() -> OnboardingCopy {
    OnboardingCopy {
        welcome_body: WELCOME_BODY,
        done_body: DONE_BODY,
        notification_purpose: NOTIFICATION_PURPOSE,
        notification_heading: NOTIFICATION_HEADING,
        notification_offer: NOTIFICATION_OFFER,
        notification_allowed: NOTIFICATION_ALLOWED,
        notification_denied: NOTIFICATION_DENIED,
        notification_unknown: NOTIFICATION_UNKNOWN,
        notification_not_asked: NOTIFICATION_NOT_ASKED,
        notification_allow: NOTIFICATION_ALLOW,
        not_now: NOT_NOW,
        system_settings: SYSTEM_SETTINGS,
        roots_required: ROOTS_REQUIRED,
        watcher_starting: WATCHER_STARTING,
        watcher_start_failed: WATCHER_START_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_preserves_configured_discovery_and_digest_contracts() {
        let value = serde_json::to_value(onboarding_copy()).unwrap();
        assert_eq!(value["welcome_body"], WELCOME_BODY);
        assert_eq!(value["done_body"], DONE_BODY);
        assert!(WELCOME_BODY.contains("according to your source settings"));
        assert!(DONE_BODY.contains("sessions to review"));
        assert!(!DONE_BODY.contains("4 hours"));
        assert!(!DONE_BODY.contains("reminder settings"));
        assert!(NOTIFICATION_PURPOSE.contains("never submit"));
        assert_ne!(NOTIFICATION_UNKNOWN, NOTIFICATION_ALLOWED);
    }

    /// The roots screen names exactly the two rows that gate the watcher
    /// (`daemon::settings::roots_declared`), and a failed start says nothing
    /// is being watched rather than implying the setup went through.
    #[test]
    fn roots_copy_names_the_gating_rows_and_a_failed_start_is_truthful() {
        let value = serde_json::to_value(onboarding_copy()).unwrap();
        assert_eq!(value["roots_required"], ROOTS_REQUIRED);
        assert_eq!(value["watcher_starting"], WATCHER_STARTING);
        assert_eq!(value["watcher_start_failed"], WATCHER_START_FAILED);
        assert!(ROOTS_REQUIRED.contains("Claude Code and Codex"));
        assert!(!ROOTS_REQUIRED.contains("each"));
        assert!(WATCHER_START_FAILED.contains("nothing is being watched"));
        assert!(!WATCHER_START_FAILED.to_lowercase().contains("invite"));
    }
}
