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

    /// Every Rust-side call as the app wires them: straight through the ABI.
    /// Nothing below is a Swift branch table -- that is the point of this
    /// file.
    private let calls = RoutingCalls(
        tokenLine: { TCRoutingCopy.tokenLine(path: $0) },
        unreachableLine: { TCRoutingCopy.unreachableLine(port: $0) },
        toolWord: { TCRoutingCopy.toolWord(sourceMode: $0, wiring: $1) },
        toolTone: { TCRoutingCopy.toolTone(sourceMode: $0, wiring: $1) },
        stateLine: { TCRoutingCopy.stateLine(state: $0) },
        stateTone: { TCRoutingCopy.stateTone(state: $0) }
    )

    private func rows(
        claude: String = "watch",
        codex: String = "watch",
        gemini: String = "watch",
        cline: String = "watch",
        evidence: RoutingEvidence?,
        copy: RoutingCopy
    ) -> [RoutingToolWord] {
        RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: claude, codex: codex, gemini: gemini, cline: cline),
            evidence: evidence,
            copy: copy,
            calls: calls
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

    /// The branch table itself crosses, not only the words.
    ///
    /// The four literals above are reached through the export, so a rename in
    /// the Rust fails there. This is the other half: change which word a
    /// *state* maps to in `routing_copy.rs` -- without touching a single
    /// string -- and this goes red. Before the export existed it could not,
    /// because the mapping was a `switch` in Swift and only the words were
    /// shared.
    func testTheWordEachWiringStateGetsIsTheRustsChoiceAndNotThisShells() {
        guard let copy = copy() else { return }
        for mode in ["watch", "unset"] {
            XCTAssertEqual(
                TCRoutingCopy.toolWord(sourceMode: mode, wiring: RoutingToolWiring.wired.abiValue),
                copy.wordPrivate
            )
            XCTAssertEqual(
                TCRoutingCopy.toolWord(
                    sourceMode: mode, wiring: RoutingToolWiring.notWired.abiValue
                ),
                copy.wordDirect
            )
            XCTAssertEqual(
                TCRoutingCopy.toolWord(
                    sourceMode: mode, wiring: RoutingToolWiring.unknown.abiValue
                ),
                copy.wordUnknown
            )
        }
        // Only "off" means not used; "unset" watches the conventional
        // location, which is a tool in use.
        XCTAssertEqual(
            TCRoutingCopy.toolWord(sourceMode: "off", wiring: RoutingToolWiring.wired.abiValue),
            copy.wordNotUsed
        )
        // A wiring value this build has never heard of claims nothing.
        XCTAssertEqual(TCRoutingCopy.toolWord(sourceMode: "watch", wiring: 99), copy.wordUnknown)
    }

    /// The reassuring tone falls on exactly the word that claims privacy,
    /// over every input pair -- and both come from the same shared table, so
    /// no styling decision anywhere reads the rendered string.
    func testTheReassuringToneFallsOnThePrivateWordAloneThroughTheRealAbi() {
        guard let copy = copy() else { return }
        for mode in ["off", "watch", "unset", "", "something_new"] {
            for wiring in [
                RoutingToolWiring.wired, .notWired, .unknown,
            ] {
                let word = RoutingSurface.toolWord(
                    sourceMode: mode, wiring: wiring, copy: copy, calls: calls
                )
                let tone = RoutingSurface.toolTone(sourceMode: mode, wiring: wiring, calls: calls)
                XCTAssertEqual(
                    tone == .clear, word == copy.wordPrivate, "\(mode)/\(wiring) -> \(word)"
                )
                XCTAssertNotEqual(
                    tone, RoutingTone.held, "a tool word may not take the daemon's held tone"
                )
            }
        }

        // And it arrives on the row, so the view never works it out.
        let rendered = rows(
            evidence: RoutingEvidence(
                outcome: .reachable,
                tools: [
                    "claude": RoutingToolRow(installed: true, wired: true),
                    "codex": RoutingToolRow(installed: true, wired: false),
                ]
            ),
            copy: copy
        )
        XCTAssertEqual(rendered[0].tone, .clear)
        XCTAssertEqual(rendered[1].tone, .neutral)
        XCTAssertEqual(rendered[2].tone, .neutral)
    }

    /// The daemon state's tone is the Rust's choice, not this shell's, and it
    /// agrees with the sentence that state gets.
    ///
    /// This was the last routing branch table still written out natively
    /// here. Change which tone a state maps to in `routing_copy.rs` --
    /// without touching a string -- and this goes red.
    func testTheToneEachStateGetsIsTheRustsChoiceAndNotThisShells() {
        guard let copy = copy() else { return }
        XCTAssertEqual(RoutingSurface.tone(forState: "awaiting_rows", calls: calls), .held)
        XCTAssertEqual(RoutingSurface.tone(forState: "rows_seen", calls: calls), .clear)
        XCTAssertEqual(RoutingSurface.tone(forState: "not_declared", calls: calls), .neutral)

        for state in ["not_declared", "awaiting_rows", "rows_seen", "", "ROWS_SEEN", "later"] {
            let tone = RoutingSurface.tone(forState: state, calls: calls)
            let line = RoutingSurface.stateLine(state, copy: copy, calls: calls)
            // The tone and the sentence are one decision.
            XCTAssertEqual(tone == .neutral, line == copy.stateOff, state)
            // The stamp is gated on the same reading.
            XCTAssertEqual(
                RoutingSurface.showsLastChecked(forState: state, calls: calls),
                tone != .neutral,
                state
            )
        }

        // `awaiting_rows` is what a contributor sees immediately after
        // touching anything on this card. It is held, and never a fault.
        XCTAssertNotEqual(RoutingSurface.tone(forState: "awaiting_rows", calls: calls), .neutral)
    }

    /// A state this build has never heard of claims nothing: it reads as the
    /// off line and never falls through to either "on" sentence.
    func testAStateThisBuildHasNeverHeardOfReadsAsTheOffLine() {
        guard let copy = copy() else { return }
        for state in ["a_state_from_a_later_daemon", "", "ROWS_SEEN"] {
            let line = RoutingSurface.stateLine(state, copy: copy, calls: calls)
            XCTAssertEqual(line, copy.stateOff, state)
            XCTAssertNotEqual(line, copy.stateReading, state)
            XCTAssertNotEqual(line, copy.stateWaiting, state)
        }
    }

    /// The tool names on the rows are the shared ones too.
    func testTheToolNamesAreTheOnesTheRustExports() {
        guard let copy = copy() else { return }
        XCTAssertEqual(
            rows(evidence: nil, copy: copy).map(\.name),
            ["Claude Code", "Codex", "Gemini CLI", "Cline"]
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

    /// Cline has no upstream row either, so the same sentence holds for it:
    /// not known, never a verdict, on a machine where it is in daily use.
    func testClineReadsNotKnownOnAMachineWhereItIsInUse() {
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
        XCTAssertEqual(rendered[3].name, "Cline")
        XCTAssertEqual(rendered[3].word, "Not known")
    }

    /// The three status states, rendered through the real payload.
    func testTheStatusStatesRenderTheRustsSentences() {
        guard let copy = copy() else { return }
        XCTAssertEqual(RoutingSurface.stateLine("not_declared", copy: copy, calls: calls), copy.stateOff)
        XCTAssertEqual(RoutingSurface.stateLine("awaiting_rows", copy: copy, calls: calls), copy.stateWaiting)
        XCTAssertEqual(RoutingSurface.stateLine("rows_seen", copy: copy, calls: calls), copy.stateReading)
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
            .tokenUnusable(path: path), copy: copy, calls: calls
        )
        XCTAssertTrue(line.contains(path), line)
        XCTAssertNotEqual(line, copy.checkUnavailable)
    }

    func testTheUnreachableLineNamesThePortThroughTheRealSentence() {
        guard let copy = copy() else { return }
        let line = RoutingSurface.probeLine(
            .unreachable(port: 8463), copy: copy, calls: calls
        )
        XCTAssertTrue(line.contains("8463"), line)
    }

    // MARK: - What the surface may never say

    /// Nothing here waits on the app being started again. The daemon applies
    /// a changed declaration to itself, so a sentence sending somebody to
    /// restart, relaunch or quit would be describing a product this is not.
    func testNothingOnThisSurfaceAsksAnybodyToRestartAnything() {
        guard let copy = copy() else { return }
        for text in Self.everySentence(copy: copy, calls: calls) {
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
        for text in Self.everySentence(copy: copy, calls: calls) {
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
        copy: RoutingCopy, calls: RoutingCalls
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
            texts.append(RoutingSurface.probeLine(outcome, copy: copy, calls: calls))
        }
        texts.append(contentsOf: [
            TCRoutingCopy.lastChecked(when: "an hour ago") ?? "",
        ])
        return texts
    }
}
