//! Turning daemon state into words and widths.
//
// Every rule here has a counterpart in `src/render.rs`, and the point of
// stating them in one file is that the two surfaces cannot quietly disagree. If
// `ironwire status` says a pool is nearly full and the menu bar draws it half
// empty, one of them is lying and the user has no way to tell which.
//
// Nothing here decides anything about routing. These are presentation rules
// over values the daemon already computed.

import Foundation

public enum Format {
    // MARK: - The capacity bar

    /// How full to draw a capacity bar, or `nil` for no bar at all.
    ///
    /// **This is the function the whole app most needs to get right.**
    /// `SwiftUI.ProgressView` wants a `Double`, and for every state but
    /// `observed` there isn't an honest one. Zero reads as "empty", a half-grey
    /// bar reads as "about half", and both are numbers IronWire made up in the
    /// one surface a user takes in without reading (`AGENTS.md` rule 2,
    /// `docs/CRITIQUE.md` §4). So the answer is no bar, and the caller renders
    /// the words instead.
    ///
    /// `exhausted` and `capReached` return `nil` for the same reason rather than
    /// a full bar: neither is a measurement of how much of the window went, and
    /// a bar pinned to 100% would claim one.
    public static func capacityFraction(_ headroom: HeadroomView) -> Double? {
        switch headroom {
        case .observed(let usedPct, _, _):
            return min(max(usedPct, 0), 100) / 100
        case .exhausted, .capReached, .unknown, .unrecognised:
            return nil
        }
    }

    /// How many cells of a `width`-wide meter are filled.
    ///
    /// The arithmetic of `meter()` in `src/render.rs`, kept here so a screenshot
    /// of the menu and a screenshot of `ironwire status` can be laid side by
    /// side without the bars disagreeing.
    public static func meterFilled(usedPct: Double, width: Int = 10) -> Int {
        let clamped = min(max(usedPct, 0), 100)
        return min(Int((clamped / 100 * Double(width)).rounded()), width)
    }

    /// The three-way scale `Style::by_usage` colours by (`src/style.rs`).
    public enum UsageLevel: Sendable, Equatable {
        case good, warn, bad
    }

    /// Which band a percentage falls in. Thresholds are the CLI's: 90 and 50.
    public static func usageLevel(usedPct: Double) -> UsageLevel {
        if usedPct >= 90 { return .bad }
        if usedPct >= 50 { return .warn }
        return .good
    }

    // MARK: - Words

    /// The capacity line, worded as `render::headroom()` words it.
    ///
    /// `unknown` says so at length on purpose: "the provider has not reported"
    /// is what makes the rows that *do* carry a number worth believing.
    public static func headroomSummary(_ headroom: HeadroomView) -> String {
        switch headroom {
        case .observed(let usedPct, let observedSecsAgo, let resetsInSecs):
            var line = "\(Int(usedPct.rounded()))% used"
            if let resets = resetsInSecs, resets > 0 {
                line += " · resets in \(duration(resets))"
            }
            return line + " · observed \(duration(observedSecsAgo)) ago"
        case .exhausted(let retryInSecs):
            return "exhausted · retry in \(duration(retryInSecs))"
        // Said differently from `exhausted` deliberately: the provider would
        // have served this one. The user is who said stop.
        case .capReached(let spentUsd, let capUsd, let resetsInSecs):
            return "cap reached — \(currency(spentUsd)) of \(currency(capUsd)) · resets in \(duration(resetsInSecs))"
        case .unknown:
            return "unknown (the provider has not reported yet)"
        case .unrecognised:
            return "unknown to this version of the app"
        }
    }

    /// Why a backend is being skipped, when it is. `nil` for the boring answer —
    /// a "circuit: closed" row on every healthy backend is noise that trains
    /// people to stop reading the block.
    public static func healthSummary(_ health: HealthView) -> String? {
        if health.isOpen {
            if let retry = health.retryInSecs, retry > 0 {
                return "skipping after \(health.consecutiveFailures) consecutive failures · next try in \(duration(retry))"
            }
            return "skipping after \(health.consecutiveFailures) consecutive failures"
        }
        if health.isRecovering { return "recovering — trying it again now" }
        if health.consecutiveFailures > 0 {
            return "\(health.consecutiveFailures) recent failure(s), still in use"
        }
        return nil
    }

    /// Where traffic went, as the status line puts it: `claude-sub → nearai`
    /// when the conversation moved, the backend alone when it did not.
    public static func routeSummary(_ route: LastRouteView) -> String {
        if let from = route.from, from != route.backend {
            return "\(from) → \(route.backend)"
        }
        return route.backend
    }

    /// `duration()` from `src/render.rs`, digit for digit.
    public static func duration(_ secs: Int) -> String {
        let secs = max(secs, 0)
        if secs < 60 { return "\(secs)s" }
        if secs < 3600 { return "\(secs / 60)m" }
        let hours = secs / 3600
        let minutes = (secs % 3600) / 60
        return minutes == 0 ? "\(hours)h" : "\(hours)h\(minutes)m"
    }

    /// How long ago something happened, on the same scale.
    public static func relative(_ date: Date, now: Date = Date()) -> String {
        duration(Int(now.timeIntervalSince(date)))
    }

    /// Dollars, to the cent.
    ///
    /// `+ 0` because summing no dollars at all gives `-0.0` in IEEE 754, and
    /// "$-0.00" reads like a refund — the same guard `balance_block` carries.
    public static func currency(_ amount: Double) -> String {
        String(format: "$%.2f", amount + 0)
    }

    /// Names the way a sentence would: "a", "a and b", "a, b and c".
    ///
    /// A banner that reads "Claude Code, Codex not routed" is a list; one that
    /// reads "Claude Code and Codex are not routed" is a sentence, and the
    /// difference is whether someone finishes reading it.
    public static func list(_ items: [String]) -> String {
        switch items.count {
        case 0: return ""
        case 1: return "\(items[0]) is"
        default:
            let last = items[items.count - 1]
            let rest = items.dropLast().joined(separator: ", ")
            return "\(rest) and \(last) are"
        }
    }

    /// Backend kind as a phrase rather than an identifier.
    public static func kind(_ raw: String) -> String {
        raw.replacingOccurrences(of: "_", with: " ")
    }
}
