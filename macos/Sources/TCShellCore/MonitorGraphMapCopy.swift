/// Ron's #1146 words for the Traces graph footer
/// (`preview_copy::MonitorTracesGraphCopy`): the zoom and jump controls,
/// the range pill and each bar's text equivalent. Numbers are `{name}`
/// holes; a singular is its own line.
public struct MonitorTracesGraphCopy: MonitorWordTable {
    public let zoomOut: String
    public let zoomIn: String
    public let jumpToNow: String
    public let lastHours: String
    public let lastDays: String
    public let hoursBackOne: String
    public let hoursBack: String
    public let daysBackOne: String
    public let daysBack: String
    public let bar: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case zoomOut = "zoom_out"
        case zoomIn = "zoom_in"
        case jumpToNow = "jump_to_now"
        case lastHours = "last_hours"
        case lastDays = "last_days"
        case hoursBackOne = "hours_back_one"
        case hoursBack = "hours_back"
        case daysBackOne = "days_back_one"
        case daysBack = "days_back"
        case bar
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }

    /// The range pill when no bar is hovered: the window's span at now, or
    /// how many whole windows back it is. `hours` false is a span of days.
    public func range(span: Int, hours: Bool, back: Int) -> String {
        let unit = hours ? "{hours}" : "{days}"
        let word: String
        switch back {
        case ...0: word = hours ? lastHours : lastDays
        case 1: word = hours ? hoursBackOne : daysBackOne
        default: word = hours ? hoursBack : daysBack
        }
        return word
            .replacingOccurrences(of: unit, with: String(span))
            .replacingOccurrences(of: "{count}", with: String(back))
    }

    /// One bar's text equivalent: its day, then shared and kept.
    public func bar(day: String, shared: Int, kept: Int) -> String {
        bar.replacingOccurrences(of: "{label}", with: day)
            .replacingOccurrences(of: "{count}", with: String(shared))
            .replacingOccurrences(of: "{total}", with: String(kept))
    }
}

/// Ron's #1146 words for the flow map (`preview_copy::MonitorFlowMapCopy`):
/// its accessible names, the node cards and the hint under a peeked card.
/// Counted nouns fill the sentences' `{label}`; a singular is its own line.
public struct MonitorFlowMapCopy: MonitorWordTable {
    public let mapLabel: String
    public let zoomLabel: String
    public let hint: String
    public let sessionsOne: String
    public let sessions: String
    public let tracesOne: String
    public let traces: String
    public let foldersOne: String
    public let folders: String
    public let toolsOne: String
    public let tools: String
    public let hub: String
    public let library: String
    public let toolTitle: String
    public let toolWatched: String
    public let toolWaiting: String
    public let toolNothingWaiting: String
    public let toolOff: String
    public let toolUnset: String
    public let folderRule: String
    public let folderRuleUnset: String
    public let folderCounts: String
    public let connected: String
    public let connectedLine: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case mapLabel = "map_label"
        case zoomLabel = "zoom_label"
        case hint
        case sessionsOne = "sessions_one"
        case sessions
        case tracesOne = "traces_one"
        case traces
        case foldersOne = "folders_one"
        case folders
        case toolsOne = "tools_one"
        case tools
        case hub
        case library
        case toolTitle = "tool_title"
        case toolWatched = "tool_watched"
        case toolWaiting = "tool_waiting"
        case toolNothingWaiting = "tool_nothing_waiting"
        case toolOff = "tool_off"
        case toolUnset = "tool_unset"
        case folderRule = "folder_rule"
        case folderRuleUnset = "folder_rule_unset"
        case folderCounts = "folder_counts"
        case connected
        case connectedLine = "connected_line"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }

    /// What a tool's card says about it: watched, off, or not set up.
    public enum ToolState: Sendable { case watched, off, unset, unknown }

    /// A number, or a dash for one the core has not reported: an unread
    /// count is never said as zero.
    static func figure(_ count: Int?) -> String {
        count.map(String.init) ?? "\u{2014}"
    }

    /// A counted noun: the singular is its own line.
    static func counted(_ count: Int?, one: String, many: String) -> String {
        count == 1 ? one : many.replacingOccurrences(of: "{count}", with: figure(count))
    }

    static func fill(_ word: String, label: String, count: Int?? = .none) -> String {
        var filled = word.replacingOccurrences(of: "{label}", with: label)
        if case .some(let count) = count { filled = filled.replacingOccurrences(of: "{count}", with: figure(count)) }
        return filled
    }

    /// This computer's card.
    public func hub(waiting: Int, contributed: Int?) -> String {
        Self.fill(hub, label: Self.counted(waiting, one: sessionsOne, many: sessions), count: contributed)
    }

    /// The library's card.
    public func library(contributed: Int?) -> String {
        Self.fill(library, label: Self.counted(contributed, one: tracesOne, many: traces))
    }

    /// A tool's card title: its name and how many folders it has.
    public func toolTitle(tool: String, folders count: Int) -> String {
        Self.fill(toolTitle, label: Self.counted(count, one: foldersOne, many: folders))
            .replacingOccurrences(of: "{tool}", with: tool)
    }

    /// A tool's card: what watching it means, then what waits. A tool the
    /// core could not say about says only what waits: unknown is never
    /// said as off or as not set up.
    public func tool(_ state: ToolState, waiting: Int) -> String {
        let waits = waiting > 0
            ? toolWaiting.replacingOccurrences(of: "{count}", with: String(waiting)) : toolNothingWaiting
        switch state {
        case .watched: return toolWatched + " " + waits
        case .off: return toolOff
        case .unset: return toolUnset
        case .unknown: return waits
        }
    }

    /// A folder's card: its path when the core named one, its rule (nil
    /// for none set), and its counts.
    public func folder(path: String?, rule: String?, waiting: Int, contributed: Int?) -> String {
        let ruleLine = rule.map { Self.fill(folderRule, label: $0) } ?? folderRuleUnset
        let counts = Self.fill(
            folderCounts, label: Self.counted(waiting, one: sessionsOne, many: sessions), count: contributed)
        let place = path.flatMap { $0.isEmpty ? nil : $0 + "." }
        return [place, ruleLine, counts].compactMap { $0 }.joined(separator: " ")
    }

    /// The Private AI destination's card sentence and the line under its
    /// node: how many tools are connected.
    public func connected(tools count: Int, sentence: Bool) -> String {
        Self.fill(sentence ? connected : connectedLine, label: Self.counted(count, one: toolsOne, many: tools))
    }
}
