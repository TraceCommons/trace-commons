import Foundation

/// Which agent's session store a candidate describes.
///
/// The slugs match the adapter names the Rust side uses. An unrecognized one
/// is dropped rather than surfaced: a build that has never heard of a future
/// adapter should show the contributor one fewer row, not fail to render the
/// screen that starts the daemon.
public enum SourceKind: String, Equatable, Sendable {
    case claudeCode = "claude-code"
    case codex
    case geminiCli = "gemini-cli"

    public var displayName: String {
        switch self {
        case .claudeCode: return "Claude Code"
        case .codex: return "Codex"
        case .geminiCli: return "Gemini CLI"
        }
    }
}

/// One candidate session store, as `tc_discover_sources` describes it.
///
/// This exists so the roots screen can ask about something specific.
/// Discovery is not consent -- nothing here selects anything -- but a
/// contributor agreeing to "953 sessions, most recent 2 hours ago" is
/// agreeing to something they can actually picture, which an empty text
/// field asking for a path from memory is not.
public struct SourceCandidate: Equatable, Sendable {
    public let source: SourceKind
    public let path: String
    public let exists: Bool
    public let sessionCount: UInt64
    public let mostRecent: Date?
    /// Whether `CLAUDE_CONFIG_DIR` / `CODEX_HOME` moved this store, so the
    /// screen can explain a path the contributor was not expecting.
    public let relocatedByEnv: Bool

    public init(
        source: SourceKind,
        path: String,
        exists: Bool,
        sessionCount: UInt64,
        mostRecent: Date?,
        relocatedByEnv: Bool
    ) {
        self.source = source
        self.path = path
        self.exists = exists
        self.sessionCount = sessionCount
        self.mostRecent = mostRecent
        self.relocatedByEnv = relocatedByEnv
    }

    /// The one line under the path that says what agreeing would mean.
    ///
    /// "Not there" and "there but empty" are kept apart on purpose. They
    /// look alike in a count and they are not alike in a decision: one says
    /// the contributor does not use this agent, the other says they do and
    /// have not started a session yet.
    public func evidence(now: Date) -> String {
        var line: String
        if !exists {
            line = "Not found on this machine"
        } else if sessionCount == 0 {
            line = "Found, but holding no sessions yet"
        } else {
            let noun = sessionCount == 1 ? "session" : "sessions"
            line = "\(sessionCount) \(noun)"
            if let mostRecent {
                line += ", most recent \(Self.age(of: mostRecent, at: now))"
            }
        }
        if relocatedByEnv {
            line += " (moved here by an environment variable)"
        }
        return line
    }

    /// Deliberately not `RelativeDateTimeFormatter`: this string is asserted
    /// in tests, and a locale- and OS-dependent formatter turns those
    /// assertions into something that passes on the machine that wrote them.
    /// The vocabulary here is small because the decision it supports is
    /// small -- recent, or not.
    static func age(of date: Date, at now: Date) -> String {
        let seconds = max(0, now.timeIntervalSince(date))
        switch seconds {
        case ..<120: return "just now"
        case ..<3600: return "\(Int(seconds / 60)) minutes ago"
        case ..<7200: return "1 hour ago"
        case ..<86400: return "\(Int(seconds / 3600)) hours ago"
        case ..<172_800: return "yesterday"
        default: return "\(Int(seconds / 86400)) days ago"
        }
    }

    /// Decode the JSON array `tc_discover_sources` returns.
    public static func decodeList(from json: String) throws -> [SourceCandidate] {
        let wire = try JSONDecoder().decode([Wire].self, from: Data(json.utf8))
        return wire.compactMap(\.candidate)
    }

    /// The wire shape, kept separate from the model so an unknown `source`
    /// slug can be dropped during mapping instead of throwing and taking the
    /// whole array with it.
    private struct Wire: Decodable {
        let source: String
        let path: String
        let exists: Bool
        let sessionCount: UInt64
        let mostRecent: String?
        let relocatedByEnv: Bool

        enum CodingKeys: String, CodingKey {
            case source
            case path
            case exists
            case sessionCount = "session_count"
            case mostRecent = "most_recent"
            case relocatedByEnv = "relocated_by_env"
        }

        var candidate: SourceCandidate? {
            guard let kind = SourceKind(rawValue: source) else { return nil }
            return SourceCandidate(
                source: kind,
                path: path,
                exists: exists,
                sessionCount: sessionCount,
                mostRecent: mostRecent.flatMap(Wire.parseTimestamp),
                relocatedByEnv: relocatedByEnv
            )
        }

        /// The Rust side emits RFC 3339 with NANOSECOND precision, e.g.
        /// `2026-08-19T12:28:34.412518838Z`. `ISO8601DateFormatter` with
        /// `.withFractionalSeconds` handles three digits and returns nil for
        /// nine, which would silently blank every timestamp -- a store
        /// holding three thousand sessions would render as though it held
        /// none dated. So the fraction is truncated to milliseconds before
        /// parsing, and a plain second-precision stamp is still accepted.
        static func parseTimestamp(_ raw: String) -> Date? {
            let fractional = ISO8601DateFormatter()
            fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            let plain = ISO8601DateFormatter()
            plain.formatOptions = [.withInternetDateTime]

            if let date = fractional.date(from: raw) ?? plain.date(from: raw) {
                return date
            }
            guard let dot = raw.firstIndex(of: "."),
                let zone = raw[dot...].firstIndex(where: { $0 == "Z" || $0 == "+" || $0 == "-" })
            else { return nil }
            let digits = raw[raw.index(after: dot)..<zone].prefix(3)
            let truncated = raw[..<dot] + "." + digits + raw[zone...]
            return fractional.date(from: String(truncated))
        }
    }
}
