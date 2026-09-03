import XCTest
@testable import TCShellCore

/// The state-to-copy mapping for the routing surface, tested against a
/// payload of sentinels rather than against the shipped words.
///
/// Every string this file compares is a sentinel like `W-PRIVATE`, and that
/// is deliberate. A test here that spelled the real word would pass whether
/// the mapping read the payload or a literal of its own, which is the exact
/// drift the shared source exists to prevent. What is asserted here is
/// *which field* each state reaches for; `RoutingSurfaceExportTests` asserts,
/// against the real dylib, that the field carries the Rust's word.
final class RoutingSurfaceTests: XCTestCase {
    /// A payload whose every field is distinguishable from every other, so
    /// a mapping that reached for the neighbouring field would be caught.
    private static let fixtureJSON = """
    {
      "tools_heading": "H-TOOLS",
      "word_private": "W-PRIVATE",
      "word_direct": "W-DIRECT",
      "word_unknown": "W-UNKNOWN",
      "word_not_used": "W-NOTUSED",
      "tool_claude": "T-CLAUDE",
      "tool_codex": "T-CODEX",
      "tool_gemini": "T-GEMINI",
      "tool_cline": "T-CLINE",
      "intro": "S-INTRO",
      "toggle": "S-TOGGLE",
      "applies_at_once": "S-APPLIES",
      "port_title": "S-PORTTITLE",
      "port_note": "S-PORTNOTE",
      "folder_title": "S-FOLDERTITLE",
      "folder_note": "S-FOLDERNOTE",
      "apply": "S-APPLY",
      "checking": "S-CHECKING",
      "check_unavailable": "S-UNAVAILABLE",
      "probe_reachable": "S-REACHABLE",
      "state_off": "S-STATEOFF",
      "state_waiting": "S-STATEWAITING",
      "state_reading": "S-STATEREADING"
    }
    """

    private func copy() -> RoutingCopy {
        guard let copy = RoutingCopy.decode(fromJSON: Self.fixtureJSON) else {
            fatalError("the fixture payload must decode")
        }
        return copy
    }

    /// Calls that report which one was asked for and what arguments it got.
    /// The real ones live in Rust and cross the ABI; this target does not
    /// link it.
    ///
    /// The word and tone fakes are echoes and **not** a Swift copy of the
    /// Rust's branch table -- the tone one deliberately keys off a different
    /// wiring state than the real rule does. What this target can honestly
    /// assert is that each row asks with its own mode and its own wiring;
    /// which word and which tone those two produce is the Rust's decision,
    /// and `RoutingSurfaceExportTests` pins it against the real dylib.
    private func calls() -> RoutingCalls {
        RoutingCalls(
            tokenLine: { path in path.map { "L-TOKEN:\($0)" } ?? "L-TOKEN:none" },
            unreachableLine: { port in port.map { "L-UNREACHABLE:\($0)" } ?? "L-UNREACHABLE:none" },
            toolWord: { mode, wiring in "W:\(mode):\(wiring)" },
            toolTone: { _, wiring in wiring == RoutingToolWiring.notWired.abiValue ? 2 : 0 },
            stateLine: { state in "S:\(state)" },
            // Echoes the state's length, so a tone read here is provably
            // the one this fake was asked for and not the real rule.
            stateTone: { state in state.count == 3 ? 1 : 2 }
        )
    }

    /// Calls the ABI refused to answer. Nil is a real answer from the bridge
    /// -- it is what a caught panic looks like -- and this surface has to
    /// have somewhere to go when it happens.
    private func silentCalls() -> RoutingCalls {
        RoutingCalls(
            tokenLine: { _ in nil },
            unreachableLine: { _ in nil },
            toolWord: { _, _ in nil },
            toolTone: { _, _ in 0 },
            stateLine: { _ in nil },
            stateTone: { _ in 0 }
        )
    }

    // MARK: - The probe result: three outcomes, three strings

    func testAReachableProbeSaysTheProbeReachedIt() {
        XCTAssertEqual(
            RoutingSurface.probeLine(.reachable, copy: copy(), calls: calls()),
            copy().probeReachable
        )
    }

    /// The likely macOS failure, and the one fact that makes it fixable: a
    /// GUI-launched daemon never sees `$IRONWIRE_HOME`, so it reads
    /// `~/.ironwire` whatever a login shell was told. The path the daemon
    /// reported has to survive into what is on screen.
    func testAnUnusableTokenNamesTheAbsolutePathTheDaemonReported() {
        let path = "/Users/someone/.ironwire/control.token"
        let line = RoutingSurface.probeLine(
            .tokenUnusable(path: path), copy: copy(), calls: calls()
        )
        XCTAssertTrue(line.contains(path), "the reported path did not survive: \(line)")
    }

    /// Nothing resolved at all is a different sentence, not an empty path.
    /// A line reading "could not use the file at " is worse than one that
    /// admits it does not know where to look.
    func testAnUnusableTokenWithNoPathIsItsOwnSentence() {
        let named = RoutingSurface.probeLine(
            .tokenUnusable(path: "/Users/someone/.ironwire/control.token"),
            copy: copy(), calls: calls()
        )
        let unnamed = RoutingSurface.probeLine(
            .tokenUnusable(path: nil), copy: copy(), calls: calls()
        )
        XCTAssertNotEqual(named, unnamed)
        XCTAssertEqual(unnamed, "L-TOKEN:none")
    }

    func testAnUnreachableProbeNamesThePortThatWasTried() {
        XCTAssertEqual(
            RoutingSurface.probeLine(
                .unreachable(port: 8463), copy: copy(), calls: calls()
            ),
            "L-UNREACHABLE:8463"
        )
    }

    /// No port tried must not become "port 0". Port 0 is the ask-the-kernel
    /// sentinel, and the daemon refuses it outright.
    func testNoPortTriedIsNotRenderedAsPortZero() {
        let line = RoutingSurface.probeLine(
            .unreachable(port: nil), copy: copy(), calls: calls()
        )
        XCTAssertEqual(line, "L-UNREACHABLE:none")
        XCTAssertFalse(line.contains("0"), line)
    }

    /// An outcome this build cannot read claims nothing about the proxy in
    /// either direction, and must not send anyone to check a port or a file
    /// that is fine.
    func testAnUnreadableOutcomeSaysTheCheckCouldNotBeRun() {
        XCTAssertEqual(
            RoutingSurface.probeLine(.unknown, copy: copy(), calls: calls()),
            copy().checkUnavailable
        )
    }

    /// A sentence the ABI would not assemble degrades to the same
    /// claims-nothing line, never to a half-sentence and never to a word
    /// this shell wrote.
    func testASentenceTheBridgeRefusedDegradesToTheCheckCouldNotBeRunLine() {
        for outcome: RoutingProbeOutcome in [
            .tokenUnusable(path: "/Users/someone/.ironwire/control.token"),
            .tokenUnusable(path: nil),
            .unreachable(port: 8463),
            .unreachable(port: nil),
        ] {
            XCTAssertEqual(
                RoutingSurface.probeLine(outcome, copy: copy(), calls: silentCalls()),
                copy().checkUnavailable,
                "\(outcome)"
            )
        }
    }

    // MARK: - Reading the daemon's probe answer

    func testTheDaemonsThreeOutcomesAreReadAsThemselves() {
        XCTAssertEqual(RoutingProbeOutcome.parse(["outcome": "reachable"]), .reachable)
        XCTAssertEqual(
            RoutingProbeOutcome.parse([
                "outcome": "token_unreadable", "token_path": "/Users/x/.ironwire/control.token",
            ]),
            .tokenUnusable(path: "/Users/x/.ironwire/control.token")
        )
        XCTAssertEqual(
            RoutingProbeOutcome.parse(["outcome": "unreachable", "port": 8463]),
            .unreachable(port: 8463)
        )
    }

    /// `token_path` is absent, not null, when nothing resolved. The parse
    /// must not turn that into an empty string.
    func testAnUnreadableTokenWithNoPathParsesAsNoPath() {
        XCTAssertEqual(
            RoutingProbeOutcome.parse(["outcome": "token_unreadable"]),
            .tokenUnusable(path: nil)
        )
    }

    /// An outcome this build does not know, a missing outcome, and a port
    /// that is not a port all degrade rather than assert.
    func testAnAnswerThisBuildCannotReadIsUnknown() {
        XCTAssertEqual(RoutingProbeOutcome.parse([:]), .unknown)
        XCTAssertEqual(RoutingProbeOutcome.parse(["outcome": "something_new"]), .unknown)
        XCTAssertEqual(
            RoutingProbeOutcome.parse(["outcome": "unreachable", "port": "eight"]),
            .unreachable(port: nil)
        )
        XCTAssertEqual(
            RoutingProbeOutcome.parse(["outcome": "unreachable", "port": 70000]),
            .unreachable(port: nil)
        )
    }

    // MARK: - The status line: three states

    /// The state reaches the shared table verbatim, and what comes back is
    /// what is shown. Which sentence each state maps to is the Rust's
    /// decision, pinned against the real dylib in `RoutingSurfaceExportTests`.
    func testTheStateIsPassedToTheSharedTableAndItsAnswerIsShown() {
        for state in ["not_declared", "awaiting_rows", "rows_seen", "some_new_state", ""] {
            XCTAssertEqual(
                RoutingSurface.stateLine(state, copy: copy(), calls: calls()),
                "S:\(state)",
                state
            )
        }
    }

    /// A line the ABI would not produce falls back to the off line, which
    /// claims nothing -- never to a half-sentence and never to either "on"
    /// sentence.
    func testAStateLineTheAbiRefusedFallsBackToTheLineThatClaimsNothing() {
        let line = RoutingSurface.stateLine("rows_seen", copy: copy(), calls: silentCalls())
        XCTAssertEqual(line, copy().stateOff)
        XCTAssertNotEqual(line, copy().stateReading)
        XCTAssertNotEqual(line, copy().stateWaiting)
    }

    /// `awaiting_rows` is not a fault. A reader built a moment ago starts
    /// empty by construction, so this is the state a contributor sees
    /// immediately after changing anything here -- painting it as an error
    /// would accuse a working proxy of being broken at that exact moment.
    /// The state reaches the shared tone table verbatim, and the answer that
    /// comes back is the one used. Which tone each state means is the Rust's
    /// decision, pinned against the real dylib in `RoutingSurfaceExportTests`.
    ///
    /// The fake keys off the state's length, which is not the real rule, so
    /// a `switch` that had quietly survived in this file could not produce
    /// these results.
    func testTheStateIsPassedToTheSharedToneTableAndItsAnswerIsUsed() {
        XCTAssertEqual(RoutingSurface.tone(forState: "abc", calls: calls()), .held)
        XCTAssertEqual(RoutingSurface.tone(forState: "abcd", calls: calls()), .clear)
        XCTAssertEqual(RoutingSurface.tone(forState: "rows_seen", calls: silentCalls()), .neutral)
    }

    /// A tone value this build has never heard of claims nothing, and the
    /// decoder is the ABI's numbering rather than this enum's declaration
    /// order.
    func testAToneTheAbiDoesNotDefineClaimsNothing() {
        XCTAssertEqual(RoutingTone.fromABI(0), .neutral)
        XCTAssertEqual(RoutingTone.fromABI(1), .held)
        XCTAssertEqual(RoutingTone.fromABI(2), .clear)
        XCTAssertEqual(RoutingTone.fromABI(3), .neutral)
        XCTAssertEqual(RoutingTone.fromABI(-1), .neutral)
    }

    /// "Last checked" is a per-process stamp on the running daemon, so it is
    /// only shown where it says something. On a state that has had no answer
    /// at all it would read as an install date or a connected-since, which is
    /// what it is not.
    ///
    /// Derived from the same shared tone, so the stamp cannot be gated on a
    /// different reading of the state than the sentence above it.
    func testTheLastCheckedStampIsWithheldOnAStateThatNeverAnswered() {
        XCTAssertFalse(RoutingSurface.showsLastChecked(forState: "any", calls: silentCalls()))
        XCTAssertTrue(RoutingSurface.showsLastChecked(forState: "abc", calls: calls()))
        XCTAssertTrue(RoutingSurface.showsLastChecked(forState: "abcd", calls: calls()))
    }

    // MARK: - Per-tool words, from the tools answer and not the switch

    private func evidence(
        outcome: RoutingProbeOutcome = .reachable,
        tools: [String: RoutingToolRow] = [:]
    ) -> RoutingEvidence {
        RoutingEvidence(outcome: outcome, tools: tools)
    }

    private func rows(
        claude: String = "watch",
        codex: String = "watch",
        gemini: String = "watch",
        cline: String = "watch",
        evidence: RoutingEvidence?
    ) -> [RoutingToolWord] {
        RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: claude, codex: codex, gemini: gemini, cline: cline),
            evidence: evidence,
            copy: copy(),
            calls: calls()
        )
    }

    /// Each row asks the shared table with **that tool's** mode and **that
    /// tool's** wiring. This is the whole of what this target decides; the
    /// mapping from the pair to a word is the Rust's.
    func testEachRowAsksWithItsOwnModeAndItsOwnWiring() {
        let rendered = rows(
            claude: "watch",
            codex: "unset",
            gemini: "off",
            cline: "unset",
            evidence: evidence(tools: [
                "claude": RoutingToolRow(installed: true, wired: true),
                "codex": RoutingToolRow(installed: true, wired: false),
            ])
        )
        XCTAssertEqual(rendered[0].word, "W:watch:\(RoutingToolWiring.wired.abiValue)")
        XCTAssertEqual(rendered[1].word, "W:unset:\(RoutingToolWiring.notWired.abiValue)")
        // Gemini has no row upstream at all, so it asks with the unknown
        // state rather than with a guess.
        XCTAssertEqual(rendered[2].word, "W:off:\(RoutingToolWiring.unknown.abiValue)")
        // Cline likewise: no upstream row, so unknown rather than a guess.
        XCTAssertEqual(rendered[3].word, "W:unset:\(RoutingToolWiring.unknown.abiValue)")
    }

    /// The declaration switch is not among those two inputs. Declaring
    /// IronWire in this app says nothing about whether Codex is configured to
    /// send through it, and a shell that rendered one switch as three
    /// verdicts would be inventing two of them.
    func testTheDeclarationIsNotAnInputToAnyToolWord() {
        for row in rows(claude: "watch", codex: "unset", gemini: "watch", cline: "watch", evidence: evidence()) {
            XCTAssertTrue(
                row.word.hasSuffix(":\(RoutingToolWiring.unknown.abiValue)"),
                "\(row.name) asked with \(row.word)"
            )
        }
    }

    /// Nothing answered is a stable state -- a port nothing listens on, a
    /// credential that is refused -- so a word built on it would keep
    /// asserting while the card underneath says nothing answered. The rows
    /// must ask with the unknown state whatever the payload carried.
    func testNothingAnsweredLeavesEveryToolUnknownWhateverWasCached() {
        for outcome: RoutingProbeOutcome in [
            .unreachable(port: 8463), .tokenUnusable(path: "/Users/x/.ironwire/control.token"),
            .unknown,
        ] {
            let rendered = rows(
                evidence: evidence(
                    outcome: outcome,
                    tools: ["claude": RoutingToolRow(installed: true, wired: true)]
                )
            )
            XCTAssertEqual(
                rendered[0].word,
                "W:watch:\(RoutingToolWiring.unknown.abiValue)",
                "\(outcome)"
            )
        }
    }

    /// No answer held at all is not a verdict either.
    func testNoEvidenceAtAllLeavesEveryToolUnknown() {
        for row in rows(evidence: nil) {
            XCTAssertTrue(
                row.word.hasSuffix(":\(RoutingToolWiring.unknown.abiValue)"),
                row.name
            )
        }
    }

    /// IronWire saying a tool is not installed, while this app is watching
    /// that tool's sessions, is two detectors disagreeing about one machine.
    /// That is not evidence for a verdict.
    func testAToolIronWireSaysIsNotInstalledGetsNoVerdict() {
        let rendered = rows(
            evidence: evidence(tools: ["claude": RoutingToolRow(installed: false, wired: false)])
        )
        XCTAssertEqual(rendered[0].word, "W:watch:\(RoutingToolWiring.unknown.abiValue)")
    }

    /// A word the ABI would not produce falls back to the one that claims
    /// nothing -- never to a blank, and never to a word chosen here.
    func testAWordTheAbiRefusedFallsBackToTheWordThatClaimsNothing() {
        let rendered = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch", cline: "watch"),
            evidence: evidence(tools: ["claude": RoutingToolRow(installed: true, wired: true)]),
            copy: copy(),
            calls: silentCalls()
        )
        for row in rendered {
            XCTAssertEqual(row.word, copy().wordUnknown, row.name)
            XCTAssertNotEqual(row.word, copy().wordPrivate, row.name)
        }
    }

    /// The four rows are always all four, in one order, so a missing
    /// answer is a word rather than a vanished row.
    func testTheSurfaceAlwaysNamesAllFourToolsInOneOrder() {
        XCTAssertEqual(
            rows(claude: "off", codex: "off", gemini: "off", cline: "off", evidence: nil).map(\.name),
            [copy().toolClaude, copy().toolCodex, copy().toolGemini, copy().toolCline]
        )
    }

    /// The tone on a row is the shared table's answer for that row's own two
    /// inputs, and nothing here reads the rendered word to reach it.
    ///
    /// The fake keys its tone off the *not-wired* state, which is not the
    /// real rule. A mapping that had quietly kept comparing the word against
    /// the private one could not produce this result.
    func testTheToneOnEachRowIsTheSharedTablesAnswerAndNotAReadingOfTheWord() {
        let rendered = rows(
            evidence: evidence(tools: [
                "claude": RoutingToolRow(installed: true, wired: true),
                "codex": RoutingToolRow(installed: true, wired: false),
            ])
        )
        XCTAssertEqual(rendered[0].tone, .neutral)
        XCTAssertEqual(rendered[1].tone, .clear)
        XCTAssertEqual(rendered[2].tone, .neutral)
    }

    /// The wiring numbering is the ABI's, not this enum's declaration order
    /// by accident. A renumbering would send "wired" across as "not wired" --
    /// a wrong verdict on a privacy claim, not a crash.
    func testTheWiringNumberingIsTheOneTheAbiSpells() {
        XCTAssertEqual(RoutingToolWiring.wired.abiValue, 0)
        XCTAssertEqual(RoutingToolWiring.notWired.abiValue, 1)
        XCTAssertEqual(RoutingToolWiring.unknown.abiValue, 2)
    }

    /// No styling decision on this surface reads the rendered word.
    ///
    /// Asserted on the source, because "this does not compare a string" is a
    /// fact a later edit reintroduces silently. `Private` is a substring of
    /// the denial that must never come back, and a comparison against a
    /// privacy claim is the same shape that once let `contains("reachable")`
    /// match `"unreachable"` on this surface.
    func testNoStylingDecisionReadsTheRenderedWord() {
        let source = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TCShellCore/RoutingSurface.swift")
        guard let text = try? String(contentsOf: source, encoding: .utf8) else {
            XCTFail("the surface's source was not found at \(source.path)")
            return
        }
        // Comments quote the rule; nothing in one is executed.
        let code = text
            .split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            .joined(separator: "\n")

        for comparison in [
            "wordPrivate", "word ==", "word !=", "word.contains", "word.hasPrefix",
        ] {
            XCTAssertFalse(
                code.contains(comparison),
                "a styling decision reads the rendered word: \(comparison)"
            )
        }
        XCTAssertTrue(
            code.contains("calls.toolTone(sourceMode, wiring.abiValue)"),
            "the tone must come from the shared branch table"
        )
    }

    // MARK: - Reading the tools answer

    func testTheToolsAnswerIsReadRowByRow() {
        let evidence = RoutingEvidence.parse([
            "outcome": "reachable",
            "tools": [
                ["id": "claude", "installed": true, "wired": true],
                ["id": "codex", "installed": true, "wired": false],
            ],
        ])
        XCTAssertEqual(evidence.outcome, .reachable)
        XCTAssertEqual(evidence.tools["claude"], RoutingToolRow(installed: true, wired: true))
        XCTAssertEqual(evidence.tools["codex"], RoutingToolRow(installed: true, wired: false))
        XCTAssertNil(evidence.tools["gemini"])
        XCTAssertNil(evidence.tools["cline"])
    }

    /// A missing `wired` is not a claim that a tool is wired, and a row
    /// without an id is not a row.
    func testAnUnreadableRowDegradesRatherThanDefaultsToAVerdict() {
        let evidence = RoutingEvidence.parse([
            "outcome": "reachable",
            "tools": [
                ["id": "claude", "installed": true],
                ["installed": true, "wired": true],
                ["id": "", "wired": true],
            ],
        ])
        XCTAssertEqual(evidence.tools["claude"], RoutingToolRow(installed: true, wired: false))
        XCTAssertEqual(evidence.tools.count, 1)
    }

    /// An answer that reached but listed nothing is `reachable` with no
    /// rows: the proxy did answer, and an empty list is exactly the right
    /// amount of evidence about every tool -- none.
    func testAnAnswerThatListedNothingIsStillAnAnswer() {
        let evidence = RoutingEvidence.parse(["outcome": "reachable", "tools": []])
        XCTAssertEqual(evidence.outcome, .reachable)
        XCTAssertTrue(evidence.tools.isEmpty)
    }

    // MARK: - The declaration

    /// The port field shows the conventional number so nobody has to know
    /// it, and that is all it does until somebody acts.
    func testTheFormShowsTheConventionalPortWhenNothingIsDeclared() {
        let form = RoutingForm.fromDeclaration(mode: nil, port: nil, tokenDir: nil)
        XCTAssertFalse(form.on)
        XCTAssertEqual(form.port, RoutingForm.conventionalPort)
        XCTAssertEqual(form.tokenDir, "")
    }

    /// The displayed default is not a declaration. Off writes `null`,
    /// whatever number is in the field.
    func testADisplayedDefaultNeverBecomesADeclaration() {
        let params = RoutingSurface.settingsParams(
            RoutingForm(on: false, port: RoutingForm.conventionalPort, tokenDir: "")
        )
        XCTAssertEqual(Array(params.keys), ["ironwire"])
        XCTAssertTrue(params["ironwire"] is NSNull, "off must be spelled null, not omitted")
    }

    func testTurningItOnDeclaresTheModeAndThePortInTheField() {
        let params = RoutingSurface.settingsParams(
            RoutingForm(on: true, port: 9001, tokenDir: "")
        )
        let declaration = params["ironwire"] as? [String: Any]
        XCTAssertEqual(declaration?["mode"] as? String, "watch")
        XCTAssertEqual(declaration?["port"] as? Int, 9001)
    }

    /// An empty folder box is left out rather than sent as an empty string:
    /// the daemon refuses an empty string outright, and absence is what
    /// falls back to the conventional location.
    func testAnEmptyFolderBoxIsLeftOutRatherThanSentEmpty() {
        for blank in ["", "   ", "\n\t "] {
            let params = RoutingSurface.settingsParams(
                RoutingForm(on: true, port: 8463, tokenDir: blank)
            )
            let declaration = params["ironwire"] as? [String: Any]
            XCTAssertEqual(
                declaration?.keys.sorted(), ["mode", "port"],
                "a blank folder became a declaration: \(blank.debugDescription)"
            )
        }
    }

    func testANamedFolderIsSentTrimmed() {
        let params = RoutingSurface.settingsParams(
            RoutingForm(on: true, port: 8463, tokenDir: "  /Users/x/ironwire  ")
        )
        let declaration = params["ironwire"] as? [String: Any]
        XCTAssertEqual(declaration?["token_dir"] as? String, "/Users/x/ironwire")
    }

    /// The probe is asked about the same port and folder the declaration
    /// carried, under the same rule about the empty box.
    func testTheProbeIsAskedAboutWhatWasDeclared() {
        let params = RoutingSurface.probeParams(RoutingForm(on: true, port: 9001, tokenDir: " "))
        XCTAssertEqual(params.keys.sorted(), ["port"])
        XCTAssertEqual(params["port"] as? Int, 9001)

        let withDir = RoutingSurface.probeParams(
            RoutingForm(on: true, port: 9001, tokenDir: "/Users/x/ironwire")
        )
        XCTAssertEqual(withDir["token_dir"] as? String, "/Users/x/ironwire")
    }

    /// A declaration the daemon is holding fills the fields, so a refresh
    /// shows what is actually declared rather than the default.
    func testADeclarationTheDaemonHoldsFillsTheFields() {
        let form = RoutingForm.fromDeclaration(
            mode: "watch", port: 9001, tokenDir: "/Users/x/ironwire"
        )
        XCTAssertTrue(form.on)
        XCTAssertEqual(form.port, 9001)
        XCTAssertEqual(form.tokenDir, "/Users/x/ironwire")
    }

    /// `mode: off` is a declaration that the proxy is not used, and it is
    /// not the same thing as `watch`.
    func testAnOffDeclarationIsNotOn() {
        let form = RoutingForm.fromDeclaration(mode: "off", port: nil, tokenDir: nil)
        XCTAssertFalse(form.on)
        XCTAssertEqual(form.port, RoutingForm.conventionalPort)
    }
}
