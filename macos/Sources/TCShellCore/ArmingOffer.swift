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

// The offer's words are `ProjectArmingCopy`, decoded from the core's
// `tc_arming_offer_copy_json` (K3, #1173).
