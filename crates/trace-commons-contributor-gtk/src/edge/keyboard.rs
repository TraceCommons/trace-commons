//! Whether the rail takes the keyboard where it decides for itself, under
//! layer-shell (KDE Plasma, Hyprland and other wlroots compositors).
//!
//! Owner, 2026-11-18, "Keyboard only after a click": opening the rail by
//! hovering over it takes nothing, so typing carries on in the window in
//! front; a click inside the open rail gives it the keyboard, so Escape
//! and Tab reach it; closing it gives the keyboard back. On GNOME the
//! extension keeps the same rule for its windows (`gnome`). The Mac's rail
//! is a non-activating panel and never takes the keyboard; taking it after
//! a click is a deliberate difference, for Escape and Tab.

use std::cell::Cell;

/// The rail's keyboard, as it opens, closes and is clicked.
#[derive(Debug, Default)]
pub struct RailKeyboard {
    clicked: Cell<bool>,
}

impl RailKeyboard {
    /// The rail was drawn open or closed. Answers whether it takes the
    /// keyboard now: a closed rail never does, and forgets the click.
    pub fn placed(&self, open: bool) -> bool {
        if !open {
            self.clicked.set(false);
        }
        open && self.clicked.get()
    }

    /// The rail starts to close. Answers whether it takes the keyboard
    /// now: never, and the click is forgotten, so a rail opened again by
    /// hover before it finished closing takes nothing.
    pub fn closing(&self) -> bool {
        self.clicked.set(false);
        false
    }

    /// A press inside the rail. Answers whether it takes the keyboard now:
    /// only an open rail does.
    pub fn pressed(&self, open: bool) -> bool {
        if open {
            self.clicked.set(true);
        }
        open && self.clicked.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hovering_open_takes_nothing() {
        let keyboard = RailKeyboard::default();
        assert!(!keyboard.placed(true));
    }

    #[test]
    fn a_click_inside_the_open_rail_takes_the_keyboard() {
        let keyboard = RailKeyboard::default();
        keyboard.placed(true);
        assert!(keyboard.pressed(true));
    }

    #[test]
    fn closing_gives_it_back_and_the_next_hover_takes_nothing() {
        let keyboard = RailKeyboard::default();
        keyboard.placed(true);
        keyboard.pressed(true);
        assert!(!keyboard.placed(false), "a closed rail kept the keyboard");
        assert!(!keyboard.placed(true), "the old click carried over");
    }

    #[test]
    fn a_press_on_the_closed_rail_takes_nothing() {
        let keyboard = RailKeyboard::default();
        assert!(!keyboard.pressed(false));
        assert!(!keyboard.placed(true));
    }

    #[test]
    fn a_rail_opened_again_while_closing_takes_nothing() {
        let keyboard = RailKeyboard::default();
        keyboard.placed(true);
        keyboard.pressed(true);
        // Closing starts; a hover opens it again before the slide ends, so
        // the closed size is never placed.
        assert!(!keyboard.closing());
        assert!(!keyboard.placed(true), "the click outlived the close");
    }
}
