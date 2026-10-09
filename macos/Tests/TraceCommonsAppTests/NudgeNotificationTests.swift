import UserNotifications
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The re-engagement notification and the digest: posted in the daemon's
/// words, unchanged, with the daemon's buttons, and each button routed to
/// the request and place its id names.
final class NudgeNotificationTests: XCTestCase {
    private static let idleFrame = #"""
        {"event":"reengage_due","data":{"kind":"idle_sessions","title":"Trace Commons",
         "body":"2 sessions from Codex have been idle for 3 days or more. Review them to send or keep.",
         "actions":[{"id":"review","label":"Review"},{"id":"not_now","label":"Not now"}]}}
        """#

    private static var idle: DaemonData.ReengageDue {
        DaemonData.ReengageDue(
            kind: "idle_sessions", title: "Trace Commons",
            body: "2 sessions from Codex have been idle for 3 days or more. Review them to send or keep.",
            actions: [.init(id: "review", label: "Review"), .init(id: "not_now", label: "Not now")])
    }

    func testTheAppParsesReengageDueAsItsOwnEvent() {
        XCTAssertEqual(DaemonEventParser.parse(Self.idleFrame), .reengageDue(Self.idle))
        XCTAssertEqual(DaemonEventParser.parse(#"{"event":"reengage_due","data":{}}"#), .unknown("reengage_due"))
    }

    /// The title and body are posted as the daemon wrote them, and the
    /// buttons are the daemon's, in its order, under a category for that
    /// set of buttons.
    func testAReengagementIsPostedInTheDaemonsWords() throws {
        let plan = try XCTUnwrap(Notifier.plan(Self.idle))
        XCTAssertEqual(plan.title, "Trace Commons")
        XCTAssertEqual(plan.body, Self.idle.body)
        XCTAssertEqual(plan.buttons.map(\.label), ["Review", "Not now"])
        XCTAssertEqual(plan.buttons.map(\.identifier), ["trace-commons.nudge.review", "trace-commons.nudge.not_now"])
        XCTAssertEqual(plan.buttons.map(\.opensApp), [true, false])
        XCTAssertEqual(plan.categoryIdentifier, "trace-commons.nudge.review+not_now")
        XCTAssertEqual(plan.userInfo[Notifier.kindKey], "idle_sessions")
        XCTAssertEqual(plan.userInfo[Notifier.defaultActionKey], "review")
        // A kind this build does not know is not posted.
        XCTAssertNil(Notifier.plan(.init(kind: "weekly_recap", title: "T", body: "B", actions: [])))
    }

    /// Each button, and a click on the notification itself, resolves to
    /// the intent its id names on that kind; a dismissal resolves to none.
    func testEachResponseResolvesToItsIntent() throws {
        let info = try XCTUnwrap(Notifier.plan(Self.idle)).userInfo
        XCTAssertEqual(Notifier.nudgeIntent(actionIdentifier: "trace-commons.nudge.review", userInfo: info),
                       .review(.idleSessions))
        XCTAssertEqual(Notifier.nudgeIntent(actionIdentifier: "trace-commons.nudge.not_now", userInfo: info),
                       .notNow(.idleSessions))
        XCTAssertEqual(Notifier.nudgeIntent(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: info),
                       .review(.idleSessions))
        XCTAssertNil(Notifier.nudgeIntent(actionIdentifier: UNNotificationDismissActionIdentifier, userInfo: info))
        // Not a nudge (the digest), or a kind it cannot act on.
        XCTAssertNil(Notifier.nudgeIntent(actionIdentifier: "trace-commons.nudge.review", userInfo: [:]))
        XCTAssertNil(Notifier.nudgeIntent(
            actionIdentifier: "trace-commons.nudge.not_now", userInfo: [Notifier.kindKey: "verdicts_landed"]))

        let verdicts = try XCTUnwrap(Notifier.plan(.init(
            kind: "verdicts_landed", title: "Trace Commons", body: "2 sessions accepted and 1 held for privacy review.",
            actions: [.init(id: "see_history", label: "See history")])))
        XCTAssertEqual(Notifier.nudgeIntent(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: verdicts.userInfo),
                       .seeHistory)
    }

    /// The digest posts the core's text unchanged, the folded sentence
    /// included; only a daemon that sent none falls back to the counts.
    func testTheDigestPostsTheCoresTextUnchanged() {
        let text = "2 sessions are waiting. Nothing is sent until you review them. 2 sessions from Codex have been idle for 3 days or more."
        XCTAssertEqual(Notifier.digestBody(text: text, pendingCount: 2, projects: ["api"]), text)
        XCTAssertEqual(Notifier.digestBody(text: "", pendingCount: 0, projects: []), nil)
        XCTAssertNotNil(Notifier.digestBody(text: "", pendingCount: 1, projects: ["api"]))
    }

    /// The digest's two buttons are the core's words.
    func testTheDigestButtonsAreTheCoresWords() {
        let copy = NudgeCopy(table: ["DIGEST_ACTION_REVIEW": "Review", "DIGEST_ACTION_NOT_NOW": "Not now"])
        XCTAssertEqual(Notifier.digestButtons(copy).map(\.label), ["Review", "Not now"])
        XCTAssertEqual(Notifier.digestButtons(nil), [])
    }
}
