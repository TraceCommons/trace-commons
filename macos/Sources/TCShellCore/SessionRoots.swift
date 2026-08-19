import Foundation

/// What the contributor has said about one agent's session store.
///
/// Three states, not two, and the third is the one that matters. An unset
/// root does not mean "no source for that agent" -- the daemon reads it as
/// the conventional per-user location, i.e. the contributor's real
/// `~/.claude/projects` or `~/.codex/sessions`. So "I do not use Codex" has
/// to be something the declaration can SAY, not something it omits.
/// `undecided` is what the screen opens in, because nothing is
/// pre-selected; it is never sent.
public enum SourceChoice: Equatable, Sendable {
    /// Not answered yet. The screen's opening state, and never persisted.
    case undecided
    /// Watch this folder.
    case watch(path: String)
    /// This agent is not used here. Watch nothing for it, and do not fall
    /// back to the conventional location.
    case off

    var trimmed: SourceChoice {
        guard case .watch(let path) = self else { return self }
        return .watch(path: path.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    /// An answer the daemon can be given. A `watch` with no path is still an
    /// unfinished row, not a declaration.
    var isAnswered: Bool {
        switch trimmed {
        case .undecided: return false
        case .watch(let path): return !path.isEmpty
        case .off: return true
        }
    }

    /// The `{"mode":...}` object the settings validator parses, or nil while
    /// this row is unfinished.
    var declaration: [String: String]? {
        switch trimmed {
        case .undecided: return nil
        case .off: return ["mode": "off"]
        case .watch(let path):
            return path.isEmpty ? nil : ["mode": "watch", "path": path]
        }
    }
}

/// The two answers the roots screen collects, and the settings object that
/// declares them to the daemon.
///
/// BOTH, always -- but "both answered" now includes answering no. The rule
/// itself is not restated here: `daemon::settings::roots_declared` owns it
/// and the C ABI enforces it. This type only refuses to send something it
/// already knows will be refused, so an unfinished screen reads as
/// unfinished instead of as an error from across the boundary.
public struct SessionRoots: Equatable, Sendable {
    public var claude: SourceChoice
    public var codex: SourceChoice

    public init(claude: SourceChoice = .undecided, codex: SourceChoice = .undecided) {
        self.claude = claude
        self.codex = codex
    }

    public subscript(kind: SourceKind) -> SourceChoice {
        get { kind == .claudeCode ? claude : codex }
        set { if kind == .claudeCode { claude = newValue } else { codex = newValue } }
    }

    /// Adopt the path discovery found for this candidate. One source only --
    /// answering for the other is exactly the pre-selection this screen does
    /// not do.
    public mutating func watch(_ candidate: SourceCandidate) {
        self[candidate.source] = .watch(path: candidate.path)
    }

    public var isComplete: Bool {
        claude.isAnswered && codex.isAnswered
    }

    /// The `settings_json` argument for the settings-bearing daemon start,
    /// or nil when either row is unfinished.
    ///
    /// Serialized, never concatenated: these paths come from a file panel or
    /// from discovery and may contain quotes or backslashes.
    ///
    /// Emits `claude_source` / `codex_source` and never the older
    /// `claude_root` / `codex_root` spelling. Both are accepted by the
    /// validator, but only one of them can say `off`, and a shell that used
    /// whichever fit would be speaking two dialects of the same file.
    public func settingsJSON() -> String? {
        guard isComplete,
            let claudeDeclaration = claude.declaration,
            let codexDeclaration = codex.declaration
        else { return nil }

        let object: [String: Any] = [
            "claude_source": claudeDeclaration,
            "codex_source": codexDeclaration,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: object),
            let json = String(data: data, encoding: .utf8)
        else { return nil }
        return json
    }
}
