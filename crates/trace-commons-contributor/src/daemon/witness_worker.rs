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
//!   and is retried on the transient-upload backoff, and the lane rests
//!   until that retry, as the pass once stopped at the first; once the
//!   budget is spent the entry waits for a person under the same label.
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

/// Background witness calls in flight at once. One: the witness is a single
/// enclave that serves background work last, and a second background call
/// only queues behind the first on its side.
pub(crate) const BACKGROUND_SLOTS: usize = 1;

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
}

impl Clock {
    fn starting(base: DateTime<Utc>) -> Self {
        Self {
            base,
            at: std::time::Instant::now(),
        }
    }

    fn now(&self) -> DateTime<Utc> {
        self.base + chrono::Duration::from_std(self.at.elapsed()).unwrap_or_default()
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
    /// The next panic backoff.
    panic_backoff: Option<std::time::Duration>,
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
        if s.stopped
            || s.in_flight.contains(&entry.entry_id)
            || s.queued.iter().any(|j| j.entry.entry_id == entry.entry_id)
            || s.queued.len() >= MAX_QUEUED
        {
            return false;
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
        s.in_flight.is_empty() && (s.queued.is_empty() || s.held_until.is_some() || s.stopped)
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
            || s.in_flight.len() >= BACKGROUND_SLOTS
            || s.held_until.is_some_and(|until| now < until)
        {
            return None;
        }
        s.held_until = None;
        let job = s.queued.pop_front()?;
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
        match end {
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
            JobEnd::Done | JobEnd::Dropped => {
                s.panic_backoff = None;
            }
        }
    }
}

/// How one job ended, for the lane.
#[derive(Debug)]
enum JobEnd {
    /// A result was recorded.
    Done,
    /// Nothing was recorded; a later pass re-vets the entry.
    Dropped,
    /// The witness answered busy; the lane rests until then.
    Busy(DateTime<Utc>),
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
            JobEnd::Done
        }
        super::uploader::WitnessJobOutcome::Decision(decision) => {
            let observed_at = || clock.now();
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
            // A busy witness is asked once, not once per waiting session:
            // everything queued behind this job is held until the witness's
            // own delay, with the same per-session spread the pass used.
            // So is one that did not answer at all: the rest of the queue
            // would meet the same wall. They are dropped from the worker,
            // not deferred, and later passes enqueue them once the lane
            // reopens at this entry's retry.
            let held = if applied.witness_busy_until.is_some() || applied.outage {
                shared.witness_worker.take_queued()
            } else {
                Vec::new()
            };
            let retry_at = if applied.outage {
                shared
                    .queue
                    .lock()
                    .expect("queue lock")
                    .get(entry.entry_id)
                    .filter(|e| e.state == QueueState::Approved)
                    .and_then(|e| e.retry_after)
            } else {
                None
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
            match (applied.witness_busy_until, retry_at) {
                (Some((until, _)), _) => JobEnd::Busy(until),
                (None, Some(until)) => JobEnd::Busy(until),
                (None, None) => JobEnd::Done,
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
