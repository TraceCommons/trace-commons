//! Which popover notice is shown over which tab, and what its X does.
//!
//! The placement and closing halves of `MonitorNotices` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorNotices.swift`
//! (owner-approved for this shell on 2026-11-16). The cards are the ones
//! this window already drew, with the words and the buttons they already
//! had; this decides where each one is shown and whether it carries an X.
//!
//! Where each one goes, as on the Mac:
//!
//! - Every tab: what is about this app or this account, not a tab's
//!   subject: the update notice and the legacy invite move and its notice.
//! - Traces and its History page: the undo, the core's health, the consent
//!   notices about folders (grant voids, arming rewordings, a gate hold)
//!   and the arming offer.
//! - Private AI: the offer to route AI requests through Private AI.
//!
//! What the X does, as on the Mac:
//!
//! - The update notice, a waiting health notice and the arming offer: the
//!   X closes the card until the window is opened again (`Close::Session`).
//!   A closed card comes back when what it says changes, or when it goes
//!   and its cause recurs.
//! - The undo: the X is the Mac's "close, it still sends": it puts the
//!   card away and the approval stands (`Close::LetItSend`).
//! - The consent notices, the account's notices and the Private AI offer
//!   carry no extra X (`Close::Own`). On the Mac each one's X is its own
//!   Acknowledge, Dismiss or Not now, which these cards already draw as a
//!   button in the same words; a second control doing the same thing is
//!   not added. A gate hold has no X on the Mac either: it goes when the
//!   hold does.
//!
//! Deliberate platform differences from the Swift:
//!
//! - The Mac has no update notice in this window (its updater draws its
//!   own); this window always had one, and it stays on every tab as one of
//!   the notices about this app. An update installing or ready to restart
//!   describes work happening to the machine and has no X
//!   (`ui::update::banner_for`).
//! - The four acknowledged safeguards the Mac keeps closed across restarts
//!   (`MonitorNoticeDismissals`) close for this window only here.
//! - The cards are a column, not the Mac's peeking stack, and the column
//!   sits above the tab's content rather than over it, so no heading or
//!   control of the tab is ever under a card. The column is never taller
//!   than `pane_layout::NOTICES_MAX_HEIGHT` and scrolls past that, so every
//!   card's buttons stay reachable; with no card showing it takes no room.

use super::tabs::Place;

/// The class a card carries while it may not be closed even though its
/// kind has an X: an update installing or ready to restart.
pub const STAYS_CLASS: &str = "tc-notice-stays";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// An update is ready. About this app, not a tab's subject.
    Update,
    /// The account's notices: the legacy invite move and its notice.
    Account,
    /// The core's health.
    Health,
    /// Consent notices about folders: grant voids, arming rewordings, a
    /// gate hold.
    Consent,
    /// The offer to route AI requests through Private AI.
    PrivateAiOffer,
    /// The offer to contribute a folder automatically.
    ArmingOffer,
    /// An approval that can still be taken back.
    Undo,
}

/// What a notice's X does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Close {
    /// Closed until the window is opened again, or until what the card says
    /// changes.
    Session,
    /// The undo's "close, it still sends": the card goes and the approval
    /// stands.
    LetItSend,
    /// No extra X: the card's own Acknowledge, Dismiss or Not now is the
    /// Mac's X, already drawn in the same words.
    Own,
}

impl Notice {
    /// In the order they are stacked: the undo first, because it is the
    /// one card with a clock on it.
    pub const ALL: [Notice; 7] = [
        Notice::Undo,
        Notice::Update,
        Notice::Account,
        Notice::Health,
        Notice::Consent,
        Notice::PrivateAiOffer,
        Notice::ArmingOffer,
    ];

    pub fn close(self) -> Close {
        match self {
            Notice::Update | Notice::Health | Notice::ArmingOffer => Close::Session,
            Notice::Undo => Close::LetItSend,
            Notice::Account | Notice::Consent | Notice::PrivateAiOffer => Close::Own,
        }
    }
}

/// Whether `notice` is shown over the tab at `place`.
pub fn shown(notice: Notice, place: Place) -> bool {
    match notice {
        Notice::Update | Notice::Account => true,
        Notice::Health | Notice::Consent | Notice::ArmingOffer | Notice::Undo => {
            matches!(place, Place::Traces | Place::History)
        }
        Notice::PrivateAiOffer => place == Place::PrivateAi,
    }
}

/// A card closed with its X, by what it said when it was closed.
///
/// The Mac keys a closed card by its item id, which carries what the card
/// is about (a health banner's title, an arming offer's folder). The cards
/// here have no id, so the key is the card's own text: a card whose words
/// change is a different card and is shown again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Closed(Vec<(Notice, String)>);

impl Closed {
    pub fn close(&mut self, notice: Notice, said: String) {
        self.0.retain(|(kind, _)| *kind != notice);
        self.0.push((notice, said));
    }

    /// Whether `notice`, saying `said` now, is closed.
    pub fn hides(&self, notice: Notice, said: &str) -> bool {
        self.0
            .iter()
            .any(|(kind, closed)| *kind == notice && closed == said)
    }

    /// The card went: its cause has gone, so when it recurs it is shown
    /// again (the Mac's `prunedClosed`).
    pub fn forget(&mut self, notice: Notice) {
        self.0.retain(|(kind, _)| *kind != notice);
    }

    /// The window was opened again.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACES: [Place; 6] = [
        Place::Home,
        Place::PrivateAi,
        Place::Traces,
        Place::History,
        Place::Insights,
        Place::HomeHistory,
    ];

    #[test]
    fn what_is_about_this_app_or_the_account_is_shown_on_every_tab() {
        for place in PLACES {
            for notice in [Notice::Update, Notice::Account] {
                assert!(shown(notice, place), "{notice:?} is hidden on {place:?}");
            }
        }
    }

    /// Health on Traces only, as on the Mac (owner-approved 2026-11-16 for
    /// this fix pass), with the consent notices about folders, the undo and
    /// the arming offer.
    #[test]
    fn health_consent_the_undo_and_the_arming_offer_are_the_traces_tabs() {
        for notice in [
            Notice::Health,
            Notice::Consent,
            Notice::Undo,
            Notice::ArmingOffer,
        ] {
            assert!(shown(notice, Place::Traces));
            assert!(shown(notice, Place::History), "History is Traces' page");
            for place in [
                Place::Home,
                Place::PrivateAi,
                Place::Insights,
                Place::HomeHistory,
            ] {
                assert!(!shown(notice, place), "{notice:?} on {place:?}");
            }
        }
    }

    #[test]
    fn the_private_ai_offer_is_the_inference_tabs() {
        for place in PLACES {
            assert_eq!(
                shown(Notice::PrivateAiOffer, place),
                place == Place::PrivateAi,
                "{place:?}"
            );
        }
    }

    #[test]
    fn every_notice_is_shown_somewhere() {
        for notice in Notice::ALL {
            assert!(
                PLACES.iter().any(|place| shown(notice, *place)),
                "{notice:?} is never shown"
            );
        }
    }

    #[test]
    fn the_x_is_the_macs_for_each_notice() {
        assert_eq!(Notice::Update.close(), Close::Session);
        assert_eq!(Notice::Health.close(), Close::Session);
        assert_eq!(Notice::ArmingOffer.close(), Close::Session);
        assert_eq!(Notice::Undo.close(), Close::LetItSend);
        for notice in [Notice::Account, Notice::Consent, Notice::PrivateAiOffer] {
            assert_eq!(notice.close(), Close::Own, "{notice:?}");
        }
    }

    #[test]
    fn a_closed_card_stays_closed_until_its_words_change_or_it_goes() {
        let mut closed = Closed::default();
        assert!(!closed.hides(Notice::Health, "held"));
        closed.close(Notice::Health, "held".into());
        assert!(closed.hides(Notice::Health, "held"));
        assert!(!closed.hides(Notice::Update, "held"), "per notice");
        // New words are a new card.
        assert!(!closed.hides(Notice::Health, "held, and the witness is busy"));
        // The card went: when its cause recurs it is shown again.
        closed.forget(Notice::Health);
        assert!(!closed.hides(Notice::Health, "held"));
        // Opening the window again shows everything.
        closed.close(Notice::Update, "built from source".into());
        closed.clear();
        assert!(!closed.hides(Notice::Update, "built from source"));
    }
}
