import Foundation

/// The two folders the contributor names on the roots screen, and the
/// settings object that declares them to the daemon.
///
/// BOTH, always. An unset root does not mean "no source for that agent" --
/// the daemon reads it as the conventional per-user location, i.e. the
/// contributor's real `~/.claude` or `~/.codex` -- so there is no such thing
/// as a half declaration worth sending. `settingsJSON()` returns nil rather
/// than build one.
///
/// The rule itself is not restated here: `daemon::settings::roots_declared`
/// owns it and the C ABI enforces it. This type only refuses to send
/// something it already knows will be refused, so an unfinished screen reads
/// as unfinished instead of as an error from across the boundary.
public struct SessionRoots: Equatable, Sendable {
    public var claude: String
    public var codex: String

    public init(claude: String = "", codex: String = "") {
        self.claude = claude
        self.codex = codex
    }

    private var trimmedClaude: String { claude.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var trimmedCodex: String { codex.trimmingCharacters(in: .whitespacesAndNewlines) }

    public var isComplete: Bool {
        !trimmedClaude.isEmpty && !trimmedCodex.isEmpty
    }

    /// The `settings_json` argument for the settings-bearing daemon start,
    /// or nil when the declaration is not complete.
    ///
    /// Serialized, never concatenated: these paths come from a file panel
    /// and may contain quotes or backslashes. It carries exactly the two
    /// recognized keys, because the settings validator rejects an unknown
    /// top-level key rather than ignoring it.
    public func settingsJSON() -> String? {
        guard isComplete else { return nil }
        let object = ["claude_root": trimmedClaude, "codex_root": trimmedCodex]
        guard let data = try? JSONSerialization.data(withJSONObject: object),
            let json = String(data: data, encoding: .utf8)
        else { return nil }
        return json
    }
}
