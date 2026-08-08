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

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = Utc::now();
                if let Err(e) = watcher::tick(&shared, now).await {
                    tracing::warn!(error = %e, "watch tick failed");
                }
                expire_and_digest(&shared, now);
                if dry_run {
                    // Everything above is read-only bookkeeping; uploading is
                    // what dry-run withholds.
                    continue;
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
