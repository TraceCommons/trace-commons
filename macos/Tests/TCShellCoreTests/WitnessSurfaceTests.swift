import XCTest

@testable import TCShellCore

/// The witness surface's mapping, tested against sentinels rather than
/// against the shipped words.
///
/// Every sentence this file compares is a sentinel like `L-ABSENT`, for the
/// reason `RoutingSurfaceTests` gives: a test that spelled the real sentence
/// would pass whether the surface read the ABI or a literal of its own,
/// which is the drift the shared source exists to prevent. What is asserted
/// here is *which answer* each state reaches for, and what happens when the
/// answer is one this build cannot name.
///
/// `TCBridgeTests` asserts the same properties against the real dylib.
final class WitnessSurfaceTests: XCTestCase {

    // MARK: - Fakes

    /// Calls that echo their argument, so a mapping reaching for the wrong
    /// state is visible in the result.
    ///
    /// Deliberately NOT a Swift copy of the Rust branch table: each returns
    /// a sentinel derived from what it was asked, and the two `unnameable`
    /// arms below are the only place a specific answer is pinned, because
    /// they are the contract this surface exists to hold.
    private func calls(
        nameableStates: Set<Int32> = [0, 1, 2, 3, 4, -1, -2],
        stateTones: [Int32: Int32] = [
            0: 10, 1: 12, 2: 14, 3: 14, 4: 14, -1: 10, -2: 14,
        ],
        lastResultLine: String? = "L-LAST",
        lastResultTone: Int32 = 12
    ) -> WitnessCalls {
        WitnessCalls(
            stateLine: { code in
                // The ABI returns NULL for a code it cannot name, and the
                // shell must then render no sentence of its own.
                nameableStates.contains(code) ? "L-STATE-\(code)" : nil
            },
            // The ABI fails closed to REFUSED on a code it cannot name.
            stateTone: { code in stateTones[code] ?? 14 },
            lastResultLine: { lastResultLine },
            lastResultTone: { lastResultTone }
        )
    }

    // MARK: - Tones

    func testEveryDefinedToneDecodes() {
        XCTAssertEqual(WitnessTone.fromABI(10), .neutral)
        XCTAssertEqual(WitnessTone.fromABI(11), .held)
        XCTAssertEqual(WitnessTone.fromABI(12), .clear)
        XCTAssertEqual(WitnessTone.fromABI(13), .attention)
        XCTAssertEqual(WitnessTone.fromABI(14), .refused)
    }

    /// A tone this build has no words for is REFUSED, never neutral: on a
    /// surface about whether sessions leave the machine, the safe reading of
    /// "I do not know" is "they are not".
    func testUnknownToneIsRefusedAndNeverNeutral() {
        for value: Int32 in [15, 99, 0, 1, 2, 3, 4, -1, -7] {
            XCTAssertEqual(
                WitnessTone.fromABI(value), .refused,
                "tone \(value) must fail closed to refused")
            XCTAssertNotEqual(WitnessTone.fromABI(value), .neutral)
        }
    }

    /// The routing tone numbering must not be readable as a witness tone.
    /// The two ranges are disjoint on purpose; a cross-wired mapper has to
    /// be wrong for every value, not only for the dangerous one.
    func testRoutingToneNumbersAreNotWitnessTones() {
        for value: Int32 in [0, 1, 2, 3] {
            XCTAssertEqual(WitnessTone.fromABI(value), .refused)
        }
    }

    // MARK: - States

    func testEveryDefinedStateDecodes() {
        XCTAssertEqual(WitnessTrustState.fromABI(0), .absent)
        XCTAssertEqual(WitnessTrustState.fromABI(1), .pinned)
        XCTAssertEqual(WitnessTrustState.fromABI(2), .refusingUnpinned)
        XCTAssertEqual(WitnessTrustState.fromABI(3), .refusingPinMalformed)
        XCTAssertEqual(WitnessTrustState.fromABI(4), .refusingInferenceReceiptsMissing)
        XCTAssertEqual(WitnessTrustState.fromABI(-1), .notEnrolled)
        XCTAssertEqual(WitnessTrustState.fromABI(-2), .unreadable)
    }

    /// A state this build cannot name is never `absent`. Defaulting an
    /// unknown state to "no witness, all is well" turns a future refusal
    /// into silence.
    func testUnknownStateIsNeverAbsent() {
        for code: Int32 in [5, 6, 99, -3, -99] {
            let state = WitnessTrustState.fromABI(code)
            XCTAssertEqual(state, .unnameable(code))
            XCTAssertNotEqual(state, .absent)
            XCTAssertTrue(WitnessSurface.isRefusal(state), "\(code) must read as a refusal")
        }
    }

    /// `absent` is a supported mode, not a refusal, and not a warning.
    func testAbsentIsNotARefusal() {
        XCTAssertFalse(WitnessSurface.isRefusal(.absent))
        XCTAssertFalse(WitnessSurface.isRefusal(.pinned))
        // Nothing about a witness is being declined; the device has no
        // account yet.
        XCTAssertFalse(WitnessSurface.isRefusal(.notEnrolled))
    }

    func testEveryRefusingStateIsARefusal() {
        for state: WitnessTrustState in [
            .refusingUnpinned, .refusingPinMalformed,
            .refusingInferenceReceiptsMissing, .unreadable,
        ] {
            XCTAssertTrue(WitnessSurface.isRefusal(state), "\(state) must read as a refusal")
        }
    }

    // MARK: - The rule this surface exists for

    /// `ABSENT` and `REFUSING_UNPINNED` must not render alike. One means
    /// redaction is happening here; the other means nothing is being sent
    /// at all.
    func testAbsentAndUnpinnedCannotRenderIdentically() {
        let calls = self.calls()
        let absentLine = WitnessSurface.stateLine(0, calls: calls)
        let unpinnedLine = WitnessSurface.stateLine(2, calls: calls)
        XCTAssertNotNil(absentLine)
        XCTAssertNotNil(unpinnedLine)
        XCTAssertNotEqual(absentLine, unpinnedLine)

        let absentTone = WitnessSurface.tone(forState: 0, calls: calls)
        let unpinnedTone = WitnessSurface.tone(forState: 2, calls: calls)
        XCTAssertEqual(absentTone, .neutral)
        XCTAssertEqual(unpinnedTone, .refused)
        XCTAssertNotEqual(absentTone, unpinnedTone)

        // And the refusal is not merely "caution": attention is reserved for
        // a degraded-but-working setup, which this is not.
        XCTAssertNotEqual(unpinnedTone, .attention)
    }

    /// A shell handed a state it cannot name renders NO sentence rather than
    /// one of its own, and paints the surface refused.
    func testUnnameableStateRendersNoSentenceAndReadsRefused() {
        let calls = self.calls()
        XCTAssertNil(WitnessSurface.stateLine(77, calls: calls))
        XCTAssertEqual(WitnessSurface.tone(forState: 77, calls: calls), .refused)
    }

    /// The tone comes from the state, never from the sentence. A state whose
    /// sentence is missing still has a tone.
    func testToneIsTakenFromTheStateNotTheSentence() {
        let noSentences = calls(nameableStates: [])
        XCTAssertNil(WitnessSurface.stateLine(1, calls: noSentences))
        XCTAssertEqual(WitnessSurface.tone(forState: 1, calls: noSentences), .clear)
    }

    // MARK: - The way out of a refusal

    /// Every refusal offers the clear action. A refusal with no affordance
    /// is the trap `AppModel.Startup.needsRoots` exists to avoid.
    func testEveryRefusalOffersAWayOut() {
        for state: WitnessTrustState in [
            .refusingUnpinned, .refusingPinMalformed,
            .refusingInferenceReceiptsMissing, .unreadable, .unnameable(42),
        ] {
            XCTAssertTrue(
                WitnessSurface.offersClear(state),
                "\(state) is a refusal and must offer a way out")
        }
    }

    /// Clearing is offered where there is something to clear, and nowhere
    /// else: there is nothing to stop using when no witness is configured,
    /// and no config at all before enrollment.
    func testClearIsNotOfferedWhereThereIsNothingToClear() {
        XCTAssertFalse(WitnessSurface.offersClear(.absent))
        XCTAssertFalse(WitnessSurface.offersClear(.notEnrolled))
        XCTAssertTrue(WitnessSurface.offersClear(.pinned))
    }

    /// The configure fields are the other way out of the unpinned refusal:
    /// pinning a measurement is what that state asks for.
    func testConfigureIsOfferedOnEveryStateThatHasAConfigToWrite() {
        for state: WitnessTrustState in [
            .absent, .pinned, .refusingUnpinned, .refusingPinMalformed,
            .refusingInferenceReceiptsMissing, .unreadable, .unnameable(42),
        ] {
            XCTAssertTrue(WitnessSurface.offersConfigure(state), "\(state)")
        }
        // Nothing can be written before there is a config to hold it.
        XCTAssertFalse(WitnessSurface.offersConfigure(.notEnrolled))
    }

    // MARK: - The form

    func testFormWillNotOfferToWriteAnUnpinnedWitness() {
        var form = WitnessForm(
            url: "https://witness.example", signingAddress: "0xabc", measurements: "")
        XCTAssertFalse(form.canConfigure, "an empty pin list is a total upload outage")
        form.measurements = "   \n\t\n  "
        XCTAssertFalse(form.canConfigure, "blank lines are not measurements")
        form.measurements = "mrtd=aa,mrconfigid=bb"
        XCTAssertTrue(form.canConfigure)
    }

    func testFormRequiresAnAddressAndASigningKey() {
        let pinned = "mrtd=aa,mrconfigid=bb"
        XCTAssertFalse(
            WitnessForm(url: "", signingAddress: "0xabc", measurements: pinned).canConfigure)
        XCTAssertFalse(
            WitnessForm(url: "https://w.example", signingAddress: " ", measurements: pinned)
                .canConfigure)
    }

    /// The measurement list is encoded, never concatenated: a pasted value
    /// can carry a quote or a backslash.
    func testMeasurementsEncodeAsJSONRatherThanConcatenating() throws {
        let form = WitnessForm(
            url: "https://w.example",
            signingAddress: "0xabc",
            measurements: "mrtd=aa,mrconfigid=bb\n  \nmr\"td=cc\n"
        )
        let json = try XCTUnwrap(form.measurementsJSON)
        let decoded = try JSONDecoder().decode(
            [String].self, from: XCTUnwrap(json.data(using: .utf8)))
        XCTAssertEqual(decoded, ["mrtd=aa,mrconfigid=bb", "mr\"td=cc"])
    }

    func testMeasurementsJSONIsNilWhenNothingIsPinned() {
        XCTAssertNil(
            WitnessForm(url: "https://w.example", signingAddress: "0xabc", measurements: "\n \n")
                .measurementsJSON)
    }

    /// The form is seeded from what came back, and from nothing else. A
    /// status this shell could not read seeds an empty form rather than
    /// carrying the previous witness's address forward.
    func testFormSeedsFromTheStatusAndFromNothingElse() {
        let form = WitnessForm.fromStatus(
            Self.status(pinned: ["mrtd=aa,mrconfigid=bb", "mrtd=cc"]))
        XCTAssertEqual(form.url, "https://w.example")
        XCTAssertEqual(form.signingAddress, "0xabc")
        // Pre-filled, one entry per line. A write-only box means retyping
        // every pin to change a URL, and an empty box that meant "keep what
        // is there" would save a pin nobody looked at.
        XCTAssertEqual(form.measurements, "mrtd=aa,mrconfigid=bb\nmrtd=cc")

        let empty = WitnessForm.fromStatus(nil)
        XCTAssertEqual(empty.url, "")
        XCTAssertEqual(empty.signingAddress, "")
        XCTAssertEqual(empty.measurements, "")
    }

    /// A witness with nothing pinned pre-fills an EMPTY box, not a
    /// placeholder line. The box is the contributor's answer, and a line
    /// this shell put there would be a pin nobody typed.
    func testAnUnpinnedWitnessSeedsAnEmptyBox() {
        let form = WitnessForm.fromStatus(Self.status(pinned: [], stateCode: 2))
        XCTAssertEqual(form.measurements, "")
        XCTAssertFalse(form.canConfigure)
    }

    /// The entries go back out exactly as they came in.
    ///
    /// `pinned_measurements` is what `tc_witness_configure` takes, so the
    /// round trip through this editor must be the identity. A shell that
    /// reformats a pin is a shell that can reformat it wrongly, and the
    /// contributor would never see which one changed.
    func testPinnedMeasurementsRoundTripUnchanged() throws {
        let stored = [
            "mrtd=aabb,mrconfigid=ccdd",
            // A malformed entry comes back as it is stored so the typo can
            // be seen and repaired. It must survive the trip unrepaired:
            // this shell is not the thing that decides what a measurement is.
            "mrtd=nothexatall",
            // Whitespace inside an entry is part of the entry. Trimming it
            // here would be this shell rewriting a pin nobody touched.
            "  mrtd=ee,mrconfigid=ff  ",
            "mrtd=\"quoted\",mrconfigid=back\\slash",
        ]
        let form = WitnessForm.fromStatus(Self.status(pinned: stored))
        let json = try XCTUnwrap(form.measurementsJSON)
        let decoded = try JSONDecoder().decode(
            [String].self, from: XCTUnwrap(json.data(using: .utf8)))
        XCTAssertEqual(decoded, stored, "an entry was rewritten on the way back out")
    }

    /// Order is part of the value: the ABI returns the entries in stored
    /// order and takes them back the same way.
    func testPinnedMeasurementsKeepTheirOrder() {
        let stored = ["mrtd=01", "mrtd=02", "mrtd=03"]
        let form = WitnessForm.fromStatus(Self.status(pinned: stored))
        XCTAssertEqual(form.measurementLines, stored)
    }

    /// The blank lines an editor makes from pressing return are not
    /// entries, and are the one thing dropped on the way out.
    func testBlankEditorLinesAreNotEntries() {
        let form = WitnessForm(
            url: "https://w.example", signingAddress: "0xabc",
            measurements: "\nmrtd=aa\n\n\nmrtd=bb\n")
        XCTAssertEqual(form.measurementLines, ["mrtd=aa", "mrtd=bb"])
    }

    // MARK: - Decoding what the ABI answers

    /// A status as the ABI answers one, with every field distinguishable.
    private static func status(
        pinned: [String], stateCode: Int32 = 1
    ) -> WitnessStatus {
        WitnessStatus(
            stateCode: stateCode,
            refusal: nil,
            url: "https://w.example",
            signingAddress: "0xabc",
            pinnedMeasurementCount: pinned.count,
            pinnedMeasurementLine: "L-COUNT",
            pinnedMeasurements: pinned
        )
    }

    /// The count and the list are one answer. A card that showed a count
    /// from one and a box from the other would disagree with itself.
    func testTheCountIsAlwaysTheLengthOfTheList() throws {
        let json = """
            {"state":"pinned","state_code":1,"refusal":null,
             "url":"https://w.example","signing_address":"0xabc",
             "pinned_measurement_count":2,
             "pinned_measurement_line":"2 measurements are pinned.",
             "pinned_measurements":["mrtd=aa","mrtd=bb"]}
            """
        let status = try XCTUnwrap(WitnessStatus.decode(fromJSON: json))
        XCTAssertEqual(status.pinnedMeasurementCount, status.pinnedMeasurements.count)
    }

    /// The count sentence is null where there is no witness to count for --
    /// a count of the pins on a witness that does not exist is not a
    /// shorter sentence, it is a wrong one.
    func testANullCountLineDecodesAsNilAndNotAsAnEmptyString() throws {
        let json = """
            {"state":"absent","state_code":0,"refusal":null,
             "url":null,"signing_address":null,
             "pinned_measurement_count":0,
             "pinned_measurement_line":null,
             "pinned_measurements":[]}
            """
        let status = try XCTUnwrap(WitnessStatus.decode(fromJSON: json))
        XCTAssertNil(status.pinnedMeasurementLine)
        XCTAssertEqual(status.pinnedMeasurements, [])
        // And nothing is invented to stand in for it.
        XCTAssertNotEqual(status.pinnedMeasurementLine, "")
    }

    func testStatusDecodesTheAbiPayload() throws {
        let json = """
            {"state":"refusing_unpinned","state_code":2,
             "refusal":"witness_expected_measurement",
             "url":"https://witness.example","signing_address":"0xabc",
             "pinned_measurement_count":0,
             "pinned_measurement_line":"No measurement is pinned.",
             "pinned_measurements":[]}
            """
        let status = try XCTUnwrap(WitnessStatus.decode(fromJSON: json))
        XCTAssertEqual(status.stateCode, 2)
        XCTAssertEqual(status.refusal, "witness_expected_measurement")
        XCTAssertEqual(status.url, "https://witness.example")
        XCTAssertEqual(status.signingAddress, "0xabc")
        XCTAssertEqual(status.pinnedMeasurementCount, 0)
        XCTAssertEqual(status.pinnedMeasurementLine, "No measurement is pinned.")
        XCTAssertEqual(status.pinnedMeasurements, [])
    }

    /// The state is read from `state_code`, never re-derived from `url`
    /// being non-null: that is the boolean this surface refuses to hand a
    /// shell, spelled differently.
    func testStatusWithAUrlIsNotAutomaticallyInUse() throws {
        let json = """
            {"state":"refusing_unpinned","state_code":2,"refusal":"x",
             "url":"https://witness.example","signing_address":"0xabc",
             "pinned_measurement_count":0,
             "pinned_measurement_line":"No measurement is pinned.",
             "pinned_measurements":[]}
            """
        let status = try XCTUnwrap(WitnessStatus.decode(fromJSON: json))
        XCTAssertNotNil(status.url)
        XCTAssertEqual(WitnessTrustState.fromABI(status.stateCode), .refusingUnpinned)
        XCTAssertNotEqual(WitnessTrustState.fromABI(status.stateCode), .pinned)
    }

    func testStatusDecodeReturnsNilRatherThanAHalfFilledValue() {
        XCTAssertNil(WitnessStatus.decode(fromJSON: "not json"))
        XCTAssertNil(WitnessStatus.decode(fromJSON: "{\"url\":\"https://w.example\"}"))
    }

    func testCopyDecodesEveryFieldAndNoneHasADefault() throws {
        let json = """
            {"heading":"C-HEADING","intro":"C-INTRO",
             "certificate_means":"C-MEANS","measurements_note":"C-NOTE",
             "url_title":"C-URL","signing_address_title":"C-SIGNING",
             "measurements_title":"C-MEASUREMENTS","configure":"C-CONFIGURE",
             "clear":"C-CLEAR","clear_note":"C-CLEARNOTE",
             "applies_at_once":"C-APPLIES"}
            """
        let copy = try XCTUnwrap(WitnessCopy.decode(fromJSON: json))
        XCTAssertEqual(copy.heading, "C-HEADING")
        XCTAssertEqual(copy.intro, "C-INTRO")
        XCTAssertEqual(copy.certificateMeans, "C-MEANS")
        XCTAssertEqual(copy.measurementsNote, "C-NOTE")
        XCTAssertEqual(copy.urlTitle, "C-URL")
        XCTAssertEqual(copy.signingAddressTitle, "C-SIGNING")
        XCTAssertEqual(copy.measurementsTitle, "C-MEASUREMENTS")
        XCTAssertEqual(copy.configure, "C-CONFIGURE")
        XCTAssertEqual(copy.clear, "C-CLEAR")
        XCTAssertEqual(copy.clearNote, "C-CLEARNOTE")
        XCTAssertEqual(copy.appliesAtOnce, "C-APPLIES")

        // A payload missing one string decodes to nothing at all. A card
        // rendering "" where a sentence belongs is worse than a card that
        // renders nothing, and one rendering a Swift-authored word is worse
        // than both.
        let short = """
            {"heading":"C-HEADING","intro":"C-INTRO"}
            """
        XCTAssertNil(WitnessCopy.decode(fromJSON: short))
    }

    // MARK: - The last submission

    func testLastResultLineAndToneComeFromTheAbi() {
        let calls = self.calls(lastResultLine: "L-LAST", lastResultTone: 14)
        XCTAssertEqual(WitnessSurface.lastResultLine(calls: calls), "L-LAST")
        XCTAssertEqual(WitnessSurface.lastResultTone(calls: calls), .refused)
    }

    /// A caught panic on the sentence is no sentence, not an invented one.
    func testLastResultLineIsNilWhenTheAbiProducesNone() {
        let calls = self.calls(lastResultLine: nil)
        XCTAssertNil(WitnessSurface.lastResultLine(calls: calls))
    }

    func testLastResultToneFailsClosedOnAnUnknownValue() {
        let calls = self.calls(lastResultTone: 99)
        XCTAssertEqual(WitnessSurface.lastResultTone(calls: calls), .refused)
    }
}
