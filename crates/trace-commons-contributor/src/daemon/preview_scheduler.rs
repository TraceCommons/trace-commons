//! Bounded scheduling for preview work.
//!
//! A preview reads a whole session file, parses it, runs the redaction
//! pipeline over it, and serializes an envelope. That is the most expensive
//! thing this daemon does per unit of contributor attention, and until this
//! module existed nothing bounded how many of them could be in flight at
//! once. A shell that asked for one preview per queued entry -- which is
//! what a list view naturally does -- got exactly that many concurrent
//! full-pipeline passes. On a machine with 4,097 queued sessions totalling
//! 11.7 GB that consumed 1.7 cores sustained and 1.34 GB resident inside
//! three minutes, and the load average reached 649 before the application
//! was force-quit.
//!
//! The bound belongs here rather than in the three native shells. All three
//! request previews, only the daemon can see the total, and a per-shell
//! bound is the same rule written three times in three languages with
//! nothing keeping the three honest. (The GTK shell already has a crude
//! one -- a single worker thread in `worker.rs` -- and the macOS shell has
//! none at all, which is precisely the asymmetry a daemon-side bound makes
//! impossible.)
//!
//! What this module is and is not:
//!
//! - It is a **queue, a worker pool, a dedup table, and a result cache**.
//! - It is **not** the preview pipeline. It calls a [`PreviewJobRunner`]
//!   and holds whatever that returns as an opaque JSON value. The pipeline
//!   lives in [`super::preview`], and nothing here reaches into it.
//!
//! See `docs/superpowers/specs/2026-08-20-preview-scheduler-design.md` for
//! the reasoning behind the concurrency number, the admission cap, and the
//! decision to keep this queue preview-specific rather than making it the
//! daemon's general whole-session work queue.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::Notify;
use uuid::Uuid;

/// How many previews may be building at once, daemon-wide.
///
/// Two, and the binding constraint is memory rather than CPU. A preview's
/// peak resident set is a multiple of the session bytes it loads -- the
/// file, its parsed form, the redacted form, and the serialized envelope
/// all exist at once -- so the scheduler's worst case is roughly
/// `PREVIEW_WORKERS * MAX_PREVIEW_SESSION_BYTES * k`. With the admission
/// cap below at 64 MiB, two workers keep that in the hundreds of megabytes
/// even for the largest sessions this daemon will look at. Four would put
/// the same machine back inside the range the incident reached.
///
/// The CPU argument points the same way and is the reason the number is not
/// derived from `available_parallelism`. This is a background daemon whose
/// visible job is to stay invisible; the cores it does not take are the
/// ones the contributor's editor and the shell rendering these cards get to
/// use. Two saturates a slow disk's read queue while leaving every machine
/// this ships to -- including two-core laptops -- something to run the UI
/// on.
///
/// One would also fix the incident, and was rejected for a reason worth
/// stating: a single worker means one 64 MiB session stalls every card on
/// screen behind it, and the visible-first ordering below cannot help,
/// because ordering only decides what starts next, never what is already
/// running. Two means a slow job costs at most half the throughput.
pub const PREVIEW_WORKERS: usize = 2;

/// The largest session, in raw bytes, this scheduler will run the preview
/// pipeline over.
///
/// 64 MiB. Grounded in the corpus that produced the incident: 1,028
/// Claude Code sessions totalling 0.9 GB with a 29.8 MB maximum, and 3,069
/// Codex rollouts totalling 10.8 GB with a 367.5 MB maximum and a 3.5 MB
/// mean. Every Claude session in that sample is admitted with room to
/// spare, and so is everything within an order of magnitude of the Codex
/// mean; what the cap excludes is the Codex tail, where a single rollout is
/// a hundred times the mean and no amount of scheduling makes parsing it to
/// draw a card a reasonable thing to do.
///
/// The exact excluded fraction of that corpus is **not** computed here: the
/// measurements available are totals, means, and maxima, and a fraction
/// cannot be recovered from those. Treating the cap as "admits almost
/// everything" is the claim being made, and it rests on the mean being two
/// orders of magnitude below the cap, not on a percentile anybody measured.
///
/// A session over the cap is **refused, visibly**, as
/// [`PreviewOutcome::TooLarge`] -- never estimated. The preview card states
/// a would-send byte count on a consent surface, and a number derived from
/// anything other than the envelope that would actually be sent is a false
/// number in the one place this product cannot put one. The shell says the
/// session is too large to preview and reports the raw size, which is a
/// stat, not an estimate.
pub const MAX_PREVIEW_SESSION_BYTES: u64 = 64 * 1024 * 1024;

/// How many completed previews the result cache holds before it is cleared
/// wholesale.
///
/// Mirrors `source::codex::CWD_MEMO_CAP` in both value and reasoning: a
/// bound a real corpus does not reach, so an always-on daemon does not grow
/// without limit, cleared wholesale rather than evicted one at a time
/// because the cost of being wrong is one rebuild pass rather than a wrong
/// answer.
pub const PREVIEW_CACHE_CAP: usize = 4096;

/// What identifies the artifact a cached preview describes.
///
/// `(path, file size, file mtime, group size, config fingerprint)`. The
/// first three are the memoization key `source::codex::peek_cwd_memoized`
/// already uses, for the reason given there: a file whose size and mtime
/// are unchanged has unchanged contents, and this is not a trust boundary
/// -- anyone able to backdate an mtime can equally well write whatever they
/// like into the file.
///
/// The last two are why this key is not simply `(path, size, mtime)`:
///
/// - `group_bytes` is the queue entry's own `size_bytes`, which for a
///   Claude Code session covers the primary file **plus every delegated
///   subagent transcript** the ref loads. A subagent file growing does not
///   change the primary file's stat, so without this the cache would keep
///   answering with a summary of fewer bytes than an upload would send.
///   The watcher refreshes that number on its poll, so the invalidation is
///   as prompt as discovery is.
/// - `config_fingerprint` is `preview::input_fingerprint`, so changing the
///   consent scopes, the identity, or the privacy-filter settings
///   invalidates every cached preview rather than leaving the contributor
///   looking at a card built under the old configuration.
///
/// The residual, stated rather than buried: a subagent transcript written
/// between one watcher poll and the next is not reflected until that poll
/// updates the entry. Entries are only offered once the eligibility check
/// judges the whole group quiescent, so this window is one where the entry
/// is being superseded anyway -- but it is a window, and a cached summary
/// inside it describes bytes that have since grown.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewKey {
    pub path: PathBuf,
    pub file_bytes: u64,
    /// `(seconds, nanoseconds)` since the unix epoch. `None` when the file
    /// could not be stat-ed or reports no mtime -- which is itself part of
    /// the key, so a file that gains an mtime invalidates.
    pub file_modified: Option<(i64, u32)>,
    pub group_bytes: u64,
    pub config_fingerprint: String,
}

impl PreviewKey {
    /// Build a key by stat-ing `path`.
    ///
    /// One `stat` per request. That is deliberate and it is the cheapest
    /// honest option: the alternative -- keying on the queue entry alone --
    /// cannot see a file rewritten in place at the same recorded size, and
    /// the alternative in the other direction -- re-resolving the session
    /// ref through discovery -- is a full session-root scan to answer a
    /// question about one file.
    ///
    /// A `stat` failure is not an error here. The key simply records the
    /// absence, and the job runs; the runner is the thing that discovers
    /// the session has vanished and says so.
    pub fn for_entry(path: &Path, group_bytes: u64, config_fingerprint: String) -> Self {
        let (file_bytes, file_modified) = match std::fs::metadata(path) {
            Ok(md) => {
                let modified = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| (d.as_secs() as i64, d.subsec_nanos()));
                (md.len(), modified)
            }
            Err(_) => (0, None),
        };
        Self {
            path: path.to_path_buf(),
            file_bytes,
            file_modified,
            group_bytes,
            config_fingerprint,
        }
    }
}

/// What a finished preview job yields.
///
/// `Ready` carries the summary as an opaque JSON object rather than a
/// `PreviewSummary`. That is the seam that keeps this module out of the
/// preview pipeline's way: the scheduler stores and replays exactly the
/// bytes the IPC layer would have sent, and knows nothing about their
/// shape.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewOutcome {
    Ready(Arc<serde_json::Value>),
    /// Refused by admission control before any parsing happened. Carries
    /// the raw size and the cap so the shell can say exactly why, and
    /// carries **no** would-send estimate -- see
    /// [`MAX_PREVIEW_SESSION_BYTES`].
    TooLarge {
        raw_session_bytes: u64,
        limit_bytes: u64,
    },
    /// The pipeline refused. `code` and `label` are the same fixed IPC
    /// error code and label the blocking `preview` method would have
    /// returned; neither carries a path or any session content.
    Failed {
        code: &'static str,
        label: &'static str,
    },
}

impl PreviewOutcome {
    /// The wire object for this outcome, as both the `preview_request`
    /// result and the `preview_ready` event carry it.
    pub fn to_value(&self, entry_id: Uuid) -> serde_json::Value {
        match self {
            PreviewOutcome::Ready(summary) => json!({
                "entry_id": entry_id.to_string(),
                "state": STATE_READY,
                "summary": summary.as_ref(),
            }),
            PreviewOutcome::TooLarge {
                raw_session_bytes,
                limit_bytes,
            } => json!({
                "entry_id": entry_id.to_string(),
                "state": STATE_TOO_LARGE,
                "raw_session_bytes": raw_session_bytes,
                "limit_bytes": limit_bytes,
            }),
            PreviewOutcome::Failed { code, label } => json!({
                "entry_id": entry_id.to_string(),
                "state": STATE_FAILED,
                "code": code,
                "label": label,
            }),
        }
    }
}

/// `state` values on `preview_request` and `preview_ready`.
pub const STATE_READY: &str = "ready";
pub const STATE_QUEUED: &str = "queued";
pub const STATE_RUNNING: &str = "running";
pub const STATE_TOO_LARGE: &str = "too_large";
pub const STATE_FAILED: &str = "failed";

/// What `request` tells the caller happened, so a shell can render a card
/// immediately rather than blocking on a parse.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestState {
    /// Answered from cache. No work was done and no event will follow.
    Cached(PreviewOutcome),
    /// Accepted and queued. A `preview_ready` event will follow.
    Queued,
    /// Already building, from an earlier request. A `preview_ready` event
    /// will follow; this request added no work.
    Running,
}

/// One unit of preview work.
#[derive(Debug, Clone)]
pub struct PreviewJob {
    pub entry_id: Uuid,
    pub key: PreviewKey,
    /// The raw bytes admission control judged. The queue entry's
    /// `size_bytes`, which covers the whole group.
    pub raw_session_bytes: u64,
}

/// How the worker pool does the work and delivers the result.
///
/// A trait, not a closure, for two reasons. The daemon's implementation
/// needs an `Arc<DaemonShared>` -- which owns the scheduler, so the
/// scheduler cannot own the runner without a reference cycle -- and the
/// tests need a runner that counts its own concurrency, which is the only
/// way to assert the bound holds rather than assert it was configured.
pub trait PreviewJobRunner: Send + Sync + 'static {
    /// Build the preview. Called only for jobs that passed admission
    /// control, and only from inside a worker.
    fn run(&self, job: PreviewJob) -> Pin<Box<dyn Future<Output = PreviewOutcome> + Send>>;

    /// Publish a finished outcome to whoever is listening. Called exactly
    /// once per delivered job, and **not at all** for a job that was
    /// cancelled while it ran.
    fn deliver(&self, entry_id: Uuid, outcome: &PreviewOutcome);
}

struct Inner {
    /// Arrival-ordered queue. Membership, not order, is what `queued`
    /// tracks; the order here is the FIFO tie-break within a priority
    /// class.
    pending: VecDeque<PreviewJob>,
    queued: HashSet<Uuid>,
    /// Entries a worker is currently building, and the key each is being
    /// built against.
    running: HashMap<Uuid, PreviewKey>,
    /// Running entries whose result must be thrown away on completion.
    cancelled: HashSet<Uuid>,
    /// The entries a shell says are on screen. Replaced wholesale by
    /// `set_visible`.
    visible: HashSet<Uuid>,
    cache: HashMap<PreviewKey, PreviewOutcome>,
}

/// The daemon's bound on concurrent preview work.
pub struct PreviewScheduler {
    inner: Mutex<Inner>,
    wake: Notify,
    stopping: AtomicBool,
    workers: usize,
    admission_cap: u64,
}

impl Default for PreviewScheduler {
    fn default() -> Self {
        Self::new(PREVIEW_WORKERS, MAX_PREVIEW_SESSION_BYTES)
    }
}

impl PreviewScheduler {
    pub fn new(workers: usize, admission_cap: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                pending: VecDeque::new(),
                queued: HashSet::new(),
                running: HashMap::new(),
                cancelled: HashSet::new(),
                visible: HashSet::new(),
                cache: HashMap::new(),
            }),
            wake: Notify::new(),
            stopping: AtomicBool::new(false),
            workers,
            admission_cap,
        }
    }

    pub fn workers(&self) -> usize {
        self.workers
    }

    /// Ask for a preview of `entry_id`.
    ///
    /// Idempotent and cheap. A shell may call this for every card it draws,
    /// on every redraw, without adding work: a cached result comes straight
    /// back, and an entry already queued or running is not enqueued twice.
    ///
    /// The one case that does enqueue a second time is a key change while
    /// the entry is running -- the session grew underneath the build. The
    /// in-flight job is then cancelled (its result would describe bytes
    /// that no longer exist) and a fresh one queued.
    pub fn request(&self, entry_id: Uuid, key: PreviewKey, raw_session_bytes: u64) -> RequestState {
        let mut inner = self.lock();
        if let Some(hit) = inner.cache.get(&key) {
            return RequestState::Cached(hit.clone());
        }
        if let Some(running_key) = inner.running.get(&entry_id) {
            if running_key == &key {
                // A repeat request revives an entry someone cancelled and
                // then asked for again -- scrolled away and scrolled back.
                inner.cancelled.remove(&entry_id);
                return RequestState::Running;
            }
            // Building against bytes that have changed. Throw that away.
            inner.cancelled.insert(entry_id);
        }
        if inner.queued.contains(&entry_id) {
            // Already waiting. Re-key it in place rather than enqueueing a
            // duplicate, so the job that eventually runs describes the file
            // as it is now.
            for job in inner.pending.iter_mut() {
                if job.entry_id == entry_id {
                    job.key = key;
                    job.raw_session_bytes = raw_session_bytes;
                    break;
                }
            }
            return RequestState::Queued;
        }
        inner.queued.insert(entry_id);
        inner.pending.push_back(PreviewJob {
            entry_id,
            key,
            raw_session_bytes,
        });
        drop(inner);
        self.wake.notify_one();
        RequestState::Queued
    }

    /// Declare which entries are on screen.
    ///
    /// Wholesale replacement, deliberately: a shell knows its own visible
    /// set after a scroll and does not know which entries left it, so an
    /// add/remove interface would make the shell diff something the daemon
    /// can diff for free. Calling this on every scroll tick is intended --
    /// it takes one lock and touches no jobs.
    ///
    /// Visibility decides **order only**, never membership. An entry that
    /// scrolls off the screen keeps its place in the queue; a shell that
    /// wants it dropped calls [`cancel`](Self::cancel).
    pub fn set_visible(&self, entry_ids: impl IntoIterator<Item = Uuid>) -> usize {
        let mut inner = self.lock();
        inner.visible = entry_ids.into_iter().collect();
        inner.visible.len()
    }

    /// Drop `entry_id` from the queue, or mark a running build's result for
    /// discard.
    ///
    /// Returns whether anything was actually dropped or marked.
    ///
    /// **What this cannot do:** it cannot stop a build that has already
    /// started. `preview::build_preview` reads, parses, redacts, and
    /// serializes with no cancellation point in it, and the expensive
    /// stretch -- the serde parse the incident's samples were all sitting
    /// in -- is a single call this scheduler does not get control back
    /// from. Cancelling a running entry therefore costs exactly the CPU
    /// that job had left to spend; what it buys is that no result is
    /// delivered and nothing is cached. The bound on how much CPU that can
    /// ever be is [`PREVIEW_WORKERS`] jobs' worth, which is why the bound
    /// is the load-bearing part of this design and cancellation is not.
    ///
    /// A cancelled running job's result is discarded rather than cached.
    /// The saving would be real, but an entry is cancelled because it was
    /// dismissed or scrolled away, and holding a redacted summary for a
    /// trace the contributor just declined is retention with no consumer.
    pub fn cancel(&self, entry_id: Uuid) -> bool {
        let mut inner = self.lock();
        let was_queued = inner.queued.remove(&entry_id);
        if was_queued {
            inner.pending.retain(|job| job.entry_id != entry_id);
        }
        let was_running = inner.running.contains_key(&entry_id);
        if was_running {
            inner.cancelled.insert(entry_id);
        }
        was_queued || was_running
    }

    /// Claim the next job, visible entries first and arrival order within
    /// each class.
    ///
    /// `pub` because it is the one piece of queue discipline worth testing
    /// directly: whether a visible entry requested after four hundred
    /// others is served before them is a property of this function, and
    /// asserting it here is stronger than inferring it from a race.
    pub fn take_next(&self) -> Option<PreviewJob> {
        let mut inner = self.lock();
        let position = inner
            .pending
            .iter()
            .position(|job| inner.visible.contains(&job.entry_id))
            .or(if inner.pending.is_empty() {
                None
            } else {
                Some(0)
            })?;
        let job = inner.pending.remove(position)?;
        inner.queued.remove(&job.entry_id);
        inner.running.insert(job.entry_id, job.key.clone());
        Some(job)
    }

    /// Record a finished job. Returns whether the outcome should be
    /// delivered -- `false` when the entry was cancelled while it ran.
    pub fn finish(&self, job: &PreviewJob, outcome: PreviewOutcome) -> bool {
        let mut inner = self.lock();
        inner.running.remove(&job.entry_id);
        if inner.cancelled.remove(&job.entry_id) {
            return false;
        }
        if inner.cache.len() >= PREVIEW_CACHE_CAP && !inner.cache.contains_key(&job.key) {
            inner.cache.clear();
        }
        inner.cache.insert(job.key.clone(), outcome);
        true
    }

    /// Whether admission control will run this job at all.
    pub fn admits(&self, raw_session_bytes: u64) -> bool {
        raw_session_bytes <= self.admission_cap
    }

    pub fn admission_cap(&self) -> u64 {
        self.admission_cap
    }

    /// Stop every worker after it finishes the job it holds.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// How many jobs are queued. Test and diagnostic use.
    pub fn queued_len(&self) -> usize {
        self.lock().pending.len()
    }

    /// How many jobs are building. Test and diagnostic use.
    pub fn running_len(&self) -> usize {
        self.lock().running.len()
    }

    /// How many results are cached. Test and diagnostic use.
    pub fn cache_len(&self) -> usize {
        self.lock().cache.len()
    }

    /// A cached outcome, without enqueueing anything.
    pub fn cached(&self, key: &PreviewKey) -> Option<PreviewOutcome> {
        self.lock().cache.get(key).cloned()
    }

    /// A poisoned scheduler lock is not recoverable state -- every field
    /// behind it is a cache or a queue, and a panicking worker leaves them
    /// consistent -- so recover rather than propagate.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One worker: claim a job, run it, deliver it, repeat.
///
/// Admission control happens here rather than in `request` so that a
/// too-large session is reported through the same path as every other
/// outcome -- one `preview_ready` event, one cache entry -- instead of
/// being a special case the shells have to handle twice.
async fn worker_loop(sched: Arc<PreviewScheduler>, runner: Arc<dyn PreviewJobRunner>) {
    loop {
        if sched.is_stopping() {
            return;
        }
        // Register interest *before* the queue check. `Notify` stores a
        // permit for `notify_one`, so a job pushed between the check and
        // the await is not lost; taking the future first also covers
        // `notify_waiters` from `stop`.
        let waiting = sched.wake.notified();
        let Some(job) = sched.take_next() else {
            waiting.await;
            continue;
        };
        let outcome = if sched.admits(job.raw_session_bytes) {
            runner.run(job.clone()).await
        } else {
            PreviewOutcome::TooLarge {
                raw_session_bytes: job.raw_session_bytes,
                limit_bytes: sched.admission_cap(),
            }
        };
        if sched.finish(&job, outcome.clone()) {
            runner.deliver(job.entry_id, &outcome);
        }
    }
}

/// Start the worker pool. Returns the join handles so a caller that owns
/// the runtime can stop and await them.
pub fn spawn_workers(
    sched: Arc<PreviewScheduler>,
    runner: Arc<dyn PreviewJobRunner>,
) -> Vec<tokio::task::JoinHandle<()>> {
    (0..sched.workers())
        .map(|_| {
            let sched = Arc::clone(&sched);
            let runner = Arc::clone(&runner);
            tokio::spawn(worker_loop(sched, runner))
        })
        .collect()
}

/// The daemon's real runner: drives [`super::preview::build_preview`]
/// through `ipc::build_and_pin_preview` and publishes the result as a
/// `preview_ready` event.
///
/// It holds an `Arc<DaemonShared>`, and `DaemonShared` owns the scheduler.
/// That is why the scheduler does not own its runner: it would close the
/// loop and neither would ever be dropped. The pool is stopped by aborting
/// the worker tasks, which drops this and releases the daemon.
pub struct DaemonPreviewRunner {
    shared: Arc<super::ipc::DaemonShared>,
}

impl DaemonPreviewRunner {
    pub fn new(shared: Arc<super::ipc::DaemonShared>) -> Self {
        Self { shared }
    }
}

impl PreviewJobRunner for DaemonPreviewRunner {
    fn run(&self, job: PreviewJob) -> Pin<Box<dyn Future<Output = PreviewOutcome> + Send>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            // Re-read the entry inside the worker rather than carrying a
            // copy on the job. A queued preview can wait behind hundreds of
            // others, and the entry it describes may have been approved,
            // dismissed, or superseded in the meantime; the entry that
            // exists now is the only one worth building.
            let entry = {
                let queue = shared.queue.lock().expect("queue lock");
                queue.get(job.entry_id).cloned()
            };
            let Some(entry) = entry else {
                return PreviewOutcome::Failed {
                    code: super::ipc::ERR_BAD_PARAMS,
                    label: super::ipc::ERR_UNKNOWN_ENTRY_ID,
                };
            };
            let cfg = shared.store.load_config().ok().flatten();
            match super::ipc::build_and_pin_preview(&shared, job.entry_id, &entry, cfg.as_ref())
                .await
            {
                Ok((summary, _body, _envelope)) => {
                    PreviewOutcome::Ready(Arc::new(super::ipc::preview_summary_value(&summary)))
                }
                Err((code, label)) => PreviewOutcome::Failed { code, label },
            }
        })
    }

    fn deliver(&self, entry_id: Uuid, outcome: &PreviewOutcome) {
        self.shared
            .publish(super::ipc::EVENT_PREVIEW_READY, outcome.to_value(entry_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::mpsc;

    fn key(n: u64) -> PreviewKey {
        PreviewKey {
            path: PathBuf::from(format!("/sessions/{n}.jsonl")),
            file_bytes: n,
            file_modified: Some((1_700_000_000, 0)),
            group_bytes: n,
            config_fingerprint: "fp".to_string(),
        }
    }

    /// A runner that observes its own concurrency, counts its runs, and can
    /// be held open until the test releases it.
    ///
    /// The counters are `Arc`s rather than plain fields so the future
    /// `run` returns can decrement `in_flight` itself. Decrementing in
    /// `deliver` would be wrong in exactly the case that matters most: a
    /// cancelled job never reaches `deliver`, and the count would leak.
    struct FakeRunner {
        in_flight: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        runs: Arc<AtomicUsize>,
        gate: Arc<tokio::sync::Semaphore>,
        delivered: mpsc::UnboundedSender<(Uuid, PreviewOutcome)>,
        /// Signalled the instant a worker enters `run`. Tests await this
        /// rather than yielding a fixed number of times: "has a worker
        /// picked the job up yet" is a fact the runner knows and a
        /// scheduler-poll count only guesses at, and guessing is how a
        /// concurrency test becomes a flaky one.
        started: mpsc::UnboundedSender<Uuid>,
    }

    struct FakeHandles {
        runner: Arc<FakeRunner>,
        delivered: mpsc::UnboundedReceiver<(Uuid, PreviewOutcome)>,
        started: mpsc::UnboundedReceiver<Uuid>,
        gate: Arc<tokio::sync::Semaphore>,
    }

    impl FakeRunner {
        /// Named `fixture` rather than `new`: it hands back the runner
        /// plus the channels and gate a test drives it with, not a bare
        /// `Self`.
        fn fixture(permits: usize) -> FakeHandles {
            let gate = Arc::new(tokio::sync::Semaphore::new(permits));
            let (tx, delivered) = mpsc::unbounded_channel();
            let (started_tx, started) = mpsc::unbounded_channel();
            FakeHandles {
                runner: Arc::new(Self {
                    in_flight: Arc::new(AtomicUsize::new(0)),
                    peak: Arc::new(AtomicUsize::new(0)),
                    runs: Arc::new(AtomicUsize::new(0)),
                    gate: Arc::clone(&gate),
                    delivered: tx,
                    started: started_tx,
                }),
                delivered,
                started,
                gate,
            }
        }
    }

    impl PreviewJobRunner for FakeRunner {
        fn run(&self, job: PreviewJob) -> Pin<Box<dyn Future<Output = PreviewOutcome> + Send>> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            self.runs.fetch_add(1, Ordering::SeqCst);
            let _ = self.started.send(job.entry_id);
            let gate = Arc::clone(&self.gate);
            let in_flight = Arc::clone(&self.in_flight);
            let id = job.entry_id;
            Box::pin(async move {
                // Parked here until the test opens the gate, which is what
                // makes `in_flight` a direct measurement of how many jobs
                // the pool allowed to start rather than a sampling of a
                // race.
                gate.acquire().await.expect("gate open").forget();
                in_flight.fetch_sub(1, Ordering::SeqCst);
                PreviewOutcome::Ready(Arc::new(json!({ "entry_id": id.to_string() })))
            })
        }

        fn deliver(&self, entry_id: Uuid, outcome: &PreviewOutcome) {
            let _ = self.delivered.send((entry_id, outcome.clone()));
        }
    }

    /// Await `n` job starts, with a deadline so a scheduling bug fails the
    /// test rather than hanging the suite.
    async fn await_starts(rx: &mut mpsc::UnboundedReceiver<Uuid>, n: usize) -> Vec<Uuid> {
        let mut seen = Vec::with_capacity(n);
        for _ in 0..n {
            let id = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                .await
                .expect("a worker must pick the job up")
                .expect("the runner is alive");
            seen.push(id);
        }
        seen
    }

    /// Give any worker that was going to start another job the chance to,
    /// so "nothing else started" is a claim about a settled pool.
    async fn quiesce() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrency_never_exceeds_the_bound_under_a_burst_of_hundreds() {
        let sched = Arc::new(PreviewScheduler::new(
            PREVIEW_WORKERS,
            MAX_PREVIEW_SESSION_BYTES,
        ));
        // Closed gate: every job that starts parks inside `run`, so
        // `in_flight` measures exactly how many jobs the pool let start.
        let FakeHandles {
            runner,
            delivered: mut rx,
            mut started,
            gate,
        } = FakeRunner::fixture(0);
        let handles = spawn_workers(Arc::clone(&sched), runner.clone());

        let ids: Vec<Uuid> = (0..400).map(|_| Uuid::new_v4()).collect();
        for (n, id) in ids.iter().enumerate() {
            sched.request(*id, key(n as u64), 1024);
        }
        await_starts(&mut started, PREVIEW_WORKERS).await;
        quiesce().await;
        assert!(
            started.try_recv().is_err(),
            "no third job may start while the first {PREVIEW_WORKERS} are still running"
        );
        assert_eq!(
            runner.in_flight.load(Ordering::SeqCst),
            PREVIEW_WORKERS,
            "400 requests must put exactly {PREVIEW_WORKERS} previews in flight"
        );

        // Let them all through and watch the peak across the whole drain.
        gate.add_permits(400);
        for _ in 0..400 {
            rx.recv().await.expect("every job delivers");
        }
        assert_eq!(runner.runs.load(Ordering::SeqCst), 400);
        assert_eq!(
            runner.peak.load(Ordering::SeqCst),
            PREVIEW_WORKERS,
            "peak concurrency across the drain must equal the bound, never exceed it"
        );
        assert_eq!(sched.queued_len(), 0);
        assert_eq!(sched.running_len(), 0);

        sched.stop();
        for h in handles {
            let _ = h.await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn fifty_requests_for_one_entry_do_the_work_once() {
        let sched = Arc::new(PreviewScheduler::new(2, MAX_PREVIEW_SESSION_BYTES));
        let FakeHandles {
            runner,
            delivered: mut rx,
            mut started,
            gate,
        } = FakeRunner::fixture(0);
        let handles = spawn_workers(Arc::clone(&sched), runner.clone());

        let id = Uuid::new_v4();
        let mut states = Vec::new();
        for _ in 0..50 {
            states.push(sched.request(id, key(1), 1024));
        }
        await_starts(&mut started, 1).await;
        quiesce().await;
        assert!(
            started.try_recv().is_err(),
            "fifty requests for one entry start one job, not fifty"
        );
        assert_eq!(states[0], RequestState::Queued);
        assert!(
            states[1..]
                .iter()
                .all(|s| matches!(s, RequestState::Queued | RequestState::Running)),
            "a repeat request is never a second job"
        );
        assert_eq!(runner.runs.load(Ordering::SeqCst), 1);

        gate.add_permits(10);
        let (delivered_id, _) = rx.recv().await.expect("one delivery");
        assert_eq!(delivered_id, id);
        quiesce().await;
        assert_eq!(
            runner.runs.load(Ordering::SeqCst),
            1,
            "still exactly one run"
        );
        assert!(rx.try_recv().is_err(), "exactly one delivery, not fifty");

        sched.stop();
        for h in handles {
            let _ = h.await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_cancelled_running_entry_delivers_nothing_and_caches_nothing() {
        let sched = Arc::new(PreviewScheduler::new(1, MAX_PREVIEW_SESSION_BYTES));
        let FakeHandles {
            runner,
            delivered: mut rx,
            mut started,
            gate,
        } = FakeRunner::fixture(0);
        let handles = spawn_workers(Arc::clone(&sched), runner.clone());

        let id = Uuid::new_v4();
        sched.request(id, key(7), 1024);
        await_starts(&mut started, 1).await;
        assert_eq!(
            sched.running_len(),
            1,
            "the job is running before we cancel"
        );

        assert!(sched.cancel(id), "cancelling a running entry reports so");
        gate.add_permits(1);
        quiesce().await;

        assert_eq!(
            runner.runs.load(Ordering::SeqCst),
            1,
            "it did run to completion"
        );
        assert!(
            rx.try_recv().is_err(),
            "a cancelled entry must not deliver a result"
        );
        assert_eq!(
            sched.cache_len(),
            0,
            "a cancelled entry's result must not be cached"
        );
        assert!(sched.cached(&key(7)).is_none());

        sched.stop();
        for h in handles {
            let _ = h.await;
        }
    }

    #[test]
    fn a_cancelled_queued_entry_is_never_taken() {
        let sched = PreviewScheduler::new(2, MAX_PREVIEW_SESSION_BYTES);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        sched.request(a, key(1), 1024);
        sched.request(b, key(2), 1024);
        assert!(sched.cancel(a));
        assert_eq!(sched.queued_len(), 1);
        let job = sched.take_next().expect("b is still queued");
        assert_eq!(
            job.entry_id, b,
            "the cancelled entry is gone from the queue"
        );
        assert!(sched.take_next().is_none(), "and nothing else is left");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_cache_answers_without_rerunning_and_invalidates_on_size_and_mtime() {
        let sched = Arc::new(PreviewScheduler::new(2, MAX_PREVIEW_SESSION_BYTES));
        let FakeHandles {
            runner,
            delivered: mut rx,
            gate: _gate,
            started: _started,
        } = FakeRunner::fixture(1000);
        let handles = spawn_workers(Arc::clone(&sched), runner.clone());

        let id = Uuid::new_v4();
        let k0 = key(11);
        assert_eq!(sched.request(id, k0.clone(), 1024), RequestState::Queued);
        rx.recv().await.expect("first build delivers");
        assert_eq!(runner.runs.load(Ordering::SeqCst), 1);

        // Same key: answered from cache, with the real summary, no run.
        match sched.request(id, k0.clone(), 1024) {
            RequestState::Cached(PreviewOutcome::Ready(v)) => {
                assert_eq!(
                    v["entry_id"],
                    id.to_string(),
                    "the cached summary, not a stub"
                );
            }
            other => panic!("expected a cache hit, got {other:?}"),
        }
        assert_eq!(
            runner.runs.load(Ordering::SeqCst),
            1,
            "a cache hit runs nothing"
        );

        // Size changed: miss.
        let mut k_size = k0.clone();
        k_size.file_bytes += 1;
        assert_eq!(sched.request(id, k_size, 1024), RequestState::Queued);
        rx.recv().await.expect("resized file rebuilds");
        assert_eq!(runner.runs.load(Ordering::SeqCst), 2);

        // mtime changed: miss.
        let mut k_mtime = k0.clone();
        k_mtime.file_modified = Some((1_700_000_001, 0));
        assert_eq!(sched.request(id, k_mtime, 1024), RequestState::Queued);
        rx.recv().await.expect("touched file rebuilds");
        assert_eq!(runner.runs.load(Ordering::SeqCst), 3);

        // Group size changed -- a subagent transcript grew: miss.
        let mut k_group = k0.clone();
        k_group.group_bytes += 1;
        assert_eq!(sched.request(id, k_group, 1024), RequestState::Queued);
        rx.recv().await.expect("grown group rebuilds");
        assert_eq!(runner.runs.load(Ordering::SeqCst), 4);

        // Config fingerprint changed: miss.
        let mut k_cfg = k0.clone();
        k_cfg.config_fingerprint = "other".to_string();
        assert_eq!(sched.request(id, k_cfg, 1024), RequestState::Queued);
        rx.recv().await.expect("reconfigured device rebuilds");
        assert_eq!(runner.runs.load(Ordering::SeqCst), 5);

        // And the original key is still a hit throughout.
        assert!(matches!(
            sched.request(id, k0, 1024),
            RequestState::Cached(PreviewOutcome::Ready(_))
        ));
        assert_eq!(runner.runs.load(Ordering::SeqCst), 5);

        sched.stop();
        for h in handles {
            let _ = h.await;
        }
    }

    #[test]
    fn a_visible_entry_requested_after_four_hundred_others_is_taken_first() {
        let sched = PreviewScheduler::new(2, MAX_PREVIEW_SESSION_BYTES);
        let bulk: Vec<Uuid> = (0..400).map(|_| Uuid::new_v4()).collect();
        for (n, id) in bulk.iter().enumerate() {
            sched.request(*id, key(n as u64), 1024);
        }
        let late = Uuid::new_v4();
        sched.request(late, key(9999), 1024);
        sched.set_visible([late]);

        let first = sched.take_next().expect("a job is available");
        assert_eq!(
            first.entry_id, late,
            "the on-screen entry jumps 400 queued ahead of it"
        );
        // And the rest keep arrival order behind it.
        let second = sched.take_next().expect("more queued");
        assert_eq!(second.entry_id, bulk[0]);
    }

    /// Priority holds through the live worker pool, not only through
    /// `take_next`.
    ///
    /// Deliberately a **one-worker** pool. With two, "which job started
    /// third" is a race between two workers reaching `take_next`, not a
    /// property of the queue -- both would take a job, one of them the
    /// visible one, and asserting on the order they got there would be
    /// asserting on the scheduler of the machine running the test. One
    /// worker makes the claim exact: the next job to start after the one
    /// in flight is the visible one. The bound itself is tested separately.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn visibility_reorders_the_live_pool_not_just_the_queue() {
        let sched = Arc::new(PreviewScheduler::new(1, MAX_PREVIEW_SESSION_BYTES));
        let FakeHandles {
            runner,
            delivered: _delivered,
            mut started,
            gate,
        } = FakeRunner::fixture(0);
        let handles = spawn_workers(Arc::clone(&sched), runner.clone());

        let bulk: Vec<Uuid> = (0..400).map(|_| Uuid::new_v4()).collect();
        for (n, id) in bulk.iter().enumerate() {
            sched.request(*id, key(n as u64), 1024);
        }
        let first = await_starts(&mut started, 1).await;
        assert_eq!(first[0], bulk[0], "arrival order with nothing visible");
        // That job is parked inside the gate. It started before the visible
        // entry existed, so nothing can preempt it -- see `cancel`.
        assert_eq!(runner.in_flight.load(Ordering::SeqCst), 1);

        let late = Uuid::new_v4();
        sched.set_visible([late]);
        sched.request(late, key(9999), 1024);
        assert_eq!(
            sched.queued_len(),
            400,
            "399 bulk entries plus the late one"
        );

        gate.add_permits(401);
        let next = await_starts(&mut started, 2).await;
        assert_eq!(
            next[0], late,
            "the on-screen entry starts next, ahead of the 399 queued before it"
        );
        assert_eq!(
            next[1], bulk[1],
            "and arrival order resumes behind it once nothing is visible"
        );

        sched.stop();
        for h in handles {
            let _ = h.await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_oversized_session_is_refused_visibly_and_never_run() {
        let sched = Arc::new(PreviewScheduler::new(2, MAX_PREVIEW_SESSION_BYTES));
        let FakeHandles {
            runner,
            delivered: mut rx,
            gate: _gate,
            started: _started,
        } = FakeRunner::fixture(1000);
        let handles = spawn_workers(Arc::clone(&sched), runner.clone());

        let id = Uuid::new_v4();
        let huge = 367_500_000u64;
        assert!(!sched.admits(huge));
        sched.request(id, key(1), huge);
        let (delivered, outcome) = rx.recv().await.expect("a refusal is still a result");
        assert_eq!(delivered, id);
        assert_eq!(
            outcome,
            PreviewOutcome::TooLarge {
                raw_session_bytes: huge,
                limit_bytes: MAX_PREVIEW_SESSION_BYTES,
            }
        );
        assert_eq!(
            runner.runs.load(Ordering::SeqCst),
            0,
            "a 367 MB session is never parsed to draw a card"
        );

        // And the refusal carries no would-send number of any kind.
        let value = outcome.to_value(id);
        assert_eq!(value["state"], STATE_TOO_LARGE);
        assert_eq!(value["raw_session_bytes"], huge);
        assert!(
            value.get("summary").is_none() && value.get("would_send_bytes").is_none(),
            "a refused preview must not put an estimated size on a consent surface"
        );

        sched.stop();
        for h in handles {
            let _ = h.await;
        }
    }

    #[test]
    fn a_key_change_while_running_cancels_the_stale_build_and_queues_a_fresh_one() {
        let sched = PreviewScheduler::new(1, MAX_PREVIEW_SESSION_BYTES);
        let id = Uuid::new_v4();
        sched.request(id, key(1), 1024);
        let job = sched.take_next().expect("queued");
        assert_eq!(sched.running_len(), 1);

        let mut grown = key(1);
        grown.file_bytes = 2;
        assert_eq!(sched.request(id, grown, 1024), RequestState::Queued);

        // The in-flight build finishes against the old bytes and is dropped.
        assert!(
            !sched.finish(&job, PreviewOutcome::Ready(Arc::new(json!({})))),
            "a build against superseded bytes is not delivered"
        );
        assert_eq!(sched.cache_len(), 0);
        assert_eq!(sched.queued_len(), 1, "and a fresh job is waiting");
    }

    #[test]
    fn requesting_a_cancelled_running_entry_again_revives_its_delivery() {
        let sched = PreviewScheduler::new(1, MAX_PREVIEW_SESSION_BYTES);
        let id = Uuid::new_v4();
        let k = key(3);
        sched.request(id, k.clone(), 1024);
        let job = sched.take_next().expect("queued");
        assert!(sched.cancel(id));
        // Scrolled back into view before the build finished.
        assert_eq!(sched.request(id, k, 1024), RequestState::Running);
        assert!(
            sched.finish(&job, PreviewOutcome::Ready(Arc::new(json!({})))),
            "an entry asked for again while running is delivered after all"
        );
        assert_eq!(sched.cache_len(), 1);
    }

    #[test]
    fn preview_key_tracks_the_files_size_and_mtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, b"one").expect("write");
        let a = PreviewKey::for_entry(&path, 3, "fp".into());
        assert_eq!(a.file_bytes, 3);
        assert!(a.file_modified.is_some(), "a real file reports an mtime");

        std::fs::write(&path, b"one-two").expect("rewrite");
        let b = PreviewKey::for_entry(&path, 7, "fp".into());
        assert_ne!(a, b, "a rewritten file is a different key");
        assert_eq!(b.file_bytes, 7);

        let missing = PreviewKey::for_entry(&dir.path().join("gone.jsonl"), 0, "fp".into());
        assert_eq!(missing.file_bytes, 0);
        assert!(missing.file_modified.is_none());
    }
}
