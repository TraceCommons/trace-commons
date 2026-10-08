import Foundation
import TCShellCore

/// The pause words, from the core's shell table (#1146's `tray.rs` words;
/// owner ruling, 2026-10-06). The menu bar itself is the glass strip and
/// panel (`MenuBarGlassPanel.swift`).
enum MenuBarWords {
    static var resume: String { MonitorWords.table?.shell.resumeWatcher ?? "" }
    static var pause: String { MonitorWords.table?.shell.pauseWatcher ?? "" }
    static var pauseHour: String { MonitorWords.table?.shell.pauseHour ?? "" }
    static var pauseMorning: String { MonitorWords.table?.shell.pauseMorning ?? "" }
    static var pauseIndefinite: String { MonitorWords.table?.shell.pauseUntilResumed ?? "" }

    /// The three pause lengths, in the panel's order: nil is until resumed.
    static func pauseUntil(_ choice: Int) -> Date? {
        switch choice {
        case 0: Date().addingTimeInterval(3600)
        case 1: Format.tomorrowMorning()
        default: nil
        }
    }
}
