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
//! - The undo, the consent notices, the account's notices and the Private
//!   AI offer carry no extra X (`Close::Own`). On the Mac each of these
//!   draws an X (`GlassPopoverNotice`'s `onClose`) that runs its own
//!   Dismiss (the undo's "close, it still sends"), Acknowledge or Not now;
//!   these cards already draw that as a button in the same words, so a
//!   second control doing the same thing is not added here (the X set the
//!   owner approved). A gate hold has no X on the Mac: it goes when the
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
//! - The stack is laid over the top of the tab's content, as on the Mac
//!   (`GlassPopoverStack`, owner decision 1 of the 2026-11-16 spec, chosen
//!   again on 2026-11-17): it pushes nothing down, so the front card covers
//!   the top of the tab until it is closed. Only the front card is drawn
//!   whole and up to two cards behind it peek out under it (`stack`). A
//!   front card with no X here (the undo, a consent or account notice, the
//!   Private AI offer, an update installing) keeps the cards behind it out
//!   of reach until its own button puts it away. The Mac differs: there
//!   every one of those but an update (which it does not show) has an X,
//!   and only a card that goes when its cause does (a gate hold, the terms
//!   hold, a down or unread core's health) has none.
//! - A new card slides in from the top and a revealed one fades, and each
//!   plate is 4% a level narrower at 0.9 and 0.7 opacity, as on the Mac
//!   (`ui::popover_stack`). Not ported: the covered card's fade (it is
//!   hidden at once), and the plates are slivers under the front card, not
//!   cards its size behind it, so they do not show in its rounded corners.

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
    /// No extra X: the card's own Dismiss, Acknowledge or Not now is the
    /// Mac's X, already drawn in the same words.
    Own,
}

impl Notice {
    /// In the order they are stacked, the front first, as `MonitorNotices`
    /// stacks them: what is happening to the traces now (the undo, which
    /// has a clock on it, then the core's health and the consent notices)
    /// in front of what waits on an answer (the arming and Private AI
    /// offers), and those in front of what is about this app or the
    /// account.
    pub const ALL: [Notice; 7] = [
        Notice::Undo,
        Notice::Health,
        Notice::Consent,
        Notice::ArmingOffer,
        Notice::PrivateAiOffer,
        Notice::Account,
        Notice::Update,
    ];

    pub fn close(self) -> Close {
        match self {
            Notice::Update | Notice::Health | Notice::ArmingOffer => Close::Session,
            Notice::Undo | Notice::Account | Notice::Consent | Notice::PrivateAiOffer => Close::Own,
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

/// How far each card behind shows past the one in front, in pixels
/// (`GlassPopoverStack.peek`).
pub const PEEK: i32 = 8;

/// How many cards behind the front one peek out at most
/// (`GlassPopoverStack.drawn`, less the front card).
pub const BEHIND: usize = 2;

/// What the stack draws, given which cards are open, front first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stack {
    /// The one card drawn whole: the first open one. `None` when none is
    /// open, and the stack takes no room.
    pub front: Option<usize>,
    /// How many cards peek out behind it, at most `BEHIND`.
    pub behind: usize,
}

/// The Mac's peeking stack: the front card whole, up to two behind it
/// peeking out. Closing the front card brings the next one forward.
pub fn stack(open: &[bool]) -> Stack {
    let front = open.iter().position(|open| *open);
    let count = open.iter().filter(|open| **open).count();
    Stack {
        front,
        behind: count.saturating_sub(1).min(BEHIND),
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
        for notice in [
            Notice::Undo,
            Notice::Account,
            Notice::Consent,
            Notice::PrivateAiOffer,
        ] {
            assert_eq!(notice.close(), Close::Own, "{notice:?}");
        }
    }

    /// The Mac's order: what is happening now in front of what waits on an
    /// answer, and both in front of what is about this app or the account.
    #[test]
    fn the_stack_order_is_the_macs_front_first() {
        assert_eq!(
            Notice::ALL,
            [
                Notice::Undo,
                Notice::Health,
                Notice::Consent,
                Notice::ArmingOffer,
                Notice::PrivateAiOffer,
                Notice::Account,
                Notice::Update,
            ]
        );
    }

    #[test]
    fn only_the_front_card_is_whole_and_two_at_most_peek_behind_it() {
        assert_eq!(
            stack(&[false; 7]),
            Stack {
                front: None,
                behind: 0
            }
        );
        assert_eq!(
            stack(&[false, false, true, false]),
            Stack {
                front: Some(2),
                behind: 0
            }
        );
        assert_eq!(
            stack(&[false, true, false, true]),
            Stack {
                front: Some(1),
                behind: 1
            }
        );
        assert_eq!(
            stack(&[true; 7]),
            Stack {
                front: Some(0),
                behind: BEHIND
            }
        );
        assert_eq!(BEHIND, 2, "the Mac draws three cards");
        assert_eq!(PEEK, 8, "the Mac's peek");
    }

    #[test]
    fn closing_the_front_card_brings_the_next_one_forward() {
        let mut open = [true, true, true];
        assert_eq!(stack(&open).front, Some(0));
        open[0] = false;
        assert_eq!(
            stack(&open),
            Stack {
                front: Some(1),
                behind: 1
            }
        );
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
