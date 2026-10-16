//! The settlement estimate's scanner: every trace on this Mac that the
//! contributor has not ruled out, read for its `lef1` features through a
//! path-hashed cache, and nothing else.
//!
//! **It cannot queue.** [`scan`] takes `&Queue`, not `&mut Queue` and not
//! `DaemonShared`, so it has no way to enqueue, preview, redact or upload
//! anything (spec S4, S9). It reads the queue only to leave out traces that
//! were already sent, kept on this Mac, or declined.
//!
//! **Never is resolved before any open** (D3). A ref's project comes from
//! `SessionRef::cwd` or from the watcher's cwd cache when that entry still
//! matches the file's size and mtime -- never from `load`. A folder that
//! resolves to `Ignore`, the global Never override included, is skipped
//! before its transcript is opened. A ref with no cheap project answer is
//! counted as unknown and is never opened either.
//!
//! No logging here: the caller writes one counts-only line.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::local_credit_estimate::{
    LOCAL_ESTIMATE_FEATURES_VERSION, LocalEstimateFeatures,
};

use crate::config::{ConfigStore, DAEMON_SETTLEMENT_FEATURES_FILE};
use crate::source::{SessionRef, SourceRoots, TraceSource, all_sources};

use super::policy::{ProjectMode, ProjectPolicy, project_for};
use super::queue::{Queue, QueueState, estimate_features_of};
use super::state::{CwdCacheEntry, PriorUpload};

/// The cache key for one trace: `"sha256:" + hex(sha256(source || 0x00 ||
/// path bytes))`. The path never reaches the cache file; only this does.
pub fn path_hash(source: &str, path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hasher.update([0u8]);
    hasher.update(path.as_os_str().as_encoded_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// One cached trace: what it looked like when it was read, and its features.
/// `features: None` records a trace that could not be loaded, so a broken
/// file is not re-read every scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedFeatures {
    pub source: String,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
    pub features: Option<LocalEstimateFeatures>,
}

/// Features by [`path_hash`]. Holds no path and no transcript content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureCache {
    entries: BTreeMap<String, CachedFeatures>,
}

impl FeatureCache {
    /// The cache on disk. An unreadable or malformed file is an empty cache:
    /// the cost is one re-read of every trace, never a wrong figure.
    pub fn load(store: &ConfigStore) -> Self {
        store
            .read_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE)
            .ok()
            .flatten()
            .and_then(|body| serde_json::from_slice::<Self>(&body).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, store: &ConfigStore) -> anyhow::Result<()> {
        let body = serde_json::to_vec(self)?;
        store.write_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE, &body)
    }

    /// Remove the cache file. Missing is not an error.
    pub fn delete(store: &ConfigStore) -> anyhow::Result<()> {
        store.remove_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The cached features for `key`, when the trace has not changed since.
    /// Features of an older version are a miss: they would be read against
    /// a table that expects the current ones.
    fn hit(
        &self,
        key: &str,
        source: &str,
        size_bytes: u64,
        modified_at: DateTime<Utc>,
    ) -> Option<Option<LocalEstimateFeatures>> {
        let cached = self.entries.get(key)?;
        if cached.source != source
            || cached.size_bytes != size_bytes
            || cached.modified_at != modified_at
        {
            return None;
        }
        match &cached.features {
            Some(f) if f.version != LOCAL_ESTIMATE_FEATURES_VERSION => None,
            features => Some(features.clone()),
        }
    }
}

/// What a scan reads besides the sources. Snapshots, taken by the caller
/// under short locks; nothing here is mutable.
pub struct ScanInputs<'a> {
    pub policy: &'a ProjectPolicy,
    pub queue: &'a Queue,
    pub prior_uploads: &'a BTreeMap<String, PriorUpload>,
    pub cwd_cache: &'a BTreeMap<String, CwdCacheEntry>,
    pub quiescence_secs: u64,
    pub now: DateTime<Utc>,
}

/// What a scan found. `features` holds one item per counted trace: `Some`
/// for a trace whose features are known, `None` for one counted as unknown
/// (no cheap project answer, or a transcript that could not be read).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanOutcome {
    pub features: Vec<Option<LocalEstimateFeatures>>,
    pub skipped_sources: Vec<&'static str>,
    pub ignored: usize,
    pub excluded: usize,
    pub not_settled: usize,
    pub no_project: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub stopped: bool,
}

/// The sources an estimate reads (D4): what the contributor declared, bare,
/// plus every native store at its conventional location, except a source
/// the contributor declared `Off` -- that is an explicit "do not read", and
/// it wins over breadth in both sets. No trajectory auto-discovery: it
/// depends on the daemon's working directory.
pub fn estimation_sources(declared: &SourceRoots) -> Vec<Box<dyn TraceSource>> {
    let mut sources = all_sources(declared);
    sources.extend(
        all_sources(&SourceRoots::conventional())
            .into_iter()
            .filter(|source| !declared.is_off(source.name())),
    );
    sources
}

/// The paths a queue entry rules out (D5): already sent or in flight.
/// Withdrawn traces were uploaded, so they are here too.
fn sent_paths(queue: &Queue) -> HashSet<&Path> {
    queue
        .all()
        .iter()
        .filter(|e| {
            e.submission_id.is_some()
                || matches!(e.state, QueueState::Uploading | QueueState::Uploaded)
        })
        .map(|e| e.path.as_path())
        .collect()
}

/// The ref's project key, from what is already known about it and never
/// from its transcript (D3). `None` means there is no cheap answer.
fn cheap_project_key(
    session_ref: &SessionRef,
    modified_at: DateTime<Utc>,
    cwd_cache: &BTreeMap<String, CwdCacheEntry>,
) -> Option<String> {
    if let Some(cwd) = session_ref.cwd.as_deref() {
        return Some(project_for(Some(cwd)).0);
    }
    let hit = cwd_cache.get(&session_ref.path.to_string_lossy().to_string())?;
    if hit.size_bytes != session_ref.size_bytes || hit.modified_at != modified_at {
        return None;
    }
    Some(
        hit.project_key
            .clone()
            .unwrap_or_else(|| project_for(hit.cwd.as_deref()).0),
    )
}

/// When the ref last changed: the group's own time when the source knows
/// it, else the file's mtime. `None` means the file is gone.
fn modified_at_of(session_ref: &SessionRef) -> Option<DateTime<Utc>> {
    session_ref.group_modified_at.or_else(|| {
        std::fs::metadata(&session_ref.path)
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from)
    })
}

/// Scan every source for the estimate. See the module doc for what this may
/// and may not do. `should_stop` is asked after each trace; a `true` ends
/// the scan with `stopped` set and leaves the cache unpruned.
pub fn scan(
    sources: &[Box<dyn TraceSource>],
    inputs: &ScanInputs<'_>,
    cache: &mut FeatureCache,
    should_stop: &dyn Fn() -> bool,
) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    let sent = sent_paths(inputs.queue);
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut seen_keys: BTreeSet<String> = BTreeSet::new();
    // An out-of-range window is the longest one: nothing reads as settled,
    // so nothing is counted, rather than a panic in the daemon.
    let quiescence = i64::try_from(inputs.quiescence_secs)
        .ok()
        .and_then(chrono::Duration::try_seconds)
        .unwrap_or(chrono::Duration::MAX);

    'sources: for source in sources {
        let refs = match source.discover() {
            Ok(refs) => refs,
            Err(_) => {
                if !outcome.skipped_sources.contains(&source.name()) {
                    outcome.skipped_sources.push(source.name());
                }
                continue;
            }
        };
        for session_ref in refs {
            scan_one(
                source.as_ref(),
                &session_ref,
                inputs,
                &sent,
                quiescence,
                cache,
                &mut seen_paths,
                &mut seen_keys,
                &mut outcome,
            );
            if should_stop() {
                outcome.stopped = true;
                break 'sources;
            }
            std::thread::yield_now();
        }
    }

    if !outcome.stopped {
        cache.entries.retain(|key, _| seen_keys.contains(key));
    }
    outcome
}

/// One ref, in the order the plan fixes: dedupe, exclusions, project,
/// Never, settle, cache, load.
#[allow(clippy::too_many_arguments)]
fn scan_one(
    source: &dyn TraceSource,
    session_ref: &SessionRef,
    inputs: &ScanInputs<'_>,
    sent: &HashSet<&Path>,
    quiescence: chrono::Duration,
    cache: &mut FeatureCache,
    seen_paths: &mut HashSet<PathBuf>,
    seen_keys: &mut BTreeSet<String>,
    outcome: &mut ScanOutcome,
) {
    let path = session_ref.path.as_path();
    if !seen_paths.insert(path.to_path_buf()) {
        return;
    }
    if sent.contains(path)
        || inputs.queue.kept_at_path(path)
        || inputs.queue.dismissed_at_path(path)
        || inputs
            .prior_uploads
            .contains_key(&path.to_string_lossy().to_string())
    {
        outcome.excluded += 1;
        return;
    }
    // A file that cannot be stat'd is gone, as for the watcher.
    let Some(modified_at) = modified_at_of(session_ref) else {
        return;
    };
    let Some(project_key) = cheap_project_key(session_ref, modified_at, inputs.cwd_cache) else {
        outcome.no_project += 1;
        outcome.features.push(None);
        return;
    };
    if inputs.policy.resolve(&project_key) == ProjectMode::Ignore {
        outcome.ignored += 1;
        return;
    }
    if inputs.now - modified_at < quiescence {
        outcome.not_settled += 1;
        return;
    }
    let key = path_hash(source.name(), path);
    seen_keys.insert(key.clone());
    if let Some(features) = cache.hit(&key, source.name(), session_ref.size_bytes, modified_at) {
        outcome.cache_hits += 1;
        outcome.features.push(features);
        return;
    }
    let features = source
        .load(session_ref)
        .ok()
        .map(|transcript| estimate_features_of(&transcript));
    cache.entries.insert(
        key,
        CachedFeatures {
            source: source.name().to_string(),
            size_bytes: session_ref.size_bytes,
            modified_at,
            features: features.clone(),
        },
    );
    outcome.cache_misses += 1;
    outcome.features.push(features);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::policy::project_key_for;
    use crate::daemon::queue::{QueueEntry, REASON_DISMISSED, REASON_KEPT};
    use crate::source::{SessionEvent, SessionEventKind, SessionTranscript};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const QUIESCENCE: u64 = 300;
    const TRANSCRIPT_TEXT: &str = "a synthetic user message about widgets";

    fn now() -> DateTime<Utc> {
        crate::daemon::test_support::at("2026-01-10T12:00:00Z")
    }

    /// Settled well past the quiescence window.
    fn settled() -> DateTime<Utc> {
        now() - chrono::Duration::hours(2)
    }

    /// A source whose refs, failing paths and load count a test controls.
    struct DoubleSource {
        name: &'static str,
        refs: Arc<Mutex<Vec<SessionRef>>>,
        /// `load` panics for these: the test's claim is that they are never
        /// opened.
        never_open: BTreeSet<PathBuf>,
        fail_discover: bool,
        loads: Arc<AtomicUsize>,
    }

    impl DoubleSource {
        fn new(name: &'static str, refs: Vec<SessionRef>) -> Self {
            Self {
                name,
                refs: Arc::new(Mutex::new(refs)),
                never_open: BTreeSet::new(),
                fail_discover: false,
                loads: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn never_opening(mut self, path: &Path) -> Self {
            self.never_open.insert(path.to_path_buf());
            self
        }

        fn failing(mut self) -> Self {
            self.fail_discover = true;
            self
        }
    }

    impl TraceSource for DoubleSource {
        fn name(&self) -> &'static str {
            self.name
        }
        fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
            if self.fail_discover {
                anyhow::bail!("synthetic discover failure");
            }
            Ok(self.refs.lock().unwrap().clone())
        }
        fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
            if self.never_open.contains(&r.path) {
                panic!("opened an Ignore folder");
            }
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(SessionTranscript {
                cwd: r.cwd.clone(),
                events: vec![SessionEvent {
                    kind: SessionEventKind::User,
                    content: Some(TRANSCRIPT_TEXT.to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            })
        }
    }

    fn session(path: &Path, cwd: Option<&Path>, modified_at: DateTime<Utc>) -> SessionRef {
        SessionRef {
            source: "double",
            declared_source: None,
            path: path.to_path_buf(),
            project: None,
            cwd: cwd.map(|c| c.to_string_lossy().to_string()),
            started_at: None,
            size_bytes: 100,
            group_modified_at: Some(modified_at),
            group_member_count: 0,
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        /// A real directory, so `project_for` resolves it to a real key.
        cwd: PathBuf,
        policy: ProjectPolicy,
        queue: Queue,
        prior_uploads: BTreeMap<String, PriorUpload>,
        cwd_cache: BTreeMap<String, CwdCacheEntry>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let cwd = dir.path().join("project");
            std::fs::create_dir_all(&cwd).unwrap();
            Self {
                dir,
                cwd,
                policy: ProjectPolicy::new(),
                queue: Queue::new(),
                prior_uploads: BTreeMap::new(),
                cwd_cache: BTreeMap::new(),
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join("sessions").join(name)
        }

        fn project_key(&self) -> String {
            project_key_for(Some(&self.cwd.to_string_lossy()))
        }

        fn inputs(&self) -> ScanInputs<'_> {
            ScanInputs {
                policy: &self.policy,
                queue: &self.queue,
                prior_uploads: &self.prior_uploads,
                cwd_cache: &self.cwd_cache,
                quiescence_secs: QUIESCENCE,
                now: now(),
            }
        }

        fn store(&self) -> ConfigStore {
            ConfigStore::open(self.dir.path().join("state")).unwrap()
        }
    }

    fn boxed(source: DoubleSource) -> Vec<Box<dyn TraceSource>> {
        vec![Box::new(source)]
    }

    fn never_stop() -> bool {
        false
    }

    #[test]
    fn an_ignore_folder_is_never_opened() {
        let mut fx = Fixture::new();
        fx.policy
            .set_mode(&fx.project_key(), ProjectMode::Ignore, now())
            .unwrap();
        let path = fx.path("ignored.jsonl");
        let sources = boxed(
            DoubleSource::new("double", vec![session(&path, Some(&fx.cwd), settled())])
                .never_opening(&path),
        );
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.ignored, 1);
        assert!(outcome.features.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn the_global_never_override_is_never_opened() {
        let mut fx = Fixture::new();
        fx.policy
            .set_mode(&fx.project_key(), ProjectMode::NotifyOnly, now())
            .unwrap();
        fx.policy
            .set_contribution_override(ProjectMode::Ignore, now(), None)
            .unwrap();
        let path = fx.path("overridden.jsonl");
        let sources = boxed(
            DoubleSource::new("double", vec![session(&path, Some(&fx.cwd), settled())])
                .never_opening(&path),
        );
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.ignored, 1);
        assert!(outcome.features.is_empty());
    }

    #[test]
    fn a_ref_without_a_cheap_cwd_is_unknown_and_unopened() {
        let fx = Fixture::new();
        let path = fx.path("no-cwd.jsonl");
        let sources = boxed(
            DoubleSource::new("double", vec![session(&path, None, settled())]).never_opening(&path),
        );
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.features, vec![None]);
        assert_eq!(outcome.no_project, 1);
        assert_eq!(outcome.cache_misses, 0);
    }

    #[test]
    fn a_stale_cwd_cache_entry_is_no_project_answer() {
        // The cache entry is for an older size: it no longer describes the
        // file, so it cannot answer, and the ref is not opened to find out.
        let mut fx = Fixture::new();
        let path = fx.path("stale-cache.jsonl");
        fx.cwd_cache.insert(
            path.to_string_lossy().to_string(),
            CwdCacheEntry {
                size_bytes: 1,
                modified_at: settled(),
                cwd: Some(fx.cwd.to_string_lossy().to_string()),
                project_key: Some(fx.project_key()),
                tool: None,
                adapter: None,
            },
        );
        let sources = boxed(
            DoubleSource::new("double", vec![session(&path, None, settled())]).never_opening(&path),
        );
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.no_project, 1);
    }

    #[test]
    fn uploaded_kept_dismissed_and_prior_uploads_are_excluded() {
        let mut fx = Fixture::new();
        let uploaded = fx.path("uploaded.jsonl");
        let kept = fx.path("kept.jsonl");
        let dismissed = fx.path("dismissed.jsonl");
        let prior = fx.path("prior.jsonl");
        let waiting = fx.path("waiting.jsonl");
        fx.queue.push_for_test(QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            path: uploaded.clone(),
            state: QueueState::Uploaded,
            submission_id: Some(uuid::Uuid::new_v4()),
            ..Default::default()
        });
        fx.queue.push_for_test(QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            path: kept.clone(),
            state: QueueState::Refused,
            reason_label: Some(REASON_KEPT.to_string()),
            ..Default::default()
        });
        fx.queue.push_for_test(QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            path: dismissed.clone(),
            state: QueueState::Refused,
            reason_label: Some(REASON_DISMISSED.to_string()),
            ..Default::default()
        });
        fx.queue.push_for_test(QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            path: waiting.clone(),
            state: QueueState::Pending,
            ..Default::default()
        });
        fx.prior_uploads.insert(
            prior.to_string_lossy().to_string(),
            PriorUpload {
                hash: "sha256:00".to_string(),
                size_bytes: 100,
                upload_count: 1,
            },
        );
        let refs = [&uploaded, &kept, &dismissed, &prior, &waiting]
            .into_iter()
            .map(|p| session(p, Some(&fx.cwd), settled()))
            .collect();
        let sources = boxed(
            DoubleSource::new("double", refs)
                .never_opening(&uploaded)
                .never_opening(&kept)
                .never_opening(&dismissed)
                .never_opening(&prior),
        );
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.features.len(), 1);
        assert!(outcome.features[0].is_some());
        assert_eq!(outcome.excluded, 4);
    }

    #[test]
    fn a_trace_still_being_written_is_not_counted_yet() {
        let fx = Fixture::new();
        let path = fx.path("live.jsonl");
        let modified =
            now() - chrono::Duration::seconds(QUIESCENCE as i64) + chrono::Duration::seconds(1);
        let sources = boxed(
            DoubleSource::new("double", vec![session(&path, Some(&fx.cwd), modified)])
                .never_opening(&path),
        );
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.not_settled, 1);
        assert!(outcome.features.is_empty());
    }

    #[test]
    fn the_cache_hits_on_an_unchanged_key_and_misses_on_an_mtime_change() {
        let fx = Fixture::new();
        let path = fx.path("cached.jsonl");
        let source = DoubleSource::new("double", vec![session(&path, Some(&fx.cwd), settled())]);
        let refs = Arc::clone(&source.refs);
        let loads = Arc::clone(&source.loads);
        let sources: Vec<Box<dyn TraceSource>> = vec![Box::new(source)];
        let mut cache = FeatureCache::default();

        let first = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(first.cache_misses, 1);
        let second = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.cache_misses, 0);
        assert_eq!(second.features, first.features);
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        *refs.lock().unwrap() = vec![session(
            &path,
            Some(&fx.cwd),
            settled() + chrono::Duration::minutes(1),
        )];
        let third = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(third.cache_misses, 1);
        assert_eq!(third.cache_hits, 0);
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_cache_holds_no_path_or_content() {
        let fx = Fixture::new();
        let path = fx.path("private-name.jsonl");
        let sources = boxed(DoubleSource::new(
            "double",
            vec![session(&path, Some(&fx.cwd), settled())],
        ));
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert!(outcome.features[0].is_some());

        let store = fx.store();
        cache.save(&store).unwrap();
        let bytes = store
            .read_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE)
            .unwrap()
            .expect("cache file written");
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains(&*path.to_string_lossy()));
        assert!(!text.contains("private-name"));
        assert!(!text.contains(&*fx.cwd.to_string_lossy()));
        assert!(!text.contains(TRANSCRIPT_TEXT));
        assert!(!text.contains("widgets"));
        let loaded = FeatureCache::load(&store);
        assert_eq!(loaded, cache);
        assert!(!loaded.entries.is_empty());
        assert!(loaded.entries.keys().all(|k| k.starts_with("sha256:")));

        FeatureCache::delete(&store).unwrap();
        assert!(
            store
                .read_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_malformed_cache_file_loads_empty() {
        let fx = Fixture::new();
        let store = fx.store();
        store
            .write_daemon_file(DAEMON_SETTLEMENT_FEATURES_FILE, b"{not json")
            .unwrap();
        assert_eq!(FeatureCache::load(&store).len(), 0);
    }

    #[test]
    fn the_scanner_cannot_enqueue() {
        let mut fx = Fixture::new();
        for name in ["a.jsonl", "b.jsonl"] {
            fx.queue.push_for_test(QueueEntry {
                entry_id: uuid::Uuid::new_v4(),
                path: fx.path(name),
                state: QueueState::Pending,
                ..Default::default()
            });
        }
        let before = fx.queue.clone();
        let n = before.all().len();
        let refs = ["fresh-1.jsonl", "fresh-2.jsonl", "fresh-3.jsonl"]
            .into_iter()
            .map(|name| session(&fx.path(name), Some(&fx.cwd), settled()))
            .collect();
        let sources = boxed(DoubleSource::new("double", refs));
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.features.len(), 3);
        assert_eq!(fx.queue.all().len(), n);
        assert_eq!(fx.queue, before);
    }

    #[test]
    fn a_failing_source_is_skipped_by_name() {
        let fx = Fixture::new();
        let path = fx.path("other.jsonl");
        let sources: Vec<Box<dyn TraceSource>> = vec![
            Box::new(DoubleSource::new("double", vec![]).failing()),
            Box::new(DoubleSource::new(
                "other",
                vec![session(&path, Some(&fx.cwd), settled())],
            )),
        ];
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.skipped_sources, vec!["double"]);
        assert_eq!(outcome.features.len(), 1);
        assert!(outcome.features[0].is_some());
    }

    #[test]
    fn should_stop_ends_the_scan_between_traces() {
        let fx = Fixture::new();
        let refs = ["one.jsonl", "two.jsonl", "three.jsonl"]
            .into_iter()
            .map(|name| session(&fx.path(name), Some(&fx.cwd), settled()))
            .collect();
        let sources = boxed(DoubleSource::new("double", refs));
        let mut cache = FeatureCache::default();
        let asked = Cell::new(0usize);
        let stop_after_first = || {
            asked.set(asked.get() + 1);
            true
        };
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &stop_after_first);
        assert!(outcome.stopped);
        assert_eq!(outcome.features.len(), 1);
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn a_duplicate_path_is_counted_once() {
        let fx = Fixture::new();
        let path = fx.path("dup.jsonl");
        let sources = boxed(DoubleSource::new(
            "double",
            vec![
                session(&path, Some(&fx.cwd), settled()),
                session(&path, Some(&fx.cwd), settled()),
            ],
        ));
        let mut cache = FeatureCache::default();
        let outcome = scan(&sources, &fx.inputs(), &mut cache, &never_stop);
        assert_eq!(outcome.features.len(), 1);
    }

    #[test]
    fn path_hash_binds_source_and_path() {
        let p = Path::new("/tmp/synthetic/session.jsonl");
        let h = path_hash("codex", p);
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
        assert_ne!(h, path_hash("claude-code", p));
        assert_ne!(
            h,
            path_hash("codex", Path::new("/tmp/synthetic/other.jsonl"))
        );
        assert_eq!(h, path_hash("codex", p));
    }
}
