import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// How the consent surface is wired to the rule underneath it.
///
/// `ReadGate.canContribute` is the rule and `ConsentCopy` is the decode.
/// Neither can see the layer between them and a contributor: which fact the
/// surface actually feeds the rule. A surface that armed `Contribute` on "a
/// preview arrived" rather than "a preview arrived and carries an
/// enrollment" would pass both suites and ship exactly the divergence this
/// slice removed -- macOS approving against an envelope that pinned nothing,
/// under a shared sentence that says it did not.
///
/// The other half is the copy. Arming requires the sentences as well as the
/// pin, because the gate statement is the whole of what a contributor is
/// told before an irreversible send, and a build that cannot read it must
/// not take an approval against it.
///
/// Where it lives (#1241): Look inside is read-only, so the one Contribute
/// is the inspector's session card. The card arms it through
/// `TracesStore.contributeArmed`, and its tooltip is chosen on the card. The
/// card's own file moves (Task 6 of the #1146 port gives it one), so the
/// tooltip is looked for across `Views/Monitor/` rather than in one file.
/// The gate statement Look inside still prints stays pinned on the sheet.
/// `TracesTreeTests.test_contributeArmsOnlyOnAnEnrolledPreviewWithConsent`
/// holds the same rule behaviourally.
///
/// A SwiftUI `body` holding `@State` cannot be built, rendered or reflected
/// outside a running window, so these assert against the views' own source
/// -- the same limitation, and the same justification, as
/// `RoutingBindingTests` and `WitnessBindingTests`. Every locator reports
/// what it was looking in when it fails, so a refactor that moves this
/// surface produces a failure to fix rather than a test that quietly stops
/// asserting.
private enum ConsentSurfaceSource {
    static let sources = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TraceCommonsAppTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources/TraceCommonsApp")

    /// `.../Views/PreviewSheet.swift`: Look inside, which still prints the
    /// gate statement.
    static let sheetPath = sources.appendingPathComponent("Views/PreviewSheet.swift")
    /// `.../Views/Monitor/TracesStore.swift`: the card's arming rule.
    static let storePath = sources.appendingPathComponent("Views/Monitor/TracesStore.swift")
    static let monitorDirectory = sources.appendingPathComponent("Views/Monitor")

    /// The arming rule the session card asks, at draw time and again at the
    /// press.
    static func contributeArmedBody(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration(
            """
                static func contributeArmed(
                    enrolled: Bool?, consent: ConsentCopy?, eligibility: ContributionEligibility?, calls: EligibilityCalls
                ) -> Bool {
            """, in: storePath, file: file, line: line)
    }

    static func gateStatementBody(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("private var gateStatement: some View {", in: sheetPath, file: file, line: line)
    }

    /// The Monitor sources, concatenated, comment lines stripped.
    static func monitorCode(file: StaticString = #filePath, line: UInt = #line) -> String? {
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: monitorDirectory.path) else {
            XCTFail("could not list \(monitorDirectory.path)", file: file, line: line)
            return nil
        }
        return names.filter { $0.hasSuffix(".swift") }.sorted().compactMap { name in
            try? String(contentsOf: monitorDirectory.appendingPathComponent(name), encoding: .utf8)
        }
        .joined(separator: "\n")
        .split(separator: "\n", omittingEmptySubsequences: false)
        .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
        .joined(separator: "\n")
    }

    /// The session card's tooltip on Contribute: the one line that chooses it.
    static let tooltip = #".help(consent == nil ? "" : TCConsentCopy.gateHelp(pinned: armed(entry)) ?? "")"#

    /// The source between `signature` and the brace that closes it, with
    /// `//` comments stripped: a comment is not something a contributor
    /// reads, and the comment above `canContribute` names both of the very
    /// expressions these assertions look for.
    static func declaration(
        _ signature: String, in viewPath: URL, file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let text = try? String(contentsOf: viewPath, encoding: .utf8) else {
            XCTFail("could not read \(viewPath.path)", file: file, line: line)
            return nil
        }
        guard let start = text.range(of: signature) else {
            XCTFail(
                "\(viewPath.lastPathComponent) no longer declares `\(signature)`",
                file: file, line: line)
            return nil
        }
        var depth = 1
        var index = start.upperBound
        var inString = false
        var inLineComment = false
        var body = ""
        while index < text.endIndex {
            let character = text[index]
            let next = text.index(after: index)
            if inLineComment {
                if character == "\n" { inLineComment = false }
            } else if inString {
                if character == "\\" {
                    body.append(character)
                    if next < text.endIndex { body.append(text[next]) }
                    index = next < text.endIndex ? text.index(after: next) : text.endIndex
                    continue
                }
                if character == "\"" { inString = false }
                body.append(character)
            } else if character == "/", next < text.endIndex, text[next] == "/" {
                inLineComment = true
            } else if character == "\"" {
                inString = true
                body.append(character)
            } else if character == "{" {
                depth += 1
                body.append(character)
            } else if character == "}" {
                depth -= 1
                if depth == 0 { return body }
                body.append(character)
            } else {
                body.append(character)
            }
            index = next
        }
        XCTFail("the body of `\(signature)` is unterminated", file: file, line: line)
        return nil
    }
}

final class ConsentBindingTests: XCTestCase {

    // MARK: - What arms Contribute

    /// The pin is the enrollment, not the arrival of a summary.
    ///
    /// This is the behaviour change the slice carried, and reverting it is
    /// one token. `summary != nil` is true of a preview built from the
    /// placeholder identity an unenrolled device carries -- nothing was
    /// pinned, so there is no envelope for an approval to bind to, and the
    /// shared tooltip says as much.
    func testContributeIsArmedByTheEnrollmentAndNotByTheSummaryArriving() throws {
        let body = try XCTUnwrap(ConsentSurfaceSource.contributeArmedBody())
        XCTAssertTrue(
            body.contains("ReadGate.canContribute(hasPinnedPreview: enrolled == true)"),
            "the card no longer arms Contribute on the preview's enrollment: \(body)")
        XCTAssertFalse(
            body.contains("summary != nil"),
            "the card arms Contribute on a summary that pinned nothing: \(body)")
        // And the card feeds it the enrollment of the preview it asked for.
        let monitor = try XCTUnwrap(ConsentSurfaceSource.monitorCode())
        XCTAssertTrue(monitor.contains("TracesStore.contributeArmed("), "the card no longer asks the rule")
        XCTAssertTrue(
            monitor.contains("?.enrolled, consent: consent,"),
            "the card feeds the rule something other than the preview's enrollment")
    }

    /// No claim, no approval.
    ///
    /// The gate statement is the whole of what a contributor is told before
    /// pressing `Contribute`. When the payload will not decode there is
    /// nothing to print, and a card that armed the button anyway would be
    /// taking an approval against a claim nobody made. The Windows shell
    /// spells the same rule as `ReadGate.CanArm`.
    func testContributeIsDisarmedWhenTheSentencesCouldNotBeRead() throws {
        let body = try XCTUnwrap(ConsentSurfaceSource.contributeArmedBody())
        XCTAssertTrue(
            body.contains("consent != nil"),
            "the card arms Contribute without the sentences that explain it: \(body)")
    }

    /// The tooltip is silent under the same condition, rather than wrong.
    ///
    /// `tc_consent_gate_help` is its own ABI call and can answer while the
    /// bundle will not decode. Without the guard that combination paints a
    /// disarmed button with the not-connected sentence -- a claim about the
    /// contributor's device, made because this shell could not read its own
    /// copy.
    func testTheTooltipIsEmptyRatherThanWrongWhenTheSentencesAreMissing() throws {
        let monitor = try XCTUnwrap(ConsentSurfaceSource.monitorCode())
        XCTAssertTrue(
            monitor.contains(ConsentSurfaceSource.tooltip),
            "the card's tooltip is chosen without checking the sentences decoded")
    }

    // MARK: - Where the words come from

    /// The statement is read, never written here.
    func testTheStatementIsReadFromTheSharedCopy() throws {
        let body = try XCTUnwrap(ConsentSurfaceSource.gateStatementBody())
        XCTAssertTrue(
            body.contains("consent?.gateStatement"),
            "the sheet no longer prints the shared statement: \(body)")
        XCTAssertFalse(
            body.contains("ReadGate.statement"),
            "the sheet reaches for a sentence that moved to consent_copy.rs: \(body)")
    }

    /// Neither the arming nor the tooltip may hold a sentence of its own.
    ///
    /// An empty literal is the refusal, not a sentence: it is what the
    /// surface renders when there is nothing honest to say. Anything longer
    /// written here would be a fourth place the consent wording lives.
    func testNoSentenceIsAuthoredOnThisSurface() throws {
        for (name, body) in [
            ("contributeArmed", try XCTUnwrap(ConsentSurfaceSource.contributeArmedBody())),
            ("the card's tooltip", ConsentSurfaceSource.tooltip),
            ("gateStatement", try XCTUnwrap(ConsentSurfaceSource.gateStatementBody())),
        ] {
            let pieces = body.split(separator: "\"", omittingEmptySubsequences: false)
            for (offset, piece) in pieces.enumerated() where !offset.isMultiple(of: 2) {
                XCTAssertTrue(
                    piece.isEmpty,
                    "\(name) authors a sentence of its own (\(piece)); consent wording comes "
                        + "from consent_copy.rs across the ABI")
            }
        }
    }
}
