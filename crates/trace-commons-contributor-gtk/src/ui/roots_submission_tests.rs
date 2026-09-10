// INTEGRATION: nest under ui::roots with #[cfg(test)] and
// #[path = "roots_submission_tests.rs"] mod submission_tests; run the ignored
// test under a Linux GTK display with --test-threads=1. No production seam or
// OS credential is needed: the real settings commit lock controls submission.
#![cfg(target_os = "linux")]

use std::cell::Cell;
use std::fs::{File, OpenOptions, Permissions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use adw::prelude::*;
use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::settings::{DaemonSettings, SourceDeclaration};
use trace_commons_contributor::source::discovery::SourceCandidate;

use crate::copy;
use crate::ui::roots::present_with;

const DEADLINE: Duration = Duration::from_secs(10);
static GTK_TEST: Mutex<()> = Mutex::new(());

struct Fixture {
    store: ConfigStore,
    lock: Arc<Mutex<Option<File>>>,
    timed_out: Arc<AtomicBool>,
    stop: mpsc::Sender<()>,
    watchdog: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("tc-roots-submit-{}", uuid::Uuid::new_v4()));
        let store = ConfigStore::open(dir.clone()).unwrap();
        DaemonSettings::default().save(&store).unwrap();
        let lock = Arc::new(Mutex::new(None));
        let timed_out = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = mpsc::channel();
        let watchdog_lock = Arc::clone(&lock);
        let watchdog_timeout = Arc::clone(&timed_out);
        let watchdog = std::thread::spawn(move || {
            if stopped.recv_timeout(DEADLINE * 3).is_err() {
                watchdog_timeout.store(true, Ordering::SeqCst);
                let _ = std::fs::set_permissions(&dir, Permissions::from_mode(0o700));
                // Also releases a regression that blocks the GTK thread itself.
                watchdog_lock.lock().unwrap().take();
            }
        });
        Self {
            store,
            lock,
            timed_out,
            stop,
            watchdog: Some(watchdog),
        }
    }

    fn lock_path(&self) -> PathBuf {
        self.store.dir().join(".cloud-credential-commit.lock")
    }

    fn hold(&self) {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(self.lock_path())
            .unwrap();
        file.lock().unwrap();
        assert!(self.lock.lock().unwrap().replace(file).is_none());
    }

    fn release(&self) {
        self.lock.lock().unwrap().take();
    }

    fn open_lock_handles(&self) -> usize {
        // The worker opens its handle after ConfigStore::open has restored
        // directory permissions. Observing it removes a chmod/startup race.
        let path = self.lock_path();
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target == &path)
            .count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.store.dir(), Permissions::from_mode(0o700));
        self.release();
        let _ = self.stop.send(());
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        let _ = std::fs::remove_dir_all(self.store.dir());
    }
}

fn spin_until(context: &glib::MainContext, fixture: &Fixture, condition: impl Fn() -> bool) {
    let until = Instant::now() + DEADLINE;
    while !condition() {
        assert!(!fixture.timed_out.load(Ordering::SeqCst), "GTK blocked");
        assert!(Instant::now() < until, "GTK submission did not advance");
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!fixture.timed_out.load(Ordering::SeqCst), "GTK blocked");
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
        result.push(widget);
    }
    result
}

fn assert_pending(
    context: &glib::MainContext,
    fixture: &Fixture,
    button: &gtk::Button,
    completed: &Cell<usize>,
) {
    spin_until(context, fixture, || fixture.open_lock_handles() >= 2);
    let responsive = Rc::new(Cell::new(false));
    let signal = Rc::clone(&responsive);
    glib::timeout_add_local_once(Duration::from_millis(100), move || signal.set(true));
    spin_until(context, fixture, || responsive.get());
    assert_eq!(fixture.open_lock_handles(), 2, "unexpected lock owners");
    assert!(!button.is_sensitive());
    assert_eq!(completed.get(), 0);
}

#[test]
#[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
fn pending_submission_is_single_and_failure_allows_retry() {
    let _serial = GTK_TEST.lock().unwrap();
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.RootsSubmissionTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let fixture = Fixture::new();
    let candidates = ["claude-code", "codex"]
        .into_iter()
        .map(|source| SourceCandidate {
            source: source.into(),
            path: fixture.store.dir().join(source),
            exists: false,
            session_count: 0,
            most_recent: None,
            relocated_by_env: false,
        })
        .collect();
    let completed = Rc::new(Cell::new(0));
    let on_declared = Rc::clone(&completed);
    present_with(
        &application,
        fixture.store.dir().into(),
        candidates,
        move || {
            on_declared.set(on_declared.get() + 1);
        },
    );
    let windows = application.windows();
    assert_eq!(windows.len(), 1);
    let window = &windows[0];
    let widgets = descendants(window.upcast_ref());
    let button = widgets
        .iter()
        .filter_map(|widget| widget.clone().downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(copy::ROOTS_CONTINUE))
        .unwrap();
    let failure = widgets
        .iter()
        .filter_map(|widget| widget.clone().downcast::<gtk::Label>().ok())
        .find(|label| label.text() == copy::ROOTS_FAILED)
        .unwrap();
    let off: Vec<_> = widgets
        .iter()
        .filter_map(|widget| widget.clone().downcast::<gtk::CheckButton>().ok())
        .filter(|button| button.label().as_deref() == Some(copy::ROOTS_OFF))
        .collect();
    assert_eq!(off.len(), 2);
    let source_controls: Vec<_> = widgets
        .iter()
        .filter(|widget| {
            widget.is::<gtk::CheckButton>()
                || widget
                    .downcast_ref::<gtk::Button>()
                    .is_some_and(|button| button.label().as_deref() == Some(copy::ROOTS_CHOOSE))
        })
        .collect();
    assert_eq!(source_controls.len(), 6);
    assert!(!button.is_sensitive());
    assert!(!failure.is_visible());
    for choice in off {
        choice.set_active(true);
    }
    assert!(button.is_sensitive());

    fixture.hold();
    button.emit_clicked();
    button.emit_clicked();
    assert_pending(&context, &fixture, &button, &completed);
    assert!(
        source_controls
            .iter()
            .all(|control| !control.is_sensitive())
    );
    std::fs::set_permissions(fixture.store.dir(), Permissions::from_mode(0o500)).unwrap();
    fixture.release();
    spin_until(&context, &fixture, || {
        failure.is_visible() && button.is_sensitive()
    });
    assert_eq!(completed.get(), 0);
    assert!(source_controls.iter().all(|control| control.is_sensitive()));
    assert!(window.is_visible());
    let unchanged = DaemonSettings::load(&fixture.store).unwrap();
    assert!(unchanged.claude_source.is_none() && unchanged.codex_source.is_none());

    std::fs::set_permissions(fixture.store.dir(), Permissions::from_mode(0o700)).unwrap();
    fixture.hold();
    button.emit_clicked();
    button.emit_clicked();
    assert_pending(&context, &fixture, &button, &completed);
    assert!(
        source_controls
            .iter()
            .all(|control| !control.is_sensitive())
    );
    fixture.release();
    spin_until(&context, &fixture, || completed.get() > 0);
    // A second queued worker must not produce a late completion callback.
    let settled = Rc::new(Cell::new(false));
    let signal = Rc::clone(&settled);
    glib::timeout_add_local_once(Duration::from_millis(100), move || signal.set(true));
    spin_until(&context, &fixture, || settled.get());
    spin_until(&context, &fixture, || fixture.open_lock_handles() == 0);
    while context.pending() {
        context.iteration(false);
    }
    assert_eq!(completed.get(), 1);
    assert!(!window.is_visible());
    assert!(!failure.is_visible());
    let persisted = DaemonSettings::load(&fixture.store).unwrap();
    assert_eq!(persisted.claude_source, Some(SourceDeclaration::Off));
    assert_eq!(persisted.codex_source, Some(SourceDeclaration::Off));
}
