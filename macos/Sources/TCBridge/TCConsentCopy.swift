import CTraceCommons
import Foundation

/// The consent surface's sentences, read from the Rust rather than written
/// here.
///
/// Handle-free for the same reason `TCRoutingCopy` is: it describes the
/// build, not a running daemon.
///
/// Nothing in this file is a word, and nothing in it is a branch. The
/// sentences cross as JSON and the choice between the two tooltips crosses
/// as its own call.
public enum TCConsentCopy {
    /// Every fixed sentence on the surface, as a JSON object, or nil if the
    /// ABI reported a caught panic. Decoded by `TCShellCore.ConsentCopy`.
    public static func copyJSON() -> String? {
        guard let raw = tc_consent_copy() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The tooltip that explains the current answer, chosen by the ABI.
    ///
    /// Nil only on a caught panic. Do not recover this by picking between
    /// the two sentences from `copyJSON`: the branch crosses so that three
    /// shells cannot each keep their own copy of it.
    public static func gateHelp(pinned: Bool) -> String? {
        guard let raw = tc_consent_gate_help(pinned ? 1 : 0) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The notice for one void, as a JSON object, from one element of
    /// `status.grant_voids` passed through as the daemon sent it. Decoded by
    /// `TCShellCore.GrantVoidNotice`.
    ///
    /// Nil when the ABI cannot read the element (an unknown kind, a project
    /// void without a label) or on a caught panic. The choice between the
    /// project and the grant wording is made by the ABI, not here.
    public static func voidNoticeJSON(forVoid wireJSON: String) -> String? {
        let raw = wireJSON.withCString { tc_grant_void_notice($0) }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The notice for one armed folder whose arming wording no longer claims
    /// a model scrubs its sessions (K5), from one element of
    /// `status.arming_rewordings` passed through (`ArmingRewordingWire.json`).
    /// Decoded by `TCShellCore.ArmingRewordedNotice`. Nil only for an
    /// argument that is not an element, or on a caught panic.
    public static func armingRewordedNoticeJSON(forRewording wireJSON: String) -> String? {
        let raw = wireJSON.withCString { tc_arming_reworded_notice($0) }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The notice for armed folders the automatic-contribution gate is
    /// holding, from `status.automatic_contribution_held` passed through
    /// (`GateHeld.json`). Decoded by `TCShellCore.GateHeldNotice`. Nil when
    /// nothing is held, for an unreadable argument, or on a caught panic.
    public static func gateHeldNoticeJSON(forHeld wireJSON: String) -> String? {
        let raw = wireJSON.withCString { tc_gate_held_notice($0) }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The notice after a legacy invite identity moved to a NEAR AI account,
    /// as a JSON object, from `status.legacy_invite_migration.notice`
    /// (`LegacyMigrationWire.noticeJSON`). Decoded by
    /// `TCShellCore.LegacyMigrationNotice`.
    ///
    /// Nil for `null` (nothing to show), an unreadable argument, or a caught
    /// panic. Which folders sentence to show is the ABI's choice.
    public static func legacyMigrationNoticeJSON(forNotice wireJSON: String) -> String? {
        let raw = wireJSON.withCString { tc_legacy_migration_notice($0) }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The notice for approved sessions held on a busy privacy witness, as a
    /// JSON object, from `status.witness_capacity` (`WitnessCapacity.wireJSON`).
    /// Decoded by `TCShellCore.WitnessCapacityNotice`.
    ///
    /// Nil when nothing is waiting, for an unreadable argument, or on a
    /// caught panic. The count and its wording are the ABI's.
    public static func witnessCapacityNoticeJSON(forCapacity wireJSON: String) -> String? {
        let raw = wireJSON.withCString { tc_witness_capacity_notice($0) }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
