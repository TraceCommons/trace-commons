import Foundation
import TCShellCore

/// The pause words, held once so the glass panel (R13) offers the choices the
/// shipping menu did, in its words, until the core exports them (D-12). The
/// menu bar itself is the glass strip and panel (`MenuBarGlassPanel.swift`).
enum MenuBarWords {
    static let resume = "Resume watching"
    static let pause = "Pause"
    static let pauseHour = "For 1 hour"
    static let pauseMorning = "Until tomorrow morning"
    static let pauseIndefinite = "Until I turn it back on"

    /// The three pause lengths, in the panel's order: nil is until resumed.
    static func pauseUntil(_ choice: Int) -> Date? {
        switch choice {
        case 0: Date().addingTimeInterval(3600)
        case 1: Format.tomorrowMorning()
        default: nil
        }
    }
}
