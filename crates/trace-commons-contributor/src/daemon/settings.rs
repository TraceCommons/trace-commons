//! Daemon configuration: the knobs governing how patient, how chatty, and how
//! autonomous the background uploader is.
//!
//! These are persisted rather than read from the process environment because a
//! daemon started by a service manager inherits none of the user's shell
//! environment. Settings read from env would leave every upload refusing with
//! `pii-filter-unavailable` under systemd while working perfectly by hand.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, DAEMON_SETTINGS_FILE};
use crate::envelope::NearAiSettings;

pub const DAEMON_SETTINGS_SCHEMA: &str = "trace_commons.daemon_settings.v1";

/// How long a session must go unwritten before it counts as finished.
const DEFAULT_QUIESCENCE_SECS: u64 = 1800;
/// How often the watcher stats the session roots. Much finer than the
/// quiescence window, so the poll rate costs nothing in responsiveness.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
/// Minimum gap between digest notifications, so a busy day is one interruption
/// rather than a dozen.
const DEFAULT_DIGEST_INTERVAL_SECS: u64 = 14_400;
const DEFAULT_QUEUE_TTL_DAYS: i64 = 14;
/// A resumed session must grow by this factor to be worth re-uploading.
const DEFAULT_GROWTH_FACTOR: f64 = 2.0;
/// ...or by this many absolute bytes, which is what actually catches growth on
/// an already-large session.
const DEFAULT_GROWTH_MIN_NEW_BYTES: u64 = 65_536;
/// A session re-uploads at most this many times. Each re-upload re-sends the
/// whole file, so an unbounded count would pay the privacy-filter bill
/// repeatedly over the same text and dilute the contributor's own credit
/// through server-side duplicate clustering.
const DEFAULT_MAX_REUPLOADS: u32 = 3;
const DEFAULT_MAX_UPLOADS_PER_DAY: u32 = 50;
const DEFAULT_MAX_BYTES_PER_DAY: u64 = 209_715_200;
const DEFAULT_MAX_QUEUE_ENTRIES: usize = 500;
const DEFAULT_HISTORY_POLL_SECS: u64 = 1800;
/// A privacy-filter self-test from days ago proves nothing about the filter
/// now, so a long-lived process re-checks on this interval.
const DEFAULT_CANARY_INTERVAL_SECS: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonSettings {
    pub schema_version: String,
    pub poll_interval_secs: u64,
    pub quiescence_secs: u64,
    pub digest_interval_secs: u64,
    pub queue_ttl_days: i64,
    pub growth_factor: f64,
    pub growth_min_new_bytes: u64,
    pub max_reuploads: u32,
    pub max_uploads_per_day: u32,
    pub max_bytes_per_day: u64,
    pub max_queue_entries: usize,
    pub history_poll_secs: u64,
    pub canary_interval_secs: u64,
    /// Whether the daemon itself renders OS notifications. Off by default:
    /// the native applications render their own, and the daemon's shell-out
    /// path needs a desktop session it may not have.
    pub local_notifications: bool,
    /// Privacy-filter credentials, persisted so a service-managed daemon can
    /// reach the filter without a shell environment.
    pub near_ai: Option<NearAiSettings>,
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            schema_version: DAEMON_SETTINGS_SCHEMA.to_string(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            quiescence_secs: DEFAULT_QUIESCENCE_SECS,
            digest_interval_secs: DEFAULT_DIGEST_INTERVAL_SECS,
            queue_ttl_days: DEFAULT_QUEUE_TTL_DAYS,
            growth_factor: DEFAULT_GROWTH_FACTOR,
            growth_min_new_bytes: DEFAULT_GROWTH_MIN_NEW_BYTES,
            max_reuploads: DEFAULT_MAX_REUPLOADS,
            max_uploads_per_day: DEFAULT_MAX_UPLOADS_PER_DAY,
            max_bytes_per_day: DEFAULT_MAX_BYTES_PER_DAY,
            max_queue_entries: DEFAULT_MAX_QUEUE_ENTRIES,
            history_poll_secs: DEFAULT_HISTORY_POLL_SECS,
            canary_interval_secs: DEFAULT_CANARY_INTERVAL_SECS,
            local_notifications: false,
            near_ai: None,
        }
    }
}

impl DaemonSettings {
    /// Load persisted settings, falling back to defaults when the daemon has
    /// never been configured on this machine.
    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_SETTINGS_FILE)? else {
            return Ok(Self::default());
        };
        serde_json::from_slice(&body).context("parsing daemon settings")
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon settings")?;
        store.write_daemon_file(DAEMON_SETTINGS_FILE, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    #[test]
    fn settings_round_trip_through_the_store() {
        let (_d, store) = temp_store();
        let s = DaemonSettings {
            quiescence_secs: 60,
            ..Default::default()
        };
        s.save(&store).unwrap();
        assert_eq!(DaemonSettings::load(&store).unwrap().quiescence_secs, 60);
    }

    #[test]
    fn settings_default_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        let s = DaemonSettings::load(&store).unwrap();
        assert_eq!(s.quiescence_secs, DEFAULT_QUIESCENCE_SECS);
        assert_eq!(s.max_reuploads, DEFAULT_MAX_REUPLOADS);
        assert!(!s.local_notifications, "notifications must be opt-in");
        assert!(s.near_ai.is_none());
    }

    #[test]
    fn settings_are_written_readable_only_by_the_owner() {
        // near_ai carries an API key.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (_d, store) = temp_store();
            DaemonSettings::default().save(&store).unwrap();
            let meta = std::fs::metadata(store.daemon_path(DAEMON_SETTINGS_FILE)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }
}
