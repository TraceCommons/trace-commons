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
    let scrub = copy
        .patterns_only
        .as_ref()
        .or(copy.model_scrubbed.as_ref())?;
    let lines = vec![scrub.scope, scrub.limit, copy.no_review];
    (!lines.iter().any(|l| l.is_empty()) && !copy.summary.is_empty()).then_some(lines)
}

/// The Automatic radio's line (`UsesScreenLayout.sharingOptions`): the
/// first run's scope line when the core has one; else the grant table's
/// path title, when its detail and scrub wording are there to stand
/// behind it; else none.
pub(crate) fn automatic_line<'a>(
    automatic_scope: &'a str,
    grant: &AutomaticGrantCopy,
) -> Option<&'a str> {
    if !automatic_scope.is_empty() {
        return Some(automatic_scope);
    }
    let scrub = grant.patterns_only.is_some() || grant.model_scrubbed.is_some();
    (!grant.path_automatic_title.is_empty() && !grant.path_automatic_detail.is_empty() && scrub)
        .then_some(grant.path_automatic_title)
}

/// Whether Settings draws the sharing and Uses cards: only once a status
/// has said first run is done (R-43, `availableBeforeOnboarding`), never on
/// a guess.
pub(crate) fn cards_shown(requires_onboarding: Option<bool>) -> bool {
    requires_onboarding == Some(false)
}

/// The account a grant or withdrawal belongs to (the Mac's
/// `settleAutomaticGrantOwner` and `accountReadGeneration`). A status read
/// settles the owner; a status naming a different account starts a new
/// generation, and a write pressed under the old one is not applied.
#[derive(Debug, Default)]
pub(crate) struct GrantOwner {
    /// The account scope the last status read named; `None` before any
    /// read, or after a failed one.
    seen: Option<Option<String>>,
    generation: u64,
}

impl GrantOwner {
    pub(crate) fn observe(&mut self, scope: Option<&str>) {
        let scope = scope.map(str::to_owned);
        if self.seen.as_ref().is_some_and(|seen| *seen != scope) {
            self.generation += 1;
        }
        self.seen = Some(scope);
    }

    /// A failed status read: no owner until the next one answers.
    pub(crate) fn forget(&mut self) {
        self.seen = None;
    }

    /// Whether the owner is known without asking again: a read named an
    /// account (the Mac's `status.accountScope != nil`).
    pub(crate) fn named(&self) -> bool {
        matches!(self.seen, Some(Some(_)))
    }

    /// The generation a write is pressed under, or `None` while no status
    /// has settled the owner.
    pub(crate) fn begin(&self) -> Option<u64> {
        self.seen.as_ref().map(|_| self.generation)
    }

    pub(crate) fn is_current(&self, pressed: u64) -> bool {
        self.seen.is_some() && self.generation == pressed
    }
}

/// The override in force, from `status.contribution_override`: its mode,
/// or none.
pub(crate) fn override_mode(value: &Value) -> Option<String> {
    value.get("mode")?.as_str().map(str::to_owned)
}

/// What the radios show: nothing picked while an override is in force,
/// since neither Ask each time nor the grant is what decides then.
pub(crate) fn shown_path(
    current: Option<SharingPath>,
    override_mode: Option<&str>,
) -> Option<SharingPath> {
    if override_mode.is_some() {
        None
    } else {
        current
    }
}

/// Whether clearing the override confirms first
/// (`MenuPanelData.clearNeedsConfirmation`): not under an Automatic
/// override, which already sends everything; otherwise when the folders
/// were not read, or one's own setting is Automatic or unreported.
pub(crate) fn clear_needs_confirmation(projects: Option<&[Project]>, mode: Option<&str>) -> bool {
    if mode == Some("auto_upload") {
        return false;
    }
    let Some(projects) = projects else {
        return true;
    };
    projects
        .iter()
        .any(|p| p.folder_mode.is_none() || p.folder_mode.as_deref() == Some("auto_upload"))
}

/// `clear_contribution_override` answered as the Mac decodes it
/// (`ContributionOverrideClearResult { cleared: Bool }`).
pub(crate) fn clear_confirmed(reply: &Result<Value, String>) -> bool {
    reply
        .as_ref()
        .is_ok_and(|v| v.get("cleared").is_some_and(Value::is_boolean))
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
    scopes.extend(
        options
            .iter()
            .filter(|o| o.always_on)
            .map(|o| o.name.clone()),
    );
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
    /// The folders `list_projects` last reported, for the Ask count;
    /// `None` when the last read did not answer.
    pub(crate) projects: RefCell<Option<Vec<Project>>>,
    pub(crate) working: Cell<bool>,
    filling: Cell<bool>,
    /// Whose grant a write belongs to (`GrantOwner`).
    pub(crate) owner: RefCell<GrantOwner>,
    /// The override in force, from the last status read; `None` for none.
    pub(crate) override_mode: RefCell<Option<String>>,
    /// Whether a status has been read, so the override is known.
    pub(crate) status_read: Cell<bool>,
    /// `status.contribution_mode_partial`.
    pub(crate) partial: Cell<bool>,
    /// What an override in force shows, and its clear.
    override_box: gtk::Box,
    override_value: gtk::Label,
    override_partial: gtk::Label,
    pub(crate) clear_override: gtk::Button,
    /// The clear confirmation is open.
    confirming: Cell<bool>,
}

impl SharingCard {
    pub fn new() -> Self {
        let uses = first_run_copy().uses;
        let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
        let grant = trace_commons_contributor::consent_copy::automatic_contribution_copy(None);
        let root = style::card(gtk::Orientation::Vertical, space::S);
        // Waits for first run (R-43) until a status says it is done.
        root.set_visible(cards_shown(None));
        root.append(&style::eyebrow(uses.sharing));
        let ask = gtk::CheckButton::with_label(uses.ask_each_time);
        let automatic = gtk::CheckButton::with_label(modes.choices[1].label);
        automatic.set_group(Some(&ask));
        root.append(&ask);
        if !grant.path_ask_first_detail.is_empty() && !grant.path_ask_first_title.is_empty() {
            style::append_caveat(&root, grant.path_ask_first_title);
        }
        root.append(&automatic);
        if let Some(line) = automatic_line(uses.automatic_scope, &grant) {
            style::append_caveat(&root, line);
        }
        // An override in force decides for every folder, so the radios
        // show nothing picked and this says what is in force, in the
        // contribution-mode table's words, with its clear.
        let override_box = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        override_box.set_visible(false);
        let override_head = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let override_title = style::body(modes.title);
        override_title.add_css_class("heading");
        override_head.append(&override_title);
        let override_value = style::body("");
        override_head.append(&override_value);
        override_box.append(&override_head);
        let override_partial = style::caveat(modes.auto_partial);
        override_partial.set_visible(false);
        override_box.append(&override_partial);
        style::append_caveat(&override_box, modes.override_active);
        let clear_override = gtk::Button::with_label(modes.clear);
        clear_override.set_halign(gtk::Align::Start);
        override_box.append(&clear_override);
        root.append(&override_box);
        if !uses.settings_sharing_note.is_empty() {
            style::append_caveat(&root, uses.settings_sharing_note);
        }
        let refusal = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .visible(false)
            .build();
        refusal.add_css_class("tc-refused");
        root.append(&refusal);
        let card = Self {
            root,
            ask,
            automatic,
            refusal,
            current: Cell::new(None),
            projects: RefCell::new(None),
            working: Cell::new(false),
            filling: Cell::new(false),
            owner: RefCell::new(GrantOwner::default()),
            override_mode: RefCell::new(None),
            status_read: Cell::new(false),
            partial: Cell::new(false),
            override_box,
            override_value,
            override_partial,
            clear_override,
            confirming: Cell::new(false),
        };
        card.render();
        card
    }

    /// Show what is in force; nothing picked and nothing pressable until
    /// the daemon has said.
    pub(crate) fn render(&self) {
        self.filling.set(true);
        let mode = self.override_mode.borrow().clone();
        let current = shown_path(self.current.get(), mode.as_deref());
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
        let modes = trace_commons_contributor::project_copy::contribution_mode_copy();
        match mode.as_deref() {
            Some(mode) => {
                let label = modes
                    .choices
                    .iter()
                    .find(|c| c.mode == mode)
                    .map_or("\u{2014}", |c| c.label);
                self.override_value.set_text(label);
                self.override_partial
                    .set_visible(mode == "auto_upload" && self.partial.get());
                self.override_box.set_visible(true);
            }
            None => self.override_box.set_visible(false),
        }
        self.clear_override.set_sensitive(
            mode.is_some()
                && self.status_read.get()
                && !self.working.get()
                && !self.confirming.get(),
        );
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
    card.clear_override.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            choose_clear(&app);
        }
    });
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

/// `list_projects` answered: the folders the Ask count and the override's
/// clear read. `None` when it did not answer.
pub(crate) fn note_projects(app: &Rc<App>, projects: Option<&[Project]>) {
    *app.settings.sharing.projects.borrow_mut() = projects.map(<[Project]>::to_vec);
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
            settle_owner(app, move |app, pressed| {
                let save_failed = first_run_copy().uses.settings_sharing_save_failed;
                let Some(pressed) = pressed else {
                    app.settings.sharing.refuse(save_failed);
                    set_working(app, false);
                    return;
                };
                app.call(method, params.clone(), move |app, result| {
                    // Applied only to the account it was pressed on.
                    let current = app.settings.sharing.owner.borrow().is_current(pressed);
                    if !current || !ask_each_time_confirmed(&result) {
                        app.settings.sharing.refuse(save_failed);
                    }
                    set_working(app, false);
                    app.refresh();
                });
            });
        }
        SharingPath::Automatic => match first_step(asking_folders(
            card.projects.borrow().as_deref().unwrap_or_default(),
        )) {
            FirstStep::KeepAsks(count) => keep_asks_question(app, count),
            FirstStep::Disclose { keep_folder_asks } => disclose_scrub(app, keep_folder_asks),
        },
    }
}

/// The Mac's `settleAutomaticGrantOwner`: a status that named an account
/// settles it; otherwise status is read first, and a failed read settles
/// nothing. `then` hears the generation the write is pressed under.
fn settle_owner(app: &Rc<App>, then: impl FnOnce(&Rc<App>, Option<u64>) + 'static) {
    if app.settings.sharing.owner.borrow().named() {
        let pressed = app.settings.sharing.owner.borrow().begin();
        then(app, pressed);
        return;
    }
    app.call("status", serde_json::json!({}), move |app, result| {
        let status = result
            .ok()
            .and_then(|v| serde_json::from_value::<crate::model::Status>(v).ok());
        let mut owner = app.settings.sharing.owner.borrow_mut();
        match status {
            Some(status) => owner.observe(status.account_scope.as_deref()),
            None => owner.forget(),
        }
        let pressed = owner.begin();
        drop(owner);
        then(app, pressed);
    });
}

/// "Use each folder's setting" (`MenuPanelStore.chooseMixed`): only on a
/// read status with an override in force and nothing else in flight. It
/// confirms first, in the core's words, when clearing would resume
/// unattended sending somewhere; without the core's confirmation nothing
/// is cleared.
fn choose_clear(app: &Rc<App>) {
    let card = &app.settings.sharing;
    let mode = card.override_mode.borrow().clone();
    if mode.is_none() || !card.status_read.get() || card.working.get() || card.confirming.get() {
        return;
    }
    card.refusal.set_visible(false);
    let needs = clear_needs_confirmation(card.projects.borrow().as_deref(), mode.as_deref());
    if !needs {
        write_clear(app);
        return;
    }
    let copy = trace_commons_contributor::project_copy::contribution_override_clear_confirm_copy();
    if copy.mode != trace_commons_contributor::project_copy::CONTRIBUTION_OVERRIDE_CLEAR_MODE {
        card.refuse(
            trace_commons_contributor::project_copy::contribution_override_refusal_line(""),
        );
        return;
    }
    card.confirming.set(true);
    card.render();
    let dialog = adw::MessageDialog::new(Some(&app.window), Some(copy.title), Some(copy.body));
    dialog.add_responses(&[("cancel", copy.cancel), ("confirm", copy.confirm)]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));
    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        app.settings.sharing.confirming.set(false);
        app.settings.sharing.render();
        if response == "confirm" {
            write_clear(&app);
        }
    });
    dialog.present();
}

/// Sends `clear_contribution_override`, then re-reads: the card shows what
/// the daemon says, never a guess. A refusal says the core's line.
fn write_clear(app: &Rc<App>) {
    set_working(app, true);
    app.call(
        "clear_contribution_override",
        serde_json::json!({}),
        |app, result| {
            if !clear_confirmed(&result) {
                let label = result.as_ref().err().cloned().unwrap_or_default();
                app.settings.sharing.refuse(
                    trace_commons_contributor::project_copy::contribution_override_refusal_line(
                        &label,
                    ),
                );
            }
            set_working(app, false);
            app.refresh();
        },
    );
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
    dialog.set_body(
        &copy
            .uses
            .keep_asks_body
            .replace("{count}", &count.to_string()),
    );
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
    dialog.add_responses(&[
        ("cancel", copy.frame.cancel),
        ("continue", copy.frame.continue_button),
    ]);
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
    app.call(
        "route_disclosure",
        serde_json::json!({}),
        move |app, result| {
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
        },
    );
}

/// Both disclosures seen: the core decides whether the grant may be asked
/// for and with which witness, from a fresh status, as at setup's Start.
fn grant(app: &Rc<App>, witness_shown: Option<String>, keep_folder_asks: bool) {
    app.call("status", serde_json::json!({}), move |app, result| {
        let Some(status) = result
            .ok()
            .and_then(|v| serde_json::from_value::<crate::model::Status>(v).ok())
        else {
            app.settings.sharing.owner.borrow_mut().forget();
            refused(app);
            return;
        };
        // This fresh read settles whose grant it is (`GrantOwner`).
        app.settings
            .sharing
            .owner
            .borrow_mut()
            .observe(status.account_scope.as_deref());
        let Some(pressed) = app.settings.sharing.owner.borrow().begin() else {
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
            // Applied only to the account it was pressed on.
            let granted = app.settings.sharing.owner.borrow().is_current(pressed)
                && result
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
        root.set_visible(cards_shown(None));
        // The Mac's `GlassEyebrowCard(consentEyebrow, title: consentHeading)`:
        // the eyebrow is drawn only when the core has one (the owner cut
        // it), and the heading is the card's title.
        if !shell.consent_eyebrow.is_empty() {
            root.append(&style::eyebrow(shell.consent_eyebrow));
        }
        let head = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        let heading = gtk::Label::builder()
            .label(words.consent_heading)
            .xalign(0.0)
            .wrap(true)
            .build();
        heading.add_css_class("tc-card-title");
        heading.set_hexpand(true);
        head.append(&heading);
        let refresh = gtk::Button::with_label(shell.settings_refresh);
        refresh.set_tooltip_text(Some(shell.settings_refresh_uses));
        refresh.update_property(&[gtk::accessible::Property::Label(
            shell.settings_refresh_uses,
        )]);
        head.append(&refresh);
        root.append(&head);
        style::append_caveat(&root, words.applies_from_now);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        root.append(&rows);
        let refusal = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .visible(false)
            .build();
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

/// `status` answered: the daemon's list and sign-in, the account whose
/// grant this is, the override in force, and whether first run is done.
pub(crate) fn note_status(app: &Rc<App>, status: &crate::model::Status) {
    *app.settings.uses.reported.borrow_mut() =
        Some((status.logged_in, status.consent_scopes.clone()));
    let card = &app.settings.sharing;
    card.owner
        .borrow_mut()
        .observe(status.account_scope.as_deref());
    *card.override_mode.borrow_mut() = status
        .contribution_override
        .as_ref()
        .and_then(override_mode);
    card.partial
        .set(status.contribution_mode_partial == Some(true));
    card.status_read.set(true);
    card.render();
    let shown = cards_shown(Some(super::onboarding::requires_onboarding(status)));
    card.root.set_visible(shown);
    app.settings.uses.root.set_visible(shown);
    render_uses(app);
}

/// A later `status` read failed: the Uses rows go back to unavailable, and
/// the override is no longer known, so nothing is pressable on it.
pub(crate) fn note_status_failed(app: &Rc<App>) {
    *app.settings.uses.reported.borrow_mut() = None;
    let card = &app.settings.sharing;
    card.status_read.set(false);
    card.render();
    render_uses(app);
}

pub(crate) fn render_uses(app: &Rc<App>) {
    let card = &app.settings.uses;
    let words = trace_commons_contributor::shell_words_copy::settings_words();
    while let Some(child) = card.rows.first_child() {
        card.rows.remove(&child);
    }
    let (logged_in, granted) = card
        .reported
        .borrow()
        .clone()
        .unwrap_or((false, Vec::new()));
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
    app.call(
        "set_consent_scopes",
        set_scopes_params(&scopes),
        |app, result| {
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
        },
    );
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

    /// The Automatic radio's line: the core's scope line, else the grant
    /// table's path title when its detail and scrub wording are there
    /// (`UsesScreenLayout.sharingOptions`), else none.
    #[test]
    fn the_automatic_radio_falls_back_to_the_path_title() {
        let mut grant = trace_commons_contributor::consent_copy::automatic_contribution_copy(None);
        assert_eq!(automatic_line("Scope.", &grant), Some("Scope."));
        assert!(!grant.path_automatic_title.is_empty());
        if grant.patterns_only.is_some() || grant.model_scrubbed.is_some() {
            assert_eq!(automatic_line("", &grant), Some(grant.path_automatic_title));
        }
        grant.path_automatic_detail = "";
        assert_eq!(automatic_line("", &grant), None);
    }

    /// The sharing and Uses cards wait for first run (R-43): shown only on
    /// a status that says setup is done, never on a guess.
    #[test]
    fn the_consent_cards_wait_for_first_run() {
        assert!(cards_shown(Some(false)));
        assert!(!cards_shown(Some(true)));
        assert!(!cards_shown(None));
    }

    /// The Mac's account-generation guard: a write applies only to the
    /// account that was current when the person pressed. A status with a
    /// different account scope starts a new generation; one with the same
    /// scope does not.
    #[test]
    fn a_grant_reply_applies_only_to_the_account_it_was_pressed_on() {
        let mut owner = GrantOwner::default();
        assert_eq!(owner.begin(), None, "no owner before a status read");
        owner.observe(Some("acct-a"));
        let pressed = owner.begin().expect("settled");
        owner.observe(Some("acct-a"));
        assert!(owner.is_current(pressed));
        owner.observe(Some("acct-b"));
        assert!(!owner.is_current(pressed));
        // A nil scope (an older core, watch-only) is still an owner, as the
        // Mac's published nil read is.
        let mut older = GrantOwner::default();
        older.observe(None);
        let pressed = older.begin().expect("a nil scope is settled");
        assert!(older.is_current(pressed));
        older.observe(Some("acct-a"));
        assert!(!older.is_current(pressed));
        // A failed status read establishes no owner.
        older.forget();
        assert_eq!(older.begin(), None);
    }

    /// While an override is in force the radios show nothing picked: the
    /// card must not say Ask each time while every folder sends.
    #[test]
    fn an_override_leaves_neither_radio_picked() {
        assert_eq!(
            shown_path(Some(SharingPath::AskMe), Some("auto_upload")),
            None
        );
        assert_eq!(
            shown_path(Some(SharingPath::Automatic), Some("ignore")),
            None
        );
        assert_eq!(
            shown_path(Some(SharingPath::AskMe), None),
            Some(SharingPath::AskMe)
        );
    }

    /// The override read from status: its mode, or none.
    #[test]
    fn the_override_is_read_from_status() {
        assert_eq!(
            override_mode(&json!({"mode": "auto_upload", "since": "x"})).as_deref(),
            Some("auto_upload")
        );
        assert_eq!(override_mode(&json!(null)), None);
        assert_eq!(override_mode(&json!({"mode": 3})), None);
    }

    /// Clearing confirms first unless the override is Automatic, and an
    /// unread folder list or a folder with no reported own setting needs
    /// the confirmation (`MenuPanelData.clearNeedsConfirmation`).
    #[test]
    fn clearing_the_override_confirms_as_the_mac_does() {
        let ask = project("notify_only", Some("notify_only"), true, false);
        let auto = project("auto_upload", Some("auto_upload"), true, false);
        let unknown = project("notify_only", None, true, false);
        assert!(!clear_needs_confirmation(None, Some("auto_upload")));
        assert!(clear_needs_confirmation(None, Some("notify_only")));
        assert!(!clear_needs_confirmation(
            Some(&[ask.clone()]),
            Some("ignore")
        ));
        assert!(clear_needs_confirmation(
            Some(&[ask.clone(), auto]),
            Some("ignore")
        ));
        assert!(clear_needs_confirmation(
            Some(&[ask, unknown]),
            Some("notify_only")
        ));
    }

    /// The clear reply as the Mac decodes it
    /// (`ContributionOverrideClearResult { cleared: Bool }`).
    #[test]
    fn a_clear_reply_is_checked_as_the_mac_checks_it() {
        assert!(clear_confirmed(&Ok(
            json!({"cleared": true, "returned": 2})
        )));
        assert!(clear_confirmed(&Ok(
            json!({"cleared": false, "returned": 0})
        )));
        assert!(!clear_confirmed(&Ok(json!({"cleared": "yes"}))));
        assert!(!clear_confirmed(&Ok(json!({}))));
        assert!(!clear_confirmed(&Err("x".into())));
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
        assert_eq!(
            first_step(0),
            FirstStep::Disclose {
                keep_folder_asks: true
            }
        );
    }

    #[test]
    fn ask_each_time_is_the_macs_withdrawal() {
        assert_eq!(
            crate::first_run::Action::SetAskEachTime.ipc(),
            Some((
                "withdraw_automatic_grant",
                json!({"return_folders_to_ask": true})
            ))
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
        let scrub = copy
            .patterns_only
            .clone()
            .or(copy.model_scrubbed.clone())
            .unwrap();
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
        let options = [
            scope("contribution", true),
            scope("training", false),
            scope("eval", false),
        ];
        let reported = vec!["eval".to_string()];
        assert_eq!(
            next_scopes(&reported, &options, &options[1], true),
            ["contribution", "eval", "training"]
                .map(String::from)
                .into()
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
        for bad in [
            json!({}),
            json!({"consent_scopes": "a"}),
            json!({"consent_scopes": [1]}),
        ] {
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
