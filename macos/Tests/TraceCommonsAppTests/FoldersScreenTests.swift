import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Folders screen and tool row (#1030 `tool-screens.tsx`,
/// `tool-row.tsx`): every offered tool needs an answer, a missing one
/// included, and nothing is answered for the person. Read from the row's
/// pure layout and the screens' sources, the house pattern for a SwiftUI
/// view.
final class FoldersScreenTests: XCTestCase {
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

    private func copy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    func test_aMissingToolStillAsksForAnAnswer() throws {
        let missing = Self.candidate(.codex, exists: false)
        let found = Self.candidate(.claudeCode, exists: true)

        // A found tool offers both answers; a missing one only "I don't use
        // it" -- still a question, never a pre-filled `off`.
        XCTAssertEqual(ToolAnswerRowLayout.options(for: found), [.watch, .dontUse])
        XCTAssertEqual(ToolAnswerRowLayout.options(for: missing), [.dontUse])
        XCTAssertTrue(ToolAnswerRowLayout.offersGetTool(missing))
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(found))
        XCTAssertFalse(ToolAnswerRowLayout.offersFolderChoice(missing))
        XCTAssertTrue(ToolAnswerRowLayout.offersFolderChoice(found))

        // Opening the screen writes nothing.
        var state = FirstRunState(step: .folders)
        XCTAssertNil(ToolAnswerRowLayout.answer(in: state, for: missing))
        XCTAssertNil(state.toolAnswers[.codex])
        XCTAssertFalse(
            FirstRunNavigation.canContinue(state, candidates: [found, missing], requiredScope: nil))

        ToolAnswerRowLayout.select(.dontUse, for: missing, in: &state)
        XCTAssertEqual(state.toolAnswers[.codex], .off)
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: missing), .dontUse)

        // The row's words are the core's: Ron's install line and "Get {tool}".
        let folders = try copy().folders
        let row = try Self.source("ToolAnswerRow.swift")
        XCTAssertTrue(row.contains("copy.notInstalled"))
        XCTAssertTrue(row.contains("copy.getTool"))
        XCTAssertTrue(folders.getTool.contains("{tool}"))
        XCTAssertTrue(row.contains("GlassToolTile(.tool("))
        XCTAssertTrue(row.contains("large: true"))
        XCTAssertTrue(row.contains("GlassPicker("))
        XCTAssertTrue(row.contains("GlassFolderButton("))
    }

    func test_continueIsDisabledUntilEveryRowIsAnswered() {
        let claude = Self.candidate(.claudeCode, exists: true)
        let codex = Self.candidate(.codex, exists: false)
        let cline = Self.candidate(.cline, exists: true)
        let candidates = [claude, codex, cline]

        var state = FirstRunState(step: .folders)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        ToolAnswerRowLayout.select(.watch, for: claude, in: &state)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        ToolAnswerRowLayout.select(.dontUse, for: codex, in: &state)
        // Claude Code and Codex are answered, so the daemon would start, but
        // Cline is offered and unanswered.
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
        ToolAnswerRowLayout.select(.dontUse, for: cline, in: &state)
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        // Clearing an answer closes it again.
        ToolAnswerRowLayout.select(nil, for: cline, in: &state)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        // The screen's Continue is that rule, and commits the roots.
        let screen = (try? Self.source("FoldersScreen.swift")) ?? ""
        XCTAssertTrue(screen.contains("FirstRunNavigation.canContinue("))
        XCTAssertTrue(screen.contains("runner.commit(.leaveRoots)"))
        XCTAssertTrue(screen.contains("TCDiscovery.sourcesJSON()"))
        XCTAssertTrue(screen.contains("FirstRunFrame("))
    }

    /// Review Focus 1, the screen's half: a failed start shows the core's
    /// `watcher_start_failed` on the step.
    func test_aFailedStartShowsTheCoresWatcherLine() throws {
        let onboarding = try XCTUnwrap(TCOnboardingCopy.load())
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .startFailed, onboarding: onboarding), onboarding.watcherStartFailed)
        XCTAssertNil(FoldersScreenLayout.notice(for: nil, onboarding: onboarding))
        XCTAssertNil(FoldersScreenLayout.notice(for: .startFailed, onboarding: nil))
    }

    func test_choosingAFolderWatchesThatPath() {
        let claude = Self.candidate(.claudeCode, exists: true)
        var state = FirstRunState(step: .folders)

        ToolAnswerRowLayout.choose(folder: "/Volumes/work/claude", for: claude, in: &state)
        XCTAssertEqual(state.toolAnswers[.claudeCode], .watch(path: "/Volumes/work/claude"))
        XCTAssertEqual(state.sessionRoots.claude, .watch(path: "/Volumes/work/claude"))
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: claude), .watch)
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: state, for: claude), "/Volumes/work/claude")

        // Without a chosen folder, Watch adopts the discovered path.
        var fresh = FirstRunState(step: .folders)
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: fresh, for: claude), claude.path)
        ToolAnswerRowLayout.select(.watch, for: claude, in: &fresh)
        XCTAssertEqual(fresh.toolAnswers[.claudeCode], .watch(path: claude.path))
    }
}
