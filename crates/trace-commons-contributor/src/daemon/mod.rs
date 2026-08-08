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

pub mod eligibility;
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
pub async fn run(store: ConfigStore, dry_run: bool) -> Result<()> {
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

    let result = supervise(Arc::clone(&shared), dry_run).await;

    server.abort();
    let _ = shared.store.remove_daemon_file(DAEMON_SOCK_FILE);
    drop(lock);
    let _ = std::fs::remove_file(&lock_path);
    result
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
    let approved: Vec<queue::QueueEntry> = {
        let q = shared.queue.lock().expect("queue lock");
        q.all()
            .iter()
            .filter(|e| e.state == queue::QueueState::Approved)
            .cloned()
            .collect()
    };
    if approved.is_empty() {
        // Re-check enrollment and retract upload-failure labels when the queue
        // is empty, since nothing is failing to upload anymore.
        if uploader::enrollment_is_live(&shared.store) {
            let mut health = shared.health.lock().expect("health lock");
            health.resolve(health::LABEL_NOT_LOGGED_IN);
        }
        // An empty approved queue means there is nothing failing to upload,
        // so stale upload-failure labels no longer describe reality.
        {
            let mut health = shared.health.lock().expect("health lock");
            health.resolve(health::LABEL_CLAIM_MINT_FAILED);
            health.resolve(health::LABEL_INGEST_UNREACHABLE);
        }
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
    let store = crate::config::ConfigStore::open(shared.store.dir().to_path_buf())?;
    let mut ctx = crate::submit::SubmitContext::new(&store, &cfg, &opts, near_ai)?;

    let sources = crate::source::all_sources(claude_root, codex_root, None);
    let mut changed = false;

    for entry in approved {
        // Re-resolve the session through its own adapter, so the uploader can
        // re-read and re-hash the file before sending anything.
        let Some((source, session_ref)) = find_session(&sources, &entry) else {
            let mut q = shared.queue.lock().expect("queue lock");
            q.set_state(
                entry.entry_id,
                queue::QueueState::Failed,
                Some("session-file-vanished".to_string()),
            );
            changed = true;
            continue;
        };

        let decision = {
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
            let decision = up.upload_entry(source, &session_ref, &entry, now).await?;
            *shared.state.lock().expect("state lock") = state;
            *shared.health.lock().expect("health lock") = health;
            decision
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

    if changed {
        let q = shared.queue.lock().expect("queue lock");
        q.save(&shared.store)?;
        drop(q);
        let state = shared.state.lock().expect("state lock");
        state.save(&shared.store)?;
        drop(state);
        shared.publish(ipc::EVENT_QUEUE_CHANGED, serde_json::json!({}));
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
    let receipts = shared.store.load_receipts()?;
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
