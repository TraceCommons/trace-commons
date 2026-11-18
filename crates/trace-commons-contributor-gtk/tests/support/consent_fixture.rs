// The real App over Worker::fixture, for the consent-parity mounted tests.
// Nested in ui under #[cfg(all(test, target_os = "linux"))] as
// `consent_fixture`. Every daemon call is recorded; a method the test names
// as held waits for the test's reply, and every other call is refused with a
// fixed synthetic label, so the window's background reads never block and
// never succeed. No daemon, credential store or network is opened.
// Run each test alone in the isolated Weston session with
// -- --exact --ignored --test-threads=1.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use serde_json::Value;

use crate::ui::App;
use crate::worker::{Job, Outcome, Worker};

const DEADLINE: Duration = Duration::from_secs(5);

pub(crate) struct Fixture {
    pub app: Rc<App>,
    pub context: glib::MainContext,
    jobs: mpsc::Receiver<(u64, Job)>,
    results: async_channel::Sender<(u64, Outcome)>,
    held_methods: BTreeSet<&'static str>,
    /// Every call the window made, in order: (id, method, params).
    pub calls: RefCell<Vec<(u64, String, Value)>>,
    /// Held calls not yet answered.
    open: RefCell<BTreeSet<u64>>,
    dir: PathBuf,
    _application: adw::Application,
}

impl Fixture {
    /// `held` names the methods whose replies the test gives itself.
    pub fn new(name: &str, held: &[&'static str]) -> Self {
        let context = glib::MainContext::default();
        std::mem::forget(context.acquire().expect("main context"));
        adw::init().expect("GTK display unavailable");
        let application = adw::Application::builder()
            .application_id(format!("ai.tracecommons.ConsentTest.{name}"))
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        application
            .register(None::<&gtk::gio::Cancellable>)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("tc-consent-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (worker, jobs, results) = Worker::fixture(dir.clone());
        let app = App::build(&application, worker);
        let fixture = Self {
            app,
            context,
            jobs,
            results,
            held_methods: held.iter().copied().collect(),
            calls: RefCell::new(Vec::new()),
            open: RefCell::new(BTreeSet::new()),
            dir,
            _application: application,
        };
        fixture.settle();
        fixture
    }

    fn drain(&self) {
        while let Ok((id, job)) = self.jobs.try_recv() {
            let Job::Call { method, params } = job else {
                // Previews and searches are not consent calls; refuse them.
                let _ = self
                    .results
                    .try_send((id, Outcome::Preview(Err("synthetic".into()))));
                continue;
            };
            let held = self.held_methods.contains(method.as_str());
            self.calls.borrow_mut().push((id, method, params));
            if held {
                self.open.borrow_mut().insert(id);
            } else {
                self.send(id, Err("synthetic_read_refused".into()));
            }
        }
    }

    fn send(&self, id: u64, result: Result<Value, String>) {
        assert!(
            self.results
                .try_send((id, Outcome::Call(result.map_err(Into::into))))
                .is_ok()
        );
    }

    /// Pump the loop until `condition` holds.
    pub fn drive_until(&self, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.drain();
            if condition() {
                return;
            }
            assert!(Instant::now() < deadline, "GTK callbacks did not advance");
            self.context.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Pump until every call but the held ones has been answered.
    pub fn settle(&self) {
        self.drive_until(|| self.app.callbacks.borrow().len() == self.open.borrow().len());
    }

    /// The calls made to `method`, in order.
    pub fn calls_to(&self, method: &str) -> Vec<(u64, Value)> {
        self.drain();
        self.calls
            .borrow()
            .iter()
            .filter(|(_, m, _)| m == method)
            .map(|(id, _, params)| (*id, params.clone()))
            .collect()
    }

    /// Answer the last open call to `method`, then settle.
    pub fn answer(&self, method: &str, result: Result<Value, String>) {
        let calls = self.calls_to(method);
        let (id, _) = calls
            .iter()
            .rev()
            .find(|(id, _)| self.open.borrow().contains(id))
            .cloned()
            .unwrap_or_else(|| panic!("no open call to {method}"));
        self.open.borrow_mut().remove(&id);
        self.send(id, result);
        self.settle();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.results.close();
        for widget in gtk::Window::list_toplevels() {
            if let Ok(window) = widget.downcast::<gtk::Window>() {
                window.destroy();
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
