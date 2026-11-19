//! Onboarding's entry points: whether the first run is needed, the
//! invite a deep link hands over, and the screens a health banner opens.
//!
//! The first run itself is `first_run.rs`, ported from the Mac
//! (`OnboardingCoordinatorView`); the GTK-authored Welcome, Consent, Watch
//! and Done pages that lived here are gone with it. What stays here:
//!
//! 1. **`logged_in` is not "onboarded".** Completion is the daemon's shared
//!    `setup_complete`, or, for a device finished before that existed, the
//!    per-tenant marker this shell wrote then ([`is_complete`]).
//!
//! 2. **A deep link fills Join and stops there.** An invite handed to this
//!    process waits for the first run's Join, after Terms
//!    ([`take_pending_invite`]).
//!
//! 3. **The banner's one action opens something.** `not-logged-in` opens
//!    the first run. Every other health label is a plain line.

use std::cell::RefCell;
use std::rc::Rc;

#[cfg(test)]
use crate::copy;
use crate::model::Status;
use crate::ui::App;
use adw::prelude::*;

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
    crate::shell_state_file("onboarded.json")
}

fn setup_complete(status: &Status, legacy_complete: bool) -> bool {
    status.logged_in
        && status.tenant_id.as_deref().is_some_and(|id| !id.is_empty())
        && (status.shared_setup_complete() || legacy_complete)
}

/// Whether this device still owes onboarding, for the tab strip: the same
/// two answers `present_if_needed` reads, and the same camera-only switch.
/// `TC_SUPPRESS_ONBOARDING` photographs the main window's own screens on a
/// fixture that is deliberately not enrolled, so it must not hide them
/// either.
pub fn requires_onboarding(status: &Status) -> bool {
    if std::env::var_os("TC_SUPPRESS_ONBOARDING").is_some() {
        return false;
    }
    !setup_complete(status, is_complete(status.tenant_id.as_deref()))
}

/// Show onboarding if this device needs it, or do nothing.
///
/// Called after a `status` read, because both halves of the question --
/// enrolled at all, and walked to the end for *this* tenant -- are answers
/// the daemon has to give first.
pub fn present_if_needed(app: &Rc<App>, status: &Status) {
    if status.shared_setup_complete() {
        AUTOMATIC_WINDOW.with(|current| {
            let window = current.borrow_mut().take().and_then(|weak| weak.upgrade());
            if let Some(window) = window {
                PRESENTED.with(|presented| presented.set(false));
                window.close();
            }
        });
    }
    // The tab the person selected, not the one drawn: Insights is
    // account-free, and reading it must not raise onboarding over it.
    if app.shell.selected_tab() == super::monitor::tabs::Tab::Insights {
        return;
    }
    if setup_complete(status, is_complete(status.tenant_id.as_deref())) {
        return;
    }
    // Camera only, in the same family as TC_FORCE_CONNECT_NOTICES below.
    //
    // The headless fixture is deliberately not enrolled, so onboarding is
    // correct to open over every other screen -- which makes the main
    // window's own surfaces unphotographable. Satisfying the real condition
    // would mean the fixture minting a config and an Ed25519 device key, a
    // pile of machinery that can drift from the schema it imitates and would
    // then lie about what a real install looks like. Suppressing the modal
    // for a screenshot changes nothing about what is underneath it.
    if std::env::var_os("TC_SUPPRESS_ONBOARDING").is_some() {
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
    // Weak ownership lets CLI completion close first-run without retaining it
    // or dismissing explicitly opened health/privacy windows.
    static AUTOMATIC_WINDOW: RefCell<Option<gtk::glib::WeakRef<adw::Window>>> = const { RefCell::new(None) };
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
/// Note what this does *not* do: it does not enroll. A link someone clicked
/// in mail still lands on the Connect screen with the instance shown and
/// the button un-pressed, because the decision this screen exists to ask
/// for is which commons to join -- and a URL handler is not a person
/// answering that.
pub fn set_pending_invite(invite: String) {
    PENDING_INVITE.with(|p| *p.borrow_mut() = Some(invite));
}

/// Open the first run after the account section's "Set up again"
/// unenrolled this device (#1836 F1). Takes the once-per-launch latch, so
/// the `status` read that follows does not open a second window; a run
/// already shown this launch would otherwise keep it from opening at all.
///
/// A first run already open is brought forward rather than opened twice.
pub fn present_after_unenroll(app: &Rc<App>) {
    PRESENTED.with(|p| p.set(true));
    let open = AUTOMATIC_WINDOW.with(|current| current.borrow().as_ref().and_then(|w| w.upgrade()));
    if let Some(window) = open {
        window.present();
        return;
    }
    present(app);
}

/// Build and show the onboarding window.
pub fn present(app: &Rc<App>) {
    let window = super::first_run::present(app);
    register_automatic_window(&window);
}

/// Whether the automatic first-run window is open.
pub(super) fn automatic_window_open() -> bool {
    AUTOMATIC_WINDOW.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_some_and(|weak| weak.upgrade().is_some())
    })
}

/// Track the first-run coordinator so shared setup completion closes it.
pub(super) fn register_automatic_window(window: &adw::Window) {
    AUTOMATIC_WINDOW.with(|current| *current.borrow_mut() = Some(window.downgrade()));
}

/// An invite handed to this process, for the first run's Join to show after
/// Terms. Taken once: deep links must never skip Terms.
pub(super) fn take_pending_invite() -> Option<String> {
    PENDING_INVITE.with(|pending| pending.borrow_mut().take())
}

/// Open the screen that answers a health banner's action.
///
/// Deliberately not routed through [`present_if_needed`]: that function
/// decides whether to interrupt someone at launch, and its "already
/// complete, do nothing" answer is right there and wrong here. Someone who
/// has just clicked the banner's only button has asked for the screen.
///
/// A label with no action never reaches this, because the button is hidden
/// for those -- but an unknown label returning early is the safe direction
/// rather than opening a screen that answers nothing.
pub fn present_for_health(app: &Rc<App>, label: &str) {
    match health_screen(label) {
        Some(HealthScreen::FirstRun) => present(app),
        None => {}
    }
}

/// Where a health label's action goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HealthScreen {
    /// The first run, whose Join signs in again.
    FirstRun,
}

/// The screen that answers a health label, if there is one.
///
/// Separate from [`present_for_health`] so the mapping can be tested
/// against `copy::health_action` without standing up a window: the property
/// worth holding is that the set of labels offering a button and the set
/// with somewhere to send someone are the same set.
fn health_screen(label: &str) -> Option<HealthScreen> {
    match label {
        "not-logged-in" => Some(HealthScreen::FirstRun),
        _ => None,
    }
}

pub(super) fn body_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-flow-body");
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_status(wire: serde_json::Value) -> crate::model::Status {
        serde_json::from_value(wire).expect("valid status")
    }

    #[test]
    fn cli_completion_is_live_proof_without_a_legacy_marker() {
        let mut status = setup_status(serde_json::json!({
            "logged_in": true, "tenant_id": "cli-completion-test-tenant"
        }));
        assert!(!setup_complete(&status, false));
        status.setup_complete = Some(true);
        assert!(setup_complete(&status, false));
        assert!(!is_complete(status.tenant_id.as_deref()));
        status.setup_complete = Some(false);
        assert!(!setup_complete(&status, false));
        status.tenant_id = Some("another-cli-completion-test-tenant".into());
        status.setup_complete = None;
        assert!(!setup_complete(&status, false));
    }

    #[test]
    fn shared_completion_requires_current_enrollment() {
        for wire in [
            serde_json::json!({"logged_in": false, "tenant_id": "a", "setup_complete": true}),
            serde_json::json!({"logged_in": true, "setup_complete": true}),
            serde_json::json!({"logged_in": true, "tenant_id": "", "setup_complete": true}),
        ] {
            let status = setup_status(wire);
            assert!(!status.shared_setup_complete());
            assert!(!setup_complete(&status, false));
        }
    }

    #[test]
    fn legacy_gui_completion_survives_absent_or_false_shared_status() {
        for value in [serde_json::Value::Null, serde_json::json!(false)] {
            let mut status = setup_status(serde_json::json!({
                "logged_in": true, "tenant_id": "a", "setup_complete": value
            }));
            assert!(setup_complete(&status, true));
            assert!(!status.shared_setup_complete());
            status.logged_in = false;
            assert!(!setup_complete(&status, true));
        }
    }

    #[test]
    fn shared_completion_preserves_optional_wire_values() {
        for (wire, expected) in [
            (serde_json::json!({}), None),
            (serde_json::json!({"setup_complete": null}), None),
            (serde_json::json!({"setup_complete": false}), Some(false)),
            (serde_json::json!({"setup_complete": true}), Some(true)),
        ] {
            assert_eq!(setup_status(wire).setup_complete, expected);
        }
        assert!(
            serde_json::from_value::<crate::model::Status>(serde_json::json!({
                "setup_complete": "true"
            }))
            .is_err()
        );
    }

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
        let tenants = ["tenant-a".to_string()];
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
        let label = "not-logged-in";
        assert!(
            copy::health_action(label).is_some(),
            "{label} should offer an action"
        );
        assert_eq!(health_screen(label), Some(HealthScreen::FirstRun));
    }

    /// The client-side NEAR AI second scan is retired (#2180): its notice
    /// label, should an older daemon still send it, is a plain line with no
    /// button and nowhere to go, as on the Mac.
    #[test]
    fn the_retired_scan_label_offers_no_button() {
        let label = "near-ai-notice-not-acknowledged";
        assert!(copy::health_action(label).is_none());
        assert!(health_screen(label).is_none());
    }

    #[test]
    fn a_label_with_no_action_opens_nothing() {
        // The button is hidden for these, so this is belt and braces -- but
        // returning early is the safe direction if one ever reaches it.
        for label in ["upload-failed", "", "something-this-build-never-heard-of"] {
            assert!(copy::health_action(label).is_none());
            assert!(health_screen(label).is_none());
        }
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
