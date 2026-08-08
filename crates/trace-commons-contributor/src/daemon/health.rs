//! Daemon-level health, and the fixed label vocabulary that describes it.
//!
//! A background process fails silently unless something surfaces the failure.
//! These four states in particular are what a tray or window has to be able
//! to render: not logged in, claim minting failing, privacy filter
//! unreachable, ingest unreachable. Each is a condition the contributor may
//! need to act on, and none of them is visible from the queue alone.
//!
//! Everything here is a fixed label. No response body, no error message from a
//! server, no path, no token ever becomes a health state.
//!
//! Health also gates queue expiry. Conditions the contributor cannot act
//! around should not burn the fourteen-day clock on their pending traces.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// No usable enrollment: no config, or no device key.
pub const LABEL_NOT_LOGGED_IN: &str = "not-logged-in";
/// A privacy filter is configured but could not be constructed or reached.
pub const LABEL_PII_FILTER_UNAVAILABLE: &str = "pii-filter-unavailable";
/// The issuer refused or failed to mint an upload claim.
pub const LABEL_CLAIM_MINT_FAILED: &str = "claim-mint-failed";
/// The ingest endpoint could not be reached after retries.
pub const LABEL_INGEST_UNREACHABLE: &str = "ingest-unreachable";
/// A daily volume cap is in force until the UTC day rolls over.
pub const LABEL_DAILY_CAP_REACHED: &str = "daily-cap-reached";
/// The NEAR AI first-use notice has not been delivered interactively yet, so
/// the daemon will not send anything through that filter.
pub const LABEL_NEAR_AI_NOTICE_PENDING: &str = "near-ai-notice-not-acknowledged";
/// The privacy-filter canary self-test failed.
pub const LABEL_CANARY_FAILED: &str = "privacy-filter-canary-failed";
/// The queue is at its configured maximum.
pub const LABEL_QUEUE_FULL: &str = "queue-full";

/// Labels describing a condition the contributor cannot resolve by making a
/// decision about a trace. While one of these is in force, pending entries do
/// not age out.
const EXPIRY_BLOCKING_LABELS: [&str; 7] = [
    LABEL_NOT_LOGGED_IN,
    LABEL_PII_FILTER_UNAVAILABLE,
    LABEL_CLAIM_MINT_FAILED,
    LABEL_INGEST_UNREACHABLE,
    LABEL_DAILY_CAP_REACHED,
    LABEL_NEAR_AI_NOTICE_PENDING,
    LABEL_CANARY_FAILED,
];

/// Return the precedence of a health label, where lower values indicate higher
/// priority. Actionable conditions (that the contributor can act on) outrank
/// transient ones (that can only be waited out). When multiple conditions are
/// present, the one with the lowest precedence value is shown.
pub fn precedence(label: &str) -> u8 {
    match label {
        LABEL_NOT_LOGGED_IN => 0,
        LABEL_NEAR_AI_NOTICE_PENDING => 1,
        LABEL_CANARY_FAILED => 2,
        LABEL_PII_FILTER_UNAVAILABLE => 3,
        LABEL_CLAIM_MINT_FAILED => 4,
        LABEL_INGEST_UNREACHABLE => 5,
        LABEL_QUEUE_FULL => 6,
        LABEL_DAILY_CAP_REACHED => 7,
        _ => 8,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthState {
    pub last_error_label: Option<String>,
    pub since: Option<DateTime<Utc>>,
}

impl HealthState {
    pub fn ok(&self) -> bool {
        self.last_error_label.is_none()
    }

    /// Record a failure. Re-recording the same label keeps the original
    /// `since`, so a consumer can show how long a condition has persisted
    /// rather than how recently it was retried.
    ///
    /// When the incoming label has worse (higher) precedence than the current
    /// one, the current label is kept. This ensures actionable conditions
    /// (like "not logged in") outrank transient ones (like "ingest unreachable")
    /// in the menu bar.
    pub fn fail(&mut self, label: &str, now: DateTime<Utc>) {
        let current_label = self.last_error_label.as_deref();

        // Check if we should replace the current label
        let should_replace = match current_label {
            None => true, // No current label, always replace
            Some(current) => {
                if current == label {
                    // Same label: don't update, keep original `since`
                    false
                } else {
                    // Different label: replace only if new one has better precedence
                    precedence(label) < precedence(current)
                }
            }
        };

        if should_replace {
            if current_label != Some(label) {
                // New label, update `since`
                self.since = Some(now);
            }
            self.last_error_label = Some(label.to_string());
        }
    }

    pub fn clear(&mut self) {
        self.last_error_label = None;
        self.since = None;
    }

    /// Whether the current condition suspends queue expiry.
    pub fn blocks_expiry(&self) -> bool {
        self.last_error_label
            .as_deref()
            .is_some_and(|l| EXPIRY_BLOCKING_LABELS.contains(&l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_fresh_state_is_healthy() {
        let h = HealthState::default();
        assert!(h.ok());
        assert!(!h.blocks_expiry());
    }

    #[test]
    fn labels_the_contributor_cannot_act_on_suspend_expiry() {
        let mut h = HealthState::default();
        h.fail(LABEL_PII_FILTER_UNAVAILABLE, at("2026-08-08T12:00:00Z"));
        assert!(!h.ok());
        assert!(h.blocks_expiry());
        h.clear();
        assert!(h.ok());
        assert!(!h.blocks_expiry());
    }

    #[test]
    fn every_blocking_label_actually_blocks() {
        for label in EXPIRY_BLOCKING_LABELS {
            let mut h = HealthState::default();
            h.fail(label, at("2026-08-08T12:00:00Z"));
            assert!(h.blocks_expiry(), "{label} should suspend expiry");
        }
    }

    #[test]
    fn a_non_blocking_label_leaves_expiry_running() {
        // Queue-full is a backlog problem, not an outage: the contributor can
        // resolve it by deciding about traces, so the clock keeps running.
        let mut h = HealthState::default();
        h.fail(LABEL_QUEUE_FULL, at("2026-08-08T12:00:00Z"));
        assert!(!h.blocks_expiry());
    }

    #[test]
    fn repeating_a_label_preserves_when_it_started() {
        // Consumers show how long a condition has persisted.
        let mut h = HealthState::default();
        h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:00:00Z"));
        h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T13:00:00Z"));
        assert_eq!(h.since, Some(at("2026-08-08T12:00:00Z")));
    }

    #[test]
    fn a_different_label_restarts_the_clock() {
        let mut h = HealthState::default();
        h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:00:00Z"));
        h.fail(LABEL_NOT_LOGGED_IN, at("2026-08-08T13:00:00Z"));
        assert_eq!(h.since, Some(at("2026-08-08T13:00:00Z")));
    }

    #[test]
    fn a_higher_priority_condition_replaces_a_lower_one() {
        // Not-logged-in outranks a transient network problem: the contributor can
        // act on the first and only wait out the second.
        let mut h = HealthState::default();
        h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:00:00Z"));
        h.fail(LABEL_NOT_LOGGED_IN, at("2026-08-08T12:01:00Z"));
        assert_eq!(h.last_error_label.as_deref(), Some(LABEL_NOT_LOGGED_IN));
    }

    #[test]
    fn a_lower_priority_condition_does_not_displace_a_higher_one() {
        let mut h = HealthState::default();
        h.fail(LABEL_NOT_LOGGED_IN, at("2026-08-08T12:00:00Z"));
        h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:01:00Z"));
        assert_eq!(h.last_error_label.as_deref(), Some(LABEL_NOT_LOGGED_IN));
    }

    #[test]
    fn the_documented_order_is_total_and_has_no_ties() {
        let order = [
            LABEL_NOT_LOGGED_IN,
            LABEL_NEAR_AI_NOTICE_PENDING,
            LABEL_CANARY_FAILED,
            LABEL_PII_FILTER_UNAVAILABLE,
            LABEL_CLAIM_MINT_FAILED,
            LABEL_INGEST_UNREACHABLE,
            LABEL_QUEUE_FULL,
            LABEL_DAILY_CAP_REACHED,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for l in order {
            assert!(seen.insert(precedence(l)), "duplicate precedence for {l}");
        }
        for pair in order.windows(2) {
            assert!(
                precedence(pair[0]) < precedence(pair[1]),
                "{} must outrank {}",
                pair[0],
                pair[1]
            );
        }
    }
}
