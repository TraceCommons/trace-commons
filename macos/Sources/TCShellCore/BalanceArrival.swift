import Foundation

/// Whether the Balance tile marks NEAR AI credit arriving, and how.
///
/// Pure: the view decides nothing but when to ask. Credit has arrived when
/// the daemon's `received_total_nanos` is larger than it was when this app
/// last drew the balance -- never inferred from the balance itself, which
/// spending also moves. Every arrival is marked, however small; the 25%
/// rule is the daemon's, and only for notifications.
public enum BalanceArrival {
    /// What the tile last drew, or draws now.
    public struct Reading: Equatable, Sendable {
        public let receivedNanos: Int64?
        public let remainingNanos: Int64?

        public init(receivedNanos: Int64?, remainingNanos: Int64?) {
            self.receivedNanos = receivedNanos
            self.remainingNanos = remainingNanos
        }

        public init(_ status: BalanceStatus) {
            self.init(receivedNanos: status.receivedTotalNanos, remainingNanos: status.remainingNanos)
        }
    }

    public enum Effect: Equatable, Sendable {
        case none
        /// Count the figure up from the one last shown to the new one, with
        /// the highlight.
        case countUp(fromNanos: Int64, toNanos: Int64)
        /// The highlight alone, still: under Reduce Motion, or with no
        /// figure that moved to count.
        case highlight
    }

    public static func effect(shown: Reading?, now: Reading, reduceMotion: Bool) -> Effect {
        guard let before = shown?.receivedNanos, let after = now.receivedNanos, after > before else {
            return .none
        }
        if reduceMotion { return .highlight }
        guard let from = shown?.remainingNanos, let to = now.remainingNanos, from != to else {
            return .highlight
        }
        return .countUp(fromNanos: from, toNanos: to)
    }

    /// The figures a count passes through, one per frame, ending exactly on
    /// `toNanos`.
    public static func countSteps(fromNanos: Int64, toNanos: Int64, frames: Int) -> [Int64] {
        guard frames > 0 else { return [toNanos] }
        let delta = Double(toNanos) - Double(fromNanos)
        var steps = (1..<frames).map { frame in
            fromNanos + Int64((delta * Double(frame) / Double(frames)).rounded(.down))
        }
        steps.append(toNanos)
        return steps
    }

    /// Tracks credit arrivals for the Dock and menu-bar icon animations.
    ///
    /// Every real increase in Trace Commons `credit_final` or NEAR AI
    /// `received_total_nanos` is celebrated. The baseline is saved per account
    /// (`save(forScope:in:)`), so credit that arrived while the app was closed
    /// or signed out plays once on the next reading. An account's first
    /// reading only sets its baseline, a transient unavailable reading (`nil`)
    /// preserves it, and a smaller total moves it down silently. Tracking
    /// `credit_final` keeps the animations working without a NEAR AI sign-in.
    public struct Tracker: Equatable, Sendable {
        public private(set) var lastReceivedNanos: Int64?
        public private(set) var lastCreditFinal: Double?

        /// The smallest `credit_final` change that counts as an arrival.
        /// The daemon sums single-precision values, so re-reading an unchanged
        /// total can differ in its last digits.
        static let creditNoiseFloor: Double = 0.001

        /// `UserDefaults` key for the saved baseline. Holds the account's
        /// hashed scope and its two totals.
        public static let baselineKey = "CreditIconArrivalBaseline"
        private static let scopeField = "scope"
        private static let receivedField = "received_nanos"
        private static let creditField = "credit_final"

        public init(lastReceivedNanos: Int64? = nil, lastCreditFinal: Double? = nil) {
            self.lastReceivedNanos = lastReceivedNanos
            self.lastCreditFinal = lastCreditFinal
        }

        /// The baseline saved for `scope`, or an empty tracker when there is
        /// no scope or the saved baseline belongs to another account.
        public static func stored(forScope scope: String?, in defaults: UserDefaults = .standard) -> Tracker {
            guard let scope,
                  let saved = defaults.dictionary(forKey: baselineKey),
                  saved[scopeField] as? String == scope
            else { return Tracker() }
            return Tracker(
                lastReceivedNanos: (saved[receivedField] as? String).flatMap(Int64.init),
                lastCreditFinal: (saved[creditField] as? Double).flatMap { $0.isFinite ? $0 : nil }
            )
        }

        /// Saves this baseline for `scope`. Without a scope nothing is saved:
        /// there is no account to attribute it to.
        public func save(forScope scope: String?, in defaults: UserDefaults = .standard) {
            guard let scope else { return }
            var saved: [String: Any] = [Self.scopeField: scope]
            if let lastReceivedNanos { saved[Self.receivedField] = String(lastReceivedNanos) }
            if let lastCreditFinal { saved[Self.creditField] = lastCreditFinal }
            defaults.set(saved, forKey: Self.baselineKey)
        }

        /// Records `receivedTotalNanos` and returns `true` when it is larger
        /// than the baseline.
        public mutating func observe(_ receivedTotalNanos: Int64?) -> Bool {
            guard let now = receivedTotalNanos else { return false }
            guard let before = lastReceivedNanos else {
                lastReceivedNanos = now
                return false
            }
            guard now != before else { return false }
            lastReceivedNanos = now
            return now > before
        }

        /// Records Trace Commons settled `credit_final` from `history_rollup`
        /// and returns `true` when it rose by more than rounding noise.
        public mutating func observeCreditFinal(_ creditFinal: Double?) -> Bool {
            guard let now = creditFinal, now.isFinite else { return false }
            guard let before = lastCreditFinal else {
                lastCreditFinal = now
                return false
            }
            guard abs(now - before) >= Self.creditNoiseFloor else { return false }
            lastCreditFinal = now
            return now > before
        }
    }

    /// Frame timing for the Dock and menu-bar icon credit-arrival animations.
    /// Pure, so both surfaces share one schedule and tests can assert it
    /// without sleeping.
    public enum IconAnimation {
        /// UserDefaults key for Settings > General's toggle. Defaults to `true`
        /// (enabled) when unset.
        public static let preferenceKey = "CreditIconAnimationEnabled"

        /// Whether the Dock and menu-bar icon animations are enabled in `defaults`.
        public static func isEnabled(in defaults: UserDefaults = .standard) -> Bool {
            (defaults.object(forKey: preferenceKey) as? Bool) ?? true
        }

        /// Matches `GlassCoinBurst.duration`.
        public static let duration: Double = 1.7
        /// Frame rate for bitmap-rendered icon animations (menu bar + Dock).
        public static let framesPerSecond: Double = 24
        /// Still moment held under Reduce Motion, matching `GlassCoinBurst.stillAt`.
        public static let stillAt: Double = 0.45
        /// How long the still frame stays under Reduce Motion, matching
        /// `BalanceArrivalFlourish.hold`.
        public static let reduceMotionHold: Duration = .milliseconds(1600)

        /// Interval between frames when animating.
        public static var frameInterval: Duration {
            .milliseconds(Int((1000.0 / framesPerSecond).rounded()))
        }

        /// Elapsed timestamps (in seconds) to render for one celebration.
        public static func frameTimes(reduceMotion: Bool) -> [Double] {
            if reduceMotion { return [stillAt] }
            let count = max(1, Int((duration * framesPerSecond).rounded(.up)))
            return (0..<count).map { Double($0) / framesPerSecond }
        }
    }
}
