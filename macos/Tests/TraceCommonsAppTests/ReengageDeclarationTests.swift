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
        let recorder = RedeclareRecorder()
        let declaration = recorder.declaration()

        declaration.update(accepts: [])
        XCTAssertEqual(recorder.calls, [], "a denied app declared")
        XCTAssertFalse(declaration.isDeclared)

        declaration.update(accepts: ["reengage_due"])
        XCTAssertEqual(recorder.calls, [Call(token: 0, accepts: ["reengage_due"])])
        XCTAssertTrue(declaration.isDeclared)
        XCTAssertEqual(declaration.token, 1, "the replacement subscription is the one kept")

        // Asked again with the same answer (every time the app comes
        // forward): nothing is re-registered.
        declaration.update(accepts: ["reengage_due"])
        XCTAssertEqual(recorder.calls.count, 1)

        // Permission withdrawn in System Settings: the declaration goes, on
        // the same subscription.
        declaration.update(accepts: [])
        XCTAssertEqual(recorder.calls.last, Call(token: 1, accepts: []))
        XCTAssertFalse(declaration.isDeclared)
        XCTAssertEqual(declaration.token, 2)
    }

    /// A refused re-registration leaves the old subscription and its
    /// declaration standing, so the next update tries again.
    @MainActor
    func testARefusedRedeclarationIsRetried() {
        let recorder = RedeclareRecorder()
        let declaration = recorder.declaration()
        recorder.refuse = true
        declaration.update(accepts: ["reengage_due"])
        XCTAssertFalse(declaration.isDeclared)
        XCTAssertEqual(declaration.token, 0)
        recorder.refuse = false
        declaration.update(accepts: ["reengage_due"])
        XCTAssertTrue(declaration.isDeclared)
        XCTAssertEqual(recorder.calls.count, 2)
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

    /// Attached to a daemon another process runs, the declaration rides
    /// on the app's one subscription: the screens keep receiving frames
    /// after it is made. (A second subscription on an attached handle
    /// would take the first one's event sink.)
    @MainActor
    func testAnAttachedAppStillHearsTheDaemonAfterDeclaring() async throws {
        let directory = URL(fileURLWithPath: "/private/tmp/tc-ra-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let incumbent = try TCDaemon(configDir: directory.path, settingsJSON: settings)
        defer { incumbent.shutdown() }

        let model = AppModel(reengageAccepts: { ["reengage_due"] })
        defer { model.shutdown() }
        let started = expectation(description: "attached")
        model.startDaemon(at: directory.path, settingsJSON: settings) { _ in started.fulfill() }
        await fulfillment(of: [started], timeout: 10)
        XCTAssertTrue(model.isAttachedDaemon)
        await model.refreshReengageDeclaration()
        XCTAssertTrue(model.declaresReengagement)

        // Let the start-up reads settle, then move status from the other
        // side; only a frame on the app's subscription can tell it.
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertFalse(model.status.paused)
        _ = incumbent.call("pause", params: "{}")
        let deadline = Date().addingTimeInterval(10)
        while !model.status.paused, Date() < deadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertTrue(model.status.paused, "the app stopped hearing the daemon")
    }
}

private struct Call: Equatable {
    let token: Int
    let accepts: [String]
}

@MainActor
private final class RedeclareRecorder {
    var calls: [Call] = []
    var refuse = false
    private var next = 0

    func declaration() -> ReengageDeclaration<Int> {
        ReengageDeclaration(token: 0) { [unowned self] token, accepts in
            calls.append(Call(token: token, accepts: accepts))
            guard !refuse else { return nil }
            next += 1
            return next
        }
    }
}
