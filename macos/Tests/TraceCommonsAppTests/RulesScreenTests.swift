import Foundation
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
        var state = FirstRunState(tier: .custom, step: .rules)

        // One write per tickable row, as the native group toggle sends them.
        for _ in sessions.filter(FirstRunRulesLayout.isTickable) {
            FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        }
        XCTAssertEqual(state.pastSelections["p1"], ["a", "b"])
        XCTAssertEqual(FirstRunRulesLayout.groupState(state, projectID: "p1", sessions: sessions), .all)

        // One unticked: the group reads mixed, and turning it on selects all.
        FirstRunRulesLayout.setTicked(&state, projectID: "p1", session: sessions[0], on: false)
        XCTAssertEqual(FirstRunRulesLayout.groupState(state, projectID: "p1", sessions: sessions), .some)
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        XCTAssertEqual(state.pastSelections["p1"], ["a", "b"])

        // Off clears the folder, however many rows the write reaches.
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: false)
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: false)
        XCTAssertNil(state.pastSelections["p1"])
        XCTAssertEqual(FirstRunRulesLayout.groupState(state, projectID: "p1", sessions: sessions), .none)

        // The selection is what the plan sends as the person's approval.
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        XCTAssertEqual(
            FirstRunPlan.calls(for: state, at: .start).filter {
                if case .includePastSessions = $0 { return true }
                return false
            },
            [.includePastSessions(projectID: "p1", ["a", "b"])])
    }

    /// Setting a folder to Never clears its selection (Ron's `applyRule`),
    /// and the folder's other rules leave it alone.
    func test_neverClearsTheFolderSelection() {
        let sessions = [session("a"), session("b")]
        var state = FirstRunState(tier: .custom, step: .rules)
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        FirstRunRulesLayout.includeEvery(&state, projectID: "p2", sessions: sessions, on: true)

        FirstRunRulesLayout.setRule(&state, projectID: "p1", mode: .ask)
        XCTAssertEqual(state.pastSelections["p1"], ["a", "b"])

        FirstRunRulesLayout.setRule(&state, projectID: "p1", mode: .ignore)
        XCTAssertEqual(state.rules["p1"], .ignore)
        XCTAssertNil(state.pastSelections["p1"])
        XCTAssertEqual(state.pastSelections["p2"], ["a", "b"])

        // A Never folder cannot be ticked back while its rule is Never.
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        FirstRunRulesLayout.setTicked(&state, projectID: "p1", session: sessions[0], on: true)
        XCTAssertNil(state.pastSelections["p1"])
    }

    /// A rule the person never touched is not written: the picker shows the
    /// daemon's mode, and the plan sends no `setProjectMode` for it.
    func test_anUntouchedRuleIsNotSent() {
        let project = ProjectRow(projectId: "p1", projectLabel: "repo", mode: .ask)
        let state = FirstRunState(tier: .custom, step: .rules)
        XCTAssertEqual(FirstRunRulesLayout.rule(state, for: project), .ask)
        XCTAssertTrue(state.rules.isEmpty)
    }

    /// A `still_active` row cannot be ticked, whatever the wire said about
    /// it, by the folder box or one by one.
    func test_aStillActiveSessionCannotBeTicked() {
        let active = session("live", state: .stillActive, selectable: true)
        XCTAssertFalse(FirstRunRulesLayout.isTickable(active))

        var state = FirstRunState(tier: .custom, step: .rules)
        FirstRunRulesLayout.setTicked(&state, projectID: "p1", session: active, on: true)
        XCTAssertNil(state.pastSelections["p1"])
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: [active], on: true)
        XCTAssertNil(state.pastSelections["p1"])

        // A row the daemon says may not be ticked is refused the same way.
        let refused = session("x", state: .pending, selectable: false)
        XCTAssertFalse(FirstRunRulesLayout.isTickable(refused))
        XCTAssertTrue(FirstRunRulesLayout.isTickable(session("ok")))
    }

    /// A `not_queued` row was never opened: it shows its date and size. A
    /// row with a title shows date, title and duration, and no size.
    func test_rowsWithoutATitleShowDateAndSize() {
        let date = FirstRunRulesLayout.dateText(Self.started)
        let size = FirstRunRulesLayout.sizeText(2_048)
        XCTAssertFalse(date.isEmpty)
        XCTAssertFalse(size.isEmpty)

        XCTAssertEqual(FirstRunRulesLayout.labelParts(session("a")), [date, size])

        let titled = session("b", state: .pending, title: "Fix the parser", durationSecs: 3_120)
        let duration = FirstRunRulesLayout.durationText(3_120)
        XCTAssertFalse(duration.isEmpty)
        XCTAssertEqual(FirstRunRulesLayout.labelParts(titled), [date, "Fix the parser", duration])

        // No start time: the date part is left out, never replaced.
        let undated = PastSession(
            id: "c", entryID: nil, state: .notQueued, selectable: true, startedAt: nil,
            durationSecs: nil, title: nil, sizeBytes: 2_048, source: "codex")
        XCTAssertEqual(FirstRunRulesLayout.labelParts(undated), [size])
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
        FirstRunRulesLayout.includeEvery(&state, projectID: "p1", sessions: sessions, on: true)
        XCTAssertEqual(FirstRunRulesLayout.summary(state, projects: projects, sessions: lists).selected, 3)
        XCTAssertEqual(FirstRunRulesLayout.summary(state, projects: projects, sessions: lists).total, 5)

        FirstRunRulesLayout.setRule(&state, projectID: "p1", mode: .ignore)
        XCTAssertEqual(FirstRunRulesLayout.summary(state, projects: projects, sessions: lists).selected, 0)
        XCTAssertEqual(FirstRunRulesLayout.summary(state, projects: projects, sessions: lists).total, 2)
    }
}
