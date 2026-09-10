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
}
