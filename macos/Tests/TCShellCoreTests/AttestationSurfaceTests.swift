import XCTest

@testable import TCShellCore

/// Captures the label a table was actually handed, so a test can assert the
/// ARGUMENT rather than the outcome.
private final class MarkRecorder: @unchecked Sendable {
    private(set) var marks: [String] = []

    func record(_ mark: String) { marks.append(mark) }
}

/// The attestation mark's logic, without the dylib.
///
/// The injected calls deliberately do NOT reimplement the Rust's three
/// tables: several of them answer something the real table never would, so a
/// surface that had quietly started deciding for itself could not agree with
/// them. That is the only way a test in this target can tell "asked the ABI"
/// from "reproduced the ABI in Swift".
///
/// The mark's presence rule is the OPPOSITE of eligibility's, and most of
/// this file is about that one difference. Eligibility asks whether this
/// contributor may send this session, and says nothing when nobody is
/// asking. The mark states whether the session carries proof of the model
/// call that produced it, which is a fact about the trace and is owed to
/// everyone -- invited contributors included.
final class AttestationSurfaceTests: XCTestCase {
    /// The 17 fields the mark reads, so a test can delete each in turn.
    private static let attestationFields = [
        "attestation_attested", "attestation_unattested_permanent",
        "attestation_unattested_configuration", "attestation_unknown",
        "attestation_reason_no_call", "attestation_reason_capture_off",
        "attestation_reason_digest_absent", "attestation_reason_upstream_id_absent",
        "attestation_reason_digest_mismatch", "attestation_reason_reference_malformed",
        "attestation_reason_bodies_unreadable", "attestation_reason_body_not_utf8",
        "attestation_reason_body_too_large", "attestation_reason_evidence_capture_off",
        "attestation_reason_marker_absent", "attestation_reason_request_malformed",
        "attestation_reason_receipt_unavailable",
    ]

    /// Decoded once, in `setUpWithError`, so a fixture that stops decoding
    /// fails THIS TEST and lets the rest of the bundle run.
    private var decodedCopy: PrivateInferenceCopy!

    override func setUpWithError() throws {
        try super.setUpWithError()
        decodedCopy = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: Self.payload),
            "the fixture payload must decode")
    }

    private func words() -> PrivateInferenceCopy { decodedCopy }

    private func calls(
        line: @escaping @Sendable (String) -> String? = { "LINE:\($0)" },
        tone: @escaping @Sendable (String) -> Int32 = { _ in 20 },
        reason: @escaping @Sendable (String) -> String? = { "REASON:\($0)" }
    ) -> AttestationCalls {
        AttestationCalls(markLine: line, markTone: tone, reasonLine: reason)
    }

    // MARK: - The payload is all or nothing

    /// Every one of the seventeen sentences is load-bearing and none of them
    /// has a default that could hide its absence.
    func testEveryAttestationSentenceIsRequiredRatherThanDefaulted() {
        XCTAssertNotNil(PrivateInferenceCopy.decode(fromJSON: Self.payload))
        for field in Self.attestationFields {
            let without = Self.payload.replacingOccurrences(
                of: "\"\(field)\":\"", with: "\"\(field)_REMOVED\":\"")
            XCTAssertNil(
                PrivateInferenceCopy.decode(fromJSON: without),
                "\(field) is not optional; a payload without it must be refused whole")
        }
    }

    // MARK: - The mark is always there

    /// The case the whole feature exists for.
    ///
    /// An invited contributor's entry carries no `eligibility` key at all,
    /// and it still carries a mark. A surface that had copied eligibility's
    /// absent-is-not-a-state rule across would show them nothing, which is
    /// exactly the hole this slice fills.
    func testAnInvitedContributorsEntryStillCarriesAMark() {
        let entry: [String: Any] = ["entry_id": "e1", "attestation": "attested"]
        XCTAssertNil(ContributionEligibility.parse(entry), "the fixture must have no eligibility")
        XCTAssertEqual(AttestationMark.parse(entry).mark, "attested")
        XCTAssertEqual(
            AttestationSurface.markLine(
                AttestationMark.parse(entry), copy: words(), calls: calls()),
            "LINE:attested")
    }

    /// A daemon that sent no `attestation` key at all still reaches a
    /// sentence -- the unknown one. There is no "draw nothing" arm on this
    /// surface, and that is the contract.
    func testAnAbsentKeyStillReachesTheUnknownSentence() {
        for object in [[:], ["entry_id": "e1"], ["attestation": NSNull()]] as [[String: Any]] {
            let mark = AttestationMark.parse(object)
            XCTAssertEqual(mark.mark, "", "an absent key is the empty label, never a real mark")
            XCTAssertNil(mark.reason)
            XCTAssertEqual(
                AttestationSurface.markLine(
                    mark, copy: words(), calls: calls(line: { _ in nil })),
                words().attestationUnknown)
        }
        XCTAssertEqual(AttestationMark.parse(nil).mark, "")
    }

    /// A caught Rust panic falls back to the unknown sentence, NEVER to an
    /// unattested one: a mark this build could not read is not evidence that
    /// a session lacks proof.
    func testAPanicFallsBackToUnknownAndNeverToAnUnattestedSentence() {
        let line = AttestationSurface.markLine(
            AttestationMark(mark: "attested"), copy: words(), calls: calls(line: { _ in nil }))
        XCTAssertEqual(line, words().attestationUnknown)
        XCTAssertNotEqual(line, words().attestationUnattestedPermanent)
        XCTAssertNotEqual(line, words().attestationUnattestedConfiguration)
    }

    // MARK: - Nothing here decides anything

    /// The sentence is the table's answer and is never composed here.
    ///
    /// The fake answers something the real table never would for `attested`,
    /// so a surface holding its own `switch` would disagree with it.
    func testTheSentenceComesFromTheTable() {
        XCTAssertEqual(
            AttestationSurface.markLine(
                AttestationMark(mark: "attested"), copy: words(),
                calls: calls(line: { _ in "FROM-THE-TABLE" })),
            "FROM-THE-TABLE")
    }

    /// The tone is the table's answer too, and is never recovered by reading
    /// the sentence or by matching on the mark.
    func testTheToneComesFromTheTable() {
        XCTAssertEqual(
            AttestationSurface.tone(
                AttestationMark(mark: "attested"), calls: calls(tone: { _ in 23 })),
            .attention,
            "the tone must be the table's, not this shell's reading of `attested`")
        XCTAssertEqual(
            AttestationSurface.tone(
                AttestationMark(mark: "unattested_configuration"), calls: calls(tone: { _ in 22 })),
            .clear)
        // An ABI value this build does not know claims nothing.
        XCTAssertEqual(
            AttestationSurface.tone(
                AttestationMark(mark: "attested"), calls: calls(tone: { _ in 99 })),
            .neutral)
    }

    /// Every table is asked with the daemon's own label, so a mark a later
    /// daemon grows never has to be spelled here before it can be shown.
    func testTheDaemonsOwnLabelIsWhatCrossesTheBoundary() {
        let recorder = MarkRecorder()
        let recording = AttestationCalls(
            markLine: { recorder.record($0); return "L" },
            markTone: { _ in 20 },
            reasonLine: { _ in "R" })
        _ = AttestationSurface.markLine(
            AttestationMark(mark: "a_mark_from_a_later_daemon"), copy: words(), calls: recording)
        XCTAssertEqual(recorder.marks, ["a_mark_from_a_later_daemon"])
    }

    // MARK: - The reason follows the KEY, not the mark

    /// The rule this slice is most likely to get wrong.
    ///
    /// `attested` never carries a reason; both unattested marks always do;
    /// `unknown` carries one only SOMETIMES -- absent when the row was never
    /// evaluated, present when a send was retracted because the receipt
    /// service was unreachable. Suppressing it on `unknown` drops the only
    /// signal telling a contributor it may work later, so the branch is on
    /// the key's presence and never on the mark.
    func testTheReasonIsDrawnWheneverTheKeyIsPresentIncludingOnUnknown() {
        XCTAssertEqual(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: "receipt_unavailable"), calls: calls()),
            "REASON:receipt_unavailable")
        XCTAssertEqual(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unattested_permanent", reason: "no_inference_call"),
                calls: calls()),
            "REASON:no_inference_call")
        XCTAssertEqual(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unattested_configuration", reason: "capture_off"),
                calls: calls()),
            "REASON:capture_off")
    }

    /// The same `unknown` mark with no reason key draws no reason line. Both
    /// halves of this test carry the same mark, which is what makes it a
    /// test of the key rather than of the mark.
    func testTheSameUnknownMarkWithNoReasonKeyDrawsNoReasonLine() {
        XCTAssertNil(
            AttestationSurface.reasonLine(AttestationMark(mark: "unknown"), calls: calls()))
        XCTAssertNil(
            AttestationSurface.reasonLine(AttestationMark(mark: "attested"), calls: calls()))
        XCTAssertEqual(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: "receipt_unavailable"), calls: calls()),
            "REASON:receipt_unavailable")
    }

    /// An empty reason, a reason the table has nothing to say about, and a
    /// JSON null all draw nothing -- an unknown reason has nothing honest to
    /// add to a sentence that has already said what is true.
    func testAnEmptyOrUnrecognisedReasonDrawsNothing() {
        XCTAssertNil(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: ""), calls: calls()))
        XCTAssertNil(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: "later_reason"),
                calls: calls(reason: { _ in "" })))
        XCTAssertNil(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: "later_reason"),
                calls: calls(reason: { _ in nil })))
        XCTAssertNil(
            AttestationMark.parse(
                ["attestation": "unknown", "attestation_reason": NSNull()]).reason)
        XCTAssertNil(
            AttestationMark.parse(["attestation": "unknown", "attestation_reason": ""]).reason)
    }

    /// The wire's two keys, read off one entry object.
    func testBothKeysAreReadFromTheEntryObject() {
        let mark = AttestationMark.parse([
            "attestation": "unattested_configuration",
            "attestation_reason": "capture_off",
        ])
        XCTAssertEqual(mark.mark, "unattested_configuration")
        XCTAssertEqual(mark.reason, "capture_off")
    }

    /// A present non-string is the daemon saying nothing this build can use,
    /// and reaches the unknown sentence rather than a crash or a real mark.
    func testAPresentNonStringIsNotAMark() {
        XCTAssertEqual(AttestationMark.parse(["attestation": 7]).mark, "")
        XCTAssertNil(
            AttestationMark.parse(["attestation": "unknown", "attestation_reason": 7]).reason)
    }

    // MARK: - The fixture

    /// The shared complete payload; see `PrivateInferenceCopyFixture`.
    /// A new required property is one edit there, not five here.
    private static let payload = PrivateInferenceCopyFixture.complete
}
