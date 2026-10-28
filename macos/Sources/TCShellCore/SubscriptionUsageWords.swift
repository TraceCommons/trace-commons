import Foundation

/// The Insights "Subscriptions" card (PR #1608 design, 5.1), read from a
/// `subscription_usage` answer. Every word is the core's
/// (`subscription_usage_*` in the Insights copy table, read through
/// `tc_insights_copy_json`); a `{name}` hole takes a figure from the answer
/// and nothing else. A window the provider did not report is not drawn, and
/// an unknown one is "Usage unknown", never an empty bar.
public enum SubscriptionUsageWords {
    /// The card's copy keys, so a test can check the core sends each one.
    public enum Key {
        public static let cardTitle = "subscription_usage_card_title"
        public static let window5h = "subscription_usage_window_5h"
        public static let windowWeekly = "subscription_usage_window_weekly"
        public static let windowHours = "subscription_usage_window_hours"
        public static let windowDays = "subscription_usage_window_days"
        public static let windowUnnamed = "subscription_usage_window_unnamed"
        public static let used = "subscription_usage_used"
        public static let resets = "subscription_usage_resets"
        public static let updated = "subscription_usage_updated"
        public static let lastUpdated = "subscription_usage_last_updated"
        public static let unknown = "subscription_usage_unknown"
        public static let planUnknown = "subscription_usage_plan_unknown"
        public static let limitReached = "subscription_usage_limit_reached"
        public static let creditsAvailable = "subscription_usage_credits_available"
        public static let creditsUnlimited = "subscription_usage_credits_unlimited"
        public static let creditsBalance = "subscription_usage_credits_balance"
        public static let resetCredits = "subscription_usage_reset_credits"
        public static let resetCreditOne = "subscription_usage_reset_credit_one"
        public static let codexNotFound = "subscription_usage_codex_not_found"
        public static let codexRunning = "subscription_usage_codex_running"
        public static let signedOutCodex = "subscription_usage_signed_out_codex"
        public static let notSubscription = "subscription_usage_not_subscription"
        public static let needsAttention = "subscription_usage_needs_attention"
        public static let refresh = "subscription_usage_refresh"
        public static let refreshA11y = "subscription_usage_refresh_a11y"
        public static let sectionTitle = "subscription_usage_section_title"
        public static let codexSwitch = "subscription_usage_codex_switch"
        public static let codexDetail = "subscription_usage_codex_detail"
        /// `subscription_usage_plan_<label>`: "ChatGPT Plus" and the rest.
        public static func plan(_ label: String) -> String { "subscription_usage_plan_" + label }
        /// `subscription_usage_provider_<provider>`: the card's title when
        /// the plan is not known.
        public static func provider(_ provider: String) -> String { "subscription_usage_provider_" + provider }

        /// Every fixed key the card and the switch read.
        public static let all: [String] = [
            cardTitle, window5h, windowWeekly, windowHours, windowDays, windowUnnamed, used, resets,
            updated, lastUpdated, unknown, planUnknown, limitReached, creditsAvailable, creditsUnlimited,
            creditsBalance, resetCredits, resetCreditOne, codexNotFound, codexRunning, signedOutCodex,
            notSubscription, needsAttention, refresh, refreshA11y, sectionTitle, codexSwitch, codexDetail,
            provider("codex"),
        ] + plans.map(plan)
    }

    /// The plan labels the daemon may send besides `unknown`.
    public static let plans = ["free", "plus", "pro", "team", "business", "enterprise", "edu"]

    /// One meter: the window's name, "{n}% used" and "Resets {relative}".
    public struct Meter: Equatable, Sendable, Identifiable {
        public let id: Int
        public let name: String
        /// "{n}% used", or "Usage unknown" when `fraction` is `nil`.
        public let used: String
        /// 0...1 for the bar; `nil` draws no bar at all.
        public let fraction: Double?
        public let resets: String?
        /// A model-specific window, drawn smaller below the others.
        public let model: Bool
    }

    /// One login's row, ready to draw.
    public struct Row: Equatable, Sendable, Identifiable {
        public let id: Int
        /// "ChatGPT Plus", or the tool's name when the plan is not known.
        public let title: String
        /// "Plan unknown" under the tool's name when the daemon said
        /// `unknown`.
        public let planLine: String?
        public let meters: [Meter]
        /// Credits and reset credits, one line each.
        public let credits: [String]
        /// "Limit reached", or why the usage is not known.
        public let status: [String]
        /// "Updated {age} ago" or, while stale, "Last updated {age} ago".
        public let updated: String?
        public let refreshing: Bool
    }

    public static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    public static func fill(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { result, hole in
            result.replacingOccurrences(of: "{" + hole.key + "}", with: hole.value)
        }
    }

    /// The rows to draw: none for `{"enabled": false}`.
    public static func rows(_ usage: DaemonData.SubscriptionUsage, copy: [String: String],
                            now: Date = Date()) -> [Row] {
        guard usage.enabled else { return [] }
        return (usage.logins ?? []).enumerated().map { index, login in
            row(login, id: index, copy: copy, now: now)
        }
    }

    public static func row(_ login: DaemonData.SubscriptionLogin, id: Int, copy: [String: String],
                           now: Date = Date()) -> Row {
        let observed = login.state == "observed"
        let stale = login.stale ?? true
        let plan = login.plan.flatMap { plans.contains($0) ? text(Key.plan($0), copy) : nil }
            .flatMap { $0.isEmpty ? nil : $0 }
        let windows = observed ? (login.windows ?? []) : []
        let ordered = windows.filter { $0.lane != "model" } + windows.filter { $0.lane == "model" }
        let meters = ordered.enumerated().map { index, window in
            meter(window, id: index, stale: stale, copy: copy, now: now)
        }
        return Row(
            id: id,
            title: plan ?? providerName(login.provider, copy: copy),
            planLine: plan == nil && login.plan != nil ? text(Key.planUnknown, copy) : nil,
            meters: meters,
            credits: observed ? creditLines(login, copy: copy) : [],
            status: status(login, observed: observed, copy: copy),
            updated: observed ? updated(login.updatedAt, stale: stale, copy: copy, now: now) : nil,
            refreshing: login.refreshing ?? false)
    }

    /// The tool's name for the title, from its own key; "Plan unknown" when
    /// the core has no name for it, never a bare provider key.
    static func providerName(_ provider: String, copy: [String: String]) -> String {
        let name = text(Key.provider(provider), copy)
        return name.isEmpty ? text(Key.planUnknown, copy) : name
    }

    /// A window's meter. A window that has reset since the reading is
    /// "Usage unknown" (design 2.6), stale or not: the figure it showed no
    /// longer holds, and nothing polls for a new one.
    public static func meter(_ window: DaemonData.SubscriptionWindow, id: Int, stale: Bool,
                             copy: [String: String], now: Date = Date()) -> Meter {
        let reset = window.resetsAt.flatMap(DaemonTimestamp.parse)
        let lapsed = reset.map { $0 <= now } == true
        let known = (0...1000).contains(window.usedPermille) && !lapsed
        return Meter(
            id: id,
            name: windowName(window, copy: copy),
            used: known ? fill(text(Key.used, copy), ["n": percent(window.usedPermille)]) : text(Key.unknown, copy),
            fraction: known ? Double(window.usedPermille) / 1_000 : nil,
            resets: known ? reset.flatMap { resets($0, copy: copy, now: now) } : nil,
            model: window.lane == "model")
    }

    /// "5-hour", "Weekly", "{n}-hour", "{n}-day", the model limit's
    /// checked name, or "Usage window" when none of those can be said.
    public static func windowName(_ window: DaemonData.SubscriptionWindow, copy: [String: String]) -> String {
        if window.lane == "model" {
            return window.label.flatMap { $0.isEmpty ? nil : $0 } ?? text(Key.windowUnnamed, copy)
        }
        switch window.durationMins {
        case 300: return text(Key.window5h, copy)
        case 10_080: return text(Key.windowWeekly, copy)
        case let minutes? where minutes > 0 && minutes % 1_440 == 0:
            return fill(text(Key.windowDays, copy), ["n": String(minutes / 1_440)])
        case let minutes? where minutes > 0 && minutes % 60 == 0:
            return fill(text(Key.windowHours, copy), ["n": String(minutes / 60)])
        case nil where window.lane == "weekly": return text(Key.windowWeekly, copy)
        default: return text(Key.windowUnnamed, copy)
        }
    }

    /// Whole percent from per mille, rounded half up.
    public static func percent(_ permille: Int) -> String { String((permille + 5) / 10) }

    /// "Resets in 3 hours"; nothing for a time already past.
    static func resets(_ date: Date, copy: [String: String], now: Date) -> String? {
        guard date > now else { return nil }
        return fill(text(Key.resets, copy), ["relative": Format.when(date, relativeTo: now)])
    }

    static func creditLines(_ login: DaemonData.SubscriptionLogin, copy: [String: String]) -> [String] {
        var lines: [String] = []
        if let credits = login.credits {
            if credits.unlimited {
                lines.append(text(Key.creditsUnlimited, copy))
            } else if let balance = credits.balance, !balance.isEmpty {
                lines.append(fill(text(Key.creditsBalance, copy), ["balance": balance]))
            } else if credits.hasCredits {
                lines.append(text(Key.creditsAvailable, copy))
            }
        }
        if let count = login.resetCredits, count > 0 {
            lines.append(count == 1 ? text(Key.resetCreditOne, copy)
                                    : fill(text(Key.resetCredits, copy), ["n": String(count)]))
        }
        return lines
    }

    /// The row's status line. An observed reading says "Limit reached" when
    /// the provider did; a stale one also says why it could not be renewed
    /// when there is something the contributor can do about it. With no
    /// reading, the reason, or "Usage unknown" for any reason without words
    /// of its own (`timeout`, `unreadable`, `failed`, a label a later daemon
    /// adds). `off` and `never_refreshed` show no line: the Refresh button
    /// is all there is to say.
    static func status(_ login: DaemonData.SubscriptionLogin, observed: Bool,
                       copy: [String: String]) -> [String] {
        var lines: [String] = []
        if observed, login.limitReached == true { lines.append(text(Key.limitReached, copy)) }
        let specific = login.reason.flatMap { reasonKey($0, provider: login.provider) }.map { text($0, copy) }
        if observed {
            if (login.stale ?? true), let specific { lines.append(specific) }
        } else if login.reason != "off", login.reason != "never_refreshed" {
            lines.append(specific ?? text(Key.unknown, copy))
        }
        return lines
    }

    /// The copy key for a reason with words of its own.
    static func reasonKey(_ reason: String, provider: String) -> String? {
        switch reason {
        case "codex_not_found": return Key.codexNotFound
        case "codex_running": return Key.codexRunning
        case "signed_out": return provider == "codex" ? Key.signedOutCodex : nil
        case "not_subscription": return Key.notSubscription
        case "blocked": return Key.needsAttention
        default: return nil
        }
    }

    static func updated(_ updatedAt: String?, stale: Bool, copy: [String: String], now: Date) -> String? {
        guard let at = updatedAt.flatMap(DaemonTimestamp.parse) else { return nil }
        return fill(text(stale ? Key.lastUpdated : Key.updated, copy), ["age": age(now.timeIntervalSince(at))])
    }

    /// "5 minutes", "3 hours", "2 days": the largest whole unit, at least
    /// one minute, so a reading just taken is "1 minute" and never "0".
    public static func age(_ seconds: TimeInterval) -> String {
        ages.string(max(seconds, 60))
    }

    private static let ages = AgeFormatter()

    /// `DateComponentsFormatter` is not documented as thread-safe; built
    /// once and locked, as `Format`'s are.
    private final class AgeFormatter: @unchecked Sendable {
        private let lock = NSLock()
        private let formatter: DateComponentsFormatter

        init() {
            let formatter = DateComponentsFormatter()
            formatter.unitsStyle = .full
            formatter.allowedUnits = [.day, .hour, .minute]
            formatter.maximumUnitCount = 1
            self.formatter = formatter
        }

        func string(_ seconds: TimeInterval) -> String {
            lock.lock()
            defer { lock.unlock() }
            return formatter.string(from: seconds) ?? ""
        }
    }
}
