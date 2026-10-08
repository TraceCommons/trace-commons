import Foundation

/// One kind whose layout a folder the contributor picked matches, as
/// `tc_describe_folder` reports it.
///
/// Decoded apart from `SourceCandidate` on purpose. `SourceCandidate` drops a
/// row whose kind it cannot name, which is right for the roots screen and
/// wrong here: `tc_describe_folder` reports every match or none, and a flat
/// folder of `.json` files is both an OpenCode export and a trajectory
/// export. Dropping the trajectory row would turn two matches into one, and
/// the shell would take as certain a folder it should ask about. So every
/// row is kept: zero matches is a refusal, one is a recognition, and more
/// than one is a question.
public struct FolderMatch: Equatable, Sendable {
    /// Which kind of store the folder looks like.
    public enum Kind: Equatable, Sendable {
        /// One of the session stores the roots screen offers.
        case source(SourceKind)
        /// A Letta trajectory export, declared as `trajectory_source`.
        case trajectory
        /// A kind this build has no name for. Kept so the number of matches
        /// stays true; the shell cannot offer it, but it must still ask.
        case unrecognised(String)

        /// The slug `tc_describe_folder` uses for a trajectory export.
        static let trajectorySlug = "trajectory"

        init(slug: String) {
            if let source = SourceKind(rawValue: slug) {
                self = .source(source)
            } else if slug == Kind.trajectorySlug {
                self = .trajectory
            } else {
                self = .unrecognised(slug)
            }
        }
    }

    public let kind: Kind
    /// The picked folder itself.
    public let path: String
    public let sessionCount: UInt64
    public let mostRecent: Date?
    /// As `SourceCandidate.answersAt`: a fixed label, or `nil`.
    public let answersAt: String?

    public init(
        kind: Kind,
        path: String,
        sessionCount: UInt64,
        mostRecent: Date?,
        answersAt: String? = nil
    ) {
        self.kind = kind
        self.path = path
        self.sessionCount = sessionCount
        self.mostRecent = mostRecent
        self.answersAt = answersAt
    }

    /// This match as a roots-screen candidate, for a kind the roots screen
    /// offers; `nil` for a trajectory export or an unrecognised kind.
    public var candidate: SourceCandidate? {
        guard case .source(let source) = kind else { return nil }
        return SourceCandidate(
            source: source,
            path: path,
            exists: true,
            sessionCount: sessionCount,
            mostRecent: mostRecent,
            relocatedByEnv: false,
            answersAt: answersAt
        )
    }

    /// Decode the JSON array `tc_describe_folder` returns, keeping every row.
    public static func decodeList(from json: String) throws -> [FolderMatch] {
        let wire = try JSONDecoder().decode([SourceCandidate.Wire].self, from: Data(json.utf8))
        return wire.map { row in
            FolderMatch(
                kind: Kind(slug: row.source),
                path: row.path,
                sessionCount: row.sessionCount,
                mostRecent: row.mostRecent.flatMap(SourceCandidate.Wire.parseTimestamp),
                answersAt: row.answersAt
            )
        }
    }
}
