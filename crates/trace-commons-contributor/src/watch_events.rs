//! Filesystem event source for the daemon's session roots.
//!
//! The poll loop learns that something moved by walking the whole corpus.
//! This module is the other source of truth for the same fact: the operating
//! system already knows when a session file was written, and will say so.
//!
//! Three things are deliberate here.
//!
//! **Watches are registered only on declared roots.** The roots arrive as an
//! explicit argument. Nothing in this module discovers a root, falls back to
//! a conventional location, or widens a watch beyond what it was handed. A
//! root the contributor never declared is a root no watch exists for, so it
//! cannot produce an event.
//!
//! **Failure is typed, never silent.** Recursive inotify consumes one watch
//! per directory and `fs.inotify.max_user_watches` commonly defaults to
//! 8192, so a large corpus can exhaust it; a root can also be unmounted,
//! deleted, or have its permissions revoked while the daemon runs. Both
//! surface as a [`WatchFailure`] the caller can raise a health label for and
//! fall back to fast polling on. Watching less than the declared roots
//! without saying so is the one outcome that is not acceptable, because it
//! is indistinguishable from "nothing is happening".
//!
//! **The source is injectable.** The daemon holds a [`SessionEventSource`]
//! trait object, never `notify` itself. CI runs on hosted runners where real
//! filesystem event delivery varies per platform and per filesystem, so
//! tests drive [`ScriptedEventSource`] by hand instead of waiting on the
//! kernel.
//!
//! Nothing here logs a path. `WatchFailure` carries the offending root so a
//! caller can decide what to do about it; the health vocabulary it maps to is
//! a fixed label, per this crate's hash-only/label-only rule.

use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

/// Watch registration failed for a declared root: the platform refused to
/// register, most commonly Linux inotify watch exhaustion. The caller falls
/// back to polling at the fast interval.
pub const LABEL_WATCH_REGISTRATION_FAILED: &str = "watch-registration-failed";
/// A declared root is not there to watch -- missing, unmounted, or no longer
/// readable.
pub const LABEL_WATCH_ROOT_UNAVAILABLE: &str = "watch-root-unavailable";

/// How long a path must be quiet before it is emitted.
///
/// Distinct from, and much shorter than, `quiescence_secs`: this only
/// coalesces the many events one logical write produces, and says nothing
/// about whether a session is finished.
pub const DEFAULT_DEBOUNCE_MILLIS: u64 = 2_000;

/// Why a declared root is not being watched.
///
/// Both variants mean the same thing operationally -- the event path is
/// covering less than it was asked to -- and differ only in what the caller
/// should tell the contributor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchFailure {
    /// The platform refused to register a watch. On Linux this is usually
    /// `fs.inotify.max_user_watches` exhaustion.
    RegistrationFailed { root: PathBuf },
    /// The root was not a readable directory at registration time, or stopped
    /// being one while watched.
    RootUnavailable { root: PathBuf },
}

impl WatchFailure {
    /// The fixed health label for this condition. The path never travels with
    /// it.
    pub fn health_label(&self) -> &'static str {
        match self {
            WatchFailure::RegistrationFailed { .. } => LABEL_WATCH_REGISTRATION_FAILED,
            WatchFailure::RootUnavailable { .. } => LABEL_WATCH_ROOT_UNAVAILABLE,
        }
    }

    /// The root this failure is about.
    pub fn root(&self) -> &Path {
        match self {
            WatchFailure::RegistrationFailed { root } | WatchFailure::RootUnavailable { root } => {
                root.as_path()
            }
        }
    }
}

/// What a watch session delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Paths that changed and have since been quiet for the debounce window.
    /// Each path appears once per batch.
    Changed(Vec<PathBuf>),
    /// A root stopped being watched after the session started. The caller
    /// should raise `failure.health_label()` and fall back to fast polling.
    Failed(WatchFailure),
}

/// A live watch over a set of declared roots.
pub struct WatchSession {
    /// Debounced batches, plus any failure that develops later.
    pub events: mpsc::Receiver<WatchEvent>,
    /// The roots a watch was actually registered on.
    pub watched: Vec<PathBuf>,
    /// Roots that could not be watched at registration time.
    pub failures: Vec<WatchFailure>,
    /// Keeps the backend alive for as long as the session is held. Dropping
    /// the session deregisters the watches.
    _backend: Box<dyn Any + Send>,
}

impl WatchSession {
    /// Build a session. `backend` is whatever the implementation must keep
    /// alive -- a `notify` watcher, a task guard, or `()`.
    pub fn new(
        events: mpsc::Receiver<WatchEvent>,
        watched: Vec<PathBuf>,
        failures: Vec<WatchFailure>,
        backend: Box<dyn Any + Send>,
    ) -> Self {
        Self {
            events,
            watched,
            failures,
            _backend: backend,
        }
    }

    /// True when at least one declared root is not covered. The caller polls
    /// at the fast interval while this holds.
    pub fn degraded(&self) -> bool {
        !self.failures.is_empty()
    }
}

/// Something that reports changed paths under a set of declared roots.
pub trait SessionEventSource: Send + Sync {
    /// Start watching exactly `roots` -- no more, and no discovery of its
    /// own. Roots that cannot be watched come back in
    /// [`WatchSession::failures`] rather than being dropped quietly.
    fn start(&self, roots: &[PathBuf]) -> WatchSession;
}

/// True when `path` is inside one of the declared roots (a root itself
/// counts). The single place "declared" is decided, so both the real backend
/// and the test double answer it the same way.
pub fn within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// Coalesces the burst of events one logical write produces.
///
/// Holds a last-seen instant per path and releases a path once it has been
/// quiet for `window`. Pure and clock-injected: every method takes `now`, so
/// tests never sleep.
#[derive(Debug)]
pub struct Debounce {
    window: Duration,
    pending: HashMap<PathBuf, Instant>,
}

impl Debounce {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            pending: HashMap::new(),
        }
    }

    /// Record activity on `path`, restarting its quiet period.
    pub fn touch(&mut self, path: PathBuf, now: Instant) {
        self.pending.insert(path, now);
    }

    /// Remove and return every path quiet for at least the window. Sorted, so
    /// a batch is deterministic.
    pub fn drain_quiet(&mut self, now: Instant) -> Vec<PathBuf> {
        let window = self.window;
        let mut ready: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, last)| now.duration_since(**last) >= window)
            .map(|(path, _)| path.clone())
            .collect();
        self.pending
            .retain(|_, last| now.duration_since(*last) < window);
        ready.sort();
        ready
    }

    /// Whether anything is waiting to go quiet.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// The `notify`-backed source: FSEvents on macOS, inotify on Linux,
/// ReadDirectoryChangesW on Windows.
///
/// [`start`](SessionEventSource::start) spawns a tokio task and must be
/// called from within a runtime.
pub struct NotifyEventSource {
    debounce_window: Duration,
    /// How often the debounce is examined. Shorter than the window, so a path
    /// is emitted close to when it goes quiet rather than a window late.
    tick: Duration,
    capacity: usize,
}

impl Default for NotifyEventSource {
    fn default() -> Self {
        Self::new(Duration::from_millis(DEFAULT_DEBOUNCE_MILLIS))
    }
}

impl NotifyEventSource {
    pub fn new(debounce_window: Duration) -> Self {
        let tick = (debounce_window / 4).max(Duration::from_millis(50));
        Self {
            debounce_window,
            tick,
            capacity: 64,
        }
    }
}

/// Classify a `notify` registration error. Watch exhaustion and a missing
/// root are different things to tell a contributor.
fn classify(err: &::notify::Error, root: &Path) -> WatchFailure {
    match &err.kind {
        ::notify::ErrorKind::PathNotFound => WatchFailure::RootUnavailable {
            root: root.to_path_buf(),
        },
        ::notify::ErrorKind::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            WatchFailure::RootUnavailable {
                root: root.to_path_buf(),
            }
        }
        _ => WatchFailure::RegistrationFailed {
            root: root.to_path_buf(),
        },
    }
}

impl SessionEventSource for NotifyEventSource {
    fn start(&self, roots: &[PathBuf]) -> WatchSession {
        use ::notify::{RecursiveMode, Watcher};

        let (out_tx, out_rx) = mpsc::channel(self.capacity);
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<::notify::Result<::notify::Event>>();

        let mut failures = Vec::new();
        let mut watched = Vec::new();

        let forward = raw_tx.clone();
        let watcher = ::notify::recommended_watcher(move |res| {
            // A closed receiver means the session was dropped; there is
            // nothing to report it to.
            let _ = forward.send(res);
        });

        let mut watcher = match watcher {
            Ok(watcher) => watcher,
            Err(err) => {
                // No backend at all: every declared root is uncovered.
                let failures = roots.iter().map(|root| classify(&err, root)).collect();
                let (_tx, rx) = mpsc::channel(1);
                return WatchSession::new(rx, Vec::new(), failures, Box::new(()));
            }
        };

        for root in roots {
            if !root.is_dir() {
                failures.push(WatchFailure::RootUnavailable { root: root.clone() });
                continue;
            }
            match watcher.watch(root, RecursiveMode::Recursive) {
                Ok(()) => watched.push(root.clone()),
                Err(err) => failures.push(classify(&err, root)),
            }
        }

        let declared: Vec<PathBuf> = watched.clone();
        let window = self.debounce_window;
        let tick = self.tick;
        let task = tokio::spawn(async move {
            let mut debounce = Debounce::new(window);
            let mut ticker = tokio::time::interval(tick);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    incoming = raw_rx.recv() => {
                        let Some(incoming) = incoming else { break };
                        match incoming {
                            Ok(event) => {
                                let now = Instant::now();
                                let removed_root = matches!(
                                    event.kind,
                                    ::notify::EventKind::Remove(_)
                                ) && event.paths.iter().any(|p| declared.iter().any(|r| r == p));
                                if removed_root {
                                    for root in event.paths.iter().filter(|p| declared.contains(p)) {
                                        let failure = WatchFailure::RootUnavailable {
                                            root: root.clone(),
                                        };
                                        if out_tx.send(WatchEvent::Failed(failure)).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                for path in event.paths {
                                    if within_roots(&path, &declared) {
                                        debounce.touch(path, now);
                                    }
                                }
                            }
                            Err(err) => {
                                // A watch that errors after registration is a
                                // root that has stopped being covered.
                                for root in err.paths.iter() {
                                    let failure = classify(&err, root);
                                    if out_tx.send(WatchEvent::Failed(failure)).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        if debounce.is_empty() {
                            continue;
                        }
                        let ready = debounce.drain_quiet(Instant::now());
                        if !ready.is_empty()
                            && out_tx.send(WatchEvent::Changed(ready)).await.is_err()
                        {
                            return;
                        }
                    }
                }
            }
        });

        WatchSession::new(
            out_rx,
            watched,
            failures,
            Box::new((watcher, TaskGuard(task))),
        )
    }
}

/// Aborts the pump task when the session is dropped.
struct TaskGuard(tokio::task::JoinHandle<()>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A hand-driven event source for tests.
///
/// It applies the same root rule and the same debounce as the real backend,
/// but takes its events and its clock from the test rather than the kernel.
/// Never wire this into anything that ships.
pub struct ScriptedEventSource {
    window: Duration,
    start_failures: Vec<WatchFailure>,
    driver: Arc<Mutex<Option<ScriptedInner>>>,
}

struct ScriptedInner {
    debounce: Debounce,
    declared: Vec<PathBuf>,
    out: mpsc::Sender<WatchEvent>,
}

impl ScriptedEventSource {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            start_failures: Vec::new(),
            driver: Arc::new(Mutex::new(None)),
        }
    }

    /// Make registration fail for these roots, as watch exhaustion or a
    /// vanished mount would.
    pub fn failing(window: Duration, failures: Vec<WatchFailure>) -> Self {
        Self {
            window,
            start_failures: failures,
            driver: Arc::new(Mutex::new(None)),
        }
    }

    /// A handle for feeding events and advancing the clock. Only valid after
    /// [`SessionEventSource::start`].
    pub fn driver(&self) -> ScriptedDriver {
        ScriptedDriver {
            inner: Arc::clone(&self.driver),
        }
    }
}

/// Drives a [`ScriptedEventSource`] by hand.
#[derive(Clone)]
pub struct ScriptedDriver {
    inner: Arc<Mutex<Option<ScriptedInner>>>,
}

impl ScriptedDriver {
    /// Report a change at `at`. A path outside the declared roots is dropped,
    /// exactly as the real backend drops it.
    pub fn emit(&self, path: impl Into<PathBuf>, at: Instant) {
        let mut guard = self.inner.lock().unwrap();
        let Some(inner) = guard.as_mut() else {
            panic!("ScriptedEventSource::start must be called before emit");
        };
        let path = path.into();
        if within_roots(&path, &inner.declared) {
            inner.debounce.touch(path, at);
        }
    }

    /// Move the clock to `now` and deliver whatever has gone quiet. Returns
    /// what was sent, which is empty when nothing is ready.
    pub fn settle(&self, now: Instant) -> Vec<PathBuf> {
        let mut guard = self.inner.lock().unwrap();
        let Some(inner) = guard.as_mut() else {
            panic!("ScriptedEventSource::start must be called before settle");
        };
        let ready = inner.debounce.drain_quiet(now);
        if !ready.is_empty() {
            let _ = inner.out.try_send(WatchEvent::Changed(ready.clone()));
        }
        ready
    }

    /// Report a root that stopped being watched mid-session.
    pub fn fail(&self, failure: WatchFailure) {
        let guard = self.inner.lock().unwrap();
        let Some(inner) = guard.as_ref() else {
            panic!("ScriptedEventSource::start must be called before fail");
        };
        let _ = inner.out.try_send(WatchEvent::Failed(failure));
    }
}

impl SessionEventSource for ScriptedEventSource {
    fn start(&self, roots: &[PathBuf]) -> WatchSession {
        let (tx, rx) = mpsc::channel(64);
        let failed: Vec<&Path> = self.start_failures.iter().map(|f| f.root()).collect();
        let declared: Vec<PathBuf> = roots
            .iter()
            .filter(|root| !failed.contains(&root.as_path()))
            .cloned()
            .collect();
        *self.driver.lock().unwrap() = Some(ScriptedInner {
            debounce: Debounce::new(self.window),
            declared: declared.clone(),
            out: tx,
        });
        WatchSession::new(rx, declared, self.start_failures.clone(), Box::new(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Duration {
        Duration::from_secs(2)
    }

    fn start_scripted(roots: &[PathBuf]) -> (ScriptedEventSource, WatchSession) {
        let source = ScriptedEventSource::new(window());
        let session = source.start(roots);
        (source, session)
    }

    #[test]
    fn burst_for_one_path_coalesces_into_one_emission() {
        let root = PathBuf::from("/roots/claude");
        let (source, mut session) = start_scripted(std::slice::from_ref(&root));
        let driver = source.driver();
        let t0 = Instant::now();
        let path = root.join("a/session.jsonl");

        for step in 0..5 {
            driver.emit(path.clone(), t0 + Duration::from_millis(100 * step));
        }
        // Still inside the window measured from the last event.
        assert!(driver.settle(t0 + Duration::from_millis(500)).is_empty());

        let ready = driver.settle(t0 + Duration::from_millis(400) + window());
        assert_eq!(ready, vec![path.clone()]);

        match session.events.try_recv() {
            Ok(WatchEvent::Changed(paths)) => assert_eq!(paths, vec![path]),
            other => panic!("expected one Changed batch, got {other:?}"),
        }
        assert!(session.events.try_recv().is_err(), "only one emission");
    }

    #[test]
    fn distinct_paths_are_emitted_independently() {
        let root = PathBuf::from("/roots/codex");
        let (source, _session) = start_scripted(std::slice::from_ref(&root));
        let driver = source.driver();
        let t0 = Instant::now();
        let early = root.join("one.jsonl");
        let late = root.join("two.jsonl");

        driver.emit(early.clone(), t0);
        driver.emit(late.clone(), t0 + Duration::from_secs(3));

        assert_eq!(driver.settle(t0 + Duration::from_secs(3)), vec![early]);
        assert_eq!(driver.settle(t0 + Duration::from_secs(6)), vec![late]);
    }

    #[test]
    fn a_path_still_receiving_events_is_not_emitted() {
        let root = PathBuf::from("/roots/claude");
        let (source, _session) = start_scripted(std::slice::from_ref(&root));
        let driver = source.driver();
        let t0 = Instant::now();
        let path = root.join("busy.jsonl");

        for step in 0..10 {
            let at = t0 + Duration::from_secs(step);
            driver.emit(path.clone(), at);
            assert!(
                driver.settle(at).is_empty(),
                "a path written every second never goes quiet"
            );
        }
        assert_eq!(driver.settle(t0 + Duration::from_secs(11)), vec![path]);
    }

    #[test]
    fn only_declared_roots_produce_events() {
        let declared = PathBuf::from("/roots/claude");
        let (source, _session) = start_scripted(std::slice::from_ref(&declared));
        let driver = source.driver();
        let t0 = Instant::now();

        driver.emit("/roots/somewhere-else/session.jsonl", t0);
        driver.emit("/roots/claude-sibling/session.jsonl", t0);
        assert!(
            driver.settle(t0 + window()).is_empty(),
            "an undeclared path produces nothing"
        );

        let inside = declared.join("nested/deep/session.jsonl");
        driver.emit(inside.clone(), t0 + window());
        assert_eq!(driver.settle(t0 + window() * 2), vec![inside]);
    }

    #[test]
    fn registration_failure_is_typed_not_a_silent_success() {
        let good = PathBuf::from("/roots/claude");
        let exhausted = PathBuf::from("/roots/codex");
        let source = ScriptedEventSource::failing(
            window(),
            vec![WatchFailure::RegistrationFailed {
                root: exhausted.clone(),
            }],
        );
        let session = source.start(&[good.clone(), exhausted.clone()]);

        assert!(session.degraded());
        assert_eq!(session.watched, vec![good]);
        assert_eq!(
            session.failures,
            vec![WatchFailure::RegistrationFailed { root: exhausted }]
        );
        assert_eq!(
            session.failures[0].health_label(),
            LABEL_WATCH_REGISTRATION_FAILED
        );
    }

    #[test]
    fn a_root_that_fails_registration_is_not_watched() {
        let exhausted = PathBuf::from("/roots/codex");
        let source = ScriptedEventSource::failing(
            window(),
            vec![WatchFailure::RegistrationFailed {
                root: exhausted.clone(),
            }],
        );
        let _session = source.start(std::slice::from_ref(&exhausted));
        let driver = source.driver();
        let t0 = Instant::now();

        driver.emit(exhausted.join("session.jsonl"), t0);
        assert!(
            driver.settle(t0 + window()).is_empty(),
            "a root that failed registration must not appear watched"
        );
    }

    #[test]
    fn a_root_lost_mid_session_is_surfaced() {
        let root = PathBuf::from("/roots/claude");
        let (source, mut session) = start_scripted(std::slice::from_ref(&root));
        source
            .driver()
            .fail(WatchFailure::RootUnavailable { root: root.clone() });

        match session.events.try_recv() {
            Ok(WatchEvent::Failed(failure)) => {
                assert_eq!(failure, WatchFailure::RootUnavailable { root });
                assert_eq!(failure.health_label(), LABEL_WATCH_ROOT_UNAVAILABLE);
            }
            other => panic!("expected a Failed event, got {other:?}"),
        }
    }

    #[test]
    fn health_labels_do_not_carry_paths() {
        let root = PathBuf::from("/home/someone/.claude/projects");
        for failure in [
            WatchFailure::RegistrationFailed { root: root.clone() },
            WatchFailure::RootUnavailable { root },
        ] {
            assert!(!failure.health_label().contains('/'));
        }
    }

    #[test]
    fn within_roots_requires_a_component_boundary() {
        let roots = vec![PathBuf::from("/roots/claude")];
        assert!(within_roots(Path::new("/roots/claude"), &roots));
        assert!(within_roots(Path::new("/roots/claude/a/b.jsonl"), &roots));
        assert!(!within_roots(
            Path::new("/roots/claude-other/b.jsonl"),
            &roots
        ));
        assert!(!within_roots(Path::new("/roots"), &roots));
    }

    #[test]
    fn notify_source_reports_a_missing_root_rather_than_watching_nothing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let missing = dir.path().join("never-created");
            let source = NotifyEventSource::new(Duration::from_millis(100));
            let session = source.start(std::slice::from_ref(&missing));
            assert!(session.watched.is_empty());
            assert_eq!(
                session.failures,
                vec![WatchFailure::RootUnavailable { root: missing }]
            );
            assert!(session.degraded());
        });
    }

    /// Real `notify` against a real directory. Ignored: event delivery timing
    /// differs per platform and per filesystem, and CI runners are the worst
    /// case for both. Local sanity check only.
    #[test]
    #[ignore]
    fn notify_source_delivers_a_real_write() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            let source = NotifyEventSource::new(Duration::from_millis(200));
            let mut session = source.start(std::slice::from_ref(&root));
            assert_eq!(session.watched, vec![root.clone()]);

            let file = root.join("session.jsonl");
            std::fs::write(&file, b"{}\n").unwrap();

            let received = tokio::time::timeout(Duration::from_secs(10), session.events.recv())
                .await
                .expect("no event within ten seconds");
            match received {
                Some(WatchEvent::Changed(paths)) => {
                    assert!(paths.iter().any(|p| p.ends_with("session.jsonl")));
                }
                other => panic!("expected Changed, got {other:?}"),
            }
        });
    }
}
