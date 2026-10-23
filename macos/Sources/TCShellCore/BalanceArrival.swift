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
}
