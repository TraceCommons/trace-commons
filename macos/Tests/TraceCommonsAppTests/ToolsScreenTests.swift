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
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(codex))
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: codex), .watch)
        XCTAssertEqual(ToolsScreenLayout.meta(for: codex, in: state, discovered: discovered, copy: copy.tools, now: Date()), copy.tools.addedByYou)

        // A found, un-added row is compact: the session count alone.
        let claude = try XCTUnwrap(rows.first { $0.source == .claudeCode })
        XCTAssertEqual(
            ToolsScreenLayout.meta(for: claude, in: state, discovered: discovered, copy: copy.tools, now: Date()),
            copy.tools.sessionCount.replacingOccurrences(of: "{count}", with: "12"))

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
        XCTAssertTrue(screen.contains("NSOpenPanel"))
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

        // Each option is named; the question is asked with a picker.
        XCTAssertEqual(ToolsScreenLayout.name(.source(.opencode)), SourceKind.opencode.displayName)
        XCTAssertFalse(ToolsScreenLayout.name(.trajectory).isEmpty)
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("GlassPicker("))
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
            ToolsScreenLayout.meta(for: rows[0], in: state, discovered: [], copy: copy.tools, now: Date()),
            copy.tools.addedByYou)

        // Discovered as missing: the row stays found at the watched path, with
        // no install line beside a folder it is watching.
        let missing = Self.candidate(.codex, exists: false)
        state.answer(.codex, .watch(path: "/c"))
        let codex = try XCTUnwrap(ToolsScreenLayout.rows([missing], state: state).first { $0.source == .codex })
        XCTAssertTrue(codex.exists)
        XCTAssertEqual(codex.path, "/c")
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(codex))
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
