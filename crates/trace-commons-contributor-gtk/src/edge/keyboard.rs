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

/// How a layer-shell rail asks for the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Not at all: the compositor never gives it the keyboard.
    Nothing,
    /// When the compositor decides: on the press on KWin 6.6.6, on the next
    /// pointer motion on Hyprland 0.56.2 (2026-11-18). Never outright: on
    /// Hyprland an exclusive rail also took every pointer event, so a click
    /// on another window went to the rail.
    OnDemand,
}

/// What the rail asks for when it takes the keyboard (`takes`) or not.
pub fn ask(takes: bool) -> Ask {
    if takes { Ask::OnDemand } else { Ask::Nothing }
}

/// The rail's keyboard, as it opens, closes and is clicked.
#[derive(Debug, Default)]
pub struct RailKeyboard {
    clicked: Cell<bool>,
    /// Mapped afresh, and not yet moved over: an enter alone is the
    /// fresh map under a parked pointer.
    fresh: Cell<bool>,
}

impl RailKeyboard {
    /// The closed rail was placed, `still_holds` whether its window still
    /// has the keyboard. Answers whether to map it afresh: KWin 6.6.6
    /// (2026-11-18) kept the keyboard on a rail that asked for none, and a
    /// surface mapped afresh starts without it.
    pub fn placed_closed(&self, still_holds: bool) -> bool {
        if still_holds {
            self.fresh.set(true);
        }
        still_holds
    }

    /// The pointer entered the closed rail. Answers whether to open it:
    /// yes, unless the rail was just mapped afresh, when the pointer may
    /// only be parked there.
    pub fn entered(&self) -> bool {
        !self.fresh.get()
    }

    /// The pointer moved inside the closed rail. Answers whether to open
    /// it: only the first motion after a fresh map, which `entered` held
    /// back; otherwise the enter has already opened it.
    pub fn moved(&self) -> bool {
        self.fresh.replace(false)
    }

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

    /// On demand, never outright. Hyprland 0.56.2 (2026-11-18), asked
    /// outright, gave the rail the keyboard on the press but also every
    /// pointer event after it: a click on another window went to the rail,
    /// which stayed open until Escape. On demand, it gives the keyboard only
    /// on the next pointer motion; KWin 6.6.6 gives it on the press.
    #[test]
    fn a_press_asks_for_the_keyboard_on_demand() {
        assert_eq!(ask(true), Ask::OnDemand);
        assert_eq!(ask(false), Ask::Nothing);
    }

    /// KWin 6.6.6 (2026-11-18) kept the keyboard on the closed rail after it
    /// asked for none; a surface mapped afresh starts without it. Hyprland
    /// took it back at once, so there the rail is not mapped again.
    #[test]
    fn a_closed_rail_still_holding_the_keyboard_is_mapped_afresh() {
        let keyboard = RailKeyboard::default();
        assert!(keyboard.placed_closed(true));
        assert!(!RailKeyboard::default().placed_closed(false));
    }

    /// Mapped afresh under a parked pointer, the rail is entered without
    /// the pointer moving; that alone does not open it, as it would not on
    /// the Mac. The next motion does, so a real hover still opens it.
    #[test]
    fn mapped_afresh_it_opens_on_motion_not_on_the_enter() {
        let keyboard = RailKeyboard::default();
        keyboard.placed_closed(true);
        assert!(!keyboard.entered(), "opened under a parked pointer");
        assert!(keyboard.moved(), "a hover after the fresh map stayed shut");
        assert!(keyboard.entered(), "the next enter is an ordinary hover");
        assert!(!keyboard.moved(), "motion opened it twice");
    }

    #[test]
    fn an_ordinary_enter_opens_it() {
        let keyboard = RailKeyboard::default();
        assert!(keyboard.entered());
        assert!(!keyboard.moved());
        keyboard.placed_closed(false);
        assert!(keyboard.entered());
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
