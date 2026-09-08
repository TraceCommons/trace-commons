//! The key this computer answers model calls with: whether one is kept here,
//! and the one thing a contributor may do about it.
//!
//! It sits on the model-calls screen, under the tools, because the fact it
//! reports is the one that decides whether a tool can be connected at all.
//! The daemon has been able to obtain a key since #715 and to pick a fresh
//! one up mid-run since #718; until this screen existed no shell offered a
//! way to ask for one, so the whole mechanism was reachable only by a person
//! who knew the IPC method's name.
//!
//! **Every sentence is read from `copy`**, which re-exports
//! `trace_commons_contributor::private_inference_copy` -- the same definition
//! the macOS and Windows shells reach across the C ABI. This shell links the
//! crate, so it calls the same functions directly and there is no shim in
//! between.
//!
//! # Render, do not decide
//!
//! The state label arrives from `near_ai_credential_status` and is handed
//! straight to three shared functions that take it: `credential_state_line`
//! for the sentence, `credential_state_tone` for the colour, and
//! `credential_action` for the one control that may be drawn. Nothing here
//! matches on a state label. The arm that matters is the one nobody would
//! think to test: a label this build has never heard of, and a daemon that
//! reports none at all, each answer `CredentialAction::None`, because the
//! control this screen would otherwise draw MINTS A KEY at a third party and
//! a contributor who already has one would end up with two.
//!
//! # Nothing is offered without the sentence that qualifies it
//!
//! [`action_explains`] is keyed on the action and not on the state, and
//! [`render`] draws it in the one place it draws the button. `Obtain` cannot
//! appear without the sentence saying a browser will open, that the sign-in
//! is with a company that is not this app, and that a key is minted and kept
//! here; `Forget` cannot appear without the sentence saying that forgetting
//! is local and the key stays valid until the contributor removes it in their
//! own account.
//!
//! # Never a value
//!
//! No key, no key prefix, no attempt id and no account name is drawn. The
//! attempt id is held so a poll can name the attempt it started and so a
//! cancel has something to cancel; it never reaches a widget.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::style::{self, space};
use super::{App, private_inference};
use crate::copy;
use crate::copy::CredentialAction;

/// How often an open ceremony is asked about while the browser is out.
///
/// The daemon does not push this state, and the window otherwise redraws only
/// on a daemon event -- so without a poll the `Cancel` control would sit under
/// a sentence that stopped being true the moment the sign-in finished.
const POLL_SECONDS: u32 = 2;

/// The section's widgets. Built once and refilled, like every other screen in
/// this window.
pub struct CredentialView {
    pub root: gtk::Box,
    /// One toned sentence, rebuilt on every read.
    status: gtk::Box,
    /// The qualifying sentence and the one control, rebuilt on every read.
    /// Emptied first, so a state that offers nothing leaves nothing behind.
    action: gtk::Box,
    /// The ceremony this shell started, so a poll can name it and a cancel
    /// has something to cancel. NEVER DRAWN.
    ///
    /// `near_ai_credential_status` echoes the attempt only to a caller that
    /// already knew its id, so this cannot be recovered by asking: a shell
    /// that did not start the ceremony holds `None` for it and says so by
    /// leaving the cancel control insensitive rather than sending a call the
    /// daemon would refuse.
    attempt: RefCell<Option<String>>,
    /// Whether a poll is already running, so a second read does not start a
    /// second one.
    polling: Cell<bool>,
    /// Set while a call this section made is in flight. The controls mint or
    /// remove a key, and a second press before the first came back would ask
    /// for a second ceremony.
    pending: Cell<bool>,
}

impl Default for CredentialView {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialView {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        root.append(&style::section(copy::CREDENTIAL_TITLE));
        style::append_body(&root, copy::CREDENTIAL_WHAT);

        let card = style::card(gtk::Orientation::Vertical, space::M);
        // Drawn before anything has been asked, so the section never opens on
        // a blank space where the state will be. Unreported is what is true at
        // that moment, and the shared table has a sentence for it.
        let status = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        status.append(&private_inference::tone_row(
            copy::credential_state_line(""),
            private_inference::indicator_tone(copy::credential_state_tone("")),
        ));
        card.append(&status);
        let action = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        card.append(&action);
        root.append(&card);

        Self {
            root,
            status,
            action,
            attempt: RefCell::new(None),
            polling: Cell::new(false),
            pending: Cell::new(false),
        }
    }
}

/// The button's words for one action, or `None` where there is no button.
///
/// **No catch-all arm, deliberately.** An action this shell has not been
/// taught must fail to compile rather than fall through to a control that
/// does something. [`CredentialAction::None`] is the state nobody could read,
/// and the whole point of it is that nothing is offered there.
pub(super) fn action_label(action: CredentialAction) -> Option<&'static str> {
    match action {
        CredentialAction::None => None,
        CredentialAction::Obtain => Some(copy::CREDENTIAL_OBTAIN),
        CredentialAction::Cancel => Some(copy::CREDENTIAL_CANCEL),
        CredentialAction::Forget => Some(copy::CREDENTIAL_FORGET),
    }
}

/// The sentence that has to be on screen for one action to be offered, or the
/// empty string where the action needs no qualifying.
///
/// **Keyed on the action, not on the state, and read in the one place the
/// button is built.** That is what makes the pairing structural: there is no
/// arrangement of daemon states that draws `Obtain` without the sentence
/// saying what pressing it costs, because the two come out of one match on
/// one value.
///
/// `Cancel` is the empty string and not an oversight. It stops a ceremony
/// this contributor started moments ago and takes nothing away; a sentence
/// there would be this shell narrating a button.
pub(super) fn action_explains(action: CredentialAction) -> &'static str {
    match action {
        CredentialAction::None | CredentialAction::Cancel => "",
        // A browser opens, the sign-in is with a company that is not this
        // app, and a key is minted and kept here. Nobody presses this without
        // having read that.
        CredentialAction::Obtain => copy::CREDENTIAL_COST,
        // Forgetting is local. The key stays valid until the contributor
        // removes it in their own account, and a shell that implied otherwise
        // would leave a live key nobody is watching.
        CredentialAction::Forget => copy::CREDENTIAL_FORGET_EXPLAINS,
    }
}

/// Ask the daemon what it holds, and draw that.
///
/// Read rather than latched: the key can be minted, or removed, without this
/// process doing anything -- the ceremony finishes in a browser -- and a
/// latched answer would keep claiming the old one.
pub fn refresh(app: &Rc<App>) {
    // The attempt is named only when this shell started one. A caller that
    // names no attempt still gets `state`, which is the part this section
    // renders; `attempt_status` is the daemon's answer about the ceremony and
    // is echoed only to a caller that already knew the id.
    let params = match app.private_inference.credential.attempt.borrow().as_deref() {
        Some(id) => serde_json::json!({ "attempt_id": id }),
        None => serde_json::json!({}),
    };
    app.call("near_ai_credential_status", params, |app, result| {
        // A read that did not come back says nothing. The state on screen is
        // the last one that was true, and replacing it with a guess is the
        // one move this section may not make.
        let Ok(value) = result else { return };
        let state = value
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        render(app, state);
    });
}

/// One sentence, one tone, and at most one control.
///
/// `state` is `near_ai_credential_status`'s `state` -- never its
/// `attempt_status`, which is the ceremony's own lifecycle word and answers a
/// different question. An absent field arrives here as the empty string, and
/// the shared table renders that as unreported, separately from a nonempty
/// label this build does not know.
pub fn render(app: &Rc<App>, state: &str) {
    let view = &app.private_inference.credential;
    while let Some(child) = view.status.first_child() {
        view.status.remove(&child);
    }
    view.status.append(&private_inference::tone_row(
        copy::credential_state_line(state),
        private_inference::indicator_tone(copy::credential_state_tone(state)),
    ));

    let action = copy::credential_action(state);
    // The ceremony this shell started is over the moment nothing can be
    // cancelled, and the id stops being worth holding.
    if action != CredentialAction::Cancel {
        view.attempt.replace(None);
        view.polling.set(false);
    }

    while let Some(child) = view.action.first_child() {
        view.action.remove(&child);
    }
    if let Some(label) = action_label(action) {
        // Before the button, because it is what somebody reads before
        // pressing it. Empty draws nothing at all rather than a blank line.
        let explains = action_explains(action);
        if !explains.is_empty() {
            style::append_body(&view.action, explains);
        }
        let button = gtk::Button::with_label(label);
        button.set_halign(gtk::Align::Start);
        // A cancel needs the id of the ceremony to cancel, and only the shell
        // that started one holds it. Rather than send a call the daemon would
        // refuse, the control says by being unpressable that this window is
        // not the one that opened the browser.
        let addressable = action != CredentialAction::Cancel || view.attempt.borrow().is_some();
        button.set_sensitive(addressable && !view.pending.get());
        let app = Rc::clone(app);
        button.connect_clicked(move |_| act(&app, action));
        view.action.append(&button);
    }

    if action == CredentialAction::Cancel {
        ensure_poll(app);
    }
}

/// Perform the one action this state offers.
///
/// A single entry point, on the value the shared table produced, so there is
/// no path from a state label to a call other than through
/// `credential_action`. [`CredentialAction::None`] reaches here only if a
/// button was drawn for it, which [`action_label`] does not do.
fn act(app: &Rc<App>, action: CredentialAction) {
    let view = &app.private_inference.credential;
    if view.pending.replace(true) {
        return;
    }
    match action {
        CredentialAction::None => {
            view.pending.set(false);
        }
        CredentialAction::Obtain => start(app),
        CredentialAction::Cancel => cancel(app),
        CredentialAction::Forget => forget(app),
    }
}

/// Open the sign-in page, and say whether anything took it.
///
/// A URL GLib will not parse, and one no installed application claims, both
/// come back as an error -- and both mean the same thing to the caller: no tab
/// opened anywhere. The distinction is not worth keeping, because the answer
/// to either is the same.
fn open_browser(url: &str) -> bool {
    gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>).is_ok()
}

/// Start the ceremony and open the browser it minted a URL for.
///
/// **The URL is served once, by `start`, and no poll re-serves it.** It is
/// used here and not stored: a link kept on screen after the attempt it
/// belongs to has finished is an invitation to sign in a second time.
///
/// **A ceremony nobody can finish is cancelled here, not left to time out.**
/// If the browser did not open -- the launch failed, the URL would not parse,
/// or the daemon sent none at all -- there is no tab anywhere for anybody to
/// sign in to, and the alternative is a contributor watching this section say
/// a sign-in is under way until the daemon's own five-minute timeout expires.
/// On this shell that is worse than on the others: the control beside that
/// sentence is Cancel, and a window that lost the attempt id draws it
/// unpressable. The attempt id is in hand at exactly this moment, so this is
/// the one place the cancel is certain to be addressable. macOS and Windows do
/// the same thing for the same reason.
///
/// Every other outcome ends in a read. There is no shared sentence for a start
/// that did not start, and inventing one in this shell is the thing this whole
/// surface is written against -- so the answer to a refusal is to ask the
/// daemon what is true and draw that.
fn start(app: &Rc<App>) {
    app.call(
        "near_ai_credential_start",
        serde_json::json!({}),
        |app, result| {
            let view = &app.private_inference.credential;
            let opened = match result {
                Ok(value) => {
                    if let Some(id) = value.get("attempt_id").and_then(serde_json::Value::as_str) {
                        view.attempt.replace(Some(id.to_string()));
                    }
                    value
                        .get("browser_url")
                        .and_then(serde_json::Value::as_str)
                        .filter(|url| !url.is_empty())
                        .is_some_and(open_browser)
                }
                Err(_) => false,
            };
            if !opened && view.attempt.borrow().is_some() {
                // `pending` stays set on purpose: the cancel that follows is
                // the same press still being answered, and clears it itself.
                cancel(app);
                return;
            }
            view.pending.set(false);
            refresh(app);
        },
    );
}

/// Stop waiting on the browser.
///
/// The attempt id is required by the method and is the one this shell started.
/// Whatever comes back, the state is read again rather than assumed: a cancel
/// that raced a sign-in which had already completed leaves a key here, and
/// this section would otherwise be claiming there is none.
fn cancel(app: &Rc<App>) {
    let attempt = app
        .private_inference
        .credential
        .attempt
        .borrow()
        .as_deref()
        .map(str::to_string);
    let Some(attempt) = attempt else {
        app.private_inference.credential.pending.set(false);
        return;
    };
    app.call(
        "near_ai_credential_cancel",
        serde_json::json!({ "attempt_id": attempt }),
        |app, _result| {
            app.private_inference.credential.pending.set(false);
            refresh(app);
        },
    );
}

/// Remove the stored key from this machine.
///
/// `revoked` comes back false and is not drawn: the key stays valid at the
/// service until the contributor removes it there, which this cannot do for
/// them. That is what `CREDENTIAL_FORGET_EXPLAINS` says, and it was on screen
/// beside the control that got pressed.
///
/// The tool list is read again afterwards because the same fact gates it: on
/// a daemon that reports `destination_credentialed`, forgetting the key takes
/// every connect control off the list, and a stale list would keep offering
/// them.
fn forget(app: &Rc<App>) {
    app.call(
        "near_ai_credential_forget",
        serde_json::json!({}),
        |app, _result| {
            app.private_inference.credential.pending.set(false);
            refresh(app);
            private_inference::render_harnesses(app);
        },
    );
}

/// Keep asking while a ceremony is open.
///
/// Started from [`render`] on the one state that has an outstanding ceremony,
/// and stopped by [`render`] the moment the state is anything else -- so the
/// poll's lifetime is the ceremony's, and neither is decided here.
fn ensure_poll(app: &Rc<App>) {
    if app.private_inference.credential.polling.replace(true) {
        return;
    }
    let app = Rc::clone(app);
    glib::timeout_add_seconds_local(POLL_SECONDS, move || {
        if !app.private_inference.credential.polling.get() {
            return glib::ControlFlow::Break;
        }
        refresh(&app);
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::private_inference_copy::{
        LABEL_CREDENTIAL_ABSENT, LABEL_CREDENTIAL_CANCELLED, LABEL_CREDENTIAL_FAILED,
        LABEL_CREDENTIAL_OBTAINING, LABEL_CREDENTIAL_PRESENT, private_inference_copy,
    };

    const SOURCE: &str = include_str!("credential.rs");

    /// Every state a daemon can report, plus the two ways of reporting none.
    fn every_label() -> Vec<&'static str> {
        vec![
            LABEL_CREDENTIAL_ABSENT,
            LABEL_CREDENTIAL_OBTAINING,
            LABEL_CREDENTIAL_FAILED,
            LABEL_CREDENTIAL_CANCELLED,
            LABEL_CREDENTIAL_PRESENT,
            "",
            "a_credential_state_from_a_later_daemon",
        ]
    }

    /// The module's own code, without the tests that quote it.
    fn code() -> &'static str {
        SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
    }

    /// The tone comes from the shared table, and this shell only carries it
    /// onto the palette.
    ///
    /// The words could not drift -- they are re-exported -- but the branching
    /// could, in three shells, and nothing in this repository would notice.
    /// So this reads the render path's own source: it asks
    /// `credential_state_tone` for the tone, and it does not spell a single
    /// state label anywhere.
    #[test]
    fn the_credential_tone_and_action_are_not_branched_on_in_this_shell() {
        let code = code();
        assert!(
            code.contains("copy::credential_state_tone(state)"),
            "the tone stopped coming from the shared table"
        );
        assert!(
            code.contains("copy::credential_state_line(state)"),
            "the sentence stopped coming from the shared table"
        );
        assert!(
            code.contains("copy::credential_action(state)"),
            "the action stopped coming from the shared table"
        );
        for spelled in [
            LABEL_CREDENTIAL_ABSENT,
            LABEL_CREDENTIAL_OBTAINING,
            LABEL_CREDENTIAL_FAILED,
            LABEL_CREDENTIAL_CANCELLED,
            LABEL_CREDENTIAL_PRESENT,
        ] {
            assert!(
                !code.contains(&format!("\"{spelled}\"")),
                "the state is branched on in this shell: {spelled}"
            );
        }
    }

    /// A state this build cannot read borrows nothing: not a sentence, not
    /// the working light, and above all not a control.
    ///
    /// The dangerous arm is `Obtain`. A daemon that answered with a label
    /// from a later release, or one that answered with nothing, would leave a
    /// contributor who already holds a key looking at a button that mints a
    /// second one.
    #[test]
    fn an_unread_credential_state_borrows_no_sentence_and_offers_nothing() {
        let shared = private_inference_copy();
        for unread in ["", "a_credential_state_from_a_later_daemon"] {
            let line = copy::credential_state_line(unread);
            for known in [
                shared.credential_absent,
                shared.credential_obtaining,
                shared.credential_failed,
                shared.credential_cancelled,
                shared.credential_present,
            ] {
                assert_ne!(line, known, "{unread:?} borrowed a known sentence");
            }
            assert_ne!(
                private_inference::indicator_tone(copy::credential_state_tone(unread)),
                super::super::style::Tone::Clear,
                "{unread:?} is painted as working"
            );
            let action = copy::credential_action(unread);
            assert_eq!(action, CredentialAction::None, "{unread:?}");
            assert_eq!(action_label(action), None, "{unread:?} drew a control");
        }
        // The two ways of reporting nothing are two different facts, and the
        // sentences say different things: one is a read that failed and the
        // other is a build that was never asked.
        assert_ne!(
            copy::credential_state_line(""),
            copy::credential_state_line("a_credential_state_from_a_later_daemon")
        );
    }

    /// A key kept here is the only state painted as working.
    #[test]
    fn only_a_key_that_is_here_is_painted_as_working() {
        let tone =
            |label: &str| private_inference::indicator_tone(copy::credential_state_tone(label));
        assert_eq!(
            tone(LABEL_CREDENTIAL_PRESENT),
            super::super::style::Tone::Clear
        );
        for label in every_label() {
            if label == LABEL_CREDENTIAL_PRESENT {
                continue;
            }
            assert_ne!(
                tone(label),
                super::super::style::Tone::Clear,
                "{label:?} must not be painted as working"
            );
        }
        // Work under way is held, never settled, and a failure is refused
        // rather than quietly neutral.
        assert_eq!(
            tone(LABEL_CREDENTIAL_OBTAINING),
            super::super::style::Tone::Held
        );
        assert_eq!(
            tone(LABEL_CREDENTIAL_FAILED),
            super::super::style::Tone::Refused
        );
    }

    /// One control per state, and it is the shared table's control.
    #[test]
    fn each_state_offers_the_action_the_shared_table_names() {
        assert_eq!(
            action_label(copy::credential_action(LABEL_CREDENTIAL_ABSENT)),
            Some(copy::CREDENTIAL_OBTAIN)
        );
        assert_eq!(
            action_label(copy::credential_action(LABEL_CREDENTIAL_FAILED)),
            Some(copy::CREDENTIAL_OBTAIN)
        );
        assert_eq!(
            action_label(copy::credential_action(LABEL_CREDENTIAL_CANCELLED)),
            Some(copy::CREDENTIAL_OBTAIN)
        );
        assert_eq!(
            action_label(copy::credential_action(LABEL_CREDENTIAL_OBTAINING)),
            Some(copy::CREDENTIAL_CANCEL)
        );
        assert_eq!(
            action_label(copy::credential_action(LABEL_CREDENTIAL_PRESENT)),
            Some(copy::CREDENTIAL_FORGET)
        );
    }

    /// The two controls that cost something cannot be drawn without the
    /// sentence that says what.
    ///
    /// Structural rather than incidental: both come out of one match on the
    /// action, and `render` reads `action_explains` in the one place it builds
    /// a button. There is no state that reaches the button by another route.
    #[test]
    fn an_offer_never_arrives_without_the_sentence_that_qualifies_it() {
        assert_eq!(
            action_explains(CredentialAction::Obtain),
            copy::CREDENTIAL_COST
        );
        assert_eq!(
            action_explains(CredentialAction::Forget),
            copy::CREDENTIAL_FORGET_EXPLAINS
        );
        assert_eq!(action_explains(CredentialAction::Cancel), "");
        assert_eq!(action_explains(CredentialAction::None), "");
        // Every state that offers Obtain therefore gets the cost sentence,
        // and every state that offers Forget gets the local-only one.
        for label in every_label() {
            let action = copy::credential_action(label);
            if action == CredentialAction::Obtain {
                assert_eq!(action_explains(action), copy::CREDENTIAL_COST, "{label:?}");
            }
            if action == CredentialAction::Forget {
                assert_eq!(
                    action_explains(action),
                    copy::CREDENTIAL_FORGET_EXPLAINS,
                    "{label:?}"
                );
            }
        }
        let body = code()
            .split("pub fn render(")
            .nth(1)
            .expect("render is in this file");
        assert_eq!(
            body.matches("action_explains(action)").count(),
            1,
            "the qualifying sentence is read somewhere other than beside the button"
        );
        assert!(
            body.split("gtk::Button::with_label(label)")
                .next()
                .expect("something precedes the button")
                .contains("action_explains(action)"),
            "the button is built before the sentence that qualifies it"
        );
    }

    /// A sign-in nobody can finish is cancelled, not left to time out.
    ///
    /// The daemon gives a ceremony five minutes. With no tab open anywhere,
    /// every one of those minutes is spent telling a contributor that a
    /// sign-in is under way, beside a Cancel control -- and on a window that
    /// does not hold the attempt id, an unpressable one. The id is in hand at
    /// this exact moment, which is what makes cancelling here reliable.
    ///
    /// Read from the source because the failure is a control-flow one: the
    /// shape that regressed on this shell was a `.ok()` discarding the launch
    /// result, which compiles, renders identically, and differs only in what
    /// happens over the following five minutes.
    #[test]
    fn a_browser_that_did_not_open_cancels_the_attempt_it_belongs_to() {
        let body = code()
            .split("fn start(app: &Rc<App>) {")
            .nth(1)
            .expect("start is in this file")
            .split("\n}\n")
            .next()
            .expect("start closes");
        assert!(
            body.contains("if !opened && view.attempt.borrow().is_some() {"),
            "a failed launch no longer reaches the cancel"
        );
        assert!(
            body.contains("cancel(app);"),
            "the failed launch does not cancel the attempt"
        );
        // The regression this replaces, in the words it was written in. A
        // launch whose result is discarded cannot reach the branch above.
        assert!(
            !body.contains(".ok();"),
            "the launch result is discarded rather than acted on"
        );
        // Absent, empty, and unopenable are one outcome, because the answer
        // to all three is the same: nobody is going to finish this.
        assert!(
            body.contains("is_some_and(open_browser)"),
            "the launch outcome stopped deciding whether the attempt stands"
        );
        assert!(
            body.contains("filter(|url| !url.is_empty())"),
            "an empty browser URL is treated as a browser that opened"
        );
    }

    /// A URL this shell cannot open is a browser that did not open.
    ///
    /// Run rather than scanned: `open_browser` is the one function whose
    /// answer the branch above turns on, and a version of it that returned
    /// `true` unconditionally would satisfy every source check here while
    /// leaving the timeout to expire. A string GLib will not parse as a URI
    /// reaches no launcher and spawns nothing.
    #[test]
    fn a_url_that_cannot_be_opened_reports_that_it_was_not() {
        assert!(!open_browser(":::not a uri"));
        assert!(!open_browser(""));
    }

    /// Nothing on this section is authored here. Every string literal in the
    /// module's own code is a JSON key, an IPC method name, or a state label
    /// this shell reads rather than a sentence it writes.
    #[test]
    fn the_section_authors_no_sentence_of_its_own() {
        // Comments are stripped BEFORE splitting on quotes: a comment quoting
        // a sentence otherwise reads as a literal, and one carrying an odd
        // number of quotes shifts the parity so every real literal lands in a
        // skipped position and this passes having examined nothing.
        let code_only: String = code()
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let found: Vec<&str> = code_only.split('"').skip(1).step_by(2).collect();
        assert!(
            !found.is_empty(),
            "the scan found no literals at all -- has the module moved?"
        );
        for literal in found {
            assert!(
                !literal.contains(' '),
                "{literal:?} is a sentence written in this view rather than read from copy"
            );
        }
    }

    /// No value ever reaches a widget.
    ///
    /// The attempt id is held so a poll can name its ceremony and a cancel has
    /// something to cancel. The browser URL is used once and never stored. A
    /// key is never read at all -- the daemon does not send one.
    #[test]
    fn no_value_is_drawn() {
        let code = code();
        for value in ["attempt_id", "browser_url", "api_key", "account"] {
            for widget in [
                &format!("Label::new(Some({value}"),
                &format!("with_label({value}"),
                &format!("set_label({value}"),
            ] {
                assert!(!code.contains(widget.as_str()), "{value} reaches a widget");
            }
        }
        assert_eq!(
            code.matches("browser_url").count(),
            1,
            "the browser URL is read more than once -- it is served once and used once"
        );
    }
}
