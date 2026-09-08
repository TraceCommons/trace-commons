import XCTest

@testable import TCShellCore

/// The surface's logic, without the dylib.
///
/// The fixture's sentences are sentinels rather than the real words, and the
/// injected calls deliberately do NOT reimplement the Rust's branch tables:
/// a fake that reproduced the real mapping would let this suite pass while
/// the shell had stopped asking the shared table at all.
final class PrivateInferenceSurfaceTests: XCTestCase {
    private let payload = """
        {"destination":"DESTINATION","subtitle":"SUBTITLE",
         "offer_title":"T","offer_what":"WHAT","offer_exposure":"EXPOSURE",
         "offer_no_repoint":"NO-REPOINT","offer_accept":"ACCEPT",
         "offer_decline":"DECLINE","offer_asked_once":"ONCE",
         "settings_title":"S-TITLE","settings_toggle":"S-TOGGLE",
         "settings_applies_at_once":"S-AT-ONCE","state_off":"S-OFF","state_unknown":"S-UNKNOWN","state_unreported":"S-UNREPORTED","state_stopping":"S-STOPPING",
         "state_running":"S-RUNNING","state_running_no_backends":"S-NO-BACKENDS","state_running_answered_elsewhere":"S-ELSEWHERE","state_running_destination_unknown":"S-DEST-UNKNOWN",
         "state_running_elsewhere":"S-ELSEWHERE","state_port_in_use":"S-PORT",
         "state_start_failed":"S-FAILED","state_crashed":"S-CRASHED",
         "quit_also_stops":"QUIT","write_unconfirmed":"UNCONFIRMED","settings_moved":"MOVED","tray_turn_off":"TRAYOFF","tray_open_to_turn_on":"TRAYON",
         "harnesses_title":"H-TITLE","harnesses_what":"H-WHAT",
         "harnesses_spend_scope":"H-SPEND-SCOPE",
         "harness_not_connected":"H-NOT-CONNECTED",
         "harness_connected_nothing_seen":"H-NOTHING-SEEN",
         "harness_answering":"H-ANSWERING","harness_connect":"H-CONNECT",
         "harness_disconnect":"H-DISCONNECT",
         "harness_preview_title":"H-PREVIEW","harness_preview_confirm":"H-CONFIRM",
         "harness_preview_cancel":"H-CANCEL","harness_slot_taken":"H-TAKEN",
         "harness_needs_restart":"H-RESTART","harnesses_none_found":"H-NONE",
         "harness_unreadable_config":"H-UNREADABLE",
         "harness_not_installed":"H-NOT-INSTALLED",
         "harness_plan_nothing_to_change":"H-NOTHING-TO-CHANGE",
         "harness_plan_entry_unusable":"H-ENTRY-UNUSABLE",
         "harness_plan_no_config_path":"H-NO-CONFIG-PATH",
         "credential_title":"C-TITLE","credential_what":"C-WHAT",
         "credential_cost":"C-COST","credential_obtain":"C-OBTAIN",
         "credential_cancel":"C-CANCEL","credential_forget":"C-FORGET",
         "credential_forget_explains":"C-FORGET-EXPLAINS",
         "credential_absent":"C-ABSENT","credential_obtaining":"C-OBTAINING",
         "credential_failed":"C-FAILED","credential_cancelled":"C-CANCELLED",
         "credential_present":"C-PRESENT","credential_unknown":"C-UNKNOWN",
         "credential_unreported":"C-UNREPORTED",
         "harness_needs_credential":"H-NEEDS-CREDENTIAL",
         "eligibility_eligible":"E-ELIGIBLE",
         "eligibility_ineligible_permanent":"E-PERMANENT",
         "eligibility_ineligible_configuration":"E-CONFIGURATION",
         "eligibility_unknown":"E-UNKNOWN",
         "eligibility_reason_no_call":"R-NO-CALL",
         "eligibility_reason_capture_off":"R-CAPTURE-OFF",
         "eligibility_reason_digest_absent":"R-DIGEST-ABSENT",
         "eligibility_reason_upstream_id_absent":"R-UPSTREAM-ID-ABSENT",
         "eligibility_reason_digest_mismatch":"R-DIGEST-MISMATCH",
         "eligibility_reason_reference_malformed":"R-REFERENCE-MALFORMED",
         "eligibility_reason_bodies_unreadable":"R-BODIES-UNREADABLE",
         "eligibility_reason_body_not_utf8":"R-BODY-NOT-UTF8",
         "eligibility_reason_body_too_large":"R-BODY-TOO-LARGE",
         "eligibility_reason_evidence_capture_off":"R-EVIDENCE-CAPTURE-OFF",
         "eligibility_reason_marker_absent":"R-MARKER-ABSENT",
         "eligibility_reason_request_malformed":"R-REQUEST-MALFORMED",
         "eligibility_reason_receipt_unavailable":"R-RECEIPT-UNAVAILABLE",
         "attestation_attested":"A-ATTESTED",
         "attestation_unattested_permanent":"A-UNATTESTED-PERMANENT",
         "attestation_unattested_configuration":"A-UNATTESTED-CONFIGURATION",
         "attestation_unknown":"A-UNKNOWN",
         "attestation_reason_no_call":"AR-NO-CALL",
         "attestation_reason_capture_off":"AR-CAPTURE-OFF",
         "attestation_reason_digest_absent":"AR-DIGEST-ABSENT",
         "attestation_reason_upstream_id_absent":"AR-UPSTREAM-ID-ABSENT",
         "attestation_reason_digest_mismatch":"AR-DIGEST-MISMATCH",
         "attestation_reason_reference_malformed":"AR-REFERENCE-MALFORMED",
         "attestation_reason_bodies_unreadable":"AR-BODIES-UNREADABLE",
         "attestation_reason_body_not_utf8":"AR-BODY-NOT-UTF8",
         "attestation_reason_body_too_large":"AR-BODY-TOO-LARGE",
         "attestation_reason_evidence_capture_off":"AR-EVIDENCE-CAPTURE-OFF",
         "attestation_reason_marker_absent":"AR-MARKER-ABSENT",
         "attestation_reason_request_malformed":"AR-REQUEST-MALFORMED",
         "attestation_reason_receipt_unavailable":"AR-RECEIPT-UNAVAILABLE"}
        """

    /// Decoded once, in `setUpWithError`, so a fixture that stops decoding
    /// fails THIS TEST and lets the rest of the bundle run.
    ///
    /// It used to be `XCTFail` followed by `fatalError("unreachable")`, which
    /// aborted the whole XCTest process at the first fixture failure: every
    /// remaining test in every target simply never ran, no total was printed,
    /// and the log carried one error line no matter how much was broken. A
    /// red `macOS app tests` job was unreadable for that reason -- it looked
    /// far emptier than a normal failing run, and its failure count was
    /// always about one.
    ///
    /// A throw from `setUpWithError` is reported against each test in this
    /// class in turn, which is honest: every one of them needs this payload.
    private var decodedCopy: PrivateInferenceCopy!

    override func setUpWithError() throws {
        try super.setUpWithError()
        decodedCopy = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: payload),
            "the fixture payload must decode")
    }

    private func copy() -> PrivateInferenceCopy { decodedCopy }

    private func calls(
        line: @escaping @Sendable (String) -> String? = { "LINE:\($0)" },
        tone: @escaping @Sendable (String) -> Int32 = { _ in 24 },
        serving: @escaping @Sendable (UInt16?) -> String? = { $0.map { "PORT:\($0)" } ?? "" },
        offer: @escaping @Sendable (Bool, Bool) -> Bool = { !$0 && !$1 }
    ) -> PrivateInferenceCalls {
        PrivateInferenceCalls(
            stateLine: line, stateTone: tone, servingLine: serving, shouldOffer: offer, quitNeedsNotice: { on, _ in on })
    }

    func testAPayloadMissingASentenceIsRefusedRatherThanRenderedBlank() {
        XCTAssertNil(
            PrivateInferenceCopy.decode(
                fromJSON: payload.replacingOccurrences(
                    of: "\"offer_exposure\":\"EXPOSURE\",", with: "")),
            "the sentence saying what the switch exposes is not optional")
    }

    /// The amount is worthless without the sentence saying what it covers,
    /// so a payload without that sentence is refused rather than rendered as
    /// a bare figure.
    func testAPayloadMissingTheAmountsScopeIsRefused() {
        XCTAssertNil(
            PrivateInferenceCopy.decode(
                fromJSON: payload.replacingOccurrences(
                    of: "\"harnesses_spend_scope\":\"H-SPEND-SCOPE\",", with: "")),
            "an amount without its scope is a lie of omission")
    }

    func testTheSentenceComesFromTheSharedTableAndNotFromThisShell() {
        let state = PrivateInferenceState(label: "running_no_backends", port: 8463)
        XCTAssertEqual(
            PrivateInferenceSurface.stateLine(state, copy: copy(), calls: calls()),
            "LINE:running_no_backends")
    }

    /// A caught panic on the far side falls back to the sentence that claims
    /// nothing -- never to one that says it is running.
    func testASilentExportFallsBackToTheUnavailableSentence() {
        let state = PrivateInferenceState(label: "running", port: 8463)
        XCTAssertEqual(
            PrivateInferenceSurface.stateLine(
                state, copy: copy(), calls: calls(line: { _ in nil })),
            copy().stateUnknown)
    }

    /// The ABI decoder is spelled out, and anything unknown is neutral --
    /// the value that claims nothing, never the working light.
    func testAnUnknownToneIsNeutralAndNeverClear() {
        XCTAssertEqual(PrivateInferenceTone.fromABI(21), .held)
        XCTAssertEqual(PrivateInferenceTone.fromABI(22), .clear)
        XCTAssertEqual(PrivateInferenceTone.fromABI(23), .attention)
        XCTAssertEqual(PrivateInferenceTone.fromABI(24), .refused)
        for stranger: Int32 in [0, 1, 2, 3, 10, 14, 20, 25, -1, 99] {
            XCTAssertEqual(
                PrivateInferenceTone.fromABI(stranger), .neutral,
                "\(stranger) must claim nothing")
        }
    }

    /// The routing surface's numbering must not decode here. Feeding a
    /// private-inference tone to `RoutingTone` is the cross-wiring the
    /// disjoint ranges exist to make wrong for every value.
    func testTheRoutingDecoderCannotStandInForThisOne() {
        for value: Int32 in [21, 22, 23, 24] {
            XCTAssertEqual(
                RoutingTone.fromABI(value), .neutral,
                "the routing decoder must not give \(value) a meaning")
        }
    }

    func testTheServingLineIsNothingWhenThereIsNoPort() {
        let none = PrivateInferenceState(label: "port_in_use", port: nil)
        XCTAssertNil(PrivateInferenceSurface.servingLine(none, calls: calls()))
        let some = PrivateInferenceState(label: "running", port: 8463)
        XCTAssertEqual(PrivateInferenceSurface.servingLine(some, calls: calls()), "PORT:8463")
    }

    /// Declining writes the marker ALONE. Writing the switch as `false`
    /// would make a refusal indistinguishable from a change.
    func testDecliningRecordsTheAnswerAndTouchesNoSwitch() {
        let declined = PrivateInferenceSurface.offerParams(accepted: false)
        XCTAssertEqual(declined.count, 1)
        XCTAssertEqual(declined[PrivateInferenceSurface.offerSeenKey] as? Bool, true)
        XCTAssertNil(declined[PrivateInferenceSurface.settingsKey])

        let accepted = PrivateInferenceSurface.offerParams(accepted: true)
        XCTAssertEqual(accepted[PrivateInferenceSurface.settingsKey] as? Bool, true)
        XCTAssertEqual(accepted[PrivateInferenceSurface.offerSeenKey] as? Bool, true)
    }

    /// A contributor who found the switch themselves has answered the
    /// question, and must not be asked it on the next launch.
    func testTheSettingsSwitchAlsoRecordsThatTheQuestionIsAnswered() {
        for on in [true, false] {
            let params = PrivateInferenceSurface.settingsParams(on: on)
            XCTAssertEqual(params[PrivateInferenceSurface.settingsKey] as? Bool, on)
            XCTAssertEqual(params[PrivateInferenceSurface.offerSeenKey] as? Bool, true)
        }
    }

    /// Whether to ask is the shared table's answer, asked through the
    /// injected call rather than decided here.
    func testWhetherToOfferIsAskedAndNotDecided() {
        // The fake answers the opposite of the real table for every input,
        // so a surface that decided for itself could not agree with it.
        let inverted = calls(offer: { answered, on in answered || on })
        XCTAssertFalse(
            PrivateInferenceSurface.shouldOffer(answered: false, on: false, calls: inverted))
        XCTAssertTrue(
            PrivateInferenceSurface.shouldOffer(answered: true, on: true, calls: inverted))
    }

    /// A daemon that never heard of the field reads as the empty label,
    /// which the shared table answers as unavailable -- never as a
    /// missing state a screen would draw as nothing at all.
    func testAnAbsentStateObjectReadsAsTheEmptyLabel() {
        XCTAssertEqual(PrivateInferenceState.parse(nil).label, "")
        XCTAssertNil(PrivateInferenceState.parse(nil).port)
        let parsed = PrivateInferenceState.parse(["state": "running", "port": NSNumber(value: 8463)])
        XCTAssertEqual(parsed.label, "running")
        XCTAssertEqual(parsed.port, 8463)
    }

    /// The quit sentence is added only while the switch is on, and it is the
    /// payload's words rather than this shell's.
    func testTheQuitSentenceIsOnlyAddedWhenTheSwitchIsOn() {
        XCTAssertNil(PrivateInferenceSurface.quitDetail(on: false, state: .init(label: "", port: nil), copy: copy(), calls: calls()))
        XCTAssertNil(PrivateInferenceSurface.quitDetail(on: true, state: .init(label: "", port: nil), copy: nil, calls: calls()))
        XCTAssertEqual(PrivateInferenceSurface.quitDetail(on: true, state: .init(label: "", port: nil), copy: copy(), calls: calls()), "QUIT")
    }
}
