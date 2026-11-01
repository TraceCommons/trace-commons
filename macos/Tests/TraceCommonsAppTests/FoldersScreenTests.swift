import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The rules Ron's Folders screen and tool row (#1030 `tool-screens.tsx`,
/// `tool-row.tsx`) set and Tools keeps: a tool not on this Mac is not
/// asked, nothing is watched for the person, and a commit that stops says
/// why in the core's words. Read from the pure layouts and the screens'
/// sources, the house pattern for a SwiftUI view.
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
    /// #1235: a tool not on this Mac is not asked. It leaves the Tools list
    /// (Ron, 2026-10-21), nothing is written for it, and the found tool's
    /// check starts unticked, Not used.
    func test_aMissingToolIsNotAsked() throws {
        let missing = Self.candidate(.codex, exists: false)
        let found = Self.candidate(.claudeCode, exists: true)
        var state = FirstRunState(step: .tools)
        state.recordDiscovery([found, missing])
        XCTAssertEqual(ToolsScreenLayout.listed([found, missing], state: state).map(\.source), [.claudeCode])
        XCTAssertNil(state.toolAnswers[.codex])
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: found), .dontUse)
        XCTAssertFalse(ToolsScreenLayout.isWatched(found, in: state))
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: [found, missing], requiredScope: nil))
        ToolsScreenLayout.setWatched(true, found, in: &state)
        XCTAssertEqual(state.toolAnswers[.claudeCode], .watch(path: found.path))
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: [found, missing], requiredScope: nil))
        // Tools records discovery, which declares the missing tools.
        XCTAssertTrue(try Self.source("ToolsScreen.swift").contains("recordDiscovery("))
    }

    func test_nativeOpenCodeDatabaseRequiresExplicitWatchAndPreservesExactPath() throws {
        let candidate = SourceCandidate(
            source: .opencode, path: "/synthetic/opencode/opencode.db", exists: true,
            sessionCount: 2, mostRecent: nil, relocatedByEnv: false)
        // Discovery includes missing required tools so roots can declare
        // them off without inventing a contributor answer.
        let candidates = [candidate, Self.candidate(.claudeCode, exists: false),
                          Self.candidate(.codex, exists: false)]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)

        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: candidate), .dontUse)
        XCTAssertEqual(state.sessionRoots.opencode, .off)
        XCTAssertFalse(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))

        ToolsScreenLayout.setWatched(true, candidate, in: &state)
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: candidate), .watch)
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: state, for: candidate), "/synthetic/opencode/opencode.db")
        XCTAssertEqual(state.sessionRoots.opencode, .watch(path: "/synthetic/opencode/opencode.db"))
        let params = try XCTUnwrap(state.sessionRoots.opencode.settingsParams(for: .opencode))
        XCTAssertEqual(params["opencode_source"] as? [String: String],
                       ["mode": "watch", "path": "/synthetic/opencode/opencode.db"])
        XCTAssertTrue(FirstRunNavigation.canContinue(state, candidates: candidates, requiredScope: nil))
    }

    func test_nativeOpenCodeDatabaseOffSurvivesRediscovery() throws {
        let candidate = SourceCandidate(
            source: .opencode, path: "/synthetic/opencode/opencode.db", exists: true,
            sessionCount: 2, mostRecent: nil, relocatedByEnv: false)
        let candidates = [candidate, Self.candidate(.claudeCode, exists: false),
                          Self.candidate(.codex, exists: false)]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)
        ToolsScreenLayout.setWatched(true, candidate, in: &state)
        ToolsScreenLayout.setWatched(false, candidate, in: &state)
        state.recordDiscovery(candidates)

        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: candidate), .dontUse)
        XCTAssertEqual(state.sessionRoots.opencode, .off)
        let params = try XCTUnwrap(state.sessionRoots.opencode.settingsParams(for: .opencode))
        XCTAssertEqual(params["opencode_source"] as? [String: String], ["mode": "off"])
    }

    /// Continue waits for a ticked tool (Ron, 2026-10-22), and for the scan
    /// to finish; it commits the roots and moves on.
    func test_continueWaitsForATickedToolAndTheScan() throws {
        let claude = Self.candidate(.claudeCode, exists: true)
        let cline = Self.candidate(.cline, exists: true)
        let candidates = [claude, Self.candidate(.codex, exists: false), cline]
        var state = FirstRunState(step: .tools)
        state.recordDiscovery(candidates)
        XCTAssertFalse(ToolsScreenLayout.canContinue(discovered: candidates, state: state, pending: false, isCommitting: false))
        ToolsScreenLayout.setWatched(true, cline, in: &state)
        XCTAssertTrue(ToolsScreenLayout.canContinue(discovered: candidates, state: state, pending: false, isCommitting: false))
        XCTAssertFalse(
            ToolsScreenLayout.canContinue(discovered: candidates, state: state, pending: false, isCommitting: false, scanned: false))
        XCTAssertFalse(ToolsScreenLayout.canContinue(discovered: candidates, state: state, pending: true, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.canContinue(discovered: candidates, state: state, pending: false, isCommitting: true))
        XCTAssertFalse(ToolsScreenLayout.canContinue(discovered: nil, state: state, pending: false, isCommitting: false))

        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("ToolsScreenLayout.canContinue("))
        XCTAssertTrue(screen.contains("runner.commit(.leaveTools)"))
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
        XCTAssertTrue(try Self.source("ToolsScreen.swift").contains("notice: FoldersScreenLayout.notice("))
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

    /// Every way Sign in on Join can stop shows a core line, so the button
    /// never does nothing in silence: under the cards, or under the near.ai
    /// card when it cleared the choice, or on the invite card for a dead
    /// invite. A failed enroll reads the core's `enroll_refused`: the invite
    /// was already accepted by lookup, so Join's "not an invite link" would
    /// be false.
    @MainActor
    func test_everyWaySignInStopsShowsACoreLine() async throws {
        let onboarding = try XCTUnwrap(TCOnboardingCopy.load())
        let firstRun: FirstRunCopy = try copy()
        XCTAssertEqual(
            FoldersScreenLayout.notice(for: .enrollFailed, copy: firstRun, onboarding: onboarding),
            firstRun.folders.enrollRefused)
        XCTAssertNotEqual(
            FoldersScreenLayout.notice(for: .enrollFailed, copy: firstRun, onboarding: onboarding),
            firstRun.join.inviteError)

        var state = FirstRunState(step: .join)
        state.invite = "INVITE-1"
        state.issuerHost = "issuer.example"
        state.account = .nearAI
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        let plan = FirstRunPlan.calls(for: state, at: .signIn)
        XCTAssertTrue(plan.contains(.enroll("INVITE-1")))
        XCTAssertTrue(plan.contains(.signInNearAI))

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
            await runner.commit(.signIn)
            let failure = try XCTUnwrap(runner.failure, "\(failing)")
            seen.append(failure)
            XCTAssertEqual(runner.state.step, .join, "\(failure)")
            let under = JoinScreenLayout.notice(
                for: failure, in: runner.state, copy: firstRun, onboarding: onboarding, privateAI: nil)
            let nearAI = JoinScreenLayout.nearAINotice(runner.state, failure: failure, copy: firstRun)
            let invite = JoinScreenLayout.inviteLine(
                runner.state, lookup: nil, failure: failure, copy: firstRun.join)
            let said = under != nil || nearAI != nil || invite == .error(firstRun.join.inviteDead)
            XCTAssertTrue(said, "\(failure)")
            XCTAssertFalse(under != nil && nearAI != nil, "\(failure) said twice")
        }
        XCTAssertTrue(seen.contains(.lookupUnavailable))
        XCTAssertTrue(seen.contains(.startFailed))
        XCTAssertTrue(seen.contains(.enrollFailed))
        XCTAssertTrue(seen.contains(.signInFailed))
        XCTAssertTrue(seen.contains(.inviteDead(label: "invite-invalid")))
        XCTAssertTrue(try Self.source("JoinScreen.swift").contains("notice: JoinScreenLayout.notice("))
    }

    /// A host whose runner does not last the whole first run (Private AI's)
    /// starts on Tools: its Continue is the start alone, and what Join would
    /// take (an invite, an account) is never committed there.
    func test_aHostWithoutJoinCommitsOnlyTheStart() throws {
        let host = try Self.source("../Monitor/InferenceViews.swift")
        XCTAssertTrue(host.contains("OnboardingCoordinatorView(startAt: .tools, takesInvites: false"))

        var state = OnboardingNavigation.initialState(startAt: .tools, daemonRunning: false, enrolled: false)
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude/projects"))
        state.answer(.codex, .off)
        let json = try XCTUnwrap(state.sessionRoots.settingsJSON())
        XCTAssertEqual(FirstRunPlan.calls(for: state, at: .leaveTools), [.startDaemon(settingsJSON: json)])
    }

    /// While a commit runs, nothing on Tools can change the plan being
    /// committed: the rows, the add row and Customize are held.
    @MainActor
    func test_rowsAndCustomizeAreHeldWhileCommitting() throws {
        XCTAssertFalse(FoldersScreenLayout.rowsEnabled(isCommitting: true))
        XCTAssertTrue(FoldersScreenLayout.rowsEnabled(isCommitting: false))
        var watching = FirstRunState(step: .tools)
        watching.answer(.claudeCode, .watch(path: "/a"))
        watching.answer(.codex, .off)
        XCTAssertTrue(ToolsScreenLayout.customizeEnabled(state: watching, open: false, scanned: true, isCommitting: false))
        XCTAssertFalse(ToolsScreenLayout.customizeEnabled(state: watching, open: false, scanned: true, isCommitting: true))
        XCTAssertFalse(ToolsScreenLayout.customizeEnabled(state: watching, open: false, scanned: false, isCommitting: false))
        XCTAssertFalse(
            ToolsScreenLayout.customizeEnabled(state: FirstRunState(step: .tools), open: false, scanned: true, isCommitting: false),
            "nothing watched, nothing to customize")
        XCTAssertTrue(ToolsScreenLayout.customizeEnabled(state: FirstRunState(step: .tools), open: true, scanned: true, isCommitting: false),
            "an open Customize can always close")

        let shared = try Self.source("ToolsScreen.swift")
        XCTAssertEqual(
            shared.components(separatedBy: ".disabled(!FoldersScreenLayout.rowsEnabled(isCommitting: runner.isCommitting))")
                .count - 1, 2, "the rows and the add row")
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
        // install made meanwhile is offered, and on the failure's retry.
        let screen = try Self.source("ToolsScreen.swift")
        XCTAssertTrue(screen.contains("NSApplication.didBecomeActiveNotification"))
        XCTAssertTrue(screen.contains("Button(copy.folders.retry, action: refreshDiscovery)"))
        XCTAssertTrue(screen.contains("FoldersScreenLayout.discovered(TCDiscovery.sourcesJSON(), keeping: discovery)"))
    }

    /// Ron's design review of #1235, item 7: an unanswered picker reads the
    /// core's "Choose…", never its question; the question stays the
    /// picker's accessible label.
    func test_anUnansweredPickerReadsChoose() throws {
        XCTAssertEqual(try self.copy().frame.choose, "Choose…")
        let lines = try Self.source("ToolsScreen.swift").split(separator: "\n").filter {
            $0.contains("placeholder:") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("///")
        }
        XCTAssertFalse(lines.isEmpty)
        for line in lines {
            XCTAssertTrue(line.contains("frame.choose"), "\(line)")
        }
    }

    /// A chosen folder survives "Not used" and back to Watch.
    func test_aChosenFolderSurvivesDontUseAndBack() {
        let claude = Self.candidate(.claudeCode, exists: true)
        var state = FirstRunState(step: .tools)

        ToolAnswerRowLayout.choose(folder: "/Volumes/work/claude", for: claude, in: &state)
        ToolAnswerRowLayout.select(.dontUse, for: claude, in: &state, chosenFolder: "/Volumes/work/claude")
        XCTAssertEqual(state.toolAnswers[.claudeCode], .off)
        ToolAnswerRowLayout.select(.watch, for: claude, in: &state, chosenFolder: "/Volumes/work/claude")
        XCTAssertEqual(state.toolAnswers[.claudeCode], .watch(path: "/Volumes/work/claude"))
    }

    func test_choosingAFolderWatchesThatPath() {
        let claude = Self.candidate(.claudeCode, exists: true)
        var state = FirstRunState(step: .tools)

        ToolAnswerRowLayout.choose(folder: "/Volumes/work/claude", for: claude, in: &state)
        XCTAssertEqual(state.toolAnswers[.claudeCode], .watch(path: "/Volumes/work/claude"))
        XCTAssertEqual(state.sessionRoots.claude, .watch(path: "/Volumes/work/claude"))
        XCTAssertEqual(ToolAnswerRowLayout.answer(in: state, for: claude), .watch)
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: state, for: claude), "/Volumes/work/claude")

        // Without a chosen folder, the tick adopts the discovered path.
        var fresh = FirstRunState(step: .tools)
        XCTAssertEqual(ToolAnswerRowLayout.shownPath(in: fresh, for: claude), claude.path)
        ToolsScreenLayout.setWatched(true, claude, in: &fresh)
        XCTAssertEqual(fresh.toolAnswers[.claudeCode], .watch(path: claude.path))
    }

    /// Every tool has a setup guide in the core's one table, over https.
    func test_everyToolHasAnInstallLink() throws {
        let guides = InstallGuides(json: TCCoreCopy.installGuidesJSON())
        for kind in SourceKind.allCases {
            let url = try XCTUnwrap(guides.url(for: kind), "\(kind)")
            XCTAssertEqual(url.scheme, "https", "\(kind)")
        }
    }
}
