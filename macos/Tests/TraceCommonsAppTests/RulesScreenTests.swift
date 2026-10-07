import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Rules screen (#1030 `rules-screen.tsx`, `ftux-model.ts`): the
/// selection rules of the past-session picker, read from the screen's pure
/// layout type, the house pattern for a SwiftUI view.
final class RulesScreenTests: XCTestCase {
    private static let started = Date(timeIntervalSince1970: 1_790_000_000)

    private func session(
        _ id: String,
        state: PastSession.State = .notQueued,
        selectable: Bool = true,
        title: String? = nil,
        durationSecs: Int? = nil,
        sizeBytes: Int = 2_048
    ) -> PastSession {
        PastSession(
            id: id,
            entryID: nil,
            state: state,
            selectable: selectable,
            startedAt: Self.started,
            durationSecs: durationSecs,
            title: title,
            sizeBytes: sizeBytes,
            source: "claude-code"
        )
    }

    /// "Include every past session in {folder}" selects each session the
    /// picker may tick, and nothing it may not; off clears the folder. The
    /// box writes the same value through every row it reaches, so the
    /// write must land the same however many times it arrives.
    func test_includeEverySelectsEachSelectableSession() {
        let sessions = [
            session("a"),
            session("b", state: .pending),
            session("c", selectable: false),
            session("d", state: .stillActive, selectable: false),
        ]
        var state = FirstRunState(tier: .custom, step: .rules, account: .enrolled)

        // One write per tickable row, as the native group toggle sends them.
        for _ in sessions.filter(RulesScreenLayout.isTickable) {
            RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        }
        XCTAssertEqual(state.pastSelections["p1"], ["a", "b"])
        XCTAssertEqual(RulesScreenLayout.groupState(state, projectID: "p1", sessions: sessions), .all)

        // One unticked: the group reads mixed, and turning it on selects all.
        RulesScreenLayout.setTicked(&state, projectID: "p1", session: sessions[0], on: false)
        XCTAssertEqual(RulesScreenLayout.groupState(state, projectID: "p1", sessions: sessions), .some)
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        XCTAssertEqual(state.pastSelections["p1"], ["a", "b"])

        // Off clears the folder, however many rows the write reaches.
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: false)
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: false)
        XCTAssertNil(state.pastSelections["p1"])
        XCTAssertEqual(RulesScreenLayout.groupState(state, projectID: "p1", sessions: sessions), .none)

        // The selection is what the plan sends as the person's approval.
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        XCTAssertEqual(
            FirstRunPlan.calls(for: state, at: .start).filter {
                if case .includePastSessions = $0 { return true }
                return false
            },
            [.includePastSessions(projectID: "p1", ["a", "b"])])
    }

    /// A folder with more past sessions than the picker lists says how many
    /// more with the core's line; one with none says nothing.
    func test_aFolderWithOlderSessionsNotListedSaysHowMany() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON()))).rules
        XCTAssertEqual(
            RulesScreenLayout.notListedNote(7, copy: copy),
            FirstRunCopy.fill(copy.notListed, ["count": "7"]))
        XCTAssertNil(RulesScreenLayout.notListedNote(0, copy: copy))
    }

    /// Setting a folder to Never clears its selection (Ron's `applyRule`),
    /// and the folder's other rules leave it alone.
    func test_neverClearsTheFolderSelection() {
        let sessions = [session("a"), session("b")]
        var state = FirstRunState(tier: .custom, step: .rules)
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        RulesScreenLayout.includeEvery(&state, projectID: "p2", sessions: sessions, on: true)

        RulesScreenLayout.setRule(&state, projectID: "p1", mode: .ask)
        XCTAssertEqual(state.pastSelections["p1"], ["a", "b"])

        RulesScreenLayout.setRule(&state, projectID: "p1", mode: .ignore)
        XCTAssertEqual(state.rules["p1"], .ignore)
        XCTAssertNil(state.pastSelections["p1"])
        XCTAssertEqual(state.pastSelections["p2"], ["a", "b"])

        // A Never folder cannot be ticked back while its rule is Never.
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        RulesScreenLayout.setTicked(&state, projectID: "p1", session: sessions[0], on: true)
        XCTAssertNil(state.pastSelections["p1"])
    }

    /// A rule the person never touched is not written: the picker shows the
    /// daemon's mode, and the plan sends no `setProjectMode` for it.
    func test_anUntouchedRuleIsNotSent() {
        let project = ProjectRow(projectId: "p1", projectLabel: "repo", mode: .ask)
        let state = FirstRunState(tier: .custom, step: .rules)
        XCTAssertEqual(RulesScreenLayout.rule(state, for: project), .ask)
        XCTAssertTrue(state.rules.isEmpty)
    }

    /// Automatic needs an enrolment. A watch-only person is not offered it per
    /// folder, and the layout refuses it however it is asked, so the plan
    /// never sends an arming the daemon would refuse for want of terms.
    func test_watchOnlyIsNotOfferedAutomaticPerFolder() {
        let project = ProjectRow(projectId: "p1", projectLabel: "repo", mode: .ask)
        let watching = FirstRunState(tier: .custom, step: .rules, account: .watchOnly)
        let joined = FirstRunState(tier: .custom, step: .rules, account: .nearAI, enrolledInvite: "INVITE-1")
        XCTAssertEqual(RulesScreenLayout.offeredModes(project, state: watching), [.ask, .ignore])
        XCTAssertEqual(RulesScreenLayout.offeredModes(project, state: joined), [.ask, .autoUpload, .ignore])

        var state = FirstRunState(tier: .custom, step: .rules, account: .watchOnly)
        XCTAssertEqual(RulesScreenLayout.pick(&state, project: project, wanted: .autoUpload), .refused)
        XCTAssertFalse(RulesScreenLayout.confirmArming(&state, project: project))
        XCTAssertTrue(state.rules.isEmpty)

        // A folder the daemon already arms keeps its mode on the picker, so
        // the picker never shows a mode it has no option for.
        let armed = ProjectRow(projectId: "p2", projectLabel: "armed", mode: .autoUpload)
        XCTAssertEqual(RulesScreenLayout.offeredModes(armed, state: watching), [.ask, .autoUpload, .ignore])
    }

    /// Arming is a grant, so it is never silent: picking Automatic asks
    /// first, and only the confirmation writes the rule the plan sends.
    func test_automaticWaitsForTheArmingConfirmation() {
        let project = ProjectRow(projectId: "p1", projectLabel: "repo", mode: .ask)
        var state = FirstRunState(tier: .custom, step: .rules, account: .nearAI, enrolledInvite: "INVITE-1")

        XCTAssertEqual(RulesScreenLayout.pick(&state, project: project, wanted: .autoUpload), .needsConfirmation)
        XCTAssertTrue(state.rules.isEmpty)
        XCTAssertFalse(
            FirstRunPlan.calls(for: state, at: .start).contains {
                if case .setProjectMode = $0 { return true }
                return false
            })

        // `setRule` is not a way around the confirmation.
        RulesScreenLayout.setRule(&state, projectID: "p1", mode: .autoUpload)
        XCTAssertTrue(state.rules.isEmpty)

        XCTAssertTrue(RulesScreenLayout.confirmArming(&state, project: project))
        XCTAssertEqual(state.rules["p1"], .autoUpload)
    }

    /// Picking the mode the daemon already has is not a change, so nothing
    /// is sent for it, as Settings does.
    func test_repickingTheDaemonsModeIsNotSent() {
        let project = ProjectRow(projectId: "p1", projectLabel: "repo", mode: .ask)
        var state = FirstRunState(tier: .custom, step: .rules, account: .nearAI, enrolledInvite: "INVITE-1")

        XCTAssertEqual(RulesScreenLayout.pick(&state, project: project, wanted: .ask), .applied)
        XCTAssertTrue(state.rules.isEmpty)

        XCTAssertEqual(RulesScreenLayout.pick(&state, project: project, wanted: .ignore), .applied)
        XCTAssertEqual(state.rules["p1"], .ignore)
        XCTAssertEqual(RulesScreenLayout.pick(&state, project: project, wanted: .ask), .applied)
        XCTAssertTrue(state.rules.isEmpty)
    }

    /// A `still_active` row cannot be ticked, whatever the wire said about
    /// it, by the folder box or one by one.
    func test_aStillActiveSessionCannotBeTicked() {
        let active = session("live", state: .stillActive, selectable: true)
        XCTAssertFalse(RulesScreenLayout.isTickable(active))

        var state = FirstRunState(tier: .custom, step: .rules)
        RulesScreenLayout.setTicked(&state, projectID: "p1", session: active, on: true)
        XCTAssertNil(state.pastSelections["p1"])
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: [active], on: true)
        XCTAssertNil(state.pastSelections["p1"])

        // A row the daemon says may not be ticked is refused the same way.
        let refused = session("x", state: .pending, selectable: false)
        XCTAssertFalse(RulesScreenLayout.isTickable(refused))
        XCTAssertTrue(RulesScreenLayout.isTickable(session("ok")))
    }

    /// A `not_queued` row was never opened: it shows its date and size. A
    /// row with a title shows date, title and duration, and no size.
    func test_rowsWithoutATitleShowDateAndSize() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON()))).rules
        let date = RulesScreenLayout.dateText(Self.started, copy: copy)
        let size = RulesScreenLayout.sizeText(2_048)
        XCTAssertFalse(date.isEmpty)
        XCTAssertFalse(size.isEmpty)

        XCTAssertEqual(RulesScreenLayout.labelParts(session("a"), copy: copy), [date, size])

        let titled = session("b", state: .pending, title: "Fix the parser", durationSecs: 3_120)
        let duration = RulesScreenLayout.durationText(3_120, copy: copy)
        XCTAssertEqual(duration, "52 min")
        XCTAssertEqual(RulesScreenLayout.labelParts(titled, copy: copy), [date, "Fix the parser", duration])

        // No start time: the date part is left out, never replaced.
        let undated = PastSession(
            id: "c", entryID: nil, state: .notQueued, selectable: true, startedAt: nil,
            durationSecs: nil, title: nil, sizeBytes: 2_048, source: "codex")
        XCTAssertEqual(RulesScreenLayout.labelParts(undated, copy: copy), [size])
    }

    /// Ron's #1030 formats (design review of #1235, item 6): "Sat 12 Sep",
    /// "52 min", "1 h 18 min", "2 h 04 min", every word the core's, and the
    /// date read in UTC. 23:30 UTC on Saturday 12 September is already
    /// Sunday in Tokyo and still Saturday in Los Angeles; it reads Saturday
    /// whatever zone this Mac is in.
    func test_sessionDatesAndDurationsUseRonsFormatInUTC() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON()))).rules
        let lateSaturday = Date(timeIntervalSince1970: 1_789_255_800)
        let saved = NSTimeZone.default
        defer { NSTimeZone.default = saved }
        for zone in ["Asia/Tokyo", "America/Los_Angeles", "UTC"] {
            NSTimeZone.default = try XCTUnwrap(TimeZone(identifier: zone))
            XCTAssertEqual(RulesScreenLayout.dateText(lateSaturday, copy: copy), "Sat 12 Sep", zone)
        }
        XCTAssertEqual(RulesScreenLayout.durationText(0, copy: copy), "0 min")
        XCTAssertEqual(RulesScreenLayout.durationText(52 * 60, copy: copy), "52 min")
        XCTAssertEqual(RulesScreenLayout.durationText(78 * 60, copy: copy), "1 h 18 min")
        XCTAssertEqual(RulesScreenLayout.durationText(65 * 60, copy: copy), "1 h 5 min")
        XCTAssertEqual(RulesScreenLayout.durationText(124 * 60, copy: copy), "2 h 04 min")
        // Rounded to the minute, as Ron's `Math.round`.
        XCTAssertEqual(RulesScreenLayout.durationText(52 * 60 + 31, copy: copy), "53 min")
        XCTAssertEqual(RulesScreenLayout.durationText(-5, copy: copy), "0 min")
    }

    /// Spec rule 9 (design review of #1235, item 5): Rules offers only the
    /// repos found in the sessions of a tool the person chose to watch, and
    /// in a repo only those sessions. A repo whose tools the daemon has not
    /// named is not known to be from a watched tool, so it is left out.
    func test_rulesOffersOnlyReposOfWatchedTools() {
        var state = FirstRunState(tier: .custom, step: .rules)
        state.answer(.claudeCode, .watch(path: "/Users/someone/.claude"))
        state.answer(.codex, .off)
        let claude = ProjectRow(
            projectId: "p1", projectLabel: "one", mode: .ask,
            tools: [ProjectTool(source: "claude-code", sessionCount: 3)])
        let codex = ProjectRow(
            projectId: "p2", projectLabel: "two", mode: .ask, tools: [ProjectTool(source: "codex", sessionCount: 2)])
        let both = ProjectRow(
            projectId: "p3", projectLabel: "three", mode: .ask,
            tools: [ProjectTool(source: "claude-code", sessionCount: 1), ProjectTool(source: "codex", sessionCount: 4)])
        let unknown = ProjectRow(projectId: "p4", projectLabel: "four", mode: .ask)
        XCTAssertEqual(
            RulesScreenLayout.offered([claude, codex, both, unknown], state: state).map(\.projectId), ["p1", "p3"])
        // A repo's count is its watched tools' sessions, not the codex ones.
        XCTAssertEqual(RulesScreenLayout.watchedSessionCount(both, state: state), 1)

        let mixed = [
            session("c1"),
            PastSession(
                id: "x1", entryID: nil, state: .notQueued, selectable: true, startedAt: nil, durationSecs: nil,
                title: nil, sizeBytes: 1, source: "codex"),
        ]
        XCTAssertEqual(RulesScreenLayout.offered(mixed, state: state).map(\.id), ["c1"])

        // An exported-traces folder the person added is a watched tool too.
        state.addedFolders = [AddedFolder(kind: .trajectory, path: "/tmp/exports")]
        let exported = ProjectRow(
            projectId: "p5", projectLabel: "five", mode: .ask, tools: [ProjectTool(source: "trajectory", sessionCount: 1)])
        XCTAssertEqual(RulesScreenLayout.offered([exported], state: state).map(\.projectId), ["p5"])
    }

    /// Ron's review of #1235, item 13: the Rules picker shows its status
    /// dots, in Ron's order -- Ask me on the ask colour, Automatic on the
    /// on colour, Never on the off colour. Settings' picker is unchanged.
    func test_theRulesPickerShowsItsStatusDots() throws {
        let modes = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        let options = ProjectModeChoices.options(
            for: [.ask, .autoUpload, .ignore], copy: modes, dot: RulesScreenLayout.dot(for:))
        XCTAssertEqual(options.map(\.value), [.ask, .autoUpload, .ignore])
        XCTAssertEqual(options.map(\.dot), [.ask, .on, .off])
        XCTAssertEqual(
            ProjectModeChoices.options(for: [.ask, .autoUpload, .ignore], copy: modes).map(\.dot), [nil, nil, nil])
        let screen = try String(
            contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent(
                    "Sources/TraceCommonsApp/Views/FirstRun/RulesScreen.swift"), encoding: .utf8)
        XCTAssertTrue(screen.contains("copy: modeCopy, dot: RulesScreenLayout.dot(for:))"))
    }

    /// "{selected} of {total} selected" counts folders that are not Never.
    func test_theSummaryLeavesNeverFoldersOut() {
        let sessions = [session("a"), session("b"), session("c")]
        var state = FirstRunState(tier: .custom, step: .rules)
        let projects = [
            ProjectRow(projectId: "p1", projectLabel: "one", mode: .ask),
            ProjectRow(projectId: "p2", projectLabel: "two", mode: .ask),
        ]
        let lists = ["p1": sessions, "p2": Array(sessions.prefix(2))]
        RulesScreenLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        XCTAssertEqual(RulesScreenLayout.summary(state, projects: projects, sessions: lists).selected, 3)
        XCTAssertEqual(RulesScreenLayout.summary(state, projects: projects, sessions: lists).total, 5)

        RulesScreenLayout.setRule(&state, projectID: "p1", mode: .ignore)
        XCTAssertEqual(RulesScreenLayout.summary(state, projects: projects, sessions: lists).selected, 0)
        XCTAssertEqual(RulesScreenLayout.summary(state, projects: projects, sessions: lists).total, 2)
    }

    /// Kristi's #1235 M1, as decided: watching only keeps the picker, and
    /// the sessions picked there are queued on this Mac. The card says so
    /// with the core's line, for watching only and for nothing else.
    func test_watchingOnlySaysPastSessionsWaitLocally() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        var state = FirstRunState(tier: .custom, step: .rules)
        state.account = .watchOnly
        XCTAssertEqual(RulesScreenLayout.pastSessionsNote(state, copy: copy.rules), copy.rules.pastSessionsWatchOnly)
        for account: AccountAnswer in [.nearAI, .passkeyChosen, .passkey(name: "p"), .enrolled, .none] {
            state.account = account
            XCTAssertNil(RulesScreenLayout.pastSessionsNote(state, copy: copy.rules), "\(account)")
        }

        // Start still sends the picks for watching only: they are queued
        // locally, which is what the line says.
        state.account = .watchOnly
        state.pastSelections = ["p1": ["s1"]]
        XCTAssertTrue(FirstRunPlan.calls(for: state, at: .start).contains(.includePastSessions(projectID: "p1", ["s1"])))

        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/RulesScreen.swift")
        XCTAssertTrue(try String(contentsOf: url, encoding: .utf8).contains("RulesScreenLayout.pastSessionsNote("))
    }

    /// Ron's ruling of 2026-10-06: folders that cannot be read show the
    /// core's unavailable line with a retry, as Compute and History do, and
    /// Continue stays disabled until they load. The line names no Back link,
    /// which the first run no longer has.
    func test_unreadableFoldersOfferARetryAndHoldContinue() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        XCTAssertFalse(RulesScreenLayout.canContinue(projects: nil))
        XCTAssertTrue(RulesScreenLayout.canContinue(projects: []))
        XCTAssertEqual(RulesScreenLayout.retryTitle(copy), copy.rules.retry)
        XCTAssertEqual(copy.rules.retry, "Try again")
        XCTAssertNotEqual(copy.rules.retry, copy.folders.retry)
        XCTAssertFalse(copy.rules.unavailable.contains("Go back"))

        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/RulesScreen.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // The retry sits in the unavailable branch and reads again.
        let branch = try XCTUnwrap(source.range(of: "case .failed:"))
        let rest = source[branch.upperBound...]
        let end = try XCTUnwrap(rest.range(of: "case .loading:"))
        let failed = rest[..<end.lowerBound]
        XCTAssertTrue(failed.contains("copy.rules.unavailable"))
        XCTAssertTrue(failed.contains("RulesScreenLayout.retryTitle(copy)"))
        XCTAssertTrue(failed.contains("await load()"))
        XCTAssertTrue(source.contains("isEnabled: RulesScreenLayout.canContinue(projects: projects)"))
    }

    /// Kristi's review of #1261: while "Try again" reads the folders again,
    /// the screen shows the loading line, not the old failure with a live
    /// retry beside it. A read that fails again shows the failure again.
    func test_aRetryShowsLoadingNotTheOldFailure() throws {
        XCTAssertEqual(RulesScreenLayout.phase(projects: nil, loadFailed: false, loading: true), .loading)
        XCTAssertEqual(RulesScreenLayout.phase(projects: nil, loadFailed: true, loading: true), .loading)
        XCTAssertEqual(RulesScreenLayout.phase(projects: nil, loadFailed: true, loading: false), .failed)
        XCTAssertEqual(RulesScreenLayout.phase(projects: [], loadFailed: false, loading: false), .loaded([]))
        // Before the first read starts, it reads as loading too.
        XCTAssertEqual(RulesScreenLayout.phase(projects: nil, loadFailed: false, loading: false), .loading)

        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/RulesScreen.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("RulesScreenLayout.phase(projects: projects, loadFailed: loadFailed, loading: loading)"))
        // One read at a time: a second press while one runs does nothing.
        XCTAssertTrue(source.contains("guard projects == nil, !loading else { return }"))
    }
}
