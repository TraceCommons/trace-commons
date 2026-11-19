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

    /// Tracks live credit arrivals during a running app session, for the Dock
    /// and menu-bar icon animations.
    ///
    /// Unlike `BalanceArrivalFlourish` (which persists in UserDefaults so the
    /// Balance tile marks credit that arrived while the app was closed), the
    /// Dock and menu-bar icons only celebrate credit that arrives while the app
    /// is running: the first known reading establishes the baseline, transient
    /// unavailable readings (`nil`) preserve it, and any strictly larger
    /// Trace Commons `credit_final` or NEAR AI `received_total_nanos` triggers
    /// a celebration. Tracking `credit_final` ensures icon animations still
    /// fire when the contributor is not signed into NEAR AI.
    public struct Tracker: Equatable, Sendable {
        public private(set) var lastReceivedNanos: Int64?
        public private(set) var lastCreditFinal: Double?

        /// Minimum Trace Commons credit increase (1 credit) required since the
        /// last celebration before animating the Dock and menu-bar icons.
        public static let minimumCreditFinal: Double = 1.0
        /// Minimum NEAR AI received increase ($1.00 = 1e9 nanos) required since
        /// the last celebration before animating the Dock and menu-bar icons.
        public static let minimumReceivedNanos: Int64 = 1_000_000_000

        static let creditEpsilon: Double = 1e-6

        public init(lastReceivedNanos: Int64? = nil, lastCreditFinal: Double? = nil) {
            self.lastReceivedNanos = lastReceivedNanos
            self.lastCreditFinal = lastCreditFinal
        }

        /// Records `receivedTotalNanos` and returns `true` once at least
        /// `minimumReceivedNanos` of new credit has accumulated since the
        /// previous celebrated baseline.
        public mutating func observe(_ receivedTotalNanos: Int64?) -> Bool {
            guard let now = receivedTotalNanos else { return false }
            guard let before = lastReceivedNanos else {
                lastReceivedNanos = now
                return false
            }
            if now < before {
                lastReceivedNanos = now
                return false
            }
            guard now - before >= Self.minimumReceivedNanos else { return false }
            lastReceivedNanos = now
            return true
        }

        /// Records Trace Commons settled `credit_final` from `history_rollup`
        /// and returns `true` once at least `minimumCreditFinal` (1 credit)
        /// has accumulated since the previous celebrated baseline.
        public mutating func observeCreditFinal(_ creditFinal: Double?) -> Bool {
            guard let now = creditFinal, now.isFinite else { return false }
            guard let before = lastCreditFinal else {
                lastCreditFinal = now
                return false
            }
            if now < before - Self.creditEpsilon {
                lastCreditFinal = now
                return false
            }
            guard now - before >= Self.minimumCreditFinal - Self.creditEpsilon else { return false }
            lastCreditFinal = now
            return true
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
