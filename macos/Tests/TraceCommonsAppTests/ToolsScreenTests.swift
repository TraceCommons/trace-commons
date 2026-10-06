import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Tools screen (#1030 `tool-screens.tsx` W-4): the Folders rows in
/// compact form, plus the "add your tool" tile. A picked folder is described
/// by the core and kept as every match it reports: one re-points that kind,
/// more than one asks which, none is refused. Read from the screen's pure
/// layout and its source, the house pattern for a SwiftUI view.
final class ToolsScreenTests: XCTestCase {
    private static func source(_ name: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/\(name)")
        return try String(contentsOf: url, encoding: .utf8)
    }

    private static func candidate(_ kind: SourceKind, exists: Bool) -> SourceCandidate {
        SourceCandidate(
            source: kind,
            path: "/Users/someone/.\(kind.rawValue)",
            exists: exists,
            sessionCount: exists ? 12 : 0,
            mostRecent: nil,
            relocatedByEnv: false
        )
    }

    /// One `tc_describe_folder` row per slug, all for the same picked folder.
    private static func describe(_ slugs: [String], path: String) -> String {
        let rows = slugs.map { slug in
            """
            {"source":"\(slug)","path":"\(path)","exists":true,"session_count":4,\
            "most_recent":null,"relocated_by_env":false,"answers_at":null}
            """
        }
        return "[" + rows.joined(separator: ",") + "]"
    }

    private func firstRunCopy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    func test_aRecognisedFolderRepointsItsKind() throws {
        let copy = try firstRunCopy()
        let path = "/Volumes/work/codex-home"
        let outcome = ToolsScreenLayout.outcome(
            path: path, json: Self.describe(["codex"], path: path))
        XCTAssertEqual(outcome, .added(AddedFolder(kind: .source(.codex), path: path)))

        // A missing Codex answered "I don't use it" is re-pointed, not kept off.
        let discovered = [Self.candidate(.claudeCode, exists: true), Self.candidate(.codex, exists: false)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.answer(.codex, .off)
        XCTAssertTrue(ToolsScreenLayout.apply(outcome, to: &state))
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: path))
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.codex), path: path)])

        // Its row is now a found tool at the added folder, marked as the
        // person's, with no "Get Codex" on it.
        let rows = ToolsScreenLayout.rows(discovered, state: state)
        let codex = try XCTUnwrap(rows.first { $0.source == .codex })
        XCTAssertTrue(codex.exists)
        XCTAssertEqual(codex.path, path)
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(codex, installURL: URL(string: "https://example.com"), in: state))
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: codex), .watch)
        XCTAssertEqual(ToolsScreenLayout.meta(for: codex, in: state, discovered: discovered, copy: copy, now: Date()), copy.tools.addedByYou)

        // A found, un-added row is compact: the session count alone.
        let claude = try XCTUnwrap(rows.first { $0.source == .claudeCode })
        XCTAssertEqual(
            ToolsScreenLayout.meta(for: claude, in: state, discovered: discovered, copy: copy, now: Date()),
            copy.frame.sessionCount.replacingOccurrences(of: "{count}", with: "12"))

        // A kind discovery did not offer still gets its row once added.
        let opencode = "/Users/someone/exports"
        XCTAssertTrue(
            ToolsScreenLayout.apply(
                ToolsScreenLayout.outcome(path: opencode, json: Self.describe(["opencode"], path: opencode)),
                to: &state))
        XCTAssertEqual(
            ToolsScreenLayout.rows(discovered, state: state).map(\.source), [.claudeCode, .codex, .opencode])

        // A trajectory export is declared as `trajectory_source`, not as a tool.
        let runs = "/Users/someone/runs"
        XCTAssertTrue(
            ToolsScreenLayout.apply(
                ToolsScreenLayout.outcome(path: runs, json: Self.describe(["trajectory"], path: runs)),
                to: &state))
        XCTAssertEqual(state.sessionRoots.trajectory, .watch(path: runs))
        XCTAssertEqual(ToolsScreenLayout.trajectoryFolder(in: state)?.path, runs)

        // The screen reads the core's description, decoded so the trajectory
        // row is kept, and writes only through the state's own mutators.
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("TCDiscovery.describeFolderJSON("))
        XCTAssertTrue(screen.contains("GlassToolTile(.folder"))
        XCTAssertTrue(screen.contains("FolderPanel.choose()"))
        XCTAssertTrue(screen.contains(".onDrop("))
        XCTAssertTrue(screen.contains(".commit(.leaveRoots)"))
        XCTAssertTrue(screen.contains("FolderMatch.decodeList("))
        XCTAssertFalse(screen.contains("SourceCandidate.decodeList("))
        XCTAssertFalse(screen.contains(".toolAnswers["))
        XCTAssertFalse(screen.contains(".addedFolders.append"))
    }

    func test_anAmbiguousFolderAsksWhichTool() throws {
        let path = "/Users/someone/exports"
        let outcome = ToolsScreenLayout.outcome(
            path: path, json: Self.describe(["opencode", "trajectory"], path: path))
        XCTAssertEqual(outcome, .ask(path: path, kinds: [.source(.opencode), .trajectory]))

        // Asking writes nothing.
        var state = FirstRunState(tier: .custom, step: .tools)
        XCTAssertTrue(ToolsScreenLayout.apply(outcome, to: &state))
        XCTAssertEqual(state, FirstRunState(tier: .custom, step: .tools))

        // The person's choice is what gets added.
        ToolsScreenLayout.choose(.trajectory, path: path, in: &state)
        XCTAssertEqual(state.sessionRoots.trajectory, .watch(path: path))
        XCTAssertEqual(state.sessionRoots.opencode, .undecided)

        // A match this build cannot name still makes it a question, though
        // only the nameable kind can be offered.
        XCTAssertEqual(
            ToolsScreenLayout.outcome(path: path, json: Self.describe(["opencode", "some-future-tool"], path: path)),
            .ask(path: path, kinds: [.source(.opencode)]))

        // Each option is named from the core, and the question is the
        // core's, naming the folder.
        let tools = try firstRunCopy().tools
        XCTAssertEqual(ToolsScreenLayout.name(.source(.opencode), copy: tools), SourceKind.opencode.displayName)
        XCTAssertEqual(ToolsScreenLayout.name(.trajectory, copy: tools), tools.trajectoryLabel)
        XCTAssertEqual(
            ToolsScreenLayout.question(path: path, copy: tools),
            tools.whichKind.replacingOccurrences(of: "{folder}", with: "exports"))
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("GlassPicker("))
        XCTAssertTrue(screen.contains("ToolsScreenLayout.question("))
        XCTAssertFalse(screen.contains("\"Trajectory\""))
    }

    /// A folder answered wrongly can be answered again: picked once as a
    /// trajectory export, then again as OpenCode, it is declared once.
    func test_anAmbiguousFolderCanBeAnsweredAgain() {
        let path = "/Users/someone/exports"
        var state = FirstRunState(tier: .custom, step: .tools)
        ToolsScreenLayout.choose(.trajectory, path: path, in: &state)
        ToolsScreenLayout.choose(.source(.opencode), path: path, in: &state)
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.opencode), path: path)])
        XCTAssertNotEqual(state.sessionRoots.trajectory, .watch(path: path))
        XCTAssertNil(ToolsScreenLayout.trajectoryFolder(in: state))
    }

    /// Kristi's #1235 M2: the ambiguous folder's picker offers the core's
    /// "Neither" after the kinds. It closes the question without adding the
    /// folder as either, so Continue is no longer held by it.
    func test_neitherDismissesAnAmbiguousFolder() throws {
        let tools = try firstRunCopy().tools
        let path = "/Users/someone/exports"
        let kinds: [AddedFolder.Kind] = [.source(.opencode), .trajectory]
        let options = ToolsScreenLayout.pendingOptions(kinds, copy: tools)
        XCTAssertEqual(
            options.map(\.title),
            [SourceKind.opencode.displayName, tools.trajectoryLabel, tools.neither])
        XCTAssertEqual(options.map(\.value), [.kind(0), .kind(1), .neither])

        let discovered = [Self.candidate(.claudeCode, exists: true)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        let before = state
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: true, isCommitting: false))

        XCTAssertTrue(ToolsScreenLayout.answer(.neither, path: path, kinds: kinds, in: &state))
        XCTAssertEqual(state, before, "Neither attributes the folder to no tool")
        XCTAssertTrue(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: false, isCommitting: false))

        // A kind is still added as before; an index the picker never offered
        // keeps the question open.
        XCTAssertTrue(ToolsScreenLayout.answer(.kind(1), path: path, kinds: kinds, in: &state))
        XCTAssertEqual(state.sessionRoots.trajectory, .watch(path: path))
        var untouched = before
        XCTAssertFalse(ToolsScreenLayout.answer(.kind(7), path: path, kinds: kinds, in: &untouched))
        XCTAssertEqual(untouched, before)

        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.pendingOptions("))
        XCTAssertTrue(screen.contains("ToolsScreenLayout.answer("))
    }

    /// The trajectory row is withdrawn with "I don't use it" on its picker.
    func test_theTrajectoryRowCanBeWithdrawn() throws {
        var state = FirstRunState(tier: .custom, step: .tools)
        ToolsScreenLayout.choose(.trajectory, path: "/t", in: &state)
        ToolsScreenLayout.selectTrajectory(.watch, in: &state)
        XCTAssertEqual(state.sessionRoots.trajectory, .watch(path: "/t"))
        ToolsScreenLayout.selectTrajectory(nil, in: &state)
        XCTAssertEqual(state.sessionRoots.trajectory, .watch(path: "/t"), "the placeholder answers nothing")
        ToolsScreenLayout.selectTrajectory(.dontUse, in: &state)
        XCTAssertNil(ToolsScreenLayout.trajectoryFolder(in: state))
        XCTAssertEqual(state.sessionRoots.trajectory, .undecided)
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.selectTrajectory("))
    }

    /// An open question holds Continue, as does a commit.
    func test_anOpenQuestionHoldsContinue() throws {
        let discovered = [Self.candidate(.claudeCode, exists: true)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.answer(.claudeCode, .watch(path: "/c"))
        state.answer(.codex, .off)
        XCTAssertTrue(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: false, isCommitting: false))
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: true, isCommitting: false))
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: false, isCommitting: true))
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(discovered: nil, state: state, pending: false, isCommitting: false))
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.canContinue("))
    }

    /// A described folder lands only while the screen is Tools and nothing is
    /// committing: a drop's answer arrives later, and a folder taken in after
    /// the start snapshot would show as added but never be watched.
    func test_aFolderArrivingDuringACommitOrAfterToolsIsIgnored() throws {
        XCTAssertTrue(ToolsScreenLayout.acceptsFolder(step: .tools, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.acceptsFolder(step: .tools, isCommitting: true))
        XCTAssertFalse(ToolsScreenLayout.acceptsFolder(step: .rules, isCommitting: false))
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.acceptsFolder("))
        // The core walks the folder off the main actor.
        XCTAssertTrue(screen.contains("Task.detached"))
    }

    func test_anUnrecognisedFolderIsRefused() throws {
        let copy = try firstRunCopy()
        let path = "/Users/someone/Documents"
        var state = FirstRunState(tier: .custom, step: .tools)
        let before = state

        for json in ["[]", nil, "not json", Self.describe(["some-future-tool"], path: path)] as [String?] {
            let outcome = ToolsScreenLayout.outcome(path: path, json: json)
            XCTAssertEqual(outcome, .refused, "\(json ?? "nil")")
            XCTAssertFalse(ToolsScreenLayout.apply(outcome, to: &state))
            XCTAssertEqual(state, before)
        }
        XCTAssertEqual(ToolsScreenLayout.refusal(copy.tools), copy.tools.addToolRefused)
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.refusal("))
    }

    func test_theCaptionNamesNoUnsupportedTool() throws {
        let tools = try firstRunCopy().tools
        for text in [tools.addTool, tools.addToolCaption] {
            XCTAssertFalse(text.contains("Theia"), text)
            XCTAssertFalse(text.contains("SSH"), text)
        }
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("copy.tools.addTool"))
        XCTAssertTrue(screen.contains("copy.tools.addToolCaption"))
        XCTAssertFalse(screen.contains("Theia"))
        XCTAssertFalse(screen.contains("SSH"))
    }

    /// A row's own folder button writes through `answer`, which drops the
    /// added folder. The row must stay, at the new path, so no watch goes to
    /// the daemon that the screen does not show.
    func test_aRowsChosenFolderKeepsItsRow() throws {
        let copy = try firstRunCopy()
        var state = FirstRunState(tier: .custom, step: .tools)
        ToolsScreenLayout.apply(.added(AddedFolder(kind: .source(.opencode), path: "/a")), to: &state)
        let row = try XCTUnwrap(ToolsScreenLayout.rows([], state: state).first { $0.source == .opencode })
        ToolAnswerRowLayout.choose(folder: "/b", for: row, in: &state)
        XCTAssertEqual(state.sessionRoots.opencode, .watch(path: "/b"))
        let rows = ToolsScreenLayout.rows([], state: state)
        XCTAssertEqual(rows.map(\.source), [.opencode])
        XCTAssertEqual(rows.first?.path, "/b")
        XCTAssertEqual(
            ToolsScreenLayout.meta(for: rows[0], in: state, discovered: [], copy: copy, now: Date()),
            copy.tools.addedByYou)

        // Discovered as missing: the row stays found at the watched path, with
        // no install line beside a folder it is watching.
        let missing = Self.candidate(.codex, exists: false)
        state.answer(.codex, .watch(path: "/c"))
        let codex = try XCTUnwrap(ToolsScreenLayout.rows([missing], state: state).first { $0.source == .codex })
        XCTAssertTrue(codex.exists)
        XCTAssertEqual(codex.path, "/c")
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(codex, installURL: URL(string: "https://example.com"), in: state))
    }

    func test_aLaterOffDropsTheAddedFolder() throws {
        let path = "/Users/someone/exports"
        var state = FirstRunState(tier: .custom, step: .tools)
        XCTAssertTrue(
            ToolsScreenLayout.apply(
                ToolsScreenLayout.outcome(path: path, json: Self.describe(["opencode"], path: path)),
                to: &state))
        let row = try XCTUnwrap(
            ToolsScreenLayout.rows([], state: state).first { $0.source == .opencode })

        // "I don't use it" on the row drops the folder: no watch survives it.
        ToolAnswerRowLayout.select(.dontUse, for: row, in: &state)
        XCTAssertEqual(state.addedFolders, [])
        XCTAssertEqual(state.toolAnswers[.opencode], .off)
        XCTAssertEqual(state.sessionRoots.opencode, .off)

        // And adding again replaces the row's "off".
        XCTAssertTrue(
            ToolsScreenLayout.apply(.added(AddedFolder(kind: .source(.opencode), path: path)), to: &state))
        XCTAssertNil(state.toolAnswers[.opencode])
        XCTAssertEqual(state.sessionRoots.opencode, .watch(path: path))
    }
}
