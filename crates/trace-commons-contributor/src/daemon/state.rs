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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ConfigStore, DAEMON_STATE_FILE};

pub const DAEMON_STATE_SCHEMA: &str = "trace_commons.daemon_state.v1";

/// How long the attention log keeps an entry. Derived, not an owner
/// decision: the longest window a cap counts over is one local ISO week
/// (owner decision 4's `STANDALONE_PER_WEEK` and the per-kind weekly caps),
/// and a week plus one day covers it from any local offset.
pub const ATTENTION_LOG_RETENTION: chrono::Duration = chrono::Duration::days(8);

/// Read the attention log entry by entry, dropping any this build cannot
/// parse. See `DaemonState::attention_log`.
fn readable_attention_entries<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<super::attention::AttentionEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect())
}

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
    /// The project key `cwd` resolves to, recorded when the entry is written
    /// so that answering "sessions per project" never canonicalizes a path.
    /// `None` on an entry written before this existed; it is filled in the
    /// first time it is needed.
    #[serde(default)]
    pub project_key: Option<String>,
    /// Which tool this session reads as having come from (K11), recorded
    /// when the entry is written: `SessionRef::declared_source` when the
    /// source that discovered it named one (an imported Antigravity
    /// conversation, say), otherwise the adapter's own name -- that is,
    /// `SessionRef::displayed_source`, the same preference the CLI's session
    /// table and the GTK shell's `agent_label` use for a contributor-facing
    /// name. For a staged import the declared name is self-declared and
    /// unverified; see that method's doc.
    ///
    /// `#[serde(default)]` so a state file written before this field
    /// existed still loads, and `None` is never backfilled retroactively:
    /// unlike `project_key`, which is re-derivable from `cwd`, the tool that
    /// discovered a session is not recoverable from where it ran. An entry
    /// whose size and mtime have not changed since before this field existed
    /// keeps reporting `None` until the file changes again and the cache
    /// entry is rewritten.
    #[serde(default)]
    pub tool: Option<String>,
    /// Which adapter actually discovered this session (K16, #1227 review):
    /// `SessionRef::source`, never `declared_source` or `displayed_source`.
    /// Recorded alongside `tool` when the entry is written, from the same
    /// `SessionRef`.
    ///
    /// `tool` above is the self-declared, contributor-facing name --
    /// unverified for a staged import, by that method's own doc -- and must
    /// stay that way for display. This field is the thing a consent
    /// decision is allowed to gate on: the adapter that actually read the
    /// bytes, which a staged file cannot spoof into claiming a different
    /// tool is switched on.
    ///
    /// `#[serde(default)]` so a state file written before this field
    /// existed still loads, and `None` is never backfilled retroactively,
    /// same as `tool`. `daemon::mission_matching::readable_sessions` reads
    /// this field, not `tool`, to decide whether a session may be read, and
    /// treats `None` as unreadable -- fails closed, rather than trusting
    /// the self-declared name while the real adapter is unknown.
    #[serde(default)]
    pub adapter: Option<String>,
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
    /// Nudge: the in-app suggestion ledger, keyed by kind label or
    /// `kind:opaque id` (`nudge::ledger_key`), never by a path or a folder
    /// label. Times only. Cleared by `unenroll` (`clear_nudges`), so a next
    /// account never inherits this one's "Not now"s; removed with the rest
    /// of this file by `ConfigStore::wipe`.
    ///
    /// `#[serde(default)]` so a file written before it existed loads, and
    /// not written while empty, so an install that never saw a suggestion
    /// keeps writing the bytes it always did.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nudges: BTreeMap<String, super::nudge::NudgeLedger>,
    /// Nudge U2: the high-water mark verdict news is diffed against, keyed
    /// by submission id (opaque) and pruned to ids still in the history
    /// cache. Only the daemon writes it, unlike the shared cache. Flags only.
    /// See `nudge::verdict_delta`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub verdict_marks: BTreeMap<String, super::nudge::VerdictMark>,
    /// Whether `verdict_marks` has been seeded. False on the first poll after
    /// an upgrade or after `unenroll`, which seeds the marks silently so
    /// history that already existed never reads as news; an empty mark map
    /// alone cannot tell "never seeded" from "seeded against an empty cache".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub verdict_marks_seeded: bool,
    /// Nudge U2: verdicts landed and not yet acknowledged, counts and times
    /// only. Cleared by `nudge_opened {kind: "verdicts_landed"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdicts_pending: Option<super::nudge::VerdictDelta>,
    /// Nudge U4: the idle candidates an announcement (folded into a digest
    /// or standalone) has already named, by queue entry id (opaque), so the
    /// next batch names only new ones and never one session at a time.
    /// Pruned to entries still `Pending` (`nudge::prune_idle_announced`) and
    /// cleared by `unenroll` (`clear_nudges`). Never a path.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub idle_announced: BTreeSet<uuid::Uuid>,
    /// Nudge A2: every notification the arbiter announced, folded or
    /// standalone, as kind label, route and time only -- the budget the
    /// global and per-kind caps are counted against. Pruned to
    /// [`ATTENTION_LOG_RETENTION`] by `record_attention` and
    /// `prune_attention_log`. Cleared by `unenroll` (`clear_nudges`).
    ///
    /// An entry this build cannot read (a kind a newer build added, then a
    /// downgrade) is dropped on load rather than refusing the whole file,
    /// which would stop the daemon starting. That can only loosen the
    /// budget for the week the entry would have counted in.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "readable_attention_entries"
    )]
    pub attention_log: Vec<super::attention::AttentionEntry>,
    /// Nudge A2: when anything last notified -- the digest included -- for
    /// `attention::MIN_GAP_ANY`. Cleared by `unenroll` (`clear_nudges`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_notified_at: Option<DateTime<Utc>>,
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
            nudges: BTreeMap::new(),
            verdict_marks: BTreeMap::new(),
            verdict_marks_seeded: false,
            verdicts_pending: None,
            idle_announced: BTreeSet::new(),
            attention_log: Vec::new(),
            last_notified_at: None,
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

    /// Forget every in-app suggestion stamp, the verdict news with its
    /// high-water mark, the idle-session batching set, and the attention log
    /// with the last-notified stamp: what `unenroll` calls so a next account
    /// inherits none of this one's stamps, news or notification budget. The marks go back to unseeded, so the next account's
    /// first poll seeds silently and the history cache this Mac keeps never
    /// replays as its news. Returns whether anything was there to forget, so
    /// the caller saves only when it must. Later nudge slices clear their
    /// own fields here too, so one call stays "every nudge field".
    pub fn clear_nudges(&mut self) -> bool {
        let had = !self.nudges.is_empty()
            || !self.verdict_marks.is_empty()
            || self.verdict_marks_seeded
            || self.verdicts_pending.is_some()
            || !self.idle_announced.is_empty()
            || !self.attention_log.is_empty()
            || self.last_notified_at.is_some();
        self.nudges.clear();
        self.verdict_marks.clear();
        self.verdict_marks_seeded = false;
        self.verdicts_pending = None;
        self.idle_announced.clear();
        self.attention_log.clear();
        self.last_notified_at = None;
        had
    }

    /// Record one announcement (folded into a digest or standalone), stamp
    /// `last_notified_at`, and prune the log. The caller saves.
    pub fn record_attention(
        &mut self,
        kind: super::attention::Kind,
        route: super::attention::Route,
        now: DateTime<Utc>,
    ) {
        self.attention_log.push(super::attention::AttentionEntry {
            kind,
            at: now,
            route,
        });
        self.last_notified_at = Some(self.last_notified_at.map_or(now, |was| was.max(now)));
        self.prune_attention_log(now);
    }

    /// Drop log entries older than [`ATTENTION_LOG_RETENTION`] behind `now`.
    /// An entry stamped after `now` is kept: a clock that went backwards may
    /// only suppress (see `attention`'s module doc), so it must still count.
    pub fn prune_attention_log(&mut self, now: DateTime<Utc>) {
        let floor = now - ATTENTION_LOG_RETENTION;
        self.attention_log.retain(|entry| entry.at >= floor);
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

    use crate::daemon::test_support::at;

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

    /// K11: a `cwd_cache` entry written before `tool` existed still loads,
    /// and reads as `None` rather than refusing to parse.
    ///
    /// Hand-written JSON rather than round-tripped through `save`, because a
    /// round trip through the current struct would always include the field
    /// and could never prove the old shape still decodes.
    #[test]
    fn a_cwd_cache_entry_written_before_tool_existed_still_loads() {
        let (_d, store) = temp_store();
        let body = serde_json::json!({
            "schema_version": DAEMON_STATE_SCHEMA,
            "cwd_cache": {
                "/tmp/old-session.jsonl": {
                    "size_bytes": 10,
                    "modified_at": "2026-08-08T10:00:00Z",
                    "cwd": "/Users/testuser/code/proj",
                    "project_key": "/Users/testuser/code/proj"
                }
            },
            "prior_uploads": {},
            "last_observation": {},
            "last_digest_at": null,
            "day_bucket": null,
            "uploads_today": 0,
            "bytes_today": 0
        });
        std::fs::write(
            store.daemon_path(DAEMON_STATE_FILE),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();

        let loaded = DaemonState::load(&store).unwrap();
        let entry = loaded.cwd_cache.get("/tmp/old-session.jsonl").unwrap();
        assert_eq!(
            entry.tool, None,
            "an entry with no `tool` key must not refuse to parse"
        );
        assert_eq!(
            entry.project_key.as_deref(),
            Some("/Users/testuser/code/proj"),
            "the field that did exist must still decode"
        );
    }

    /// A state file from a build that kept `verdicts_acked_through` still
    /// loads, and the next save drops the key: nothing ever read it.
    #[test]
    fn a_state_file_with_verdicts_acked_through_loads_and_drops_it() {
        let (_d, store) = temp_store();
        let mut body = serde_json::to_value(DaemonState::new()).unwrap();
        body["verdicts_acked_through"] = serde_json::json!("2026-10-01T00:00:00Z");
        std::fs::write(
            store.daemon_path(DAEMON_STATE_FILE),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        let mut loaded = DaemonState::load(&store).unwrap();
        loaded.save(&store).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.daemon_path(DAEMON_STATE_FILE)).unwrap())
                .unwrap();
        assert!(written.get("verdicts_acked_through").is_none(), "{written}");
    }

    /// Nudge S3: a state file written before the nudge ledger existed loads
    /// with an empty one, and an empty ledger adds nothing to the file, so
    /// an install that never saw a suggestion writes the bytes it always did.
    #[test]
    fn the_nudge_ledger_defaults_empty_and_writes_nothing_while_empty() {
        let (_d, store) = temp_store();
        let body = serde_json::json!({
            "schema_version": DAEMON_STATE_SCHEMA,
            "cwd_cache": {},
            "prior_uploads": {},
            "last_observation": {},
            "last_digest_at": null,
            "day_bucket": null,
            "uploads_today": 0,
            "bytes_today": 0
        });
        std::fs::write(
            store.daemon_path(DAEMON_STATE_FILE),
            serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        let loaded = DaemonState::load(&store).unwrap();
        assert!(loaded.nudges.is_empty());
        // Nudge S4: the verdict fields load empty and unseeded too.
        assert!(loaded.verdict_marks.is_empty());
        assert!(!loaded.verdict_marks_seeded);
        assert_eq!(loaded.verdicts_pending, None);
        // Nudge U4: the idle batching set loads empty too.
        assert!(loaded.idle_announced.is_empty());
        let written = serde_json::to_value(DaemonState::new()).unwrap();
        for key in [
            "nudges",
            "verdict_marks",
            "verdict_marks_seeded",
            "verdicts_pending",
            "idle_announced",
            "attention_log",
            "last_notified_at",
        ] {
            assert!(written.get(key).is_none(), "{key}: {written}");
        }
    }

    /// A stamped ledger survives a restart, and `clear_nudges` (what
    /// `unenroll` calls) empties it.
    #[test]
    fn the_nudge_ledger_round_trips_and_clears() {
        let (_d, store) = temp_store();
        let mut state = DaemonState::new();
        let at = DateTime::parse_from_rfc3339("2026-10-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        state.nudges.insert(
            "review_backlog".to_string(),
            super::super::nudge::NudgeLedger {
                declined_at: Some(at),
                ..Default::default()
            },
        );
        state.save(&store).unwrap();
        let mut loaded = DaemonState::load(&store).unwrap();
        assert_eq!(loaded, state);
        assert!(loaded.clear_nudges(), "a non-empty ledger reports a change");
        assert!(loaded.nudges.is_empty());
        assert!(!loaded.clear_nudges(), "an empty one reports none");
    }

    /// Nudge U4: the idle batching set holds opaque entry ids only,
    /// survives a restart, and `clear_nudges` (what `unenroll` calls)
    /// empties it, so a next account never inherits this one's batches.
    #[test]
    fn idle_announced_round_trips_and_clears_on_unenroll() {
        let (_d, store) = temp_store();
        let mut state = DaemonState::new();
        let id = uuid::Uuid::from_u128(7);
        state.idle_announced.insert(id);
        state.save(&store).unwrap();
        let body = std::fs::read_to_string(store.daemon_path(DAEMON_STATE_FILE)).unwrap();
        assert!(body.contains(&id.to_string()), "{body}");
        let mut loaded = DaemonState::load(&store).unwrap();
        assert_eq!(loaded.idle_announced, state.idle_announced);
        assert!(loaded.clear_nudges(), "a non-empty set reports a change");
        assert!(loaded.idle_announced.is_empty());
        assert!(!loaded.clear_nudges());
    }

    fn utc(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Nudge A2: the attention log (kind labels, routes and times only) and
    /// the last-notified stamp survive a restart, spelled as the arbiter
    /// spells its kinds, and `clear_nudges` (what `unenroll` calls) empties
    /// both, so a next account never inherits this one's budget.
    #[test]
    fn the_attention_log_round_trips_by_label_and_clears_on_unenroll() {
        use super::super::attention::{AttentionEntry, Kind, Route};
        let (_d, store) = temp_store();
        let mut state = DaemonState::new();
        let now = utc("2026-10-07T12:00:00Z");
        for (kind, route) in [
            (Kind::IdleSessions, Route::Folded),
            (Kind::VerdictsLanded, Route::Standalone),
        ] {
            state.record_attention(kind, route, now);
        }
        assert_eq!(state.last_notified_at, Some(now));
        state.save(&store).unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.daemon_path(DAEMON_STATE_FILE)).unwrap())
                .unwrap();
        assert_eq!(body["attention_log"][0]["kind"], "idle_sessions");
        assert_eq!(body["attention_log"][0]["route"], "folded");
        assert_eq!(body["attention_log"][1]["kind"], "verdicts_landed");
        assert_eq!(body["attention_log"][1]["route"], "standalone");
        let mut loaded = DaemonState::load(&store).unwrap();
        assert_eq!(loaded, state);
        assert_eq!(
            loaded.attention_log[1],
            AttentionEntry {
                kind: Kind::VerdictsLanded,
                at: now,
                route: Route::Standalone
            }
        );
        assert!(loaded.clear_nudges(), "a non-empty log reports a change");
        assert!(loaded.attention_log.is_empty());
        assert_eq!(loaded.last_notified_at, None);
        assert!(!loaded.clear_nudges());

        // The stamp alone is enough to report a change.
        loaded.last_notified_at = Some(now);
        assert!(loaded.clear_nudges());
    }

    /// Every arbiter kind serializes as its own label, so the file, the
    /// wire and the logs spell a kind one way.
    #[test]
    fn every_kind_serializes_as_its_label() {
        for kind in super::super::attention::Kind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.label());
        }
    }

    /// Pruned to `ATTENTION_LOG_RETENTION` behind `now`, and never ahead of
    /// it: an entry stamped after `now` (a clock that went backwards) is
    /// kept, because it may only suppress.
    #[test]
    fn the_attention_log_is_pruned_to_its_retention_and_keeps_future_entries() {
        use super::super::attention::{AttentionEntry, Kind, Route};
        let now = utc("2026-10-20T12:00:00Z");
        let mut state = DaemonState::new();
        let too_old = now - ATTENTION_LOG_RETENTION - chrono::Duration::seconds(1);
        let boundary = now - ATTENTION_LOG_RETENTION;
        let future = now + chrono::Duration::days(30);
        for when in [too_old, boundary, future] {
            state.attention_log.push(AttentionEntry {
                kind: Kind::WeeklyRecap,
                at: when,
                route: Route::Standalone,
            });
        }
        state.prune_attention_log(now);
        let kept: Vec<_> = state.attention_log.iter().map(|e| e.at).collect();
        assert_eq!(kept, vec![boundary, future]);
        assert_eq!(ATTENTION_LOG_RETENTION, chrono::Duration::days(8));

        // `record_attention` prunes as it pushes.
        state.record_attention(
            Kind::IdleSessions,
            Route::Folded,
            now + chrono::Duration::days(1),
        );
        assert!(!state.attention_log.iter().any(|e| e.at == boundary));
    }

    /// A kind this build does not know (a newer build wrote it, then the
    /// install was downgraded) is dropped from the log rather than refusing
    /// the whole state file, which would stop the daemon starting.
    #[test]
    fn an_unknown_kind_in_the_attention_log_is_dropped_not_fatal() {
        let (_d, store) = temp_store();
        let mut v = serde_json::to_value(DaemonState::new()).unwrap();
        v["attention_log"] = serde_json::json!([
            {"kind": "future_kind", "at": "2026-10-07T12:00:00Z", "route": "standalone"},
            {"kind": "idle_sessions", "at": "2026-10-07T12:00:00Z", "route": "folded"},
            {"kind": "idle_sessions", "at": "2026-10-07T12:00:00Z", "route": "beamed"}
        ]);
        store
            .write_daemon_file(DAEMON_STATE_FILE, v.to_string().as_bytes())
            .unwrap();
        let loaded = DaemonState::load(&store).expect("state still loads");
        assert_eq!(loaded.attention_log.len(), 1);
        assert_eq!(
            loaded.attention_log[0].kind,
            super::super::attention::Kind::IdleSessions
        );
    }
}
