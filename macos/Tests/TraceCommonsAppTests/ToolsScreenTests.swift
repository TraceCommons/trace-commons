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

    /// Ron's review of #1235, item 3: a recognised folder is added as a row
    /// of its own, unanswered, named after the folder, with a folder tile
    /// and "Added by you", and the same picker and folder button as every
    /// other row. The matched tool's row is left as it was.
    func test_aRecognisedFolderIsAddedAsItsOwnUnansweredRow() throws {
        let copy = try firstRunCopy()
        let path = "/Volumes/work/codex-home"
        let outcome = ToolsScreenLayout.outcome(
            path: path, json: Self.describe(["codex"], path: path))
        XCTAssertEqual(outcome, .added(AddedFolder(kind: .source(.codex), path: path)))

        let discovered = [Self.candidate(.claudeCode, exists: true), Self.candidate(.codex, exists: true)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.answer(.codex, .off)
        XCTAssertTrue(ToolsScreenLayout.apply(outcome, to: &state))
        XCTAssertEqual(state.toolAnswers[.codex], .off, "the Codex row is not answered for the person")
        XCTAssertEqual(state.sessionRoots.codex, .off)

        // Codex's own row is untouched: discovery's folder, and no caption
        // (no session count, owner 2026-10-08).
        let rows = ToolsScreenLayout.rows(discovered, state: state)
        XCTAssertEqual(rows.map(\.source), [.claudeCode, .codex])
        let codex = try XCTUnwrap(rows.first { $0.source == .codex })
        XCTAssertEqual(codex.path, "/Users/someone/.codex")
        XCTAssertNil(ToolsScreenLayout.meta(for: codex, in: state, discovered: discovered, copy: copy))

        // The added folder's row: its folder's name, unanswered.
        let added = ToolsScreenLayout.addedRows(state)
        XCTAssertEqual(added.map(\.path), [path])
        XCTAssertEqual(ToolsScreenLayout.folderName(path), "codex-home")
        XCTAssertNil(ToolsScreenLayout.addedAnswer(added[0]))
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(
                discovered: discovered, state: { var answered = state; answered.answer(.claudeCode, .off); return answered }(),
                pending: false, isCommitting: false))

        ToolsScreenLayout.selectAdded(.watch, path: path, in: &state)
        XCTAssertEqual(ToolsScreenLayout.addedAnswer(ToolsScreenLayout.addedRows(state)[0]), .watch)
        XCTAssertEqual(state.sessionRoots.codex, .watch(path: path))
        // Codex's own row still shows its own answer and folder.
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: codex), .dontUse)
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: state, for: codex), "/Users/someone/.codex")
        ToolsScreenLayout.selectAdded(.dontUse, path: path, in: &state)
        XCTAssertEqual(state.sessionRoots.codex, .off)
        XCTAssertEqual(ToolsScreenLayout.addedRows(state).count, 1, "the row stays, answered")

        // A kind discovery did not offer is added the same way.
        let opencode = "/Users/someone/exports"
        XCTAssertTrue(
            ToolsScreenLayout.apply(
                ToolsScreenLayout.outcome(path: opencode, json: Self.describe(["opencode"], path: opencode)),
                to: &state))
        XCTAssertEqual(ToolsScreenLayout.addedRows(state).map(\.path), [path, opencode])
        XCTAssertEqual(ToolsScreenLayout.rows(discovered, state: state).map(\.source), [.claudeCode, .codex])

        // A trajectory export is declared as `trajectory_source`, not as a
        // tool, and keeps its row as built: it reads Watch.
        let runs = "/Users/someone/runs"
        XCTAssertTrue(
            ToolsScreenLayout.apply(
                ToolsScreenLayout.outcome(path: runs, json: Self.describe(["trajectory"], path: runs)),
                to: &state))
        XCTAssertEqual(state.sessionRoots.trajectory, .watch(path: runs))
        XCTAssertEqual(ToolsScreenLayout.trajectoryFolder(in: state)?.path, runs)
        XCTAssertFalse(ToolsScreenLayout.addedRows(state).contains { $0.path == runs })

        // The screen reads the core's description, decoded so the trajectory
        // row is kept, and writes only through the state's own mutators.
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("TCDiscovery.describeFolderJSON("))
        XCTAssertTrue(screen.contains("GlassToolTile(.folder"))
        XCTAssertTrue(screen.contains("FolderPanel.choose()"))
        XCTAssertTrue(screen.contains(".onDrop("))
        XCTAssertTrue(screen.contains(".commit(.leaveRoots)"))
        XCTAssertTrue(screen.contains("FolderMatch.decodeList("))
        XCTAssertTrue(screen.contains("copy.tools.addedByYou"))
        XCTAssertTrue(screen.contains("GlassFolderButton("))
        XCTAssertFalse(screen.contains("SourceCandidate.decodeList("))
        XCTAssertFalse(screen.contains(".toolAnswers["))
        XCTAssertFalse(screen.contains(".addedFolders.append"))
    }

    /// One tool watched in two rows holds Continue, and the added row says
    /// so in the core's words, naming the tool.
    func test_aToolWatchedTwiceIsSaidOnTheAddedRow() throws {
        let copy = try firstRunCopy()
        let discovered = [Self.candidate(.claudeCode, exists: true), Self.candidate(.codex, exists: true)]
        var state = FirstRunState(tier: .custom, step: .tools)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .watch(path: "/Users/someone/.codex"))
        ToolsScreenLayout.apply(.added(AddedFolder(kind: .source(.codex), path: "/v/codex")), to: &state)
        ToolsScreenLayout.selectAdded(.watch, path: "/v/codex", in: &state)
        let row = ToolsScreenLayout.addedRows(state)[0]
        XCTAssertEqual(
            ToolsScreenLayout.conflict(row, in: state, copy: copy.tools),
            copy.tools.oneFolderPerTool.replacingOccurrences(of: "{tool}", with: "Codex"))
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: false, isCommitting: false))
        ToolsScreenLayout.selectAdded(.dontUse, path: "/v/codex", in: &state)
        XCTAssertNil(ToolsScreenLayout.conflict(ToolsScreenLayout.addedRows(state)[0], in: state, copy: copy.tools))
        XCTAssertTrue(
            ToolsScreenLayout.canContinue(discovered: discovered, state: state, pending: false, isCommitting: false))
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
        // Quick's Folders takes folders too (owner, 2026-10-08).
        XCTAssertTrue(ToolsScreenLayout.acceptsFolder(step: .folders, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.acceptsFolder(step: .folders, isCommitting: true))
        XCTAssertFalse(ToolsScreenLayout.acceptsFolder(step: .uses, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.acceptsFolder(step: .join, isCommitting: false))
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.acceptsFolder("))
        // The core walks the folder off the main actor.
        XCTAssertTrue(screen.contains("Task.detached"))
    }

    /// Owner, 2026-10-08: Quick's Folders and Custom's Tools give the same
    /// add-tool component and behaviour; the cards scroll in the frame's
    /// body while the add tile is pinned under them, above the footer; and
    /// a card shows no session count and no "not found" line, its answer
    /// centred beside the tile and name.
    func test_bothToolScreensShareTheCardsAndThePinnedAddTile() throws {
        for file in ["FoldersScreen.swift", "ToolsScreen.swift"] {
            let screen = try Self.source(file)
            let content = try XCTUnwrap(screen.range(of: "} content: {"), file)
            let pinned = try XCTUnwrap(screen.range(of: "} pinned: {"), file)
            XCTAssertLessThan(content.lowerBound, pinned.lowerBound, file)
            let body = String(screen[content.upperBound..<pinned.lowerBound])
            XCTAssertTrue(body.contains("ToolList("), file)
            XCTAssertFalse(body.contains("AddToolTile("), "\(file): the add tile does not scroll")
            let pinnedBody = String(screen[pinned.upperBound...].prefix(200))
            XCTAssertTrue(pinnedBody.contains("AddToolTile(copy: copy, runner: runner, adding: adding)"), file)
            XCTAssertTrue(screen.contains("@StateObject private var adding = AddToolModel()"), file)
            XCTAssertTrue(screen.contains("ToolsScreenLayout.canContinue("), file)
            XCTAssertTrue(screen.contains("pending: adding.pending != nil"), file)
        }
        // One tile and one list, defined once.
        let tools = try Self.source("ToolsScreen.swift")
        XCTAssertEqual(tools.components(separatedBy: "struct AddToolTile: View").count - 1, 1)
        XCTAssertEqual(tools.components(separatedBy: "struct ToolList: View").count - 1, 1)
        XCTAssertFalse(try Self.source("FoldersScreen.swift").contains(".onDrop("))

        // No session count or evidence line on a card; the found tool reads
        // nothing, a missing one too.
        let copy = try firstRunCopy()
        let found = Self.candidate(.claudeCode, exists: true)
        let missing = Self.candidate(.codex, exists: false)
        let state = FirstRunState(tier: .quick, step: .folders)
        XCTAssertNil(ToolsScreenLayout.meta(for: found, in: state, discovered: [found, missing], copy: copy))
        XCTAssertNil(ToolsScreenLayout.meta(for: missing, in: state, discovered: [found, missing], copy: copy))
        for file in ["ToolAnswerRow.swift", "ToolsScreen.swift", "FoldersScreen.swift"] {
            let source = try Self.source(file)
            XCTAssertFalse(source.contains(".evidence(now:"), file)
            XCTAssertFalse(source.contains("copy.frame.sessionCount"), file)
        }
        // The answer is centred beside the tile and name, in one row.
        let row = try Self.source("ToolAnswerRow.swift")
        let card = try XCTUnwrap(row.range(of: "GlassCard {\n            HStack(alignment: .center"))
        let picker = try XCTUnwrap(row.range(of: "GlassPicker("))
        XCTAssertLessThan(card.lowerBound, picker.lowerBound)
        XCTAssertTrue(tools.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s6) {\n            GlassToolTile(.folder"))

        // The frame puts the pinned area between the scrolling cards and the
        // footer.
        let frame = try Self.source("FirstRunFrame.swift")
        let scroll = try XCTUnwrap(frame.range(of: "ScrollView {"))
        let pinnedSlot = try XCTUnwrap(frame.range(of: "                pinned\n                footerRow"))
        XCTAssertLessThan(scroll.lowerBound, pinnedSlot.lowerBound)
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

    /// An added row's folder button picks another folder for that row: it
    /// is described again and replaces the row, watched, as choosing a
    /// folder on any other row watches it. A tool row's folder button never
    /// touches an added row.
    func test_anAddedRowsFolderButtonReplacesItsFolder() throws {
        var state = FirstRunState(tier: .custom, step: .tools)
        ToolsScreenLayout.apply(.added(AddedFolder(kind: .source(.opencode), path: "/a")), to: &state)
        XCTAssertTrue(
            ToolsScreenLayout.replace(
                path: "/a", with: .added(AddedFolder(kind: .source(.opencode), path: "/b")), in: &state))
        XCTAssertEqual(state.addedFolders, [AddedFolder(kind: .source(.opencode), path: "/b", watched: true)])
        XCTAssertEqual(state.sessionRoots.opencode, .watch(path: "/b"))

        // A refused folder leaves the row as it was.
        XCTAssertFalse(ToolsScreenLayout.replace(path: "/b", with: .refused, in: &state))
        XCTAssertEqual(state.addedFolders.map(\.path), ["/b"])

        // A tool row's own folder writes only that tool's answer.
        let claude = Self.candidate(.claudeCode, exists: true)
        ToolAnswerRowLayout.choose(folder: "/c", for: claude, in: &state)
        XCTAssertEqual(state.addedFolders.map(\.path), ["/b"])

        // Discovered as missing but watched by its row: the row stays found
        // at the watched path, with no install line beside it.
        let missing = Self.candidate(.codex, exists: false)
        state.answer(.codex, .watch(path: "/d"))
        let codex = try XCTUnwrap(ToolsScreenLayout.rows([missing], state: state).first { $0.source == .codex })
        XCTAssertTrue(codex.exists)
        XCTAssertEqual(codex.path, "/d")
        XCTAssertFalse(
            ToolAnswerRowLayout.offersGetTool(codex, installURL: URL(string: "https://example.com"), in: state))
    }
}
