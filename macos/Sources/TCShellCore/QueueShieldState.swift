import Foundation

/// What the queue's nav item says about the queue, beyond how much is in it.
///
/// This is deliberately NOT a replacement for the numeric badge. The ask was
/// to swap the count for an icon; the count is the signal a contributor with
/// 149 waiting sessions is actually reading, and an icon meaning "some" is a
/// downgrade exactly at the scale that prompted the request. The shield adds
/// a state the count cannot carry. The two go together.
public enum QueueShieldState: Equatable {
    /// Nothing waiting.
    case clear
    /// Sessions waiting, none of them flagged.
    case waiting
    /// Something in the queue is worth a second look: a session where no
    /// pattern fired, or one trimmed to fit the byte budget.
    case attention

    public static func state(waiting: Int, nothingMatched: Int, trimmed: Int) -> QueueShieldState {
        guard waiting > 0 else { return .clear }
        return (nothingMatched > 0 || trimmed > 0) ? .attention : .waiting
    }
}
