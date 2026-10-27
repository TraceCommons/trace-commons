/// Ron's #1146 words for the Traces graph footer
/// (`preview_copy::MonitorTracesGraphCopy`): the zoom and jump controls,
/// the range pill and each bar's text equivalent. Numbers are `{name}`
/// holes; a singular is its own line.
public struct MonitorTracesGraphCopy: MonitorWordTable {
    public let zoomOut: String
    public let zoomIn: String
    public let jumpToNow: String
    /// The glyphs on the zoom out, zoom in and jump-to-now pills.
    public let zoomOutGlyph: String
    public let zoomInGlyph: String
    public let jumpToNowGlyph: String
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
        case zoomOutGlyph = "zoom_out_glyph"
        case zoomInGlyph = "zoom_in_glyph"
        case jumpToNowGlyph = "jump_to_now_glyph"
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
/// its accessible names and the node cards.
/// Counted nouns fill the sentences' `{label}`; a singular is its own line.
public struct MonitorFlowMapCopy: MonitorWordTable {
    public let mapLabel: String
    public let zoomLabel: String
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
    public let legendNotWatched: String
    /// The folder pill's accessible names (`revealFolders(_:)`), and its
    /// text and name once every folder is drawn.
    public let revealFoldersOne: String
    public let revealFolders: String
    public let foldFolders: String
    public let foldFoldersLabel: String
    /// The tool group: its heading, a coin's waiting line, and the legend's
    /// lock (a tool's memory stays on this Mac).
    public let toolsHeading: String
    public let coinWaiting: String
    public let legendMemory: String
    /// The card under the pointer while a trace moves through the tool
    /// group (`transferNote(contributing:tool:)`).
    public let transferContributing: String
    public let transferKeeping: String
    public let transferDismissed: String
    public let transferTrace: String
    public let memoryStays: String
    public let transferContributeNote: String
    /// The contribute note on a batch from several tools, which names none.
    public let transferContributeNoteMixed: String
    public let transferKeepNote: String
    public let transferDismissNote: String
    /// A batch card's count, `{count}` the traces (owner, 2026-10-19).
    public let transferTraces: String
    /// The tools' own memory files, named where known (`memoryFiles(_:)`).
    public let memoryClaudeCode: String
    public let memoryCodex: String
    public let memoryOpencode: String
    public let memoryGeminiCli: String
    public let memoryCline: String
    /// Under this computer while a contribution gathers its traces, and
    /// after traces are kept.
    public let collected: String
    public let keptHereOne: String
    public let keptHere: String
    /// The Traces map's style switch in Settings, General: its card's
    /// title, the switch, and what it does.
    public let styleTitle: String
    public let styleToggle: String
    public let styleCaption: String
    public let credential: String
    public let noneFound: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case mapLabel = "map_label"
        case zoomLabel = "zoom_label"
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
        case legendNotWatched = "legend_not_watched"
        case revealFoldersOne = "reveal_folders_one"
        case revealFolders = "reveal_folders"
        case foldFolders = "fold_folders"
        case foldFoldersLabel = "fold_folders_label"
        case toolsHeading = "tools_heading"
        case coinWaiting = "coin_waiting"
        case legendMemory = "legend_memory"
        case transferContributing = "transfer_contributing"
        case transferKeeping = "transfer_keeping"
        case transferDismissed = "transfer_dismissed"
        case transferTrace = "transfer_trace"
        case memoryStays = "memory_stays"
        case transferContributeNote = "transfer_contribute_note"
        case transferContributeNoteMixed = "transfer_contribute_note_mixed"
        case transferKeepNote = "transfer_keep_note"
        case transferDismissNote = "transfer_dismiss_note"
        case transferTraces = "transfer_traces"
        case memoryClaudeCode = "memory_claude_code"
        case memoryCodex = "memory_codex"
        case memoryOpencode = "memory_opencode"
        case memoryGeminiCli = "memory_gemini_cli"
        case memoryCline = "memory_cline"
        case collected
        case keptHereOne = "kept_here_one"
        case keptHere = "kept_here"
        case styleTitle = "style_title"
        case styleToggle = "style_toggle"
        case styleCaption = "style_caption"
        case credential
        case noneFound = "none_found"
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
    /// A count of traces in the core's own words: "1 trace", "12 traces".
    public func tracesCounted(_ count: Int) -> String {
        Self.counted(count, one: tracesOne, many: traces)
    }

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

    /// The "+N" pill's accessible name: how many folders the click draws.
    public func revealFolders(_ count: Int) -> String {
        count == 1 ? revealFoldersOne : revealFolders.replacingOccurrences(of: "{count}", with: String(count))
    }

    /// A coin's line under its name: the traces waiting from that tool.
    public func coinWaiting(_ count: Int) -> String {
        coinWaiting.replacingOccurrences(of: "{count}", with: String(count))
    }

    /// The transfer card's note on the memory text inside the trace.
    public func transferNote(contributing: Bool, tool: String) -> String {
        contributing ? transferContributeNote.replacingOccurrences(of: "{tool}", with: tool) : transferKeepNote
    }

    /// A batch card's count line: "{count} traces".
    public func traces(_ count: Int) -> String {
        transferTraces.replacingOccurrences(of: "{count}", with: String(count))
    }

    /// A tool's own memory files, by its source name; nil for a tool whose
    /// files the core does not name.
    public func memoryFiles(_ source: String) -> String? {
        switch source {
        case "claude-code": memoryClaudeCode
        case "codex": memoryCodex
        case "opencode": memoryOpencode
        case "gemini-cli": memoryGeminiCli
        case "cline": memoryCline
        default: nil
        }
    }

    /// Under this computer while a contribution gathers its traces.
    public func collected(_ count: Int, of total: Int) -> String {
        collected.replacingOccurrences(of: "{count}", with: String(count))
            .replacingOccurrences(of: "{total}", with: String(total))
    }

    /// Under this computer once traces are kept.
    public func keptHere(_ count: Int) -> String {
        count == 1 ? keptHereOne : keptHere.replacingOccurrences(of: "{count}", with: String(count))
    }

    /// The Private AI destination's card sentence and the line under its
    /// node: how many tools are connected.
    public func connected(tools count: Int, sentence: Bool) -> String {
        Self.fill(sentence ? connected : connectedLine, label: Self.counted(count, one: toolsOne, many: tools))
    }
}
