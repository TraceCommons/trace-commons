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

    /// Sentences that report which one was asked for and what argument it
    /// got. The real ones are assembled in Rust and cross the ABI; this
    /// target does not link it.
    private func sentences() -> RoutingSentences {
        RoutingSentences(
            tokenLine: { path in path.map { "L-TOKEN:\($0)" } ?? "L-TOKEN:none" },
            unreachableLine: { port in port.map { "L-UNREACHABLE:\($0)" } ?? "L-UNREACHABLE:none" }
        )
    }

    /// Sentences the ABI refused to produce. Nil is a real answer from the
    /// bridge -- it is what a caught panic looks like -- and this surface has
    /// to have somewhere to go when it happens.
    private func silentSentences() -> RoutingSentences {
        RoutingSentences(tokenLine: { _ in nil }, unreachableLine: { _ in nil })
    }

    // MARK: - The probe result: three outcomes, three strings

    func testAReachableProbeSaysTheProbeReachedIt() {
        XCTAssertEqual(
            RoutingSurface.probeLine(.reachable, copy: copy(), sentences: sentences()),
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
            .tokenUnusable(path: path), copy: copy(), sentences: sentences()
        )
        XCTAssertTrue(line.contains(path), "the reported path did not survive: \(line)")
    }

    /// Nothing resolved at all is a different sentence, not an empty path.
    /// A line reading "could not use the file at " is worse than one that
    /// admits it does not know where to look.
    func testAnUnusableTokenWithNoPathIsItsOwnSentence() {
        let named = RoutingSurface.probeLine(
            .tokenUnusable(path: "/Users/someone/.ironwire/control.token"),
            copy: copy(), sentences: sentences()
        )
        let unnamed = RoutingSurface.probeLine(
            .tokenUnusable(path: nil), copy: copy(), sentences: sentences()
        )
        XCTAssertNotEqual(named, unnamed)
        XCTAssertEqual(unnamed, "L-TOKEN:none")
    }

    func testAnUnreachableProbeNamesThePortThatWasTried() {
        XCTAssertEqual(
            RoutingSurface.probeLine(
                .unreachable(port: 8463), copy: copy(), sentences: sentences()
            ),
            "L-UNREACHABLE:8463"
        )
    }

    /// No port tried must not become "port 0". Port 0 is the ask-the-kernel
    /// sentinel, and the daemon refuses it outright.
    func testNoPortTriedIsNotRenderedAsPortZero() {
        let line = RoutingSurface.probeLine(
            .unreachable(port: nil), copy: copy(), sentences: sentences()
        )
        XCTAssertEqual(line, "L-UNREACHABLE:none")
        XCTAssertFalse(line.contains("0"), line)
    }

    /// An outcome this build cannot read claims nothing about the proxy in
    /// either direction, and must not send anyone to check a port or a file
    /// that is fine.
    func testAnUnreadableOutcomeSaysTheCheckCouldNotBeRun() {
        XCTAssertEqual(
            RoutingSurface.probeLine(.unknown, copy: copy(), sentences: sentences()),
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
                RoutingSurface.probeLine(outcome, copy: copy(), sentences: silentSentences()),
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

    func testTheThreeDaemonStatesEachHaveTheirOwnLine() {
        XCTAssertEqual(RoutingSurface.stateLine("not_declared", copy: copy()), copy().stateOff)
        XCTAssertEqual(RoutingSurface.stateLine("awaiting_rows", copy: copy()), copy().stateWaiting)
        XCTAssertEqual(RoutingSurface.stateLine("rows_seen", copy: copy()), copy().stateReading)
    }

    /// A state a later daemon grows says what the off state says: it claims
    /// nothing.
    func testAStateThisBuildDoesNotKnowClaimsNothing() {
        XCTAssertEqual(RoutingSurface.stateLine("some_new_state", copy: copy()), copy().stateOff)
        XCTAssertEqual(RoutingSurface.stateLine("", copy: copy()), copy().stateOff)
    }

    /// `awaiting_rows` is not a fault. A reader built a moment ago starts
    /// empty by construction, so this is the state a contributor sees
    /// immediately after changing anything here -- painting it as an error
    /// would accuse a working proxy of being broken at that exact moment.
    func testWaitingForRowsIsNotAFault() {
        XCTAssertEqual(RoutingSurface.tone(forState: "awaiting_rows"), .held)
        let line = RoutingSurface.stateLine("awaiting_rows", copy: copy())
        XCTAssertNotEqual(line, copy().checkUnavailable)
        XCTAssertNotEqual(line, copy().stateOff)
        XCTAssertEqual(line, copy().stateWaiting)
    }

    func testTheOtherTwoStatesKeepTheirOwnTone() {
        XCTAssertEqual(RoutingSurface.tone(forState: "rows_seen"), .clear)
        XCTAssertEqual(RoutingSurface.tone(forState: "not_declared"), .neutral)
        XCTAssertEqual(RoutingSurface.tone(forState: "some_new_state"), .neutral)
    }

    /// "Last checked" is a per-process stamp on the running daemon, so it is
    /// only shown where it says something. On a state that has had no answer
    /// at all it would read as an install date or a connected-since, which is
    /// what it is not.
    func testTheLastCheckedStampIsWithheldOnAStateThatNeverAnswered() {
        XCTAssertFalse(RoutingSurface.showsLastChecked(forState: "not_declared"))
        XCTAssertFalse(RoutingSurface.showsLastChecked(forState: "some_new_state"))
        XCTAssertTrue(RoutingSurface.showsLastChecked(forState: "awaiting_rows"))
        XCTAssertTrue(RoutingSurface.showsLastChecked(forState: "rows_seen"))
    }

    // MARK: - Per-tool words, from the tools answer and not the switch

    private func evidence(
        outcome: RoutingProbeOutcome = .reachable,
        tools: [String: RoutingToolRow] = [:]
    ) -> RoutingEvidence {
        RoutingEvidence(outcome: outcome, tools: tools)
    }

    func testAToolIronWireCallsWiredGetsTheWiredWord() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch"),
            evidence: evidence(tools: ["claude": RoutingToolRow(installed: true, wired: true)]),
            copy: copy()
        )
        XCTAssertEqual(rows.first?.name, copy().toolClaude)
        XCTAssertEqual(rows.first?.word, copy().wordPrivate)
    }

    func testAToolIronWireListsButDoesNotCallWiredGetsTheNotWiredWord() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch"),
            evidence: evidence(tools: ["codex": RoutingToolRow(installed: true, wired: false)]),
            copy: copy()
        )
        XCTAssertEqual(rows[1].word, copy().wordDirect)
    }

    /// The whole reason this surface reads the tools answer. Declaring
    /// IronWire in this app says nothing about whether Codex is configured
    /// to send through it, and a shell that rendered one switch as three
    /// verdicts would be inventing two of them.
    func testTheDeclarationIsNotAnInputToAnyToolWord() {
        // Every tool in use, IronWire declared and reachable, and an answer
        // that listed nothing. Every word must still be "not known".
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "unset", gemini: "watch"),
            evidence: evidence(tools: [:]),
            copy: copy()
        )
        for row in rows {
            XCTAssertEqual(row.word, copy().wordUnknown, row.name)
        }
    }

    /// Gemini CLI has no row upstream at all -- neither built-in nor in
    /// IronWire's catalogue -- so it legitimately reads unknown on a machine
    /// where it is installed and in daily use. This is the case the old
    /// single-switch word got confidently wrong.
    func testGeminiReadsUnknownEvenWhenItIsInstalledAndInUse() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch"),
            evidence: evidence(tools: [
                "claude": RoutingToolRow(installed: true, wired: true),
                "codex": RoutingToolRow(installed: true, wired: true),
            ]),
            copy: copy()
        )
        XCTAssertEqual(rows[2].name, copy().toolGemini)
        XCTAssertEqual(rows[2].word, copy().wordUnknown)
        // And not because the row went missing: the two IronWire did answer
        // about are still verdicts.
        XCTAssertEqual(rows[0].word, copy().wordPrivate)
        XCTAssertEqual(rows[1].word, copy().wordPrivate)
    }

    /// Nothing answered is a stable state -- a port nothing listens on, a
    /// credential that is refused -- so a word built on it would keep
    /// asserting while the card underneath says nothing answered.
    func testNothingAnsweredLeavesEveryToolUnknownWhateverWasCached() {
        for outcome: RoutingProbeOutcome in [
            .unreachable(port: 8463), .tokenUnusable(path: "/Users/x/.ironwire/control.token"),
            .unknown,
        ] {
            let rows = RoutingSurface.toolRows(
                sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch"),
                evidence: evidence(
                    outcome: outcome,
                    tools: ["claude": RoutingToolRow(installed: true, wired: true)]
                ),
                copy: copy()
            )
            XCTAssertEqual(rows[0].word, copy().wordUnknown, "\(outcome)")
        }
    }

    /// No answer held at all is not a verdict either.
    func testNoEvidenceAtAllLeavesEveryToolUnknown() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch"),
            evidence: nil,
            copy: copy()
        )
        for row in rows {
            XCTAssertEqual(row.word, copy().wordUnknown, row.name)
        }
    }

    /// Only `off` means not used. `unset` watches the conventional location,
    /// which is a tool in use.
    func testOnlyAnOffSourceReadsAsNotUsed() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "off", codex: "unset", gemini: "off"),
            evidence: evidence(tools: [
                "claude": RoutingToolRow(installed: true, wired: true),
                "codex": RoutingToolRow(installed: true, wired: false),
            ]),
            copy: copy()
        )
        XCTAssertEqual(rows[0].word, copy().wordNotUsed)
        XCTAssertEqual(rows[1].word, copy().wordDirect)
        XCTAssertEqual(rows[2].word, copy().wordNotUsed)
    }

    /// IronWire saying a tool is not installed, while this app is watching
    /// that tool's sessions, is two detectors disagreeing about one machine.
    /// That is not evidence for a verdict.
    func testAToolIronWireSaysIsNotInstalledGetsNoVerdict() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "watch", codex: "watch", gemini: "watch"),
            evidence: evidence(tools: ["claude": RoutingToolRow(installed: false, wired: false)]),
            copy: copy()
        )
        XCTAssertEqual(rows[0].word, copy().wordUnknown)
    }

    /// The three rows are always all three, in one order, so a missing
    /// answer is a word rather than a vanished row.
    func testTheSurfaceAlwaysNamesAllThreeToolsInOneOrder() {
        let rows = RoutingSurface.toolRows(
            sourceModes: RoutingSourceModes(claude: "off", codex: "off", gemini: "off"),
            evidence: nil,
            copy: copy()
        )
        XCTAssertEqual(rows.map(\.name), [copy().toolClaude, copy().toolCodex, copy().toolGemini])
    }

    /// Only the wired word is painted as reassurance. Every other word is
    /// neutral -- including "not used", which is a preference and not an
    /// achievement.
    func testOnlyTheWiredWordIsPaintedAsReassurance() {
        XCTAssertEqual(RoutingSurface.tone(forWord: copy().wordPrivate, copy: copy()), .clear)
        for word in [copy().wordDirect, copy().wordUnknown, copy().wordNotUsed] {
            XCTAssertEqual(RoutingSurface.tone(forWord: word, copy: copy()), .neutral, word)
        }
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
