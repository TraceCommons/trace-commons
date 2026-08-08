//! Watcher bookkeeping that has to survive a restart.
//!
//! Three things live here that cannot be derived from anything else:
//!
//! - **What we last uploaded per path.** Receipts are keyed by session hash,
//!   never by path, so nothing else on disk can answer "has this file been
//!   uploaded, and at what size?" -- the question bounded growth re-queue
//!   depends on.
//! - **The previous poll's size per path**, which is how a session still being
//!   written is told apart from one that is finished.
//! - **A working-directory cache**, because resolving a session's cwd means
//!   reading into the file, and doing that for every session on every poll is
//!   continuous disk churn on a laptop.
//!
//! Paths appear in this file and never leave it.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, DAEMON_STATE_FILE};

pub const DAEMON_STATE_SCHEMA: &str = "trace_commons.daemon_state.v1";

/// What the daemon last shipped for a given session file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorUpload {
    pub hash: String,
    pub size_bytes: u64,
    pub upload_count: u32,
}

/// A cached working directory, valid only while the file's size and mtime are
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CwdCacheEntry {
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonState {
    pub schema_version: String,
    pub cwd_cache: BTreeMap<String, CwdCacheEntry>,
    pub prior_uploads: BTreeMap<String, PriorUpload>,
    pub last_observation: BTreeMap<String, u64>,
    pub last_digest_at: Option<DateTime<Utc>>,
    /// UTC day the counters below belong to, as `YYYY-MM-DD`.
    pub day_bucket: Option<String>,
    pub uploads_today: u32,
    pub bytes_today: u64,
    /// Whether the contributor has paused the daemon. Persisted, so a pause
    /// survives a restart and is visible to a one-shot CLI invocation rather
    /// than living only in a running process's memory.
    #[serde(default)]
    pub paused: bool,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            schema_version: DAEMON_STATE_SCHEMA.to_string(),
            cwd_cache: BTreeMap::new(),
            prior_uploads: BTreeMap::new(),
            last_observation: BTreeMap::new(),
            last_digest_at: None,
            day_bucket: None,
            uploads_today: 0,
            bytes_today: 0,
            paused: false,
        }
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_STATE_FILE)? else {
            return Ok(Self::new());
        };
        serde_json::from_slice(&body).context("parsing daemon state")
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon state")?;
        store.write_daemon_file(DAEMON_STATE_FILE, &body)
    }

    /// Reset the daily volume counters when the UTC day has rolled over.
    /// Call before every cap check; without it a daemon running for a week
    /// would still be measuring against its first day.
    pub fn roll_day(&mut self, now: DateTime<Utc>) {
        let today = now.format("%Y-%m-%d").to_string();
        if self.day_bucket.as_deref() != Some(today.as_str()) {
            self.day_bucket = Some(today);
            self.uploads_today = 0;
            self.bytes_today = 0;
        }
    }

    /// Record a completed upload: both the per-path index that bounds growth
    /// re-queue and the daily volume counters.
    pub fn record_upload(&mut self, path: &Path, hash: &str, size_bytes: u64, now: DateTime<Utc>) {
        self.roll_day(now);
        let key = path.to_string_lossy().to_string();
        let count = self
            .prior_uploads
            .get(&key)
            .map(|p| p.upload_count)
            .unwrap_or(0);
        self.prior_uploads.insert(
            key,
            PriorUpload {
                hash: hash.to_string(),
                size_bytes,
                upload_count: count + 1,
            },
        );
        self.uploads_today = self.uploads_today.saturating_add(1);
        self.bytes_today = self.bytes_today.saturating_add(size_bytes);
    }

    /// The size this path had at the previous poll, if it was seen then.
    pub fn previous_size(&self, path: &Path) -> Option<u64> {
        self.last_observation
            .get(&path.to_string_lossy().to_string())
            .copied()
    }

    pub fn observe(&mut self, path: &Path, size_bytes: u64) {
        self.last_observation
            .insert(path.to_string_lossy().to_string(), size_bytes);
    }

    pub fn prior_upload(&self, path: &Path) -> Option<&PriorUpload> {
        self.prior_uploads.get(&path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn roll_day_resets_counters_on_a_utc_day_change() {
        let mut s = DaemonState::new();
        s.day_bucket = Some("2026-08-07".to_string());
        s.uploads_today = 9;
        s.bytes_today = 1234;
        s.roll_day(at("2026-08-08T00:00:01Z"));
        assert_eq!(s.uploads_today, 0);
        assert_eq!(s.bytes_today, 0);
        assert_eq!(s.day_bucket.as_deref(), Some("2026-08-08"));
    }

    #[test]
    fn roll_day_preserves_counters_within_the_same_day() {
        let mut s = DaemonState::new();
        s.roll_day(at("2026-08-08T01:00:00Z"));
        s.uploads_today = 3;
        s.roll_day(at("2026-08-08T23:59:00Z"));
        assert_eq!(s.uploads_today, 3);
    }

    #[test]
    fn record_upload_increments_the_count_for_the_same_path() {
        let mut s = DaemonState::new();
        let now = at("2026-08-08T01:00:00Z");
        s.record_upload(Path::new("/tmp/a.jsonl"), "sha256:aa", 10, now);
        s.record_upload(Path::new("/tmp/a.jsonl"), "sha256:bb", 30, now);
        let prior = s.prior_upload(Path::new("/tmp/a.jsonl")).unwrap();
        assert_eq!(prior.upload_count, 2);
        assert_eq!(prior.hash, "sha256:bb");
        assert_eq!(prior.size_bytes, 30);
        assert_eq!(s.uploads_today, 2);
        assert_eq!(s.bytes_today, 40);
    }

    #[test]
    fn record_upload_tracks_paths_independently() {
        let mut s = DaemonState::new();
        let now = at("2026-08-08T01:00:00Z");
        s.record_upload(Path::new("/tmp/a.jsonl"), "sha256:aa", 10, now);
        s.record_upload(Path::new("/tmp/b.jsonl"), "sha256:bb", 10, now);
        assert_eq!(
            s.prior_upload(Path::new("/tmp/a.jsonl"))
                .unwrap()
                .upload_count,
            1
        );
        assert_eq!(
            s.prior_upload(Path::new("/tmp/b.jsonl"))
                .unwrap()
                .upload_count,
            1
        );
    }

    #[test]
    fn observation_round_trips_per_path() {
        let mut s = DaemonState::new();
        assert_eq!(s.previous_size(Path::new("/tmp/a.jsonl")), None);
        s.observe(Path::new("/tmp/a.jsonl"), 42);
        assert_eq!(s.previous_size(Path::new("/tmp/a.jsonl")), Some(42));
    }

    #[test]
    fn state_round_trips_through_the_store() {
        let (_d, store) = temp_store();
        let mut s = DaemonState::new();
        s.record_upload(
            Path::new("/tmp/a.jsonl"),
            "sha256:aa",
            10,
            at("2026-08-08T01:00:00Z"),
        );
        s.save(&store).unwrap();
        let loaded = DaemonState::load(&store).unwrap();
        assert_eq!(loaded, s);
    }

    #[test]
    fn a_pause_survives_a_round_trip_through_the_store() {
        // Otherwise `daemon pause` would appear to work and then be forgotten
        // by the next command, and by the daemon on restart.
        let (_d, store) = temp_store();
        let mut s = DaemonState::new();
        s.paused = true;
        s.save(&store).unwrap();
        assert!(DaemonState::load(&store).unwrap().paused);
    }

    #[test]
    fn state_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(DaemonState::load(&store).unwrap(), DaemonState::new());
    }
}
