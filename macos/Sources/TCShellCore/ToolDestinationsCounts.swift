import Foundation

/// Counts the glass screens read off `tool_destinations`, in one place, so
/// Home and the menu-bar strip never each re-derive them (#1195, #1202).
extension DaemonData.ToolDestinations {
    /// Tools whose sessions the core reads now: `sessions.watch ==
    /// "watched"`. That includes a tool left unset that the core reads from
    /// its usual folder, which a count of tools set to watch would miss.
    public var watchedCount: Int {
        tools.filter { $0.sessions?.watch == "watched" }.count
    }
}
