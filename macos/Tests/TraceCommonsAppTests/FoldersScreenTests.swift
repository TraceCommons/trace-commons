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
        let firstRun: FirstRunCopy = try copy()
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .startFailed, copy: firstRun, onboarding: onboarding),
            onboarding.watcherStartFailed)
        XCTAssertNil(FoldersScreenLayout.notice(for: nil, copy: firstRun, onboarding: onboarding))
        XCTAssertNil(FoldersScreenLayout.notice(for: .startFailed, copy: firstRun, onboarding: nil))
    }

    /// Every way `.leaveRoots` can stop while the person stays on Folders
    /// shows a core line, except a failed near.ai sign-in, which is silent
    /// by decision: Continue reopens the sheet, and a cancelled sheet is the
    /// person's own act. A failed enroll reads the core's `enroll_refused`:
    /// the invite was already accepted by lookup, so Join's "not an invite
    /// link" would be false.
    @MainActor
    func test_everyFailureThatStaysOnFoldersShowsACoreLine() async throws {
        let onboarding = try XCTUnwrap(TCOnboardingCopy.load())
        let firstRun: FirstRunCopy = try copy()
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .enrollFailed, copy: firstRun, onboarding: onboarding),
            firstRun.folders.enrollRefused)
        XCTAssertNotEqual(
            FoldersScreenLayout.notice(for: .enrollFailed, copy: firstRun, onboarding: onboarding),
            firstRun.join.inviteError)

        var state = FirstRunState(tier: .quick, step: .folders)
        state.invite = "INVITE-1"
        state.issuerHost = "issuer.example"
        state.account = .nearAI
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        let plan = FirstRunPlan.calls(for: state, at: .leaveRoots)
        XCTAssertTrue(plan.contains(.enroll("INVITE-1")))
        XCTAssertTrue(plan.contains(.signInNearAI))

        var seen: [FirstRunFailure] = []
        for failing in plan {
            let daemon = RecordingFirstRunDaemon()
            if case .lookupInvite = failing {
                daemon.lookup = .refused(label: "invite-invalid")
            } else {
                daemon.failing = { $0 == failing }
            }
            let runner = FirstRunRunner(state: state, daemon: daemon)
            await runner.commit(.leaveRoots)
            let failure = try XCTUnwrap(runner.failure, "\(failing)")
            seen.append(failure)
            guard runner.state.step == .folders else { continue }
            let notice = FoldersScreenLayout.notice(for: failure, copy: firstRun, onboarding: onboarding)
            switch failure {
            case .signInFailed:
                XCTAssertNil(notice)
            case .startFailed, .inviteDead, .enrollFailed, .scopesFailed, .rulesFailed, .privateAIFailed,
                .grantRefused:
                XCTAssertNotNil(notice, "\(failure)")
            }
        }
        XCTAssertTrue(seen.contains(.startFailed))
        XCTAssertTrue(seen.contains(.enrollFailed))
        XCTAssertTrue(seen.contains(.signInFailed))

        // The frame is handed that notice.
        let screen = try Self.source("FoldersScreen.swift")
        XCTAssertTrue(screen.contains("notice: FoldersScreenLayout.notice("))
    }

    /// Back is withdrawn while a commit runs: the runner moves the step on
    /// from wherever the state is when the calls finish.
    func test_backIsWithdrawnWhileCommitting() throws {
        var pressed = 0
        XCTAssertNil(FoldersScreenLayout.backAction(isCommitting: true, back: { pressed += 1 }))
        let back = try XCTUnwrap(FoldersScreenLayout.backAction(isCommitting: false, back: { pressed += 1 }))
        back()
        XCTAssertEqual(pressed, 1)

        let screen = try Self.source("FoldersScreen.swift")
        XCTAssertTrue(screen.contains("onBack: FoldersScreenLayout.backAction(isCommitting: runner.isCommitting"))
    }

    /// While a commit runs, nothing on the screen can change the plan being
    /// committed: the rows are disabled and the frame offers no tier switch.
    /// A row flipped mid-commit would otherwise show "I don't use it" while
    /// the daemon keeps watching the folder (`.start` sends no roots).
    func test_rowsAndTierSwitchAreHeldWhileCommitting() throws {
        XCTAssertFalse(FoldersScreenLayout.rowsEnabled(isCommitting: true))
        XCTAssertTrue(FoldersScreenLayout.rowsEnabled(isCommitting: false))

        let quickFolders = FirstRunState(tier: .quick, step: .folders)
        XCTAssertTrue(FirstRunFrameLayout.offersCustomSetupInstead(quickFolders, isCommitting: false))
        XCTAssertFalse(FirstRunFrameLayout.offersCustomSetupInstead(quickFolders, isCommitting: true))

        let screen = try Self.source("FoldersScreen.swift")
        XCTAssertTrue(screen.contains(".disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))"))
        XCTAssertTrue(screen.contains("isCommitting: runner.isCommitting,"))
    }

    /// Discovery that returns nothing readable is a failure with the core's
    /// line and a retry, never an empty list behind a closed Continue. A
    /// later refresh that fails keeps the rows already shown.
    func test_discoveryFailureShowsALineAndRefreshKeepsRows() throws {
        let rowsJSON = """
            [{"source":"claude-code","path":"/Users/someone/.claude/projects","exists":true,\
            "session_count":3,"most_recent":null,"relocated_by_env":false}]
            """
        let found = FoldersScreenLayout.discovered(rowsJSON, keeping: .loading)
        guard case .found(let rows) = found else { return XCTFail("\(found)") }
        XCTAssertEqual(rows.map(\.source), [.claudeCode])

        for bad in [nil, "not json", "[]"] as [String?] {
            XCTAssertEqual(FoldersScreenLayout.discovered(bad, keeping: .loading), .failed, "\(String(describing: bad))")
            XCTAssertEqual(FoldersScreenLayout.discovered(bad, keeping: .failed), .failed)
            XCTAssertEqual(FoldersScreenLayout.discovered(bad, keeping: found), found)
        }
        XCTAssertNil(FoldersScreenLayout.discovered(rowsJSON, keeping: .loading).failureLine(try copy().folders))
        XCTAssertEqual(DiscoveredRows.failed.failureLine(try copy().folders), try copy().folders.discoveryFailed)

        // Discovery runs again when the app comes back to the front, so an
        // install made meanwhile shows ("Install it, then this row asks
        // again."), and on the failure's retry.
        let screen = try Self.source("FoldersScreen.swift")
        XCTAssertTrue(screen.contains("NSApplication.didBecomeActiveNotification"))
        XCTAssertTrue(screen.contains("copy.folders.retry"))
        XCTAssertTrue(screen.contains("FoldersScreenLayout.discovered(TCDiscovery.sourcesJSON(), keeping: discovery)"))
    }

    /// A missing tool the state already watches (restored, or added on
    /// Custom's Tools) offers Watch, so the picker shows what Continue reads.
    func test_aWatchedMissingToolOffersWatch() {
        let missing = Self.candidate(.codex, exists: false)
        var state = FirstRunState(step: .folders)
        XCTAssertEqual(ToolAnswerRowLayout.options(for: missing, in: state), [.dontUse])

        state.answer(.codex, .watch(path: "/Volumes/work/codex"))
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: missing), .watch)
        XCTAssertEqual(ToolAnswerRowLayout.options(for: missing, in: state), [.watch, .dontUse])
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: state, for: missing), "/Volumes/work/codex")

        let row = (try? Self.source("ToolAnswerRow.swift")) ?? ""
        XCTAssertTrue(row.contains("ToolAnswerRowLayout.options(for: candidate, in: state)"))
    }

    /// A chosen folder survives "I don't use it" and back to Watch.
    func test_aChosenFolderSurvivesDontUseAndBack() {
        let claude = Self.candidate(.claudeCode, exists: true)
        var state = FirstRunState(step: .folders)

        ToolAnswerRowLayout.choose(folder: "/Volumes/work/claude", for: claude, in: &state)
        ToolAnswerRowLayout.select(.dontUse, for: claude, in: &state, chosenFolder: "/Volumes/work/claude")
        XCTAssertEqual(state.toolAnswers[.claudeCode], .off)
        ToolAnswerRowLayout.select(.watch, for: claude, in: &state, chosenFolder: "/Volumes/work/claude")
        XCTAssertEqual(state.toolAnswers[.claudeCode], .watch(path: "/Volumes/work/claude"))

        let row = (try? Self.source("ToolAnswerRow.swift")) ?? ""
        XCTAssertTrue(row.contains("chosenFolder: chosenFolder"))
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
