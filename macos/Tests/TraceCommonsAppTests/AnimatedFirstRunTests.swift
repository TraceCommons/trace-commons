import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The animated first run's choreography and the screens it added (Ron's
/// 2026-10-21 canvas, rulings through 2026-10-22), read from their pure
/// layouts.
final class AnimatedFirstRunTests: XCTestCase {
    private func words() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private static func candidate(_ kind: SourceKind, exists: Bool, sessions: UInt64 = 12) -> SourceCandidate {
        SourceCandidate(
            source: kind, path: "/Users/someone/.\(kind.rawValue)", exists: exists,
            sessionCount: exists ? sessions : 0, mostRecent: nil, relocatedByEnv: false,
            answersAt: kind == .claudeCode ? "Anthropic" : kind == .codex ? "OpenAI" : nil)
    }

    // MARK: Welcome

    /// The N is drawn in one second after a short pause, with no pulse; the
    /// shine crosses it first just after the draw, then every 15 seconds,
    /// and rests off the mark between crossings.
    func test_welcomeDrawsTheMarkThenShinesEveryFifteenSeconds() {
        XCTAssertEqual(WelcomeScreenLayout.drawn(after: 0, reduceMotion: false), 0)
        XCTAssertEqual(WelcomeScreenLayout.drawn(after: 0.7, reduceMotion: false), 0.5, accuracy: 0.001)
        XCTAssertEqual(WelcomeScreenLayout.drawn(after: 1.2, reduceMotion: false), 1)
        XCTAssertEqual(WelcomeScreenLayout.drawn(after: 0, reduceMotion: true), 1, "Reduce Motion shows the mark whole")

        XCTAssertNil(WelcomeScreenLayout.shine(after: 1.4, reduceMotion: false))
        XCTAssertNotNil(WelcomeScreenLayout.shine(after: 2, reduceMotion: false))
        XCTAssertNil(WelcomeScreenLayout.shine(after: 3, reduceMotion: false), "off the mark between crossings")
        XCTAssertNotNil(WelcomeScreenLayout.shine(after: 16.6, reduceMotion: false))
        XCTAssertNil(WelcomeScreenLayout.shine(after: 2, reduceMotion: true))

        XCTAssertEqual(WelcomeScreenLayout.glow(after: 0, reduceMotion: false), 0)
        XCTAssertEqual(WelcomeScreenLayout.glow(after: 2, reduceMotion: false), 1, "the glow fades in while it draws")
        XCTAssertEqual(WelcomeScreenLayout.words("Welcome to Near AI"), ["Welcome", "to", "Near", "AI"])
        XCTAssertEqual(WelcomeScreenLayout.risen(after: 0, delay: 1, reduceMotion: false), 0)
        XCTAssertEqual(WelcomeScreenLayout.risen(after: 5, delay: 1, reduceMotion: false), 1, accuracy: 0.001)
    }

    /// Ron's words on Welcome, and Get started into Tools.
    func test_welcomeReadsRonsWords() throws {
        let welcome = try words().welcome
        XCTAssertEqual(welcome.title, "Welcome to Near AI")
        XCTAssertEqual(welcome.body, "Private AI solutions that put you in control of your data.")
        XCTAssertEqual(FirstRunNavigation.next(FirstRunState()).step, .tools)
    }

    // MARK: Uses

    /// On Ask the orb waits at the review gate while the gate fills green
    /// and the thumbs-up pops in white; on Automatic it passes straight
    /// through. Each arriving orb fills the next of five places, which then
    /// clear and fill again.
    func test_theUsesFlowWaitsAtTheGateOnAskAndFillsTheRing() {
        let waiting = UsesFlowLayout.orb(.askMe, loop: 0.5)
        XCTAssertEqual(waiting.along, 0.397, accuracy: 0.001, "held at the gate")
        XCTAssertEqual(UsesFlowLayout.gate(.askMe, loop: 0.5).fill, 1)
        XCTAssertEqual(UsesFlowLayout.gate(.askMe, loop: 0.5).white, 1)
        XCTAssertEqual(UsesFlowLayout.gate(.askMe, loop: 0.1), .init(fill: 0, scale: 0.86, turn: 0, white: 0))
        XCTAssertGreaterThan(UsesFlowLayout.gate(.askMe, loop: 0.36).scale, 1.2, "it pops in")
        let passing = UsesFlowLayout.orb(.automatic, loop: 0.5)
        XCTAssertGreaterThan(passing.along, 0.5, "Automatic does not wait")
        XCTAssertEqual(UsesFlowLayout.period(.automatic), 2.6)
        XCTAssertEqual(UsesFlowLayout.period(.askMe), 4.4)

        let first = UsesFlowLayout.delay(.askMe) + UsesFlowLayout.period(.askMe) * UsesFlowLayout.arrival(.askMe)
        XCTAssertEqual(UsesFlowLayout.satellites(.askMe, elapsed: first - 0.01), 0)
        XCTAssertEqual(UsesFlowLayout.satellites(.askMe, elapsed: first + 0.01), 1)
        XCTAssertEqual(UsesFlowLayout.satellites(.askMe, elapsed: first + 4.4 * 4 + 0.5), 5)
        XCTAssertEqual(UsesFlowLayout.satellites(.askMe, elapsed: first + 4.4 * 4 + 1.4), 0, "full, it clears")
        XCTAssertEqual(UsesFlowLayout.satellites(.askMe, elapsed: first + 4.4 * 5 + 0.1), 1, "and fills again")
        XCTAssertEqual(UsesFlowLayout.keyframes(0.5, [(0, 0), (1, 10)]), 5)
    }

    /// The gate reads "You review" on Ask and the mode table's Automatic on
    /// Automatic; This Mac and The commons are the core's.
    func test_theUsesGraphicIsWordedByTheCore() throws {
        let copy = try words()
        let modes = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        XCTAssertEqual(UsesScreenLayout.gateLabel(FirstRunState(), uses: copy.uses, modes: modes), copy.uses.youReview)
        XCTAssertEqual(
            UsesScreenLayout.gateLabel(FirstRunState(sharing: .automatic), uses: copy.uses, modes: modes),
            modes.label(for: .autoUpload))
        XCTAssertEqual(copy.uses.thisMac, "This Mac")
        XCTAssertEqual(copy.uses.theCommons, "The commons")
    }

    // MARK: Traces

    /// The watched tools' traces are counted, and the core's line names no
    /// amount, whatever an invite says (owner, 2026-11-04). Nothing counted
    /// yet, or nothing found, draws no line.
    func test_tracesShowsOneTotalInPointsOrInTheCommonsDollars() throws {
        let discovered = [
            Self.candidate(.claudeCode, exists: true, sessions: 953), Self.candidate(.codex, exists: true, sessions: 412),
            Self.candidate(.cline, exists: true, sessions: 37),
        ]
        var state = FirstRunState(step: .traces)
        state.recordDiscovery(discovered)
        state.answer(.claudeCode, .watch(path: "/a"))
        state.answer(.codex, .watch(path: "/b"))
        let rows = TracesScreenLayout.rows(state, discovered: discovered)
        XCTAssertEqual(rows.map(\.kind), [.claudeCode, .codex], "only watched tools count")
        XCTAssertEqual(TracesScreenLayout.total(rows), 1365)
        XCTAssertEqual(TracesScreenLayout.found(state, discovered: discovered), 1365)
        XCTAssertNil(TracesScreenLayout.found(state, discovered: nil), "not counted yet")

        let copy = try words()
        // One line, no amount, whatever an invite or the commons says
        // (owner, 2026-11-04: the MVP shows no figure).
        XCTAssertEqual(
            TracesScreenLayout.estimate(found: 1365, lookup: nil), copy.traces.foundCredit)
        let invite = try JSONDecoder().decode(
            DaemonData.InviteLookup.self,
            from: Data(#"{"valid":true,"credit_range":{"min":2,"max":6,"unit":"points_per_accepted_trace"}}"#.utf8))
        XCTAssertEqual(
            TracesScreenLayout.estimate(found: 1365, lookup: invite), copy.traces.foundCredit)
        for line in [copy.traces.foundCredit, copy.traces.foundCouldCredit] {
            XCTAssertFalse(line.contains("$"), line)
            XCTAssertFalse(line.contains("point"), line)
        }

        for found in [nil, 0] as [UInt64?] {
            XCTAssertNil(TracesScreenLayout.estimate(found: found, lookup: invite))
            XCTAssertNil(TracesScreenLayout.estimate(found: found, lookup: nil))
        }

        XCTAssertEqual(TracesScreenLayout.counted(after: 0, reduceMotion: false), 0)
        XCTAssertEqual(TracesScreenLayout.counted(after: 3, reduceMotion: false), 1)
        XCTAssertEqual(TracesScreenLayout.counted(after: 0, reduceMotion: true), 1)
        XCTAssertEqual(FirstRunStageLayout.countEase(0.5), 0.875, accuracy: 0.001)
    }

    /// The lines under the count are the daemon's `first_run_copy` answer
    /// (owner, 2026-11-04): the commons' lines, or the protocol's default
    /// lines it fell back to, in the same layout: the headline, and under
    /// it the note while the use that earns credit is on. Without an answer,
    /// the core's own line alone. Nothing found draws nothing. Synthetic
    /// commons lines.
    func test_tracesDrawsTheDaemonsLinesOrTheCoresLine() throws {
        let decode = { (json: String) in
            try JSONDecoder().decode(DaemonData.FirstRunCopyLines.self, from: Data(json.utf8))
        }
        let commons = try decode(
            #"{"found_credit":"SAMPLE line.","found_could_credit":"SAMPLE could line.","credit_depends":"SAMPLE note.","source":"commons"}"#)
        XCTAssertEqual(commons.source, "commons")
        let on = try XCTUnwrap(TracesScreenLayout.lines(found: 1365, served: commons, coreLine: "core", coreNote: "core note", earnsCredit: true))
        XCTAssertEqual(on.headline, "SAMPLE line.")
        XCTAssertEqual(on.note, "SAMPLE note.")
        let off = try XCTUnwrap(TracesScreenLayout.lines(found: 1365, served: commons, coreLine: "core", coreNote: "core note", earnsCredit: false))
        XCTAssertEqual(off.headline, "SAMPLE could line.")
        XCTAssertNil(off.note, "the no-credit note sits there instead")

        // The daemon's fallback: the protocol's default lines, drawn alike.
        let fallback = try decode(
            #"{"found_credit":"We have detected some traces that can earn you credit.","found_could_credit":"We have detected some traces that could earn you credit.","credit_depends":"Final credit depends on how many traces are accepted.","source":"fallback"}"#)
        let drawn = try XCTUnwrap(TracesScreenLayout.lines(found: 10, served: fallback, coreLine: nil, coreNote: "core note", earnsCredit: true))
        XCTAssertEqual(drawn.headline, "We have detected some traces that can earn you credit.")
        XCTAssertEqual(drawn.note, "Final credit depends on how many traces are accepted.")

        // A commons answer with no note draws the headline alone.
        let bare = try decode(#"{"found_credit":"SAMPLE line.","found_could_credit":"SAMPLE could line."}"#)
        XCTAssertNil(try XCTUnwrap(TracesScreenLayout.lines(found: 1, served: bare, coreLine: nil, coreNote: "core note", earnsCredit: true)).note)

        // No answer: the core's line, with its note while credit is on.
        let core = try XCTUnwrap(TracesScreenLayout.lines(
            found: 1365, served: nil, coreLine: "core line", coreNote: "core note", earnsCredit: true))
        XCTAssertEqual(core.headline, "core line")
        XCTAssertEqual(core.note, "core note")
        XCTAssertNil(try XCTUnwrap(TracesScreenLayout.lines(
            found: 1365, served: nil, coreLine: "core line", coreNote: "core note", earnsCredit: false)).note)
        XCTAssertNil(TracesScreenLayout.lines(found: 1365, served: nil, coreLine: nil, coreNote: "core note", earnsCredit: true))

        for found in [nil, 0] as [UInt64?] {
            XCTAssertNil(TracesScreenLayout.lines(found: found, served: commons, coreLine: "core", coreNote: "core note", earnsCredit: true))
        }
    }

    /// The size sent beside the count is the watched tools' session files'
    /// total, from discovery's `total_bytes`; a store not measured adds 0.
    func test_tracesSendsTheWatchedToolsTotalSize() throws {
        let rows = try SourceCandidate.decodeList(from: """
            [{"source":"claude-code","path":"/a","exists":true,"session_count":3,"most_recent":null,"total_bytes":3000,"relocated_by_env":false,"answers_at":"Anthropic"},
             {"source":"codex","path":"/b","exists":true,"session_count":2,"most_recent":null,"total_bytes":500,"relocated_by_env":false,"answers_at":"OpenAI"},
             {"source":"cline","path":"/c","exists":true,"session_count":9,"most_recent":null,"total_bytes":99999,"relocated_by_env":false,"answers_at":null},
             {"source":"opencode","path":"/d","exists":true,"session_count":4,"most_recent":null,"total_bytes":null,"relocated_by_env":false,"answers_at":null}]
            """)
        XCTAssertEqual(rows.map(\.totalBytes), [3000, 500, 99999, nil])
        var state = FirstRunState(step: .traces)
        state.recordDiscovery(rows)
        state.answer(.claudeCode, .watch(path: "/a"))
        state.answer(.codex, .watch(path: "/b"))
        state.answer(.opencode, .watch(path: "/d"))
        XCTAssertEqual(TracesScreenLayout.totalBytes(state, discovered: rows), 3500, "cline is not watched")
        XCTAssertEqual(TracesScreenLayout.totalBytes(state, discovered: nil), 0)
    }

    // MARK: Private AI

    /// The diagram includes all three persistent connection tools, with
    /// known default vendors only; the line counts the
    /// tools found here.
    func test_privateAIShowsTheToolsItCanRoute() throws {
        let copy = try words()
        let discovered = [
            Self.candidate(.codex, exists: true), Self.candidate(.claudeCode, exists: true),
            Self.candidate(.cline, exists: true), Self.candidate(.geminiCli, exists: false),
            Self.candidate(.opencode, exists: true),
        ]
        let tools = PrivateAIScreenLayout.connectable(discovered)
        XCTAssertEqual(tools.map(\.source), [.claudeCode, .codex, .opencode])
        XCTAssertEqual(tools.map(\.answersAt), ["Anthropic", "OpenAI", nil])
        XCTAssertEqual(PrivateAIScreenLayout.found(discovered), 4)
        XCTAssertEqual(PrivateAIScreenLayout.ofTotal(3, copy: copy.privateAi), "of 3 tools can connect")
        XCTAssertEqual(PrivateAIScreenLayout.ofTotal(1, copy: copy.privateAi), "of 1 tool can connect")
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 0.5, reduceMotion: false), 0)
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 0.95, reduceMotion: false), 1)
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 5, reduceMotion: false), 2)
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 0, reduceMotion: true), 2)
        XCTAssertEqual(PrivateAIScreenLayout.rowY(0, of: 1), 101, "one tool sits level with the N")
        XCTAssertEqual((0..<3).map { PrivateAIScreenLayout.rowY($0, of: 3) }, [21, 101, 181])
    }

    /// With no switch the diagram is what Private AI does (owner,
    /// 2026-10-27): every tool's route runs into the N and one line runs
    /// on to NEAR AI. No provider pills: a provider's pill on the N's row
    /// sat on NEAR AI's, OpenCode has none, and NEAR AI floated unjoined.
    @MainActor
    func test_everyToolRoutesThroughTheNToNearAI() throws {
        let screen = try MonitorNavigationTests.text("Views/FirstRun/PrivateAIScreen.swift")
        XCTAssertFalse(screen.contains("tool.answersAt"), "no provider pills")
        XCTAssertTrue(screen.contains("to: CGPoint(x: hub.x - 36, y: hub.y)"), "each tool into the N")
        XCTAssertTrue(screen.contains("path.addLine(to: CGPoint(x: destination - half, y: hub.y))"),
                      "the N on to NEAR AI's pill")
        XCTAssertLessThanOrEqual(
            PrivateAIScreenLayout.destinationX + PrivateAIScreenLayout.pillWidth / 2,
            FirstRunStageLayout.canvas.width, "NEAR AI stays on the canvas")
    }

    /// The step has no switch (owner, 2026-10-27: it changed little). It
    /// says what Private AI is, with Learn more; it never turns it on, so
    /// setup leaves Private AI as it was and Settings turns it on.
    @MainActor
    func test_thePrivateAIStepHasNoSwitch() throws {
        let screen = try MonitorNavigationTests.text("Views/FirstRun/PrivateAIScreen.swift")
        XCTAssertFalse(screen.contains("Toggle("))
        XCTAssertFalse(screen.contains("runner.state.privateAI = "))
        XCTAssertTrue(screen.contains("copy.privateAi.learnMore"))
    }

    // MARK: The stage

    /// The scan plays once per first run, and not at all under Reduce Motion.
    @MainActor
    func test_theScanPlaysOnce() async throws {
        let memory = FirstRunStageMemory()
        memory.startScan(reduceMotion: true)
        XCTAssertTrue(memory.toolsScanned)
        XCTAssertNil(memory.scanStartedAt)

        let moving = FirstRunStageMemory()
        moving.startScan(reduceMotion: false)
        let started = try XCTUnwrap(moving.scanStartedAt)
        moving.startScan(reduceMotion: false)
        XCTAssertEqual(moving.scanStartedAt, started, "a second start does not replay it")
        XCTAssertFalse(moving.toolsScanned)
    }

    /// Join's watched tiles circle the N: one on top, two either side,
    /// three as a triangle.
    func test_joinsTilesCircleTheMark() {
        XCTAssertEqual(FirstRunStageLayout.joinAngle(0, of: 1), -90)
        XCTAssertEqual([0, 1].map { FirstRunStageLayout.joinAngle($0, of: 2) }, [180, 0])
        XCTAssertEqual([0, 1, 2].map { FirstRunStageLayout.joinAngle($0, of: 3) }, [-90, 30, 150])
        XCTAssertEqual(FirstRunStageLayout.ringAngle(1, of: 4), 0)
        let top = FirstRunStageLayout.point(CGPoint(x: 10, y: 10), radius: 5, degrees: -90)
        XCTAssertEqual(top.x, 10, accuracy: 0.001)
        XCTAssertEqual(top.y, 5, accuracy: 0.001)
    }

    /// The marker check is a checkbox to assistive tech, in the native
    /// toggle's words, as `GlassCheckboxStyle` is.
    func test_theMarkerCheckSpeaksAsACheckbox() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/FirstRunStage.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("Toggle(isOn: configuration.$isOn) { configuration.label }"))
    }
}
