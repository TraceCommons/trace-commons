//! The background upload daemon.
//!
//! The daemon watches the local coding-agent session roots, decides which
//! sessions are finished and uploadable, tells the contributor about them,
//! uploads the ones they approve, and auto-uploads the projects they have
//! explicitly opted in. It serves a versioned IPC contract so native tray and
//! window applications can drive all of that without reimplementing any of it.
//!
//! Every upload takes the same path an interactive `submit` takes, via
//! `submit::SubmitContext`. There is no second pipeline.
//!
//! Privacy posture, which the rest of this module tree is built to preserve:
//!
//! - A local filesystem path appears only in `daemon-queue.jsonl` and
//!   `daemon-state.json`. It never reaches a receipt, a history record, a log
//!   line, or the wire. Consumers get `project_label`.
//! - Nothing is uploaded from a project the contributor has not opted in, and
//!   sessions whose working directory cannot be resolved can never be opted in
//!   at all.
//! - A configured privacy filter that is unavailable stops the pipeline. It
//!   never degrades to sending unfiltered text.

pub mod audit;
pub mod client;
pub mod eligibility;
pub mod enroll;
pub mod health;
pub mod history;
pub mod install;
pub mod ipc;
pub mod notify;
pub mod policy;
pub mod preview;
pub mod queue;
pub mod settings;
pub mod state;
pub mod uploader;
pub mod watcher;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::config::{ConfigStore, DAEMON_LOCK_FILE, DAEMON_SOCK_FILE};

/// Run the daemon in the foreground. A service manager, or the contributor's
/// own terminal, is what puts it in the background.
///
/// Holds an exclusive lock for its whole life, so a second daemon against the
/// same state directory fails loudly instead of two of them racing over the
/// same queue.
///
/// `run_supervisor` is awaited *inline* here (not spawned and then joined)
/// on purpose: if this async fn's own future is itself dropped before
/// completion -- the ordinary way a caller races the daemon against, say, a
/// ctrl-C future in a `tokio::select!` -- Rust's structured-concurrency
/// cancellation stops the supervise loop's execution immediately, as part of
/// dropping this same stack frame, before `lock` (the file backing
/// `daemon.lock`) is also dropped and the lock released. An earlier version
/// of this function spawned the supervise loop as an independent task and
/// awaited its `JoinHandle` instead; `JoinHandle::drop` does not abort the
/// task, so dropping `run`'s future left the supervisor detached and still
/// mutating the queue after the lock had already been released to a second
/// daemon -- exactly the corruption `daemon.lock` exists to prevent. See
/// `EmbeddedDaemon`'s and `run_supervisor`'s docs for the embedding case
/// that still needs the loop to run as a background task.
pub async fn run(store: ConfigStore, dry_run: bool) -> Result<()> {
    let embedded = start_embedded(store).await?;
    let result = run_supervisor(Arc::clone(&embedded.shared), dry_run).await;
    embedded.close();
    result
}

/// The pieces of a running daemon that a caller needing direct, in-process
/// access to `shared` -- rather than only running the loop to completion the
/// way `run` does -- holds onto.
///
/// This is what `trace-commons-contributor-ffi` embeds: the C ABI's
/// `tc_daemon_start` calls `start_embedded` instead of `run` so it gets back
/// the same `Arc<DaemonShared>` the loop is mutating, for `tc_call` and
/// `tc_preview_open` to act on directly via `ipc::handle_local` /
/// `ipc::open_preview` -- not a second, independently-loaded, and therefore
/// divergent, view of the on-disk state.
///
/// Deliberately does **not** itself run the supervise loop (the periodic
/// watch/upload/digest/history pass): `run` awaits `run_supervisor` inline
/// for cancel-safety (see its doc). An embedder that needs the loop running
/// in the background, independent of any one call's lifetime, spawns
/// `run_supervisor` itself and keeps the resulting `JoinHandle` for its own
/// explicit shutdown.
pub struct EmbeddedDaemon {
    pub shared: Arc<ipc::DaemonShared>,
    lock_path: std::path::PathBuf,
    lock: std::fs::File,
    server: tokio::task::JoinHandle<Result<()>>,
}

impl EmbeddedDaemon {
    /// Stop serving the socket and release the exclusive lock. Does *not*
    /// touch `shared.shutdown` or stop anything running the supervise loop
    /// -- `EmbeddedDaemon` no longer owns that task (see the struct doc), so
    /// a caller that spawned `run_supervisor` itself is responsible for
    /// signalling and awaiting it before (or after; order does not matter
    /// for correctness, only for how long the loop keeps working past the
    /// request) calling this.
    pub fn close(self) {
        self.server.abort();
        let _ = self.shared.store.remove_daemon_file(DAEMON_SOCK_FILE);
        drop(self.lock);
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Take the daemon's exclusive lock, build the shared state, and bind and
/// spawn the socket server -- everything `run` does before it starts the
/// supervise loop, returned as pieces instead of run to completion.
///
/// Locking happens exactly once per call, the same as `run` used to: this
/// function (not `run`) is now the one place that takes `daemon.lock`, so a
/// second `start_embedded` -- or a second `run` -- against the same state
/// directory still fails loudly on the `try_lock`, whether or not the caller
/// is the same process. The failure is a plain `anyhow::Error` built with
/// `bail!`, not a structured variant, so a caller that needs to distinguish
/// "lock held by another daemon" from other failures (a state-directory
/// permissions problem, a socket bind failure) has to match on the message
/// text; see `tests::a_second_start_embedded_fails_specifically_on_the_lock`
/// for the exact text this asserts.
pub async fn start_embedded(store: ConfigStore) -> Result<EmbeddedDaemon> {
    let lock_path = store.daemon_path(DAEMON_LOCK_FILE);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    if lock.try_lock().is_err() {
        bail!(
            "another trace-commons-contributor daemon is already running for \
             this state directory"
        );
    }

    let shared = Arc::new(ipc::DaemonShared::load(store)?);
    let listener = ipc::bind(&shared.store).await?;

    let serve_shared = Arc::clone(&shared);
    let server = tokio::spawn(async move { ipc::serve(listener, serve_shared).await });

    Ok(EmbeddedDaemon {
        shared,
        lock_path,
        lock,
        server,
    })
}

/// Run the periodic watch/upload/digest/history pass to completion -- i.e.,
/// until `shared`'s shutdown flag or signal fires. This is `supervise`,
/// exposed under a stable name so a caller holding only an
/// `Arc<DaemonShared>` (`supervise` itself is private) can run it -- either
/// awaited inline, as `run` does for cancel-safety, or `tokio::spawn`ed as
/// its own background task by an embedder that needs the loop running
/// independent of any one call's lifetime (see `EmbeddedDaemon`'s doc).
pub async fn run_supervisor(shared: Arc<ipc::DaemonShared>, dry_run: bool) -> Result<()> {
    supervise(shared, dry_run).await
}

/// Run `f` -- blocking, non-yielding work with no `.await` of its own
/// (filesystem scanning, hashing, reading a receipts file) -- off whichever
/// worker is currently executing this task, via `tokio::task::
/// block_in_place`, when the current runtime is multi-thread. That is the
/// only flavor `block_in_place` supports (it panics under
/// `current_thread`, the default `#[tokio::test]` flavor most of this
/// crate's async tests use, so `f` just runs inline there instead -- the
/// same as it always did) and the only one where running `f` off-worker
/// actually matters: on a `current_thread` runtime there is only ever one
/// worker regardless.
///
/// Without this, blocking work called from inside an async task can
/// monopolize a runtime's sole worker thread for its entire duration,
/// starving every other task -- the socket server, `tc_subscribe`
/// delivery, even a reentrant `tc_daemon_stop`'s own wait on the
/// supervisor's `JoinHandle`. First found in `watcher::tick`'s session-root
/// scan; `drain_approved`'s `find_session` re-scan and `refresh_history`'s
/// receipts read go through this too, for the same reason -- see each call
/// site.
pub(crate) fn run_blocking<R>(f: impl FnOnce() -> R) -> R {
    let multi_thread = tokio::runtime::Handle::try_current()
        .map(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
        .unwrap_or(false);
    if multi_thread {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

/// The periodic work: watch, expire, and decide about digests, until asked to
/// stop.
async fn supervise(shared: Arc<ipc::DaemonShared>, dry_run: bool) -> Result<()> {
    let poll_interval = {
        let s = shared.settings.lock().expect("settings lock");
        std::time::Duration::from_secs(s.poll_interval_secs.max(1))
    };
    let mut ticker = tokio::time::interval(poll_interval);
    let mut sigterm = signal_stream();
    let shutdown_signal = Arc::clone(&shared.shutdown_signal);

    loop {
        // Checked at the top as well as after the select, so a request that
        // arrived while the previous pass was still working is acted on
        // immediately rather than waiting out another poll interval.
        if shared.shutdown.load(Ordering::Relaxed) {
            tracing::info!("daemon stopping on request");
            return Ok(());
        }
        tokio::select! {
            _ = shutdown_signal.notified() => {
                tracing::info!("daemon stopping on request");
                return Ok(());
            }
            _ = ticker.tick() => {
                let now = Utc::now();
                if let Err(e) = watcher::tick(&shared, now).await {
                    tracing::warn!(error = %e, "watch tick failed");
                }
                expire_and_digest(&shared, now);
                // Everything above is read-only bookkeeping; uploading is
                // what dry-run withholds.
                if !dry_run {
                    if let Err(e) = drain_approved(&shared, now).await {
                        tracing::warn!(error = %e, "upload pass failed");
                    }
                    if let Err(e) = refresh_history(&shared, now).await {
                        tracing::warn!(error = %e, "history refresh failed");
                    }
                }
            }
            _ = &mut sigterm => {
                tracing::info!("daemon stopping on signal");
                return Ok(());
            }
        }
        if shared.shutdown.load(Ordering::Relaxed) {
            tracing::info!("daemon stopping on request");
            return Ok(());
        }
    }
}

/// Upload everything that has been approved, whether by the contributor or by
/// their standing opt-in for the project.
///
/// One `SubmitContext` covers the whole pass, so the claim is minted once and
/// the privacy-filter canary runs once, exactly as an interactive `submit`
/// batch does.
async fn drain_approved(shared: &Arc<ipc::DaemonShared>, now: chrono::DateTime<Utc>) -> Result<()> {
    // Pause used to be checked only inside `watcher::tick`, so a pause
    // stopped *discovery* and nothing else: everything already `Approved`
    // -- including everything an armed project had auto-approved before the
    // pause -- kept uploading while `status` said paused. Pause has to mean
    // "nothing leaves this machine", or it means nothing.
    if shared.is_paused(now) {
        return Ok(());
    }

    let approved: Vec<queue::QueueEntry> = {
        let q = shared.queue.lock().expect("queue lock");
        q.all()
            .iter()
            .filter(|e| e.state == queue::QueueState::Approved)
            .cloned()
            .collect()
    };
    if approved.is_empty() {
        // Re-check enrollment when the queue is empty, so a stale not-logged-in
        // condition gets retracted if the contributor has logged back in.
        // This is sound: enrollment_is_live genuinely re-checks the condition.
        if uploader::enrollment_is_live(&shared.store) {
            let mut health = shared.health.lock().expect("health lock");
            health.resolve(health::LABEL_NOT_LOGGED_IN);
        }
        // Do NOT retract LABEL_CLAIM_MINT_FAILED or LABEL_INGEST_UNREACHABLE here.
        // The approved queue empties because upload entries move to Failed state when
        // uploads fail. Retracting those labels with no evidence would be dishonest:
        // ingest could still be down, and the HealthState.since field already tells
        // the consumer how old the information is. A label saying "last attempt failed
        // 3 hours ago" is accurate; one that silently says "now healthy" is not.
        return Ok(());
    }

    let Some(cfg) = shared.store.load_config()? else {
        let mut health = shared.health.lock().expect("health lock");
        health.fail(health::LABEL_NOT_LOGGED_IN, now);
        return Ok(());
    };
    let (near_ai, claude_root, codex_root) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.near_ai.clone(),
            s.claude_root.clone(),
            s.codex_root.clone(),
        )
    };
    let opts = crate::submit::SubmitOptions {
        dry_run: false,
        pii_filter: cfg.pii_filter.clone(),
        no_reasoning: false,
        machine_readable: true,
        unenrolled_preview: false,
        remediate_quarantined: false,
    };
    // Both of these hit the filesystem synchronously with no `.await` of
    // their own -- `ConfigStore::open` creates/permissions the state dir,
    // `SubmitContext::new` reads the config and the receipts file -- so
    // they go off-worker for the same reason `watcher::tick`'s scan does.
    // See `run_blocking`'s doc.
    let store =
        run_blocking(|| crate::config::ConfigStore::open(shared.store.dir().to_path_buf()))?;
    let mut ctx = run_blocking(|| crate::submit::SubmitContext::new(&store, &cfg, &opts, near_ai))?;

    let sources = crate::source::all_sources(claude_root, codex_root, None);
    let mut changed = false;
    // A fail-closed precondition (`SubmitPreconditionFailure`) aborts the
    // pass. It is held here rather than propagated with `?` so the pass's
    // own mutations -- the entries already resolved, the health label the
    // uploader just set -- are still persisted before it surfaces. The old
    // `?` threw all of that away, including the very label that suspends
    // expiry.
    let mut aborted: Option<anyhow::Error> = None;

    for entry in approved {
        // Claim the entry, atomically, before anything is read or sent. A
        // `cancel` that landed between the snapshot above and here wins and
        // the entry is skipped; from this point `cancel` is refused,
        // because the upload really is in flight. See
        // `Queue::claim_for_upload`.
        {
            let mut q = shared.queue.lock().expect("queue lock");
            let Some(current) = q.get(entry.entry_id).cloned() else {
                continue;
            };
            if current.state != queue::QueueState::Approved {
                continue;
            }
            // The approval covers the scopes that were in force when it was
            // given. `set_consent_scopes` can widen them at any moment with
            // nothing coupling it to already-approved entries, so an entry
            // whose scopes have moved is put back in front of the
            // contributor rather than sent under terms they never saw --
            // the same rule the re-hash guard applies to content.
            if current.approved_scopes.as_deref() != Some(cfg.consent_scopes.as_slice()) {
                q.revoke_approval(entry.entry_id, "consent-scopes-changed-after-approval");
                changed = true;
                continue;
            }
            if !q.claim_for_upload(entry.entry_id) {
                continue;
            }
        }

        // Re-resolve the session through its own adapter, so the uploader can
        // re-read and re-hash the file before sending anything.
        // `find_session` re-scans every source (`source.discover()`), the
        // same blocking, non-yielding pass `watcher::tick` runs -- see
        // `run_blocking`'s doc.
        let Some((source, session_ref)) = run_blocking(|| find_session(&sources, &entry)) else {
            let mut q = shared.queue.lock().expect("queue lock");
            q.set_state(
                entry.entry_id,
                queue::QueueState::Failed,
                Some("session-file-vanished".to_string()),
            );
            changed = true;
            continue;
        };

        let result = {
            let mut state = shared.state.lock().expect("state lock").clone();
            let settings = shared.settings.lock().expect("settings lock").clone();
            let mut health = shared.health.lock().expect("health lock").clone();
            let mut up = uploader::Uploader {
                ctx: &mut ctx,
                store: &store,
                settings: &settings,
                state: &mut state,
                health: &mut health,
            };
            let result = up.upload_entry(source, &session_ref, &entry, now).await;
            // Copied back on the failure path too: the uploader sets the
            // fail-closed health label (canary, notice, identity) right
            // before it returns `Err`, and that label is what suspends
            // queue expiry.
            *shared.state.lock().expect("state lock") = state;
            *shared.health.lock().expect("health lock") = health;
            result
        };
        let decision = match result {
            Ok(d) => d,
            Err(e) => {
                aborted = Some(e);
                break;
            }
        };

        let mut q = shared.queue.lock().expect("queue lock");
        match decision {
            uploader::UploadDecision::Uploaded { submission_id }
            | uploader::UploadDecision::AlreadySubmitted { submission_id } => {
                q.set_state(entry.entry_id, queue::QueueState::Uploaded, None);
                q.set_submission_id(entry.entry_id, submission_id);
            }
            uploader::UploadDecision::Superseded { new_hash } => {
                let size = std::fs::metadata(&entry.path).map(|m| m.len()).unwrap_or(0);
                if let Some(fresh) = q.supersede(entry.entry_id, &new_hash, size, now) {
                    let max = shared
                        .settings
                        .lock()
                        .expect("settings lock")
                        .max_queue_entries;
                    let _ = q.upsert(fresh, max);
                }
            }
            uploader::UploadDecision::Refused { reason_label } => {
                q.set_state(
                    entry.entry_id,
                    queue::QueueState::Refused,
                    Some(reason_label),
                );
            }
            uploader::UploadDecision::Failed { reason_label } => {
                q.record_attempt(entry.entry_id, None);
                q.set_state(
                    entry.entry_id,
                    queue::QueueState::Failed,
                    Some(reason_label),
                );
            }
            uploader::UploadDecision::CapReached => {
                // Leave it approved: the cap lifts when the day rolls over.
                break;
            }
        }
        changed = true;
    }

    // Nothing may be left claimed. Every entry that reached a decision above
    // already has a terminal state; anything still `Uploading` is one this
    // pass broke out on (a daily cap, a fail-closed precondition), and
    // `Uploading` is a state nothing else would ever move it out of.
    {
        let mut q = shared.queue.lock().expect("queue lock");
        if q.release_in_flight() {
            changed = true;
        }
    }

    if changed {
        let q = shared.queue.lock().expect("queue lock");
        q.save(&shared.store)?;
        drop(q);
        let state = shared.state.lock().expect("state lock");
        state.save(&shared.store)?;
        drop(state);
        shared.publish(ipc::EVENT_QUEUE_CHANGED, serde_json::json!({}));
    }
    if let Some(e) = aborted {
        return Err(e);
    }
    Ok(())
}

/// One upload pass, exposed so an integration test can drive the same code
/// the supervisor runs rather than a reimplementation of it.
pub async fn drain_approved_for_test(
    shared: &Arc<ipc::DaemonShared>,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    drain_approved(shared, now).await
}

/// Find the adapter and session reference matching a queue entry's path.
fn find_session<'a>(
    sources: &'a [Box<dyn crate::source::TraceSource>],
    entry: &queue::QueueEntry,
) -> Option<(
    &'a dyn crate::source::TraceSource,
    crate::source::SessionRef,
)> {
    for source in sources {
        let Ok(refs) = source.discover() else {
            continue;
        };
        if let Some(r) = refs.into_iter().find(|r| r.path == entry.path) {
            return Some((source.as_ref(), r));
        }
    }
    None
}

/// Refresh the cached contribution history from the server, on its own
/// interval so history stays readable without every application polling.
async fn refresh_history(
    shared: &Arc<ipc::DaemonShared>,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let interval = {
        let s = shared.settings.lock().expect("settings lock");
        chrono::Duration::seconds(s.history_poll_secs as i64)
    };
    {
        let state = shared.state.lock().expect("state lock");
        if let Some(last) = state.last_history_poll_at {
            if now.signed_duration_since(last) < interval {
                return Ok(());
            }
        }
    }
    let Some(cfg) = shared.store.load_config()? else {
        return Ok(());
    };
    let updates = match crate::submit::status(&shared.store, &cfg).await {
        Ok(u) => u,
        Err(_) => {
            // A failed poll serves the cache as-is; history is not worth a
            // health failure of its own.
            return Ok(());
        }
    };
    // A blocking file read with no `.await` of its own; see `run_blocking`'s
    // doc.
    let receipts = run_blocking(|| shared.store.load_receipts())?;
    let labels = {
        let q = shared.queue.lock().expect("queue lock");
        let mut m = std::collections::BTreeMap::new();
        for e in q.all() {
            if let Some(id) = e.submission_id {
                m.insert(id, e.project_label.clone());
            }
        }
        m
    };
    let records = history::join(&receipts, &updates, &labels, now);
    history::HistoryCache::save(&shared.store, &records)?;
    let mut state = shared.state.lock().expect("state lock");
    state.last_history_poll_at = Some(now);
    state.save(&shared.store)?;
    Ok(())
}

/// Age out undecided entries, then decide whether a digest is due.
fn expire_and_digest(shared: &Arc<ipc::DaemonShared>, now: chrono::DateTime<Utc>) {
    let (ttl_days, digest_interval_secs, local_notifications) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.queue_ttl_days,
            s.digest_interval_secs,
            s.local_notifications,
        )
    };
    let blocked = shared.health.lock().expect("health lock").blocks_expiry();

    let (expired, pending_count, digest) = {
        let mut queue = shared.queue.lock().expect("queue lock");
        let expired = queue.expire(now, ttl_days, blocked);
        let pending = queue.pending();
        let count = pending.len();
        let text = notify::digest_text(&pending);
        (expired, count, text)
    };
    if expired > 0 {
        let queue = shared.queue.lock().expect("queue lock");
        let _ = queue.save(&shared.store);
    }

    let last_digest_at = shared.state.lock().expect("state lock").last_digest_at;
    if notify::digest_due(last_digest_at, now, digest_interval_secs, pending_count) {
        shared.publish(
            ipc::EVENT_DIGEST_DUE,
            serde_json::json!({ "pending": pending_count, "text": digest }),
        );
        if local_notifications {
            notify::emit_local(&digest);
        }
        let mut state = shared.state.lock().expect("state lock");
        state.last_digest_at = Some(now);
        let _ = state.save(&shared.store);
    }
}

/// A future that resolves when the process is asked to terminate.
fn signal_stream() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut term = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => return std::future::pending().await,
            };
            let mut interrupt = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(_) => return std::future::pending().await,
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = interrupt.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> chrono::DateTime<Utc> {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn empty_approved_queue_does_not_retract_ingest_unreachable() {
        // When the approved queue is empty, do NOT retract ingest-unreachable.
        // The queue emptied because upload entries moved to Failed state when
        // uploads failed. With no evidence that ingest recovered, retracting
        // the label would be dishonest.
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().join("state")).unwrap();
        let shared = Arc::new(ipc::DaemonShared::load(store).unwrap());

        // Set ingest-unreachable manually
        {
            let mut health = shared.health.lock().expect("health lock");
            health.fail(health::LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:00:00Z"));
        }
        assert!(!{ shared.health.lock().expect("health lock").ok() });

        // Call drain_approved with empty approved queue
        drain_approved_for_test(&shared, at("2026-08-08T13:00:00Z"))
            .await
            .unwrap();

        // ingest-unreachable should SURVIVE because no recovery was proven
        assert!(
            !{ shared.health.lock().expect("health lock").ok() },
            "ingest-unreachable must persist when queue is empty"
        );
        let label = {
            shared
                .health
                .lock()
                .expect("health lock")
                .last_error_label
                .clone()
        };
        assert_eq!(label.as_deref(), Some(health::LABEL_INGEST_UNREACHABLE));
    }

    /// `trace-commons-contributor-ffi`'s own lock-contention test
    /// (`a_second_start_against_the_same_directory_fails_on_the_lock`)
    /// only asserts that `tc_daemon_start` returns NULL with a non-null
    /// `*err` -- which `tc_daemon_start` would also do if the *second*
    /// start failed for an unrelated reason (a socket-bind failure, a
    /// `ConfigStore::open` permissions error), since it collapses every
    /// failure into one fixed label before crossing the FFI boundary (see
    /// that crate's module doc on why). This test, against
    /// `start_embedded` directly rather than through the FFI, is the one
    /// that actually proves the second failure is the lock, not something
    /// else: it asserts on `anyhow::Error`'s `Display` text.
    #[tokio::test]
    async fn a_second_start_embedded_fails_specifically_on_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let embedded = start_embedded(store_a).await.unwrap();

        let store_b = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let err = match start_embedded(store_b).await {
            Ok(_) => panic!("a second start_embedded against a locked directory must fail"),
            Err(e) => e,
        };
        assert!(
            format!("{err:#}").contains("already running"),
            "expected a lock-contention message, got: {err:#}"
        );

        embedded.close();
    }
}
