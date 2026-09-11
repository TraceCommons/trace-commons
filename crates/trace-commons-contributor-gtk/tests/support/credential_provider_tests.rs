// INTEGRATION: nest in ui::credential with #[cfg(test)] and
// #[path = "../../tests/support/credential_provider_tests.rs"] mod provider_tests;
// Add Worker::fixture(dir: PathBuf) under #[cfg(test)], returning
// (Self, mpsc::Receiver<(u64, Job)>, async_channel::Sender<(u64, Outcome)>).
// It creates the same job/result/event channels as start_with, drops event_tx,
// and returns Self { jobs: job_tx, results, events, hosts_the_loop: false,
// next_id: Cell::new(1), dir }, job_rx, result_tx. No backend or thread opens.
// Run the ignored test alone under the existing isolated Weston/XDG CI session:
// cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
// --lib ui::credential::provider_tests::wallet_provider_signals_preserve_pending_and_recover
// -- --ignored --exact --test-threads=1
// Uses real App/widget callbacks and its result pump with synthetic refused IPC.
// App::build still invokes its existing desktop portal/tray helpers; use the CI
// session rather than a personal desktop. No Cloud/browser/OS credential calls.
#![cfg(target_os = "linux")]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use serde_json::{Value, json};

use crate::copy;
use crate::ui::{App, PRIVATE_INFERENCE_SCREEN, credential, onboarding};
use crate::worker::{Job, Outcome, Worker};

const DEADLINE: Duration = Duration::from_secs(5);

struct Fixture {
    app: Rc<App>,
    jobs: mpsc::Receiver<(u64, Job)>,
    results: async_channel::Sender<(u64, Outcome)>,
    starts: RefCell<Vec<(u64, Value)>>,
    methods: RefCell<Vec<String>>,
    dir: PathBuf,
}

impl Fixture {
    fn new(application: &adw::Application) -> Self {
        let dir = std::env::temp_dir().join(format!("tc-wallet-provider-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (worker, jobs, results) = Worker::fixture(dir.clone());
        Self {
            app: App::build(application, worker),
            jobs,
            results,
            starts: RefCell::new(Vec::new()),
            methods: RefCell::new(Vec::new()),
            dir,
        }
    }

    fn drain_jobs(&self) {
        while let Ok((id, job)) = self.jobs.try_recv() {
            let Job::Call { method, params } = job else {
                panic!("credential interaction submitted a non-call job");
            };
            self.methods.borrow_mut().push(method.clone());
            match method.as_str() {
                "near_ai_credential_start" => self.starts.borrow_mut().push((id, params)),
                "preview_visible" => {
                    assert_eq!(params, json!({ "entry_ids": [] }));
                    self.reply(id, Err("synthetic_read_refused".into()));
                }
                "native_wallet_flow" => {
                    assert_eq!(params["action"], "open");
                    assert_eq!(params["ingest_url"], "");
                    assert_eq!(params["account_id"], "");
                    self.reply(id, Err("synthetic_read_refused".into()));
                }
                "near_ai_credential_status" => self.reply(
                    id,
                    Ok(json!({ "state": "absent", "session_state": "absent" })),
                ),
                // These are the real App::build refreshes. Refusing them avoids
                // inventing unrelated screen models or opening onboarding from status.
                "status"
                | "arming_suggestion"
                | "get_settings"
                | "list_projects"
                | "list_pending"
                | "queue_outcome_counts"
                | "history_rollup"
                | "list_history"
                | "harness_list"
                | "near_ai_balance"
                | "near_ai_funding"
                | "get_public_profile"
                | "list_audit"
                | "discover_routing" => {
                    self.reply(id, Err("synthetic_read_refused".into()));
                }
                _ => panic!("unexpected credential-side operation: {method}"),
            }
        }
    }

    fn reply(&self, id: u64, result: Result<Value, String>) {
        assert!(self.results.try_send((id, Outcome::Call(result))).is_ok());
    }

    fn drive_until(&self, context: &glib::MainContext, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.drain_jobs();
            if condition() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "GTK credential callbacks did not advance"
            );
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn settle(&self, context: &glib::MainContext) {
        self.drive_until(context, || self.app.callbacks.borrow().is_empty());
    }

    fn status_reads(&self) -> usize {
        self.methods
            .borrow()
            .iter()
            .filter(|method| method.as_str() == "near_ai_credential_status")
            .count()
    }

    fn assert_near_start(&self, index: usize) {
        let starts = self.starts.borrow();
        assert_eq!(
            starts.len(),
            index + 1,
            "duplicate start escaped the pending guard"
        );
        assert_eq!(starts[index].1, json!({ "provider": "near" }));
    }

    fn refuse_start(&self, index: usize) {
        let id = self.starts.borrow()[index].0;
        // No attempt ID or URL is ever supplied, so the real callback cannot
        // launch a browser or cancel somebody else's browser ceremony.
        self.reply(id, Err("synthetic_start_refused".into()));
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

fn choice_index(selector: &gtk::DropDown, label: &str) -> Option<u32> {
    let model = selector.model()?;
    (0..model.n_items()).find(|index| {
        model
            .item(*index)
            .and_then(|item| item.downcast::<gtk::StringObject>().ok())
            .is_some_and(|item| item.string() == label)
    })
}

fn selector(root: &gtk::Widget) -> gtk::DropDown {
    let label = trace_commons_contributor::private_inference_copy::private_inference_copy()
        .credential_provider_near;
    let matches: Vec<_> = descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::DropDown>().ok())
        .filter(|selector| choice_index(selector, label).is_some())
        .collect();
    assert_eq!(matches.len(), 1, "expected the actual provider selector");
    matches[0].clone()
}

fn obtain_button(root: &gtk::Widget) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(copy::CREDENTIAL_OBTAIN))
        .expect("the credential sign-in button must be rendered")
}

fn choose_near(root: &gtk::Widget) {
    let copy = trace_commons_contributor::private_inference_copy::private_inference_copy();
    let selector = selector(root);
    selector.set_selected(choice_index(&selector, copy.credential_provider_near).unwrap());
    let notice = descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .find(|label| label.text() == copy.credential_wallet_notice)
        .expect("wallet key-custody notice must accompany the selector");
    assert!(notice.is_visible());
}

#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn wallet_provider_signals_preserve_pending_and_recover() {
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.WalletProviderTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let fixture = Fixture::new(&application);
    fixture.settle(&context);
    let app = &fixture.app;
    app.stack.set_visible_child_name(PRIVATE_INFERENCE_SCREEN);
    app.window.present();
    let root = app
        .private_inference
        .credential
        .root
        .upcast_ref::<gtk::Widget>();
    fixture.drive_until(&context, || root.is_mapped());

    choose_near(root);
    obtain_button(root).emit_clicked();
    obtain_button(root).emit_clicked();
    fixture.drive_until(&context, || !fixture.starts.borrow().is_empty());
    fixture.assert_near_start(0);
    assert!(credential::pending(app));
    assert!(
        !obtain_button(root).is_sensitive(),
        "settings sign-in stayed enabled while pending"
    );
    assert!(
        !selector(root).is_sensitive(),
        "settings provider stayed enabled while pending"
    );
    let locked = selector(root);
    let selected = locked.selected();
    let google = trace_commons_contributor::private_inference_copy::private_inference_copy()
        .credential_provider_google;
    locked.set_selected(choice_index(&locked, google).unwrap());
    assert_eq!(
        locked.selected(),
        selected,
        "pending selection changed the captured provider"
    );
    fixture.refuse_start(0);
    fixture.settle(&context);
    assert!(!credential::pending(app));
    assert!(obtain_button(root).is_sensitive());
    assert!(selector(root).is_sensitive());
    obtain_button(root).emit_clicked();
    fixture.drive_until(&context, || fixture.starts.borrow().len() == 2);
    fixture.assert_near_start(1);
    fixture.refuse_start(1);
    fixture.settle(&context);

    assert!(onboarding::present_at_page(app, "connect"));
    let window = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.transient_for().as_ref() == Some(app.window.upcast_ref()))
        .expect("real onboarding window must open");
    let root = window.upcast_ref::<gtk::Widget>();
    fixture.drive_until(&context, || {
        descendants(root).into_iter().any(|widget| {
            widget.downcast::<gtk::Button>().ok().is_some_and(|button| {
                button.label().as_deref() == Some(copy::CREDENTIAL_OBTAIN) && button.is_mapped()
            })
        })
    });
    choose_near(root);
    obtain_button(root).emit_clicked();
    obtain_button(root).emit_clicked();
    fixture.drive_until(&context, || fixture.starts.borrow().len() == 3);
    fixture.assert_near_start(2);
    assert!(credential::pending(app));
    assert!(!obtain_button(root).is_sensitive());
    assert!(!selector(root).is_sensitive());
    let reads = fixture.status_reads();
    // Let the actual mapped card's periodic status callback execute while its
    // sign-in response stays held. A poll must not make the action pressable.
    fixture.drive_until(&context, || fixture.status_reads() > reads);
    fixture.drive_until(&context, || app.callbacks.borrow().len() == 1);
    assert!(
        !obtain_button(root).is_sensitive(),
        "onboarding poll re-enabled a pending sign-in"
    );
    assert!(!selector(root).is_sensitive());
    obtain_button(root).emit_clicked();
    fixture.drain_jobs();
    fixture.assert_near_start(2);
    fixture.refuse_start(2);
    fixture.drive_until(&context, || {
        !credential::pending(app)
            && obtain_button(root).is_sensitive()
            && selector(root).is_sensitive()
    });
    obtain_button(root).emit_clicked();
    fixture.drive_until(&context, || fixture.starts.borrow().len() == 4);
    fixture.assert_near_start(3);
    fixture.refuse_start(3);
    fixture.settle(&context);
    assert!(!credential::pending(app));
}
