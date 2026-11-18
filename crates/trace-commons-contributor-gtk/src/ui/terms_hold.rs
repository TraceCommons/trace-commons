//! The terms hold, ported from the Mac (`AppModel+Terms.swift`): while
//! `status.terms_hold` is set, a notice with no close action says uploads
//! are paused, and its one button opens the terms to accept again. Every
//! word is the core's (`first_run_copy().terms_hold`).

use std::rc::Rc;

use adw::prelude::*;
use serde_json::Value;
use trace_commons_contributor::first_run_copy::{TermsHoldCopy, first_run_copy};

use super::App;

/// The notice's words while held, or `None`.
pub(crate) fn notice_words(held: bool) -> Option<TermsHoldCopy> {
    held.then(|| first_run_copy().terms_hold)
}

/// How one `terms_accept` from the sheet ended (`TermsAcceptOutcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcceptOutcome {
    /// Recorded; the sheet closes and status is read again.
    Accepted,
    /// The version read is no longer the published one: the terms are
    /// fetched again.
    Stale,
    /// Refused or not answered. The sheet stays.
    Failed,
}

/// The Mac reads any result object as recorded, and the daemon's
/// `terms-version-stale` as a stale read.
pub(crate) fn accept_outcome(reply: &Result<Value, String>) -> AcceptOutcome {
    match reply {
        Ok(value) if value.is_object() => AcceptOutcome::Accepted,
        Ok(_) => AcceptOutcome::Failed,
        Err(label) if label == "terms-version-stale" => AcceptOutcome::Stale,
        Err(_) => AcceptOutcome::Failed,
    }
}

/// The notice: no close action, it goes only when the hold does.
pub(crate) fn card(app: &Rc<App>, words: &TermsHoldCopy) -> gtk::Box {
    let (card, column, buttons) = App::notice_card();
    column.append(&App::notice_text(words.notice_title, "tc-card-title"));
    column.append(&App::notice_text(words.notice_body, "tc-body"));
    let review = gtk::Button::with_label(words.review);
    buttons.append(&review);
    let weak = Rc::downgrade(app);
    review.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            open_sheet(&app);
        }
    });
    card
}

/// The terms, to read and accept again. Accept sends the exact document
/// read; a stale read is fetched again, and any other refusal keeps the
/// sheet.
pub(crate) fn open_sheet(app: &Rc<App>) {
    let window = adw::Window::builder()
        .transient_for(&app.window)
        .destroy_with_parent(true)
        .modal(true)
        .default_width(640)
        .default_height(640)
        .build();
    // The page owns the receipt; Continue reads it through this slot,
    // filled once the page exists.
    type Receipt = Rc<std::cell::RefCell<Option<super::first_run_terms::Accepted>>>;
    let slot: Rc<std::cell::RefCell<Option<Receipt>>> = Rc::default();
    let page = {
        let weak_app = Rc::downgrade(app);
        let weak_window = window.downgrade();
        let slot = Rc::clone(&slot);
        super::first_run_terms::TermsPage::new(move || {
            let receipt = slot
                .borrow()
                .as_ref()
                .and_then(|accepted| accepted.borrow().clone());
            let (Some(app), Some(receipt)) = (weak_app.upgrade(), receipt) else {
                return;
            };
            let weak_window = weak_window.clone();
            app.call("terms_accept", receipt.params(), move |app, result| {
                let Some(window) = weak_window.upgrade() else {
                    return;
                };
                match accept_outcome(&result) {
                    AcceptOutcome::Accepted => {
                        window.close();
                        app.refresh();
                    }
                    AcceptOutcome::Stale => {
                        window.close();
                        open_sheet(app);
                    }
                    AcceptOutcome::Failed => {}
                }
            });
        })
    };
    *slot.borrow_mut() = Some(Rc::clone(&page.accepted));
    let body = gtk::Box::new(gtk::Orientation::Vertical, super::style::space::M);
    body.set_margin_top(super::style::space::L);
    body.set_margin_bottom(super::style::space::L);
    body.set_margin_start(super::style::space::L);
    body.set_margin_end(super::style::space::L);
    body.append(&page.root);
    window.set_content(Some(&body));
    window.present();
}

#[cfg(all(test, target_os = "linux"))]
#[path = "../../tests/support/consent_terms_tests.rs"]
mod consent_mounted_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_notice_shows_only_while_held() {
        let words = trace_commons_contributor::first_run_copy::first_run_copy().terms_hold;
        assert!(notice_words(false).is_none());
        let shown = notice_words(true).expect("held");
        assert_eq!(shown.notice_title, words.notice_title);
        assert_eq!(shown.notice_body, words.notice_body);
        assert_eq!(shown.review, words.review);
    }

    #[test]
    fn the_status_reads_the_hold() {
        let held: crate::model::Status =
            serde_json::from_value(json!({"terms_hold": true})).unwrap();
        assert!(held.terms_hold);
        let older: crate::model::Status = serde_json::from_value(json!({})).unwrap();
        assert!(!older.terms_hold);
    }

    #[test]
    fn an_acceptance_is_read_as_the_mac_reads_it() {
        assert_eq!(
            accept_outcome(&Ok(json!({"held": false}))),
            AcceptOutcome::Accepted
        );
        assert_eq!(accept_outcome(&Ok(json!({}))), AcceptOutcome::Accepted);
        assert_eq!(accept_outcome(&Ok(json!(null))), AcceptOutcome::Failed);
        assert_eq!(
            accept_outcome(&Err("terms-version-stale".into())),
            AcceptOutcome::Stale
        );
        assert_eq!(
            accept_outcome(&Err("unavailable".into())),
            AcceptOutcome::Failed
        );
    }
}
