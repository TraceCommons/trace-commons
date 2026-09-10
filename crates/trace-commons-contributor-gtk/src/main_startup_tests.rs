// INTEGRATION: register in main.rs with #[cfg(test)]
// #[path = "main_startup_tests.rs"] mod startup_tests; this calls the actual
// connect_startup(application, dir, drivers) -> Rc<Cell<StartupState>> wiring.
// Under Weston run cargo test --bin trace-commons-shell
// startup_tests::quit_during_pending_start_suppresses_completion_and_releases_daemon
// -- --exact --ignored --test-threads=1. The FIFO blocks real backend startup;
// no constructor injection, OS credential, or real source directory is used.
#![cfg(target_os = "linux")]

use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use adw::prelude::*;
use trace_commons_contributor::config::{ConfigStore, DAEMON_LOCK_FILE, DAEMON_STATE_FILE};
use trace_commons_contributor::daemon::settings::{DaemonSettings, SourceDeclaration};
use trace_commons_contributor::daemon::state::DaemonState;

use crate::{Drivers, StartupState, connect_startup};

const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    store: ConfigStore,
    writer: Arc<Mutex<Option<File>>>,
    state: Vec<u8>,
    timed_out: Arc<AtomicBool>,
    stop: mpsc::Sender<()>,
    watchdog: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let dir = std::env::temp_dir().join(format!("tc-start-{}", &suffix[..8]));
        let store = ConfigStore::open(dir).unwrap();
        DaemonSettings {
            claude_source: Some(SourceDeclaration::Off),
            codex_source: Some(SourceDeclaration::Off),
            private_inference: false,
            ..Default::default()
        }
        .save(&store)
        .unwrap();
        let mut state = DaemonState::default();
        state.paused = true;
        let state = serde_json::to_vec(&state).unwrap();
        let fifo = store.daemon_path(DAEMON_STATE_FILE);
        assert!(
            Command::new("mkfifo")
                .args(["--mode=600", "--"])
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        // Linux permits opening a FIFO read/write without another peer.
        // The backend's read_to_end cannot finish until this writer closes.
        let writer = Arc::new(Mutex::new(Some(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(fifo)
                .unwrap(),
        )));
        let timed_out = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = mpsc::channel();
        let rescue_store = store.clone();
        let rescue_writer = Arc::clone(&writer);
        let rescue_state = state.clone();
        let rescue_timeout = Arc::clone(&timed_out);
        let watchdog = std::thread::spawn(move || {
            if stopped.recv_timeout(DEADLINE * 3).is_err() {
                rescue_timeout.store(true, Ordering::SeqCst);
                // A synchronous-start regression must fail instead of
                // stranding the GTK thread on this test's own barrier.
                let _ = release_barrier(&rescue_store, &rescue_writer, &rescue_state);
            }
        });
        Self {
            store,
            writer,
            state,
            timed_out,
            stop,
            watchdog: Some(watchdog),
        }
    }

    fn reader_is_blocked(&self) -> bool {
        let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
            return false;
        };
        let path = self.store.daemon_path(DAEMON_STATE_FILE);
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target == &path)
            .count()
            == 2
    }

    fn release(&self) {
        release_barrier(&self.store, &self.writer, &self.state).unwrap();
    }
}

fn release_barrier(
    store: &ConfigStore,
    writer: &Mutex<Option<File>>,
    state: &[u8],
) -> anyhow::Result<()> {
    if let Some(mut writer) = writer.lock().unwrap().take() {
        // Replace the path first so any later state read sees a regular
        // file. The pending reader still holds the original FIFO inode.
        store.write_daemon_file(DAEMON_STATE_FILE, state)?;
        writer.write_all(state)?;
    }
    Ok(())
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = release_barrier(&self.store, &self.writer, &self.state);
        let _ = self.stop.send(());
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        let _ = std::fs::remove_dir_all(self.store.dir());
    }
}

#[test]
#[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
fn quit_during_pending_start_suppresses_completion_and_releases_daemon() {
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let fixture = Rc::new(Fixture::new());
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.StartupShutdownTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let drivers = Rc::new(Drivers {
        exit_after_realize: false,
        realize_seconds: 0,
        open_preview: false,
        search_term: None,
        preview_tab: None,
        start_page: None,
        onboarding_page: None,
        show_toast: false,
    });
    let startup = connect_startup(
        &application,
        fixture.store.dir().into(),
        Rc::clone(&drivers),
    );
    let activations = Rc::new(Cell::new(0));
    application.connect_activate({
        let activations = Rc::clone(&activations);
        move |_| activations.set(activations.get() + 1)
    });
    let windows_added = Rc::new(Cell::new(0));
    application.connect_window_added({
        let windows_added = Rc::clone(&windows_added);
        move |_, _| windows_added.set(windows_added.get() + 1)
    });
    let observed = Rc::new(Cell::new(false));
    let timer_finished = Rc::new(Cell::new(false));
    let timer_expired = Rc::new(Cell::new(false));
    let timer = glib::timeout_add_local(Duration::from_millis(2), {
        let application = application.clone();
        let fixture = Rc::clone(&fixture);
        let observed = Rc::clone(&observed);
        let timer_finished = Rc::clone(&timer_finished);
        let timer_expired = Rc::clone(&timer_expired);
        let deadline = Instant::now() + DEADLINE;
        let mut activated_again = None;
        move || {
            if Instant::now() >= deadline {
                timer_expired.set(true);
                timer_finished.set(true);
                application.quit();
                return glib::ControlFlow::Break;
            }
            if fixture.reader_is_blocked() {
                observed.set(true);
                let since = activated_again.get_or_insert_with(|| {
                    application.activate();
                    Instant::now()
                });
                // Allow another activation's wrongly queued worker to run;
                // the FIFO keeps the original startup blocked throughout.
                if since.elapsed() >= Duration::from_millis(100) {
                    timer_finished.set(true);
                    application.quit();
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        }
    });
    // Uses the real activate/shutdown signals, including application.hold().
    application.run_with_args::<&str>(&[]);
    if !timer_finished.get() {
        timer.remove();
    }
    assert!(
        observed.get(),
        "application exited before pending startup was observed"
    );
    assert!(
        !timer_expired.get(),
        "GTK event loop did not advance during startup"
    );
    assert!(
        !fixture.timed_out.load(Ordering::SeqCst),
        "startup blocked GTK"
    );
    assert_eq!(activations.get(), 2);
    assert!(startup.get() == StartupState::Stopped);
    assert_eq!(windows_added.get(), 0);
    assert!(application.windows().is_empty());
    assert_eq!(
        Rc::strong_count(&drivers),
        3,
        "one startup must still be pending"
    );
    let daemon_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.store.daemon_path(DAEMON_LOCK_FILE))
        .unwrap();
    assert!(
        daemon_lock.try_lock().is_err(),
        "pending backend owns the daemon lock"
    );

    fixture.release();
    let deadline = Instant::now() + DEADLINE;
    loop {
        context.iteration(false);
        assert!(
            startup.get() == StartupState::Stopped,
            "late startup was adopted"
        );
        assert_eq!(windows_added.get(), 0, "a window appeared after shutdown");
        if Rc::strong_count(&drivers) == 2 && daemon_lock.try_lock().is_ok() {
            daemon_lock.unlock().unwrap();
            break;
        }
        assert!(
            Instant::now() < deadline,
            "late startup retained the daemon lock"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(application.windows().is_empty());
}
