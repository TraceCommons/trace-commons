//! The NEAR AI credit section, as the Mac's `PayoutLinkView` draws it on
//! Join once an enrollment is held: which organization the commons sends
//! NEAR AI credit to. Hidden unless the commons lists the `near_ai` rail, or
//! the account session is missing, which it says. Every sentence is the
//! core's (`payout_link_copy`); every decision `crate::payout_link`. The
//! person's click on Choose, after the consent for the selected
//! organization is on screen, is the confirmation; nothing is preselected.
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use serde_json::json;
use trace_commons_contributor::consent_copy::payout_link_copy;

use super::App;
use super::onboarding::body_label;
use crate::payout_link::{self as surface, Listing, NEAR_AI_RAIL, Phase, Picker, Rail, Refusal};

pub(super) struct PayoutLinkView {
    pub root: gtk::Box,
    app: Weak<App>,
    rail: RefCell<Option<Rail>>,
    picker: RefCell<Picker>,
    busy: Cell<bool>,
    refusal: Cell<Option<Refusal>>,
    status_refusal: Cell<Option<Refusal>>,
    generation: Cell<u64>,
}

fn clear(root: &gtk::Box) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
}

impl PayoutLinkView {
    pub(super) fn new(app: &Rc<App>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("tc-card");
        root.set_visible(false);
        Rc::new(Self {
            root,
            app: Rc::downgrade(app),
            rail: RefCell::new(None),
            picker: RefCell::new(Picker::default()),
            busy: Cell::new(false),
            refusal: Cell::new(None),
            status_refusal: Cell::new(None),
            generation: Cell::new(0),
        })
    }

    /// Read the rail. A failed read keeps nothing stale on screen, except a
    /// missing account session, which it says. A link with no name the
    /// daemon holds, or one the person may choose, lists the orgs.
    pub(super) fn refresh(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let weak = Rc::downgrade(self);
        app.call("payout_rails_status", json!({}), move |_, result| {
            let Some(view) = weak.upgrade().filter(|v| v.generation.get() == generation) else {
                return;
            };
            match result {
                Ok(status) => {
                    *view.rail.borrow_mut() = surface::near_ai_rail(&status);
                    view.status_refusal.set(None);
                }
                Err(label) => {
                    view.rail.borrow_mut().take();
                    let refusal = Refusal::from_label(&label);
                    view.status_refusal
                        .set((refusal == Refusal::SessionEnded).then_some(refusal));
                }
            }
            let rail = view.rail.borrow().clone();
            view.render();
            if let Some(rail) = rail {
                let copy = payout_link_copy();
                if !view.picker.borrow().changing
                    && (surface::status_line(&copy, &rail).is_none() || surface::can_choose(&rail))
                {
                    view.load_organizations(Some(generation));
                }
            }
        });
    }

    fn load_organizations(self: &Rc<Self>, refresh: Option<u64>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if self.busy.get() {
            return;
        }
        self.picker.borrow_mut().begin_loading();
        let weak = Rc::downgrade(self);
        app.call("payout_link_orgs", json!({}), move |app, result| {
            let Some(view) = weak.upgrade() else { return };
            if refresh.is_some_and(|g| g != view.generation.get()) {
                return;
            }
            match result.map(serde_json::from_value::<Listing>) {
                Ok(Ok(listing)) => {
                    view.refusal.set(None);
                    view.picker.borrow_mut().listed(listing);
                }
                Ok(Err(_)) => view.refuse(Refusal::Failed),
                Err(label) => view.refuse(Refusal::from_label(&label)),
            }
            view.render();
            // The org list can supply the linked org's name: read the rail
            // again for it.
            if let Some(generation) = refresh {
                let weak = Rc::downgrade(&view);
                app.call("payout_rails_status", json!({}), move |_, result| {
                    if let (Some(view), Ok(status)) = (weak.upgrade(), result) {
                        if view.generation.get() == generation {
                            *view.rail.borrow_mut() = surface::near_ai_rail(&status);
                            view.render();
                        }
                    }
                });
            }
        });
    }

    fn refuse(&self, refusal: Refusal) {
        self.refusal.set(Some(refusal));
        self.picker.borrow_mut().refused(refusal);
    }

    /// Choose, after the consent was shown for the selected org: start,
    /// then confirm, against the version shown. Nothing is saved if the
    /// start fails.
    fn choose(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let rail = self.rail.borrow().clone();
        let (org, revision) = {
            let picker = self.picker.borrow();
            (
                picker.selected_org().map(|o| o.id.clone()),
                picker.listing.as_ref().map(|l| l.orgs_revision.clone()),
            )
        };
        let (Some(rail), Some(org), Some(revision)) = (rail, org, revision) else {
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        self.picker.borrow_mut().begin_working();
        self.render();
        let weak = Rc::downgrade(self);
        app.call(
            "payout_link_start",
            json!({"rail_id": NEAR_AI_RAIL, "organization_id": org,
                "orgs_revision": revision, "expected_current_version": rail.version}),
            move |app, result| {
                if let Err(label) = result {
                    if let Some(view) = weak.upgrade() {
                        view.finish(Some(label));
                    }
                    return;
                }
                app.call(
                    "payout_link_confirm",
                    json!({"rail_id": NEAR_AI_RAIL, "action": "link",
                        "organization_id": org, "confirmed": true}),
                    move |_, result| {
                        if let Some(view) = weak.upgrade() {
                            view.finish(result.err());
                        }
                    },
                );
            },
        );
    }

    /// Stop sending credit: start and confirm an unlink in one click.
    fn stop(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(rail) = self.rail.borrow().clone() else {
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        self.render();
        let weak = Rc::downgrade(self);
        app.call(
            "payout_link_unlink",
            json!({"rail_id": NEAR_AI_RAIL, "expected_current_version": rail.version}),
            move |app, result| {
                if let Err(label) = result {
                    if let Some(view) = weak.upgrade() {
                        view.finish(Some(label));
                    }
                    return;
                }
                app.call(
                    "payout_link_confirm",
                    json!({"rail_id": NEAR_AI_RAIL, "action": "unlink", "confirmed": true}),
                    move |_, result| {
                        if let Some(view) = weak.upgrade() {
                            view.finish(result.err());
                        }
                    },
                );
            },
        );
    }

    fn finish(self: &Rc<Self>, failure: Option<String>) {
        self.busy.set(false);
        match failure {
            None => {
                self.refusal.set(None);
                self.picker.borrow_mut().finished();
            }
            Some(label) => self.refuse(Refusal::from_label(&label)),
        }
        // Re-read: a refused change may mean the link moved underneath.
        self.refresh();
    }

    fn render(self: &Rc<Self>) {
        let copy = payout_link_copy();
        clear(&self.root);
        let rail = self.rail.borrow().clone();
        let busy = self.busy.get();
        let title = body_label(copy.title);
        title.add_css_class("tc-flow-emphasis");
        let Some(rail) = rail else {
            // Only the missing account session is said; anything else hides
            // the section rather than show a stale link.
            let Some(refusal) = self.status_refusal.get() else {
                self.root.set_visible(false);
                return;
            };
            self.root.append(&title);
            let again = gtk::Button::with_label(refusal.line(&copy));
            again.connect_clicked({
                let weak = Rc::downgrade(self);
                move |button| {
                    let Some(view) = weak.upgrade() else { return };
                    let Some(app) = view.app.upgrade() else {
                        return;
                    };
                    button.set_sensitive(false);
                    let weak = Rc::downgrade(&view);
                    app.call("account_sign_in", json!({}), move |_, _| {
                        if let Some(view) = weak.upgrade() {
                            view.refresh();
                        }
                    });
                }
            });
            self.root.append(&again);
            self.root.set_visible(true);
            return;
        };
        self.root.append(&title);
        if let Some(line) = surface::status_line(&copy, &rail) {
            self.root.append(&body_label(&line));
        }
        if surface::can_unlink(&rail) {
            let body = body_label(copy.unlink_body);
            body.add_css_class("tc-meta");
            self.root.append(&body);
            let stop = gtk::Button::with_label(copy.unlink_button);
            stop.set_halign(gtk::Align::Start);
            stop.set_sensitive(!busy);
            if let Some(name) = rail.organization_name.as_deref() {
                let label = copy.unlink_accessibility_name.replace("{org_name}", name);
                stop.update_property(&[gtk::accessible::Property::Label(&label)]);
            }
            stop.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(view) = weak.upgrade() {
                        view.stop();
                    }
                }
            });
            self.root.append(&stop);
        }
        let picker = self.picker.borrow().clone();
        if surface::shows_change_button(&rail, &picker) {
            let change = gtk::Button::with_label(copy.change_button);
            change.set_halign(gtk::Align::Start);
            change.set_sensitive(!busy);
            change.update_property(&[gtk::accessible::Property::Label(
                copy.change_accessibility_name,
            )]);
            change.connect_clicked({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(view) = weak.upgrade() {
                        if view.busy.get() {
                            return;
                        }
                        view.picker.borrow_mut().begin_change();
                        view.render();
                        view.load_organizations(None);
                    }
                }
            });
            self.root.append(&change);
        }
        if surface::shows_picker(&rail, &picker) {
            self.append_picker(&picker);
        }
        if let Some(refusal) = self.refusal.get() {
            let line = body_label(refusal.line(&copy));
            line.add_css_class("tc-refused");
            self.root.append(&line);
        }
        self.root.set_visible(true);
    }

    fn append_picker(self: &Rc<Self>, picker: &Picker) {
        let copy = payout_link_copy();
        let busy = self.busy.get();
        match picker.phase {
            Phase::Idle | Phase::Loading | Phase::Refused(_) => {}
            Phase::NoOrganization => {
                let line = body_label(copy.no_organization);
                line.add_css_class("tc-meta");
                self.root.append(&line);
            }
            Phase::Choosing | Phase::Working => {
                let mut group: Option<gtk::CheckButton> = None;
                for org in picker.rows() {
                    let row = gtk::CheckButton::with_label(&org.name);
                    row.set_group(group.as_ref());
                    if group.is_none() {
                        group = Some(row.clone());
                    }
                    row.set_active(picker.selected.as_deref() == Some(org.id.as_str()));
                    row.set_sensitive(!busy);
                    let id = org.id.clone();
                    row.connect_toggled({
                        let weak = Rc::downgrade(self);
                        move |row| {
                            if !row.is_active() {
                                return;
                            }
                            if let Some(view) = weak.upgrade() {
                                view.picker.borrow_mut().select(&id);
                                let weak = Rc::downgrade(&view);
                                glib::idle_add_local_once(move || {
                                    if let Some(view) = weak.upgrade() {
                                        view.render();
                                    }
                                });
                            }
                        }
                    });
                    self.root.append(&row);
                }
                if let Some(consent) = picker.consent(&copy) {
                    let scope = body_label(&consent.scope);
                    scope.add_css_class("tc-meta");
                    self.root.append(&scope);
                    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let choose = gtk::Button::with_label(copy.button);
                    choose.set_sensitive(!busy);
                    choose.update_property(&[gtk::accessible::Property::Label(
                        &consent.accessibility_name,
                    )]);
                    choose.connect_clicked({
                        let weak = Rc::downgrade(self);
                        move |_| {
                            if let Some(view) = weak.upgrade() {
                                view.choose();
                            }
                        }
                    });
                    let not_now = gtk::Button::with_label(copy.secondary);
                    not_now.add_css_class("flat");
                    not_now.set_sensitive(!busy);
                    not_now.connect_clicked({
                        let weak = Rc::downgrade(self);
                        move |_| {
                            if let Some(view) = weak.upgrade() {
                                view.picker.borrow_mut().dismiss_selection();
                                view.render();
                            }
                        }
                    });
                    actions.append(&choose);
                    actions.append(&not_now);
                    self.root.append(&actions);
                }
            }
        }
    }
}
