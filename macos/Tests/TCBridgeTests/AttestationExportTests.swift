import XCTest

@testable import TCBridge
@testable import TCShellCore

/// The attestation mark as it really crosses the C ABI.
///
/// Links the dylib. What this can assert and `TCShellCoreTests` cannot is
/// that the three tables answer HERE what they answer there -- so the
/// mutation proofs in that target, which only show the surface asks a table,
/// are joined to the real table's answers.
///
/// It also proves the two surfaces are not one surface. The mark's reason
/// labels are the eligibility reason's thirteen, and its SENTENCES are
/// different: a shell that reached for `tc_contribution_eligibility_*` here
/// would compile, render, and quietly tell a contributor their session had
/// been refused.
final class AttestationExportTests: XCTestCase {
    private func copy() -> PrivateInferenceCopy? {
        guard let json = TCPrivateInference.copyJSON() else {
            XCTFail("the export returned nothing")
            return nil
        }
        guard let copy = PrivateInferenceCopy.decode(fromJSON: json) else {
            XCTFail("the payload did not decode into this shell's struct")
            return nil
        }
        return copy
    }

    /// The production wiring, verbatim -- the same three closures `AppModel`
    /// builds `attestationCalls` from.
    private func calls() -> AttestationCalls {
        AttestationCalls(
            markLine: { TCAttestation.markLine(mark: $0) },
            markTone: { TCAttestation.markTone(mark: $0) },
            reasonLine: { TCAttestation.reasonLine(reason: $0) }
        )
    }

    private func at(_ mark: String) -> AttestationMark { AttestationMark(mark: mark) }

    /// Every mark the daemon can send reaches its own sentence, and one this
    /// build cannot read reaches the unknown one.
    func testEachMarkRendersTheSentenceTheRustExports() throws {
        let copy = try XCTUnwrap(copy())
        let line = { (label: String) -> String in
            AttestationSurface.markLine(self.at(label), copy: copy, calls: self.calls())
        }
        XCTAssertEqual(line("attested"), copy.attestationAttested)
        XCTAssertEqual(line("unattested_permanent"), copy.attestationUnattestedPermanent)
        XCTAssertEqual(line("unattested_configuration"), copy.attestationUnattestedConfiguration)
        XCTAssertEqual(line("unknown"), copy.attestationUnknown)
        // Neither of these may degrade to an unattested mark, which is a
        // claim about a contributor's own session.
        XCTAssertEqual(line("a_mark_from_a_later_daemon"), copy.attestationUnknown)
        XCTAssertEqual(line(""), copy.attestationUnknown)
        XCTAssertNotEqual(copy.attestationUnknown, copy.attestationUnattestedPermanent)
        XCTAssertNotEqual(copy.attestationUnknown, copy.attestationUnattestedConfiguration)
        XCTAssertNotEqual(copy.attestationUnknown, copy.attestationAttested)
    }

    /// The four tones, across the boundary.
    ///
    /// One mark is settled and one asks for attention; the other two claim
    /// nothing. `.refused` is the arm worth naming: nothing was refused
    /// here, and painting a contributor's ordinary older work as a failure
    /// is a judgement this surface has no business making.
    func testTheToneBesideEachMarkCrossesTheAbi() {
        let tone = { (label: String) -> PrivateInferenceTone in
            AttestationSurface.tone(self.at(label), calls: self.calls())
        }
        XCTAssertEqual(tone("attested"), .clear)
        XCTAssertEqual(tone("unattested_permanent"), .neutral)
        XCTAssertEqual(tone("unattested_configuration"), .attention)
        XCTAssertEqual(tone("unknown"), .neutral)
        XCTAssertEqual(tone("a_mark_from_a_later_daemon"), .neutral)
        XCTAssertEqual(tone(""), .neutral)
        for label in [
            "attested", "unattested_permanent", "unattested_configuration", "unknown",
            "a_mark_from_a_later_daemon", "",
        ] {
            XCTAssertNotEqual(tone(label), .refused, label)
            XCTAssertNotEqual(tone(label), .held, label)
        }
    }

    /// Each of the thirteen reason labels reaches its own sentence, and no
    /// two share one.
    func testEachReasonLabelReachesItsOwnSentence() throws {
        let copy = try XCTUnwrap(copy())
        let expected: [(String, String)] = [
            ("no_inference_call", copy.attestationReasonNoCall),
            ("capture_off", copy.attestationReasonCaptureOff),
            ("digest_absent", copy.attestationReasonDigestAbsent),
            ("upstream_id_absent", copy.attestationReasonUpstreamIdAbsent),
            ("digest_mismatch", copy.attestationReasonDigestMismatch),
            ("reference_malformed", copy.attestationReasonReferenceMalformed),
            ("bodies_unreadable", copy.attestationReasonBodiesUnreadable),
            ("body_not_utf8", copy.attestationReasonBodyNotUtf8),
            ("body_too_large", copy.attestationReasonBodyTooLarge),
            ("evidence_capture_off", copy.attestationReasonEvidenceCaptureOff),
            ("marker_absent", copy.attestationReasonMarkerAbsent),
            ("request_malformed", copy.attestationReasonRequestMalformed),
            ("receipt_unavailable", copy.attestationReasonReceiptUnavailable),
            ("receipt_not_issued", copy.attestationReasonReceiptNotIssued),
        ]
        XCTAssertEqual(expected.count, 14)
        for (label, sentence) in expected {
            XCTAssertFalse(sentence.isEmpty, label)
            XCTAssertEqual(
                AttestationSurface.reasonLine(
                    AttestationMark(mark: "unattested_permanent", reason: label),
                    calls: calls()),
                sentence, label)
        }
        XCTAssertEqual(Set(expected.map(\.1)).count, 14, "two reasons share a sentence")
    }

    /// A reason this build has never heard of, and the empty string, both
    /// draw nothing rather than a guess.
    func testAnUnfamiliarReasonDrawsNothingAcrossTheRealAbi() {
        XCTAssertNil(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: "a_reason_from_a_later_daemon"),
                calls: calls()))
        XCTAssertNil(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: ""), calls: calls()))
    }

    /// `receipt_unavailable` on an `unknown` mark is a real row, and its
    /// sentence is what tells a contributor the answer may change. It has to
    /// survive the whole path.
    func testARetractedSendKeepsItsReasonOnAnUnknownMark() throws {
        let copy = try XCTUnwrap(copy())
        XCTAssertEqual(
            AttestationSurface.markLine(
                AttestationMark(mark: "unknown", reason: "receipt_unavailable"),
                copy: copy, calls: calls()),
            copy.attestationUnknown)
        XCTAssertEqual(
            AttestationSurface.reasonLine(
                AttestationMark(mark: "unknown", reason: "receipt_unavailable"),
                calls: calls()),
            copy.attestationReasonReceiptUnavailable)
    }

    /// The mark's words are its own, never the eligibility surface's.
    ///
    /// The two vocabularies partition the same facts, and the reason labels
    /// really are the same thirteen strings. The four MARK sentences are
    /// disjoint from the four eligibility state sentences outright.
    ///
    /// The reasons are NOT, and asserting that they were is what this test
    /// used to do wrongly. Eight of the thirteen reason sentences are
    /// deliberately word-for-word identical across the two surfaces, because
    /// those reasons state a plain fact about the recording -- the copy could
    /// not be read back, the request was malformed -- and that fact does not
    /// change with the question being asked. The five that diverge are
    /// exactly the ones whose sentence names a CONSEQUENCE, which is the
    /// half that does change: eligibility says a session cannot be sent,
    /// the mark says nothing can be checked against it.
    ///
    /// So the wrong accessor is caught on those five and only those five,
    /// and they are asserted individually. A blanket "all thirteen differ"
    /// cannot be made to pass without flattening eight sentences that are
    /// right as they are.
    func testTheMarkNeverBorrowsTheEligibilitySentences() throws {
        let copy = try XCTUnwrap(copy())
        let marks = [
            copy.attestationAttested, copy.attestationUnattestedPermanent,
            copy.attestationUnattestedConfiguration, copy.attestationUnknown,
        ]
        let states = [
            copy.eligibilityEligible, copy.eligibilityIneligiblePermanent,
            copy.eligibilityIneligibleConfiguration, copy.eligibilityUnknown,
        ]
        for sentence in marks {
            XCTAssertFalse(sentence.isEmpty)
            XCTAssertFalse(states.contains(sentence), "a mark is wearing an eligibility sentence")
        }
        // The five whose sentences name a consequence, so the two surfaces
        // must not share them. Each is asserted through the LABEL, through
        // the accessor production actually calls: reading the fields alone
        // would compare the payload with itself and pass even if the bridge
        // called `tc_contribution_eligibility_reason_line` for every row.
        let divergent: [(String, String, String)] = [
            // The label is `no_inference_call`; the field it reaches is
            // `...ReasonNoCall`. The two are spelled differently on
            // purpose and only the label crosses the ABI.
            (
                "no_inference_call", copy.attestationReasonNoCall,
                copy.eligibilityReasonNoCall
            ),
            (
                "digest_mismatch", copy.attestationReasonDigestMismatch,
                copy.eligibilityReasonDigestMismatch
            ),
            (
                "body_too_large", copy.attestationReasonBodyTooLarge,
                copy.eligibilityReasonBodyTooLarge
            ),
            (
                "evidence_capture_off", copy.attestationReasonEvidenceCaptureOff,
                copy.eligibilityReasonEvidenceCaptureOff
            ),
            (
                "marker_absent", copy.attestationReasonMarkerAbsent,
                copy.eligibilityReasonMarkerAbsent
            ),
            (
                "receipt_not_issued", copy.attestationReasonReceiptNotIssued,
                copy.eligibilityReasonReceiptNotIssued
            ),
        ]
        for (label, mine, theirs) in divergent {
            XCTAssertNotEqual(
                mine, theirs,
                "\(label) is meant to read differently on the two surfaces")
            let rendered = TCAttestation.reasonLine(reason: label)
            XCTAssertEqual(
                rendered, mine,
                "the attestation table did not answer \(label) in its own words")
            XCTAssertNotEqual(
                rendered, theirs,
                "\(label) came back wearing the eligibility reason's sentence")
        }

        // The other eight are identical ON PURPOSE and are pinned here so
        // that a later edit splitting one of them has to come back and say
        // which list it belongs in.
        //
        // Eight, and the arithmetic is the point: 6 + 8 = 14, and a count
        // that does not close is a parse that missed a constant rather than
        // a fact about the copy. `body_not_utf8` is the one most easily
        // dropped -- its label carries a digit, and its sentence puts the
        // literal on the line after the `=`.
        let identical = [
            "capture_off", "digest_absent", "upstream_id_absent", "reference_malformed",
            "bodies_unreadable", "body_not_utf8", "request_malformed", "receipt_unavailable",
        ]
        XCTAssertEqual(
            divergent.count + identical.count, 14,
            "the two lists must account for all fourteen reason labels")
        for label in identical {
            let rendered = TCAttestation.reasonLine(reason: label)
            XCTAssertNotNil(rendered)
            XCTAssertFalse(rendered?.isEmpty ?? true, "\(label) reached no sentence")
        }
    }

    /// The tables tolerate what a queue can really hand them.
    func testTheTablesAreSafeOnEmptyAndUnfamiliarInput() {
        XCTAssertNotNil(TCAttestation.markLine(mark: ""))
        XCTAssertNotNil(TCAttestation.markLine(mark: "🙂 not a label"))
        XCTAssertNotNil(TCAttestation.reasonLine(reason: ""))
        XCTAssertNotNil(TCAttestation.reasonLine(reason: "🙂 not a label"))
        XCTAssertEqual(TCAttestation.markTone(mark: "🙂 not a label"), 20)
    }
}
