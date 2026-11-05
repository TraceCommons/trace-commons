import CTraceCommons
import Foundation

/// The balance row's tables and its one money formatter, across the C ABI.
///
/// Handle-free, like `TCNearAiCredential`: every call here describes the
/// build, not a running daemon. The sentences, the tone, the button and the
/// rounding all come from
/// `crates/trace-commons-contributor/src/private_inference_copy.rs`, so this
/// shell, the GTK shell and the Windows shell print one figure for one
/// balance and offer one button beside it.
///
/// **Nothing in this file formats money and nothing in it judges an amount.**
/// A shell that divided by a billion of its own would be wrong by a factor of
/// a thousand the day the daemon changed `scale`, and a shell that painted a
/// low figure red would be inventing a threshold nobody set on an account
/// whose ceiling may not exist.
///
/// A `nil` means a caught Rust panic, and the caller renders the payload's
/// unreported sentence rather than filling the hole in.
public enum TCNearAiBalance {
    /// The sentence for one `near_ai_balance` state label.
    ///
    /// An empty label reports that this daemon does not answer the question;
    /// an unfamiliar one reports that the answer arrived in a form this build
    /// cannot read. Neither says that no sign-in is kept here, and neither
    /// says the service failed -- those are claims about a machine and about
    /// a service, and the Rust decides them, not this shell.
    ///
    /// `known` answers the EMPTY STRING: that state's row is figures, and a
    /// sentence above them announcing the read succeeded is this app
    /// narrating itself.
    public static func stateLine(state: String) -> String? {
        // Legacy equivalent: tc_near_ai_balance_state_line.
        LocalizedPresentation.line("near_ai_balance_state_line", parameters: ["state": state])
    }

    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value -- the same five every other row on this destination uses.
    ///
    /// `known` is the only `_CLEAR`, and it means THE READ SUCCEEDED rather
    /// than that the balance is healthy. There is no failure value: an
    /// unknown state and a caught panic both answer the tone that claims
    /// nothing.
    public static func stateTone(state: String) -> Int32 {
        state.withCString { tc_near_ai_balance_state_tone($0) }
    }

    /// The one action this state may offer, as a raw
    /// `TC_CREDENTIAL_ACTION_*` value.
    ///
    /// The sign-in row's enum and not a second one. `no_session` and
    /// `session_expired` answer OBTAIN and those two only -- and the refused
    /// session gets it WITHOUT a forget first, because the ceremony
    /// overwrites both records and forgetting would throw away a working key
    /// to fix an unrelated sign-in.
    public static func action(state: String) -> Int32 {
        state.withCString { tc_near_ai_balance_action($0) }
    }

    /// One amount as money.
    ///
    /// `present` is separate from `nanos` because these amounts are SIGNED:
    /// an overdrawn account is a negative figure, so folding "null" onto
    /// "out of range" would render a real debt as no figure at all. A
    /// `present` of 0 gives the EMPTY STRING, and **an empty string is never
    /// `$0.00`** -- a null means "we know we do not know", while a zero is a
    /// real balance and means the money is gone.
    ///
    /// `scale` is the wire's own field. Pass what arrived.
    public static func amount(present: Int32, nanos: Int64, scale: UInt8) -> String? {
        // Legacy equivalent: tc_near_ai_balance_amount.
        LocalizedPresentation.line("near_ai_balance_amount", parameters: ["present": present, "nanos": nanos, "scale": scale])
    }

    /// What is left, as a finished sentence.
    ///
    /// `present == 0` DOES NOT give the empty string here. It gives the
    /// sentence for an account with no spending limit set -- the ordinary
    /// case for an account nobody has capped, and a contributor in it must
    /// not be told they have `$0.00` left.
    public static func remainingLine(present: Int32, nanos: Int64, scale: UInt8) -> String? {
        // Legacy equivalent: tc_near_ai_balance_remaining_line.
        LocalizedPresentation.line("near_ai_balance_remaining_line", parameters: ["present": present, "nanos": nanos, "scale": scale])
    }

    /// The configured ceiling, or the empty string -- which is what an absent
    /// limit gets, because `remainingLine` has already said the part that
    /// matters about an uncapped account.
    public static func limitLine(present: Int32, nanos: Int64, scale: UInt8) -> String? {
        // Legacy equivalent: tc_near_ai_balance_limit_line.
        LocalizedPresentation.line("near_ai_balance_limit_line", parameters: ["present": present, "nanos": nanos, "scale": scale])
    }

    /// What the account has spent -- the WHOLE account, not this computer.
    ///
    /// An absent figure is the empty string, drawn as no line. A zero is not
    /// that: an account that has spent nothing renders `$0.00`, which is
    /// true. `remainingPresent` is whether the same reading's remaining
    /// figure is present: beside one Spent rounds up to the cent so Limit,
    /// Spent and what is left add up; without one it is floored.
    public static func spentLine(present: Int32, nanos: Int64, scale: UInt8, remainingPresent: Int32) -> String? {
        // Legacy equivalent: tc_near_ai_balance_spent_line.
        LocalizedPresentation.line("near_ai_balance_spent_line", parameters: ["present": present, "nanos": nanos, "scale": scale, "remaining_present": remainingPresent])
    }

    /// How long ago this computer asked.
    ///
    /// Any negative value -- what a caller passes for a null `observed_at` --
    /// gives the empty string. The sentence says when the QUESTION WAS PUT,
    /// never that anything was updated then.
    public static func observedLine(secondsAgo: Int64) -> String? {
        // Legacy equivalent: tc_near_ai_balance_observed_line.
        LocalizedPresentation.line("near_ai_balance_observed_line", parameters: ["seconds_ago": secondsAgo])
    }

    /// Signed movement at the core's displayed precision. Nil selects the
    /// canonical unavailable explanation, never a zero change.
    public static func changeLine(nanos: Int64?, scale: UInt8) -> String? {
        LocalizedPresentation.line("near_ai_balance_change_line", parameters: [
            "nanos": nanos.map { $0 as Any } ?? NSNull(), "scale": scale
        ])
    }

    /// Incoming transfers are distinct from net balance movement.
    public static func receivedLine(nanos: Int64?, scale: UInt8) -> String? {
        LocalizedPresentation.line("near_ai_credit_received_24h_line", parameters: [
            "nanos": nanos.map { $0 as Any } ?? NSNull(), "scale": scale
        ])
    }

    /// Only the spending-organization prose tree, selected from the current
    /// listed name. IDs, selection, and authorization never enter this tree.
    public static func spendOrganizationViewJSON(currentName: String?) -> String? {
        LocalizedPresentation.json("spend_org_view", parameters: [
            "current_name": currentName.map { $0 as Any } ?? NSNull()
        ])
    }

    /// A validated funding report's state and display name, not its URL or
    /// credential binding. Localization cannot authorize a destination.
    public static func fundingMessage(state: String, organizationName: String?) -> String? {
        LocalizedPresentation.line("funding_message", parameters: [
            "state": state, "organization_name": organizationName ?? ""
        ])
    }
}
