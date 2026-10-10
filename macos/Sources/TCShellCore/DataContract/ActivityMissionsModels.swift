import Foundation

// The separate trace_activity contract from protocol activity_missions.rs.
// Counts are source facts; Swift never evaluates assignments or credit.
extension DaemonData {
    public struct ActivityCatalogue: Codable, Equatable, Sendable {
        public let schemaVersion: UInt32
        public let kind: String
        public let state: String
        public let policySHA256: String?
        public let policy: ActivityPolicy?
        public let rewardsEnabled: Bool
        public let creditPointsPending: UInt64?
        public let creditCondition: String

        public enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case kind
            case state
            case policySHA256 = "policy_sha256"
            case policy
            case rewardsEnabled = "rewards_enabled"
            case creditPointsPending = "credit_points_pending"
            case creditCondition = "credit_condition"
        }
    }

    /// Calendar fields are UTC dates (YYYY-MM-DD), not local timestamps.
    public struct ActivityPolicy: Codable, Equatable, Sendable {
        public let schemaVersion: UInt32
        public let policyId: String
        public let startsOn: String
        public let endsBefore: String
        public let qualification: String
        public let missions: [ActivityMission]
        public let daily: ActivityDailyRule?
        public let levels: [ActivityThreshold]?
        public let badges: [ActivityBadgeRule]?

        public enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case policyId = "policy_id"
            case startsOn = "starts_on"
            case endsBefore = "ends_before"
            case qualification
            case missions
            case daily
            case levels
            case badges
        }
    }

    public struct ActivityMission: Codable, Equatable, Sendable {
        public let id: String
        public let title: String
        public let requiredContributions: UInt32

        public enum CodingKeys: String, CodingKey {
            case id
            case title
            case requiredContributions = "required_contributions"
        }
    }

    public struct ActivityDailyRule: Codable, Equatable, Sendable {
        public let kind: String
        public let missionId: String?
        public let missionIds: [String]?

        public enum CodingKeys: String, CodingKey {
            case kind
            case missionId = "mission_id"
            case missionIds = "mission_ids"
        }
    }

    public struct ActivityThreshold: Codable, Equatable, Sendable {
        public let id: String
        public let required: UInt32

        public enum CodingKeys: String, CodingKey {
            case id
            case required
        }
    }

    public struct ActivityBadgeRule: Codable, Equatable, Sendable {
        public let id: String
        public let metric: String
        public let required: UInt32

        public enum CodingKeys: String, CodingKey {
            case id
            case metric
            case required
        }
    }

    public struct ActivityDailyProgress: Codable, Equatable, Sendable {
        public let day: String
        public let missionId: String
        public let contributions: UInt64
        public let requiredContributions: UInt32
        public let complete: Bool

        public enum CodingKeys: String, CodingKey {
            case day
            case missionId = "mission_id"
            case contributions
            case requiredContributions = "required_contributions"
            case complete
        }
    }

    public struct ActivityBadgeProgress: Codable, Equatable, Sendable {
        public let id: String
        public let achieved: Bool?

        public enum CodingKeys: String, CodingKey {
            case id
            case achieved
        }
    }

    public struct ActivityProgress: Codable, Equatable, Sendable {
        public let schemaVersion: UInt32
        public let kind: String
        public let policySHA256: String
        public let observedAt: Date
        public let source: String
        public let qualification: String
        public let coverageStartsOn: String
        public let monthStartsOn: String
        public let monthlyContributions: UInt64
        public let daily: ActivityDailyProgress?
        public let completedDays: UInt32?
        public let currentStreak: UInt32?
        public let level: String?
        public let levelsConfigured: Bool
        public let badges: [ActivityBadgeProgress]?
        public let rewardsEnabled: Bool
        public let creditPointsPending: UInt64?
        public let creditCondition: String

        public enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case kind
            case policySHA256 = "policy_sha256"
            case observedAt = "observed_at"
            case source
            case qualification
            case coverageStartsOn = "coverage_starts_on"
            case monthStartsOn = "month_starts_on"
            case monthlyContributions = "monthly_contributions"
            case daily
            case completedDays = "completed_days"
            case currentStreak = "current_streak"
            case level
            case levelsConfigured = "levels_configured"
            case badges
            case rewardsEnabled = "rewards_enabled"
            case creditPointsPending = "credit_points_pending"
            case creditCondition = "credit_condition"
        }
    }

    /// Rust-owned disclosure plus the unchanged protocol payload.
    public struct ActivityMissionsCatalogue: Codable, Equatable, Sendable {
        public let catalogue: ActivityCatalogue
        public let disclosure: String

        public enum CodingKeys: String, CodingKey { case catalogue, disclosure }

        public init(from decoder: Decoder) throws {
            let raw = try ActivityWireValidation.wrapper(decoder, payload: "catalogue")
            try ActivityWireValidation.catalogue(raw)
            let values = try decoder.container(keyedBy: CodingKeys.self)
            catalogue = try values.decode(ActivityCatalogue.self, forKey: .catalogue)
            disclosure = try values.decode(String.self, forKey: .disclosure)
            try ActivityWireValidation.validate(catalogue)
        }
    }

    /// Rust-owned disclosure plus the unchanged protocol payload.
    public struct ActivityMissionsStatus: Codable, Equatable, Sendable {
        public let status: ActivityProgress
        public let disclosure: String

        public enum CodingKeys: String, CodingKey { case status, disclosure }

        public init(from decoder: Decoder) throws {
            let raw = try ActivityWireValidation.wrapper(decoder, payload: "status")
            try ActivityWireValidation.status(raw)
            let values = try decoder.container(keyedBy: CodingKeys.self)
            status = try values.decode(ActivityProgress.self, forKey: .status)
            disclosure = try values.decode(String.self, forKey: .disclosure)
            try ActivityWireValidation.validate(status)
        }
    }

}

/// Activity reply wrappers enforce the strict Rust field sets. Reject unsafe economic,
/// profile and unversioned extensions instead of displaying an unvalidated claim.
/// A mission's `predicate` is the one versioned extension point, read as the
/// protocol reads it: a version-1 block against its own key set, a later
/// version carried unread. The shell never matches on it.
///
/// Deliberate, and it applies to version skew too: these key sets mirror
/// `#[serde(deny_unknown_fields)]` on every activity type in
/// `crates/trace-commons-protocol/src/activity_missions.rs`, so the daemon
/// is exactly as strict toward the server. An additive field on these
/// replies must bump `schema_version`. Until this app knows the new field,
/// a newer attached daemon's reply throws `.undecodable` -- never decodes
/// into a partial reward or progress claim this build could not check,
/// which for economic fields is the point.
private enum ActivityWireValidation {
    static func failure() -> DecodingError {
        .dataCorrupted(.init(codingPath: [], debugDescription: "activity-missions-invalid"))
    }

    static func object(_ raw: ActivityWireNode, keys: Set<String>) throws -> [String: ActivityWireNode] {
        guard case .object(let value) = raw, Set(value.keys).isSubset(of: keys) else { throw failure() }
        return value
    }

    static func wrapper(_ decoder: Decoder, payload: String) throws -> ActivityWireNode {
        var keys: Set<String> = [payload, "disclosure"]
        #if DEBUG
        keys.insert("_sample")
        #endif
        let value = try object(ActivityWireNode(from: decoder), keys: keys)
        guard let raw = value[payload] else { throw failure() }
        return raw
    }

    static func optional(_ raw: ActivityWireNode?, check: (ActivityWireNode) throws -> Void) throws {
        guard let raw else { return }
        if case .null = raw { return }
        try check(raw)
    }

    static func array(_ raw: ActivityWireNode, check: (ActivityWireNode) throws -> Void) throws {
        guard case .array(let values) = raw else { throw failure() }
        for value in values { try check(value) }
    }

    static func catalogue(_ raw: ActivityWireNode) throws {
        let value = try object(raw, keys: ["schema_version", "kind", "state", "policy_sha256", "policy", "rewards_enabled", "credit_points_pending", "credit_condition"])
        try optional(value["policy"], check: policy)
    }

    static func policy(_ raw: ActivityWireNode) throws {
        let value = try object(raw, keys: ["schema_version", "policy_id", "starts_on", "ends_before", "qualification", "missions", "daily", "levels", "badges"])
        guard let missions = value["missions"] else { throw failure() }
        try array(missions) { raw in
            let mission = try object(raw, keys: ["id", "title", "required_contributions", "predicate"])
            try optional(mission["predicate"], check: predicate)
        }
        try optional(value["daily"]) { raw in
            guard case .object(let fields) = raw, case .string(let kind)? = fields["kind"] else { throw failure() }
            switch kind {
            case "fixed": _ = try object(raw, keys: ["kind", "mission_id"])
            case "rotation": _ = try object(raw, keys: ["kind", "mission_ids"])
            default: throw failure()
            }
        }
        try optional(value["levels"]) { try array($0) { _ = try object($0, keys: ["id", "required"]) } }
        try optional(value["badges"]) { try array($0) { _ = try object($0, keys: ["id", "metric", "required"]) } }
    }

    /// Mirrors `MissionPredicate` in the protocol: a positive integer
    /// `version`; version 1 has a closed key set of string lists and a
    /// required `min_sessions`; a later version is any object. Its bounds are
    /// the daemon's to enforce, before the reply reaches the shell.
    static func predicate(_ raw: ActivityWireNode) throws {
        guard case .object(let fields) = raw, case .number(let version)? = fields["version"],
              version >= 1, version == version.rounded(.towardZero) else { throw failure() }
        guard version == 1 else { return }
        let value = try object(raw, keys: ["version", "tools", "tool_families", "languages", "min_sessions"])
        guard case .number? = value["min_sessions"] else { throw failure() }
        for key in ["tools", "tool_families", "languages"] {
            try optional(value[key]) { try array($0) { guard case .string = $0 else { throw failure() } } }
        }
    }

    static func status(_ raw: ActivityWireNode) throws {
        let value = try object(raw, keys: ["schema_version", "kind", "policy_sha256", "observed_at", "source", "qualification", "coverage_starts_on", "month_starts_on", "monthly_contributions", "daily", "completed_days", "current_streak", "level", "levels_configured", "badges", "rewards_enabled", "credit_points_pending", "credit_condition"])
        try optional(value["daily"]) { _ = try object($0, keys: ["day", "mission_id", "contributions", "required_contributions", "complete"]) }
        try optional(value["badges"]) { try array($0) { _ = try object($0, keys: ["id", "achieved"]) } }
    }

    static func digest(_ raw: String) -> Bool {
        raw.utf8.count == 64 && raw.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }

    static func qualification(_ raw: String) -> Bool {
        raw == "accepted" || raw == "received_or_accepted"
    }

    static func day(_ raw: String) -> Bool {
        raw.range(of: #"^\d{4}-\d{2}-\d{2}$"#, options: .regularExpression) != nil
            && DaemonDataDecoding.parseDate(raw + "T00:00:00Z") != nil
    }

    static func validate(_ value: DaemonData.ActivityCatalogue) throws {
        guard value.schemaVersion == 1, value.kind == "trace_activity", !value.rewardsEnabled,
              value.creditPointsPending == nil, value.creditCondition == "mission_credit_ledger_unavailable"
        else { throw failure() }
        switch value.state {
        case "unconfigured":
            guard value.policy == nil, value.policySHA256 == nil else { throw failure() }
        case "configured":
            guard let policy = value.policy, let hash = value.policySHA256, digest(hash) else { throw failure() }
            guard policy.schemaVersion == 1, qualification(policy.qualification), day(policy.startsOn), day(policy.endsBefore) else { throw failure() }
            if let rule = policy.daily {
                guard (rule.kind == "fixed" && rule.missionId != nil && rule.missionIds == nil)
                    || (rule.kind == "rotation" && rule.missionIds != nil && rule.missionId == nil) else { throw failure() }
            }
            if let badges = policy.badges {
                guard badges.allSatisfy({ ["monthly_contributions", "completed_days", "current_streak"].contains($0.metric) }) else { throw failure() }
            }
        default: throw failure()
        }
    }

    static func validate(_ value: DaemonData.ActivityProgress) throws {
        guard value.schemaVersion == 1, value.kind == "trace_activity", digest(value.policySHA256),
              value.source == "account_contributed_submissions", qualification(value.qualification),
              day(value.coverageStartsOn), day(value.monthStartsOn), !value.rewardsEnabled,
              value.creditPointsPending == nil, value.creditCondition == "mission_credit_ledger_unavailable"
        else { throw failure() }
        if let daily = value.daily { guard day(daily.day) else { throw failure() } }
    }
}

/// Structural inspection keeps the original Decoder for exact UInt64 decoding;
/// numeric values are never projected through floating-point conversion.
private indirect enum ActivityWireNode: Decodable {
    case object([String: ActivityWireNode]), array([ActivityWireNode]), string(String), bool(Bool), number(Double), null

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let raw = try? value.decode(Bool.self) { self = .bool(raw) }
        else if let raw = try? value.decode(String.self) { self = .string(raw) }
        else if let raw = try? value.decode([String: ActivityWireNode].self) { self = .object(raw) }
        else if let raw = try? value.decode([ActivityWireNode].self) { self = .array(raw) }
        else if let raw = try? value.decode(Double.self) { self = .number(raw) }
        else { throw ActivityWireValidation.failure() }
    }
}
