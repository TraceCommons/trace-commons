import Foundation
import TCDesign
@testable import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class MenuBarTraceAgeTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    func testGraphThresholdIncludesExactlyThreeDays() {
        XCTAssertFalse(MenuPanelData.showsGraph(firstTraceAt: nil, now: now))
        XCTAssertFalse(MenuPanelData.showsGraph(firstTraceAt: now.addingTimeInterval(1), now: now))
        XCTAssertFalse(MenuPanelData.showsGraph(firstTraceAt: now.addingTimeInterval(-259_199), now: now))
        XCTAssertTrue(MenuPanelData.showsGraph(firstTraceAt: now.addingTimeInterval(-259_200), now: now))
        XCTAssertTrue(MenuPanelData.showsGraph(firstTraceAt: now.addingTimeInterval(-259_201), now: now))
    }

    func testStatusAgeDoesNotDependOnCappedOrFailedLists() async {
        let transport = TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-345_600))
        transport.failing = ["list_history", "list_pending"]
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        store.clock = { self.now }
        await store.load()
        XCTAssertEqual(store.firstTraceAt, now.addingTimeInterval(-345_600))
        XCTAssertTrue(store.stripShowsGraph)
        XCTAssertFalse(store.columnsCurrent, "the age does not make failed graph reads current")
        // A fresh store learns the same age without reading app defaults.
        let reopened = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        reopened.clock = { self.now }
        await reopened.load()
        XCTAssertTrue(reopened.stripShowsGraph)
    }

    func testMissingAndNullAgeNeverInferAgeFromAnotherRead() async {
        for includeField in [false, true] {
            let store = MenuPanelStore(client: LiveDaemonClient(
                transport: TraceAgeTransport(firstTraceAt: nil, includeField: includeField)))
            store.clock = { self.now }
            await store.load()
            XCTAssertNil(store.firstTraceAt)
            XCTAssertFalse(store.stripShowsGraph)
        }
    }

    func testLoadingKeepsTheExistingWideLayoutUntilStatusAnswers() async {
        let store = MenuPanelStore(client: LiveDaemonClient(
            transport: TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-345_600))))
        store.clock = { self.now }
        XCTAssertTrue(store.stripShowsGraph, "unknown age should not briefly collapse an existing user's item")
        await store.load()
        XCTAssertTrue(store.stripShowsGraph)
    }

    func testAccountChangeClearsAgeEvenWhenTheListsFail() async {
        let transport = TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-345_600))
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        store.clock = { self.now }
        await store.load()
        XCTAssertTrue(store.stripShowsGraph)
        // Account changes can use the same daemon connection.
        transport.firstTraceAt = nil
        transport.failing = ["list_history", "list_pending"]
        await store.load()
        XCTAssertNil(store.firstTraceAt)
        XCTAssertFalse(store.stripShowsGraph)
    }

    func testAttachDiscardsPreviousClientDataBeforeAnyNewRead() async {
        let store = MenuPanelStore(client: LiveDaemonClient(
            transport: TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-345_600))))
        store.clock = { self.now }
        await store.load()
        let fresh = TraceAgeTransport(firstTraceAt: nil)
        fresh.failing = ["list_history", "list_pending"]
        store.attach(LiveDaemonClient(transport: fresh), configDirectory: "/new-config")
        XCTAssertNil(store.firstTraceAt)
        XCTAssertNil(store.status)
        XCTAssertTrue(store.history.isEmpty)
        XCTAssertTrue(store.pending.isEmpty)
        await store.load()
        XCTAssertFalse(store.stripShowsGraph)
        XCTAssertTrue(store.history.isEmpty)
        XCTAssertTrue(store.pending.isEmpty)
        store.attach(nil)
        XCTAssertNil(store.configDirectory)
        XCTAssertNil(store.firstTraceAt)
    }

    func testOldClientCannotPublishAfterAttach() async {
        let entered = expectation(description: "old status read started")
        let release = DispatchSemaphore(value: 0)
        let old = TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-345_600))
        old.beforeStatus = { entered.fulfill(); release.wait() }
        let store = MenuPanelStore(client: LiveDaemonClient(transport: old))
        store.clock = { self.now }
        let loading = Task { await store.load() }
        await fulfillment(of: [entered], timeout: 5)
        store.attach(LiveDaemonClient(transport: TraceAgeTransport(firstTraceAt: nil)))
        await store.load()
        release.signal()
        await loading.value
        XCTAssertNil(store.firstTraceAt)
        XCTAssertFalse(store.stripShowsGraph)
    }

    func testOlderLoadCannotRestoreAgeAfterANewerLoadOnTheSameClient() async {
        let entered = expectation(description: "old status captured")
        let release = DispatchSemaphore(value: 0)
        let transport = TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-345_600))
        transport.beforeStatus = { entered.fulfill(); release.wait() }
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        store.clock = { self.now }
        let older = Task { await store.load() }
        await fulfillment(of: [entered], timeout: 5)
        transport.firstTraceAt = nil
        await store.load()
        XCTAssertFalse(store.stripShowsGraph)
        release.signal()
        await older.value
        XCTAssertNil(store.firstTraceAt)
        XCTAssertFalse(store.stripShowsGraph)
    }

    func testClockCrossingThresholdInvalidatesTheRenderedInputs() async {
        let store = MenuPanelStore(client: LiveDaemonClient(
            transport: TraceAgeTransport(firstTraceAt: now.addingTimeInterval(-259_199))))
        store.clock = { self.now }
        await store.load()
        let model = AppModel()
        let before = MenuBarStripInputs.of(model: model, store: store, scale: 2)
        XCTAssertFalse(before.graph)
        store.clock = { self.now.addingTimeInterval(1) }
        await store.loadActivity()
        let after = MenuBarStripInputs.of(model: model, store: store, scale: 2)
        XCTAssertTrue(after.graph)
        XCTAssertNotEqual(before, after)
    }
}

private final class TraceAgeTransport: DaemonTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var age: Date?
    private var failures: Set<String> = []
    private let includeField: Bool
    private var statusHook: (@Sendable () -> Void)?

    var beforeStatus: (@Sendable () -> Void)? {
        get { lock.withLock { statusHook } }
        set { lock.withLock { statusHook = newValue } }
    }

    init(firstTraceAt: Date?, includeField: Bool = true) {
        age = firstTraceAt
        self.includeField = includeField
    }

    var firstTraceAt: Date? {
        get { lock.withLock { age } }
        set { lock.withLock { age = newValue } }
    }

    var failing: Set<String> {
        get { lock.withLock { failures } }
        set { lock.withLock { failures = newValue } }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        if failing.contains(method) {
            return #"{"id":0,"error":{"code":"unavailable","message":"read-failed"}}"#
        }
        if method == "status" {
            let date = firstTraceAt.map { "\"\(ISO8601DateFormatter().string(from: $0))\"" } ?? "null"
            let hook = lock.withLock {
                defer { statusHook = nil }
                return statusHook
            }
            hook?()
            let ageField = includeField ? ",\"first_trace_at\":\(date)" : ""
            return "{\"id\":0,\"result\":{\"logged_in\":true\(ageField)}}"
        }
        guard let reply = SampleDaemonData.reply(method, in: .normalDay) else {
            return #"{"id":0,"error":{"code":"unknown_method","message":"unknown-method"}}"#
        }
        return #"{"id":0,"result":\#(reply)}"#
    }
}
