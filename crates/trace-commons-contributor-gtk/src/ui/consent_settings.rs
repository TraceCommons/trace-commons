//! Settings' consent controls, ported from the Mac: the sharing choice at
//! the top of Projects (`SharingChoiceCard.swift`) and the Uses section
//! (`ConsentSection.swift`). Every word is the core's.

use std::collections::BTreeSet;

use serde_json::Value;
use trace_commons_contributor::consent_copy::AutomaticGrantCopy;

use crate::model::{ConsentScope, Project};

/// Setup's two sharing paths, as Settings offers them again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SharingPath {
    AskMe,
    Automatic,
}

/// What is in force, from `automatic_grant`: Automatic while the
/// include-existing grant is, Ask each time otherwise. `None` for a reply
/// that does not say, so nothing is shown picked on a guess.
pub(crate) fn sharing_current(grant: &Value) -> Option<SharingPath> {
    let granted = grant.get("granted")?.as_bool()?;
    let include_existing = grant.get("include_existing").and_then(Value::as_bool);
    Some(if granted && include_existing == Some(true) {
        SharingPath::Automatic
    } else {
        SharingPath::AskMe
    })
}

/// Folders set to Ask by a rule of their own (`SharingChoice.askingFolders`).
pub(crate) fn asking_folders(projects: &[Project]) -> usize {
    projects
        .iter()
        .filter(|p| {
            p.configured
                && !p.is_unresolved_bucket
                && p.folder_mode.as_deref().unwrap_or(&p.mode) == "notify_only"
        })
        .count()
}

/// What pressing Automatic shows first (`SharingChoice.Step`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirstStep {
    KeepAsks(usize),
    Disclose { keep_folder_asks: bool },
}

pub(crate) fn first_step(asking: usize) -> FirstStep {
    if asking > 0 {
        FirstStep::KeepAsks(asking)
    } else {
        FirstStep::Disclose {
            keep_folder_asks: true,
        }
    }
}

/// `withdraw_automatic_grant` answered as the Mac decodes it
/// (`Withdrawal { withdrawn: Bool }`).
pub(crate) fn ask_each_time_confirmed(reply: &Result<Value, String>) -> bool {
    crate::first_run::acknowledged(
        &crate::first_run::Action::SetAskEachTime,
        match reply {
            Ok(value) => value,
            Err(_) => return false,
        },
    )
}

/// The scrub sheet's sentences (`AutomaticGrantCopy.lines`): the scrub
/// scope, its limit, and that nobody reviews. `None` when the core named no
/// scrub wording or a line is empty, so the sheet is not drawn on a guess.
pub(crate) fn grant_lines(copy: &AutomaticGrantCopy) -> Option<Vec<&'static str>> {
    let scrub = copy.patterns_only.as_ref().or(copy.model_scrubbed.as_ref())?;
    let lines = vec![scrub.scope, scrub.limit, copy.no_review];
    (!lines.iter().any(|l| l.is_empty()) && !copy.summary.is_empty()).then_some(lines)
}

/// The list `set_consent_scopes` is sent (`ConsentScopeRows.nextScopes`):
/// built from what the daemon reported, never from the ticks on screen.
pub(crate) fn next_scopes(
    reported: &[String],
    options: &[ConsentScope],
    toggling: &ConsentScope,
    granted: bool,
) -> BTreeSet<String> {
    let mut scopes: BTreeSet<String> = reported.iter().cloned().collect();
    scopes.extend(options.iter().filter(|o| o.always_on).map(|o| o.name.clone()));
    if granted {
        scopes.insert(toggling.name.clone());
    } else if !toggling.always_on {
        scopes.remove(&toggling.name);
    }
    scopes
}

/// With no daemon answer nothing reads as granted, the always-on row
/// included; otherwise always-on is locked on and the rest follow the
/// daemon's list.
pub(crate) fn scope_on(scope: &ConsentScope, granted: &[String], unavailable: bool) -> bool {
    !unavailable && (scope.always_on || granted.iter().any(|g| g == &scope.name))
}

pub(crate) fn scope_enabled(scope: &ConsentScope, busy: bool, unavailable: bool) -> bool {
    !scope.always_on && !busy && !unavailable
}

/// `set_consent_scopes`' parameters as the Mac sends them: none for an
/// empty list.
pub(crate) fn set_scopes_params(scopes: &BTreeSet<String>) -> Value {
    if scopes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "scopes": scopes })
    }
}

/// The confirmed list, or `None` for anything the Mac would not decode.
pub(crate) fn scopes_confirmed(reply: &Result<Value, String>) -> Option<Vec<String>> {
    let value = reply.as_ref().ok()?;
    value
        .get("consent_scopes")?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// A refused scope write's line: the Settings table's sentence.
pub(crate) fn consent_refusal_line() -> &'static str {
    trace_commons_contributor::source_copy::source_settings_copy().consent_save_failed
}


// --- The core's copy for a configuration ---------------------------------

/// The grant screens' words for the contributor configuration in `dir`, as
/// the Mac reads them (`tc_automatic_contribution_copy_json(configDir)`).
/// `None` when the configuration cannot be read, so nothing is disclosed on
/// a guess and the grant is not offered.
pub(crate) fn automatic_copy_for(dir: &std::path::Path) -> Option<AutomaticGrantCopy> {
    let store = trace_commons_contributor::config::ConfigStore::open(dir.to_path_buf()).ok()?;
    let config = store.load_config().ok()?;
    Some(trace_commons_contributor::consent_copy::automatic_contribution_copy(config.as_ref()))
}

// --- Widgets ---------------------------------------------------------------

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use trace_commons_contributor::first_run_copy::first_run_copy;

use super::App;
use super::style::{self, Tone, space};

/// Settings > Projects' sharing choice (`SharingChoiceCard`).
pub(crate) struct SharingCard {
    pub root: gtk::Box,
    pub(crate) ask: gtk::CheckButton,
    pub(crate) automatic: gtk::CheckButton,
    pub(crate) refusal: gtk::Label,
    /// The daemon's last `automatic_grant` answer, read; `None` until it
    /// answers, and the radios stay off and disabled until then.
    pub(crate) current: Cell<Option<SharingPath>>,
    /// The folders `list_projects` last reported, for the Ask count.
    pub(crate) projects: RefCell<Vec<Project>>,
    pub(crate) working: Cell<bool>,
    filling: Cell<bool>,
}

impl SharingCard {
    pub fn new() -> Self {
        let uses = first_run_copy().uses;
        let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
        let grant = trace_commons_contributor::consent_copy::automatic_contribution_copy(None);
        let root = style::card(gtk::Orientation::Vertical, space::S);
        root.append(&style::eyebrow(uses.sharing));
        let ask = gtk::CheckButton::with_label(uses.ask_each_time);
        let automatic = gtk::CheckButton::with_label(modes.choices[1].label);
        automatic.set_group(Some(&ask));
        root.append(&ask);
        if !grant.path_ask_first_detail.is_empty() && !grant.path_ask_first_title.is_empty() {
            style::append_caveat(&root, grant.path_ask_first_title);
        }
        root.append(&automatic);
        if !uses.automatic_scope.is_empty() {
            style::append_caveat(&root, uses.automatic_scope);
        }
        if !uses.settings_sharing_note.is_empty() {
            style::append_caveat(&root, uses.settings_sharing_note);
        }
        let refusal = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).build();
        refusal.add_css_class("tc-refused");
        root.append(&refusal);
        let card = Self {
            root,
            ask,
            automatic,
            refusal,
            current: Cell::new(None),
            projects: RefCell::new(Vec::new()),
            working: Cell::new(false),
            filling: Cell::new(false),
        };
        card.render();
        card
    }

    /// Show what is in force; nothing picked and nothing pressable until
    /// the daemon has said.
    pub(crate) fn render(&self) {
        self.filling.set(true);
        let current = self.current.get();
        self.ask.set_active(current == Some(SharingPath::AskMe));
        self.automatic
            .set_active(current == Some(SharingPath::Automatic));
        if current.is_none() {
            // A group always has one active; neither is the truth yet.
            self.ask.set_active(false);
            self.automatic.set_active(false);
        }
        let enabled = current.is_some() && !self.working.get();
        self.ask.set_sensitive(enabled);
        self.automatic.set_sensitive(enabled);
        self.filling.set(false);
    }

    fn refuse(&self, line: &str) {
        self.refusal.set_text(line);
        self.refusal.set_visible(true);
    }
}

pub(crate) fn wire(app: &Rc<App>) {
    let card = &app.settings.sharing;
    for (button, path) in [
        (card.ask.clone(), SharingPath::AskMe),
        (card.automatic.clone(), SharingPath::Automatic),
    ] {
        let weak = Rc::downgrade(app);
        button.connect_toggled(move |button| {
            let Some(app) = weak.upgrade() else { return };
            if app.settings.sharing.filling.get() || !button.is_active() {
                return;
            }
            // The radio shows what is in force until the daemon confirms
            // the change, so put it back before asking.
            app.settings.sharing.render();
            choose(&app, path);
        });
    }
    let weak = Rc::downgrade(app);
    app.settings.uses.refresh.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            app.refresh();
        }
    });
}

/// The reads both cards need, on every Settings refresh.
pub(crate) fn refresh(app: &Rc<App>) {
    app.call("automatic_grant", serde_json::json!({}), |app, result| {
        let card = &app.settings.sharing;
        card.current
            .set(result.ok().as_ref().and_then(sharing_current));
        card.render();
    });
    app.call("consent_options", serde_json::json!({}), |app, result| {
        let options: Option<Vec<ConsentScope>> = result
            .ok()
            .and_then(|v| serde_json::from_value(v.get("scopes")?.clone()).ok());
        *app.settings.uses.options.borrow_mut() = options.unwrap_or_default();
        render_uses(app);
    });
}

/// `list_projects` answered: the folders the Ask count reads.
pub(crate) fn note_projects(app: &Rc<App>, projects: &[Project]) {
    *app.settings.sharing.projects.borrow_mut() = projects.to_vec();
}

fn choose(app: &Rc<App>, path: SharingPath) {
    let card = &app.settings.sharing;
    if card.working.get() || Some(path) == card.current.get() || card.current.get().is_none() {
        return;
    }
    card.refusal.set_visible(false);
    match path {
        SharingPath::AskMe => {
            let Some((method, params)) = crate::first_run::Action::SetAskEachTime.ipc() else {
                return;
            };
            set_working(app, true);
            app.call(method, params, |app, result| {
                if !ask_each_time_confirmed(&result) {
                    app.settings
                        .sharing
                        .refuse(first_run_copy().uses.settings_sharing_save_failed);
                }
                set_working(app, false);
                app.refresh();
            });
        }
        SharingPath::Automatic => {
            match first_step(asking_folders(&card.projects.borrow())) {
                FirstStep::KeepAsks(count) => keep_asks_question(app, count),
                FirstStep::Disclose { keep_folder_asks } => disclose_scrub(app, keep_folder_asks),
            }
        }
    }
}

fn set_working(app: &Rc<App>, working: bool) {
    app.settings.sharing.working.set(working);
    app.settings.sharing.render();
}

fn refused(app: &Rc<App>) {
    app.settings
        .sharing
        .refuse(first_run_copy().uses.settings_sharing_refused);
    set_working(app, false);
}

fn keep_asks_question(app: &Rc<App>, count: usize) {
    let copy = first_run_copy();
    let title = if copy.uses.keep_asks_title.is_empty() {
        copy.uses.sharing
    } else {
        copy.uses.keep_asks_title
    };
    let dialog = adw::MessageDialog::new(Some(&app.window), Some(title), None);
    dialog.set_body(&copy.uses.keep_asks_body.replace("{count}", &count.to_string()));
    dialog.add_responses(&[
        ("cancel", copy.frame.cancel),
        ("all", copy.uses.keep_asks_all),
        ("keep", copy.uses.keep_asks_keep),
    ]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("keep"));
    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        match response {
            "keep" => disclose_scrub(&app, true),
            "all" => disclose_scrub(&app, false),
            _ => {}
        }
    });
    dialog.present();
}

/// One disclosure sheet: Cancel closes it and grants nothing; Continue is
/// not the default, so a stray Return never passes a disclosure.
fn sheet(app: &Rc<App>, title: &str, body: gtk::Box, on_continue: impl Fn() + 'static) {
    let copy = first_run_copy();
    let dialog = adw::MessageDialog::new(Some(&app.window), Some(title), None);
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", copy.frame.cancel), ("continue", copy.frame.continue_button)]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(None);
    dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
    let weak = Rc::downgrade(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response == "continue" {
            on_continue();
        } else if let Some(app) = weak.upgrade() {
            set_working(&app, false);
        }
    });
    dialog.present();
}

fn disclose_scrub(app: &Rc<App>, keep_folder_asks: bool) {
    let Some(lines) = automatic_copy_for(&app.worker.dir)
        .as_ref()
        .and_then(grant_lines)
    else {
        refused(app);
        return;
    };
    set_working(app, true);
    let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
    let body = gtk::Box::new(gtk::Orientation::Vertical, space::S);
    for line in lines {
        style::append_body(&body, line);
    }
    let weak = Rc::downgrade(app);
    sheet(app, modes.choices[1].label, body, move || {
        if let Some(app) = weak.upgrade() {
            disclose_witness(&app, keep_folder_asks);
        }
    });
}

fn disclose_witness(app: &Rc<App>, keep_folder_asks: bool) {
    app.call("route_disclosure", serde_json::json!({}), move |app, result| {
        let Some(facts) = result.ok().filter(|v| {
            trace_commons_contributor::consent_copy::route_disclosure_for_wire(v).is_some()
        }) else {
            refused(app);
            return;
        };
        let panel = crate::disclosure::panel(Some(&facts));
        let body = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        super::fill_disclosure_rows(&body, &panel.rows);
        let witness = facts["witness"]["signing_address"]
            .as_str()
            .map(str::to_owned);
        let weak = Rc::downgrade(app);
        sheet(app, &panel.title, body, move || {
            if let Some(app) = weak.upgrade() {
                grant(&app, witness.clone(), keep_folder_asks);
            }
        });
    });
}

/// Both disclosures seen: the core decides whether the grant may be asked
/// for and with which witness, from a fresh status, as at setup's Start.
fn grant(app: &Rc<App>, witness_shown: Option<String>, keep_folder_asks: bool) {
    app.call("status", serde_json::json!({}), move |app, result| {
        let Some(status) = result
            .ok()
            .and_then(|v| serde_json::from_value::<crate::model::Status>(v).ok())
        else {
            refused(app);
            return;
        };
        let progress = trace_commons_contributor::flow1::Flow1Progress {
            connected: status.logged_in,
            scopes_saved: Some(status.consent_scopes.clone()),
            path: Some(trace_commons_contributor::flow1::ContributionPath::Automatic),
            scrub_disclosure_seen: true,
            witness_disclosure_seen: true,
            witness_shown,
        };
        let Ok(witness) = trace_commons_contributor::flow1::grant_request(&progress) else {
            refused(app);
            return;
        };
        let action = crate::first_run::Action::GrantAutomatic {
            witness: witness.map(str::to_owned),
            keep_folder_asks,
        };
        let Some((method, params)) = action.ipc() else {
            refused(app);
            return;
        };
        app.call(method, params, move |app, result| {
            let granted = result
                .as_ref()
                .is_ok_and(|v| crate::first_run::acknowledged(&action, v));
            if granted {
                set_working(app, false);
            } else {
                refused(app);
            }
            app.refresh();
        });
    });
}

// --- Uses ------------------------------------------------------------------

/// Settings > Uses (`ConsentSection`).
pub(crate) struct UsesCard {
    pub root: gtk::Box,
    pub(crate) refresh: gtk::Button,
    rows: gtk::Box,
    pub(crate) refusal: gtk::Label,
    pub(crate) options: RefCell<Vec<ConsentScope>>,
    /// The daemon's own list and sign-in, from `status`; `None` until it
    /// answers.
    pub(crate) reported: RefCell<Option<(bool, Vec<String>)>>,
    pub(crate) busy: Cell<bool>,
}

impl UsesCard {
    pub fn new() -> Self {
        let words = trace_commons_contributor::shell_words_copy::settings_words();
        let shell = trace_commons_contributor::preview_copy::monitor_screens_copy().shell;
        let root = style::card(gtk::Orientation::Vertical, space::S);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let heading = style::eyebrow(words.consent_heading);
        heading.set_hexpand(true);
        head.append(&heading);
        let refresh = gtk::Button::with_label(shell.settings_refresh);
        refresh.set_tooltip_text(Some(shell.settings_refresh_uses));
        refresh.update_property(&[gtk::accessible::Property::Label(shell.settings_refresh_uses)]);
        head.append(&refresh);
        root.append(&head);
        style::append_caveat(&root, words.applies_from_now);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        root.append(&rows);
        let refusal = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).build();
        refusal.add_css_class("tc-refused");
        root.append(&refusal);
        Self {
            root,
            refresh,
            rows,
            refusal,
            options: RefCell::new(Vec::new()),
            reported: RefCell::new(None),
            busy: Cell::new(false),
        }
    }
}

/// `status` answered: the daemon's list and sign-in.
pub(crate) fn note_status(app: &Rc<App>, status: &crate::model::Status) {
    *app.settings.uses.reported.borrow_mut() =
        Some((status.logged_in, status.consent_scopes.clone()));
    render_uses(app);
}

pub(crate) fn render_uses(app: &Rc<App>) {
    let card = &app.settings.uses;
    let words = trace_commons_contributor::shell_words_copy::settings_words();
    while let Some(child) = card.rows.first_child() {
        card.rows.remove(&child);
    }
    let (logged_in, granted) = card.reported.borrow().clone().unwrap_or((false, Vec::new()));
    let unavailable = !logged_in || card.reported.borrow().is_none();
    let options = card.options.borrow().clone();
    for scope in options.iter().filter(|o| o.always_on) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let title = style::body(&scope.title);
        title.add_css_class("heading");
        row.append(&title);
        row.append(&style::tag(words.required, Tone::Neutral));
        card.rows.append(&row);
        if !scope.description.is_empty() {
            style::append_caveat(&card.rows, &scope.description);
        }
    }
    let reward = style::caveat(words.reward_opportunities);
    if !words.reward_opportunities_tooltip.is_empty() {
        reward.set_tooltip_text(Some(words.reward_opportunities_tooltip));
    }
    card.rows.append(&reward);
    for scope in options.iter().filter(|o| !o.always_on && o.grants_data_use) {
        let check = gtk::CheckButton::with_label(&scope.title);
        check.set_active(scope_on(scope, &granted, unavailable));
        check.set_sensitive(scope_enabled(scope, card.busy.get(), unavailable));
        card.rows.append(&check);
        if !scope.description.is_empty() {
            style::append_caveat(&card.rows, &scope.description);
        }
        let weak = Rc::downgrade(app);
        let scope = scope.clone();
        check.connect_toggled(move |check| {
            if let Some(app) = weak.upgrade() {
                toggle_scope(&app, &scope, check.is_active());
            }
        });
    }
}

/// Adds or removes one optional scope, sending the daemon's own list with
/// this one changed. No second write starts until the first has answered.
fn toggle_scope(app: &Rc<App>, scope: &ConsentScope, granted: bool) {
    let card = &app.settings.uses;
    let Some((true, reported)) = card.reported.borrow().clone() else {
        return;
    };
    if card.busy.get() || scope.always_on {
        return;
    }
    let scopes = next_scopes(&reported, &card.options.borrow(), scope, granted);
    card.refusal.set_visible(false);
    card.busy.set(true);
    // The rows show the daemon's list, not the click, until it answers.
    glib_idle_render(app);
    app.call("set_consent_scopes", set_scopes_params(&scopes), |app, result| {
        let card = &app.settings.uses;
        match scopes_confirmed(&result) {
            Some(confirmed) => {
                if let Some((_, list)) = card.reported.borrow_mut().as_mut() {
                    *list = confirmed;
                }
            }
            None => {
                card.refusal.set_text(consent_refusal_line());
                card.refusal.set_visible(true);
            }
        }
        card.busy.set(false);
        render_uses(app);
        app.refresh();
    });
}

/// Re-render the rows outside the toggle handler that asked, so the check
/// being toggled is not destroyed under its own signal.
fn glib_idle_render(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    gtk::glib::idle_add_local_once(move || {
        if let Some(app) = weak.upgrade() {
            render_uses(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConsentScope, Project};
    use serde_json::json;

    fn project(mode: &str, folder_mode: Option<&str>, configured: bool, bucket: bool) -> Project {
        serde_json::from_value(json!({
            "project_id": "p", "mode": mode, "folder_mode": folder_mode,
            "configured": configured, "is_unresolved_bucket": bucket,
        }))
        .unwrap()
    }

    fn scope(name: &str, always_on: bool) -> ConsentScope {
        serde_json::from_value(json!({
            "name": name, "title": name, "always_on": always_on, "grants_data_use": true,
        }))
        .unwrap()
    }

    #[test]
    fn the_choice_in_force_is_read_from_the_grant() {
        assert_eq!(
            sharing_current(&json!({"granted": true, "include_existing": true})),
            Some(SharingPath::Automatic)
        );
        for ask in [
            json!({"granted": true, "include_existing": false}),
            json!({"granted": true}),
            json!({"granted": false, "include_existing": true}),
        ] {
            assert_eq!(sharing_current(&ask), Some(SharingPath::AskMe), "{ask}");
        }
        for unknown in [json!({}), json!({"granted": "yes"}), json!(null)] {
            assert_eq!(sharing_current(&unknown), None, "{unknown}");
        }
    }

    #[test]
    fn folders_set_to_ask_by_their_own_rule_are_counted() {
        let projects = [
            project("notify_only", None, true, false),
            project("auto_upload", Some("notify_only"), true, false),
            project("notify_only", Some("auto_upload"), true, false),
            project("notify_only", None, false, false),
            project("notify_only", None, true, true),
            project("ignore", None, true, false),
        ];
        assert_eq!(asking_folders(&projects), 2);
    }

    #[test]
    fn automatic_asks_about_ask_folders_first() {
        assert_eq!(first_step(3), FirstStep::KeepAsks(3));
        assert_eq!(first_step(0), FirstStep::Disclose { keep_folder_asks: true });
    }

    #[test]
    fn ask_each_time_is_the_macs_withdrawal() {
        assert_eq!(
            crate::first_run::Action::SetAskEachTime.ipc(),
            Some(("withdraw_automatic_grant", json!({"return_folders_to_ask": true})))
        );
        assert!(ask_each_time_confirmed(&Ok(json!({"withdrawn": true}))));
        assert!(ask_each_time_confirmed(&Ok(json!({"withdrawn": false}))));
        assert!(!ask_each_time_confirmed(&Ok(json!({}))));
        assert!(!ask_each_time_confirmed(&Err("x".into())));
    }

    #[test]
    fn the_grant_carries_the_keep_asks_answer() {
        for keep in [true, false] {
            let (method, params) = crate::first_run::Action::GrantAutomatic {
                witness: Some("0xabc".into()),
                keep_folder_asks: keep,
            }
            .ipc()
            .unwrap();
            assert_eq!(method, "grant_automatic_including_existing");
            assert_eq!(
                params,
                json!({"confirmed": true, "witness_signing_address": "0xabc",
                       "include_existing": true, "keep_folder_asks": keep})
            );
        }
    }

    #[test]
    fn the_scrub_sheet_says_the_macs_three_lines() {
        let copy = trace_commons_contributor::consent_copy::automatic_contribution_copy(None);
        let scrub = copy.patterns_only.clone().or(copy.model_scrubbed.clone()).unwrap();
        assert_eq!(
            grant_lines(&copy),
            Some(vec![scrub.scope, scrub.limit, copy.no_review])
        );
    }

    /// The scrub sheet reads the contributor's configuration, as the Mac's
    /// `automaticContributionCopyJSON(configDir:)` does, and offers nothing
    /// when that configuration cannot be read.
    #[test]
    fn the_scrub_words_come_from_the_real_configuration() {
        let dir = std::env::temp_dir().join(format!("tc-scrub-copy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(automatic_copy_for(&dir).is_some());
        std::fs::write(dir.join("contributor.json"), "not json").unwrap();
        assert!(automatic_copy_for(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Setup's scrub sheet goes through the same reader.
    #[test]
    fn the_first_run_scrub_sheet_passes_the_configuration() {
        let source = include_str!("first_run.rs");
        assert!(!source.contains(concat!("automatic_contribution_copy", "(None)")));
        assert!(source.contains("automatic_copy_for("));
    }

    #[test]
    fn a_scope_write_starts_from_the_daemons_list() {
        let options = [scope("contribution", true), scope("training", false), scope("eval", false)];
        let reported = vec!["eval".to_string()];
        assert_eq!(
            next_scopes(&reported, &options, &options[1], true),
            ["contribution", "eval", "training"].map(String::from).into()
        );
        assert_eq!(
            next_scopes(&reported, &options, &options[2], false),
            ["contribution"].map(String::from).into()
        );
        // An always-on scope is never removed.
        assert_eq!(
            next_scopes(&reported, &options, &options[0], false),
            ["contribution", "eval"].map(String::from).into()
        );
    }

    #[test]
    fn scope_rows_read_off_and_locked_without_an_answer() {
        let always = scope("contribution", true);
        let optional = scope("training", false);
        let granted = vec!["training".to_string()];
        assert!(!scope_on(&always, &granted, true));
        assert!(scope_on(&always, &[], false));
        assert!(scope_on(&optional, &granted, false));
        assert!(!scope_on(&optional, &[], false));
        assert!(!scope_enabled(&always, false, false));
        assert!(scope_enabled(&optional, false, false));
        assert!(!scope_enabled(&optional, true, false));
        assert!(!scope_enabled(&optional, false, true));
    }

    #[test]
    fn the_scope_write_and_its_reply_follow_the_mac() {
        let empty = std::collections::BTreeSet::new();
        assert_eq!(set_scopes_params(&empty), json!({}));
        let some: std::collections::BTreeSet<String> = ["a".to_string()].into();
        assert_eq!(set_scopes_params(&some), json!({"scopes": ["a"]}));
        assert_eq!(
            scopes_confirmed(&Ok(json!({"consent_scopes": ["a", "b"]}))),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        for bad in [json!({}), json!({"consent_scopes": "a"}), json!({"consent_scopes": [1]})] {
            assert_eq!(scopes_confirmed(&Ok(bad)), None);
        }
        assert_eq!(scopes_confirmed(&Err("x".into())), None);
    }

    #[test]
    fn a_refused_scope_write_says_the_shared_line() {
        assert_eq!(
            consent_refusal_line(),
            trace_commons_contributor::source_copy::source_settings_copy().consent_save_failed
        );
    }
}
