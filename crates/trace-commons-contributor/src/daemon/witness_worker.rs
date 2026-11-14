//! The witness worker: the daemon's calls to the redaction witness, off the
//! periodic loop.
//!
//! The live witness is slow (about two minutes for a 241 KB session, with a
//! 300 s limit of its own), and the owner's direction (2026-11-05) is that
//! it never synchronously blocks anything. Before this module the upload
//! pass awaited each witness call inline, so one slow call held up the whole
//! tick: watching, expiry, history, account admission and every other
//! upload, for minutes per trace.
//!
//! # Shape
//!
//! - **Producer.** The upload pass (`drain_approved`) runs every gate it
//!   always ran (pause, quiesce, the unenroll hold, dev dry run, the approval
//!   hold, the project and override policy, chosen consent scopes, the
//!   approved scopes). An entry that passes and still needs the witness
//!   ([`needs_witness_job`]) is handed to [`WitnessWorker::enqueue`] instead
//!   of being claimed, and the pass moves on.
//! - **Lanes.** [`pump`] spawns jobs while a slot is free:
//!   [`BACKGROUND_SLOTS`] for the upload pass's work, declared background so
//!   a busy witness turns it away first. A person's explicit review
//!   (`witness_preview_request` with `background: true`) runs on its own
//!   [`INTERACTIVE_SLOTS`], so it never waits behind a background call.
//! - **Job.** Re-checks the cheap gates, then runs the same send gates,
//!   re-hash and approval-terms check as an upload
//!   (`Uploader::witness_entry`), sends the raw session to the witness, and
//!   on success pins the certified review on the still-approved entry
//!   (`Queue::pin_witnessed_approval`). The next upload pass sends exactly
//!   those bytes through the witness-pin path, which re-mints the claim,
//!   checks the certified grants and applies the risk and Scrub holds for a
//!   session nobody looked at. Nothing reaches ingest from here.
//! - **Results.** Any other outcome is recorded with the pass's own rules
//!   (`apply_upload_decision`): a busy witness defers the entry and holds the
//!   background lane until the witness's own delay. A witness whose evidence
//!   could not be fetched (`submit::witness_failure_is_transient`: timeout,
//!   unreachable, attestation or collateral unavailable) keeps the approval
//!   and is retried on its own backoff. Other traces continue unless three
//!   distinct traces fail within five minutes; then one recovery probe follows
//!   a short shared pause (see `witness_retry`). The entry
//!   keeps its approval for as long as that lasts, with no budget (owner,
//!   2026-11-12).
//!   Evidence that was fetched and did not verify
//!   (`submit::witness_trust_hold`) holds the entry for a person under
//!   `witness-not-trusted` or `witness-measurement-not-accepted`, never
//!   retried. Everything else follows the existing refusal rules.
//!
//! # Persistence and restart
//!
//! Nothing new is persisted. A job's state is the queue entry itself: it
//! stays `Approved` while the witness works, and the in-flight set lives in
//! memory. So a restart, a stop or a quiesce mid-call loses only the call:
//! the entry is still approved with no pin, and the first pass after the
//! restart enqueues it again. The raw session is sent to the witness again
//! only in that case, when the first request's result was never received; a
//! received result is pinned on disk before anything else happens, and an
//! entry holding a pin is never enqueued.
//!
//! A result applies only while the approval it was started under still
//! holds ([`JobKey`]): a cancel, a revoke, an unenroll or a person's own
//! review in the meantime and the result is dropped, nothing pinned.
//!
//! # Pause, stop, unenroll, panics
//!
//! A job checks pause, quiesce, the unenroll hold and shutdown before it
//! sends anything, and drops itself if any is set. A call already in flight
//! is not cut off by pause: the raw session has already left, and its
//! result is pinned but not uploaded until pause lifts (the pass is where
//! uploads stop). Unenroll raises `upload_interrupt`, which stops a job at
//! its next await point, and waits for the in-flight set to empty. A panic
//! in a job refuses that one entry under `redaction-panicked` (re-offered by
//! a newer build, as for a panicking upload) and holds the background lane
//! for a backoff; the worker itself carries on. A panic that left a state
//! lock poisoned stops the worker until the next start, as it stops the
//! passes. Logs are labels only.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::ipc::{self, DaemonShared};
use super::queue::{self, QueueEntry, QueueState};

/// Background witness calls in flight at once, once the witness is
/// answering. Two (owner, 2026-11-12): the witness runs two classify calls
/// at once and one job sends its windows one after another, so a single
/// job left the witness's paced NEAR AI budget (30 requests a minute)
/// mostly idle while a backlog waited.
///
/// The second slot opens only after a background call certified a review,
/// and closes on any other outcome ([`WorkerState::answering`]): a witness
/// that is failing or busy is still asked once, never once per slot. Past
/// what a healthy witness can take it answers busy, and the lane rests for
/// the delay it names.
pub(crate) const BACKGROUND_SLOTS: usize = 2;

/// A person's explicit reviews in flight at once, on a lane of their own so
/// a review never waits behind a background call.
pub(crate) const INTERACTIVE_SLOTS: usize = 1;

/// Most background jobs waiting for a slot. Anything past it stays approved
/// and is enqueued by a later pass.
const MAX_QUEUED: usize = 64;

/// How long one job may run before it is stopped: the upload pipeline's
/// allowance, which covers the witness's own deadline with room to spare.
const JOB_BOUND: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// After a panic, the background lane rests this long, doubling to
/// [`PANIC_MAX_BACKOFF`].
const PANIC_FIRST_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);
const PANIC_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Diagnostics events, label-only.
pub(crate) const EVENT_JOB_PANICKED: &str = "witness_job_panicked";
pub(crate) const EVENT_JOB_STOPPED: &str = "witness_job_stopped";

/// Whether an approved entry still needs a witness call before it can be
/// uploaded: a witness is configured and the entry holds no pin.
///
/// An entry pinned to a local preview under a witness is not sent here: the
/// upload refuses it (`witness_certificate_missing`) as before. Phase 4 of
/// the witness-at-submission spec routes those through this worker with its
/// subset check.
pub(crate) fn needs_witness_job(
    cfg: &crate::config::ContributorConfig,
    entry: &QueueEntry,
) -> bool {
    cfg.witness.is_some() && entry.previewed_envelope_digest.is_none()
}

/// The approval a job was started under. A result is applied only while
/// the entry is still approved under exactly these terms with no pin.
#[derive(Debug, PartialEq, Eq)]
struct JobKey<'a> {
    session_hash: &'a str,
    approved_at: Option<DateTime<Utc>>,
    approved_inputs: Option<&'a str>,
    approved_scopes: Option<&'a [String]>,
    approved_verdict: Option<&'a str>,
    approved_correction: Option<&'a str>,
    approved_unattended: bool,
}

fn key_of(e: &QueueEntry) -> JobKey<'_> {
    JobKey {
        session_hash: &e.session_hash,
        approved_at: e.approved_at,
        approved_inputs: e.approved_inputs.as_deref(),
        approved_scopes: e.approved_scopes.as_deref(),
        approved_verdict: e.approved_verdict.as_deref(),
        approved_correction: e.approved_correction.as_deref(),
        approved_unattended: e.approved_unattended,
    }
}

/// Whether `current` is still the approval `snapshot` was taken of.
fn still_current(current: Option<&QueueEntry>, snapshot: &QueueEntry) -> bool {
    current.is_some_and(|e| {
        e.state == QueueState::Approved
            && e.previewed_envelope_digest.is_none()
            && key_of(e) == key_of(snapshot)
    })
}

/// The clock a job runs on: the instant its pass reckoned with, advanced by
/// the monotonic time since. A test hands the pass a clock of its own, and a
/// job's deadlines and retry times must agree with the pass that queued it.
#[derive(Debug, Clone, Copy)]
struct Clock {
    base: DateTime<Utc>,
    at: std::time::Instant,
    wall_at: DateTime<Utc>,
}

impl Clock {
    fn starting(base: DateTime<Utc>) -> Self {
        Self {
            base,
            at: std::time::Instant::now(),
            wall_at: Utc::now(),
        }
    }

    fn now(&self) -> DateTime<Utc> {
        // Instant may stop during sleep. Keep injected test clocks relative,
        // while accounting for wall time that passed since the job started.
        let elapsed = chrono::Duration::from_std(self.at.elapsed()).unwrap_or_default();
        self.base + elapsed.max(Utc::now() - self.wall_at)
    }
}

/// One queued background job.
struct Job {
    entry: QueueEntry,
    clock: Clock,
}

#[derive(Default)]
struct WorkerState {
    queued: VecDeque<Job>,
    /// Background jobs running, by entry.
    in_flight: HashSet<Uuid>,
    /// A person's reviews running, by entry.
    interactive: HashSet<Uuid>,
    /// The background lane takes nothing new before this: the witness's own
    /// delay after it answered busy, or a panic backoff.
    held_until: Option<DateTime<Utc>>,
    circuit: super::witness_retry::Circuit,
    /// The next panic backoff.
    panic_backoff: Option<std::time::Duration>,
    /// The last background call certified a review, so every slot is open.
    /// False at start and after any other outcome: one slot, one probe.
    answering: bool,
    /// A job panicked over a poisoned state lock: nothing more runs until
    /// the next start.
    stopped: bool,
}

/// The worker's state, owned by `DaemonShared`. Built without a runtime;
/// jobs are spawned by [`pump`].
#[derive(Default)]
pub(crate) struct WitnessWorker {
    state: Mutex<WorkerState>,
    /// Bumped whenever a background job finishes, for tests that wait on one.
    #[cfg(test)]
    pub(crate) finished: tokio::sync::Notify,
}

impl WitnessWorker {
    fn lock(&self) -> std::sync::MutexGuard<'_, WorkerState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queue an approved entry the pass has vetted. `false` when it is
    /// already queued or in flight, or the queue is full.
    pub(crate) fn enqueue(&self, entry: QueueEntry, now: DateTime<Utc>) -> bool {
        let mut s = self.lock();
        if s.stopped || s.in_flight.contains(&entry.entry_id) {
            return false;
        }
        if let Some(job) = s
            .queued
            .iter_mut()
            .find(|j| j.entry.entry_id == entry.entry_id)
        {
            // The producer re-vetted this approval. A cancel and manual
            // reapproval must replace the obsolete snapshot and priority,
            // without duplicating a queued or already-running request.
            job.entry = entry;
            return false;
        }
        if s.queued.len() >= MAX_QUEUED {
            let worst = s
                .queued
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| {
                    super::compare_send_priority(&left.entry, &right.entry)
                })
                .map(|(index, job)| (index, &job.entry));
            let Some((index, worst)) = worst else {
                return false;
            };
            if !super::compare_send_priority(&entry, worst).is_lt() {
                return false;
            }
            // Displacement only removes the in-memory scheduling hint. The
            // entry remains approved in the durable queue for a later pass.
            s.queued.remove(index);
        }
        s.queued.push_back(Job {
            entry,
            clock: Clock::starting(now),
        });
        true
    }

    /// Whether a background job for `entry_id` is running.
    pub(crate) fn is_in_flight(&self, entry_id: Uuid) -> bool {
        self.lock().in_flight.contains(&entry_id)
    }

    /// Background jobs waiting for a slot.
    pub(crate) fn queued(&self) -> usize {
        self.lock().queued.len()
    }

    /// Witness calls running now, background and interactive.
    pub(crate) fn in_flight(&self) -> usize {
        let s = self.lock();
        s.in_flight.len() + s.interactive.len()
    }

    /// Whether no background job is running and none could start now:
    /// nothing queued, or the lane is held or stopped.
    pub(crate) fn idle(&self) -> bool {
        let s = self.lock();
        s.in_flight.is_empty()
            && (s.queued.is_empty()
                || s.held_until.is_some()
                || s.circuit.recovering()
                || s.stopped)
    }

    /// Claim the interactive lane for a person's review of `entry_id`.
    /// `false` when the lane is full or that entry is already in review.
    pub(crate) fn start_interactive(&self, entry_id: Uuid) -> bool {
        let mut s = self.lock();
        if s.interactive.len() >= INTERACTIVE_SLOTS || s.interactive.contains(&entry_id) {
            return false;
        }
        s.interactive.insert(entry_id);
        true
    }

    /// Whether a person's review of `entry_id` is running.
    pub(crate) fn in_interactive_review(&self, entry_id: Uuid) -> bool {
        self.lock().interactive.contains(&entry_id)
    }

    /// Release the interactive lane. Paired with [`Self::start_interactive`].
    pub(crate) fn finish_interactive(&self, entry_id: Uuid) {
        self.lock().interactive.remove(&entry_id);
    }

    /// Drop everything queued and not yet started (unenroll returns those
    /// approvals; the next pass re-vets anything still approved).
    pub(crate) fn clear_queued(&self) {
        self.lock().queued.clear();
    }

    /// Take the next background job if a slot is free and the lane is not
    /// held, marking it in flight.
    fn take_next(&self, now: DateTime<Utc>) -> Option<Job> {
        let mut s = self.lock();
        if s.stopped
            || s.in_flight.len()
                >= if s.answering && !s.circuit.recovering() {
                    BACKGROUND_SLOTS
                } else {
                    1
                }
            || s.held_until.is_some_and(|until| now < until)
            || !s.circuit.can_start(now)
        {
            return None;
        }
        s.held_until = None;
        // Re-sort at dispatch, too: a manual approval can arrive behind an
        // already populated background queue. An outage probe prefers a trace
        // other than the one that last failed, without changing approvals.
        let avoid = s.circuit.avoid_probe();
        let index = s
            .queued
            .iter()
            .enumerate()
            .filter(|(_, job)| job.entry.ready_for_upload(now))
            .min_by(|(_, left), (_, right)| {
                (Some(left.entry.entry_id) == avoid)
                    .cmp(&(Some(right.entry.entry_id) == avoid))
                    .then_with(|| super::compare_send_priority(&left.entry, &right.entry))
            })
            .map(|(index, _)| index)?;
        let mut job = s.queued.remove(index)?;
        job.clock = Clock::starting(now);
        s.circuit.started(job.entry.entry_id);
        s.in_flight.insert(job.entry.entry_id);
        Some(job)
    }

    /// Take every queued job, for the busy-witness deferral.
    fn take_queued(&self) -> Vec<Uuid> {
        self.lock()
            .queued
            .drain(..)
            .map(|j| j.entry.entry_id)
            .collect()
    }

    fn finish(&self, entry_id: Uuid, end: &JobEnd, now: DateTime<Utc>) {
        let mut s = self.lock();
        s.in_flight.remove(&entry_id);
        // Only a certified review opens the second slot; a failure, a busy
        // answer or a panic closes it. A dropped job says nothing either way.
        match end {
            JobEnd::Witnessed => {
                // A request dispatched before a capacity/panic pause is
                // not its recovery probe, even if it finishes after expiry.
                s.answering = s.circuit.succeeded(entry_id) && s.held_until.is_none();
            }
            JobEnd::Dropped => {}
            _ => s.answering = false,
        }
        if !matches!(end, JobEnd::Witnessed | JobEnd::Transient) {
            s.circuit.cancelled(entry_id);
        }
        match end {
            JobEnd::Transient => s.circuit.failed(entry_id, now),
            JobEnd::Busy(until) => {
                s.held_until = Some(s.held_until.map_or(*until, |held| held.max(*until)));
            }
            JobEnd::Panicked => {
                let wait = s.panic_backoff.unwrap_or(PANIC_FIRST_BACKOFF);
                s.panic_backoff = Some(wait.saturating_mul(2).min(PANIC_MAX_BACKOFF));
                let until =
                    now + chrono::Duration::from_std(wait).unwrap_or(chrono::Duration::seconds(5));
                s.held_until = Some(s.held_until.map_or(until, |held| held.max(until)));
            }
            JobEnd::Poisoned => {
                s.stopped = true;
                s.queued.clear();
            }
            JobEnd::Witnessed | JobEnd::Done | JobEnd::Dropped => {
                s.panic_backoff = None;
            }
        }
    }
}

/// How one job ended, for the lane.
#[derive(Debug)]
enum JobEnd {
    /// The witness certified a review and it was pinned.
    Witnessed,
    /// Some other result was recorded.
    Done,
    /// Nothing was recorded; a later pass re-vets the entry.
    Dropped,
    /// The witness answered busy; the lane rests until then.
    Busy(DateTime<Utc>),
    /// Only this trace failed; the circuit decides whether failures correlate.
    Transient,
    Panicked,
    Poisoned,
}

/// Start every background job a free slot can take. Spawned, never awaited.
/// A no-op outside a Tokio runtime.
pub(crate) fn pump(shared: &Arc<DaemonShared>, now: DateTime<Utc>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    while let Some(Job { entry, clock }) = shared.witness_worker.take_next(now) {
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            let id = entry.entry_id;
            let end = match crate::envelope::catch_unwind_future(job(&shared, &entry, clock)).await
            {
                Ok(end) => end,
                Err(()) => panicked(&shared, &entry),
            };
            shared.witness_worker.finish(id, &end, clock.now());
            #[cfg(test)]
            shared.witness_worker.finished.notify_waiters();
            if !matches!(end, JobEnd::Poisoned) {
                pump(&shared, clock.now());
            }
        });
    }
}

/// A job panicked: refuse its entry under the panic label, unless the panic
/// left shared state poisoned, which stops the worker until the next start.
fn panicked(shared: &DaemonShared, entry: &QueueEntry) -> JobEnd {
    if let Some(lock) = shared.poisoned_state_lock() {
        tracing::error!(pass = "witness", "witness job panicked over state");
        super::diagnostics_log::record_labels(
            tracing::Level::ERROR,
            EVENT_JOB_PANICKED,
            &[("lock", lock)],
        );
        return JobEnd::Poisoned;
    }
    tracing::warn!(pass = "witness", "witness job panicked; refusing its entry");
    super::diagnostics_log::record_labels(
        tracing::Level::WARN,
        EVENT_JOB_PANICKED,
        &[("lock", "none")],
    );
    {
        let mut q = shared.queue.lock().expect("queue lock");
        if q.get(entry.entry_id)
            .is_some_and(|e| e.state == QueueState::Approved)
        {
            q.set_state(
                entry.entry_id,
                QueueState::Refused,
                Some(crate::envelope::REASON_REDACTION_PANICKED.to_string()),
            );
            q.note_refusal_version(entry.entry_id, env!("CARGO_PKG_VERSION"));
            if q.save(&shared.store).is_err() {
                tracing::warn!("could not persist a witness job refusal");
            }
        }
    }
    shared.publish(ipc::EVENT_QUEUE_CHANGED, serde_json::json!({}));
    JobEnd::Panicked
}

/// Whether nothing may be sent right now.
fn sends_held(shared: &DaemonShared, now: DateTime<Utc>) -> bool {
    shared.shutdown.load(Ordering::Relaxed)
        || shared.is_paused(now)
        || shared.quiesced.load(Ordering::Relaxed)
        || shared.uploads_held.load(Ordering::SeqCst)
        || shared.terms_held.load(Ordering::SeqCst)
        || shared.dev_dry_run
}

/// Whether the project and override policy still allow this send. The
/// same questions the pass asks before it enqueues, asked again because a
/// job may start well after the pass vetted it.
fn policy_allows(policy: &super::policy::ProjectPolicy, entry: &QueueEntry) -> bool {
    if policy.holds_every_send() {
        return false;
    }
    if !entry.approved_unattended {
        return true;
    }
    policy.resolve(&entry.project_key) == super::policy::ProjectMode::AutoUpload
        && !policy.waits_for_a_person_at_send(&entry.project_key, &entry.path.to_string_lossy())
}

/// One background job. Never uploads.
async fn job(shared: &Arc<DaemonShared>, snapshot: &QueueEntry, clock: Clock) -> JobEnd {
    #[cfg(test)]
    if let Some(hook) = test_hook(shared.store.dir()) {
        hook(snapshot);
    }
    let now = clock.now();
    if sends_held(shared, now) {
        return JobEnd::Dropped;
    }
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return JobEnd::Dropped;
    };
    if crate::config::consent_hold(Some(&cfg)).is_some() {
        return JobEnd::Dropped;
    }
    // Policy before queue, as everywhere.
    let entry = {
        let policy = shared.policy.lock().expect("policy lock");
        let q = shared.queue.lock().expect("queue lock");
        let current = q.get(snapshot.entry_id);
        if !still_current(current, snapshot)
            || !current.is_some_and(|e| e.ready_for_upload(now))
            || !policy_allows(&policy, snapshot)
            || snapshot.approved_scopes.as_deref() != Some(cfg.consent_scopes.as_slice())
            || !needs_witness_job(&cfg, snapshot)
        {
            return JobEnd::Dropped;
        }
        current.cloned().expect("checked above")
    };

    let sources = crate::source::all_sources(&shared.source_roots_with_routing());
    let found = super::run_blocking(|| super::find_session(&sources, &entry));
    let Some((source, session_ref)) = found else {
        let mut q = shared.queue.lock().expect("queue lock");
        if !still_current(q.get(entry.entry_id), &entry) {
            return JobEnd::Dropped;
        }
        q.set_state(
            entry.entry_id,
            QueueState::Failed,
            Some("session-file-vanished".to_string()),
        );
        let _ = q.save(&shared.store);
        drop(q);
        shared.publish(ipc::EVENT_QUEUE_CHANGED, serde_json::json!({}));
        return JobEnd::Done;
    };

    let near_ai = shared
        .settings
        .lock()
        .expect("settings lock")
        .near_ai
        .clone();
    let opts = crate::submit::SubmitOptions {
        dry_run: false,
        pii_filter: cfg.pii_filter.clone(),
        no_reasoning: false,
        machine_readable: true,
        unenrolled_preview: false,
        remediate_quarantined: false,
        verdict: None,
    };
    let Ok(store) =
        super::run_blocking(|| crate::config::ConfigStore::open(shared.store.dir().to_path_buf()))
    else {
        return JobEnd::Dropped;
    };
    let Ok(mut ctx) =
        super::run_blocking(|| crate::submit::SubmitContext::new(&store, &cfg, &opts, near_ai))
    else {
        return JobEnd::Dropped;
    };
    // Nobody is waiting on this call, so it says so, and a witness that
    // keeps a slot for a person's review turns it away first (#1014).
    ctx.witness_as_background();

    let mut interrupt = shared.upload_interrupt.subscribe();
    let mut state = shared.state.lock().expect("state lock").clone();
    let settings = shared.settings.lock().expect("settings lock").clone();
    // A scratch copy. A job runs for minutes while the tick's passes keep
    // changing health, so it is never written back whole: only the label
    // this job's own outcome implies is applied, below.
    let mut health = shared.health.lock().expect("health lock").clone();
    let raced = {
        let mut up = super::uploader::Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        super::race_upload(
            up.witness_entry(source, &session_ref, &entry, now),
            JOB_BOUND,
            &mut interrupt,
        )
        .await
    };
    let outcome = match raced {
        super::RacedUpload::Settled(outcome) => outcome,
        super::RacedUpload::Stopped(why) => {
            super::diagnostics_log::record_labels(
                tracing::Level::WARN,
                EVENT_JOB_STOPPED,
                &[("reason", why.label())],
            );
            tracing::warn!(reason = why.label(), "witness job stopped");
            // One that hung waits before it is tried again, as a hung
            // upload does, so it does not hold the lane every pass.
            if why == super::UploadStop::TimedOut {
                let mut q = shared.queue.lock().expect("queue lock");
                if still_current(q.get(entry.entry_id), &entry) {
                    q.defer_without_attempt(entry.entry_id, clock.now() + super::UPLOAD_HUNG_RETRY);
                    let _ = q.save(&shared.store);
                }
            }
            return JobEnd::Dropped;
        }
    };
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(e) => {
            // A fail-closed precondition: its label is what suspends queue
            // expiry, so it reaches shared health.
            let label = super::uploader::precondition_health_label(&e);
            shared
                .health
                .lock()
                .expect("health lock")
                .fail(label, clock.now());
            tracing::warn!(pass = "witness", reason = label, "witness job failed");
            return JobEnd::Dropped;
        }
    };
    if let super::uploader::WitnessJobOutcome::Decision(decision) = &outcome
        && let Some(label) = super::uploader::health_label_for(decision)
    {
        shared
            .health
            .lock()
            .expect("health lock")
            .fail(label, clock.now());
    }
    record(shared, &entry, &session_ref, outcome, clock)
}

/// Record a job's outcome, if the approval it ran under still holds.
fn record(
    shared: &DaemonShared,
    entry: &QueueEntry,
    session_ref: &crate::source::SessionRef,
    outcome: super::uploader::WitnessJobOutcome,
    clock: Clock,
) -> JobEnd {
    let mut q = shared.queue.lock().expect("queue lock");
    if shared.uploads_held.load(Ordering::SeqCst) || !still_current(q.get(entry.entry_id), entry) {
        return JobEnd::Dropped;
    }
    let end = match outcome {
        super::uploader::WitnessJobOutcome::Witnessed(artifact) => {
            let Ok(pin) = artifact.digest() else {
                return JobEnd::Dropped;
            };
            // Saved under the queue lock, so a sweep cannot run between the
            // write and the pin that keeps it.
            if super::approved_envelope::save_witnessed(&shared.store, entry.entry_id, &artifact)
                .is_err()
            {
                tracing::warn!("could not save a witnessed review");
                return JobEnd::Dropped;
            }
            let previous = q.clone();
            // K10: the certified response's own bytes, as an upload of this
            // pin would send them.
            let would_send_bytes = Some(artifact.response().envelope_bytes.len() as u64);
            if !q.pin_witnessed_approval(
                entry.entry_id,
                &pin,
                artifact.attested_inference().cloned(),
                would_send_bytes,
            ) || q.save(&shared.store).is_err()
            {
                *q = previous;
                let _ = super::approved_envelope::sweep(&shared.store, &q.pinned_entry_ids());
                return JobEnd::Dropped;
            }
            drop(q);
            // The pinned bytes go out on the next pass, brought forward.
            shared.request_upload_at(Utc::now());
            JobEnd::Witnessed
        }
        super::uploader::WitnessJobOutcome::Decision(decision) => {
            let observed_at = || clock.now();
            let transient = matches!(&decision,
                super::uploader::UploadDecision::Failed { reason_label }
                if crate::submit::witness_failure_is_transient(reason_label));
            let applied = super::apply_upload_decision(
                shared,
                q,
                entry,
                decision,
                session_ref,
                clock.now(),
                &observed_at,
                None,
                None,
            );
            if super::note_witness_blocked(shared, &applied) {
                let mut state = shared.state.lock().expect("state lock");
                let _ = state.save(&shared.store);
            }
            // Explicit saturation pauses the lane. Other transient failures
            // belong to this entry; do not discard healthy work behind it.
            let held = if applied.witness_busy_until.is_some() {
                shared.witness_worker.take_queued()
            } else {
                Vec::new()
            };
            {
                let mut q = shared.queue.lock().expect("queue lock");
                if let Some((until, retry_after_secs)) = applied.witness_busy_until {
                    for id in held {
                        let held_until =
                            until + super::witness_capacity_jitter(id, retry_after_secs);
                        q.defer_for_witness_capacity(id, held_until);
                    }
                }
                if q.save(&shared.store).is_err() {
                    tracing::warn!("could not persist a witness job result");
                }
                let _ = super::approved_envelope::sweep(&shared.store, &q.pinned_entry_ids());
            }
            match applied.witness_busy_until {
                Some((until, _)) => JobEnd::Busy(until),
                None if transient => JobEnd::Transient,
                None => JobEnd::Done,
            }
        }
    };
    shared.publish(ipc::EVENT_QUEUE_CHANGED, serde_json::json!({}));
    end
}

/// How many approved entries are waiting on, or in, the privacy check:
/// approved with a witness configured and no certified review pinned yet,
/// whether queued, running, or waiting out a retry. Label-free.
pub(crate) fn privacy_check_waiting(
    cfg: Option<&crate::config::ContributorConfig>,
    queue: &queue::Queue,
) -> usize {
    let Some(cfg) = cfg.filter(|c| c.witness.is_some()) else {
        return 0;
    };
    queue
        .all()
        .iter()
        .filter(|e| e.state == QueueState::Approved && needs_witness_job(cfg, e))
        .count()
}

#[cfg(test)]
type TestHook = fn(&QueueEntry);

#[cfg(test)]
static TEST_HOOKS: std::sync::Mutex<
    Option<std::collections::HashMap<std::path::PathBuf, TestHook>>,
> = std::sync::Mutex::new(None);

#[cfg(test)]
fn test_hook(dir: &std::path::Path) -> Option<TestHook> {
    TEST_HOOKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(dir).copied())
}

/// Run `hook` at the start of every job for one state directory (a test
/// that makes a job panic).
#[cfg(test)]
pub(crate) fn set_test_hook(dir: &std::path::Path, hook: TestHook) {
    TEST_HOOKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(dir.to_path_buf(), hook);
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    fn now() -> DateTime<Utc> {
        Utc::now()
    }
    fn entry(n: u128, attempts: u32, manual: bool) -> QueueEntry {
        QueueEntry {
            entry_id: Uuid::from_u128(n),
            state: QueueState::Approved,
            approved_unattended: !manual,
            attempts,
            ..Default::default()
        }
    }

    #[test]
    fn an_isolated_failure_does_not_drop_or_delay_healthy_queued_jobs() {
        let w = WitnessWorker::default();
        let now = now();
        for n in 1..=4 {
            w.enqueue(entry(n, 0, false), now);
        }
        let first = w.take_next(now).unwrap();
        w.finish(first.entry.entry_id, &JobEnd::Transient, now);
        assert_eq!(w.queued(), 3);
        assert_eq!(w.take_next(now).unwrap().entry.entry_id, Uuid::from_u128(2));
    }

    #[test]
    fn a_shared_pause_recovers_on_one_different_trace_and_then_opens_two_slots() {
        let w = WitnessWorker::default();
        let now = now();
        for n in 1..=7 {
            w.enqueue(entry(n, 0, false), now);
        }
        for _ in 0..3 {
            let job = w.take_next(now).unwrap();
            w.finish(job.entry.entry_id, &JobEnd::Transient, now);
        }
        assert!(w.take_next(now).is_none());
        // Even a manual retry of the last failure is not the recovery probe
        // while another approved trace is available.
        w.enqueue(entry(3, 1, true), now);
        let later = now + chrono::Duration::seconds(30);
        let probe = w.take_next(later).unwrap();
        assert_ne!(probe.entry.entry_id, Uuid::from_u128(3));
        assert!(w.take_next(later).is_none());
        w.finish(probe.entry.entry_id, &JobEnd::Witnessed, later);
        assert!(w.take_next(later).is_some());
        assert!(w.take_next(later).is_some());
        assert!(w.take_next(later).is_none());
    }

    #[test]
    fn late_manual_approval_preempts_a_full_background_queue() {
        let w = WitnessWorker::default();
        let now = now();
        for n in 1..=MAX_QUEUED {
            assert!(w.enqueue(entry(n as u128, 1, false), now));
        }
        assert!(w.enqueue(entry(999, 0, true), now));
        assert_eq!(w.queued(), MAX_QUEUED);
        assert_eq!(
            w.take_next(now).unwrap().entry.entry_id,
            Uuid::from_u128(999)
        );
    }

    #[test]
    fn cancelling_then_manually_reapproving_refreshes_a_queued_automatic_snapshot() {
        let w = WitnessWorker::default();
        let now = now();
        let mut q = queue::Queue::new();
        let original = entry(2, 0, false);
        let id = original.entry_id;
        q.push_for_test(original.clone());
        w.enqueue(entry(1, 0, false), now);
        w.enqueue(original.clone(), now);
        q.cancel(id).unwrap();
        assert!(q.approve(id, &[], None, None, None, Some(now)));
        let renewed = q.get(id).unwrap().clone();
        assert!(!still_current(Some(&renewed), &original));
        assert!(!w.enqueue(renewed.clone(), now));
        assert_eq!(w.queued(), 2);
        let job = w.take_next(now).unwrap();
        assert_eq!(job.entry.entry_id, id);
        assert!(still_current(Some(&renewed), &job.entry));
        assert!(!job.entry.approved_unattended);
        assert!(
            !w.enqueue(renewed, now),
            "in-flight jobs remain deduplicated"
        );
    }

    #[test]
    fn new_work_precedes_retries_already_waiting_in_the_worker() {
        let w = WitnessWorker::default();
        let now = now();
        w.enqueue(entry(1, 17, false), now);
        w.enqueue(entry(2, 0, false), now);
        assert_eq!(w.take_next(now).unwrap().entry.entry_id, Uuid::from_u128(2));
    }

    #[test]
    fn saturation_deadline_survives_a_late_success_and_restarts_with_one_job() {
        let w = WitnessWorker::default();
        let now = now();
        w.finish(Uuid::from_u128(90), &JobEnd::Witnessed, now);
        for n in 1..=5 {
            w.enqueue(entry(n, 0, false), now);
        }
        let a = w.take_next(now).unwrap();
        let b = w.take_next(now).unwrap();
        w.finish(
            a.entry.entry_id,
            &JobEnd::Busy(now + chrono::Duration::seconds(120)),
            now,
        );
        w.finish(b.entry.entry_id, &JobEnd::Witnessed, now);
        assert!(w.take_next(now + chrono::Duration::seconds(119)).is_none());
        assert!(w.take_next(now + chrono::Duration::seconds(120)).is_some());
        assert!(w.take_next(now + chrono::Duration::seconds(120)).is_none());
    }

    #[test]
    fn a_pre_pause_success_after_expiry_still_requires_one_fresh_recovery_job() {
        let w = WitnessWorker::default();
        let now = now();
        w.finish(Uuid::from_u128(90), &JobEnd::Witnessed, now);
        for n in 1..=6 {
            w.enqueue(entry(n, 0, false), now);
        }
        let a = w.take_next(now).unwrap();
        let b = w.take_next(now).unwrap();
        w.finish(
            a.entry.entry_id,
            &JobEnd::Busy(now + chrono::Duration::seconds(120)),
            now,
        );
        assert!(w.take_next(now + chrono::Duration::seconds(119)).is_none());
        let after = now + chrono::Duration::seconds(121);
        w.finish(b.entry.entry_id, &JobEnd::Witnessed, after);
        let probe = w.take_next(after).unwrap();
        assert!(w.take_next(after).is_none());
        w.finish(probe.entry.entry_id, &JobEnd::Witnessed, after);
        assert!(w.take_next(after).is_some());
        assert!(w.take_next(after).is_some());
        assert!(w.take_next(after).is_none());
    }

    #[test]
    fn a_future_entry_deadline_remains_in_force_after_worker_restart() {
        let w = WitnessWorker::default();
        let now = now();
        let mut retry = entry(1, 17, false);
        retry.reason_label = Some("witness_unreachable".into());
        retry.retry_after = Some(now + chrono::Duration::hours(1));
        w.enqueue(retry, now);
        assert!(w.take_next(now).is_none());
        assert!(w.take_next(now + chrono::Duration::hours(1)).is_some());
    }

    #[test]
    fn wall_time_after_sleep_advances_the_job_clock() {
        let base = now();
        let mut clock = Clock::starting(base);
        clock.wall_at -= chrono::Duration::hours(2);
        assert!(clock.now() >= base + chrono::Duration::hours(2));
    }
}
