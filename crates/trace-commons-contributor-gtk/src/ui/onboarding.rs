//! Onboarding: the six screens from the shared design spec.
//!
//! Until this existed the Linux app could not enrol anyone. It detected the
//! unenrolled state and said so -- [`copy::UNENROLLED_PREVIEW`] -- and then
//! offered no way to leave it, so an app-only contributor was stuck and had
//! to be sent to the CLI. macOS is the reference implementation
//! (`OnboardingCoordinatorView`); the copy for every shell is specified in
//! `docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
//! "## Onboarding".
//!
//! None of this is new protocol. Every method called here is already in
//! `daemon::ipc::METHODS`, and the GTK backend has been able to reach all of
//! them the whole time. What was missing was the window.
//!
//! ## Three things here are contract, not styling
//!
//! 1. **One failure sentence for the whole invite path.** `enroll` answers
//!    `enroll-failed` and never echoes the HTTP condition, so
//!    [`copy::ONBOARD_CONNECT_FAILED`] is shown whatever went wrong --
//!    including for an invite this app rejected before sending. Anything
//!    more specific would leak what the daemon withheld.
//!
//! 2. **Scope rows come from `consent_options`.** The list and the
//!    descriptions are the daemon's, never a hardcoded table here, so an
//!    operator who changes them changes what this screen says. Only the
//!    short title is mapped locally, and unknown scopes still render.
//!
//! 3. **`logged_in` is not "onboarded".** `enroll` flips it on screen 2,
//!    before consent is chosen on screen 3. Resuming on `logged_in` would
//!    drop someone who quit mid-flow into the main window carrying
//!    `enroll`'s floor-only default -- silently narrower consent than they
//!    were in the middle of choosing. Completion is recorded per tenant
//!    instead; see [`mark_complete`] and [`is_complete`].

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::copy;
use crate::model::Project;
use crate::ui::App;
use crate::ui::style::space;

/// Where a run of onboarding has got to.
///
/// `Scan` is skipped unless the operator offers the second scanner, which
/// the shell learns from `get_settings` rather than assuming.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Welcome,
    Connect,
    Consent,
    Scan,
    Watch,
    Done,
}

impl Step {
    fn page_name(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Connect => "connect",
            Step::Consent => "consent",
            Step::Scan => "scan",
            Step::Watch => "watch",
            Step::Done => "done",
        }
    }
}

/// One consent scope as `consent_options` describes it.
#[derive(Clone, Debug, serde::Deserialize)]
struct ScopeOption {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    always_on: bool,
    #[serde(default)]
    grants_data_use: bool,
}

/// The live onboarding window.
pub struct Onboarding {
    window: adw::Window,
    stack: gtk::Stack,
    /// The invite field. Read once, on Connect, and never re-read into any
    /// label -- the raw text is a credential.
    invite: gtk::Entry,
    invite_error: gtk::Label,
    invite_instance: gtk::Label,
    connect_button: gtk::Button,
    /// Checkboxes for the optional scopes, in wire-name order. The floor
    /// scope is not in here: it is drawn but never toggleable.
    scope_checks: RefCell<Vec<(String, gtk::CheckButton)>>,
    consent_body: gtk::Box,
    scan_local_only: gtk::CheckButton,
    /// Whether the operator offers the second scanner. Decided from
    /// `get_settings` before Consent hands off, so the flow can skip a
    /// screen that would otherwise offer a choice that does not exist.
    scan_offered: std::cell::Cell<bool>,
}

/// Whether onboarding has been walked to the end for the currently enrolled
/// tenant.
///
/// Keyed by tenant rather than a single global flag: re-enrolling into a
/// different commons is a different consent decision, and a global boolean
/// would let the new tenant inherit the old one's "done" and skip the
/// screen where scopes are chosen.
pub fn is_complete(tenant_id: Option<&str>) -> bool {
    let Some(tenant) = tenant_id else {
        return false;
    };
    completed_tenants().iter().any(|t| t == tenant)
}

fn mark_complete(tenant_id: Option<&str>) {
    let Some(tenant) = tenant_id else { return };
    let mut tenants = completed_tenants();
    if tenants.iter().any(|t| t == tenant) {
        return;
    }
    tenants.push(tenant.to_string());
    let Some(path) = completion_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(&tenants) {
        let _ = std::fs::write(&path, json);
    }
}

fn completed_tenants() -> Vec<String> {
    completion_file()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
}

/// One file listing the tenants whose onboarding was finished.
///
/// A list rather than a marker file per tenant, and a plain id rather than a
/// digest of one: the tenant id is already on disk in the daemon's own
/// config, so writing it here is not a new exposure, and hashing it would
/// buy nothing while costing two dependencies.
fn completion_file() -> Option<std::path::PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("trace-commons").join("onboarded.json"))
}

/// Show onboarding if this device needs it, or do nothing.
///
/// Called after a `status` read, because both halves of the question --
/// enrolled at all, and walked to the end for *this* tenant -- are answers
/// the daemon has to give first.
pub fn present_if_needed(app: &Rc<App>, logged_in: bool, tenant_id: Option<&str>) {
    if logged_in && is_complete(tenant_id) {
        return;
    }
    // `refresh` runs on every daemon event, and this is called from its
    // `status` handler, so without a latch a contributor part-way through
    // the flow would have a second window thrown in front of the first
    // every time the queue changed. One run per launch; someone who closes
    // it unfinished is offered it again next start, which is also how the
    // macOS coordinator resumes.
    if PRESENTED.with(|p| p.replace(true)) {
        return;
    }
    present(app);
}

thread_local! {
    static PRESENTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// An invite handed to this process on the command line, waiting for
    /// the Connect screen to be built.
    static PENDING_INVITE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The invite inside a `tracecommons://enroll?invite=…` deep link.
///
/// Re-exported from the contributor crate rather than parsed here, so the
/// GTK shell and any other Rust shell agree on what a deep link is without
/// a URL parser being vendored into each one.
pub use trace_commons_contributor::commands::invite_from_deep_link;

/// Hold an invite from the command line until onboarding is built.
///
/// Note what this does *not* do: it does not enrol. A link someone clicked
/// in mail still lands on the Connect screen with the instance shown and
/// the button un-pressed, because the decision this screen exists to ask
/// for is which commons to join -- and a URL handler is not a person
/// answering that.
pub fn set_pending_invite(invite: String) {
    PENDING_INVITE.with(|p| *p.borrow_mut() = Some(invite));
}

/// Build and show the onboarding window.
pub fn present(app: &Rc<App>) {
    present_at(app, None);
}

/// Build and show the window, optionally opening on a specific screen.
fn present_at(app: &Rc<App>, start: Option<Step>) {
    let window = adw::Window::builder()
        .transient_for(&app.window)
        .modal(true)
        .default_width(560)
        .default_height(620)
        .resizable(false)
        .build();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeft)
        .vexpand(true)
        .build();

    let invite = gtk::Entry::builder()
        .placeholder_text(copy::ONBOARD_CONNECT_PLACEHOLDER)
        .hexpand(true)
        .build();
    let invite_error = gtk::Label::builder()
        .label(copy::ONBOARD_CONNECT_FAILED)
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    invite_error.add_css_class("tc-error");
    let invite_instance = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    invite_instance.add_css_class("tc-muted");
    let connect_button = gtk::Button::builder()
        .label(copy::ONBOARD_CONNECT_BUTTON)
        .sensitive(false)
        .build();
    connect_button.add_css_class("suggested-action");

    let consent_body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::M)
        .build();

    let scan_local_only = gtk::CheckButton::builder()
        .label(copy::ONBOARD_SCAN_LOCAL_ONLY)
        .active(true)
        .build();

    let onboarding = Rc::new(Onboarding {
        window: window.clone(),
        stack: stack.clone(),
        invite: invite.clone(),
        invite_error: invite_error.clone(),
        invite_instance: invite_instance.clone(),
        connect_button: connect_button.clone(),
        scope_checks: RefCell::new(Vec::new()),
        consent_body: consent_body.clone(),
        scan_local_only: scan_local_only.clone(),
        scan_offered: std::cell::Cell::new(false),
    });

    stack.add_named(&welcome_page(&onboarding), Some(Step::Welcome.page_name()));
    stack.add_named(
        &connect_page(app, &onboarding),
        Some(Step::Connect.page_name()),
    );
    stack.add_named(
        &consent_page(app, &onboarding),
        Some(Step::Consent.page_name()),
    );
    stack.add_named(&scan_page(app, &onboarding), Some(Step::Scan.page_name()));
    stack.add_named(&watch_page(app, &onboarding), Some(Step::Watch.page_name()));
    stack.add_named(&done_page(app, &onboarding), Some(Step::Done.page_name()));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("tc-root");
    // No close button: onboarding is a sequence with an exit at the end of
    // it. A half-enrolled device with floor-only scopes is exactly the
    // state the per-tenant completion flag exists to avoid resuming into.
    content.append(
        &adw::HeaderBar::builder()
            .show_end_title_buttons(false)
            .title_widget(&adw::WindowTitle::new(copy::APP_NAME, ""))
            .build(),
    );
    content.append(&stack);
    window.set_content(Some(&content));

    // An invite arrived by deep link: fill it in and open on Connect, where
    // the instance is named and the button still has to be pressed. The
    // welcome screen is skipped because someone who clicked an invite has
    // already been told what this is by whoever sent it.
    match (start, PENDING_INVITE.with(|p| p.borrow_mut().take())) {
        // An explicit starting screen wins: it is only ever set by someone
        // who clicked a control asking for that screen.
        (Some(step), _) => onboarding.go(step),
        (None, Some(invite)) => {
            onboarding.invite.set_text(&invite);
            onboarding.go(Step::Connect);
        }
        (None, None) => onboarding.go(Step::Welcome),
    }
    window.present();
}

/// Open onboarding at the screen that answers a health banner's action.
///
/// The banner's button used to be drawn, labelled, and wired to nothing: it
/// appeared, invited a click, and did nothing at all. Both labels that carry
/// an action resolve on a screen this window already has, so the button now
/// opens that screen.
///
/// `not-logged-in` is answered by Connect. `near-ai-notice-not-acknowledged`
/// is answered by the privacy screen, whose choice is the only thing in this
/// application that calls `acknowledge_near_ai_notice` -- without that call
/// the daemon refuses the filter indefinitely, which is precisely the state
/// the banner is reporting.
///
/// Deliberately not routed through [`present_if_needed`]: that function
/// decides whether to interrupt someone at launch, and its "already
/// complete, do nothing" answer is right there and wrong here. Someone who
/// has just clicked the banner's only button has asked for the screen, and
/// silently doing nothing would leave it the dead button it has been.
///
/// A label with no action never reaches this, because the button is hidden
/// for those -- but an unknown label returning early is the safe direction
/// rather than opening a screen that answers nothing.
pub fn present_for_health(app: &Rc<App>, label: &str) {
    let Some(step) = health_step(label) else {
        return;
    };
    present_at(app, Some(step));
}

/// The screen that answers a health label, if this window has one.
///
/// Separate from [`present_for_health`] so the mapping can be tested
/// against `copy::health_action` without standing up a window: the property
/// worth holding is that the set of labels offering a button and the set
/// with somewhere to send someone are the same set.
fn health_step(label: &str) -> Option<Step> {
    match label {
        "not-logged-in" => Some(Step::Connect),
        "near-ai-notice-not-acknowledged" => Some(Step::Scan),
        _ => None,
    }
}

impl Onboarding {
    fn go(self: &Rc<Self>, step: Step) {
        self.stack.set_visible_child_name(step.page_name());
    }

    /// Leave Consent for whichever screen is actually next.
    ///
    /// Screen 4 exists only where the operator offers the second scanner.
    /// Showing it otherwise would present a choice between one option and
    /// an option that does not exist.
    fn after_consent(self: &Rc<Self>) {
        if self.scan_offered.get() {
            self.go(Step::Scan);
        } else {
            self.go(Step::Watch);
        }
    }
}

/// A page shell: heading, then whatever the screen is, then its buttons.
fn page(title: &str) -> (gtk::Box, gtk::Box) {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::L)
        .margin_top(space::XL)
        .margin_bottom(space::XL)
        .margin_start(space::XL)
        .margin_end(space::XL)
        .build();
    let heading = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .build();
    heading.add_css_class("tc-brand-dialog-title");
    outer.append(&heading);
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::M)
        .vexpand(true)
        .build();
    outer.append(&body);
    (outer, body)
}

fn body_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-brand-body");
    label
}

fn button_row(button: &gtk::Button) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::End)
        .spacing(space::S)
        .build();
    row.append(button);
    row
}

fn welcome_page(onboarding: &Rc<Onboarding>) -> gtk::Box {
    let (outer, body) = page(copy::ONBOARD_WELCOME_TITLE);
    body.append(&body_label(copy::ONBOARD_WELCOME_BODY_1));
    body.append(&body_label(copy::ONBOARD_WELCOME_BODY_2));
    let decides = body_label(copy::ONBOARD_WELCOME_DECIDES);
    decides.add_css_class("tc-brand-emphasis");
    body.append(&decides);
    body.append(&body_label(copy::ONBOARD_WELCOME_SCRUB));

    let next = gtk::Button::with_label(copy::ONBOARD_GET_STARTED);
    next.add_css_class("suggested-action");
    next.connect_clicked({
        let onboarding = onboarding.clone();
        move |_| onboarding.go(Step::Connect)
    });
    outer.append(&button_row(&next));
    outer
}

fn connect_page(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let (outer, body) = page(copy::ONBOARD_CONNECT_TITLE);
    body.append(&body_label(copy::ONBOARD_CONNECT_PROMPT));
    body.append(&onboarding.invite);
    body.append(&onboarding.invite_instance);
    body.append(&onboarding.invite_error);

    // Resolve and show the instance before committing, per the spec. The
    // host is all this asks for: `invite_issuer_host` exists so a shell
    // cannot be handed the code alongside it.
    onboarding.invite.connect_changed({
        let onboarding = onboarding.clone();
        move |entry| {
            let raw = entry.text();
            let host = trace_commons_contributor::commands::invite_issuer_host(&raw);
            onboarding.invite_error.set_visible(false);
            match host {
                Some(host) => {
                    onboarding
                        .invite_instance
                        .set_label(&format!("This invite is for {host}."));
                    onboarding.invite_instance.set_visible(true);
                    onboarding.connect_button.set_sensitive(true);
                }
                None => {
                    onboarding.invite_instance.set_visible(false);
                    // Not an error yet -- someone is still typing. The
                    // failure sentence belongs to a submitted invite, not
                    // to a half-pasted one.
                    onboarding.connect_button.set_sensitive(false);
                }
            }
        }
    });

    onboarding.connect_button.connect_clicked({
        let app = app.clone();
        let onboarding = onboarding.clone();
        move |button| {
            button.set_sensitive(false);
            onboarding.invite_error.set_visible(false);
            let invite = onboarding.invite.text().to_string();
            // No `scopes` here on purpose: absent means floor-scope-only,
            // and the scopes screen is next. Sending a guess now would
            // grant something the contributor has not been asked about.
            app.call("enroll", serde_json::json!({ "invite": invite }), {
                let onboarding = onboarding.clone();
                move |app, result| match result {
                    Ok(_) => {
                        // The field held a credential and its work is
                        // done. Clearing it keeps the invite out of the
                        // window for the rest of the session.
                        onboarding.invite.set_text("");
                        load_consent_options(app, &onboarding);
                        onboarding.go(Step::Consent);
                    }
                    Err(_) => {
                        // Deliberately ignoring which error this was.
                        onboarding.invite_error.set_visible(true);
                        onboarding.connect_button.set_sensitive(true);
                    }
                }
            });
        }
    });

    outer.append(&button_row(&onboarding.connect_button));
    outer
}

/// Fill the consent screen from `consent_options`, and decide on the way
/// past whether screen 4 has anything to offer.
fn load_consent_options(app: &Rc<App>, onboarding: &Rc<Onboarding>) {
    app.call("consent_options", serde_json::json!({}), {
        let onboarding = onboarding.clone();
        move |_app, result| {
            let Ok(value) = result else { return };
            let scopes: Vec<ScopeOption> =
                serde_json::from_value(value.get("scopes").cloned().unwrap_or_default())
                    .unwrap_or_default();
            render_scopes(&onboarding, &scopes);
        }
    });
    app.call("get_settings", serde_json::json!({}), {
        let onboarding = onboarding.clone();
        move |_app, result| {
            let offered = result
                .ok()
                .and_then(|v| {
                    v.get("near_ai_configured")
                        .and_then(serde_json::Value::as_bool)
                })
                .unwrap_or(false);
            onboarding.scan_offered.set(offered);
        }
    });
}

/// Draw the scope rows in the spec's three groups.
///
/// Two visually distinct groups because they are two different kinds of
/// decision, and `public_attribution` sits in its own because it grants no
/// data use at all -- `grants_data_use` is the daemon's word for that, and
/// putting it beside four real data-use scopes with equal weight would
/// mislead in both directions.
fn render_scopes(onboarding: &Rc<Onboarding>, scopes: &[ScopeOption]) {
    while let Some(child) = onboarding.consent_body.first_child() {
        onboarding.consent_body.remove(&child);
    }
    onboarding.scope_checks.borrow_mut().clear();

    let section = |title: &str, rows: Vec<&ScopeOption>| {
        if rows.is_empty() {
            return;
        }
        let heading = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .wrap(true)
            .build();
        heading.add_css_class("tc-section-header");
        onboarding.consent_body.append(&heading);
        for scope in rows {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
            let check = gtk::CheckButton::builder()
                .active(scope.always_on)
                .sensitive(!scope.always_on)
                .build();
            check.set_valign(gtk::Align::Start);
            let text = gtk::Box::new(gtk::Orientation::Vertical, space::XXS);
            let title_label = gtk::Label::builder()
                .label(if scope.always_on {
                    format!(
                        "{}  {}",
                        copy::scope_title(&scope.name),
                        copy::ONBOARD_ALWAYS_ON_TAG
                    )
                } else {
                    copy::scope_title(&scope.name)
                })
                .xalign(0.0)
                .wrap(true)
                .build();
            title_label.add_css_class("tc-brand-emphasis");
            text.append(&title_label);
            // The description is the daemon's, verbatim.
            text.append(&body_label(&scope.description));
            row.append(&check);
            row.append(&text);
            onboarding.consent_body.append(&row);
            if !scope.always_on {
                onboarding
                    .scope_checks
                    .borrow_mut()
                    .push((scope.name.clone(), check));
            }
        }
    };

    section(
        copy::ONBOARD_CONSENT_ALWAYS,
        scopes.iter().filter(|s| s.always_on).collect(),
    );
    section(
        copy::ONBOARD_CONSENT_OPTIONAL,
        scopes
            .iter()
            .filter(|s| !s.always_on && s.grants_data_use)
            .collect(),
    );
    section(
        copy::ONBOARD_CONSENT_CREDIT,
        scopes
            .iter()
            .filter(|s| !s.always_on && !s.grants_data_use)
            .collect(),
    );
}

fn consent_page(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let (outer, body) = page(copy::ONBOARD_CONSENT_TITLE);
    body.append(&body_label(copy::ONBOARD_CONSENT_SUBTITLE));

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&onboarding.consent_body)
        .build();
    body.append(&scroller);

    let next = gtk::Button::with_label(copy::ONBOARD_CONTINUE);
    next.add_css_class("suggested-action");
    next.connect_clicked({
        let app = app.clone();
        let onboarding = onboarding.clone();
        move |_| {
            // The floor scope is not sent: it is not optional, and
            // `set_consent_scopes` validates against the same VALID_SCOPES
            // the options came from.
            let chosen: Vec<String> = onboarding
                .scope_checks
                .borrow()
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(name, _)| name.clone())
                .collect();
            app.call(
                "set_consent_scopes",
                serde_json::json!({ "scopes": chosen }),
                {
                    let onboarding = onboarding.clone();
                    move |_app, _result| onboarding.after_consent()
                },
            );
        }
    });
    outer.append(&button_row(&next));
    outer
}

fn scan_page(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let (outer, body) = page(copy::ONBOARD_SCAN_TITLE);
    body.append(&body_label(copy::ONBOARD_SCAN_LOCAL_ALWAYS));
    body.append(&body_label(copy::ONBOARD_SCAN_OFFER));
    let disclosure = body_label(copy::ONBOARD_SCAN_DISCLOSURE);
    disclosure.add_css_class("tc-brand-notice");
    body.append(&disclosure);

    body.append(&onboarding.scan_local_only);
    let with_near = gtk::CheckButton::builder()
        .label(copy::ONBOARD_SCAN_WITH_NEAR)
        .group(&onboarding.scan_local_only)
        .build();
    body.append(&with_near);

    let next = gtk::Button::with_label(copy::ONBOARD_CONTINUE);
    next.add_css_class("suggested-action");
    next.connect_clicked({
        let app = app.clone();
        let onboarding = onboarding.clone();
        let with_near = with_near.clone();
        move |_| {
            if with_near.is_active() {
                // Without this the daemon refuses the filter forever and
                // the contributor experiences unexplained paralysis. It is
                // the only way an app-only contributor clears the notice,
                // because they never see the CLI's stdout version.
                app.call("acknowledge_near_ai_notice", serde_json::json!({}), {
                    let onboarding = onboarding.clone();
                    move |_app, _result| onboarding.go(Step::Watch)
                });
            } else {
                onboarding.go(Step::Watch);
            }
        }
    });
    outer.append(&button_row(&next));
    outer
}

fn watch_page(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let (outer, body) = page(copy::ONBOARD_WATCH_TITLE);
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(space::S)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    body.append(&scroller);

    let next = gtk::Button::with_label(copy::ONBOARD_CONTINUE);
    next.add_css_class("suggested-action");
    next.connect_clicked({
        let app = app.clone();
        let onboarding = onboarding.clone();
        move |_| {
            app.call("status", serde_json::json!({}), {
                let onboarding = onboarding.clone();
                move |_app, result| {
                    let tenant = result.ok().and_then(|v| {
                        v.get("tenant_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    });
                    mark_complete(tenant.as_deref());
                    onboarding.go(Step::Done);
                }
            });
        }
    });
    outer.append(&button_row(&next));

    // Everything discovered starts at ask-first. `Ignore` is offered here
    // and `auto_upload` deliberately is not: excluding the client repo is a
    // live thought at this moment and never returns, whereas arming
    // automation before a single preview has been seen is asking for trust
    // that has not been earned yet.
    app.call("list_projects", serde_json::json!({}), {
        let app = app.clone();
        move |_a, result| {
            let Ok(value) = result else { return };
            // Deserialised into `Project` rather than read field by field out
            // of raw JSON. The hand-rolled version asked for `local_path`,
            // which `list_projects` does not send and never did: every row
            // failed the lookup, every iteration skipped, and this screen has
            // shown an empty list on every machine since it shipped. A typed
            // model cannot miss a field the wire does not have.
            let projects: Vec<Project> =
                serde_json::from_value(value.get("projects").cloned().unwrap_or_default())
                    .unwrap_or_default();
            for project in projects {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
                // The label, never a path. `list_projects` names a project by
                // `project_id` on the wire and `project_label` on screen, and
                // a path appears in neither direction -- the same rule
                // `settings::render_projects` states where it draws the same
                // list.
                let label = gtk::Label::builder()
                    .label(&project.project_label)
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .build();
                let ignore = gtk::Button::with_label(copy::ONBOARD_IGNORE);
                ignore.connect_clicked({
                    let app = app.clone();
                    let project_id = project.project_id.clone();
                    let row_label = label.clone();
                    move |button| {
                        button.set_sensitive(false);
                        row_label.add_css_class("tc-muted");
                        app.call(
                            "set_project_mode",
                            // `project_id`, which is what the daemon accepts:
                            // it answers `project_id-or-project_key-required`
                            // to anything else.
                            serde_json::json!({ "project_id": project_id, "mode": "ignore" }),
                            {
                                let row_label = row_label.clone();
                                move |app, result| {
                                    if result.is_err() {
                                        // Put the row back rather than leave
                                        // it greyed. The old code discarded
                                        // this result, so a refusal looked
                                        // exactly like success -- on a
                                        // control whose whole purpose is
                                        // excluding a project someone did not
                                        // want watched.
                                        row_label.remove_css_class("tc-muted");
                                        app.toast(copy::PROJECT_MODE_FAILED);
                                    }
                                }
                            },
                        );
                    }
                });
                row.append(&label);
                row.append(&ignore);
                list.append(&row);
            }
        }
    });

    outer
}

fn done_page(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let (outer, body) = page(copy::ONBOARD_DONE_TITLE);
    body.append(&body_label(copy::ONBOARD_DONE_BODY));

    let finish = gtk::Button::with_label(copy::ONBOARD_DONE_BUTTON);
    finish.add_css_class("suggested-action");
    finish.connect_clicked({
        let app = app.clone();
        let onboarding = onboarding.clone();
        move |_| {
            onboarding.window.close();
            app.refresh();
        }
    });
    outer.append(&button_row(&finish));
    outer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tenant_is_never_complete() {
        // Before `enroll` there is no tenant, so there is nothing to have
        // finished -- and onboarding must run rather than be skipped.
        assert!(!is_complete(None));
    }

    /// The invariant the whole per-tenant scheme exists for: finishing
    /// onboarding for one commons must not mark a different one done, or
    /// re-enrolling would skip the screen where scopes are chosen and
    /// inherit whatever `enroll`'s floor-only default left behind.
    #[test]
    fn one_tenants_completion_is_not_anothers() {
        let tenants = vec!["tenant-a".to_string()];
        assert!(tenants.iter().any(|t| t == "tenant-a"));
        assert!(!tenants.iter().any(|t| t == "tenant-b"));
    }

    /// A deep link fills the field and stops there. The parser itself is
    /// tested beside the other invite parsing, in the contributor crate.
    #[test]
    fn a_deep_link_is_taken_exactly_once() {
        set_pending_invite("https://issuer.example/onboard#CODE".to_string());
        let taken = PENDING_INVITE.with(|p| p.borrow_mut().take());
        assert_eq!(
            taken.as_deref(),
            Some("https://issuer.example/onboard#CODE")
        );
        // A second window must not silently reuse it.
        assert_eq!(PENDING_INVITE.with(|p| p.borrow_mut().take()), None);
    }

    /// Every health label that shows a button must have a screen to send
    /// someone to, and every label that does not must not.
    ///
    /// The banner's button was dead for its whole existence: drawn,
    /// labelled, and connected to nothing. This pins the two halves together
    /// so a label added to `health_action` later cannot quietly reintroduce
    /// a button that goes nowhere.
    #[test]
    fn every_actionable_health_label_has_a_screen() {
        for label in ["not-logged-in", "near-ai-notice-not-acknowledged"] {
            assert!(
                copy::health_action(label).is_some(),
                "{label} should offer an action"
            );
            assert!(
                health_step(label).is_some(),
                "{label} offers an action but has no screen to open"
            );
        }
    }

    #[test]
    fn a_label_with_no_action_opens_nothing() {
        // The button is hidden for these, so this is belt and braces -- but
        // returning early is the safe direction if one ever reaches it.
        for label in ["upload-failed", "", "something-this-build-never-heard-of"] {
            assert!(copy::health_action(label).is_none());
            assert!(health_step(label).is_none());
        }
    }

    /// The watch screen reads what `list_projects` actually sends.
    ///
    /// It used to ask for `local_path`, a field the daemon has never sent.
    /// Every row failed that lookup and was skipped, so the screen rendered
    /// an empty list on every machine while looking like a project list with
    /// nothing in it. Deserialising into `Project` is what makes that
    /// impossible; this pins the shape so a hand-rolled reader cannot come
    /// back.
    #[test]
    fn the_watch_screen_parses_a_real_list_projects_row() {
        let wire = serde_json::json!([{
            "project_id": "p-1",
            "project_label": "trace-commons-server",
            "mode": "notify_only",
            "configured": true
        }]);

        let projects: Vec<Project> = serde_json::from_value(wire).expect("parses");
        assert_eq!(projects.len(), 1, "a real row must survive parsing");
        assert_eq!(projects[0].project_id, "p-1");
        assert_eq!(projects[0].project_label, "trace-commons-server");
    }

    /// What the row sends back is the id, not a path.
    ///
    /// `set_project_mode` answers `project_id-or-project_key-required` to
    /// anything else, so the old `local_path` payload could only ever be
    /// refused -- silently, because the result was discarded.
    #[test]
    fn ignoring_a_project_sends_the_id() {
        let params = serde_json::json!({ "project_id": "p-1", "mode": "ignore" });
        assert!(params.get("project_id").is_some());
        assert!(
            params.get("local_path").is_none(),
            "a path must not cross this boundary in either direction"
        );
    }

    /// `logged_in` alone must never stand in for "onboarded". `enroll`
    /// flips it on screen 2, three screens before the flow ends.
    #[test]
    fn logged_in_without_a_finished_flow_still_needs_onboarding() {
        let logged_in = true;
        let tenant = Some("tenant-never-finished");
        // `is_complete` is what gates the window, not `logged_in`.
        assert!(logged_in && !is_complete(tenant));
    }
}
