import Foundation

/// The daemon's answer to `arming_suggestion`: the one project worth
/// offering to arm right now, and the evidence for offering it.
///
/// Optional at the call site rather than represented by a sentinel: the
/// daemon returns an empty object when there is nothing to suggest, because
/// a shell that receives no suggestion must draw no card and a null-filled
/// one would be a claim about a project the daemon never made.
public struct ArmingOffer: Decodable, Equatable, Sendable {
    public let projectId: String
    public let projectLabel: String
    public let contributedCount: Int

    public init(projectId: String, projectLabel: String, contributedCount: Int) {
        self.projectId = projectId
        self.projectLabel = projectLabel
        self.contributedCount = contributedCount
    }

    public enum CodingKeys: String, CodingKey {
        case projectId = "project_id"
        case projectLabel = "project_label"
        case contributedCount = "contributed_count"
    }
}

/// The words for the arming offer.
///
/// This is the offer, not the confirmation. It appears in the queue once a
/// project has been approved several times, and its whole job is to make the
/// case from evidence the contributor already has: they have read previews
/// from this project and kept approving them. Arming asks someone to stop
/// reading those previews, and the only honest basis for that question is
/// the history of them saying yes.
public enum ArmingOfferCopy {
    /// The evidence, stated before the question. A contributor who reads
    /// only the first line should still learn why they are being asked.
    public static func evidence(project: String, count: Int) -> String {
        let times = count == 1 ? "once" : "\(count) times"
        return "You've contributed from \(project) \(times)."
    }

    public static func question(project: String) -> String {
        "Contribute from \(project) automatically?"
    }

    /// Carries the action rather than agreeing in the abstract.
    public static let confirm = "Turn on automatic contributing"

    /// "Not now" rather than "No": declining is a decision about this
    /// moment, and the daemon treats it that way -- the offer is silenced
    /// for thirty days, not forever. Settings still arms the project at any
    /// point in between, without being asked.
    public static let decline = "Not now"
}
