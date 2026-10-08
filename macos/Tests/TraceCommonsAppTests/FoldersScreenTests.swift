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

    /// Spec rule 1, as the owner reversed it in Ron's design review of
    /// #1235: a tool not on this Mac is not asked. Its row reads only the
    /// core's install line and, when there is an install page, "Get
    /// {tool}"; no picker and no folder button. Continue counts only the
    /// tools found here.
    func test_aMissingToolIsNotAsked() throws {
        let missing = Self.candidate(.codex, exists: false)
        let found = Self.candidate(.claudeCode, exists: true)

        let state = FirstRunState(step: .folders)
        XCTAssertEqual(ToolAnswerRowLayout.options(for: found, in: state), [.watch, .dontUse])
        XCTAssertEqual(ToolAnswerRowLayout.options(for: missing, in: state), [])
        XCTAssertFalse(ToolAnswerRowLayout.asks(missing, in: state))
        XCTAssertTrue(ToolAnswerRowLayout.asks(found, in: state))
        let url = URL(string: "https://example.com/codex")
        XCTAssertTrue(ToolAnswerRowLayout.offersGetTool(missing, installURL: url, in: state))
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(missing, installURL: nil, in: state))
        XCTAssertFalse(ToolAnswerRowLayout.offersGetTool(found, installURL: url, in: state))
        XCTAssertFalse(ToolAnswerRowLayout.offersFolderChoice(missing, in: state))
        XCTAssertTrue(ToolAnswerRowLayout.offersFolderChoice(found, in: state))

        // Opening the screen writes nothing, and only the found tool holds
        // Continue.
        var answered = state
        answered.recordDiscovery([found, missing])
        XCTAssertNil(answered.toolAnswers[.codex])
        XCTAssertFalse(FirstRunNavigation.canContinue(answered, candidates: [found, missing], requiredScope: nil))
        ToolAnswerRowLayout.select(.dontUse, for: found, in: &answered)
        XCTAssertTrue(FirstRunNavigation.canContinue(answered, candidates: [found, missing], requiredScope: nil))

        // The row's words are the core's: "Get {tool}", and no install line
        // (owner, 2026-10-08). The button is the folder picker's neutral
        // glass pill, not a coloured one.
        let folders = try copy().folders
        let row = try Self.source("ToolAnswerRow.swift")
        XCTAssertFalse(row.contains("copy.notInstalled"))
        XCTAssertTrue(row.contains("copy.getTool"))
        let getTool = try XCTUnwrap(row.range(of: "ToolAnswerRowLayout.fill(copy.getTool, tool: candidate.source)"))
        let getStyle = String(row[getTool.upperBound...].prefix(400))
        XCTAssertTrue(getStyle.contains(".buttonStyle(GlassButtonStyle(.glass))"), getStyle)
        XCTAssertFalse(row.contains("GlassButtonStyle(.secondary"))
        XCTAssertTrue(folders.getTool.contains("{tool}"))
        XCTAssertTrue(row.contains("GlassToolTile("))
        XCTAssertTrue(row.contains("large: true"))
        XCTAssertTrue(row.contains("GlassPicker("))
        XCTAssertTrue(row.contains("GlassFolderButton("))
        // Both screens record discovery, which declares the missing tools.
        for screen in ["FoldersScreen.swift", "ToolsScreen.swift"] {
            XCTAssertTrue(try Self.source(screen).contains("recordDiscovery("), screen)
        }
    }

    func test_continueIsDisabledUntilEveryFoundRowIsAnswered() {
        let claude = Self.candidate(.claudeCode, exists: true)
        let codex = Self.candidate(.codex, exists: false)
        let cline = Self.candidate(.cline, exists: true)
        let candidates = [claude, codex, cline]

        var state = FirstRunState(step: .folders)
        state.recordDiscovery(candidates)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        ToolAnswerRowLayout.select(.watch, for: claude, in: &state)
        // Codex is not on this Mac and is not asked, but Cline is found and
        // unanswered.
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

    /// Kristi's #1235 M3: a refused change of folders reads its own core
    /// line, never the watcher's, and needs no onboarding copy to be said.
    func test_aRefusedSettingsChangeReadsItsOwnLine() throws {
        let onboarding = try XCTUnwrap(TCOnboardingCopy.load())
        let firstRun: FirstRunCopy = try copy()
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .settingsFailed, copy: firstRun, onboarding: onboarding),
            firstRun.folders.settingsFailed)
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .settingsFailed, copy: firstRun, onboarding: nil),
            firstRun.folders.settingsFailed)
        XCTAssertNotEqual(firstRun.folders.settingsFailed, onboarding.watcherStartFailed)
    }

    /// Every way `.leaveRoots` can stop while the person stays on Folders
    /// shows a core line, so Continue never does nothing in silence. A
    /// near.ai sign-in that did not finish reads the core's sign-in line;
    /// an invite the issuer could not be asked about reads
    /// `lookup_unavailable`. A failed enroll reads the core's `enroll_refused`:
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

        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .signInFailed, copy: firstRun, onboarding: onboarding),
            firstRun.folders.signInFailed)
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .lookupUnavailable, copy: firstRun, onboarding: onboarding),
            firstRun.folders.lookupUnavailable)

        var seen: [FirstRunFailure] = []
        for failing in plan + [.lookupInvite("unavailable")] {
            let daemon = RecordingFirstRunDaemon()
            if failing == .lookupInvite("unavailable") {
                daemon.lookup = .unavailable
            } else if case .lookupInvite = failing {
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
            case .startFailed, .settingsFailed, .inviteDead, .lookupUnavailable, .enrollFailed, .signInFailed,
                .nearAIEnrollFailed, .scopesFailed, .rulesFailed, .privateAIFailed, .grantRefused:
                XCTAssertNotNil(notice, "\(failure)")
            // Leaving the roots never marks completion, and Join's own
            // passkey start is never a Folders failure.
            case .completeFailed, .passkeyUnavailable:
                XCTFail("\(failure)")
            }
        }
        XCTAssertTrue(seen.contains(.lookupUnavailable))
        XCTAssertTrue(seen.contains(.startFailed))
        XCTAssertTrue(seen.contains(.enrollFailed))
        XCTAssertTrue(seen.contains(.signInFailed))

        // The frame is handed that notice.
        let screen = try Self.source("FoldersScreen.swift")
        XCTAssertTrue(screen.contains("notice: FoldersScreenLayout.notice("))
    }

    /// A host whose runner does not last the whole first run (Private AI's)
    /// starts on Folders, and no screen has a Back (Ron's review of #1235,
    /// item 9), so it never reaches Join: what Join would take (an invite,
    /// an account) is never committed there. Its commit is the start alone.
    func test_aHostWithoutJoinCommitsOnlyTheStart() throws {
        let host = try Self.source("../Monitor/InferenceViews.swift")
        XCTAssertTrue(host.contains("OnboardingCoordinatorView(startAt: .folders, takesInvites: false"))

        var state = OnboardingNavigation.initialState(startAt: .folders, daemonRunning: false, enrolled: false)
        state.answer(.claudeCode, .off)
        state.answer(.codex, .off)
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveRoots), [.startDaemon(settingsJSON: json)])
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
    /// Ron's design review of #1235, item 7: an unanswered picker reads
    /// the core's "Choose…", never its question; the question stays the
    /// picker's accessible label. Every first-run picker draws its
    /// placeholder from that one word.
    func test_anUnansweredPickerReadsChoose() throws {
        XCTAssertEqual(try self.copy().frame.choose, "Choose…")
        for file in ["ToolAnswerRow.swift", "ToolsScreen.swift", "RulesScreen.swift", "UsesScreen.swift"] {
            let lines = try Self.source(file).split(separator: "\n").filter {
                $0.contains("placeholder:") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("///")
            }
            XCTAssertFalse(lines.isEmpty, file)
            for line in lines {
                XCTAssertTrue(line.contains("placeholder: choose") || line.contains("frame.choose"), "\(file): \(line)")
            }
        }
    }

    func test_aWatchedMissingToolOffersWatch() {
        let missing = Self.candidate(.codex, exists: false)
        var state = FirstRunState(step: .folders)
        XCTAssertEqual(ToolAnswerRowLayout.options(for: missing, in: state), [])

        // A restored state that watches it: the row asks, so the picker can
        // show the answer Continue counts.
        state.answer(.codex, .watch(path: "/Volumes/work/codex"))
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: missing), .watch)
        XCTAssertEqual(ToolAnswerRowLayout.options(for: missing, in: state), [.watch, .dontUse])
        XCTAssertTrue(ToolAnswerRowLayout.asks(missing, in: state))
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

    /// "Get {tool}" opens the tool's install page: every tool the Mac is
    /// asked about has one, from the core, over https.
    func test_everyToolHasAnInstallLink() throws {
        let folders = try copy().folders
        for kind in SourceKind.allCases {
            let url = try XCTUnwrap(folders.installURL(for: kind), "\(kind)")
            XCTAssertEqual(url.scheme, "https", "\(kind)")
        }
    }

    /// Both screens pass the core's links to the row, so the button is live.
    func test_bothScreensPassTheInstallLinks() throws {
        let coordinator = try String(
            contentsOf: URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("Sources/TraceCommonsApp/Views/OnboardingCoordinatorView.swift"),
            encoding: .utf8)
        XCTAssertTrue(coordinator.contains("FoldersScreen(copy: copy, runner: runner, installURL: copy.folders.installURL(for:))"))
        XCTAssertTrue(coordinator.contains("ToolsScreen(copy: copy, runner: runner, installURL: copy.folders.installURL(for:))"))
    }
}
