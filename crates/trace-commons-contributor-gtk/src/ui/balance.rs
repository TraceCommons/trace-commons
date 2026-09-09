//! What is left in the Private AI account, on the screen that describes it.
//!
//! Beside the sign-in it belongs to, because it is a fact about that
//! account: the key kept here is what the daemon spends with, and what
//! remains is the other half of the same sentence. The daemon has read this
//! since #745 and no shell drew it, so a contributor could sign in, spend
//! against the account all week, and never be told what was left.
//!
//! **Every sentence is read from `copy`**, which re-exports
//! `trace_commons_contributor::private_inference_copy` -- the same
//! definition the macOS and Windows shells reach across the C ABI as
//! `tc_near_ai_balance_*`. This shell links the crate, so it calls the same
//! functions directly and there is no shim in between. Nothing here formats
//! a dollar, and nothing here matches on a state label:
//! [`crate::balance::view`] does the reading, and it decides nothing either.
//!
//! # Nothing on this row judges an amount
//!
//! The tone says the READ SUCCEEDED and never that the balance is healthy.
//! A low figure is painted exactly as a large one is, because a threshold
//! would be this shell's invention on an account whose ceiling may not
//! exist. There is no arm here that could paint a number red.
//!
//! # The `known` row is figures, and they carry it
//!
//! `balance_state_line` answers the EMPTY STRING for a read that succeeded,
//! deliberately: a sentence above the figures announcing that the read
//! worked is this app narrating itself. So [`render`] promotes the remaining
//! sentence into the toned row there, and the row reads as a balance rather
//! than as a glyph over a blank line with a number under it.
//!
//! # One control, and it is the sign-in row's
//!
//! `balance_action` answers `Obtain` on `no_session` and `session_expired`
//! and on nothing else, and the press goes to [`super::credential::act`] --
//! the same function the sign-in row's own button calls, so there is one
//! ceremony mechanism in this shell and not two. A refused session gets
//! `Obtain` WITHOUT a forget first: the ceremony overwrites both records,
//! and forgetting would throw away a working key to fix an unrelated
//! sign-in.

use std::rc::Rc;

use adw::prelude::*;

use super::style::{self, space};
use super::{App, private_inference};
use crate::copy;

/// The section's widgets. Built once and refilled, like every other screen
/// in this window.
pub struct BalanceSection {
    pub root: gtk::Box,
    /// The toned row and the figures under it, rebuilt on every read.
    status: gtk::Box,
    /// The one control and the sentence that qualifies it, rebuilt on every
    /// read. Emptied first, so a state that offers nothing leaves nothing
    /// behind.
    action: gtk::Box,
}

impl Default for BalanceSection {
    fn default() -> Self {
        Self::new()
    }
}

impl BalanceSection {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        root.append(&style::section(copy::BALANCE_TITLE));

        let card = style::card(gtk::Orientation::Vertical, space::M);
        // Drawn before anything has been asked, so the section never opens
        // on a blank space where the balance will be. Unreported is what is
        // true at that moment, and the shared table has a sentence for it.
        let status = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        status.append(&private_inference::tone_row(
            copy::balance_state_line(""),
            private_inference::indicator_tone(copy::balance_state_tone("")),
        ));
        card.append(&status);
        let action = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        card.append(&action);
        root.append(&card);

        Self {
            root,
            status,
            action,
        }
    }
}

/// Ask the daemon what is left, and draw that.
///
/// Read rather than latched: the account is spent from by other computers
/// and by a browser, so a figure this window kept would go stale without
/// anything here happening.
pub fn refresh(app: &Rc<App>) {
    app.call("near_ai_balance", serde_json::json!({}), |app, result| {
        // A read that did not come back says nothing. What is on screen is
        // the last answer that was true, and replacing it with a guess --
        // in either direction -- is the one move this section may not make.
        let Ok(Ok(balance)) = result.map(serde_json::from_value::<crate::model::NearAiBalance>)
        else {
            return;
        };
        render(app, &balance);
    });
}

/// The sentence the toned row carries.
///
/// **The empty state line is not a missing case, and this is the function
/// that says so.** `balance_state_line` answers the empty string for a read
/// that succeeded, because a sentence announcing that the read worked is
/// this app narrating itself -- so what takes the row there is the figure,
/// which is what a contributor opened the screen for. A view that drew the
/// sentence and stopped would give the one state with a balance in it a
/// glyph over a blank line.
///
/// Gated on [`crate::balance::BalanceView::figures_carry_the_row`], which is
/// the shared table's own answer rather than a label matched on here.
fn head(row: &crate::balance::BalanceView) -> &str {
    if row.figures_carry_the_row() {
        &row.remaining_line
    } else {
        row.state_line
    }
}

/// The lines drawn under the toned row, in order, with the empty ones gone.
///
/// An empty sentence is NO LINE AT ALL rather than a blank one: an uncapped
/// account has no ceiling line because [`head`] has already said the part
/// that matters about one, and an answer with no time on it has no age line
/// rather than "just now" invented under a figure.
///
/// `BALANCE_WHAT` comes last and only where there are figures. It says the
/// amounts are the whole account rather than this computer, and on a row
/// with no amounts on it it would be qualifying something that is not on
/// screen -- the rule the spend line above the tool list follows.
fn details(row: &crate::balance::BalanceView) -> Vec<&str> {
    let mut lines: Vec<&str> = [&row.limit_line, &row.spent_line, &row.observed_line]
        .into_iter()
        .map(String::as_str)
        .filter(|line| !line.is_empty())
        .collect();
    if row.figures_carry_the_row() {
        lines.push(copy::BALANCE_WHAT);
    }
    lines
}

/// One toned statement, the figures under it, and at most one control.
pub fn render(app: &Rc<App>, balance: &crate::model::NearAiBalance) {
    let view = &app.private_inference.balance;
    let row = crate::balance::view(balance);
    while let Some(child) = view.status.first_child() {
        view.status.remove(&child);
    }

    let head = head(&row);
    if !head.is_empty() {
        view.status.append(&private_inference::tone_row(
            head,
            private_inference::indicator_tone(row.tone),
        ));
    }
    for line in details(&row) {
        style::append_meta(&view.status, line);
    }

    while let Some(child) = view.action.first_child() {
        view.action.remove(&child);
    }
    let action = row.action;
    if let Some(label) = super::credential::action_label(action) {
        // Before the button, because it is what somebody reads before
        // pressing it. Empty draws nothing at all rather than a blank line.
        let explains = super::credential::action_explains(action);
        if !explains.is_empty() {
            style::append_body(&view.action, explains);
        }
        let button = gtk::Button::with_label(label);
        button.set_halign(gtk::Align::Start);
        let app = Rc::clone(app);
        // The sign-in row's own entry point, not a second one. One ceremony
        // mechanism in this shell means one in-flight guard, so a press here
        // and a press there cannot open two sign-ins.
        button.connect_clicked(move |_| super::credential::act(&app, action));
        view.action.append(&button);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::private_inference_copy::{
        LABEL_BALANCE_KNOWN, LABEL_BALANCE_NO_ORGANIZATION, LABEL_BALANCE_NO_SESSION,
        LABEL_BALANCE_SESSION_EXPIRED, LABEL_BALANCE_UNAVAILABLE,
    };

    const SOURCE: &str = include_str!("balance.rs");

    /// The module's own code, without the tests that quote it.
    fn code() -> &'static str {
        SOURCE
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
    }

    /// What one state actually puts on screen, in order: the toned head
    /// first, then the lines under it.
    ///
    /// **Assembled by calling the render path's own two functions**, not by
    /// restating what they do. A helper that rebuilt the ordering here would
    /// be a second implementation agreeing with itself, and would keep
    /// passing while `render` drew something else.
    fn drawn(body: &str) -> Vec<String> {
        let balance: crate::model::NearAiBalance =
            serde_json::from_str(body).expect("the daemon's own body decodes");
        let row = crate::balance::view(&balance);
        let mut lines = Vec::new();
        let head = head(&row);
        if !head.is_empty() {
            lines.push(head.to_string());
        }
        lines.extend(details(&row).into_iter().map(str::to_string));
        lines
    }

    /// A read that succeeded opens with a figure, not with a blank line.
    ///
    /// The failure this guards is the one the empty state line invites: a
    /// view that drew the sentence and stopped would give `known` a glyph
    /// over nothing at all, and the balance -- the whole point of the
    /// section -- would be the one state that showed no balance.
    #[test]
    fn a_read_balance_opens_with_the_figure_and_never_with_a_blank_line() {
        let lines = drawn(
            r#"{"state":"known","scale":9,"remaining_nanos":8500000000,
                 "spend_limit_nanos":10000000000,"total_spent_nanos":1500000000}"#,
        );
        assert!(!lines.is_empty(), "a read balance drew nothing");
        assert!(
            lines[0].contains("$8.50"),
            "the row does not open with the figure: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("$10.00")),
            "the ceiling is missing: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("$1.50")),
            "what the account has spent is missing: {lines:?}"
        );
        // And the sentence saying whose money this is rides with them.
        assert_eq!(lines.last().map(String::as_str), Some(copy::BALANCE_WHAT));
    }

    /// A null figure never renders as a zero one, on screen and not just in
    /// the view.
    #[test]
    fn an_uncapped_account_is_never_told_it_has_nothing_left() {
        let uncapped = drawn(
            r#"{"state":"known","scale":9,"remaining_nanos":null,
                 "spend_limit_nanos":null,"total_spent_nanos":0}"#,
        );
        assert!(
            !uncapped[0].contains("$0.00"),
            "an uncapped account was told it is empty: {uncapped:?}"
        );
        assert!(
            !uncapped[0].contains('$'),
            "an absent figure drew a figure: {uncapped:?}"
        );
        // No ceiling line either: the head already said it, and twice is
        // once too many.
        assert!(
            !uncapped.iter().any(|line| line.contains("Spending limit")),
            "an uncapped account drew a ceiling: {uncapped:?}"
        );
        // A zero SPEND is a real figure and is drawn.
        assert!(
            uncapped.iter().any(|line| line.contains("$0.00")),
            "a zero spend was suppressed: {uncapped:?}"
        );

        // An account that really is empty says so, and says something else.
        let empty = drawn(r#"{"state":"known","scale":9,"remaining_nanos":0}"#);
        assert!(empty[0].contains("$0.00"), "{empty:?}");
        assert_ne!(empty[0], uncapped[0]);
    }

    /// An overdrawn account renders its debt rather than no figure at all.
    ///
    /// The reason `present` is a separate field on the ABI: these amounts
    /// are signed, so absence could not ride on an out-of-range integer
    /// without rendering a real debt as nothing.
    #[test]
    fn an_overdrawn_account_shows_what_it_owes() {
        let lines = drawn(r#"{"state":"known","scale":9,"remaining_nanos":-2500000000}"#);
        assert!(lines[0].contains("-$2.50"), "{lines:?}");
    }

    /// Every state that is not a read draws its own sentence and no figure.
    #[test]
    fn an_unread_state_draws_its_sentence_and_no_figure() {
        for label in [
            LABEL_BALANCE_NO_SESSION,
            LABEL_BALANCE_SESSION_EXPIRED,
            LABEL_BALANCE_NO_ORGANIZATION,
            LABEL_BALANCE_UNAVAILABLE,
            "",
            "a_balance_state_from_a_later_daemon",
        ] {
            let lines = drawn(&format!(
                r#"{{"state":"{label}","scale":9,"remaining_nanos":null,
                      "spend_limit_nanos":null,"total_spent_nanos":null}}"#
            ));
            assert_eq!(lines.len(), 1, "{label:?} drew more than its sentence");
            assert_eq!(lines[0], copy::balance_state_line(label), "{label:?}");
            assert!(!lines[0].contains('$'), "{label:?} drew a figure");
            // And never the sentence that qualifies figures nobody drew.
            assert_ne!(lines[0], copy::BALANCE_WHAT, "{label:?}");
        }
    }

    /// The control appears on exactly the two states that answer it, and
    /// never without the sentence saying what pressing it costs.
    #[test]
    fn the_sign_in_control_appears_on_exactly_two_states() {
        let action = |label: &str| copy::balance_action(label);
        for offered in [LABEL_BALANCE_NO_SESSION, LABEL_BALANCE_SESSION_EXPIRED] {
            let action = action(offered);
            assert_eq!(
                super::super::credential::action_label(action),
                Some(copy::CREDENTIAL_OBTAIN),
                "{offered:?}"
            );
            // The cost sentence comes with it, out of the same match on the
            // same value the button is built from.
            assert_eq!(
                super::super::credential::action_explains(action),
                copy::CREDENTIAL_COST,
                "{offered:?}"
            );
        }
        for silent in [
            LABEL_BALANCE_KNOWN,
            LABEL_BALANCE_NO_ORGANIZATION,
            LABEL_BALANCE_UNAVAILABLE,
            "",
            "a_balance_state_from_a_later_daemon",
        ] {
            assert_eq!(
                super::super::credential::action_label(action(silent)),
                None,
                "{silent:?} drew a control"
            );
        }
    }

    /// `render` draws exactly what the two functions above return.
    ///
    /// The tests around this one exercise `head` and `details` directly, so
    /// they bite on any change to WHAT is drawn. What they cannot see is
    /// `render` quietly ceasing to call them -- a widget tree is not built
    /// in a headless test -- so this reads its source for the two calls, the
    /// two widget kinds they feed, and the button coming from the sign-in
    /// row's own table.
    #[test]
    fn render_draws_the_lines_these_functions_return() {
        let body = code()
            .split("pub fn render(")
            .nth(1)
            .expect("render is in this file");
        assert!(
            body.contains("let head = head(&row);"),
            "the toned row no longer comes from `head`"
        );
        assert!(
            body.contains("for line in details(&row) {")
                && body.contains("style::append_meta(&view.status, line);"),
            "the lines under the head no longer come from `details`"
        );
        assert!(
            body.contains("if !head.is_empty() {"),
            "an empty head would be drawn as a blank toned line"
        );
        assert!(
            body.contains("super::credential::action_label(action)"),
            "the control stopped coming from the sign-in row's own table"
        );
        assert!(
            body.contains("super::credential::act(&app, action)"),
            "the press no longer goes to the one ceremony mechanism"
        );
    }

    /// Nothing on this section is authored here, and no state is branched
    /// on.
    #[test]
    fn the_section_authors_no_sentence_and_branches_on_no_state() {
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
        for spelled in [
            LABEL_BALANCE_KNOWN,
            LABEL_BALANCE_NO_SESSION,
            LABEL_BALANCE_SESSION_EXPIRED,
            LABEL_BALANCE_NO_ORGANIZATION,
            LABEL_BALANCE_UNAVAILABLE,
        ] {
            assert!(
                !code().contains(&format!("\"{spelled}\"")),
                "the state is branched on in this view: {spelled}"
            );
        }
    }

    /// No amount is formatted in this shell, and no threshold is applied to
    /// one.
    ///
    /// The two failures this row is written against, read from the source
    /// because both compile and render plausibly: a shell dividing by its
    /// own constant is wrong by a factor of a thousand the day the daemon
    /// changes `scale`, and a shell comparing a figure to a number is
    /// inventing a threshold nobody set.
    #[test]
    fn no_figure_is_formatted_or_judged_here() {
        let code = code();
        for invented in ["1_000_000_000", "1000000000", "{:.2}", "/ 100", "* 100"] {
            assert!(
                !code.contains(invented),
                "an amount is being computed in this view: {invented}"
            );
        }
        // Nothing compares a figure to anything.
        for judged in [
            "remaining_nanos <",
            "remaining_nanos >",
            "row.remaining_line.contains",
        ] {
            assert!(!code.contains(judged), "a figure is being judged: {judged}");
        }
        // The tone is carried, never chosen.
        assert!(
            code.contains("private_inference::indicator_tone(row.tone)"),
            "the tone stopped coming from the shared table"
        );
        assert_eq!(
            code.matches("indicator_tone(row.tone)").count(),
            1,
            "the tone is decided in more than one place"
        );
    }
}
