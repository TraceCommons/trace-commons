import TCBridge
import TCShellCore
import XCTest

/// The witness surface, driven by the real dylib.
///
/// `WitnessSurfaceTests` proves the mapping against sentinels. This proves
/// the ABI actually answers what that mapping assumes: that every defined
/// state has its own sentence, that `ABSENT` and `REFUSING_UNPINNED` cannot
/// come back alike, and that a state this build cannot name produces no
/// sentence and a refused tone.
///
/// It also stands behind the claim that this shell prints the shared
/// wording: change a sentence in `trace_commons_contributor::witness_copy`
/// and the words here change with it, because none of them is written in
/// Swift.
final class WitnessExportTests: XCTestCase {
    /// The calls as the app wires them: straight through the ABI. Nothing
    /// here is a Swift branch table -- that is the point of this file.
    private let calls = WitnessCalls(
        stateLine: { TCWitness.stateLine(state: $0) },
        stateTone: { TCWitness.stateTone(state: $0) },
        lastResultLine: { TCWitness.lastResultLine() },
        lastResultTone: { TCWitness.lastResultTone() }
    )

    private static let definedStates: [WitnessTrustState] = [
        .absent, .pinned, .refusingUnpinned, .refusingPinMalformed,
        .refusingInferenceReceiptsMissing, .notEnrolled, .unreadable,
    ]

    // MARK: - The words

    func testTheCopyExportDecodes() throws {
        let json = try XCTUnwrap(TCWitness.copyJSON(), "the witness copy export returned nil")
        let copy = try XCTUnwrap(
            WitnessCopy.decode(fromJSON: json), "the payload did not decode: \(json)")
        for word in [
            copy.heading, copy.intro, copy.certificateMeans, copy.measurementsNote,
            copy.urlTitle, copy.signingAddressTitle, copy.measurementsTitle,
            copy.configure, copy.clear, copy.clearNote, copy.appliesAtOnce,
        ] {
            XCTAssertFalse(word.isEmpty)
        }
    }

    /// A certificate covers redaction mechanics and a residual-risk verdict.
    /// No word on this surface may claim a session is genuine or attested.
    func testNoWordOnTheSurfaceClaimsATraceIsClean() throws {
        let json = try XCTUnwrap(TCWitness.copyJSON())
        let copy = try XCTUnwrap(WitnessCopy.decode(fromJSON: json))
        var everySentence = [
            copy.heading, copy.intro, copy.certificateMeans, copy.measurementsNote,
            copy.urlTitle, copy.signingAddressTitle, copy.measurementsTitle,
            copy.configure, copy.clear, copy.clearNote, copy.appliesAtOnce,
        ]
        for state in Self.definedStates {
            everySentence.append(try XCTUnwrap(TCWitness.stateLine(state: state.abiCode)))
        }
        for sentence in everySentence {
            let lowered = sentence.lowercased()
            XCTAssertFalse(lowered.contains("attested"), "\(sentence)")
            XCTAssertFalse(lowered.contains("genuine"), "\(sentence)")
        }
    }

    // MARK: - The states

    func testEveryDefinedStateHasItsOwnSentence() throws {
        var seen: [String] = []
        for state in Self.definedStates {
            let line = try XCTUnwrap(
                WitnessSurface.stateLine(state.abiCode, calls: calls),
                "\(state) has no sentence")
            XCTAssertFalse(line.isEmpty)
            XCTAssertFalse(seen.contains(line), "\(state) reuses another state's sentence")
            seen.append(line)
        }
    }

    /// The rule this whole surface exists for, asserted against the shipped
    /// words: no witness is local redaction, an unpinned witness is a total
    /// upload outage, and the two must not read alike.
    func testAbsentAndUnpinnedDoNotRenderAlike() throws {
        let absent = try XCTUnwrap(WitnessSurface.stateLine(0, calls: calls))
        let unpinned = try XCTUnwrap(WitnessSurface.stateLine(2, calls: calls))
        XCTAssertNotEqual(absent, unpinned)
        XCTAssertEqual(WitnessSurface.tone(forState: 0, calls: calls), .neutral)
        XCTAssertEqual(WitnessSurface.tone(forState: 2, calls: calls), .refused)
    }

    /// The reserved receipts refusal already has a branch, so the
    /// attested-inference work can start returning it without this shell
    /// changing. Its instruction is not "pin a measurement", which is why it
    /// is not the unpinned sentence.
    func testTheReservedReceiptsStateAlreadyHasASentence() throws {
        let line = try XCTUnwrap(WitnessSurface.stateLine(4, calls: calls))
        XCTAssertNotEqual(line, try XCTUnwrap(WitnessSurface.stateLine(2, calls: calls)))
        XCTAssertEqual(WitnessSurface.tone(forState: 4, calls: calls), .refused)
    }

    /// A state this build cannot name renders no sentence at all, and the
    /// surface reads refused rather than neutral or absent.
    func testAnUnnameableStateHasNoSentenceAndAFailClosedTone() {
        for code: Int32 in [5, 6, 99, -3, -99] {
            XCTAssertNil(
                WitnessSurface.stateLine(code, calls: calls),
                "state \(code) must produce no sentence")
            XCTAssertEqual(WitnessSurface.tone(forState: code, calls: calls), .refused)
            XCTAssertNotEqual(WitnessTrustState.fromABI(code), .absent)
        }
    }

    /// The witness tone numbers are disjoint from the routing tone numbers,
    /// so a cross-wired mapper is wrong for every value rather than only for
    /// the dangerous one.
    func testWitnessTonesDoNotOverlapRoutingTones() {
        for state in Self.definedStates {
            let tone = TCWitness.stateTone(state: state.abiCode)
            XCTAssertGreaterThanOrEqual(tone, 10, "\(state) answered a routing-range tone")
            XCTAssertLessThanOrEqual(tone, 14, "\(state)")
        }
    }

    // MARK: - The last submission

    /// This process has submitted nothing, so the surface says so rather
    /// than guessing -- and says it in a tone that is not reassuring.
    func testAFreshProcessReportsNoSubmission() throws {
        let line = try XCTUnwrap(WitnessSurface.lastResultLine(calls: calls))
        XCTAssertFalse(line.isEmpty)
        XCTAssertEqual(WitnessSurface.lastResultTone(calls: calls), .held)

        let json = try XCTUnwrap(TCWitness.lastResultJSON())
        XCTAssertTrue(json.contains("not_observed"), json)
        // Every key is present in every outcome, so a shell never has to
        // decide what an absent key meant.
        for key in [
            "outcome", "certificate_obtained", "certificate_verified", "refusal", "n_of_m",
        ] {
            XCTAssertTrue(json.contains(key), "\(key) missing from \(json)")
        }
    }

    // MARK: - Reading and writing a config

    /// An unenrolled directory is `NOT_ENROLLED`, which is neither "absent"
    /// nor a refusal, and the status call reports a fixed label rather than
    /// a nil that could be mistaken for "no witness".
    func testAnUnenrolledDirectoryIsNotEnrolledAndNotAbsent() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }

        let code = TCWitness.trustState(configDir: dir)
        XCTAssertEqual(WitnessTrustState.fromABI(code), .notEnrolled)
        XCTAssertNotEqual(WitnessTrustState.fromABI(code), .absent)
        XCTAssertFalse(WitnessSurface.isRefusal(.notEnrolled))

        switch TCWitness.statusJSON(configDir: dir) {
        case .status(let json):
            XCTFail("an unenrolled directory must not report a status: \(json)")
        case .refused(let label):
            XCTAssertEqual(label, "witness-not-enrolled")
        }
    }

    /// The ABI will not write an unpinned witness, and this shell does not
    /// offer the button that would ask it to.
    func testConfiguringWithoutAPinIsRefused() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }

        let outcome = TCWitness.configure(
            configDir: dir,
            url: "https://witness.example",
            signingAddress: "0x0000000000000000000000000000000000000000",
            measurementsJSON: "[]"
        )
        guard case .refused(let label) = outcome else {
            return XCTFail("an empty pin list must be refused, got \(outcome)")
        }
        // Not enrolled is checked before the pins are, so either fixed label
        // is a refusal -- what must never happen is a write.
        XCTAssertTrue(
            ["witness-pin-required", "witness-not-enrolled"].contains(label), label)
    }

    /// Clearing a witness that is not there is not an error.
    func testClearingIsIdempotent() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        // Unenrolled refuses with a fixed label rather than pretending to
        // have cleared something; either way it is never a crash and never
        // an invented sentence.
        switch TCWitness.clear(configDir: dir) {
        case .done(let changed):
            XCTAssertFalse(changed)
        case .refused(let label):
            XCTAssertEqual(label, "witness-not-enrolled")
        }
    }

    private func makeTempDir() throws -> String {
        let dir = NSTemporaryDirectory() + "tcw-\(UUID().uuidString.prefix(8))"
        try FileManager.default.createDirectory(
            atPath: dir, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        return dir
    }
}
