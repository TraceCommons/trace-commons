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
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// What `save` last actually wrote, and where: the store directory it was
/// written to, and a digest of the exact bytes.
///
/// The daemon saves state at the end of every sixty-second tick whether or
/// not the tick moved anything, and on the corpus this was measured against
/// that is a 1.24 MB serialize, write and `fsync` per minute -- around
/// 1.8 GB of writes a day -- for bytes identical to the ones already on
/// disk. Remembering the digest lets an unchanged save skip the write.
///
/// The comparison is over the serialized bytes rather than a "did anything
/// change" flag on purpose. `observe()` rewrites the previous-size
/// bookkeeping for every path on every poll, so a flag set by mutation would
/// be true on every tick even though the map's contents never moved; and a
/// hand-maintained flag would go stale the first time a field is added
/// without a matching `mark_dirty`. Bytes cannot miss a field.
///
/// Excluded from `PartialEq` (and from serde) because it is a write-elision
/// memo, not state: two `DaemonState`s holding the same data are equal
/// whatever either has last written.
#[derive(Debug, Clone, Default)]
pub struct LastWritten(Option<(PathBuf, [u8; 32])>);

impl PartialEq for LastWritten {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
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
    /// When a timed pause lapses. `None` means either not paused, or paused
    /// with no timer. Persisted so a pause set by one process (the app) is
    /// honored by another (the daemon), and survives a restart of either --
    /// an app-side timer alone would die with the app and silently fail to
    /// resume the daemon.
    #[serde(default)]
    pub paused_until: Option<DateTime<Utc>>,
    /// When history was last refreshed from the server.
    #[serde(default)]
    pub last_history_poll_at: Option<DateTime<Utc>>,
    /// When an out-of-band history read-back falls due, set by an upload
    /// pass that actually sent something.
    ///
    /// A trace that has just been uploaded has no verdict yet, and waiting
    /// out the full `history_poll_secs` meant up to half an hour in which a
    /// successful submission and a broken one looked identical. This is the
    /// deadline for the read-back that closes that window; it holds the
    /// *earliest* pending one, so a burst of uploads is one refresh rather
    /// than one per upload.
    ///
    /// Persisted with the rest of the counters so a daemon restarted inside
    /// the window still performs the read-back rather than falling back to
    /// the half-hour interval. `#[serde(default)]` so a state file written
    /// before this field existed parses.
    #[serde(default)]
    pub history_refresh_due_at: Option<DateTime<Utc>>,
    /// When the public community roster was last fetched.
    #[serde(default)]
    pub last_community_poll_at: Option<DateTime<Utc>>,
    /// This contributor's line on the public roster, as the last poll found
    /// it. `None` means there is no standing to report -- no handle, no
    /// snapshot, or not on the roster -- and the clients then draw no
    /// community section at all.
    ///
    /// Cached here rather than in a file of its own so it lands inside the
    /// state that `ConfigStore::wipe` already removes: a public handle and
    /// the standing attached to it must not survive a wipe in a file nothing
    /// sweeps. It survives a restart on purpose, so the section is drawn
    /// immediately rather than after the first poll interval; the serve path
    /// re-checks its age (`community::CommunityStanding::is_fresh`) so a
    /// standing restored from disk can never outlive the roster's withdrawal
    /// bound.
    #[serde(default)]
    pub community: Option<super::community::CommunityStanding>,
    /// Write-elision memo; see [`LastWritten`]. Never persisted, so a fresh
    /// process always writes once before it can skip anything.
    #[serde(skip)]
    last_written: LastWritten,
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
            paused_until: None,
            last_history_poll_at: None,
            history_refresh_due_at: None,
            last_community_poll_at: None,
            community: None,
            last_written: LastWritten::default(),
        }
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_STATE_FILE)? else {
            return Ok(Self::new());
        };
        serde_json::from_slice(&body).context("parsing daemon state")
    }

    /// Persist, unless the exact bytes are already on disk.
    ///
    /// Takes `&mut self` so it can record what it wrote. The skip is
    /// conditioned on the destination file still existing as well as on the
    /// digest matching, so a `ConfigStore::wipe` (or anything else removing
    /// the file underneath a running daemon) is followed by a real write on
    /// the next save rather than by a memo insisting the file is already
    /// correct. That keeps the observable behaviour identical to the
    /// unconditional write it replaces in every case except the one being
    /// elided: bytes that are already there.
    pub fn save(&mut self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(&*self).context("serializing daemon state")?;
        let digest: [u8; 32] = Sha256::digest(&body).into();
        let already_on_disk = self
            .last_written
            .0
            .as_ref()
            .is_some_and(|(dir, seen)| dir == store.dir() && *seen == digest)
            && store.daemon_path(DAEMON_STATE_FILE).exists();
        if already_on_disk {
            return Ok(());
        }
        store.write_daemon_file(DAEMON_STATE_FILE, &body)?;
        self.last_written = LastWritten(Some((store.dir().to_path_buf(), digest)));
        Ok(())
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

    /// Overwrite the state file with bytes `save` would never produce, so a
    /// later write is detectable as the sentinel being gone.
    ///
    /// A write counter is what these tests actually need, and an identical
    /// rewrite is invisible from the file's contents alone -- the bytes are
    /// the same either way. Planting a sentinel makes "was this file
    /// written?" observable without depending on mtime granularity or on a
    /// unix-only inode.
    fn plant_sentinel(store: &ConfigStore) {
        std::fs::write(store.daemon_path(DAEMON_STATE_FILE), b"SENTINEL").unwrap();
    }

    fn sentinel_survived(store: &ConfigStore) -> bool {
        std::fs::read(store.daemon_path(DAEMON_STATE_FILE)).unwrap() == b"SENTINEL"
    }

    #[test]
    fn a_save_of_unchanged_state_does_not_write() {
        // The daemon saves at the end of every sixty-second tick whether or
        // not the tick moved anything: a 1.24 MB serialize, write and fsync
        // a minute on the measured corpus, for bytes already on disk.
        let (_d, store) = temp_store();
        let mut s = DaemonState::new();
        s.observe(Path::new("/tmp/a.jsonl"), 42);
        s.save(&store).unwrap();

        plant_sentinel(&store);
        s.save(&store).unwrap();
        s.save(&store).unwrap();
        assert!(
            sentinel_survived(&store),
            "two saves of unchanged state must not touch the file"
        );
    }

    #[test]
    fn a_save_after_a_real_change_writes() {
        // The other half: the elision must not swallow a genuine change.
        // `observe` of the SAME size for the same path is deliberately not a
        // change -- that is what happens on every poll for every path -- but
        // a new path, or a new size for a known one, is.
        let (_d, store) = temp_store();
        let mut s = DaemonState::new();
        s.observe(Path::new("/tmp/a.jsonl"), 42);
        s.save(&store).unwrap();

        plant_sentinel(&store);
        s.observe(Path::new("/tmp/a.jsonl"), 42);
        s.save(&store).unwrap();
        assert!(
            sentinel_survived(&store),
            "re-observing the same size is not a change"
        );

        s.observe(Path::new("/tmp/a.jsonl"), 99);
        s.save(&store).unwrap();
        assert!(!sentinel_survived(&store), "a moved size must be written");
        assert_eq!(
            DaemonState::load(&store)
                .unwrap()
                .previous_size(Path::new("/tmp/a.jsonl")),
            Some(99),
            "and the written bytes must be the new state"
        );

        plant_sentinel(&store);
        s.observe(Path::new("/tmp/b.jsonl"), 7);
        s.save(&store).unwrap();
        assert!(!sentinel_survived(&store), "a new path must be written");
    }

    #[test]
    fn a_save_writes_again_once_the_file_has_disappeared() {
        // `ConfigStore::wipe` removes the state file underneath whatever is
        // holding the state in memory. The memo must not then insist the
        // file is already correct: nothing is there at all.
        let (_d, store) = temp_store();
        let mut s = DaemonState::new();
        s.observe(Path::new("/tmp/a.jsonl"), 42);
        s.save(&store).unwrap();
        std::fs::remove_file(store.daemon_path(DAEMON_STATE_FILE)).unwrap();

        s.save(&store).unwrap();
        assert_eq!(
            DaemonState::load(&store)
                .unwrap()
                .previous_size(Path::new("/tmp/a.jsonl")),
            Some(42),
            "an unchanged save must still write when the file is gone"
        );
    }

    #[test]
    fn a_save_to_a_different_store_always_writes() {
        // The memo records where it wrote, not just what: the same state
        // handed a second store has never been written there.
        let (_a, first) = temp_store();
        let (_b, second) = temp_store();
        let mut s = DaemonState::new();
        s.observe(Path::new("/tmp/a.jsonl"), 42);
        s.save(&first).unwrap();
        s.save(&second).unwrap();
        assert_eq!(
            DaemonState::load(&second)
                .unwrap()
                .previous_size(Path::new("/tmp/a.jsonl")),
            Some(42)
        );
    }

    #[test]
    fn state_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(DaemonState::load(&store).unwrap(), DaemonState::new());
    }
}
