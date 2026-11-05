import CTraceCommons
import Foundation
import TCLocalization

/// The settings screen's session-source rows, read from the Rust rather than
/// written here.
///
/// Handle-free like `TCRoutingCopy`: this is wording about a declaration, not
/// a call into a running daemon.
///
/// Nothing in this file is a word. The sentence crosses already assembled, so
/// there is no template for this shell to fill in and therefore no fourth
/// place the wording can drift. The GTK and Windows shells render the same
/// row from the same Rust.
public enum TCSourceChecks {
    public static func settingsCopy() -> SourceSettingsCopy? {
        guard let raw = tc_source_settings_copy() else { return nil }
        defer { tc_string_free(raw) }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let json = Localization.current.copyJSON(String(cString: raw), namespace: "source_settings_copy") else { return nil }
        return try? decoder.decode(SourceSettingsCopy.self, from: Data(json.utf8))
    }

    /// The wire key for Claude Code's session source.
    public static let claude = "claude"

    /// The wire key for Codex's session source.
    public static let codex = "codex"

    /// The wire key for Gemini CLI's session source.
    public static let gemini = "gemini"

    /// The wire key for Cline's session source.
    public static let cline = "cline"

    /// The wire key for OpenCode's session source.
    public static let opencode = "opencode"

    /// One tool's row, from `get_settings`'s `*_source_mode` -- `watch`,
    /// `off` or `unset`.
    ///
    /// PASS THE MODE, NOT THE BOOLEAN. `claudeRootConfigured` is
    /// `mode == "watch"`, so it is false for a source the contributor
    /// declared OFF as well as for one nobody was asked about. The GTK and
    /// Windows shells branched on it and told a contributor who does not use
    /// Claude Code that its sessions were being read from the usual place.
    /// Nothing is read from an off source. What an UNSET source means is per
    /// tool -- claude and codex are scanned where they usually live, gemini
    /// and cline construct no adapter and open nothing -- so never carry one
    /// tool's unset sentence to another. The Rust picks the words from the
    /// mode and the tool together.
    ///
    /// Nil when the ABI refused -- an unknown tool key, or a caught panic.
    /// The caller shows nothing rather than a sentence written in Swift.
    public static func checkLine(tool: String, sourceMode: String) -> String? {
        // Legacy equivalent: tc_source_check_line.
        LocalizedPresentation.line("source_check_line", parameters: ["tool": tool, "source_mode": sourceMode])
    }
}

public struct SourceSettingsCopy: Decodable, Sendable {
    public let opencodeVersionTitle, opencodeVersionDetail: String?
    public let heading, explanation, saveFailed, consentSaveFailed, unavailable: String
    public let selectedFolder, noCandidate, watchCandidate, chooseFolder, retry: String
    /// `{tool}`, `{action}`: a row button's accessible name, so the same
    /// button on every row is told apart. Nil from an older core.
    public let toolActionAccessibilityName: String?
    /// The line under a found folder. Nil from an older core, and then no
    /// line is drawn.
    public let evidence: Evidence?
    public let tools: [String: Tool]
    /// The row for a declared folder of exported traces; nil from a core
    /// that does not carry one, and then no row is drawn.
    public let trajectory: Trajectory?
    public struct Tool: Decodable, Sendable {
        public let key, decline: String
        public let explanation, chooseFolder: String?
        public let unsetScansConventional: Bool
    }
    /// See `source_copy::SourceEvidenceCopy`.
    public struct Evidence: Decodable, Sendable {
        public let missing, empty, countOne, count, mostRecent, relocated: String
        public let ageJustNow, ageMinutes, ageHour, ageHours, ageYesterday, ageDays: String
    }
    /// A row button's accessible name: its visible words and the row's
    /// tool, or the visible words alone from an older core.
    public func toolActionName(_ action: String, tool: String) -> String {
        guard let template = toolActionAccessibilityName else { return action }
        return template.replacingOccurrences(of: "{tool}", with: tool)
            .replacingOccurrences(of: "{action}", with: action)
    }
    public struct Trajectory: Decodable, Sendable {
        public let title, explanation, watching, off, decline: String
    }
}
