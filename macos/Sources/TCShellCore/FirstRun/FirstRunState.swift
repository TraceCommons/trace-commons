import Foundation

/// Ron's two tiers (#1030 `ftux-model.ts`). Quick is the default; "Custom
/// setup instead" switches. There is no chooser screen.
public enum FirstRunTier: String, Codable, Equatable, Sendable {
    case quick
    case custom
}

/// One #1030 screen. Which of them a tier shows is `FirstRunNavigation`'s.
public enum FirstRunStep: String, Codable, Equatable, Sendable {
    case join
    case folders
    case tools
    case rules
    case uses
}

/// What the person answered on Join about an account.
public enum AccountAnswer: Codable, Equatable, Sendable {
    /// Not answered yet; Join cannot continue.
    case none
    /// "Skip: watch only". No account, so nothing is joined and nothing can
    /// be shared automatically.
    case watchOnly
    /// Sign in with near.ai after the daemon starts.
    case nearAI
    /// A passkey the person named.
    case passkey(name: String)
}

/// The sharing decision on Uses.
public enum SharingPath: String, Codable, Equatable, Sendable {
    case askMe
    case automatic
}

/// A folder the person picked with "add your tool" on Tools, and the kind it
/// was recognised (or chosen) as.
///
/// No case for an unrecognised kind: a folder that matches nothing is
/// refused before it gets here, and nothing could be declared for it.
public struct AddedFolder: Codable, Equatable, Sendable {
    public enum Kind: Codable, Equatable, Sendable {
        /// A session store of a tool the roots screen offers -- a moved
        /// Claude Code or Codex folder, or OpenCode. It becomes that tool's
        /// answer.
        case source(SourceKind)
        /// A folder of exported traces, declared as `trajectory_source`.
        case trajectory
    }

    public let kind: Kind
    public let path: String

    public init(kind: Kind, path: String) {
        self.kind = kind
        self.path = path
    }
}

/// Every answer the person gives during the first run, and how far the
/// daemon calls have got. Pure: nothing here calls anything.
/// `FirstRunPlan` turns it into calls; the runner records their outcomes
/// back into `daemonStarted` and `enrolled`.
public struct FirstRunState: Codable, Equatable, Sendable {
    public var tier: FirstRunTier
    public var step: FirstRunStep
    /// The invite as pasted on Join. Kept through a failed start and a
    /// rejected lookup; only the person changes it.
    public var invite: String
    /// `TCInvite.issuerHost` of `invite`, for display.
    public var issuerHost: String?
    public var account: AccountAnswer
    /// One answer per offered tool. A missing key or `.undecided` is not an
    /// answer.
    public var toolAnswers: [SourceKind: SourceChoice]
    public var addedFolders: [AddedFolder]
    /// Rule per `project_id` (Custom only).
    public var rules: [String: ProjectMode]
    /// Chosen past sessions per `project_id` (Custom only).
    public var pastSelections: [String: Set<String>]
    public var scopes: Set<String>
    public var sharing: SharingPath
    /// The Private AI switch (Custom only).
    public var privateAI: Bool
    /// The witness the disclosure showed, passed to `grant_automatic`.
    public var witnessSigningAddress: String?
    public var daemonStarted: Bool
    public var enrolled: Bool

    public init(
        tier: FirstRunTier = .quick,
        step: FirstRunStep = .join,
        invite: String = "",
        issuerHost: String? = nil,
        account: AccountAnswer = .none,
        toolAnswers: [SourceKind: SourceChoice] = [:],
        addedFolders: [AddedFolder] = [],
        rules: [String: ProjectMode] = [:],
        pastSelections: [String: Set<String>] = [:],
        scopes: Set<String> = [],
        sharing: SharingPath = .askMe,
        privateAI: Bool = false,
        witnessSigningAddress: String? = nil,
        daemonStarted: Bool = false,
        enrolled: Bool = false
    ) {
        self.tier = tier
        self.step = step
        self.invite = invite
        self.issuerHost = issuerHost
        self.account = account
        self.toolAnswers = toolAnswers
        self.addedFolders = addedFolders
        self.rules = rules
        self.pastSelections = pastSelections
        self.scopes = scopes
        self.sharing = sharing
        self.privateAI = privateAI
        self.witnessSigningAddress = witnessSigningAddress
        self.daemonStarted = daemonStarted
        self.enrolled = enrolled
    }

    /// The tool answers and added folders as one declaration. An added folder
    /// for a tool is that tool's answer; a later one replaces an earlier one.
    /// Both Continue on Folders/Tools and the daemon start read this, so they
    /// cannot disagree about what is answered.
    public var sessionRoots: SessionRoots {
        var roots = SessionRoots()
        for (kind, choice) in toolAnswers {
            roots[kind] = choice
        }
        for folder in addedFolders {
            switch folder.kind {
            case .source(let kind): roots[kind] = .watch(path: folder.path)
            case .trajectory: roots.trajectory = .watch(path: folder.path)
            }
        }
        return roots
    }
}

// Codable for the core types the state holds, so a first run can be
// restored. `SourceKind` and `ProjectMode` get theirs from their raw values.
extension SourceKind: Codable {}
extension ProjectMode: Encodable {}

extension SourceChoice: Codable {
    private enum CodingKeys: String, CodingKey {
        case mode
        case path
    }

    private enum Mode: String, Codable {
        case undecided
        case watch
        case off
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(Mode.self, forKey: .mode) {
        case .undecided: self = .undecided
        case .off: self = .off
        case .watch: self = .watch(path: try container.decode(String.self, forKey: .path))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .undecided: try container.encode(Mode.undecided, forKey: .mode)
        case .off: try container.encode(Mode.off, forKey: .mode)
        case .watch(let path):
            try container.encode(Mode.watch, forKey: .mode)
            try container.encode(path, forKey: .path)
        }
    }
}
