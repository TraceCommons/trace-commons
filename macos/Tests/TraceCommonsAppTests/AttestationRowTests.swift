import Foundation
import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// The attestation mark on a queue row: what the decoder keeps, and what the
/// card actually draws.
///
/// The rendering half is a SOURCE SCAN rather than a press-and-inspect test.
/// SwiftUI's accessibility tree is not built in-process -- an
/// `NSHostingView`'s `accessibilityChildren()` answers nothing regardless of
/// what the view holds -- so a test that inspected it would be green for the
/// wrong reason. Every scan below asserts its own preconditions, so a rename
/// fails it rather than quietly emptying it.
final class AttestationRowTests: XCTestCase {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(type, from: Data(json.utf8))
    }

    private func entry(_ extra: String) throws -> QueueEntry {
        try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0\(extra)}
        """)
    }

    // MARK: - The decoder

    /// The case the feature exists for: an invited contributor's entry
    /// carries no `eligibility` key and still carries a mark.
    func testAnInvitedContributorsEntryCarriesAMarkAndNoEligibility() throws {
        let entry = try entry(",\"attestation\":\"attested\"")
        XCTAssertNil(entry.eligibility)
        XCTAssertNil(entry.contributionEligibility)
        XCTAssertEqual(entry.attestation, "attested")
        XCTAssertEqual(entry.attestationMark.mark, "attested")
        XCTAssertNil(entry.attestationMark.reason)
    }

    /// All four marks survive the decoder as the daemon's own label. Nothing
    /// in this shell parses them into cases.
    func testEveryMarkSurvivesTheDecoderAsItsOwnLabel() throws {
        for mark in ["attested", "unattested_permanent", "unattested_configuration", "unknown"] {
            XCTAssertEqual(try entry(",\"attestation\":\"\(mark)\"").attestationMark.mark, mark)
        }
        // And one this build has never heard of is carried, not dropped --
        // the shared table is what degrades it to the unknown sentence.
        XCTAssertEqual(
            try entry(",\"attestation\":\"a_mark_from_a_later_daemon\"").attestationMark.mark,
            "a_mark_from_a_later_daemon")
    }

    /// An `unknown` mark whose send was retracted keeps its reason, and the
    /// same mark from a row that was never evaluated has none. Both halves
    /// carry the same mark: the reason follows the KEY.
    func testTheReasonFollowsTheKeyAndNotTheMark() throws {
        let retracted = try entry(
            ",\"attestation\":\"unknown\",\"attestation_reason\":\"receipt_unavailable\"")
        XCTAssertEqual(retracted.attestationMark.mark, "unknown")
        XCTAssertEqual(retracted.attestationMark.reason, "receipt_unavailable")

        let neverEvaluated = try entry(",\"attestation\":\"unknown\"")
        XCTAssertEqual(neverEvaluated.attestationMark.mark, "unknown")
        XCTAssertNil(neverEvaluated.attestationMark.reason)
    }

    /// A daemon predating the fields sends neither, and the row still has a
    /// mark to draw -- the empty label, which the shared table answers with
    /// the unknown sentence.
    func testAnOlderDaemonsEntryStillHasAMarkToDraw() throws {
        let entry = try entry("")
        XCTAssertNil(entry.attestation)
        XCTAssertNil(entry.attestationReason)
        XCTAssertEqual(entry.attestationMark.mark, "")
        XCTAssertNil(entry.attestationMark.reason)
    }

    // MARK: - What the card draws

    /// The row draws the mark, its tone and its reason, and it gets all
    /// three from `AttestationSurface`.
    func testTheQueueRowDrawsTheMarkThroughTheSharedSurface() throws {
        let source = try Self.source("TraceCommonsApp/Views/QueueView.swift")
        XCTAssertTrue(
            source.contains("struct QueueRow"),
            "QueueView no longer holds QueueRow; this scan has to be repointed")
        for call in [
            "AttestationSurface.markLine(", "AttestationSurface.tone(",
            "AttestationSurface.reasonLine(",
        ] {
            XCTAssertTrue(source.contains(call), "the row never calls \(call)")
        }
        XCTAssertTrue(
            source.contains("entry.attestationMark"),
            "the row must read the mark off the entry it is drawing")
        XCTAssertTrue(
            source.contains("attestationCalls"),
            "the row must be handed the attestation tables")
    }

    /// The mark is drawn OUTSIDE the eligibility line's `if let`.
    ///
    /// This is the presence rule made structural: a mark nested under
    /// `if let eligibilityLine` would be invisible to exactly the
    /// contributors the feature is for. The scan checks the reason line is
    /// gated on its own optional and the mark line is not gated on
    /// eligibility at all.
    func testTheMarkIsNotNestedUnderTheEligibilityLine() throws {
        let source = try Self.source("TraceCommonsApp/Views/QueueView.swift")
        let lines = source.components(separatedBy: "\n")
        let markIndex = try XCTUnwrap(
            lines.firstIndex { $0.contains("if let attestationLine") },
            "the mark's own line is not drawn from an `if let attestationLine`")
        let eligibilityIndex = try XCTUnwrap(
            lines.firstIndex { $0.contains("if let eligibilityLine") },
            "the eligibility precedent has moved; repoint this scan")
        let markIndent = lines[markIndex].prefix { $0 == " " }.count
        let eligibilityIndent = lines[eligibilityIndex].prefix { $0 == " " }.count
        XCTAssertEqual(
            markIndent, eligibilityIndent,
            "the mark and the eligibility line must be siblings, not nested one in the other")
        XCTAssertTrue(
            source.contains("if let attestationReasonLine"),
            "the reason must be drawn only when the key was present")
    }

    /// The mark offers nothing to press.
    ///
    /// There is no `tc_contribution_attestation_control` and that is the
    /// contract: the mark describes the trace, and sendability stays the
    /// eligibility control's question. A button drawn off the mark would be
    /// inventing an action out of a description.
    func testNoControlIsDerivedFromTheMark() throws {
        let sources = try Self.shellSources()
        XCTAssertGreaterThanOrEqual(
            sources.count, 60,
            "only \(sources.count) sources were scanned; the whole shell is expected")
        var offenders: [String] = []
        for (path, text) in sources.sorted(by: { $0.key < $1.key }) {
            for forbidden in [
                "attestation_control", "attestationControl", "attestationOffersContribute",
            ] where text.contains(forbidden) {
                offenders.append("\(path) derives a control from the mark (\(forbidden))")
            }
        }
        XCTAssertTrue(offenders.isEmpty, offenders.joined(separator: "\n"))
        // The precondition: the surface this forbids a control on is really
        // present in what was scanned.
        XCTAssertTrue(
            sources.keys.contains("TCShellCore/AttestationSurface.swift"),
            "the attestation surface was not among the scanned sources")
    }

    /// The bridge wires the attestation accessors, not the eligibility ones.
    ///
    /// The two reason tables take the SAME thirteen labels and answer
    /// different sentences, so a mis-wired closure compiles, renders, and
    /// tells a contributor their session was refused.
    func testTheProductionWiringAsksTheAttestationTables() throws {
        let model = try Self.source("TraceCommonsApp/AppModel.swift")
        XCTAssertTrue(model.contains("let attestationCalls = AttestationCalls("))
        for call in [
            "TCAttestation.markLine(", "TCAttestation.markTone(", "TCAttestation.reasonLine(",
        ] {
            XCTAssertTrue(model.contains(call), "the model never wires \(call)")
        }
        let bridge = try Self.source("TCBridge/TCAttestation.swift")
        for symbol in [
            "tc_contribution_attestation_line", "tc_contribution_attestation_tone",
            "tc_contribution_attestation_reason_line",
        ] {
            XCTAssertTrue(bridge.contains(symbol), "the bridge never calls \(symbol)")
        }
        XCTAssertFalse(
            bridge.contains("tc_contribution_eligibility"),
            "the attestation bridge must never reach for an eligibility accessor")
    }

    // MARK: - Reading the shell's own sources

    private static func source(_ relative: String) throws -> String {
        let sources = try shellSources()
        return try XCTUnwrap(sources[relative], "\(relative) was not found under macos/Sources")
    }

    /// `.../macos/Tests/TraceCommonsAppTests/<this file>` ->
    /// `.../macos/Sources`, located from this file's own path the way
    /// `ShellWordingTests` does.
    private static func shellSources() throws -> [String: String] {
        let base = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TraceCommonsAppTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources")
        let walker = try XCTUnwrap(
            FileManager.default.enumerator(at: base, includingPropertiesForKeys: nil))
        var scanned: [String: String] = [:]
        for case let url as URL in walker where url.pathExtension == "swift" {
            let relative = url.path.replacingOccurrences(of: base.path + "/", with: "")
            scanned[relative] = try String(contentsOf: url, encoding: .utf8)
        }
        return scanned
    }
}
