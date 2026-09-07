//! Keyboard accelerators, and the one rule they may not route around.
//!
//! This shell had none at all until now, while the other two shipped
//! `Cmd/Ctrl-1..N` and a toggle chord. That is the gap this closes.
//!
//! Two things are load-bearing here.
//!
//! **The screen accelerators are derived from `ui::SCREENS`**, never listed.
//! A screen added to that array arrives with its accelerator already
//! attached and in the switcher's own order, so the number a contributor
//! presses cannot drift from the position they see. `tray.rs` builds its
//! menu from the same array for the same reason.
//!
//! **The toggle chord is asymmetric, and it does not restate the rule.** It
//! asks `tray::toggle_request`, which is the single definition of the
//! asymmetry the tray menu already obeys: while this computer is answering,
//! the chord stops it; while it is not, the chord opens the screen where the
//! switch sits beside the sentence about what turning it on exposes, and
//! writes nothing. A chord that could enable machine-wide exposure with that
//! sentence off-screen is the exact fail-open the destination was built to
//! prevent -- and `preview.rs` refuses an accelerator on the one irreversible
//! control for the same reason.
//!
//! An accelerator is an accelerant and never the only path. GNOME has no
//! tray and `ui/mod.rs` requires every capability be reachable from the
//! window; each of these presses something a contributor can already click.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use gtk::glib;

use super::{App, SCREENS};

/// The window action the screen accelerators drive, with the stack name as
/// its target. One action rather than one per screen, so the set of
/// accelerators is a loop over `SCREENS` and not a list somebody maintains.
const SCREEN_ACTION: &str = "screen";

/// The window action the toggle chord drives.
const TOGGLE_ACTION: &str = "model-calls-toggle";

/// The chord that toggles answering, matching the Windows shell's
/// `Ctrl-Shift-M` so the two say the same thing to the same fingers.
///
/// `<Control><Shift>m` and not a bare `<Control>m`: a single-modifier letter
/// is the space desktop text controls live in, and this one may act.
const TOGGLE_ACCEL: &str = "<Control><Shift>m";

/// Highest screen an accelerator is offered for. `<Control>0` is not a tenth
/// item anywhere, so a tenth screen gets no chord rather than a wrong one --
/// and it is still one click away in the switcher, which is the rule.
const MAX_NUMBERED_SCREENS: usize = 9;

/// The accelerator for the screen at `index` of the switcher, or `None` past
/// what a single digit can name.
///
/// Derived from the position, so the digit a contributor presses is the
/// position they see. Nothing here names a screen.
pub(crate) fn screen_accel(index: usize) -> Option<String> {
    (index < MAX_NUMBERED_SCREENS).then(|| format!("<Control>{}", index + 1))
}

/// The detailed action name GTK binds an accelerator to, for one stack name.
fn screen_action(name: &str) -> String {
    format!("win.{SCREEN_ACTION}::{name}")
}

/// Every screen accelerator, in the switcher's order, as
/// `(stack name, accelerator)`.
///
/// Built from `SCREENS` and from nothing else, which is the property that
/// keeps the numbering from drifting: a screen inserted in the middle of
/// that array renumbers everything after it here, in the switcher, and in
/// the tray menu together, because all three read the one array.
pub(crate) fn screen_accels() -> Vec<(&'static str, String)> {
    SCREENS
        .iter()
        .enumerate()
        .filter_map(|(index, (name, _, _))| Some((*name, screen_accel(index)?)))
        .collect()
}

/// What GTK would print for an accelerator, in the contributor's own
/// keyboard and language.
///
/// Asked of GTK rather than composed here, which is why this shell can show
/// a chord without authoring a word: `gtk::accelerator_get_label` is what
/// GNOME's own menus print. `None` for anything that does not parse, so a
/// bad accelerator shows no tooltip rather than a wrong one.
fn accel_label(accel: &str) -> Option<String> {
    let (key, modifiers) = gtk::accelerator_parse(accel)?;
    let label = gtk::accelerator_get_label(key, modifiers);
    (!label.is_empty()).then(|| label.to_string())
}

/// The switcher item's tooltip: the chord and nothing else.
pub(crate) fn screen_accel_label(index: usize) -> Option<String> {
    accel_label(&screen_accel(index)?)
}

/// The model-calls switch's tooltip, for the same reason.
pub(crate) fn toggle_accel_label() -> Option<String> {
    accel_label(TOGGLE_ACCEL)
}

/// Attach both sets of accelerators to the window.
///
/// The screen action takes the stack name as a target and resolves it
/// against `SCREENS` before touching the stack: an action activated with a
/// name no page carries would otherwise be a silent no-op, which is the
/// failure `PRIVATE_INFERENCE_SCREEN` exists to prevent elsewhere.
///
/// The toggle asks `tray::toggle_request` and hands the answer to the same
/// `apply_tray_request` the tray menu uses. That is deliberate reuse rather
/// than a second copy of the rule: while answering is off the request is an
/// `Open`, so the chord navigates and writes nothing at all.
pub(crate) fn wire(app: &Rc<App>, application: &adw::Application) {
    let screen = gio::SimpleAction::new(SCREEN_ACTION, Some(glib::VariantTy::STRING));
    let target = Rc::clone(app);
    screen.connect_activate(move |_, parameter| {
        let Some(wanted) = parameter.and_then(glib::Variant::str) else {
            return;
        };
        let Some((name, _, _)) = SCREENS.iter().find(|(name, _, _)| *name == wanted) else {
            return;
        };
        target.stack.set_visible_child_name(name);
        target.window.present();
    });
    app.window.add_action(&screen);

    let toggle = gio::SimpleAction::new(TOGGLE_ACTION, None);
    let target = Rc::clone(app);
    toggle.connect_activate(move |_, _| {
        super::apply_tray_request(
            &target,
            crate::tray::toggle_request(target.tray.answering()),
        );
    });
    app.window.add_action(&toggle);

    for (name, accel) in screen_accels() {
        application.set_accels_for_action(&screen_action(name), &[&accel]);
    }
    application.set_accels_for_action(&format!("win.{TOGGLE_ACTION}"), &[TOGGLE_ACCEL]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The accelerators must be attached from `SCREENS`, not written out.
    ///
    /// Scanned rather than asserted over a value, because the drift this
    /// guards against is somebody listing four accelerators by hand: that
    /// version passes any test that only compares the list it produced.
    ///
    /// Only the module's body is read, never this test module. A sweep that
    /// reads its own needles finds them and reports green over an empty
    /// file -- which is exactly what the first draft of this did.
    #[test]
    fn the_screen_accelerators_are_attached_from_the_screens_array() {
        let code: String = include_str!("shortcuts.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("set_accels_for_action"),
            "this shell attaches no accelerators at all"
        );
        assert!(
            code.contains("SCREENS"),
            "the accelerators must come from ui::SCREENS"
        );
        // The numbering is computed. A digit written out is the drift.
        for digit in '1'..='9' {
            assert!(
                !code.contains(&format!("<Control>{digit}")),
                "accelerator <Control>{digit} is written out; the digit must come from the \
                 screen's position in SCREENS"
            );
        }
    }

    /// One accelerator per screen, in the switcher's own order.
    ///
    /// The pairing is what matters: `<Control>2` must reach whatever screen
    /// is second in `SCREENS` today, not whatever screen was second when
    /// this was written. Nothing below names a screen.
    #[test]
    fn every_screen_gets_the_accelerator_for_its_position() {
        let accels = screen_accels();
        assert_eq!(
            accels.len(),
            SCREENS.len(),
            "a screen without an accelerator, or one too many"
        );
        for (index, ((name, accel), (screen, _, _))) in
            accels.iter().zip(SCREENS.iter()).enumerate()
        {
            assert_eq!(name, screen, "the accelerators are out of switcher order");
            assert_eq!(*accel, format!("<Control>{}", index + 1));
        }
    }

    /// A tenth screen gets no chord rather than a wrong one.
    #[test]
    fn a_position_past_a_single_digit_has_no_accelerator() {
        assert_eq!(screen_accel(0).as_deref(), Some("<Control>1"));
        assert_eq!(screen_accel(8).as_deref(), Some("<Control>9"));
        assert_eq!(screen_accel(9), None);
    }

    /// Every accelerator this shell attaches is a well-formed one.
    ///
    /// An accelerator GTK rejects is attached to nothing and reports no
    /// error at all, so it looks exactly like a working one from the code.
    ///
    /// Checked against the written form rather than by asking
    /// `gtk::accelerator_parse`: the gtk4 binding asserts the toolkit has
    /// been initialised, and these tests run with no display. Both forms
    /// below are the canonical spelling GTK's own parser produces.
    #[test]
    fn every_accelerator_is_well_formed() {
        for (name, accel) in screen_accels() {
            let digit = accel
                .strip_prefix("<Control>")
                .unwrap_or_else(|| panic!("{accel} for {name} is not a Control chord"));
            assert!(
                digit.len() == 1 && digit.chars().all(|c| ('1'..='9').contains(&c)),
                "{accel} for {name} must be Control and one digit 1-9"
            );
        }
        assert_eq!(TOGGLE_ACCEL, "<Control><Shift>m");
    }

    /// Nothing here may take a chord GTK, libadwaita or GNOME already own.
    ///
    /// The list is the set this shell could plausibly collide with, written
    /// down so the collision is checked rather than remembered:
    /// `<Control>question` is the only accelerator `GtkApplicationWindow`
    /// installs by default (`win.show-help-overlay`); `AdwTabView`'s tab
    /// chords are `<Alt>1`..`<Alt>9`, deliberately Alt and not Control; and
    /// GTK's text widgets reserve `<Control><Shift>u` for Unicode entry,
    /// `<Control><Shift>e` for the preedit and `<Control><Shift>a` for
    /// deselect-all. GNOME Shell's own bindings are Super- and
    /// `<Control><Alt>`-based and take no bare `<Control><digit>`.
    ///
    /// Compared as written forms, for the reason
    /// `every_accelerator_is_well_formed` gives: GTK's parser cannot be
    /// asked without a display. Every string on both sides is already in
    /// GTK's canonical spelling, modifiers in GTK's own order, so equal
    /// chords compare equal.
    #[test]
    fn no_accelerator_collides_with_a_toolkit_or_desktop_default() {
        let reserved = [
            "<Control>question",
            "<Alt>1",
            "<Alt>2",
            "<Alt>3",
            "<Alt>4",
            "<Alt>5",
            "<Alt>6",
            "<Alt>7",
            "<Alt>8",
            "<Alt>9",
            "<Control><Shift>u",
            "<Control><Shift>e",
            "<Control><Shift>a",
        ];
        let mut ours: Vec<String> = screen_accels()
            .into_iter()
            .map(|(_, accel)| accel)
            .collect();
        ours.push(TOGGLE_ACCEL.to_string());
        for accel in ours {
            assert!(
                !reserved.contains(&accel.as_str()),
                "{accel} is already spoken for by the toolkit or the desktop"
            );
        }
    }

    /// **The toggle chord cannot turn answering on.**
    ///
    /// Asserted as the no-write property itself and not as a label: while
    /// this computer is not answering, the chord's request is an `Open`, and
    /// `apply_tray_request` reaches `private_inference::turn_off` for one
    /// variant and for no other. So there is no state of the switch from
    /// which the chord writes `true`, and no press that can enable
    /// machine-wide exposure with the sentence about it off-screen.
    ///
    /// The match is exhaustive on purpose, the way `tray.rs`'s own guard is:
    /// a third variant does not compile until somebody comes back here.
    #[test]
    fn the_toggle_chord_can_only_stop_answering_and_can_never_start() {
        for answering in [false, true] {
            let request = crate::tray::toggle_request(answering);
            match request {
                crate::tray::TrayRequest::StopAnsweringModelCalls => assert!(
                    answering,
                    "the chord asked for a write while nothing was answering"
                ),
                crate::tray::TrayRequest::Open(screen) => {
                    assert!(!answering);
                    // Writes nothing, and lands where the exposure sentence
                    // is -- which is the same screen `connect_needs_exposure`
                    // sends a first connect to.
                    assert_eq!(screen, super::super::PRIVATE_INFERENCE_SCREEN);
                }
            }
        }
    }

    /// The chord does not hold a second copy of the asymmetry.
    ///
    /// It asks `tray::toggle_request`, which is where the rule lives. A
    /// module that decided the direction for itself would pass the test
    /// above and still drift from the tray menu the next time the rule
    /// moved.
    #[test]
    fn the_chord_asks_the_tray_for_the_rule_rather_than_restating_it() {
        let code: String = include_str!("shortcuts.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("tray::toggle_request"),
            "the toggle chord must ask tray::toggle_request for its direction"
        );
        assert!(
            !code.contains("turn_off") && !code.contains("set_settings"),
            "the chord must not write; it hands a TrayRequest to apply_tray_request"
        );
    }
}
