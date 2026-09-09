import Foundation

/// Every fixed sentence on the private-inference offer and settings card.
///
/// A pure `Decodable` view of `tc_private_inference_copy`'s payload. No
/// property has a default: a payload missing a field is refused whole rather
/// than rendered with a blank where a sentence should be, and on this surface
/// the blank could be the sentence about what turning the switch on exposes.
public struct PrivateInferenceCopy: Decodable, Equatable, Sendable {
    /// The switcher label for the top-level destination, from the Rust.
    public let destination: String
    /// The line under the destination's title, from the Rust.
    public let subtitle: String
    public let offerTitle: String
    public let offerWhat: String
    public let offerExposure: String
    public let offerNoRepoint: String
    public let offerAccept: String
    public let offerDecline: String
    public let offerAskedOnce: String
    public let settingsTitle: String
    public let settingsToggle: String
    public let settingsAppliesAtOnce: String
    public let stateUnreported: String
    public let stateUnknown: String
    public let stateStopping: String
    public let stateOff: String
    public let stateRunning: String
    public let stateRunningNoBackends: String
    public let stateRunningAnsweredElsewhere: String
    public let stateRunningDestinationUnknown: String
    public let stateRunningElsewhere: String
    public let statePortInUse: String
    public let stateStartFailed: String
    public let stateCrashed: String
    public let quitAlsoStops: String
    public let writeUnconfirmed: String
    public let settingsMoved: String
    public let trayTurnOff: String
    public let trayOpenToTurnOn: String
    /// The heading over the list of tools found on this computer.
    public let harnessesTitle: String
    /// The line under that heading, qualifying what the list is.
    public let harnessesWhat: String
    /// What the amount `HarnessSurface.spendSentence` names does and does
    /// not cover. Drawn beside that sentence and only when it is drawn.
    public let harnessesSpendScope: String
    public let harnessNotConnected: String
    public let harnessConnectedNothingSeen: String
    /// The only per-harness state that means a call was answered.
    public let harnessAnswering: String
    public let harnessConnect: String
    public let harnessDisconnect: String
    public let harnessPreviewTitle: String
    public let harnessPreviewConfirm: String
    public let harnessPreviewCancel: String
    /// A slot already in use, reported and never offered.
    public let harnessSlotTaken: String
    public let harnessNeedsRestart: String
    public let harnessesNoneFound: String
    /// A file that could not be read, refused rather than rewritten.
    public let harnessUnreadableConfig: String
    /// A tool this app could not find on this computer.
    ///
    /// Rendered INSTEAD of `harnessNotConnected` and never beside it: that
    /// sentence says a tool's own settings still send its calls wherever they
    /// went before, which is a claim about the settings of something that is
    /// not on this computer.
    public let harnessNotInstalled: String
    public let harnessPlanNothingToChange: String
    public let harnessPlanEntryUnusable: String
    public let harnessPlanNoConfigPath: String

    /// The heading over the sign-in card.
    public let credentialTitle: String
    /// What holding a key of one's own changes about this destination.
    public let credentialWhat: String
    /// The three consequences of pressing Obtain: a browser opens, the
    /// contributor signs in with a company that is not this app, and a key
    /// is minted and kept here.
    ///
    /// Drawn WHEREVER Obtain is offered, and never separated from it. The
    /// button is the only control on this shell that opens a browser and
    /// mints something at a third party, and a contributor who pressed it
    /// having read only its label was not told what they agreed to.
    public let credentialCost: String
    public let credentialObtain: String
    public let credentialCancel: String
    public let credentialForget: String
    /// That forgetting is local: the key stays valid at the service until
    /// the contributor removes it in their own account. Drawn WHEREVER
    /// Forget is offered -- `handle_forget`'s `revoked: false` in words, and
    /// a button labelled only "Forget" reads as a revocation it is not.
    public let credentialForgetExplains: String
    public let credentialAbsent: String
    public let credentialObtaining: String
    public let credentialFailed: String
    public let credentialCancelled: String
    public let credentialPresent: String
    /// A state label this build has never heard of.
    ///
    /// Its own sentence, and it must never degrade to `credentialAbsent`:
    /// that is a claim about what this machine holds, and a contributor who
    /// already has a key would read it as an invitation to mint a second one
    /// in their own account that nothing here would ever mention again.
    public let credentialUnknown: String
    /// A daemon that does not answer the question at all. Distinct from
    /// `credentialUnknown` for the same reason and by the same rule.
    public let credentialUnreported: String
    /// Why a connect control is not on offer. Drawn from
    /// `tc_harness_credential_notice` and never from this shell's own
    /// reading of a `destination_credentialed` field.
    public let harnessNeedsCredential: String

    /// The four `eligibility` states a queue entry can carry.
    ///
    /// Never picked by a `switch` here: the sentence for a state comes back
    /// from `tc_contribution_eligibility_line`, and these are carried so a
    /// test can pin what that table answered against the set this build was
    /// compiled with. `eligibilityUnknown` is the one an unfamiliar state
    /// reaches, and it must never degrade into an ineligibility.
    public let eligibilityEligible: String
    public let eligibilityIneligiblePermanent: String
    public let eligibilityIneligibleConfiguration: String
    public let eligibilityUnknown: String
    /// The thirteen `eligibility_reason` labels, in the order the Rust
    /// declares them. An `eligible` row carries no reason at all, and an
    /// unfamiliar one renders nothing rather than borrowing one of these.
    public let eligibilityReasonNoCall: String
    public let eligibilityReasonCaptureOff: String
    public let eligibilityReasonDigestAbsent: String
    public let eligibilityReasonUpstreamIdAbsent: String
    public let eligibilityReasonDigestMismatch: String
    public let eligibilityReasonReferenceMalformed: String
    public let eligibilityReasonBodiesUnreadable: String
    public let eligibilityReasonBodyNotUtf8: String
    public let eligibilityReasonBodyTooLarge: String
    public let eligibilityReasonEvidenceCaptureOff: String
    public let eligibilityReasonMarkerAbsent: String
    public let eligibilityReasonRequestMalformed: String
    public let eligibilityReasonReceiptUnavailable: String

    /// The four `attestation` marks a queue entry can carry, and the thirteen
    /// `attestation_reason` sentences.
    ///
    /// Separate sentences from the `eligibility*` fields above over the SAME
    /// thirteen reason labels. Five of the eligibility sentences say the
    /// session cannot be sent, which is true for an evidence-admitted
    /// contributor and false for an invited one, whose session sends
    /// perfectly well and merely arrives without a copy of its call. Do not
    /// render one where the other belongs.
    public let attestationAttested: String
    public let attestationUnattestedPermanent: String
    public let attestationUnattestedConfiguration: String
    public let attestationUnknown: String
    public let attestationReasonNoCall: String
    public let attestationReasonCaptureOff: String
    public let attestationReasonDigestAbsent: String
    public let attestationReasonUpstreamIdAbsent: String
    public let attestationReasonDigestMismatch: String
    public let attestationReasonReferenceMalformed: String
    public let attestationReasonBodiesUnreadable: String
    public let attestationReasonBodyNotUtf8: String
    public let attestationReasonBodyTooLarge: String
    public let attestationReasonEvidenceCaptureOff: String
    public let attestationReasonMarkerAbsent: String
    public let attestationReasonRequestMalformed: String
    public let attestationReasonReceiptUnavailable: String

    /// The balance row: its heading, what it is a fact about, and the seven
    /// sentences a state can reach.
    ///
    /// Never picked by a `switch` here: the sentence for a state comes back
    /// from `tc_near_ai_balance_state_line`, and these are carried so a test
    /// can pin what that table answered against the set this build was
    /// compiled with. Nothing on this row judges an amount -- a shell that
    /// painted a low balance as a warning would be inventing a claim the
    /// Rust does not make.
    public let balanceTitle: String
    public let balanceWhat: String
    public let balanceNoSession: String
    public let balanceSessionExpired: String
    public let balanceNoOrganization: String
    public let balanceUnavailable: String
    public let balanceUnknown: String
    public let balanceUnreported: String
    /// The sentence a `null` remaining figure gets INSTEAD of `$0.00`. An
    /// unreported figure is not a spent-out account.
    public let balanceNoRemaining: String

    /// `CaseIterable` so a test on the far side can compare the exported
    /// field set against the declared one in BOTH directions -- a field the
    /// Rust grows and this struct drops would sail past a test that only
    /// checked the fields it already knows about.
    public enum CodingKeys: String, CodingKey, CaseIterable {
        case destination
        case subtitle
        case offerTitle = "offer_title"
        case offerWhat = "offer_what"
        case offerExposure = "offer_exposure"
        case offerNoRepoint = "offer_no_repoint"
        case offerAccept = "offer_accept"
        case offerDecline = "offer_decline"
        case offerAskedOnce = "offer_asked_once"
        case settingsTitle = "settings_title"
        case settingsToggle = "settings_toggle"
        case settingsAppliesAtOnce = "settings_applies_at_once"
        case stateUnreported = "state_unreported"
        case stateUnknown = "state_unknown"
        case stateStopping = "state_stopping"
        case stateOff = "state_off"
        case stateRunning = "state_running"
        case stateRunningNoBackends = "state_running_no_backends"
        case stateRunningAnsweredElsewhere = "state_running_answered_elsewhere"
        case stateRunningDestinationUnknown = "state_running_destination_unknown"
        case stateRunningElsewhere = "state_running_elsewhere"
        case statePortInUse = "state_port_in_use"
        case stateStartFailed = "state_start_failed"
        case stateCrashed = "state_crashed"
        case quitAlsoStops = "quit_also_stops"
        case writeUnconfirmed = "write_unconfirmed"
        case settingsMoved = "settings_moved"
        case trayTurnOff = "tray_turn_off"
        case trayOpenToTurnOn = "tray_open_to_turn_on"
        case harnessesTitle = "harnesses_title"
        case harnessesWhat = "harnesses_what"
        case harnessesSpendScope = "harnesses_spend_scope"
        case harnessNotConnected = "harness_not_connected"
        case harnessConnectedNothingSeen = "harness_connected_nothing_seen"
        case harnessAnswering = "harness_answering"
        case harnessConnect = "harness_connect"
        case harnessDisconnect = "harness_disconnect"
        case harnessPreviewTitle = "harness_preview_title"
        case harnessPreviewConfirm = "harness_preview_confirm"
        case harnessPreviewCancel = "harness_preview_cancel"
        case harnessSlotTaken = "harness_slot_taken"
        case harnessNeedsRestart = "harness_needs_restart"
        case harnessesNoneFound = "harnesses_none_found"
        case harnessUnreadableConfig = "harness_unreadable_config"
        case harnessNotInstalled = "harness_not_installed"
        case harnessPlanNothingToChange = "harness_plan_nothing_to_change"
        case harnessPlanEntryUnusable = "harness_plan_entry_unusable"
        case harnessPlanNoConfigPath = "harness_plan_no_config_path"
        case credentialTitle = "credential_title"
        case credentialWhat = "credential_what"
        case credentialCost = "credential_cost"
        case credentialObtain = "credential_obtain"
        case credentialCancel = "credential_cancel"
        case credentialForget = "credential_forget"
        case credentialForgetExplains = "credential_forget_explains"
        case credentialAbsent = "credential_absent"
        case credentialObtaining = "credential_obtaining"
        case credentialFailed = "credential_failed"
        case credentialCancelled = "credential_cancelled"
        case credentialPresent = "credential_present"
        case credentialUnknown = "credential_unknown"
        case credentialUnreported = "credential_unreported"
        case harnessNeedsCredential = "harness_needs_credential"
        case eligibilityEligible = "eligibility_eligible"
        case eligibilityIneligiblePermanent = "eligibility_ineligible_permanent"
        case eligibilityIneligibleConfiguration = "eligibility_ineligible_configuration"
        case eligibilityUnknown = "eligibility_unknown"
        case eligibilityReasonNoCall = "eligibility_reason_no_call"
        case eligibilityReasonCaptureOff = "eligibility_reason_capture_off"
        case eligibilityReasonDigestAbsent = "eligibility_reason_digest_absent"
        case eligibilityReasonUpstreamIdAbsent = "eligibility_reason_upstream_id_absent"
        case eligibilityReasonDigestMismatch = "eligibility_reason_digest_mismatch"
        case eligibilityReasonReferenceMalformed = "eligibility_reason_reference_malformed"
        case eligibilityReasonBodiesUnreadable = "eligibility_reason_bodies_unreadable"
        case eligibilityReasonBodyNotUtf8 = "eligibility_reason_body_not_utf8"
        case eligibilityReasonBodyTooLarge = "eligibility_reason_body_too_large"
        case eligibilityReasonEvidenceCaptureOff = "eligibility_reason_evidence_capture_off"
        case eligibilityReasonMarkerAbsent = "eligibility_reason_marker_absent"
        case eligibilityReasonRequestMalformed = "eligibility_reason_request_malformed"
        case eligibilityReasonReceiptUnavailable = "eligibility_reason_receipt_unavailable"
        case attestationAttested = "attestation_attested"
        case attestationUnattestedPermanent = "attestation_unattested_permanent"
        case attestationUnattestedConfiguration = "attestation_unattested_configuration"
        case attestationUnknown = "attestation_unknown"
        case attestationReasonNoCall = "attestation_reason_no_call"
        case attestationReasonCaptureOff = "attestation_reason_capture_off"
        case attestationReasonDigestAbsent = "attestation_reason_digest_absent"
        case attestationReasonUpstreamIdAbsent = "attestation_reason_upstream_id_absent"
        case attestationReasonDigestMismatch = "attestation_reason_digest_mismatch"
        case attestationReasonReferenceMalformed = "attestation_reason_reference_malformed"
        case attestationReasonBodiesUnreadable = "attestation_reason_bodies_unreadable"
        case attestationReasonBodyNotUtf8 = "attestation_reason_body_not_utf8"
        case attestationReasonBodyTooLarge = "attestation_reason_body_too_large"
        case attestationReasonEvidenceCaptureOff = "attestation_reason_evidence_capture_off"
        case attestationReasonMarkerAbsent = "attestation_reason_marker_absent"
        case attestationReasonRequestMalformed = "attestation_reason_request_malformed"
        case attestationReasonReceiptUnavailable = "attestation_reason_receipt_unavailable"
        case balanceTitle = "balance_title"
        case balanceWhat = "balance_what"
        case balanceNoSession = "balance_no_session"
        case balanceSessionExpired = "balance_session_expired"
        case balanceNoOrganization = "balance_no_organization"
        case balanceUnavailable = "balance_unavailable"
        case balanceUnknown = "balance_unknown"
        case balanceUnreported = "balance_unreported"
        case balanceNoRemaining = "balance_no_remaining"
    }

    /// All or nothing, for the reason on the type.
    public static func decode(fromJSON json: String) -> PrivateInferenceCopy? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(PrivateInferenceCopy.self, from: data)
    }
}

/// How firmly a state reads.
///
/// Five values, and the ABI numbering they decode from is deliberately
/// disjoint from the routing surface's and the witness surface's. Do not
/// share a mapper with either: `RoutingTone.fromABI` would answer `.neutral`
/// for every value here, turning a refusal into "nothing to say".
public enum PrivateInferenceTone: Equatable, Sendable {
    case neutral
    case held
    case clear
    case attention
    case refused

    /// The arms are spelled out rather than derived from declaration order,
    /// and anything unknown is `.neutral`.
    ///
    /// Neutral is the safe direction on this surface because the dangerous
    /// value is `.clear`: a state a later daemon grows must not be drawn as
    /// a thing that is running.
    public static func fromABI(_ value: Int32) -> PrivateInferenceTone {
        switch value {
        case 21: return .held
        case 22: return .clear
        case 23: return .attention
        case 24: return .refused
        default: return .neutral
        }
    }

    /// Whether an indicator may paint this tone as working.
    ///
    /// `Clear` alone. A tab badge and a tray glyph both invite a green dot,
    /// and painting `refused` or `held` as "on" is the fail-open this
    /// surface exists to prevent. Shells must ask this, never the settings
    /// boolean: the switch says what was asked for, this says what is true.
    public var readsAsWorking: Bool { self == .clear }
}

/// `private_inference_state` as the daemon reports it.
///
/// The label is carried as the daemon's own string, never parsed into a
/// Swift enum: a state a later daemon grows would then have to be spelled
/// here before it could be shown, and the shared table already answers an
/// unknown label safely.
public struct PrivateInferenceState: Equatable, Sendable {
    public let label: String
    public let port: UInt16?

    public init(label: String, port: UInt16?) {
        self.label = label
        self.port = port
    }

    /// From `get_settings`/`status`'s `private_inference_state` object.
    ///
    /// A daemon that has never heard of the field sends nothing, and that
    /// reads as the empty label -- which the shared table answers as unreported,
    /// separately from an unfamiliar nonempty state. Never `nil`: a settings screen with no state at all
    /// would show the switch and nothing beneath it, which is the shape that
    /// says "on" over a listener that refused to start.
    public static func parse(_ object: [String: Any]?) -> PrivateInferenceState {
        let label = object?["state"] as? String ?? ""
        let port = (object?["port"] as? NSNumber).map { UInt16(truncatingIfNeeded: $0.intValue) }
        return PrivateInferenceState(label: label, port: port)
    }
}

/// The five calls this surface makes into the Rust, injected so
/// `TCShellCore` can be tested without linking the dylib.
///
/// Production wiring is `TCPrivateInference`; see `AppModel`.
public struct PrivateInferenceCalls: Sendable {
    public let stateLine: @Sendable (String) -> String?
    public let stateTone: @Sendable (String) -> Int32
    public let servingLine: @Sendable (UInt16?) -> String?
    public let shouldOffer: @Sendable (Bool, Bool) -> Bool
    public let quitNeedsNotice: @Sendable (Bool, String) -> Bool

    public init(
        stateLine: @escaping @Sendable (String) -> String?,
        stateTone: @escaping @Sendable (String) -> Int32,
        servingLine: @escaping @Sendable (UInt16?) -> String?,
        shouldOffer: @escaping @Sendable (Bool, Bool) -> Bool,
        quitNeedsNotice: @escaping @Sendable (Bool, String) -> Bool
    ) {
        self.stateLine = stateLine
        self.stateTone = stateTone
        self.servingLine = servingLine
        self.shouldOffer = shouldOffer
        self.quitNeedsNotice = quitNeedsNotice
    }
}

/// What this shell renders about answering model calls on this computer.
///
/// Holds no words. Every sentence comes from `PrivateInferenceCopy` or from
/// `calls`, and every fallback lands on a payload field rather than on a
/// literal.
public enum PrivateInferenceSurface {
    /// The `set_settings` key for the switch.
    public static let settingsKey = "private_inference"
    /// The `set_settings` key recording that the question was put.
    public static let offerSeenKey = "private_inference_offer_seen"

    /// The sentence under the switch. Falls back to the payload's unavailable
    /// sentence when the Rust caught a panic -- the sentence that claims
    /// nothing, never the one that says it is running.
    public static func stateLine(
        _ state: PrivateInferenceState,
        copy: PrivateInferenceCopy,
        calls: PrivateInferenceCalls
    ) -> String {
        calls.stateLine(state.label) ?? copy.stateUnknown
    }

    /// The tone that sentence is painted in.
    public static func tone(
        _ state: PrivateInferenceState,
        calls: PrivateInferenceCalls
    ) -> PrivateInferenceTone {
        PrivateInferenceTone.fromABI(calls.stateTone(state.label))
    }

    /// The reported local port, or nothing at all. An empty string is drawn as
    /// no line rather than as a blank one.
    public static func servingLine(
        _ state: PrivateInferenceState,
        calls: PrivateInferenceCalls
    ) -> String? {
        guard let line = calls.servingLine(state.port), !line.isEmpty else { return nil }
        return line
    }

    /// Whether to put the offer in front of the contributor. Asked of the
    /// shared table, never decided here.
    public static func shouldOffer(
        answered: Bool,
        on: Bool,
        calls: PrivateInferenceCalls
    ) -> Bool {
        calls.shouldOffer(answered, on)
    }

    /// The `set_settings` body for one answer to the offer.
    ///
    /// Declining writes the marker ALONE. It must never write the switch,
    /// not even as `false`: the switch is already false, and writing it
    /// would make a refusal indistinguishable from a change on every
    /// surface that watches settings.
    ///
    /// Accepting writes both in one call, so an accept cannot record the
    /// answer and fail to start, or start and fail to record.
    public static func offerParams(accepted: Bool) -> [String: Any] {
        var params: [String: Any] = [offerSeenKey: true]
        if accepted { params[settingsKey] = true }
        return params
    }

    /// The `set_settings` body for the switch on the settings card.
    ///
    /// Carries the marker too: a contributor who found the switch on their
    /// own has answered the question, and should not be asked it on the next
    /// launch.
    public static func settingsParams(on: Bool) -> [String: Any] {
        [settingsKey: on, offerSeenKey: true]
    }

    /// The extra sentence the quit confirmation carries while the switch is
    /// on.
    ///
    /// `nil` when it is off: a contributor who never turned it on should not
    /// be warned about losing it. The words are the payload's, never this
    /// shell's -- the rest of that dialog is Swift-authored, and this
    /// sentence deliberately is not.
    public static func quitDetail(on: Bool, state: PrivateInferenceState, copy: PrivateInferenceCopy?, calls: PrivateInferenceCalls) -> String? {
        guard calls.quitNeedsNotice(on, state.label), let copy else { return nil }
        return copy.quitAlsoStops
    }
}
