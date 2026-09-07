import CTraceCommons
import Foundation

/// The harness list's branch tables and its two sentences, across the C ABI.
///
/// Handle-free, like `TCPrivateInference`: none of these describes a running
/// daemon. The list itself arrives over the daemon socket; what crosses here
/// is only the deciding -- which state a label means, which outcome an
/// outcome means, whether an action may be offered at all, and which sentence
/// a state gets. A branch written three times in three languages agrees today
/// and drifts in silence tomorrow, which is why none of them is written here.
///
/// Nothing in this file authors a sentence and nothing in it picks a colour.
/// The two calls that return text return whatever the shared copy module
/// composed; this shell only hands the answer on.
public enum TCHarness {
    /// One `harness_list` row's `state`, as a raw `TC_HARNESS_STATE_*` value.
    ///
    /// There is no failure value: an unfamiliar label and a caught panic both
    /// answer the code that claims nothing. Never recover the state by
    /// matching on the label in Swift -- "answering" is the one value meaning
    /// a call was actually served, and it is the one a shell is most tempted
    /// to infer from "connected".
    public static func stateCode(state: String) -> Int32 {
        state.withCString { tc_harness_state_code($0) }
    }

    /// One `harness_plan` result's `outcome`, as a raw `TC_HARNESS_PLAN_*`
    /// value.
    ///
    /// The branch that matters is unparseable against noop, and it is decided
    /// on the far side so all three shells decide it the same way.
    public static func planOutcomeCode(outcome: String) -> Int32 {
        outcome.withCString { tc_harness_plan_outcome_code($0) }
    }

    /// The sentence for one `harness_list` row's `state`.
    ///
    /// THE SENTENCE CROSSES, NOT ONLY THE CODE. This shell used to switch on
    /// the decoded state and pick one of `PrivateInferenceCopy`'s fields,
    /// and so did the other two: three copies of one decision. Ask for it.
    ///
    /// An EMPTY STRING is the answer for `activity_shared` and `unknown`,
    /// and it means draw no line at all. Neither may borrow the answering
    /// sentence: that sentence says a call from *it* reached this computer,
    /// and the pronoun names the row's own tool -- exactly what
    /// `activity_shared` says cannot be worked out. `nil` only on a caught
    /// panic, which is drawn the same way.
    public static func stateLine(state: String) -> String {
        guard let raw = state.withCString({ tc_harness_state_line($0) }) else { return "" }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// When the last call from a connected tool was answered here.
    ///
    /// `secondsAgo` is how long ago this shell worked out that call arrived.
    /// ABSENCE IS AN OUT-OF-RANGE INTEGER, the convention
    /// `tc_private_inference_serving_line` already uses: pass a negative
    /// value for an absent or unparseable timestamp and the answer is the
    /// empty string, drawn as no line at all.
    ///
    /// Before this crossed the ABI the sentence existed in Rust and was
    /// reachable only by the shell that links the crate natively, so this
    /// app drew nothing where the GNOME one drew a line.
    public static func lastCallLine(secondsAgo: Int64) -> String {
        guard let raw = tc_harness_last_call_line(secondsAgo) else { return "" }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// Whether one action may be offered for a tool in this state.
    ///
    /// Answers false -- do not offer -- for an action this build does not
    /// know and on a caught panic.
    public static func actionAvailable(action: String, installed: Bool, connected: Bool) -> Bool {
        action.withCString {
            tc_harness_action_available($0, installed ? 1 : 0, connected ? 1 : 0) != 0
        }
    }
}
