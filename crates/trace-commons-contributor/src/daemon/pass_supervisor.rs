//! Keeps the daemon's periodic passes running when one of them panics.
//!
//! Every periodic pass (watch, expiry, upload, history, community, ...) runs
//! in one loop, `supervise_passes`. A panic anywhere in it used to end the
//! loop for the life of the process, while the socket server kept answering:
//! the app looked alive and nothing was watched or uploaded until a relaunch
//! (2026-11-04, a redaction panic since fixed in #1935). This module catches
//! such a panic, waits a bounded backoff, and starts the loop again.
//!
//! The catch is inline (a poll wrapper, not a spawned task), so the CLI's
//! `run` keeps its cancel-safety: dropping its future still drops the loop
//! before `daemon.lock` is released.
//!
//! Fail closed: the loop is restarted only when the panic left none of
//! `DaemonShared`'s data-bearing locks poisoned. A poisoned one (the queue,
//! say) may hold a half-applied change, so the loop is not restarted over it
//! and the passes stay stopped until the next start, which reloads that state
//! from disk. A restarted loop runs every gate from the top, exactly as the
//! first pass after a start does; nothing is resumed mid-pass.
//!
//! Logs are labels only. The panic hook has already written the location to
//! `daemon-panics.log`; the payload is never read.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;

use super::diagnostics_log;
use super::ipc::DaemonShared;

/// The stop signal (SIGTERM / SIGINT, or ctrl-C on Windows) the loop and the
/// backoff both wait on. Created once, so a signal arriving between two runs
/// of the loop is not lost.
pub(crate) type StopSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

/// One run of the periodic loop, borrowing the stop signal.
pub(crate) type PassesFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// The wait after the first panic.
pub(crate) const FIRST_BACKOFF: Duration = Duration::from_secs(5);
/// The longest wait between restarts.
pub(crate) const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// A loop that ran this long before panicking counts as healthy, and the
/// backoff starts over from [`FIRST_BACKOFF`].
pub(crate) const HEALTHY_RUN: Duration = Duration::from_secs(10 * 60);

/// Fixed event labels, for the warn/error log.
pub(crate) const EVENT_PANICKED: &str = "daemon_pass_task_panicked";
pub(crate) const EVENT_POISONED: &str = "daemon_pass_task_poisoned";

/// Bounded exponential backoff: doubles from `first` up to `max`, and starts
/// over after a run of at least `healthy`.
#[derive(Debug, Clone)]
pub(crate) struct Backoff {
    first: Duration,
    max: Duration,
    healthy: Duration,
    next: Duration,
}

impl Backoff {
    pub(crate) fn new(first: Duration, max: Duration, healthy: Duration) -> Self {
        Self {
            first,
            max,
            healthy,
            next: first,
        }
    }

    /// The daemon's schedule: 5 s doubling to 5 min, reset after 10 min.
    pub(crate) fn daemon() -> Self {
        Self::new(FIRST_BACKOFF, MAX_BACKOFF, HEALTHY_RUN)
    }

    /// How long to wait after a run that lasted `ran_for` and then panicked.
    pub(crate) fn after_panic(&mut self, ran_for: Duration) -> Duration {
        if ran_for >= self.healthy {
            self.next = self.first;
        }
        let wait = self.next;
        self.next = self.next.saturating_mul(2).min(self.max);
        wait
    }
}

/// Run the periodic loop `run_passes` builds, starting it again after a
/// panic (see the module doc). Returns what the loop returns when it ends
/// without panicking, `Ok(())` when a stop is asked for during a backoff, and
/// never restarts once `shared.shutdown` is set.
pub(crate) async fn run_restarting<F>(
    shared: &DaemonShared,
    signal: &mut StopSignal,
    mut backoff: Backoff,
    mut run_passes: F,
) -> Result<()>
where
    F: for<'a> FnMut(&'a mut StopSignal) -> PassesFuture<'a>,
{
    let mut restarts: u64 = 0;
    loop {
        let started = tokio::time::Instant::now();
        // Unwind safety: the loop's own locals die with the dropped future.
        // What outlives it is `DaemonShared`; its unit locks guard nothing,
        // and its data-bearing locks are checked for poison below before
        // anything runs again.
        let ended = crate::envelope::catch_unwind_future(run_passes(signal)).await;
        if let Ok(result) = ended {
            return result;
        }
        if shared.shutdown.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Some(lock) = shared.poisoned_state_lock() {
            tracing::error!(pass = "supervisor", "daemon pass task panicked over state");
            diagnostics_log::record_labels(
                tracing::Level::ERROR,
                EVENT_POISONED,
                &[("lock", lock)],
            );
            // Stay stopped, as before this module existed, but keep the
            // daemon up: the socket keeps answering and Private AI keeps
            // running. Returning would end both (`supervise`'s epilogue).
            wait_for_stop(shared, signal, None).await;
            return Ok(());
        }
        restarts = restarts.saturating_add(1);
        let wait = backoff.after_panic(started.elapsed());
        tracing::warn!(pass = "supervisor", "daemon pass task panicked; restarting");
        diagnostics_log::record_labels(
            tracing::Level::WARN,
            EVENT_PANICKED,
            &[
                ("restarts", &restarts.to_string()),
                ("retry_in_secs", &wait.as_secs().to_string()),
            ],
        );
        if wait_for_stop(shared, signal, Some(wait)).await {
            return Ok(());
        }
    }
}

/// Wait for a stop request or the stop signal, up to `limit` (forever when
/// `None`). True when a stop arrived.
async fn wait_for_stop(
    shared: &DaemonShared,
    signal: &mut StopSignal,
    limit: Option<Duration>,
) -> bool {
    let sleep = async {
        match limit {
            Some(limit) => tokio::time::sleep(limit).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(sleep);
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            return true;
        }
        tokio::select! {
            _ = shared.shutdown_signal.notified() => {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return true;
                }
            }
            _ = signal.as_mut() => {
                tracing::info!("daemon stopping on signal");
                // A signal sets nothing else; the loop's own exit is what
                // `supervise` turns into a shutdown.
                return true;
            }
            _ = &mut sleep => return shared.shutdown.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    fn shared_in(dir: &std::path::Path) -> Arc<DaemonShared> {
        let store = crate::config::ConfigStore::open(dir.join("state")).unwrap();
        Arc::new(DaemonShared::load(store).unwrap())
    }

    fn never() -> StopSignal {
        Box::pin(std::future::pending())
    }

    #[test]
    fn backoff_doubles_to_the_cap_and_resets_after_a_healthy_run() {
        let mut b = Backoff::daemon();
        let quick = Duration::from_secs(1);
        let waits: Vec<u64> = (0..9).map(|_| b.after_panic(quick).as_secs()).collect();
        assert_eq!(waits, vec![5, 10, 20, 40, 80, 160, 300, 300, 300]);
        assert_eq!(b.after_panic(HEALTHY_RUN).as_secs(), 5);
        assert_eq!(b.after_panic(quick).as_secs(), 10);
    }

    /// A loop that panics once is started again after the first backoff,
    /// and the next run goes ahead.
    #[tokio::test(start_paused = true)]
    async fn a_pass_that_panics_once_is_restarted_and_the_next_pass_runs() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_in(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let mut signal = never();
        let began = tokio::time::Instant::now();
        let result = run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
            let run = counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if run == 0 {
                    panic!("first pass panics");
                }
                Ok(())
            })
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 2, "the loop ran again");
        assert!(
            began.elapsed() >= FIRST_BACKOFF,
            "it waited the backoff first"
        );
    }

    /// The motivating case: the panic unwinds while the watcher holds
    /// `pass_lock`. The next run's real watcher pass takes the poisoned lock
    /// and goes ahead instead of panicking on the poison, which would have
    /// stopped every pass for good.
    #[tokio::test(start_paused = true)]
    async fn a_panic_under_the_pass_lock_does_not_stop_the_next_watch_pass() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_in(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let pass_shared = Arc::clone(&shared);
        // Undeclared Claude Code and Codex roots fall back to the real
        // `~/.claude` and `~/.codex`; declare every source off so the pass
        // reads nothing from this machine.
        {
            use crate::daemon::settings::SourceDeclaration;
            let mut settings = shared.settings.lock().unwrap();
            settings.claude_source = Some(SourceDeclaration::Off);
            settings.codex_source = Some(SourceDeclaration::Off);
            settings.gemini_source = Some(SourceDeclaration::Off);
            settings.cline_source = Some(SourceDeclaration::Off);
            settings.opencode_source = Some(SourceDeclaration::Off);
            settings.trajectory_source = Some(SourceDeclaration::Off);
        }
        let mut signal = never();
        let outcome = tokio::time::timeout(
            Duration::from_secs(600),
            run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
                let run = counted.fetch_add(1, Ordering::SeqCst);
                let shared = Arc::clone(&pass_shared);
                Box::pin(async move {
                    if run == 0 {
                        let _pass = shared.pass_lock.lock().unwrap();
                        panic!("panics holding the pass lock");
                    }
                    // Its result does not matter here, only that it returns.
                    let _ = crate::daemon::watcher::tick(&shared, chrono::Utc::now()).await;
                    Ok(())
                })
            }),
        )
        .await;
        assert!(shared.pass_lock.is_poisoned(), "the panic did poison it");
        assert!(
            matches!(outcome, Ok(Ok(()))),
            "the watch pass after the panic ran to completion"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert!(shared.queue.lock().unwrap().all().is_empty());
    }

    /// A loop that always panics is not spun: over ten virtual minutes it
    /// starts 7 times (at 0, 5, 15, 35, 75, 155 and 315 s), not thousands.
    #[tokio::test(start_paused = true)]
    async fn repeated_panics_back_off_and_do_not_spin() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_in(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let mut signal = never();
        let outcome = tokio::time::timeout(
            Duration::from_secs(600),
            run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { panic!("always panics") })
            }),
        )
        .await;
        assert!(outcome.is_err(), "still supervising at the deadline");
        assert_eq!(runs.load(Ordering::SeqCst), 7);
    }

    /// A stop asked for during the backoff ends supervision at once, without
    /// another run.
    #[tokio::test(start_paused = true)]
    async fn a_stop_during_the_backoff_ends_without_restarting() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_in(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let stopper = Arc::clone(&shared);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            stopper.shutdown.store(true, Ordering::Relaxed);
            stopper.shutdown_signal.notify_one();
        });
        let mut signal = never();
        let began = tokio::time::Instant::now();
        let result = run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { panic!("panics") })
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(began.elapsed() < FIRST_BACKOFF);
    }

    /// The stop signal during the backoff ends supervision too: under a
    /// service manager the signal handler is installed for good, so a
    /// backoff that ignored it would ignore SIGTERM.
    #[tokio::test(start_paused = true)]
    async fn the_stop_signal_during_the_backoff_ends_without_restarting() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_in(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let mut signal: StopSignal = Box::pin(tokio::time::sleep(Duration::from_secs(1)));
        let result = run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { panic!("panics") })
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    /// Fail closed: a panic that poisoned the queue lock may have left the
    /// queue half-updated, so the loop is not started over it.
    #[tokio::test(start_paused = true)]
    async fn a_panic_that_poisons_queue_state_stops_the_passes() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_in(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let pass_shared = Arc::clone(&shared);
        let mut signal = never();
        let outcome = tokio::time::timeout(
            Duration::from_secs(3600),
            run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                let shared = Arc::clone(&pass_shared);
                Box::pin(async move {
                    let _queue = shared.queue.lock().unwrap();
                    panic!("panics holding the queue lock");
                })
            }),
        )
        .await;
        assert!(outcome.is_err(), "parked until a stop, not returned");
        assert_eq!(shared.poisoned_state_lock(), Some("queue"));
        assert_eq!(runs.load(Ordering::SeqCst), 1, "never restarted");
        assert!(
            !shared.shutdown.load(Ordering::Relaxed),
            "the daemon itself stays up"
        );
    }

    /// The socket keeps answering while the loop is backing off after a
    /// panic: `status` over the real transport gets a result.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_socket_keeps_serving_while_the_passes_back_off() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().join("state")).unwrap();
        let embedded = crate::daemon::start_embedded(store).await.unwrap();
        let shared = Arc::clone(&embedded.shared);
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let supervised = Arc::clone(&shared);
        let supervisor = tokio::spawn(async move {
            let mut signal = never();
            let backoff = Backoff::new(
                Duration::from_millis(50),
                Duration::from_millis(200),
                HEALTHY_RUN,
            );
            run_restarting(&supervised, &mut signal, backoff, move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { panic!("panics") })
            })
            .await
        });
        // Let it panic and restart a few times first.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while runs.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(runs.load(Ordering::SeqCst) >= 3, "the loop was restarted");
        let call_store = shared.store.clone();
        let response = tokio::task::spawn_blocking(move || {
            crate::daemon::client::try_call(&call_store, "status", &serde_json::json!({}))
        })
        .await
        .unwrap()
        .unwrap()
        .expect("the socket answered");
        assert!(response.error.is_none(), "status returned an error frame");
        assert!(response.result.is_some());
        shared.shutdown.store(true, Ordering::Relaxed);
        shared.shutdown_signal.notify_one();
        let ended = tokio::time::timeout(Duration::from_secs(10), supervisor).await;
        assert!(matches!(ended, Ok(Ok(Ok(())))));
        embedded.close();
    }
}
