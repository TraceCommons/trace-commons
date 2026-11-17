import Foundation

/// Immutable identity only. Rust owns all profile/default resolution.
public struct BuildFeatureProfile: Decodable, Equatable, Sendable {
    public let schemaVersion: Int
    public let environment: String
    public let profileDigest: String
    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version", environment, profileDigest = "profile_digest"
    }
    public func agrees(environment: String?, digest: String?) -> Bool {
        valid && self.environment == environment && profileDigest == digest
    }
    public var valid: Bool {
        schemaVersion == 1 && ["production", "staging", "testing"].contains(environment)
            && profileDigest.count == 64 && profileDigest.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
}

/// A daemon snapshot, never a second feature evaluator.
public struct FeatureFlagSnapshot: Decodable, Equatable, Sendable {
    public struct Decision: Decodable, Equatable, Sendable {
        public enum Timing: String, Decodable, Sendable { case live, nextStart = "next_start" }
        public let enabled, pending, capabilityBlocked: Bool
        public let timing: Timing
        enum CodingKeys: String, CodingKey { case enabled, pending, capabilityBlocked = "capability_blocked", timing }
    }
    public enum Source: String, Decodable, Sendable { case `default`, remote, cache }
    public let environment, profileDigest, instanceID, sampledAt: String
    public let generation: UInt64
    public let source: Source
    public let revision: UInt64?
    public let expiresAt: String?
    public let fresh: Bool
    public let flags: [String: Decision]
    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version", environment, profileDigest = "profile_digest"
        case instanceID = "instance_id", generation, sampledAt = "sampled_at", source, revision
        case expiresAt = "expires_at", fresh, flags
    }
    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let version = try c.decode(Int.self, forKey: .schemaVersion)
        environment = try c.decode(String.self, forKey: .environment)
        profileDigest = try c.decode(String.self, forKey: .profileDigest)
        instanceID = try c.decode(String.self, forKey: .instanceID)
        generation = try c.decode(UInt64.self, forKey: .generation)
        sampledAt = try c.decode(String.self, forKey: .sampledAt)
        source = try c.decode(Source.self, forKey: .source)
        revision = try c.decodeIfPresent(UInt64.self, forKey: .revision)
        expiresAt = try c.decodeIfPresent(String.self, forKey: .expiresAt)
        fresh = try c.decode(Bool.self, forKey: .fresh)
        flags = try c.decode([String: Decision].self, forKey: .flags)
        guard version == 1, generation > 0, UUID(uuidString: instanceID) != nil,
              ["production", "staging", "testing"].contains(environment),
              profileDigest.count == 64, profileDigest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) })
        else { throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "feature-snapshot-invalid")) }
    }
    public func isEnabled(_ id: String) -> Bool { flags[id]?.enabled ?? false }
    public func acceptsSuccessor(_ newer: Self) -> Bool {
        environment == newer.environment && profileDigest == newer.profileDigest
            && (instanceID != newer.instanceID || newer.generation > generation)
    }
}
