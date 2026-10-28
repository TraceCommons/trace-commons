import Foundation

extension DaemonData {
    /// Current, account-wide provider limits. These do not belong to the
    /// historical week or the local Insights analytics store.
    public struct SubscriptionUsage: Decodable, Equatable, Sendable {
        public let schemaVersion: String
        public let providers: [SubscriptionUsageProvider]
        public let truncated: Bool?

        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case providers, truncated
        }

        public init(schemaVersion: String = "trace_commons.subscription_usage.v1",
                    providers: [SubscriptionUsageProvider], truncated: Bool? = nil) {
            self.schemaVersion = schemaVersion
            self.providers = providers
            self.truncated = truncated
        }
    }

    public struct SubscriptionUsageProvider: Decodable, Equatable, Sendable, Identifiable {
        public let provider: String
        public let enabled: Bool
        public let refreshing: Bool
        public let logins: [SubscriptionUsageLogin]
        public var id: String { provider }

        enum CodingKeys: String, CodingKey { case provider, enabled, refreshing, logins }

        public init(provider: String, enabled: Bool, refreshing: Bool = false,
                    logins: [SubscriptionUsageLogin] = []) {
            self.provider = provider
            self.enabled = enabled
            self.refreshing = refreshing
            self.logins = logins
        }
    }

    public struct SubscriptionUsageLogin: Decodable, Equatable, Sendable, Identifiable {
        public let id: String
        public let label: String
        public let defaultLogin: Bool
        public let state: String
        public let reason: String?
        public let updatedAt: Date?
        public let stale: Bool
        public let windows: [SubscriptionUsageWindow]
        public let bankedResets: Int?
        public let refreshing: Bool?
        /// Where the figures came from: "api", or "cli" for Claude figures read
        /// off the Claude Code CLI's /usage screen. Nil without a reading, or
        /// from an older daemon.
        public let source: String?

        enum CodingKeys: String, CodingKey {
            case id, label, state, reason, stale, windows, refreshing, source
            case defaultLogin = "default_login"
            case updatedAt = "updated_at"
            case bankedResets = "banked_resets"
        }

        public init(id: String, label: String, defaultLogin: Bool = false,
                    state: String, reason: String? = nil, updatedAt: Date? = nil,
                    stale: Bool = false, windows: [SubscriptionUsageWindow] = [],
                    bankedResets: Int? = nil, refreshing: Bool? = nil, source: String? = nil) {
            self.id = id
            self.label = label
            self.defaultLogin = defaultLogin
            self.state = state
            self.reason = reason
            self.updatedAt = updatedAt
            self.stale = stale
            self.windows = windows
            self.refreshing = refreshing
            self.bankedResets = bankedResets.flatMap { $0 >= 0 ? $0 : nil }
            self.source = source
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            id = try container.decode(String.self, forKey: .id)
            label = try container.decode(String.self, forKey: .label)
            defaultLogin = try container.decode(Bool.self, forKey: .defaultLogin)
            state = try container.decode(String.self, forKey: .state)
            reason = try container.decodeIfPresent(String.self, forKey: .reason)
            updatedAt = try? container.decodeIfPresent(Date.self, forKey: .updatedAt)
            stale = try container.decode(Bool.self, forKey: .stale)
            windows = try container.decode([SubscriptionUsageWindow].self, forKey: .windows)
            refreshing = try? container.decodeIfPresent(Bool.self, forKey: .refreshing)
            let count = try? container.decodeIfPresent(Int.self, forKey: .bankedResets)
            bankedResets = count.flatMap { $0 >= 0 ? $0 : nil }
            source = try? container.decodeIfPresent(String.self, forKey: .source)
        }
    }

    public struct SubscriptionUsageWindow: Decodable, Equatable, Sendable {
        public let lane: String
        public let label: String?
        public let usedPermille: Int?
        /// A finished Rust-formatted figure; no shell infers allowance from
        /// a missing percentage or a count of recent local traces.
        public let remainingText: String?
        public let resetsAt: Date?
        public let durationMins: Int?

        enum CodingKeys: String, CodingKey {
            case lane, label
            case usedPermille = "used_permille"
            case remainingText = "remaining_text"
            case resetsAt = "resets_at"
            case durationMins = "duration_mins"
        }

        public init(lane: String, label: String? = nil, usedPermille: Int? = nil,
                    remainingText: String? = nil, resetsAt: Date? = nil, durationMins: Int? = nil) {
            self.lane = lane
            self.label = label
            self.usedPermille = usedPermille.flatMap { (0...1000).contains($0) ? $0 : nil }
            self.remainingText = remainingText
            self.resetsAt = resetsAt
            self.durationMins = durationMins.flatMap { $0 > 0 ? $0 : nil }
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            lane = try container.decode(String.self, forKey: .lane)
            label = try container.decodeIfPresent(String.self, forKey: .label)
            let used = try? container.decodeIfPresent(Int.self, forKey: .usedPermille)
            usedPermille = used.flatMap { (0...1000).contains($0) ? $0 : nil }
            remainingText = try? container.decodeIfPresent(String.self, forKey: .remainingText)
            resetsAt = try? container.decodeIfPresent(Date.self, forKey: .resetsAt)
            let duration = try? container.decodeIfPresent(Int.self, forKey: .durationMins)
            durationMins = duration.flatMap { $0 > 0 ? $0 : nil }
        }
    }
}
