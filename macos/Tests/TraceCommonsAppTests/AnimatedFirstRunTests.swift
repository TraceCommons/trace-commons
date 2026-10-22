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

    /// The watched tools' traces are counted; points need a per-trace range
    /// in the one unit the daemon accepts, which only an invite's lookup
    /// gives, so without one the card makes up no figure.
    func test_tracesCountsTheWatchedToolsAndMakesUpNoRate() throws {
        let copy = try words()
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

        XCTAssertNil(TracesScreenLayout.points(total: 1365, lookup: nil, copy: copy.traces))
        XCTAssertEqual(TracesScreenLayout.rate(lookup: nil, copy: copy.traces), copy.traces.rateUnknown)
        XCTAssertFalse(copy.traces.rateUnknown.contains { $0.isNumber })

        XCTAssertEqual(TracesScreenLayout.counted(after: 0, reduceMotion: false), 0)
        XCTAssertEqual(TracesScreenLayout.counted(after: 3, reduceMotion: false), 1)
        XCTAssertEqual(TracesScreenLayout.counted(after: 0, reduceMotion: true), 1)
        XCTAssertEqual(FirstRunStageLayout.countEase(0.5), 0.875, accuracy: 0.001)
    }

    // MARK: Private AI

    /// The diagram shows the found tools Private AI can route today (Claude
    /// Code and Codex), with the vendors they answer at; the line counts the
    /// tools found here.
    func test_privateAIShowsTheToolsItCanRoute() throws {
        let copy = try words()
        let discovered = [
            Self.candidate(.codex, exists: true), Self.candidate(.claudeCode, exists: true),
            Self.candidate(.cline, exists: true), Self.candidate(.geminiCli, exists: false),
        ]
        let tools = PrivateAIScreenLayout.connectable(discovered)
        XCTAssertEqual(tools.map(\.source), [.claudeCode, .codex])
        XCTAssertEqual(tools.map(\.answersAt), ["Anthropic", "OpenAI"])
        XCTAssertEqual(PrivateAIScreenLayout.found(discovered), 3)
        XCTAssertEqual(PrivateAIScreenLayout.ofTotal(3, copy: copy.privateAi), "of 3 tools can connect")
        XCTAssertEqual(PrivateAIScreenLayout.ofTotal(1, copy: copy.privateAi), "of 1 tool can connect")
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 0.5, reduceMotion: false), 0)
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 0.95, reduceMotion: false), 1)
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 5, reduceMotion: false), 2)
        XCTAssertEqual(PrivateAIScreenLayout.counted(2, after: 0, reduceMotion: true), 2)
        XCTAssertEqual(PrivateAIScreenLayout.rowY(0, of: 1), 101, "one tool sits level with the N")
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
