//! Joining a commons with the NEAR AI login a contributor already has.
//!
//! **The way in that needs no wallet.** A contributor cannot produce an
//! admissible receipt without a NEAR AI account in the first place, because
//! the receipt comes from their own inference calls, so requiring a wallet as
//! well is a second onboarding for an identity they already hold. Both paths
//! are drawn on this screen and neither replaces the other.
//!
//! **Nothing here authors a sentence.** The offer, the working line, the
//! done line and all ten refusals come from
//! `trace_commons_contributor::private_inference_copy`, which macOS and
//! Windows reach across the C ABI as `tc_near_ai_enroll_*`. There is no
//! `match` on a control name in this shell and there must not be.
//!
//! **Not signed in is not a refusal.** The daemon checks the session before
//! it reaches out, and reports `near_ai_enroll_no_session` rather than
//! blaming the commons, so a contributor who has simply never signed in is
//! told to sign in. This module keeps that distinction: it reads the
//! credential state first and, with no session, draws the step to take rather
//! than a control that can only fail.

use crate::copy;
use crate::ui::onboarding::{Onboarding, Step, body_label, load_consent_options};
use crate::ui::{App, credential};
use adw::prelude::*;
use std::{cell::Cell, rc::Rc};

#[path = "onboarding_nearai_result.rs"]
mod enrollment_result;

/// The card's own state: whether a NEAR AI session exists and whether a join
/// is in flight.
struct NearAiJoin {
    root: gtk::Box,
    commons: gtk::Entry,
    join: gtk::Button,
    message: gtk::Label,
    sign_in: gtk::Button,
    sign_in_explains: gtk::Label,
    sign_in_action: Cell<copy::CredentialAction>,
    polling: Cell<bool>,
    reading_status: Cell<bool>,
    /// Whether `near_ai_credential_status` last reported a usable session.
    ///
    /// False until the first read, which is the answer that claims less: a
    /// card that has not been told yet must not offer a control whose only
    /// possible outcome is `near_ai_enroll_no_session`.
    signed_in: Cell<bool>,
    pending: Cell<bool>,
}

impl NearAiJoin {
    /// One sentence and one control, both decided by the two facts above.
    fn refresh(&self) {
        let payload = trace_commons_contributor::private_inference_copy::private_inference_copy();
        let busy = self.pending.get();
        self.commons.set_sensitive(!busy);
        // Offered only where it can succeed. Without a session the control is
        // absent rather than present-and-refusing, and the sentence beneath
        // says what to do instead.
        self.join.set_visible(self.signed_in.get());
        self.join
            .set_sensitive(!busy && !self.commons.text().trim().is_empty());
        if busy {
            self.message.remove_css_class("tc-refused");
            self.message.set_label(payload.near_ai_enroll_working);
        } else if !self.signed_in.get() {
            self.message.remove_css_class("tc-refused");
            self.message.set_label(payload.near_ai_enroll_needs_login);
        }
    }

    /// What the daemon said, in its own words.
    fn report(&self, label: &str, refused: bool) {
        self.pending.set(false);
        if refused {
            self.message.set_label(copy::near_ai_enroll_line(label));
            match copy::near_ai_enroll_tone(label) {
                copy::PrivateInferenceTone::Refused => self.message.add_css_class("tc-refused"),
                _ => self.message.remove_css_class("tc-refused"),
            }
        } else {
            self.message.remove_css_class("tc-refused");
            self.message.set_label(
                trace_commons_contributor::private_inference_copy::private_inference_copy()
                    .near_ai_enroll_done,
            );
        }
        self.refresh();
    }
}

pub(super) fn build(app: &Rc<App>, onboarding: &Rc<Onboarding>) -> gtk::Box {
    let payload = trace_commons_contributor::private_inference_copy::private_inference_copy();
    let card = Rc::new(NearAiJoin {
        root: gtk::Box::new(gtk::Orientation::Vertical, 12),
        commons: gtk::Entry::builder()
            .placeholder_text(
                trace_commons_contributor::witness_copy::witness_copy()
                    .wallet
                    .commons,
            )
            .build(),
        join: gtk::Button::with_label(payload.near_ai_enroll_action),
        message: body_label(""),
        sign_in: gtk::Button::new(),
        sign_in_explains: body_label(""),
        sign_in_action: Cell::new(copy::CredentialAction::None),
        polling: Cell::new(false),
        reading_status: Cell::new(false),
        signed_in: Cell::new(false),
        pending: Cell::new(false),
    });
    card.root.append(&body_label(payload.near_ai_enroll_title));
    card.root.append(&body_label(payload.near_ai_enroll_what));
    card.root.append(&card.commons);
    card.root.append(&card.join);
    card.root.append(&card.message);
    card.root.append(&card.sign_in_explains);
    card.root.append(&card.sign_in);

    let sign_in_card = card.clone();
    let sign_in_app = app.clone();
    card.sign_in.connect_clicked(move |button| {
        button.set_sensitive(false);
        credential::act(&sign_in_app, sign_in_card.sign_in_action.get());
    });

    let entry_card = card.clone();
    card.commons.connect_changed(move |_| entry_card.refresh());

    let click_card = card.clone();
    let click_app = app.clone();
    let click_onboarding = onboarding.clone();
    card.join.connect_clicked(move |_| {
        if click_card.pending.get() || click_onboarding.connection_busy.get() {
            return;
        }
        click_card.pending.set(true);
        click_onboarding.connection_busy.set(true);
        click_card.refresh();
        let params = serde_json::json!({ "ingest_url": click_card.commons.text().as_str() });
        let result_card = click_card.clone();
        let result_onboarding = click_onboarding.clone();
        click_app.call("near_ai_account_enroll", params, move |app, result| {
            enrollment_result::complete(
                result,
                &result_onboarding.connection_busy,
                |label, refused| result_card.report(label, refused),
                || {
                    result_card.commons.set_text("");
                    result_onboarding.invite.set_text("");
                    load_consent_options(app, &result_onboarding);
                    result_onboarding.go(Step::Consent);
                },
            );
        });
    });

    // Read on every appearance and while visible: the browser and the other
    // credential surface can both change the retained session.
    let mapped_card = Rc::downgrade(&card);
    let mapped_app = Rc::downgrade(app);
    card.root.connect_map(move |_| {
        let (Some(app), Some(card)) = (mapped_app.upgrade(), mapped_card.upgrade()) else {
            return;
        };
        if card.polling.replace(true) {
            return;
        }
        refresh_sign_in(&app, &card);
        let weak_card = Rc::downgrade(&card);
        let weak_app = Rc::downgrade(&app);
        gtk::glib::timeout_add_seconds_local(2, move || {
            let (Some(app), Some(card)) = (weak_app.upgrade(), weak_card.upgrade()) else {
                return gtk::glib::ControlFlow::Break;
            };
            if !card.root.is_mapped() {
                card.polling.set(false);
                return gtk::glib::ControlFlow::Break;
            }
            refresh_sign_in(&app, &card);
            gtk::glib::ControlFlow::Continue
        });
    });

    card.refresh();
    card.root.clone()
}

fn refresh_sign_in(app: &Rc<App>, card: &Rc<NearAiJoin>) {
    if card.reading_status.replace(true) {
        return;
    }
    let status_card = card.clone();
    app.call(
        "near_ai_credential_status",
        serde_json::json!({}),
        move |_, result| {
            status_card.reading_status.set(false);
            let value = result.ok();
            let state = value
                .as_ref()
                .and_then(|v| v.get("session_state"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let present = state
                == trace_commons_contributor::daemon::nearai_credential::LABEL_CREDENTIAL_PRESENT;
            let action = copy::credential_action(state);
            status_card.sign_in_action.set(action);
            status_card
                .sign_in
                .set_visible(!present && credential::action_label(action).is_some());
            status_card
                .sign_in
                .set_label(credential::action_label(action).unwrap_or_default());
            status_card
                .sign_in
                .set_sensitive(!status_card.pending.get());
            status_card.sign_in_explains.set_label(if present {
                ""
            } else {
                credential::action_explains(action)
            });
            status_card.signed_in.set(present);
            status_card.refresh();
        },
    );
}

#[cfg(test)]
mod tests {
    /// This shell asks the shared table and spells no refusal of its own.
    ///
    /// Ten control names reach ten sentences in
    /// `private_inference_copy`; a `match` here would be an eleventh table
    /// that agrees until it does not. The sweep reads this module's own
    /// source from outside, as `eligibility.rs` does over the queue.
    #[test]
    fn the_card_asks_the_shared_table_and_names_no_refusal() {
        const SOURCE: &str = include_str!("onboarding_nearai.rs");
        let production: String = SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests")
            .lines()
            // Prose stripped first: these comments name the control names on
            // purpose, and a guard that failed them would teach the next
            // reader to delete the explanation.
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            production.contains("copy::near_ai_enroll_line("),
            "the card does not ask for the shared sentence"
        );
        assert!(
            production.contains("copy::near_ai_enroll_tone("),
            "the card does not ask for the shared tone"
        );

        // Not one control name spelled in this shell. The daemon's label
        // goes straight through.
        for label in [
            "near_ai_enroll_no_session",
            "near_ai_enroll_commons_unreachable",
            "near_ai_enroll_commons_unsupported",
            "near_ai_enroll_already_enrolled",
        ] {
            assert!(
                !production.contains(label),
                "{label} is branched on in this shell rather than passed through"
            );
        }
    }

    /// The control is withheld until a session exists, and the step is said.
    ///
    /// Offering "Join with NEAR AI" to somebody who has not signed in gives
    /// them a button whose only outcome is `near_ai_enroll_no_session`. The
    /// daemon deliberately reports that rather than blaming the commons, and
    /// throwing that distinction away in the shell would undo the fix.
    #[test]
    fn without_a_session_the_step_is_offered_rather_than_a_control_that_can_only_fail() {
        const SOURCE: &str = include_str!("onboarding_nearai.rs");
        let production: String = SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            production.contains("self.join.set_visible(self.signed_in.get())"),
            "the join control is not withheld from a contributor with no session"
        );
        assert!(
            production.contains("near_ai_enroll_needs_login"),
            "a contributor with no session is shown no step to take"
        );
        assert!(
            production.contains("signed_in: Cell::new(false)"),
            "the card assumes a session before it has been told of one"
        );
    }
}
