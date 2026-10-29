import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// Limits at a glance: the cells and the pace, from the provider rows.
final class InsightsLimitsTableTests: XCTestCase {
    private typealias Window = DaemonData.SubscriptionUsageWindow
    private let now = Date(timeIntervalSince1970: 1_800_000_000)
    private let copy = [
        "analytics_unavailable": "—", "analytics_subscriptions_provider_codex": "Codex",
        "analytics_subscriptions_provider_claude": "Claude", "analytics_limits_on_pace": "On pace",
        "analytics_limits_runs_out": "Runs out in {time}",
    ]

    private func weekly(used: Int, hoursLeft: Double) -> Window {
        Window(lane: "weekly", usedPermille: used, resetsAt: now.addingTimeInterval(hoursLeft * 3_600), durationMins: 10_080)
    }

    /// Half the week gone, a quarter used: lasts until the reset.
    func testUsingLessThanTheTimePassedIsOnPace() {
        XCTAssertEqual(InsightsLimitsRows.pace(weekly(used: 250, hoursLeft: 84), now: now), .onPace)
    }

    /// A quarter of the week gone, half used: runs out a quarter-week from
    /// now, well before the reset.
    func testUsingFasterThanTheTimePassedRunsOutBeforeTheReset() {
        let pace = InsightsLimitsRows.pace(weekly(used: 500, hoursLeft: 126), now: now)
        XCTAssertEqual(pace, .runsOut(now.addingTimeInterval(42 * 3_600)))
    }

    /// Too early in the window, or a window without a length, says nothing.
    func testNoPaceWithoutEnoughOfTheWindow() {
        XCTAssertNil(InsightsLimitsRows.pace(weekly(used: 100, hoursLeft: 167), now: now))
        XCTAssertNil(InsightsLimitsRows.pace(Window(lane: "weekly", usedPermille: 500), now: now))
        XCTAssertEqual(InsightsLimitsRows.pace(weekly(used: 1_000, hoursLeft: 10), now: now), .runsOut(now))
    }

    /// One row per login found (owner, 2026-10-28): an unmeasured login
    /// keeps its row with every figure the dash; unmeasured lanes are the
    /// dash too.
    func testRowsFillOnlyMeasuredCells() {
        let codex = DaemonData.SubscriptionUsageProvider(provider: "codex", enabled: true, logins: [
            .init(id: "a", label: "me@example.com", defaultLogin: true, state: "observed", windows: [
                Window(lane: "session", usedPermille: 400, resetsAt: now.addingTimeInterval(3_600), durationMins: 300),
                weekly(used: 250, hoursLeft: 84),
            ]),
            .init(id: "b", label: "other", state: "unavailable", reason: "auth_expired"),
        ])
        let claude = DaemonData.SubscriptionUsageProvider(provider: "claude", enabled: true, logins: [
            .init(id: "c", label: "c", state: "observed", windows: [weekly(used: 500, hoursLeft: 126)]),
        ])
        let all = InsightsLimitsRows.rows([codex, claude], now: now, copy: copy)
        XCTAssertEqual(all.map(\.account), ["Codex", "Codex · other", "Claude"])
        let unmeasured = all[1]
        XCTAssertFalse(unmeasured.measured)
        XCTAssertEqual([unmeasured.session, unmeasured.weekly, unmeasured.reset, unmeasured.pace], ["—", "—", "—", "—"])
        XCTAssertFalse(unmeasured.paceWarns)
        XCTAssertEqual(unmeasured.loginId, "b")
        XCTAssertTrue(all[0].measured)
        let rows = [all[0], all[2]]
        XCTAssertEqual(rows[0].session, "60%")
        XCTAssertEqual(rows[0].weekly, "75%")
        XCTAssertEqual(rows[0].pace, "On pace")
        XCTAssertFalse(rows[0].paceWarns)
        XCTAssertEqual(rows[1].session, "—")
        XCTAssertEqual(rows[1].weekly, "50%")
        XCTAssertTrue(rows[1].pace.hasPrefix("Runs out in "))
        XCTAssertTrue(rows[1].paceWarns)
    }
}
