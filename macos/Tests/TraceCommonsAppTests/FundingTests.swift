// INTEGRATION: include in TraceCommonsAppTests alongside FundingRow, the funding
// DaemonClient/AppModel methods, and their matching FFI copy. Run --filter FundingTests.
// TC_FUNDING_SCREENSHOTS optionally retains light/dark native renders at 375/900pt.
// Hosted button tests send mouse events only to their own visible NSWindow and
// intercept OpenURLAction; no accessibility permission or external browser is used.
// Synthetic daemon/UI tests do not verify Cloud integration. Retained screenshots
// require inspection; functional button assertions do not depend on screenshots.

import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

private final class FundingDaemon: DaemonCalling {
    struct Call {
        let method: String
        let params: String
    }

    private let condition = NSCondition()
    private let response: String
    private let entered: XCTestExpectation?
    private var held: Bool
    private var recorded: [Call] = []
    private var expired = false

    init(response: String, held: Bool = false, entered: XCTestExpectation? = nil) {
        self.response = response
        self.held = held
        self.entered = entered
    }

    var calls: [Call] {
        condition.lock()
        defer { condition.unlock() }
        return recorded
    }

    var timedOut: Bool {
        condition.lock()
        defer { condition.unlock() }
        return expired
    }

    func release() {
        condition.lock()
        held = false
        condition.broadcast()
        condition.unlock()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        condition.lock()
        defer { condition.unlock() }
        recorded.append(Call(method: method, params: paramsJSON))
        guard method == "near_ai_funding" else {
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected_test_method"}}"#
        }
        entered?.fulfill()
        let deadline = Date().addingTimeInterval(5)
        while held {
            if !condition.wait(until: deadline), held {
                expired = true
                return #"{"id":1,"error":{"code":"unavailable","message":"test_barrier_timeout"}}"#
            }
        }
        return response
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

/// Queues one existing bounded daemon barrier per funding reply. A credential
/// start is refused locally so the real model can publish a busy cycle safely.
private final class FundingViewDaemon: DaemonCalling {
    private let lock = NSLock()
    private var replies: [FundingDaemon] = []
    private var allReplies: [FundingDaemon] = []
    private var recorded: [FundingDaemon.Call] = []

    var calls: [FundingDaemon.Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func enqueue(_ response: String, held: Bool = false, entered: XCTestExpectation? = nil) -> FundingDaemon {
        let reply = FundingDaemon(response: response, held: held, entered: entered)
        lock.lock()
        replies.append(reply)
        allReplies.append(reply)
        lock.unlock()
        return reply
    }

    func releaseAll() {
        lock.lock()
        let pending = allReplies
        lock.unlock()
        pending.forEach { $0.release() }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        recorded.append(.init(method: method, params: paramsJSON))
        let reply = method == "near_ai_funding" && !replies.isEmpty ? replies.removeFirst() : nil
        lock.unlock()
        return reply?.call(method, params: paramsJSON)
            ?? #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

@MainActor
private final class HostedFunding {
    let model = AppModel()
    let daemon = FundingViewDaemon()
    let hosting = NSHostingView(rootView: AnyView(EmptyView()))
    let window: NSWindow
    private let copy: PrivateInferenceCopy
    var opened: [URL] = []
    var acceptOpen = true

    init() throws {
        _ = NSApplication.shared
        copy = try XCTUnwrap(model.privateInferenceCopy, "The matching FFI copy must be available")
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 375, height: 300),
                          styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        model.setClientForTesting(DaemonClient(daemon: daemon))
    }

    func show() {
        hosting.rootView = AnyView(FundingRow(copy: copy).environmentObject(model)
            .environment(\.openURL, OpenURLAction { [weak self] url in
                guard let self else { return .discarded }
                self.opened.append(url)
                return self.acceptOpen ? .handled : .discarded
            })
            .padding(24).frame(width: 375).fixedSize(horizontal: false, vertical: true))
        window.contentView = hosting
        window.orderFront(nil)
    }

    func disappear() { hosting.rootView = AnyView(EmptyView()) }

    func close() {
        daemon.releaseAll()
        window.close()
        model.shutdown()
    }

    func draw() async throws {
        // AppKit must receive a rendering turn after the asynchronous IPC
        // continuation. Subsequent assertions inspect requests and launches,
        // so this bounded drawing interval cannot manufacture a passing result.
        try await Task.sleep(nanoseconds: 50_000_000)
        hosting.layoutSubtreeIfNeeded()
        window.setContentSize(NSSize(width: 375, height: max(1, ceil(hosting.fittingSize.height))))
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
    }

    func press() async throws {
        try await draw()
        // The real row has one action, bottom-leading inside the test's 24pt
        // padding. Its 44pt label makes this point fall inside the native button.
        let point = NSPoint(x: 75, y: 46)
        XCTAssertNotNil(hosting.hitTest(hosting.convert(point, from: nil)))
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: type, location: point,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, eventNumber: 1,
                clickCount: 1, pressure: 1))
            window.sendEvent(event)
            await Task.yield()
        }
    }
}

final class FundingTests: XCTestCase {
    private let organization = "synthetic-org_42"
    private let revision = String(repeating: "0123456789abcdef", count: 4)

    private func ready() -> [String: Any] {
        [
            "state": "ready",
            "organization_id": organization,
            "organization_name": "Synthetic organization",
            "connection_revision": revision,
            "browser_url": "https://cloud.near.ai/dashboard/organizations/\(organization)/credits",
            "observed_at": "2026-09-09T12:00:00Z",
            "view": ["message": "Cloud organization: Synthetic organization"],
        ]
    }

    private func decode(_ value: [String: Any]) throws -> FundingStatus {
        try JSONDecoder().decode(FundingStatus.self, from: JSONSerialization.data(withJSONObject: value))
    }

    private func response(_ value: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: ["id": 1, "result": value])
        return try XCTUnwrap(String(data: data, encoding: .utf8))
    }

    private func expected() throws -> FundingDestination {
        try XCTUnwrap(FundingDestination(organizationID: organization, connectionRevision: revision))
    }

    func testReadyDecodesExactAccountBindingAndCanonicalBrowserDestination() throws {
        let status = try decode(ready())
        XCTAssertEqual(status.state, "ready")
        XCTAssertEqual(status.organizationName, "Synthetic organization")
        XCTAssertEqual(status.view.message, "Cloud organization: Synthetic organization")
        XCTAssertEqual(status.destination, try expected())
        XCTAssertEqual(status.destination?.browserURL?.absoluteString, ready()["browser_url"] as? String)
    }

    func testMalformedOrganizationIDsCannotProduceDestinations() throws {
        for id in ["", ".", "..", "org/other", "org?other", "org#other", "org%2fother",
                   "org name", "org\nname", "é", String(repeating: "a", count: 129)] {
            var value = ready()
            value["organization_id"] = id
            value["browser_url"] = "https://cloud.near.ai/dashboard/organizations/\(id)/credits"
            XCTAssertNil(FundingDestination(organizationID: id, connectionRevision: revision))
            XCTAssertThrowsError(try decode(value), "Accepted malformed organization ID")
        }
    }

    func testMalformedRevisionsCannotProduceDestinations() throws {
        for candidate in ["", String(repeating: "a", count: 63), String(repeating: "a", count: 65),
                          revision.uppercased(), String(repeating: "g", count: 64),
                          String(repeating: "a", count: 63) + "\n"] {
            var value = ready()
            value["connection_revision"] = candidate
            XCTAssertNil(FundingDestination(organizationID: organization, connectionRevision: candidate))
            XCTAssertThrowsError(try decode(value), "Accepted malformed credential revision")
        }
    }

    func testReadyRefusesEveryNonexactBrowserURL() throws {
        let path = "/dashboard/organizations/\(organization)/credits"
        for url in [
            "http://cloud.near.ai\(path)", "https://cloud.near.ai:443\(path)",
            "https://user:password@cloud.near.ai\(path)", "https://cloud.near.ai.evil.invalid\(path)",
            "https://CLOUD.near.ai\(path)", "https://cloud.near.ai.\(path)",
            "https://cloud.near.ai\(path)?action=pay", "https://cloud.near.ai\(path)#fragment",
            "https://cloud.near.ai\(path)/", "//cloud.near.ai\(path)",
            "https://cloud.near.ai/dashboard/organizations/other/credits",
            "https://cloud.near.ai/dashboard/organizations/%73ynthetic-org_42/credits",
        ] {
            var value = ready()
            value["browser_url"] = url
            XCTAssertThrowsError(try decode(value), "Accepted noncanonical browser URL")
        }
    }

    func testIncompleteReadyResponsesAndUnboundedPresentationAreRejected() throws {
        for field in ["organization_id", "connection_revision", "organization_name", "browser_url", "view", "state"] {
            var value = ready()
            value.removeValue(forKey: field)
            XCTAssertThrowsError(try decode(value), "Accepted missing \(field)")
            value[field] = NSNull()
            XCTAssertThrowsError(try decode(value), "Accepted null \(field)")
        }
        for message in ["", String(repeating: "a", count: 4097)] {
            var value = ready()
            value["view"] = ["message": message]
            XCTAssertThrowsError(try decode(value))
        }
        for name in ["", String(repeating: "a", count: 1025)] {
            var value = ready()
            value["organization_name"] = name
            XCTAssertThrowsError(try decode(value))
        }
    }

    func testNonreadyStatesNeverCarryAnOtherwiseValidDestination() throws {
        for state in ["no_session", "session_expired", "no_organization", "unavailable", "future_state", "READY"] {
            var value = ready()
            value["state"] = state
            let status = try decode(value)
            XCTAssertEqual(status.state, state)
            XCTAssertNil(status.destination)
            XCTAssertNil(status.organizationName)
            let minimal = try decode(["state": state, "view": ["message": "Synthetic refusal"]])
            XCTAssertNil(minimal.destination)
        }
    }

    func testDaemonClientSendsExactlyTheFundingMethodAndOptionalBinding() throws {
        let daemon = FundingDaemon(response: try response(ready()))
        let client = DaemonClient(daemon: daemon)
        XCTAssertEqual(try client.nearAiFunding(expected: nil).destination, try expected())
        XCTAssertEqual(try client.nearAiFunding(expected: expected()).destination, try expected())
        XCTAssertEqual(daemon.calls.map(\.method), ["near_ai_funding", "near_ai_funding"])
        let params = try daemon.calls.map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.params.utf8)) as? [String: String])
        }
        XCTAssertEqual(params, [[:], ["expected_organization_id": organization, "expected_connection_revision": revision]])
        XCTAssertFalse(daemon.timedOut)
    }

    @MainActor
    func testCurrentHeldResponseIsAcceptedWithoutBlockingTheMainActor() async throws {
        try await checkHeldResponse(.current)
    }

    @MainActor
    func testReplacingClientDiscardsHeldResponse() async throws {
        try await checkHeldResponse(.replace)
    }

    @MainActor
    func testShutdownDiscardsHeldResponseAndPreventsAnotherCall() async throws {
        try await checkHeldResponse(.shutdown)
    }

    @MainActor
    func testCancellationDiscardsHeldResponse() async throws {
        try await checkHeldResponse(.cancel)
    }

    private enum WhileHeld { case current, replace, shutdown, cancel }

    @MainActor
    private func checkHeldResponse(_ action: WhileHeld) async throws {
        let entered = expectation(description: "Funding read entered off the main actor")
        let daemon = FundingDaemon(response: try response(ready()), held: true, entered: entered)
        let replacement = FundingDaemon(response: try response(ready()))
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        defer { daemon.release(); model.shutdown() }
        let binding = try expected()
        let task = Task { @MainActor in await model.nearAiFunding(expected: binding) }
        defer { task.cancel() }
        await fulfillment(of: [entered], timeout: 2)
        XCTAssertFalse(daemon.timedOut, "The main actor could not resume while IPC was held")
        XCTAssertFalse(model.credentialBusy, "Funding reads must not claim a credential write")

        switch action {
        case .current: break
        case .replace: model.setClientForTesting(DaemonClient(daemon: replacement))
        case .shutdown: model.shutdown()
        case .cancel: task.cancel()
        }
        daemon.release()
        let result = await task.value
        if case .current = action {
            XCTAssertEqual(result?.destination, binding, "Positive control must accept the current response")
        } else {
            XCTAssertNil(result, "A stale or cancelled response must not authorize a browser destination")
        }
        XCTAssertFalse(daemon.timedOut)
        XCTAssertEqual(daemon.calls.map(\.method), ["near_ai_funding"])
        XCTAssertTrue(replacement.calls.isEmpty, "A stale completion must not retry against the new client")
        if case .shutdown = action {
            let afterShutdown = await model.nearAiFunding(expected: binding)
            XCTAssertNil(afterShutdown)
            XCTAssertEqual(daemon.calls.count, 1)
        }
    }

    @MainActor
    func testHostedManageRechecksBindingAndSuppressesDuplicateClicks() async throws {
        let view = try HostedFunding()
        defer { view.close() }
        try await prime(view)
        let entered = expectation(description: "Manage reaches the daemon")
        let held = view.daemon.enqueue(try response(ready()), held: true, entered: entered)
        try await view.press()
        await fulfillment(of: [entered], timeout: 2)
        try assertLastFundingCall(view, bound: true)
        try await view.press()
        XCTAssertEqual(view.daemon.calls.count, 2, "A pending button must not submit twice")
        XCTAssertTrue(view.opened.isEmpty)
        held.release()
        try await view.draw()
        XCTAssertFalse(held.timedOut)
        XCTAssertEqual(view.opened.map(\.absoluteString), [try XCTUnwrap(expected().browserURL).absoluteString])
    }

    @MainActor
    func testHostedManageRefusesChangedOrganizationOrRevisionAndRecovers() async throws {
        for field in ["organization_id", "connection_revision"] {
            let view = try HostedFunding()
            defer { view.close() }
            try await prime(view)
            var changed = ready()
            if field == "organization_id" {
                changed[field] = "synthetic-other"
                changed["browser_url"] = "https://cloud.near.ai/dashboard/organizations/synthetic-other/credits"
            } else {
                changed[field] = String(repeating: "b", count: 64)
            }
            try await click(view, reply: changed, bound: true)
            XCTAssertTrue(view.opened.isEmpty, "A changed displayed binding must not open")
            try await click(view, reply: ready(), bound: false)
            XCTAssertTrue(view.opened.isEmpty, "Recovery must refresh before another Manage action")
            try await click(view, reply: ready(), bound: true)
            XCTAssertEqual(view.opened.count, 1)
        }
    }

    @MainActor
    func testHostedHeldManageCannotSurviveDisappearanceOrCredentialBusyCycle() async throws {
        for disappears in [false, true] {
            let view = try HostedFunding()
            defer { view.close() }
            try await prime(view)
            let entered = expectation(description: "Manage is held before invalidation")
            let held = view.daemon.enqueue(try response(ready()), held: true, entered: entered)
            try await view.press()
            await fulfillment(of: [entered], timeout: 2)
            try assertLastFundingCall(view, bound: true)
            if disappears {
                view.disappear()
                try await view.draw()
            } else {
                // This real model method publishes both busy values. The fake
                // refuses the start, so no wallet or credential is created.
                let browser = await view.model.startNearAiCredential()
                XCTAssertNil(browser)
                XCTAssertFalse(view.model.credentialBusy)
            }
            held.release()
            try await view.draw()
            XCTAssertFalse(held.timedOut)
            XCTAssertTrue(view.opened.isEmpty, "An invalidated reply must never open")
            if disappears {
                try await prime(view)
            } else {
                try await click(view, reply: ready(), bound: false)
                XCTAssertEqual(view.daemon.calls.filter { $0.method == "near_ai_credential_start" }.count, 1)
            }
            XCTAssertTrue(view.opened.isEmpty)
            try await click(view, reply: ready(), bound: true)
            XCTAssertEqual(view.opened.count, 1, "The replacement action must remain usable")
        }
    }

    @MainActor
    func testHostedBrowserRefusalClearsTheBindingBeforeRecovery() async throws {
        let view = try HostedFunding()
        defer { view.close() }
        try await prime(view)
        view.acceptOpen = false
        try await click(view, reply: ready(), bound: true)
        XCTAssertEqual(view.opened.count, 1)
        view.acceptOpen = true
        try await click(view, reply: ready(), bound: false)
        XCTAssertEqual(view.opened.count, 1, "Recovery refresh must not open a browser")
        try await click(view, reply: ready(), bound: true)
        XCTAssertEqual(view.opened.count, 2)
    }

    @MainActor
    private func prime(_ view: HostedFunding) async throws {
        let entered = expectation(description: "Visible row reads its account")
        _ = view.daemon.enqueue(try response(ready()), entered: entered)
        view.show()
        await fulfillment(of: [entered], timeout: 2)
        try await view.draw()
        try assertLastFundingCall(view, bound: false)
        XCTAssertTrue(view.opened.isEmpty, "Appearing must not open a browser")
    }

    @MainActor
    private func click(_ view: HostedFunding, reply: [String: Any], bound: Bool) async throws {
        let entered = expectation(description: "Native button reaches the daemon")
        _ = view.daemon.enqueue(try response(reply), entered: entered)
        try await view.press()
        await fulfillment(of: [entered], timeout: 2)
        try await view.draw()
        try assertLastFundingCall(view, bound: bound)
    }

    @MainActor
    private func assertLastFundingCall(_ view: HostedFunding, bound: Bool) throws {
        let call = try XCTUnwrap(view.daemon.calls.last { $0.method == "near_ai_funding" })
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(call.params.utf8)) as? [String: String])
        XCTAssertEqual(params, bound
            ? ["expected_organization_id": organization, "expected_connection_revision": revision] : [:])
    }

    @MainActor
    func testSyntheticFundingRowScreenshotMatrix() async throws {
        guard let path = ProcessInfo.processInfo.environment["TC_FUNDING_SCREENSHOTS"], !path.isEmpty else {
            throw XCTSkip("Set TC_FUNDING_SCREENSHOTS to retain the synthetic native funding matrix")
        }
        let directory = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        _ = NSApplication.shared
        for width in [375, 900] {
            var images: [Data] = []
            for (label, scheme) in [("light", ColorScheme.light), ("dark", ColorScheme.dark)] {
                let model = AppModel()
                let copy = try XCTUnwrap(model.privateInferenceCopy, "The matching FFI copy must be available")
                let entered = expectation(description: "Native funding row reads its status")
                let daemon = FundingDaemon(response: try response(ready()), entered: entered)
                model.setClientForTesting(DaemonClient(daemon: daemon))
                var opened: [URL] = []
                let content = FundingRow(copy: copy).environmentObject(model)
                    .environment(\.openURL, OpenURLAction { url in opened.append(url); return .discarded })
                    .padding(24).frame(width: CGFloat(width), alignment: .topLeading)
                    .fixedSize(horizontal: false, vertical: true)
                    .background(Color(nsColor: .windowBackgroundColor))
                    .environment(\.colorScheme, scheme)
                let hosting = NSHostingView(rootView: content)
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 600),
                                      styleMask: [.borderless], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false
                window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                window.contentView = hosting
                defer { window.close(); model.shutdown() }
                await fulfillment(of: [entered], timeout: 3)
                // Request entry is earlier than its main-actor completion and
                // SwiftUI drawing. Yielding can immediately resume this test;
                // allow a bounded drawing interval instead. These optional
                // artifacts require manual Ready-state and clipping inspection.
                try await Task.sleep(nanoseconds: 100_000_000)
                hosting.layoutSubtreeIfNeeded()
                let height = ceil(hosting.fittingSize.height)
                XCTAssertGreaterThan(height, 80)
                XCTAssertLessThan(height, 1000)
                let bounds = NSRect(x: 0, y: 0, width: CGFloat(width), height: height)
                window.setContentSize(bounds.size)
                hosting.frame = bounds
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
                let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
                hosting.cacheDisplay(in: bounds, to: bitmap)
                let image = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                XCTAssertGreaterThan(image.count, 1000)
                images.append(image)
                try image.write(to: directory.appendingPathComponent("funding-ready-\(width)-\(label).png"))
                XCTAssertTrue(opened.isEmpty, "Rendering funding must never open a browser")
                XCTAssertEqual(daemon.calls.map(\.method), ["near_ai_funding"])
            }
            XCTAssertNotEqual(images[0], images[1], "The native view must honor both color appearances")
        }
    }
}
