import Foundation

/// What a search actually found, across both the redacted body and the
/// original session.
///
/// `tc_preview_search` scans the REDACTED body, which is the right thing
/// for "what would be sent" and the wrong thing for "was it ever here". A
/// value the scrubber removed returns zero matches, and so does a value that
/// never existed. This type is the difference between those.
public enum OriginalSearchOutcome: Equatable {
    /// Not in the session at all.
    case absent
    /// It was there, and scrubbing took all of it. The count is how many
    /// times it appeared originally.
    case allRemoved(Int)
    /// Still present in what would be sent. The alarming case.
    case someRemain(remaining: Int, total: Int)
    /// The original could not be checked. Never reported as `absent`.
    case unknown

    /// `remaining` is the match count in the redacted body; `original` is
    /// the count in the pre-redaction session, or nil if that call failed.
    public static func classify(remaining: Int, original: Int?) -> OriginalSearchOutcome {
        guard let original else {
            // Fail toward what is certain. The redacted body is in hand, so
            // matches in it are known; the absence of a check is not the
            // same as a clean result and must never render as one.
            return remaining > 0
                ? .someRemain(remaining: remaining, total: remaining)
                : .unknown
        }
        if remaining > 0 {
            return .someRemain(remaining: remaining, total: max(original, remaining))
        }
        return original > 0 ? .allRemoved(original) : .absent
    }

    public var sentence: String {
        switch self {
        case .absent:
            return "0 matches -- not in this session"
        case .allRemoved(let count):
            return "\(count) matches -- all \(count) were removed"
        case .someRemain(let remaining, let total):
            return "\(total) matches -- \(remaining) would still be sent"
        case .unknown:
            return "0 matches in what would be sent. Couldn't check the original."
        }
    }

    /// Whether this is the answer to slow down on.
    public var isAlarming: Bool {
        if case .someRemain = self { return true }
        return false
    }
}
