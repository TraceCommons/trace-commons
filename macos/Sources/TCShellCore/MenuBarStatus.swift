import Foundation

/// What the status item shows, in the order the shared design fixes: a
/// count of decisions owed, then unhealthy, then paused, then idle.
///
/// The count is DECISIONS OWED -- entries waiting for a yes or no -- and
/// never queue depth, never credit. If it says 3 there are exactly three
/// things to decide. Every state that is not idle carries a glyph or a
/// figure as well as the mark, because a dimmed mark on its own is not a
/// state anybody can read.
public enum MenuBarState: Equatable {
    /// The badge text, already formatted -- see `MenuBarStatus.badgeText`.
    case count(String, paused: Bool = false)
    case attention
    case paused
    case idle

    public var isPaused: Bool {
        switch self {
        case .paused: return true
        case .count(_, let paused): return paused
        default: return false
        }
    }
}

public enum MenuBarStatus {
    /// The most the badge will state. Past this it says `99+`: a status
    /// item is not a place to read three digits, and past two of them the
    /// number stops meaning "this many" and starts meaning "a lot".
    public static let badgeCap = 99

    /// The badge's text, or nil when there is nothing to decide.
    public static func badgeText(decisionsOwed: Int) -> String? {
        guard decisionsOwed > 0 else { return nil }
        return decisionsOwed > badgeCap ? "\(badgeCap)+" : "\(decisionsOwed)"
    }

    /// Precedence: count, then unhealthy, then paused, then idle. A paused
    /// watcher with three decisions waiting shows the three; the pause is
    /// stated in the menu, and the mark dims either way.
    public static func state(decisionsOwed: Int, unhealthy: Bool, paused: Bool, available: Bool = true) -> MenuBarState {
        guard available else { return .attention }
        if let text = badgeText(decisionsOwed: decisionsOwed) { return .count(text, paused: paused) }
        if unhealthy { return .attention }
        if paused { return .paused }
        return .idle
    }
}

public extension MenuBarStatus {
    static func accessibilityLabel(decisionsOwed: Int, unhealthy: Bool, paused: Bool, available: Bool = true) -> String {
        guard available else { return "Trace Commons. Watcher unavailable. Needs attention." }
        let detail: String
        if decisionsOwed > 0 {
            detail = "\(decisionsOwed) \(decisionsOwed == 1 ? "session" : "sessions") waiting for your decision."
        } else if unhealthy { detail = "Needs attention." }
        else { detail = "Nothing waiting." }
        return "Trace Commons. " + detail + (paused ? " Paused." : "")
    }
}
