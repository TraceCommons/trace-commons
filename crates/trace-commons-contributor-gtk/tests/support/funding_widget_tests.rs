// INTEGRATION: nest in ui::funding under #[cfg(test)] with
// #[path = "../../tests/support/funding_widget_tests.rs"] mod widget_tests;
// Reuse Worker::fixture(dir), returning (Worker, job receiver, result sender).
// FundingSection needs a test-only field initialized to None:
// test_launcher: RefCell<Option<Box<dyn Fn(&str) -> bool>>>.
// Its launch(&self, url: &str) helper calls that closure when present, otherwise
// the existing gio launcher; request's completion uses view.launch(&target.url).
// This fixture installs the recorder before mapping or clicking any widget.
// Run alone in the existing isolated Weston/XDG session:
// cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
// --lib ui::funding::widget_tests::billing_widgets_bind_and_invalidate_browser_handoffs
// -- --exact --ignored --test-threads=1
// App::build still starts its normal desktop portal/tray helpers. No daemon,
// credential store, Cloud service, or real browser is opened by this fixture.
#![cfg(target_os = "linux")]

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;
use serde_json::{Value, json};
use trace_commons_contributor::daemon::nearai_credential::funding::FundingReport;
use trace_commons_contributor::private_inference_copy as copy;

use crate::ui::funding::FundingSection;
use crate::ui::{App, PRIVATE_INFERENCE_SCREEN};
use crate::worker::{Job, Outcome, Worker};

const DEADLINE: Duration = Duration::from_secs(5);
const ORGANIZATION: &str = "synthetic-org-a";
const OTHER_ORGANIZATION: &str = "synthetic-org-b";

fn revision(character: char) -> String {
    character.to_string().repeat(64)
}

fn browser_url(organization: &str) -> String {
    format!("https://cloud.near.ai/dashboard/organizations/{organization}/credits")
}

fn ready(organization: &str, revision: &str) -> Value {
    json!({
        "state": "ready",
        "organization_id": organization,
        "organization_name": format!("Synthetic {organization}"),
        "connection_revision": revision,
        "browser_url": browser_url(organization),
        "observed_at": "2026-09-09T12:00:00Z",
    })
}

struct Fixture {
    app: Rc<App>,
    jobs: mpsc::Receiver<(u64, Job)>,
    results: async_channel::Sender<(u64, Outcome)>,
    funding: RefCell<Vec<(u64, Value)>>,
    forget: RefCell<Vec<(u64, Value)>>,
    held: RefCell<BTreeSet<u64>>,
    credential_state: Cell<&'static str>,
    launched: Rc<RefCell<Vec<String>>>,
    accept_launch: Rc<Cell<bool>>,
    dir: PathBuf,
}

impl Fixture {
    fn new(application: &adw::Application) -> Self {
        let dir = std::env::temp_dir().join(format!("tc-billing-widget-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (worker, jobs, results) = Worker::fixture(dir.clone());
        let app = App::build(application, worker);
        let launched = Rc::new(RefCell::new(Vec::new()));
        let accept_launch = Rc::new(Cell::new(true));
        let recorded = Rc::clone(&launched);
        let accepted = Rc::clone(&accept_launch);
        app.private_inference
            .funding
            .test_launcher
            .replace(Some(Box::new(move |url| {
                recorded.borrow_mut().push(url.to_owned());
                accepted.get()
            })));
        Self {
            app,
            jobs,
            results,
            funding: RefCell::new(Vec::new()),
            forget: RefCell::new(Vec::new()),
            held: RefCell::new(BTreeSet::new()),
            credential_state: Cell::new(copy::LABEL_CREDENTIAL_PRESENT),
            launched,
            accept_launch,
            dir,
        }
    }

    fn view(&self) -> &FundingSection {
        &self.app.private_inference.funding
    }

    fn drain_jobs(&self) {
        while let Ok((id, job)) = self.jobs.try_recv() {
            let Job::Call { method, params } = job else {
                panic!("billing submitted a non-call operation");
            };
            match method.as_str() {
                "preview_visible" => {
                    assert_eq!(params, json!({ "entry_ids": [] }));
                    self.reply(id, Err("synthetic_read_refused".into()));
                }
                "near_ai_funding" => {
                    self.held.borrow_mut().insert(id);
                    self.funding.borrow_mut().push((id, params));
                }
                "near_ai_credential_forget" => {
                    self.held.borrow_mut().insert(id);
                    self.forget.borrow_mut().push((id, params));
                }
                "near_ai_credential_status" => self.reply(
                    id,
                    Ok(json!({
                        "state": self.credential_state.get(),
                        "session_state": self.credential_state.get(),
                    })),
                ),
                // Refuse unrelated reads made by the actual App refresh. A
                // write, enrollment, or consent method is never permitted.
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
                | "get_public_profile"
                | "list_audit"
                | "discover_routing" => {
                    self.reply(id, Err("synthetic_read_refused".into()));
                }
                _ => panic!("unexpected billing-side operation: {method}"),
            }
        }
    }

    fn reply(&self, id: u64, result: Result<Value, String>) {
        self.held.borrow_mut().remove(&id);
        assert!(self.results.try_send((id, Outcome::Call(result))).is_ok());
    }

    fn answer(&self, index: usize, result: Result<Value, String>) {
        let id = self.funding.borrow()[index].0;
        assert!(
            self.held.borrow().contains(&id),
            "reply was already delivered"
        );
        self.reply(id, result);
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
                "GTK billing callbacks did not advance"
            );
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn settle(&self, context: &glib::MainContext) {
        self.drive_until(context, || {
            self.app.callbacks.borrow().len() == self.held.borrow().len()
        });
    }

    fn click(&self) -> usize {
        assert!(self.view().action.is_sensitive());
        let index = self.funding.borrow().len();
        self.view().action.emit_clicked();
        assert!(!self.view().action.is_sensitive());
        // Programmatically emitting a second click bypasses GTK sensitivity,
        // exercising the real handler's pending guard too.
        self.view().action.emit_clicked();
        self.drain_jobs();
        assert_eq!(self.funding.borrow().len(), index + 1);
        index
    }

    fn assert_binding(&self, index: usize, organization: &str, revision: &str) {
        assert_eq!(
            self.funding.borrow()[index].1,
            json!({
                "expected_organization_id": organization,
                "expected_connection_revision": revision,
            })
        );
    }

    fn assert_cleared(&self) {
        assert!(self.view().report.borrow().is_none());
        assert_eq!(self.view().message.text(), copy::FUNDING_UNAVAILABLE);
        assert_eq!(
            self.view().action.label().as_deref(),
            Some(copy::FUNDING_REFRESH)
        );
    }

    fn assert_ready(&self, organization: &str) {
        assert_eq!(
            self.view().message.text(),
            format!("Cloud organization: Synthetic {organization}")
        );
        assert_eq!(
            self.view().action.label().as_deref(),
            Some(copy::FUNDING_MANAGE)
        );
        assert!(self.view().action.is_sensitive());
    }

    fn refresh_ready(&self, context: &glib::MainContext, organization: &str, revision: &str) {
        let launches = self.launched.borrow().len();
        let index = self.click();
        assert_eq!(self.funding.borrow()[index].1, json!({}));
        self.answer(index, Ok(ready(organization, revision)));
        self.settle(context);
        self.assert_ready(organization);
        assert_eq!(
            self.launched.borrow().len(),
            launches,
            "refresh opened a browser"
        );
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

fn button_named(root: &gtk::Widget, label: &str) -> gtk::Button {
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.label().as_deref() == Some(label) {
                return button.clone();
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            pending.push(current);
        }
    }
    panic!("the actual credential button was not rendered");
}

#[test]
#[ignore = "requires an isolated Linux GTK display; run alone with --ignored --test-threads=1"]
fn billing_widgets_bind_and_invalidate_browser_handoffs() {
    let context = glib::MainContext::default();
    let _owner = context.acquire().unwrap();
    adw::init().expect("GTK display unavailable");
    let application = adw::Application::builder()
        .application_id("ai.tracecommons.BillingWidgetTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    let fixture = Fixture::new(&application);
    fixture.settle(&context);
    assert!(fixture.funding.borrow().is_empty());
    fixture
        .app
        .stack
        .set_visible_child_name(PRIVATE_INFERENCE_SCREEN);
    fixture.app.window.present();
    fixture.drive_until(&context, || {
        fixture.view().root.is_mapped() && !fixture.funding.borrow().is_empty()
    });
    fixture.settle(&context);
    assert_eq!(fixture.funding.borrow().len(), 1);
    assert_eq!(fixture.funding.borrow()[0].1, json!({}));
    assert!(!fixture.view().action.is_sensitive());
    assert!(fixture.launched.borrow().is_empty());
    let first_revision = revision('a');
    fixture.answer(0, Ok(ready(ORGANIZATION, &first_revision)));
    fixture.settle(&context);
    fixture.assert_ready(ORGANIZATION);

    let index = fixture.click();
    fixture.assert_binding(index, ORGANIZATION, &first_revision);
    fixture.answer(index, Ok(ready(ORGANIZATION, &first_revision)));
    fixture.settle(&context);
    assert_eq!(*fixture.launched.borrow(), [browser_url(ORGANIZATION)]);

    let mut wrong_url = ready(ORGANIZATION, &first_revision);
    wrong_url["browser_url"] = json!("https://example.invalid/credits");
    for refused in [
        Ok(ready(OTHER_ORGANIZATION, &first_revision)),
        Ok(ready(ORGANIZATION, &revision('b'))),
        Ok(wrong_url),
        Ok(json!({"state":"unavailable"})),
        Ok(json!({"state":42})),
        Err("synthetic_funding_refused".into()),
    ] {
        let launches = fixture.launched.borrow().len();
        let index = fixture.click();
        fixture.assert_binding(index, ORGANIZATION, &first_revision);
        fixture.answer(index, refused);
        fixture.settle(&context);
        fixture.assert_cleared();
        assert!(fixture.view().action.is_sensitive());
        assert_eq!(fixture.launched.borrow().len(), launches);
        fixture.refresh_ready(&context, ORGANIZATION, &first_revision);
    }

    fixture.accept_launch.set(false);
    let index = fixture.click();
    let launches = fixture.launched.borrow().len();
    fixture.answer(index, Ok(ready(ORGANIZATION, &first_revision)));
    fixture.settle(&context);
    assert_eq!(fixture.launched.borrow().len(), launches + 1);
    fixture.assert_cleared();
    fixture.accept_launch.set(true);
    fixture.refresh_ready(&context, ORGANIZATION, &first_revision);

    // Remap before delivering the old answer. It must not clear the new
    // request's pending guard, restore the old organization, or launch.
    let old = fixture.click();
    let launches = fixture.launched.borrow().len();
    fixture.view().root.set_visible(false);
    fixture.drive_until(&context, || !fixture.view().root.is_mapped());
    fixture.assert_cleared();
    let fresh = fixture.funding.borrow().len();
    fixture.view().root.set_visible(true);
    fixture.drive_until(&context, || {
        fixture.view().root.is_mapped() && fixture.funding.borrow().len() > fresh
    });
    assert_eq!(fixture.funding.borrow()[fresh].1, json!({}));
    fixture.answer(old, Ok(ready(ORGANIZATION, &first_revision)));
    fixture.settle(&context);
    assert!(!fixture.view().action.is_sensitive());
    fixture.assert_cleared();
    assert_eq!(fixture.launched.borrow().len(), launches);
    let second_revision = revision('b');
    fixture.answer(fresh, Ok(ready(OTHER_ORGANIZATION, &second_revision)));
    fixture.settle(&context);
    fixture.assert_ready(OTHER_ORGANIZATION);

    // The actual Forget button exercises credential.rs's invalidation wiring.
    // Holding its worker answer also proves a funding click cannot bypass it.
    let old = fixture.click();
    fixture.assert_binding(old, OTHER_ORGANIZATION, &second_revision);
    let root = fixture
        .app
        .private_inference
        .credential
        .root
        .upcast_ref::<gtk::Widget>();
    button_named(root, copy::CREDENTIAL_FORGET).emit_clicked();
    fixture.drain_jobs();
    assert_eq!(fixture.forget.borrow().len(), 1);
    assert_eq!(fixture.forget.borrow()[0].1, json!({}));
    fixture.assert_cleared();
    assert!(!fixture.view().action.is_sensitive());
    let calls = fixture.funding.borrow().len();
    fixture.view().action.emit_clicked();
    fixture.drain_jobs();
    assert_eq!(fixture.funding.borrow().len(), calls);
    fixture.answer(old, Ok(ready(OTHER_ORGANIZATION, &second_revision)));
    fixture.settle(&context);
    fixture.assert_cleared();
    assert!(!fixture.view().action.is_sensitive());
    assert_eq!(fixture.launched.borrow().len(), launches);
    fixture.credential_state.set(copy::LABEL_CREDENTIAL_ABSENT);
    let forget_id = fixture.forget.borrow()[0].0;
    fixture.reply(forget_id, Ok(json!({"revoked":false})));
    fixture.drive_until(&context, || fixture.funding.borrow().len() > calls);
    assert_eq!(fixture.funding.borrow()[calls].1, json!({}));
    fixture.answer(calls, Ok(json!({"state":"no_session"})));
    fixture.settle(&context);
    assert_eq!(
        fixture.view().message.text(),
        copy::funding_message(&FundingReport::NoSession)
    );
    assert_eq!(
        fixture.view().action.label().as_deref(),
        Some(copy::FUNDING_REFRESH)
    );
    assert!(fixture.view().action.is_sensitive());
    assert_eq!(fixture.launched.borrow().len(), launches);
    assert!(fixture.held.borrow().is_empty());
    assert!(fixture.app.callbacks.borrow().is_empty());
}
