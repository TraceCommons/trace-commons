//! The update surface: one banner under the header bar, one confirmation
//! dialog, and the restart prompt that finishes the job.
//!
//! The decision of *what* to say for a given state is a pure function
//! ([`banner_for`]) so it can be tested without a display; the widgets
//! below are a direct mapping of its output onto libadwaita and carry no
//! logic of their own. Nothing here installs anything: the confirmation
//! hands off to `update::UpdateMonitor::request_install`, which asks the
//! portal, which does the work.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::App;
use super::style::{Tone, space};
use crate::copy;
use crate::update::{self, UpdateState};

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

/// The banner itself. Built in the same shape as the health banner in
/// `ui::mod` -- glyph, wrapping body, one optional button -- because a
/// contributor should not have to learn two kinds of notice bar.
pub struct UpdateView {
    pub root: gtk::Box,
    glyph: gtk::Label,
    body: gtk::Label,
    button: gtk::Button,
    /// The live state, owned by the UI thread. The D-Bus threads never
    /// touch it; they send signals and this applies them.
    state: RefCell<UpdateState>,
    /// What the button currently means, so one handler serves both actions.
    action: RefCell<Option<BannerAction>>,
    /// `None` outside a flatpak, where no monitor is ever created.
    monitor: RefCell<Option<update::UpdateMonitor>>,
}

impl Default for UpdateView {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateView {
    pub fn new() -> Self {
        let glyph = gtk::Label::new(Some(Tone::Neutral.glyph()));
        glyph.add_css_class("tc-card-title");
        glyph.set_valign(gtk::Align::Start);

        let body = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build();
        body.add_css_class("tc-body");

        let button = gtk::Button::builder().visible(false).build();
        button.add_css_class("tc-quiet");
        button.set_valign(gtk::Align::Center);

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(space::M)
            .visible(false)
            .margin_top(space::M)
            .margin_start(space::L)
            .margin_end(space::L)
            .build();
        root.append(&glyph);
        root.append(&body);
        root.append(&button);
        root.add_css_class("tc-banner");

        Self {
            root,
            glyph,
            body,
            button,
            state: RefCell::new(UpdateState::Idle),
            action: RefCell::new(None),
            monitor: RefCell::new(None),
        }
    }
}

/// Draw the current state. Called once at startup and after every signal.
fn render(app: &Rc<App>) {
    let view = &app.update;
    let banner = banner_for(&view.state.borrow());

    let Some(banner) = banner else {
        view.root.set_visible(false);
        *view.action.borrow_mut() = None;
        return;
    };

    view.glyph.set_text(banner.tone.glyph());
    // One tone class at a time, so a state change does not leave the
    // previous state's colour behind.
    for tone in [
        Tone::Neutral,
        Tone::Clear,
        Tone::Attention,
        Tone::Held,
        Tone::Refused,
    ] {
        view.glyph.remove_css_class(tone.css());
    }
    view.glyph.add_css_class(banner.tone.css());
    view.body.set_text(&banner.body);

    match banner.action {
        Some((label, action)) => {
            view.button.set_label(label);
            view.button.set_visible(true);
            *view.action.borrow_mut() = Some(action);
        }
        None => {
            view.button.set_visible(false);
            *view.action.borrow_mut() = None;
        }
    }
    view.root.set_visible(true);
}

/// Start the monitor, pump its signals onto the main loop, and connect the
/// one button.
///
/// Outside a flatpak nothing is started at all: the state goes straight to
/// `Unmanaged` and the banner says so. Under flatpak the monitor runs on
/// its own threads and the window never blocks on it, so a portal that
/// never answers costs nothing but a missing banner.
pub fn wire(app: &Rc<App>) {
    if update::detect_install_kind() != update::InstallKind::Flatpak {
        *app.update.state.borrow_mut() = UpdateState::Unmanaged;
        render(app);
        return;
    }

    let monitor = update::spawn_monitor();
    let signals = monitor.signals.clone();
    *app.update.monitor.borrow_mut() = Some(monitor);
    render(app);

    let pump = Rc::clone(app);
    gtk::glib::spawn_future_local(async move {
        while let Ok(signal) = signals.recv().await {
            let next = {
                let current = pump.update.state.borrow();
                update::next_state(&current, &signal)
            };
            *pump.update.state.borrow_mut() = next;
            render(&pump);
        }
    });

    let pressed = Rc::clone(app);
    app.update.button.connect_clicked(move |_| {
        let action = *pressed.update.action.borrow();
        match action {
            Some(BannerAction::Confirm) => confirm_install(&pressed),
            // The existing close-request handler runs, so quitting still
            // says what keeps running afterwards. This does not relaunch:
            // a confined process cannot start itself, and the honest
            // instruction is to reopen it.
            Some(BannerAction::Restart) => pressed.window.close(),
            None => {}
        }
    });
}

/// The confirmation. Nothing about the installed application changes
/// without a person pressing the accept response here.
fn confirm_install(app: &Rc<App>) {
    let dialog = adw::MessageDialog::new(
        Some(&app.window),
        Some(copy::UPDATE_CONFIRM_HEADING),
        Some(copy::UPDATE_CONFIRM_BODY),
    );
    dialog.add_responses(&[
        ("cancel", copy::UPDATE_CONFIRM_CANCEL),
        ("install", copy::UPDATE_CONFIRM_ACCEPT),
    ]);
    dialog.set_close_response("cancel");

    let app = Rc::clone(app);
    dialog.connect_response(None, move |dialog, response| {
        dialog.close();
        if response != "install" {
            return;
        }
        // Move into Installing before the call goes out, so the window
        // shows work starting rather than sitting on a stale offer until
        // the first Progress signal arrives. `begin_install` is a no-op
        // from any state that is not an offer.
        let next = {
            let current = app.update.state.borrow();
            update::begin_install(&current)
        };
        *app.update.state.borrow_mut() = next;
        render(&app);

        if let Some(monitor) = app.update.monitor.borrow().as_ref() {
            monitor.request_install();
        }
    });
    dialog.present();
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
