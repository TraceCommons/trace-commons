//! Verified Cloud organization handoff. Pending reads belong to a visible row
//! and are discarded when the credential changes or the row leaves the screen.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use trace_commons_contributor::daemon::nearai_credential::funding::{FundingReport, credits_url};
use trace_commons_contributor::private_inference_copy as copy;

use crate::ui::App;
use crate::ui::style::{self, space};

#[cfg(test)]
#[path = "../../tests/support/funding_widget_tests.rs"]
mod widget_tests;

pub struct FundingSection {
    pub root: gtk::Box,
    message: gtk::Label,
    action: gtk::Button,
    report: RefCell<Option<FundingReport>>,
    credential: RefCell<String>,
    generation: Cell<u64>,
    pending: Cell<bool>,
    blocked: Cell<bool>,
    #[cfg(test)]
    test_launcher: RefCell<Option<Box<dyn Fn(&str) -> bool>>>,
}

impl Default for FundingSection {
    fn default() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        root.append(&style::section(copy::FUNDING_TITLE));
        let message = gtk::Label::builder().xalign(0.0).wrap(true).build();
        message.add_css_class("tc-body");
        message.set_text(copy::FUNDING_UNAVAILABLE);
        root.append(&message);
        style::append_body(&root, copy::FUNDING_WHAT);
        let action = gtk::Button::with_label(copy::FUNDING_REFRESH);
        action.set_halign(gtk::Align::Start);
        action.set_height_request(44);
        root.append(&action);
        Self {
            root,
            message,
            action,
            report: RefCell::new(None),
            credential: RefCell::new(String::new()),
            generation: Cell::new(0),
            pending: Cell::new(false),
            blocked: Cell::new(false),
            #[cfg(test)]
            test_launcher: RefCell::new(None),
        }
    }
}

impl FundingSection {
    fn launch(&self, url: &str) -> bool {
        #[cfg(test)]
        if let Some(launch) = self.test_launcher.borrow().as_ref() {
            return launch(url);
        }
        gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>).is_ok()
    }
}

pub fn invalidate(app: &Rc<App>) {
    let view = &app.private_inference.funding;
    view.generation.set(view.generation.get().wrapping_add(1));
    view.pending.set(false);
    view.report.replace(None);
    view.message.set_text(copy::FUNDING_UNAVAILABLE);
    view.action.set_label(copy::FUNDING_REFRESH);
    view.action.set_sensitive(!view.blocked.get());
}

pub fn credential_pending(app: &Rc<App>) {
    app.private_inference.funding.blocked.set(true);
    invalidate(app);
}

pub fn credential_changed(app: &Rc<App>, state: &str) {
    let view = &app.private_inference.funding;
    let changed = *view.credential.borrow() != state || view.blocked.get();
    if !changed {
        return;
    }
    view.credential.replace(state.to_owned());
    view.blocked.set(state == copy::LABEL_CREDENTIAL_OBTAINING);
    invalidate(app);
    if view.root.is_mapped() && !view.blocked.get() {
        request(app, None);
    }
}

pub fn wire(app: &Rc<App>) {
    let view = &app.private_inference.funding;
    let weak = Rc::downgrade(app);
    view.root.connect_map(move |_| {
        if let Some(app) = weak.upgrade() {
            request(&app, None);
        }
    });
    let weak = Rc::downgrade(app);
    view.root.connect_unmap(move |_| {
        if let Some(app) = weak.upgrade() {
            invalidate(&app);
        }
    });
    let weak = Rc::downgrade(app);
    view.action.connect_clicked(move |_| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let expected = app
            .private_inference
            .funding
            .report
            .borrow()
            .as_ref()
            .and_then(destination);
        request(&app, expected);
    });
}

#[derive(Clone, PartialEq, Eq)]
struct Destination {
    organization: String,
    revision: String,
    url: String,
}

fn destination(report: &FundingReport) -> Option<Destination> {
    let FundingReport::Ready {
        organization_id,
        connection_revision,
        browser_url,
        ..
    } = report
    else {
        return None;
    };
    let url = credits_url(organization_id)?;
    if &url != browser_url
        || connection_revision.len() != 64
        || !connection_revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    Some(Destination {
        organization: organization_id.clone(),
        revision: connection_revision.clone(),
        url,
    })
}

fn request(app: &Rc<App>, expected: Option<Destination>) {
    let view = &app.private_inference.funding;
    if !view.root.is_mapped() || view.blocked.get() || view.pending.replace(true) {
        return;
    }
    let generation = view.generation.get();
    view.action.set_sensitive(false);
    let params = expected.as_ref().map_or_else(
        || json!({}),
        |expected| {
            json!({
                "expected_organization_id": expected.organization,
                "expected_connection_revision": expected.revision,
            })
        },
    );
    app.call("near_ai_funding", params, move |app, result| {
        let view = &app.private_inference.funding;
        if generation != view.generation.get() || !view.root.is_mapped() || view.blocked.get() {
            return;
        }
        view.pending.set(false);
        view.action.set_sensitive(true);
        let report = result
            .ok()
            .and_then(|value: Value| serde_json::from_value::<FundingReport>(value).ok());
        let target = report.as_ref().and_then(destination);
        let launch = expected
            .as_ref()
            .is_some_and(|expected| target.as_ref() == Some(expected));
        if expected.is_some() && !launch {
            invalidate(app);
            return;
        }
        if let Some(report) = report {
            view.message.set_text(&copy::funding_message(&report));
            view.action.set_label(if target.is_some() {
                copy::FUNDING_MANAGE
            } else {
                copy::FUNDING_REFRESH
            });
            view.report.replace(Some(report));
        } else {
            invalidate(app);
        }
        if launch {
            if let Some(target) = target {
                if !view.launch(&target.url) {
                    invalidate(app);
                }
            }
        }
    });
}
