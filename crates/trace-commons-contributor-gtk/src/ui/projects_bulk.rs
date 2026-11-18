//! Settings > Projects' bulk change, ported from the Mac's Projects
//! manager (`ProjectsManagerSheet`): select projects, Set to one of the
//! daemon's `bulk_modes` (Ask or Never, never Automatic), and the daemon
//! plans what that does (`plan_project_modes`) and whether it asks first.
//! The change itself is `set_project_modes`, with the counts the person
//! agreed to; a plan that grew since is asked again. Every word is the
//! core's.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::space;

/// `ProjectsBulkPlan.Outcome`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Nothing,
    Apply,
    Confirm,
    Refused,
}

/// The confirmation's words, composed by the core.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Confirmation {
    pub title: String,
    pub body: String,
    pub button: String,
    pub keep: String,
    #[serde(default)]
    pub help: Option<String>,
    pub destructive: bool,
}

/// `plan_project_modes`' answer, every field the Mac requires.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Plan {
    pub outcome: Outcome,
    pub project_ids: Vec<String>,
    pub turns_off_automatic: u64,
    pub pending: u64,
    #[serde(default)]
    pub confirmation: Option<Confirmation>,
    pub bulk_modes: Vec<String>,
}

pub(crate) fn bulk_plan(reply: &Result<Value, String>) -> Option<Plan> {
    serde_json::from_value(reply.as_ref().ok()?.clone()).ok()
}

/// The menu's modes: the daemon's `bulk_modes`, and never Automatic.
pub(crate) fn offered_modes(modes: &[String]) -> Vec<String> {
    modes
        .iter()
        .filter(|m| m.as_str() == "notify_only" || m.as_str() == "ignore")
        .cloned()
        .collect()
}

/// `set_project_modes`' parameters: the counts agreed to, only after a
/// confirmation.
pub(crate) fn set_params(ids: &[String], mode: &str, agreed: Option<&Plan>) -> Value {
    let mut params = serde_json::json!({ "project_ids": ids, "mode": mode });
    if let Some(plan) = agreed {
        params["agreed"] = serde_json::json!({
            "turns_off_automatic": plan.turns_off_automatic,
            "pending": plan.pending,
        });
    }
    params
}

/// How a bulk change ended (`AppModel.BulkChange`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Ran { changed: Vec<String>, total: u64 },
    Stale,
    Failed,
}

pub(crate) fn change_outcome(reply: &Result<Value, String>) -> Change {
    match reply {
        Ok(value) => {
            let changed: Option<Vec<String>> = value
                .get("changed")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let total = value.get("total").and_then(Value::as_u64);
            match (changed, total) {
                (Some(changed), Some(total)) => Change::Ran { changed, total },
                _ => Change::Failed,
            }
        }
        Err(label) if label == "bulk-confirmation-stale" => Change::Stale,
        Err(_) => Change::Failed,
    }
}

pub(crate) fn after_change(selection: &BTreeSet<String>, changed: &[String]) -> BTreeSet<String> {
    selection
        .iter()
        .filter(|id| !changed.contains(id))
        .cloned()
        .collect()
}

/// A change stopped part way, in the core's words.
pub(crate) fn changed_line(changed: usize, total: u64) -> Option<String> {
    ((changed as u64) < total).then(|| {
        trace_commons_contributor::shell_words_copy::projects_manager_words()
            .changed_some
            .replace("{count}", &changed.to_string())
            .replace("{total}", &total.to_string())
    })
}

// --- Widgets ---------------------------------------------------------------

/// The selection, the daemon's modes and whether a change is running.
#[derive(Default)]
pub(crate) struct BulkState {
    pub(crate) selection: RefCell<BTreeSet<String>>,
    /// `None` until `plan_project_modes` has answered; the menu offers
    /// nothing until then.
    pub(crate) modes: RefCell<Option<Vec<String>>>,
    pub(crate) running: Cell<bool>,
    /// The projects the list last showed, for Select all.
    pub(crate) shown: RefCell<Vec<String>>,
}

/// Which modes a bulk change may set, with no project named, as the Mac's
/// sheet asks when it opens.
pub(crate) fn refresh(app: &Rc<App>) {
    app.call(
        "plan_project_modes",
        serde_json::json!({ "project_ids": [], "mode": "notify_only" }),
        |app, result| {
            *app.settings.bulk.modes.borrow_mut() =
                bulk_plan(&result).map(|plan| offered_modes(&plan.bulk_modes));
        },
    );
}

/// The bar above the list: Select all, how many are selected, and Set to.
pub(crate) fn bar(app: &Rc<App>, shown: &[String]) -> gtk::Box {
    let words = trace_commons_contributor::shell_words_copy::projects_manager_words();
    let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
    let state = &app.settings.bulk;
    *state.shown.borrow_mut() = shown.to_vec();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    let selection = state.selection.borrow().clone();
    let count = shown.iter().filter(|id| selection.contains(*id)).count();
    let all = gtk::CheckButton::with_label(words.select_all);
    all.set_active(!shown.is_empty() && count == shown.len());
    all.set_inconsistent(count > 0 && count < shown.len());
    all.set_sensitive(!shown.is_empty());
    bar.append(&all);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    if count > 0 {
        bar.append(&gtk::Label::new(Some(
            &words.selected.replace("{count}", &count.to_string()),
        )));
    }
    let menu = gtk::MenuButton::builder().label(words.set_to).build();
    menu.update_property(&[gtk::accessible::Property::Label(
        &words
            .bulk_menu_accessibility
            .replace("{count}", &count.to_string()),
    )]);
    let offered = state.modes.borrow().clone();
    menu.set_sensitive(count > 0 && !state.running.get() && offered.is_some());
    let popover_box = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
    for mode in offered.unwrap_or_default() {
        let Some(choice) = modes.choices.iter().find(|c| c.mode == mode) else {
            continue;
        };
        let item = gtk::Button::with_label(choice.label);
        item.add_css_class("flat");
        popover_box.append(&item);
        let weak = Rc::downgrade(app);
        let menu_for_item = menu.clone();
        item.connect_clicked(move |_| {
            menu_for_item.popdown();
            let Some(app) = weak.upgrade() else { return };
            let ids: Vec<String> = {
                let selection = app.settings.bulk.selection.borrow();
                app.settings
                    .bulk
                    .shown
                    .borrow()
                    .iter()
                    .filter(|id| selection.contains(*id))
                    .cloned()
                    .collect()
            };
            run(&app, ids, mode.clone());
        });
    }
    menu.set_popover(Some(&gtk::Popover::builder().child(&popover_box).build()));
    bar.append(&menu);
    let weak = Rc::downgrade(app);
    let shown_ids = shown.to_vec();
    all.connect_toggled(move |all| {
        let Some(app) = weak.upgrade() else { return };
        {
            let mut selection = app.settings.bulk.selection.borrow_mut();
            for id in &shown_ids {
                if all.is_active() {
                    selection.insert(id.clone());
                } else {
                    selection.remove(id);
                }
            }
        }
        rerender(&app);
    });
    bar
}

/// One project's checkbox (`select_accessibility`).
pub(crate) fn row_check(app: &Rc<App>, project_id: &str, title: &str) -> gtk::CheckButton {
    let words = trace_commons_contributor::shell_words_copy::projects_manager_words();
    let check = gtk::CheckButton::new();
    check.set_active(app.settings.bulk.selection.borrow().contains(project_id));
    check.update_property(&[gtk::accessible::Property::Label(
        &words.select_accessibility.replace("{title}", title),
    )]);
    let weak = Rc::downgrade(app);
    let id = project_id.to_string();
    check.connect_toggled(move |check| {
        let Some(app) = weak.upgrade() else { return };
        if check.is_active() {
            app.settings.bulk.selection.borrow_mut().insert(id.clone());
        } else {
            app.settings.bulk.selection.borrow_mut().remove(&id);
        }
        rerender(&app);
    });
    check
}

/// Redraw the list from the projects it last showed, outside the signal
/// that asked.
fn rerender(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    gtk::glib::idle_add_local_once(move || {
        if let Some(app) = weak.upgrade() {
            super::settings::rerender_projects(&app);
        }
    });
}

/// Ask the daemon what setting `ids` to `mode` does, then do it, or ask
/// first in its words. Nothing runs while another change does.
pub(crate) fn run(app: &Rc<App>, ids: Vec<String>, mode: String) {
    let state = &app.settings.bulk;
    if state.running.get() || ids.is_empty() {
        return;
    }
    state.running.set(true);
    plan(app, ids, mode);
}

fn done(app: &Rc<App>) {
    app.settings.bulk.running.set(false);
    app.refresh();
}

fn plan(app: &Rc<App>, ids: Vec<String>, mode: String) {
    let params = serde_json::json!({ "project_ids": ids.clone(), "mode": mode.clone() });
    app.call("plan_project_modes", params, move |app, result| {
        let Some(plan) = bulk_plan(&result) else {
            done(app);
            return;
        };
        match plan.outcome {
            Outcome::Nothing | Outcome::Refused => done(app),
            Outcome::Apply => apply(app, ids.clone(), mode.clone(), None),
            Outcome::Confirm => confirm(app, ids.clone(), mode.clone(), plan),
        }
    });
}

/// The daemon's confirmation. Keep is the close and the default: every
/// bulk confirmation turns off Automatic or removes waiting traces, so it
/// is pressed, never entered (owner, 2026-11-04 on #1963).
fn confirm(app: &Rc<App>, ids: Vec<String>, mode: String, plan: Plan) {
    let Some(copy) = plan.confirmation.clone() else {
        done(app);
        return;
    };
    let dialog = adw::MessageDialog::new(Some(&app.window), Some(&copy.title), Some(&copy.body));
    dialog.add_responses(&[
        ("keep", copy.keep.as_str()),
        ("confirm", copy.button.as_str()),
    ]);
    if copy.destructive {
        dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    }
    dialog.set_close_response("keep");
    dialog.set_default_response(Some("keep"));
    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response == "confirm" {
            apply(&app, ids.clone(), mode.clone(), Some(plan.clone()));
        } else {
            done(&app);
        }
    });
    dialog.present();
}

/// Make the change. Only the projects that changed leave the selection; a
/// plan whose counts grew since it was shown is planned and asked again.
fn apply(app: &Rc<App>, ids: Vec<String>, mode: String, agreed: Option<Plan>) {
    let params = set_params(&ids, &mode, agreed.as_ref());
    app.call(
        "set_project_modes",
        params,
        move |app, result| match change_outcome(&result) {
            Change::Ran { changed, total } => {
                let after = after_change(&app.settings.bulk.selection.borrow(), &changed);
                *app.settings.bulk.selection.borrow_mut() = after;
                if let Some(line) = changed_line(changed.len(), total) {
                    app.toast(&line);
                }
                done(app);
            }
            Change::Stale => plan(app, ids.clone(), mode.clone()),
            Change::Failed => {
                app.toast(crate::copy::project_mode_failed());
                done(app);
            }
        },
    );
}

#[cfg(test)]
#[path = "projects_bulk_tests.rs"]
mod tests;
