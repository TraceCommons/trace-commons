import Foundation

/// What the daemon does with sessions from a project.
///
/// The raw values are the daemon's wire spelling, which is snake_case
/// (`#[serde(rename_all = "snake_case")]` on the Rust enum). `ask` used to be
/// spelled `"ask"` here while the daemon sent `"notify_only"`, so every
/// ask-first row failed to decode -- and ask-first is the default for every
/// project, so in practice `list_projects` never decoded at all.
public enum ProjectMode: String, Decodable, Equatable, Sendable {
    /// Queue it and mention it in the next digest. The daemon calls this
    /// `notify_only`; the shell calls it asking first, because that is what
    /// it means to a contributor.
    case ask = "notify_only"
    case autoUpload = "auto_upload"
    case ignore
}

/// One row of `list_projects`.
///
/// ## Why `addedAt` is optional
///
/// The daemon reports two kinds of project: those it has a policy entry for
/// (`configured: true`, with an `added_at`) and those it has merely seen in
/// the queue (`configured: false`, `added_at: null`). Everything is the
/// second kind until the contributor decides something about it, which is
/// every project on first run. A non-optional `Date` here meant one null
/// failed the whole array, so the screen that asks which projects to watch
/// showed nothing to watch.
///
/// ## Why the id, and why it is the identity
///
/// `projectId` is an opaque digest the daemon mints from the project key. It
/// is what a row IS; `projectLabel` is only what it displays, and labels are
/// neither unique nor stable -- two projects can share a final path segment,
/// and the unresolvable bucket's label is reworded by every client. Writes go
/// back by id for the same reason: the key is a full local path the shell is
/// never given.
///
/// ## Why `isUnresolvedBucket` is read and not computed
///
/// The daemon marks that row itself. A shell could compare `projectId`
/// against a digest of the bucket's key instead, but only by re-implementing
/// the daemon's hash -- one copy of the rule per client, with nothing keeping
/// them in step. Matching on the label is worse still: every shell rewords
/// it, so the match would break silently the day the wording improved.
public struct ProjectRow: Decodable, Identifiable, Equatable, Sendable {
    public let projectId: String
    public let projectLabel: String
    public let mode: ProjectMode
    /// Nil for a project the daemon has seen but the contributor has not yet
    /// decided anything about.
    public let addedAt: Date?
    /// Whether the daemon holds a policy entry for this project.
    public let configured: Bool
    /// Whether this row is the bucket for sessions whose working directory
    /// had no usable final segment. Reported by the daemon, never derived.
    public let isUnresolvedBucket: Bool

    public var id: String { projectId }

    public init(
        projectId: String,
        projectLabel: String,
        mode: ProjectMode,
        addedAt: Date? = nil,
        configured: Bool = false,
        isUnresolvedBucket: Bool = false
    ) {
        self.projectId = projectId
        self.projectLabel = projectLabel
        self.mode = mode
        self.addedAt = addedAt
        self.configured = configured
        self.isUnresolvedBucket = isUnresolvedBucket
    }

    public enum CodingKeys: String, CodingKey {
        case projectId = "project_id"
        case projectLabel = "project_label"
        case mode
        case addedAt = "added_at"
        case configured
        case isUnresolvedBucket = "is_unresolved_bucket"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        projectId = try c.decode(String.self, forKey: .projectId)
        projectLabel = try c.decode(String.self, forKey: .projectLabel)
        mode = try c.decode(ProjectMode.self, forKey: .mode)
        addedAt = try c.decodeIfPresent(Date.self, forKey: .addedAt)
        // Older daemons predate the flag. Absent means "not configured",
        // which is what a row the daemon only saw in the queue is.
        configured = try c.decodeIfPresent(Bool.self, forKey: .configured) ?? false
        // Absent means "not the bucket". An older daemon that does not send
        // the flag leaves the row plain rather than explained, which is the
        // safe direction: claiming an ordinary project can never be
        // contributed automatically would be a lie about the contributor's
        // own repository.
        isUnresolvedBucket = try c.decodeIfPresent(Bool.self, forKey: .isUnresolvedBucket) ?? false
    }

    /// Whether this project could ever be armed to contribute without asking.
    ///
    /// False for the unresolvable bucket, and not as a UI preference: the
    /// daemon refuses `auto_upload` for that key in two independent places
    /// (`daemon/policy.rs`, in `set_mode` and in `resolve`). Any surface that
    /// offers a mode control must consult this rather than listing every
    /// `ProjectMode` case, because offering a mode the daemon will refuse
    /// invites a contributor to believe they have armed something that cannot
    /// be armed.
    ///
    /// Read by `offerableModes`, which is what every macOS surface with a
    /// mode control consults. It is stated here rather than in a view
    /// because it is a claim about what the daemon will accept, not a
    /// presentation choice, and because a view is the one place in this
    /// codebase nothing can execute.
    public var canBeArmed: Bool { !isUnresolvedBucket }

    /// The modes this row may be put in front of a contributor, in the order
    /// a picker should show them.
    ///
    /// Ask-first leads because it is the default and what happens to a
    /// contributor who reads nothing; arming sits in the middle; the one
    /// that stops a project being offered at all is last. Windows expresses
    /// the same list in the same order, for the same reasons, in
    /// `UnresolvedBucketCopy.OfferableModes`.
    ///
    /// A surface that wants a mode control must iterate this rather than
    /// `ProjectMode.allCases`. The bucket cannot be armed -- `Policy`
    /// refuses `auto_upload` for it in `set_mode` and again in `resolve` --
    /// and offering a mode the daemon will refuse invites a contributor to
    /// believe they have armed something that cannot be armed, with the
    /// refusal arriving silently or not at all. Omitting the choice is the
    /// honest answer; offering it disabled still puts an arming affordance
    /// on a row that has none.
    public var offerableModes: [ProjectMode] {
        canBeArmed ? [.ask, .autoUpload, .ignore] : [.ask, .ignore]
    }

    /// The name to show for this row.
    ///
    /// The bucket's own `projectLabel` is the slug `unknown-project`, which
    /// is a key rather than words. Every shell replaces it, with the same
    /// replacement, because it is one fact stated on several surfaces.
    public var displayLabel: String {
        isUnresolvedBucket ? ProjectCopy.unresolvedBucketLabel : projectLabel
    }
}

/// Words shared by every macOS surface that lists projects.
///
/// Onboarding screen 5 and Settings show the same row and must say the same
/// thing about it; the shared design spec states these once for all three
/// shells, under "The unresolvable bucket in Settings". Kept here rather than
/// beside either view so that neither becomes the owner and the other the
/// copy -- a near-duplicate is how two surfaces start disagreeing.
public enum ProjectCopy {
    public static let unresolvedBucketLabel = "Sessions with no project"

    /// The label for one option in a mode picker.
    ///
    /// These are *actions* -- what selecting this does -- and are not the
    /// state sentences Settings prints beside a row to say what the mode
    /// currently is ("Contributed without asking"). Both exist and they are
    /// not interchangeable: a picker of state sentences reads as a report,
    /// and a row labelled with an action reads as a button that has not been
    /// pressed. The words are the Linux shell's `mode_choices`.
    public static func modeChoiceLabel(_ mode: ProjectMode) -> String {
        switch mode {
        case .ask: return "Ask me first"
        case .autoUpload: return "Contribute automatically"
        case .ignore: return "Never offer this one"
        }
    }


    /// A statement of what the daemon does, not an apology. Nothing in it is
    /// a contributor's to fix: the bucket exists so that a directory the
    /// daemon cannot name never has its path written into the audit log,
    /// notification text or history, and not being armable is the protective
    /// half of that.
    public static let unresolvedBucketNote = """
        Trace Commons can't tell which folder these ran in, so they can never \
        be contributed automatically. You'll always be asked.
        """
}
