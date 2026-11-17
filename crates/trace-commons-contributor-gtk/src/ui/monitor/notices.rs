//! Which popover notice is laid over which tab.
//!
//! The placement half of `MonitorNotices` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorNotices.swift`
//! (owner-approved for this shell on 2026-11-16: popover notices are laid
//! over the tab content and push nothing down). The cards are the ones this
//! window already drew, with the words and the buttons they already had;
//! this decides only where each one is shown.
//!
//! Deliberate platform differences from the Swift:
//!
//! - The Mac shows the core's health and the consent notices about folders
//!   (grant voids, arming rewordings, a gate hold) on Traces only. This
//!   window has always shown them on every screen, so that someone reading
//!   another screen still learns contributions are held up or a grant was
//!   voided. They stay on every tab: showing a safeguard in fewer places is
//!   not a change this slice makes.
//! - No card gains a close button. The Mac's X closes a card until the
//!   window is opened again; here each card keeps exactly the buttons it
//!   had, so nothing that had to be acknowledged can now be put away.
//! - The cards are a column, not the Mac's peeking stack. The column is
//!   never taller than the tab's own area and scrolls past that, so every
//!   card's buttons stay reachable; with no card showing it is hidden.

use super::tabs::Place;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// An update is ready. About this app, not a tab's subject.
    Update,
    /// The core's health.
    Health,
    /// Consent notices and the account's: grant voids, arming rewordings,
    /// a gate hold, the legacy invite move and its notice.
    Consent,
    /// The offer to route AI requests through Private AI.
    PrivateAiOffer,
    /// The offer to contribute a folder automatically.
    ArmingOffer,
    /// An approval that can still be taken back.
    Undo,
}

impl Notice {
    /// In the order they are stacked: the undo first, because it is the
    /// one card with a clock on it.
    pub const ALL: [Notice; 6] = [
        Notice::Undo,
        Notice::Update,
        Notice::Health,
        Notice::Consent,
        Notice::PrivateAiOffer,
        Notice::ArmingOffer,
    ];
}

/// Whether `notice` is laid over the tab at `place`.
pub fn shown(notice: Notice, place: Place) -> bool {
    match notice {
        Notice::Update | Notice::Health | Notice::Consent => true,
        Notice::ArmingOffer | Notice::Undo => matches!(place, Place::Traces | Place::History),
        Notice::PrivateAiOffer => place == Place::PrivateAi,
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
    fn a_safeguard_is_shown_on_every_tab() {
        for place in PLACES {
            for notice in [Notice::Update, Notice::Health, Notice::Consent] {
                assert!(shown(notice, place), "{notice:?} is hidden on {place:?}");
            }
        }
    }

    #[test]
    fn the_undo_and_the_arming_offer_are_the_traces_tabs() {
        for notice in [Notice::Undo, Notice::ArmingOffer] {
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
}
