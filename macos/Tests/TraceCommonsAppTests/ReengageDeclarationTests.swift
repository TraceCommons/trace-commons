import Foundation
import TCBridge
import UserNotifications
import XCTest
@testable import TraceCommonsApp

/// The `reengage_due` declaration stands only while this app can post the
/// notification it announces. The daemon stamps each announcement against
/// its caps, and retires the idle sessions it named, only while a
/// subscriber has declared it; a declaration made while the system refuses
/// notifications spends that budget on notifications nobody sees.
final class ReengageDeclarationTests: XCTestCase {
    func testOnlyAPostableStatusDeclaresReengagement() {
        for status in [UNAuthorizationStatus.authorized, .provisional] {
            XCTAssertEqual(Notifier.acceptedEvents(available: true, status: status), ["reengage_due"], "\(status)")
        }
        for status: UNAuthorizationStatus? in [.denied, .notDetermined, nil, UNAuthorizationStatus(rawValue: 999)] {
            XCTAssertEqual(Notifier.acceptedEvents(available: true, status: status), [], "\(String(describing: status))")
        }
        // No notification centre at all (a bare binary): nothing is drawn.
        XCTAssertEqual(Notifier.acceptedEvents(available: false, status: .authorized), [])
    }

    @MainActor
    func testTheDeclarationFollowsWhatCanBePosted() {
        let recorder = SubscribeRecorder()
        let declaration = recorder.declaration()

        declaration.update(accepts: [])
        XCTAssertEqual(recorder.subscribed, [], "a denied app declared")
        XCTAssertFalse(declaration.isDeclared)

        declaration.update(accepts: ["reengage_due"])
        XCTAssertEqual(recorder.subscribed, [["reengage_due"]])
        XCTAssertTrue(declaration.isDeclared)

        // Asked again with the same answer: still one declaration.
        declaration.update(accepts: ["reengage_due"])
        XCTAssertEqual(recorder.subscribed, [["reengage_due"]])

        // Permission withdrawn in System Settings: the declaration goes.
        declaration.update(accepts: [])
        XCTAssertEqual(recorder.unsubscribed, [1])
        XCTAssertFalse(declaration.isDeclared)
    }

    /// An unsubscribe the ABI refused leaves the declaration standing, so
    /// the next update tries again rather than forgetting it.
    @MainActor
    func testARefusedWithdrawalIsRetried() {
        let recorder = SubscribeRecorder()
        let declaration = recorder.declaration()
        declaration.update(accepts: ["reengage_due"])
        recorder.refuseUnsubscribe = true
        declaration.update(accepts: [])
        XCTAssertTrue(declaration.isDeclared)
        recorder.refuseUnsubscribe = false
        declaration.withdraw()
        XCTAssertFalse(declaration.isDeclared)
        XCTAssertEqual(recorder.unsubscribed, [1, 1])
    }

    /// The declaring subscription hands on `reengage_due` alone: the app's
    /// plain subscription already delivers every ordinary frame.
    func testTheDeclaringSubscriptionForwardsOnlyReengagement() {
        let frame = #"""
            {"event":"reengage_due","data":{"kind":"idle_sessions","title":"T","body":"B",
             "actions":[{"id":"review","label":"R"}]}}
            """#
        XCTAssertEqual(ReengageDeclaration<Int>.reengageDue(frame)?.kind, "idle_sessions")
        XCTAssertNil(ReengageDeclaration<Int>.reengageDue(#"{"event":"status_changed","data":{}}"#))
        XCTAssertNil(ReengageDeclaration<Int>.reengageDue(#"{"event":"reengage_due","data":{}}"#))
    }

    private let settings = #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#

    /// The app wires it: against a real daemon, a postable status declares
    /// through the app's own subscribe, and a denied one does not.
    @MainActor
    func testTheAppDeclaresOnlyWhileItCanPost() async throws {
        for (accepts, declared) in [([String](), false), (["reengage_due"], true)] {
            let directory = URL(fileURLWithPath: "/private/tmp/tc-rd-\(UUID().uuidString.prefix(8))")
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: directory) }
            let model = AppModel(reengageAccepts: { accepts })
            defer { model.shutdown() }
            let started = expectation(description: "running")
            model.startDaemon(at: directory.path, settingsJSON: settings) { _ in started.fulfill() }
            await fulfillment(of: [started], timeout: 10)
            XCTAssertEqual(model.startup, .running)
            await model.refreshReengageDeclaration()
            XCTAssertEqual(model.declaresReengagement, declared, "\(accepts)")
        }
    }
}

@MainActor
private final class SubscribeRecorder {
    var subscribed: [[String]] = []
    var unsubscribed: [Int] = []
    var refuseUnsubscribe = false
    private var next = 0

    func declaration() -> ReengageDeclaration<Int> {
        ReengageDeclaration(
            subscribe: { [unowned self] accepts, _ in
                subscribed.append(accepts)
                next += 1
                return next
            },
            unsubscribe: { [unowned self] token in
                unsubscribed.append(token)
                return !refuseUnsubscribe
            },
            deliver: { _ in })
    }
}
