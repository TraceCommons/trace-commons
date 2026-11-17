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

/// The outside request to stop that the loop and the backoff both wait on.
/// Created once, so a request arriving between two runs of the loop is not
/// lost. For the CLI's `daemon run` it is SIGTERM / SIGINT (ctrl-C on
/// Windows); for an application embedding the daemon it is [`never`], since
/// that application owns its process's signals and stops the loop through
/// `shared.shutdown`.
pub(crate) type StopSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

/// No outside stop request: the loop ends only through `shared.shutdown`.
pub(crate) fn never() -> StopSignal {
    Box::pin(std::future::pending())
}

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

/// The wait before a pass that panicked runs again.
pub(crate) const PASS_FIRST_BACKOFF: Duration = Duration::from_secs(60);
/// The longest wait before a pass that keeps panicking runs again.
pub(crate) const PASS_MAX_BACKOFF: Duration = Duration::from_secs(60 * 60);

/// Fixed event label for one pass's panic, for the warn/error log.
pub(crate) const EVENT_PASS_PANICKED: &str = "daemon_pass_panicked";

/// The payload a pass guard unwinds with when a pass's panic poisoned
/// shared state; never a pass's own payload.
const POISONED_PASS_PAYLOAD: &str = "a pass panicked over shared state";

/// How one guarded pass went this tick.
#[derive(Debug)]
pub(crate) enum PassRun<T> {
    /// It ran to completion with this result.
    Ran(T),
    /// It panicked this tick; it is skipped until its backoff ends.
    Panicked,
    /// It is waiting out the backoff of an earlier panic and did not run.
    Skipped,
}

impl<T> PassRun<T> {
    /// Whether the pass ran to completion this tick.
    pub(crate) fn ran(&self) -> bool {
        matches!(self, PassRun::Ran(_))
    }
}

/// One pass's backoff state.
struct PassGuard {
    backoff: Backoff,
    skip_until: Option<tokio::time::Instant>,
}

/// A panic guard and backoff per periodic pass, so one pass that panics on
/// every run is skipped for a growing while instead of ending the tick, and
/// the passes after it still run. Owned by the pass loop: a whole-loop
/// restart starts every pass over from no backoff.
pub(crate) struct PassGuards {
    first: Duration,
    max: Duration,
    passes: std::collections::HashMap<&'static str, PassGuard>,
    #[cfg(test)]
    pub(crate) inject: std::collections::HashMap<&'static str, fn(&DaemonShared)>,
    #[cfg(test)]
    pub(crate) ran: Vec<&'static str>,
}

impl PassGuards {
    pub(crate) fn new(first: Duration, max: Duration) -> Self {
        Self {
            first,
            max,
            passes: std::collections::HashMap::new(),
            #[cfg(test)]
            inject: std::collections::HashMap::new(),
            #[cfg(test)]
            ran: Vec::new(),
        }
    }

    /// The daemon's schedule: 1 min doubling to 1 hour, reset by a run that
    /// completes.
    pub(crate) fn daemon() -> Self {
        Self::new(PASS_FIRST_BACKOFF, PASS_MAX_BACKOFF)
    }

    /// How long `pass` is still held back by an earlier panic, if it is.
    #[cfg(test)]
    pub(crate) fn held_for(&self, pass: &str) -> Option<Duration> {
        let until = self.passes.get(pass)?.skip_until?;
        until.checked_duration_since(tokio::time::Instant::now())
    }

    /// Run `pass` under its guard: not at all while an earlier panic's
    /// backoff lasts (`fut` is dropped unpolled), and with a panic caught
    /// and logged by label, which starts or doubles that backoff. A run
    /// that completes resets it.
    ///
    /// A panic that left one of `DaemonShared`'s data-bearing locks poisoned
    /// is not absorbed here: it unwinds on out of the tick, with a fixed
    /// payload, so the whole-loop rule in [`run_restarting`] stops the passes
    /// rather than letting later passes read half-updated state.
    pub(crate) async fn run<F: Future>(
        &mut self,
        shared: &DaemonShared,
        pass: &'static str,
        fut: F,
    ) -> PassRun<F::Output> {
        #[cfg(test)]
        if let Some(inject) = self.inject.get(pass).copied() {
            drop(fut);
            return self
                .guarded(shared, pass, async move {
                    inject(shared);
                    panic!("injected pass panic");
                })
                .await;
        }
        self.guarded(shared, pass, fut).await
    }

    async fn guarded<F: Future>(
        &mut self,
        shared: &DaemonShared,
        pass: &'static str,
        fut: F,
    ) -> PassRun<F::Output> {
        let (first, max) = (self.first, self.max);
        let guard = self.passes.entry(pass).or_insert_with(|| PassGuard {
            backoff: Backoff::new(first, max, Duration::MAX),
            skip_until: None,
        });
        if guard
            .skip_until
            .is_some_and(|until| tokio::time::Instant::now() < until)
        {
            return PassRun::Skipped;
        }
        // Unwind safety: what the pass leaves behind is `DaemonShared`; its
        // data-bearing locks are checked for poison below before anything
        // else runs.
        match crate::envelope::catch_unwind_future(fut).await {
            Ok(output) => {
                guard.backoff = Backoff::new(first, max, Duration::MAX);
                guard.skip_until = None;
                #[cfg(test)]
                self.ran.push(pass);
                PassRun::Ran(output)
            }
            Err(()) => {
                if shared.poisoned_state_lock().is_some() {
                    std::panic::resume_unwind(Box::new(POISONED_PASS_PAYLOAD));
                }
                let wait = guard.backoff.after_panic(Duration::ZERO);
                guard.skip_until = Some(tokio::time::Instant::now() + wait);
                tracing::warn!(pass = pass, "daemon pass panicked; skipping it for a while");
                diagnostics_log::record_labels(
                    tracing::Level::WARN,
                    EVENT_PASS_PANICKED,
                    &[
                        ("pass", pass),
                        ("retry_in_secs", &wait.as_secs().to_string()),
                    ],
                );
                PassRun::Panicked
            }
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

    fn tick_shared(dir: &std::path::Path) -> Arc<DaemonShared> {
        let shared = shared_in(dir);
        // Nothing in these ticks may ask the network.
        shared.settings.lock().unwrap().share_usage_data = false;
        shared
    }

    /// A watch pass that panics on every run does not hold back upload or
    /// the passes after it, in that tick or later ones, and its own backoff
    /// grows: skipped while it lasts, panicking again once it ends.
    #[tokio::test(start_paused = true)]
    async fn an_always_panicking_watch_pass_does_not_stop_upload() {
        let dir = tempfile::tempdir().unwrap();
        let shared = tick_shared(dir.path());
        let mut state = super::super::TickState::starting(chrono::Utc::now());
        state.guards.inject.insert("watch", |_| {});

        super::super::run_tick(&shared, false, &mut state).await;
        assert!(!state.guards.ran.contains(&"watch"));
        for later in ["upload", "expire", "history", "time-driven-status"] {
            assert!(state.guards.ran.contains(&later), "{later} ran this tick");
        }
        let first = state.guards.held_for("watch").expect("watch is held back");
        assert!(first <= PASS_FIRST_BACKOFF && first > PASS_FIRST_BACKOFF / 2);

        // The next tick, inside the backoff: watch skipped, upload runs.
        state.guards.ran.clear();
        super::super::run_tick(&shared, false, &mut state).await;
        assert!(state.guards.ran.contains(&"upload"));
        assert!(state.guards.held_for("watch").unwrap() <= first);

        // After the backoff watch runs, panics again, and waits twice as long.
        tokio::time::advance(PASS_FIRST_BACKOFF + Duration::from_secs(1)).await;
        state.guards.ran.clear();
        super::super::run_tick(&shared, false, &mut state).await;
        assert!(state.guards.ran.contains(&"upload"));
        let second = state.guards.held_for("watch").expect("held back again");
        assert!(second > PASS_FIRST_BACKOFF, "the backoff grew: {second:?}");
        assert!(second <= PASS_FIRST_BACKOFF * 2);
    }

    /// Fail closed: a tick whose account-admission or expiry pass panicked
    /// does not upload, since the gate would read a stale answer or an
    /// entry that should have aged out. The tick's other passes still run.
    #[tokio::test(start_paused = true)]
    async fn upload_is_withheld_in_a_tick_whose_gate_inputs_panicked() {
        for prerequisite in ["account-admission", "expire"] {
            let dir = tempfile::tempdir().unwrap();
            let shared = tick_shared(dir.path());
            let mut state = super::super::TickState::starting(chrono::Utc::now());
            state.guards.inject.insert(prerequisite, |_| {});
            super::super::run_tick(&shared, false, &mut state).await;
            assert!(
                !state.guards.ran.contains(&"upload"),
                "upload withheld after {prerequisite} panicked"
            );
            assert!(state.guards.ran.contains(&"history"));
            // Still withheld while that pass waits out its backoff.
            state.guards.ran.clear();
            super::super::run_tick(&shared, false, &mut state).await;
            assert!(!state.guards.ran.contains(&"upload"));
        }
    }

    /// Unchanged from the whole-loop rule: a pass whose panic poisons a data
    /// lock is not absorbed by its guard. It ends the tick, no later pass in
    /// it runs, and the passes stay stopped.
    #[tokio::test(start_paused = true)]
    async fn a_pass_that_poisons_shared_state_still_stops_the_passes() {
        let dir = tempfile::tempdir().unwrap();
        let shared = tick_shared(dir.path());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let uploads = Arc::new(AtomicUsize::new(0));
        let uploaded = Arc::clone(&uploads);
        let tick_shared = Arc::clone(&shared);
        let mut signal = never();
        let outcome = tokio::time::timeout(
            Duration::from_secs(3600),
            run_restarting(&shared, &mut signal, Backoff::daemon(), move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                let shared = Arc::clone(&tick_shared);
                let uploaded = Arc::clone(&uploaded);
                Box::pin(async move {
                    let mut state = super::super::TickState::starting(chrono::Utc::now());
                    state.guards.inject.insert("expire", |shared| {
                        let _queue = shared.queue.lock().unwrap();
                        panic!("panics holding the queue lock");
                    });
                    super::super::run_tick(&shared, false, &mut state).await;
                    if state.guards.ran.contains(&"upload") {
                        uploaded.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(())
                })
            }),
        )
        .await;
        assert!(outcome.is_err(), "parked, not returned");
        assert_eq!(shared.poisoned_state_lock(), Some("queue"));
        assert_eq!(runs.load(Ordering::SeqCst), 1, "never restarted");
        assert_eq!(uploads.load(Ordering::SeqCst), 0, "no upload after it");
    }
}
