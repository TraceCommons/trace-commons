//! The update surface: one banner under the header bar, one confirmation
//! dialog, and the restart prompt that finishes the job.
//!
//! The decision of *what* to say for a given state is a pure function
//! ([`banner_for`]) so it can be tested without a display; the widgets
//! below are a direct mapping of its output onto libadwaita and carry no
//! logic of their own. Nothing here installs anything: the confirmation
//! hands off to `update::UpdateMonitor::request_install`, which asks the
//! portal, which does the work.

use super::style::Tone;
use crate::copy;
use crate::update::UpdateState;

/// What the banner's one button does, if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerAction {
    /// Open the confirmation dialog. Never installs directly: the banner is
    /// something a person notices, not something they act on by reflex.
    Confirm,
    /// Close the window so the next start runs the installed build.
    Restart,
}

/// Everything the banner renders, decided without touching a widget.
pub struct Banner {
    pub tone: Tone,
    pub body: String,
    /// Button label and what pressing it means, or `None` for a banner that
    /// is only telling you something.
    pub action: Option<(&'static str, BannerAction)>,
}

/// What to show for a state, or `None` to show nothing at all.
///
/// `Idle` is `None` on purpose: "you are up to date" is a sentence that
/// occupies the top of the window permanently and tells a contributor
/// nothing they can act on. `Installing` and `Ready` have no dismissal
/// because they describe work that is happening to their machine.
pub fn banner_for(state: &UpdateState) -> Option<Banner> {
    match state {
        UpdateState::Idle => None,
        UpdateState::Unmanaged => Some(Banner {
            tone: Tone::Neutral,
            body: copy::UPDATE_UNMANAGED_BODY.to_string(),
            action: None,
        }),
        UpdateState::Unavailable => Some(Banner {
            tone: Tone::Attention,
            body: copy::UPDATE_UNAVAILABLE_BODY.to_string(),
            action: None,
        }),
        UpdateState::Available { remote_commit } => Some(Banner {
            tone: Tone::Clear,
            body: copy::update_offer_line(remote_commit),
            action: Some((copy::UPDATE_AVAILABLE_ACTION, BannerAction::Confirm)),
        }),
        UpdateState::Installing { percent } => Some(Banner {
            tone: Tone::Held,
            body: copy::update_installing_line(*percent),
            action: None,
        }),
        UpdateState::Ready => Some(Banner {
            tone: Tone::Clear,
            body: copy::UPDATE_READY_BODY.to_string(),
            action: Some((copy::UPDATE_READY_ACTION, BannerAction::Restart)),
        }),
        UpdateState::Failed { .. } => Some(Banner {
            tone: Tone::Attention,
            body: copy::UPDATE_FAILED_BODY.to_string(),
            action: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::FAILED_LABEL;

    #[test]
    fn being_up_to_date_puts_nothing_on_screen() {
        assert!(banner_for(&UpdateState::Idle).is_none());
    }

    #[test]
    fn an_offer_is_the_only_state_with_a_confirm_button() {
        let banner = banner_for(&UpdateState::Available {
            remote_commit: "beefbeefbeef".to_string(),
        })
        .expect("an offer must be shown");
        assert_eq!(banner.action, Some(("Install", BannerAction::Confirm)));
        // The commit a person is being offered is named, truncated.
        assert!(banner.body.contains("beefbeefbeef"), "{}", banner.body);

        for state in [
            UpdateState::Unmanaged,
            UpdateState::Unavailable,
            UpdateState::Installing { percent: 50 },
            UpdateState::Ready,
            UpdateState::Failed {
                label: FAILED_LABEL,
            },
        ] {
            let action = banner_for(&state).and_then(|banner| banner.action);
            assert_ne!(
                action.map(|(_, what)| what),
                Some(BannerAction::Confirm),
                "{state:?} must not offer an install"
            );
        }
    }

    #[test]
    fn an_install_in_flight_cannot_be_confirmed_again() {
        let banner = banner_for(&UpdateState::Installing { percent: 40 })
            .expect("work underway must be visible");
        assert!(banner.action.is_none());
        assert!(banner.body.contains("40"), "{}", banner.body);
    }

    #[test]
    fn a_finished_install_asks_for_a_restart_and_says_the_queue_is_safe() {
        let banner = banner_for(&UpdateState::Ready).expect("a finished install must be visible");
        assert_eq!(banner.action, Some(("Quit now", BannerAction::Restart)));
        assert!(banner.body.contains("queue"), "{}", banner.body);
    }

    #[test]
    fn a_failure_states_the_data_consequence_and_never_the_portal_text() {
        let banner = banner_for(&UpdateState::Failed {
            label: FAILED_LABEL,
        })
        .expect("a failure must be visible");
        assert!(banner.tone == Tone::Attention);
        assert!(banner.body.contains("unchanged"), "{}", banner.body);
        // The internal label is for logs, not for a window.
        assert!(!banner.body.contains(FAILED_LABEL), "{}", banner.body);
    }

    #[test]
    fn a_source_build_is_told_plainly_that_nothing_updates_it() {
        let banner = banner_for(&UpdateState::Unmanaged).expect("a source build must be told");
        assert!(banner.action.is_none());
        assert!(banner.tone == Tone::Neutral);
        assert!(banner.body.contains("built from source"), "{}", banner.body);
    }

    #[test]
    fn a_missing_portal_is_told_where_to_go_instead() {
        let banner = banner_for(&UpdateState::Unavailable).expect("a missing portal must be told");
        assert!(banner.action.is_none());
        assert!(banner.body.contains("flatpak update"), "{}", banner.body);
    }
}
