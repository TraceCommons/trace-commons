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

    /// How loudly to draw this outcome, in the three states the sheet has
    /// words for.
    ///
    /// `unknown` is deliberately its own state rather than falling in with
    /// the clean answers. It used to render in the clear tone, which put the
    /// app's all-clear glyph -- a green checkmark -- next to the sentence
    /// "Couldn't check the original.", so the one outcome that means *no
    /// answer* looked exactly like the one that means *nothing was found*.
    /// That is the single direction this tab must never fail in.
    ///
    /// It is not alarming either: nothing was found, and shouting about a
    /// call that did not come back would spend attention where nothing is
    /// known to be wrong. Neutral is the honest third thing.
    public enum Emphasis: Equatable {
        /// A question that got a clean answer.
        case clear
        /// Still in what would be sent.
        case attention
        /// No answer. Neither reassuring nor alarming.
        case unchecked
    }

    public var emphasis: Emphasis {
        switch self {
        case .someRemain: return .attention
        case .unknown: return .unchecked
        case .absent, .allRemoved: return .clear
        }
    }
}
