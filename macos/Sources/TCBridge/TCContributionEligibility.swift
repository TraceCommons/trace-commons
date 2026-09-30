import CTraceCommons
import Foundation

/// The contribution-eligibility surface's four branch tables, across the C
/// ABI.
///
/// Handle-free, like `TCNearAiCredential`: every call here describes the
/// build, not a running daemon. The sentence, the tone and whether a send
/// control is offered all come from
/// `crates/trace-commons-contributor/src/private_inference_copy.rs`, so this
/// shell, the GTK shell and the Windows shell say one thing about a session
/// and offer one button beside it.
///
/// Nothing in this file authors a sentence and nothing in it decides a tone.
/// A `nil` means a caught Rust panic, and the caller renders the payload's
/// unknown sentence rather than filling the hole in.
///
/// **Call none of this for an entry that carried no `eligibility` field.**
/// An absent field means the contributor was invited and has no eligibility
/// question; answering one they do not have puts a caveat on work that
/// carries none. Absent is not "unknown" -- `ContributionEligibility.parse`
/// is where that distinction is kept.
public enum TCContributionEligibility {
    /// The sentence for one `eligibility` state label.
    ///
    /// An empty or unfamiliar label reports that the answer has not been
    /// worked out. IT NEVER REPORTS AN INELIGIBILITY: a state this build
    /// cannot read is not evidence about a contributor's session, and saying
    /// it is would stop them offering work that is fine.
    public static func stateLine(state: String) -> String? {
        guard let raw = state.withCString({ tc_contribution_eligibility_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value -- the same five the listener state row uses.
    ///
    /// Takes what the sentence takes, so the two stay in step. There is no
    /// failure value: an unknown state and a caught panic both answer the
    /// tone that claims nothing. A permanent ineligibility is deliberately
    /// NOT refused -- nothing was refused.
    public static func stateTone(state: String) -> Int32 {
        state.withCString { tc_contribution_eligibility_tone($0) }
    }

    /// The one control this state may offer, as a raw
    /// `TC_CONTRIBUTION_CONTROL_*` value.
    ///
    /// The branch table crosses, not only the words. Three shells each
    /// deciding which rows get a send button is three chances to offer one
    /// beside a session the server will refuse.
    public static func control(state: String) -> Int32 {
        state.withCString { tc_contribution_eligibility_control($0) }
    }

    /// The sentence for one `eligibility_reason` label, or the empty string.
    ///
    /// The empty string for an unfamiliar, empty or absent reason, and the
    /// caller renders nothing for it. That is deliberately different from the
    /// state line's fallback: an unknown state still has to say something, an
    /// unknown reason has nothing honest to say.
    public static func reasonLine(reason: String) -> String? {
        guard let raw = reason.withCString({ tc_contribution_eligibility_reason_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// Whether a group's submit control may be offered, as a raw
    /// `TC_CONTRIBUTION_CONTROL_*` value.
    ///
    /// `pending` is a `list_projects` row's `pending_count`. `contributable`
    /// is its `contributable_count`, **or any negative value when that key
    /// was absent** -- an invited contributor, for whom every pending
    /// session is sendable.
    ///
    /// ABSENT IS NOT ZERO. `0` means the question applies and nothing here
    /// can be sent; a negative means the question does not apply and the
    /// control is offered on `pending` alone. Passing `0` for an absent
    /// field would refuse the control to somebody whose sessions are all
    /// perfectly sendable -- the same trap as reading an absent
    /// `eligibility` as `unknown`, one field over.
    public static func groupControl(pending: Int64, contributable: Int64) -> Int32 {
        tc_contribution_group_control(pending, contributable)
    }

    /// How many sessions a group submit is leaving behind, as a sentence.
    ///
    /// `withheld` is `approve`'s `excluded_ineligible`, or the difference
    /// between a project row's `pending_count` and its
    /// `contributable_count`.
    ///
    /// The empty string for zero AND for a negative, which no honest caller
    /// produces; the caller renders nothing. There is no gap to explain, and
    /// a line reading "0 sessions are not being sent" invents a caveat where
    /// none exists. The sentence says how many and not why -- the reason a
    /// particular session cannot be sent is that row's own sentence, one
    /// level in.
    public static func withheldLine(withheld: Int64) -> String? {
        guard let raw = tc_contribution_withheld_line(withheld) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
