import Foundation

// `subscription_usage` (PR #1608 design, sections 3.3 and 3.4): the plan and
// limit windows the Codex app on this Mac reported for its ChatGPT sign-in,
// answered from the daemon's held snapshot. Behind the contributor crate's
// `subscription-usage` feature and a settings switch that is off by
// default; a daemon built without it refuses the method as
// `unknown_method`, which draws no card and no switch.
extension DaemonData {
    /// The whole answer. With the switch off the daemon sends only
    /// `{"enabled": false}` and reads nothing, so `logins` is absent.
    public struct SubscriptionUsage: Codable, Equatable, Sendable {
        public let enabled: Bool
        public let logins: [SubscriptionLogin]?

        public init(enabled: Bool, logins: [SubscriptionLogin]?) {
            self.enabled = enabled
            self.logins = logins
        }
    }

    /// One subscription login. `state` is `observed` or `unknown`; `reason`
    /// is a fixed label (`off`, `never_refreshed`, `codex_not_found`,
    /// `codex_running`, `signed_out`, `not_subscription`, `timeout`,
    /// `unreadable`, `blocked`, `failed`) or `nil`. A failed refresh keeps
    /// the previous reading as `observed` with `stale: true` and the
    /// failure's reason. Labels stay strings, so one a later daemon adds
    /// decodes and reads as "Usage unknown".
    public struct SubscriptionLogin: Codable, Equatable, Sendable {
        public let provider: String
        public let state: String
        public let reason: String?
        /// A probe is running; the answer to come arrives with
        /// `status_changed`, or on the next read. Absent reads as not.
        public let refreshing: Bool?
        /// `cli` (the Codex app-server) or `nil` before any reading.
        public let source: String?
        /// RFC 3339; `nil` before any reading.
        public let updatedAt: String?
        /// `nil` is treated as stale: fail closed.
        public let stale: Bool?
        /// A fixed label (`free`, `plus`, `pro`, `team`, `business`,
        /// `enterprise`, `edu`, `unknown`), never free text; `nil` before
        /// any reading.
        public let plan: String?
        public let limitReached: Bool?
        /// Only the windows the provider reported; one it did not report is
        /// absent, never a zero.
        public let windows: [SubscriptionWindow]?
        public let credits: SubscriptionCredits?
        public let resetCredits: Int?

        public init(provider: String, state: String, reason: String?, refreshing: Bool?, source: String?,
                    updatedAt: String?, stale: Bool?, plan: String?, limitReached: Bool?,
                    windows: [SubscriptionWindow]?, credits: SubscriptionCredits?, resetCredits: Int?) {
            self.provider = provider
            self.state = state
            self.reason = reason
            self.refreshing = refreshing
            self.source = source
            self.updatedAt = updatedAt
            self.stale = stale
            self.plan = plan
            self.limitReached = limitReached
            self.windows = windows
            self.credits = credits
            self.resetCredits = resetCredits
        }

        public enum CodingKeys: String, CodingKey {
            case provider, state, reason, refreshing, source, stale, plan, windows, credits
            case updatedAt = "updated_at"
            case limitReached = "limit_reached"
            case resetCredits = "reset_credits"
        }
    }

    /// One limit window. `lane` is `session`, `weekly` or `model`; `label`
    /// is the daemon's checked model-limit name, or `nil`.
    public struct SubscriptionWindow: Codable, Equatable, Sendable {
        public let lane: String
        public let label: String?
        public let durationMins: Int?
        /// 0..=1000. A value outside that is unknown, never clamped into a
        /// figure.
        public let usedPermille: Int
        /// RFC 3339, or `nil` when the provider did not say.
        public let resetsAt: String?

        public init(lane: String, label: String?, durationMins: Int?, usedPermille: Int, resetsAt: String?) {
            self.lane = lane
            self.label = label
            self.durationMins = durationMins
            self.usedPermille = usedPermille
            self.resetsAt = resetsAt
        }

        public enum CodingKeys: String, CodingKey {
            case lane, label
            case durationMins = "duration_mins"
            case usedPermille = "used_permille"
            case resetsAt = "resets_at"
        }
    }

    /// The plan's credits. `balance` is the provider's decimal as a string,
    /// shown as sent.
    public struct SubscriptionCredits: Codable, Equatable, Sendable {
        public let hasCredits: Bool
        public let unlimited: Bool
        public let balance: String?

        public init(hasCredits: Bool, unlimited: Bool, balance: String?) {
            self.hasCredits = hasCredits
            self.unlimited = unlimited
            self.balance = balance
        }

        public enum CodingKeys: String, CodingKey {
            case unlimited, balance
            case hasCredits = "has_credits"
        }
    }
}
