//! Source-owned compute prose. Identities travel with text, independently of
//! state/reason, which may change without replacing the displayed message.

use super::policy::PolicyReason;

#[derive(Debug, Clone, Copy)]
pub struct Message {
    pub key: &'static str,
    pub english: &'static str,
}

macro_rules! messages {
    ($($name:ident => ($key:literal, $english:literal)),+ $(,)?) => {
        $(pub const $name: Message = Message { key: $key, english: $english };)+
        pub const MESSAGES: &[Message] = &[$($name),+];
    };
}

messages! {
    ALLOWANCE => ("title.allowance", "Check RAM allowance"),
    INVALID_ALLOWANCE => ("detail.invalid_allowance", "Enter a whole number above zero."),
    DISABLED => ("title.disabled", "Compute is disabled"),
    EMPTY => ("detail.empty", ""),
    SAVE_FAILED => ("title.save_failed", "Couldn't save compute settings"),
    NOTHING_CHANGED => ("detail.nothing_changed", "Nothing changed."),
    PAUSED => ("title.paused", "Compute is paused"),
    NO_AUTO_RESUME => ("detail.no_auto_resume", "It never resumes on its own after a restart."),
    NOT_AVAILABLE => ("detail.not_available", "Not available in this build."),
    UNAVAILABLE => ("title.unavailable", "Compute is unavailable"),
    STATUS_UNAVAILABLE => ("title.status_unavailable", "Compute status is unavailable"),
    RESTART => ("detail.restart", "Restart the app to try again."),
    LOCAL => ("title.local", "Local compute development"),
    LOCAL_ONLY => ("detail.local_only", "Local test worker only. No pool, no earnings."),
    STOPPING => ("title.stopping", "Stopping compute"),
    WAITING_FOR_STOP => ("detail.waiting_for_stop", "Waiting for the worker to stop."),
    STALE => ("title.stale", "Compute status is stale"),
    UNKNOWN_PROGRESS => ("detail.unknown_progress", "Progress unknown."),
    CANNOT_START => ("title.cannot_start", "Compute cannot start"),
    TRAINING => ("title.training", "Training"),
    SERVING => ("title.serving", "Serving"),
    DRAINING => ("title.draining", "Compute is draining"),
    WAITING => ("title.waiting", "Waiting for work"),
    STOPPED => ("title.stopped", "Compute is stopped"),
    WORKER_EXITED => ("detail.worker_exited", "The worker has exited."),
    STOP_FAILED => ("title.stop_failed", "Couldn't stop compute"),
    KEEP_OPEN => ("detail.keep_open", "Keep the app open."),
    SETTINGS_UNSAVED => ("title.settings_unsaved", "Compute settings could not be saved"),
    INVALID_OR_UNSAVED => ("detail.invalid_or_unsaved", "The RAM allowance is invalid or wasn't saved."),
    CONSENT_WITHDRAWN => ("detail.consent_withdrawn", "Consent withdrawn."),
    STARTING => ("title.starting", "Starting local compute"),
    CHECKING_WORKER => ("detail.checking_worker", "Checking the worker."),
    ALREADY_RUNNING => ("title.already_running", "Another worker is running"),
    STOP_OTHER => ("detail.stop_other", "Stop it from the app that started it, or restart the computer."),
    START_FAILED => ("title.start_failed", "Compute could not start"),
    FAILED_CHECKS => ("detail.failed_checks", "The worker failed its checks and was stopped."),
    STATUS_LOST => ("title.status_lost", "Compute stopped after losing status"),
    CHECK_SETUP => ("detail.check_setup", "Check the worker setup."),
}

const POLICY_REASONS: &[PolicyReason] = &[
    PolicyReason::MissingObservation,
    PolicyReason::StaleObservation,
    PolicyReason::InvalidClock,
    PolicyReason::Sleeping,
    PolicyReason::Power,
    PolicyReason::LowPowerMode,
    PolicyReason::UnknownObservation,
    PolicyReason::Thermal,
    PolicyReason::CriticalThermal,
    PolicyReason::Memory,
    PolicyReason::CriticalMemory,
    PolicyReason::Paused,
    PolicyReason::Disabled,
    PolicyReason::Shutdown,
];

/// Policy owns its canonical detail wording; its closed label is an identity,
/// never inferred from incoming English or the snapshot's current reason.
pub fn policy(reason: PolicyReason) -> Message {
    Message {
        key: reason.label(),
        english: reason.detail(),
    }
}

pub fn messages() -> impl Iterator<Item = Message> {
    MESSAGES
        .iter()
        .copied()
        .chain(POLICY_REASONS.iter().copied().map(policy))
}

pub fn find(key: &str) -> Option<Message> {
    messages().find(|message| message.key == key)
}

#[cfg(test)]
pub(crate) fn assert_snapshot(snapshot: &super::ComputeSnapshot) {
    assert_eq!(find(snapshot.title_key).unwrap().english, snapshot.title);
    assert_eq!(find(snapshot.detail_key).unwrap().english, snapshot.detail);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_unique_and_policy_messages_use_policy_owned_wording() {
        let mut keys = std::collections::BTreeSet::new();
        for message in messages() {
            assert!(
                keys.insert(message.key),
                "duplicate identity: {}",
                message.key
            );
        }
        for reason in POLICY_REASONS {
            assert_eq!(find(policy(*reason).key).unwrap().english, reason.detail());
        }
    }
}
