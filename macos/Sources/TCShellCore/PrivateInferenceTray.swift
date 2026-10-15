import Foundation

/// What a press on the menu bar's Private AI row does.
///
/// Two values rather than a toggle, because the two directions are not
/// symmetrical. Turning it OFF only ever reduces what this computer will
/// answer, so it is safe from a menu with nothing else on screen. Turning
/// it ON changes what anything else running on this computer may send
/// through, charged to the contributor's own accounts, and the sentence
/// saying so (`offer_exposure`) is the reason model calls became a
/// top-level destination rather than a settings switch. A menu press that
/// enabled it would route around that sentence, so out here the on
/// direction opens the screen and writes nothing.
public enum PrivateInferenceTray {
    public enum Action: Equatable, Sendable {
        /// Stop answering. The one write the menu bar may make.
        case stopAnswering
        /// Raise the window at the destination. Writes nothing.
        case openDestination
    }

    /// The action for one switch position.
    ///
    /// Reads the switch and not the tone, deliberately: a listener that
    /// refused to start still leaves something to turn off, and a row that
    /// vanished because nothing was running would strand the one press the
    /// menu bar exists to offer.
    public static func action(on: Bool) -> Action {
        on ? .stopAnswering : .openDestination
    }

    /// The row's words, both of them the Rust's. The off direction's ends in
    /// an ellipsis, which is this platform's convention for an item that
    /// opens something rather than acting -- see the copy module.
    public static func label(on: Bool, copy: PrivateInferenceCopy) -> String {
        on ? copy.trayTurnOff : copy.trayOpenToTurnOn
    }

    /// Runs the action, with the two things it can do handed in so a test can
    /// assert the one that matters: while it is off, `turnOff` is never
    /// called. That is the safety claim, and it is checked directly rather
    /// than inferred from which sentence the row was showing.
    ///
    /// The stop direction raises the destination as well as writing, which
    /// is not decoration. The write goes through
    /// `PrivateInferenceSurface.settingsParams`, and that carries
    /// `private_inference_offer_seen` -- a marker whose entire contract is
    /// that it records an *asking*. A menu press showing no sentence at all
    /// would record a question nobody was asked and permanently suppress the
    /// first-run offer for a contributor who had enabled the switch out of
    /// band. On Windows and in GTK the switch only exists on the screen that
    /// carries `offer_exposure`, so their off press always shows those
    /// words; raising the destination here is what gives this shell the same
    /// property, and the destination shows `offer_exposure` whether or not
    /// the offer card is still due.
    public static func perform(on: Bool, turnOff: () -> Void, open: () -> Void) {
        switch action(on: on) {
        case .stopAnswering:
            turnOff()
            open()
        case .openDestination: open()
        }
    }
}
