import TCBridge
import TCShellCore
import XCTest

/// The rendered surface, driven by the real dylib.
///
/// `RoutingSurfaceTests` proves the mapping reaches for the right *field*,
/// against a payload of sentinels. This proves the field carries the Rust's
/// word: everything below renders a state through the same functions the
/// window calls and compares the result to a literal. Change a word in
/// `trace_commons_contributor::routing_copy` and these go red, which is what
/// stands behind the claim that this shell prints the shared wording rather
/// than a copy of its own.
final class RoutingSurfaceExportTests: XCTestCase {
    private func copy(file: StaticString = #filePath, line: UInt = #line) -> RoutingCopy? {
        guard let json = TCRoutingCopy.copyJSON() else {
            XCTFail("the routing copy export returned nil", file: file, line: line)
            return nil
        }
        guard let copy = RoutingCopy.decode(fromJSON: json) else {
            XCTFail("the payload did not decode: \(json)", file: file, line: line)
            return nil
        }
        return copy
    }

    /// The sentences as the app wires them: straight through the ABI.
    private let sentences = RoutingSentences(
        tokenLine: { TCRoutingCopy.tokenLine(path: $0) },
        unreachableLine: { TCRoutingCopy.unreachableLine(port: $0) }
    )

    private func rows(
        claude: String = "watch",
        codex: String = "watch",
        gemini: String = "watch",
        evidence: RoutingEvidence?,
        copy: RoutingCopy
    ) -> [RoutingToolWord] {
        RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: claude, codex: codex, gemini: gemini),
            evidence: evidence,
            copy: copy
        )
    }

    // MARK: - The word the Rust chose is the word that is rendered

    /// The tripwire. These four literals are the shipped vocabulary, reached
    /// through the rendering path rather than read off the payload, so a
    /// rename in the Rust fails here and not only in the payload test.
    func testEachStateRendersTheWordTheRustExports() {
        guard let copy = copy() else { return }

        let wired = rows(
            evidence: RoutingEvidence(
                outcome: .reachable, tools: ["claude": RoutingToolRow(installed: true, wired: true)]
            ),
            copy: copy
        )
        XCTAssertEqual(wired[0].word, "Private")

        let direct = rows(
            evidence: RoutingEvidence(
                outcome: .reachable, tools: ["claude": RoutingToolRow(installed: true, wired: false)]
            ),
            copy: copy
        )
        XCTAssertEqual(direct[0].word, "Sends direct")

        let nothing = rows(evidence: nil, copy: copy)
        XCTAssertEqual(nothing[0].word, "Not known")

        let unused = rows(claude: "off", evidence: nil, copy: copy)
        XCTAssertEqual(unused[0].word, "Not used")
    }

    /// The tool names on the rows are the shared ones too.
    func testTheToolNamesAreTheOnesTheRustExports() {
        guard let copy = copy() else { return }
        XCTAssertEqual(
            rows(evidence: nil, copy: copy).map(\.name),
            ["Claude Code", "Codex", "Gemini CLI"]
        )
    }

    /// Gemini CLI on a machine where it is installed and in daily use. There
    /// is no `gemini` row upstream at all, so this is what a correct surface
    /// says about it -- and saying anything else would be inventing a
    /// verdict.
    func testGeminiReadsNotKnownOnAMachineWhereItIsInUse() {
        guard let copy = copy() else { return }
        let rendered = rows(
            evidence: RoutingEvidence(
                outcome: .reachable,
                tools: [
                    "claude": RoutingToolRow(installed: true, wired: true),
                    "codex": RoutingToolRow(installed: true, wired: true),
                ]
            ),
            copy: copy
        )
        XCTAssertEqual(rendered[2].name, "Gemini CLI")
        XCTAssertEqual(rendered[2].word, "Not known")
    }

    /// The three status states, rendered through the real payload.
    func testTheStatusStatesRenderTheRustsSentences() {
        guard let copy = copy() else { return }
        XCTAssertEqual(RoutingSurface.stateLine("not_declared", copy: copy), copy.stateOff)
        XCTAssertEqual(RoutingSurface.stateLine("awaiting_rows", copy: copy), copy.stateWaiting)
        XCTAssertEqual(RoutingSurface.stateLine("rows_seen", copy: copy), copy.stateReading)
        XCTAssertTrue(copy.stateReading.hasPrefix("On"), copy.stateReading)
    }

    /// The probe outcome that matters most on macOS, end to end: a
    /// GUI-launched daemon never sees `$IRONWIRE_HOME`, so it reads
    /// `~/.ironwire` whatever a login shell was told, and the path it
    /// actually looked at is the one fact that makes that fixable.
    func testTheTokenLineNamesTheAbsolutePathThroughTheRealSentence() {
        guard let copy = copy() else { return }
        let path = "/Users/someone/.ironwire/control.token"
        let line = RoutingSurface.probeLine(
            .tokenUnusable(path: path), copy: copy, sentences: sentences
        )
        XCTAssertTrue(line.contains(path), line)
        XCTAssertNotEqual(line, copy.checkUnavailable)
    }

    func testTheUnreachableLineNamesThePortThroughTheRealSentence() {
        guard let copy = copy() else { return }
        let line = RoutingSurface.probeLine(
            .unreachable(port: 8463), copy: copy, sentences: sentences
        )
        XCTAssertTrue(line.contains("8463"), line)
    }

    // MARK: - What the surface may never say

    /// Nothing here waits on the app being started again. The daemon applies
    /// a changed declaration to itself, so a sentence sending somebody to
    /// restart, relaunch or quit would be describing a product this is not.
    func testNothingOnThisSurfaceAsksAnybodyToRestartAnything() {
        guard let copy = copy() else { return }
        for text in Self.everySentence(copy: copy, sentences: sentences) {
            for banned in ["restart", "relaunch", "reopen", "quit", "reboot", "start it again"] {
                XCTAssertFalse(
                    text.lowercased().contains(banned),
                    "a restart notice reached the routing surface: \(text)"
                )
            }
        }
    }

    /// This surface is read by someone with no invite and no account. A word
    /// about corpora, credit, ownership, contribution or money would be a
    /// pitch on a privacy screen -- and greying one out is still saying it.
    func testNothingOnThisSurfaceMentionsCorporaCreditsOrMoney() {
        guard let copy = copy() else { return }
        for text in Self.everySentence(copy: copy, sentences: sentences) {
            for banned in [
                "corpus", "corpora", "credit", "reward", "earn", "payment", "paid", "money",
                "ownership", "contribute", "contribution", "invite", "sign up", "account",
            ] {
                XCTAssertFalse(
                    text.lowercased().contains(banned),
                    "\(banned.trimmingCharacters(in: .whitespaces)) reached the routing surface: \(text)"
                )
            }
        }
    }

    /// One word claims privacy and none denies it. Asserted on what is
    /// rendered, not only on the payload: this is the side that would print
    /// the wrong one.
    func testExactlyOneRenderedWordClaimsPrivacy() {
        guard let copy = copy() else { return }
        var claims = 0
        for word in copy.words {
            let lower = word.lowercased()
            if lower.contains("privat") {
                claims += 1
                XCTAssertEqual(word, "Private", "only the wired word may use that stem")
            }
        }
        XCTAssertEqual(claims, 1)
    }

    /// Every fixed string on the payload, plus every sentence that
    /// interpolates, in both of each one's shapes.
    private static func everySentence(
        copy: RoutingCopy, sentences: RoutingSentences
    ) -> [String] {
        var texts: [String] = []
        for child in Mirror(reflecting: copy).children {
            if let text = child.value as? String { texts.append(text) }
        }
        for outcome: RoutingProbeOutcome in [
            .reachable, .unknown,
            .tokenUnusable(path: "/Users/someone/.ironwire/control.token"),
            .tokenUnusable(path: nil),
            .unreachable(port: 8463), .unreachable(port: nil),
        ] {
            texts.append(RoutingSurface.probeLine(outcome, copy: copy, sentences: sentences))
        }
        texts.append(contentsOf: [
            TCRoutingCopy.lastChecked(when: "an hour ago") ?? "",
        ])
        return texts
    }
}
