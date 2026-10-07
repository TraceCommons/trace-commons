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
    /// Create passkey chosen on Join. The sheets that create it complete
    /// with the daemon, so they open once Folders or Tools started it
    /// (`FirstRunCall.openPasskeySheets`); until then the choice is undoable.
    case passkeyChosen
    /// A passkey whose account Verify bound, or joined this Mac to (another
    /// Mac had bound it), which enrolled this Mac (`account_bind`), with the
    /// name the person gave it (empty when an existing passkey signed in, or
    /// the bind answered `existing_account`).
    /// A sign-in alone never records it: it holds no enrolment.
    case passkey(name: String)
    /// The daemon was already enrolled when this first run began: an earlier
    /// first run joined and was quit before Start. An account it holds, so
    /// nothing is joined, signed in or created again.
    case enrolled
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
    /// The person's answer on the folder's own row: nil until answered,
    /// true for Watch, false for "I don't use it". A tool's folder starts
    /// unanswered (Ron's review of #1235, item 3). A folder of exported
    /// traces reads Watch, and "I don't use it" removes it (`Fine as
    /// built`), so it starts at true.
    public var watched: Bool?

    public init(kind: Kind, path: String, watched: Bool? = nil) {
        self.kind = kind
        self.path = path
        self.watched = watched ?? (kind == .trajectory ? true : nil)
    }
}

/// Every answer the person gives during the first run, and how far the
/// daemon calls have got. Pure: nothing here calls anything.
/// `FirstRunPlan` turns it into calls; the runner records their outcomes
/// back into `daemonStarted`, `startedSettingsJSON`, `enrolledInvite`,
/// `signedIn` and `nearAIEnrolled`. Those are facts the daemon holds, so navigation never
/// clears them.
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
    /// answer. Set through `answer(_:_:)`, which keeps one answer per kind.
    public var toolAnswers: [SourceKind: SourceChoice]
    /// Folders added on Tools, each its own row. Set through `add(_:)` and
    /// answered through `answerAdded(path:watched:)`.
    public var addedFolders: [AddedFolder]
    /// Rule per `project_id` (Custom only).
    public var rules: [String: ProjectMode]
    /// Chosen past sessions per `project_id` (Custom only).
    public var pastSelections: [String: Set<String>]
    public var scopes: Set<String>
    /// Writing it, to any value, clears `grantReady`: a new choice needs a
    /// new answer from the core.
    public var sharing: SharingPath {
        didSet { grantReady = false }
    }
    /// The Private AI switch (Custom only).
    public var privateAI: Bool
    /// The witness the disclosure showed, passed to `grant_automatic`.
    public var witnessSigningAddress: String?
    /// The core answered ready (`flow1::grant_request`) after both
    /// disclosures, for this Start. Only `SharingDisclosureFlow.resolve`
    /// sets it; writing `sharing` and Back clear it. `FirstRunPlan` sends
    /// `grantAutomatic` only with it set.
    public var grantReady: Bool
    public var daemonStarted: Bool
    /// The declaration the daemon now holds: the `startDaemon` settings, then
    /// each applied `setSourceSettings`. Nil with `daemonStarted` means it is
    /// unknown, and the whole declaration is sent again.
    public var startedSettingsJSON: String?
    /// The trimmed invite the daemon enrolled. Its last use may be spent, so
    /// it is neither looked up nor enrolled again.
    public var enrolledInvite: String?
    /// The near.ai sign-in completed; going forward again does not reopen it.
    public var signedIn: Bool
    /// The daemon enrolled this Mac through the near.ai login, with no
    /// invite (`near_ai_account_enroll`). A fact the daemon holds.
    public var nearAIEnrolled: Bool
    /// The tools discovery last reported not on this Mac
    /// (`recordDiscovery`). Such a tool is not asked (spec rule 1); see
    /// `sessionRoots` for how it is declared.
    public var notFound: Set<SourceKind>
    /// The person signed out on Join (#1030 rule 6) while the daemon held an
    /// enrolment: this run's invite, a passkey Verify bound, or an earlier
    /// first run's. The daemon has no call that drops an enrolment, so it
    /// may still hold one; this first run no longer treats it as an account
    /// (`holdsEnrolment` is false) and sends nothing that belongs to one --
    /// no scopes, no grant, no enrolment marker -- and an enrolment the
    /// daemon reports is not recorded again. A later enrolment (a new
    /// invite enrolled, a passkey bound) clears it.
    public var signedOutOfEnrolment: Bool

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
        grantReady: Bool = false,
        daemonStarted: Bool = false,
        startedSettingsJSON: String? = nil,
        enrolledInvite: String? = nil,
        signedIn: Bool = false,
        nearAIEnrolled: Bool = false,
        notFound: Set<SourceKind> = [],
        signedOutOfEnrolment: Bool = false
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
        self.grantReady = grantReady
        self.daemonStarted = daemonStarted
        self.startedSettingsJSON = startedSettingsJSON
        self.enrolledInvite = enrolledInvite
        self.signedIn = signedIn
        self.nearAIEnrolled = nearAIEnrolled
        self.notFound = notFound
        self.signedOutOfEnrolment = signedOutOfEnrolment
    }

    /// Record what discovery found. A tool not on this Mac is not asked,
    /// and Continue counts only the tools found here (spec rule 1, the
    /// owner's reversal in Ron's review of #1235).
    public mutating func recordDiscovery(_ candidates: [SourceCandidate]) {
        notFound = Set(candidates.filter { !$0.exists }.map(\.source))
    }

    /// Whether the daemon holds an enrolment for this first run, which is
    /// what consent scopes, the Automatic grant and the enrolment's marker
    /// need. An earlier first run's enrolment; a passkey Verify bound; or
    /// near.ai once its invite enrolled, or once it enrolled this Mac with no
    /// invite. An account answer alone -- near.ai chosen, a passkey chosen --
    /// is not one, and nor is an enrolment the person signed out of
    /// (`signedOutOfEnrolment`).
    public var holdsEnrolment: Bool {
        if signedOutOfEnrolment { return false }
        switch account {
        case .enrolled, .passkey: return true
        case .nearAI: return enrolledInvite != nil || nearAIEnrolled
        case .none, .watchOnly, .passkeyChosen: return false
        }
    }

    /// Answer a tool's row. A folder added for that tool is a row of its
    /// own and keeps its own answer.
    public mutating func answer(_ kind: SourceKind, _ choice: SourceChoice) {
        toolAnswers[kind] = choice
    }

    /// Add a folder as a row of its own. It answers nothing: no tool row is
    /// written, and a tool's folder starts unanswered. One folder is one
    /// thing: an earlier folder at the same path, of any kind, is dropped,
    /// so a folder answered again is never declared under two adapters. A
    /// second folder of exported traces replaces the first, which has one
    /// declaration of its own.
    public mutating func add(_ folder: AddedFolder) {
        addedFolders.removeAll { $0.path == folder.path || (folder.kind == .trajectory && $0.kind == .trajectory) }
        addedFolders.append(folder)
    }

    /// What a tool's own row answered, apart from any folder added for it.
    public func rowAnswer(_ kind: SourceKind) -> SourceChoice {
        toolAnswers[kind] ?? .undecided
    }

    /// Answer an added folder's row: true for Watch, false for "I don't use
    /// it", nil to take the answer back.
    public mutating func answerAdded(path: String, watched: Bool?) {
        guard let index = addedFolders.firstIndex(where: { $0.path == path }) else { return }
        addedFolders[index].watched = watched
    }

    /// Tools that two rows both watch: the tool's own row and a folder
    /// added for it, or two added folders. The daemon watches one folder per
    /// tool, so Continue is held until one of them says "I don't use it".
    public var watchedTwice: Set<SourceKind> {
        var counts: [SourceKind: Int] = [:]
        for (kind, choice) in toolAnswers {
            if case .watch = choice { counts[kind, default: 0] += 1 }
        }
        for folder in addedFolders where folder.watched == true {
            if case .source(let kind) = folder.kind { counts[kind, default: 0] += 1 }
        }
        return Set(counts.filter { $0.value > 1 }.keys)
    }

    /// Every added tool folder has an answer. A folder of exported traces
    /// always reads Watch.
    public var everyAddedFolderAnswered: Bool {
        addedFolders.allSatisfy { $0.watched != nil }
    }

    /// Withdraw the trajectory folder, which has no tool row to answer "I
    /// don't use it" on. Nothing is declared for it afterwards; once the
    /// daemon has started, `FirstRunPlan` sends the dropped key as `off`.
    public mutating func withdrawTrajectory() {
        addedFolders.removeAll { $0.kind == .trajectory }
    }

    /// The tool answers and added folders as one declaration. An added folder
    /// answered Watch is that tool's folder (while a tool is watched in two
    /// rows, `watchedTwice`, Continue is held, so this is never sent); one
    /// unanswered or answered "I don't use it" declares nothing, and the
    /// tool's own row stands.
    /// Both Continue on Folders/Tools and the daemon start read this, so they
    /// cannot disagree about what is answered.
    ///
    /// A tool not on this Mac is not asked, so it has no answer. Claude Code
    /// and Codex must be declared for the daemon to start, and an absent
    /// declaration of either reads its conventional folder, which would read
    /// the tool unasked once it is installed; a missing Claude Code or Codex
    /// the person did not answer is therefore declared `off`, watch nothing.
    /// A missing optional tool stays undeclared, which constructs no adapter.
    public var sessionRoots: SessionRoots {
        var roots = SessionRoots()
        for (kind, choice) in toolAnswers {
            roots[kind] = choice
        }
        for kind in [SourceKind.claudeCode, .codex] where notFound.contains(kind) && !roots[kind].isAnswered {
            roots[kind] = .off
        }
        for folder in addedFolders where folder.watched == true {
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
