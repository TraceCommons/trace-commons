import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

final class SubscriptionUsageTests: XCTestCase {
    private static let now = Date(timeIntervalSince1970: 1_792_000_000)
    private static func snapshot(enabled: Bool = true, refreshing: Bool = false) -> DaemonData.SubscriptionUsage {
        .init(providers: [.init(provider: "codex", enabled: enabled, refreshing: refreshing,
                               logins: enabled ? [.init(id: "codex-default", label: "Codex", defaultLogin: true,
                                                       state: "observed", windows: [
                                                        .init(lane: "weekly", usedPermille: 300, remainingText: "70%",
                                                              resetsAt: now.addingTimeInterval(3600))], bankedResets: 2)] : [])])
    }
    private func uiCopy() throws -> [String: String] { try XCTUnwrap(TCInsights.copy()) }

    @MainActor
    private func waitForCalls(_ gate: UsageReaderGate, _ count: Int,
                              file: StaticString = #filePath, line: UInt = #line) async {
        let ready = await gate.waitForCalls(count)
        XCTAssertTrue(ready, file: file, line: line)
    }

    @MainActor
    func testNoReaderAndOlderDaemonNeverSubstituteSampleUsage() async {
        let absent = SubscriptionUsageModel()
        await absent.load(refresh: true, interactive: true)
        XCTAssertFalse(absent.isVisible)
        let old = SubscriptionUsageModel(reader: { _, _, _ in
            throw DaemonDataError.daemon(code: "unknown_method", message: "unknown-method")
        })
        await old.load()
        XCTAssertFalse(old.isVisible)
        XCTAssertNil(old.shown)
        XCTAssertNil(old.failureKey)
    }

    @MainActor
    func testRefreshesCoalesceAndOnlyExplicitRefreshIsInteractive() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        })
        let first = Task { await model.load() }
        await waitForCalls(gate, 1)
        await model.load(refresh: true, interactive: true)
        await model.load(refresh: true, interactive: true)
        let actual1 = await gate.count()
        XCTAssertEqual(actual1, 1)
        await gate.resolve(0, value: Self.snapshot())
        await first.value
        await waitForCalls(gate, 2)
        let actual3 = await gate.flags(0)
        XCTAssertEqual(actual3, [false, false])
        let actual4 = await gate.flags(1)
        XCTAssertEqual(actual4, [true, true])
        await gate.resolve(1, value: Self.snapshot())
        for _ in 0..<50 where model.isRefreshing { await Task.yield() }
        let actual2 = await gate.count()
        XCTAssertEqual(actual2, 2)
    }

    @MainActor
    func testStoppingHidesUsageAndDiscardsLateRead() async {
        let gate = UsageReaderGate(initial: Self.snapshot())
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        }, writer: { _, _ in })
        await model.load()
        let old = Task { await model.load() }
        await waitForCalls(gate, 2)
        let stop = Task { await model.setEnabled("codex", on: false) }
        await waitForCalls(gate, 3)
        XCTAssertEqual(model.shown?.providers.first?.enabled, false)
        XCTAssertEqual(model.shown?.providers.first?.logins, [])
        await gate.resolve(2, value: Self.snapshot(enabled: false))
        await stop.value
        await gate.resolve(1, value: Self.snapshot())
        await old.value
        XCTAssertEqual(model.shown?.providers.first?.enabled, false)
        XCTAssertEqual(model.shown?.providers.first?.logins, [])
        let actual5 = await gate.flags(2)
        XCTAssertEqual(actual5, [false, false])
    }

    @MainActor
    func testEnablingUsesExplicitInteractiveReadAfterSettingIsSaved() async {
        let log = UsageRequestLog()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider in
            await log.append("read:\(refresh):\(interactive):\(provider ?? "all")")
            return Self.snapshot()
        }, writer: { provider, on in await log.append("set:\(provider):\(on)") })
        await model.setEnabled("claude", on: true)
        let actual6 = await log.lines()
        XCTAssertEqual(actual6, ["set:claude:true", "read:true:true:claude"])
    }

    @MainActor
    func testEnablingCodexScopesAnInteractiveReadToCodex() async {
        let log = UsageRequestLog()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider in
            await log.append("read:\(refresh):\(interactive):\(provider ?? "all")")
            return Self.snapshot()
        }, writer: { provider, on in await log.append("set:\(provider):\(on)") })
        await model.setEnabled("codex", on: true)
        let lines = await log.lines()
        XCTAssertEqual(lines, ["set:codex:true", "read:true:true:codex"])
    }

    @MainActor
    func testLeavingOverviewDiscardsAReadThatIgnoresCancellation() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        })
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        observing.cancel()
        await gate.resolve(0, value: Self.snapshot())
        await observing.value
        XCTAssertNil(model.shown)
        XCTAssertFalse(model.isRefreshing)
    }

    @MainActor
    func testLeavingOverviewDropsQueuedInteractiveRefresh() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        })
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        await model.load(refresh: true, interactive: true)
        observing.cancel()
        await gate.resolve(0, value: Self.snapshot())
        await observing.value
        for _ in 0..<1_000 { await Task.yield() }
        let count = await gate.count()
        // Resolve any unexpected extra read before asserting, so a regression
        // does not leave a suspended checked continuation behind.
        for index in 1..<max(1, count) { await gate.resolve(index, value: Self.snapshot()) }
        XCTAssertEqual(count, 1)
        XCTAssertNil(model.shown)
    }

    @MainActor
    func testEnableFinishingAfterLeavingOverviewDoesNotStartInteractiveRead() async {
        let reader = UsageReaderGate()
        let writer = UsageWriterGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _ in
            try await reader.read(refresh: refresh, interactive: interactive)
        }, writer: { _, _ in await writer.write() })
        let observing = Task { await model.observe() }
        await waitForCalls(reader, 1)
        let enabling = Task { await model.setEnabled("claude", on: true) }
        let writerStarted = await writer.waitForCall()
        XCTAssertTrue(writerStarted)
        observing.cancel()
        for _ in 0..<1_000 { await Task.yield() }
        await writer.resolve()
        for _ in 0..<1_000 { await Task.yield() }
        let count = await reader.count()
        for index in 1..<max(1, count) { await reader.resolve(index, value: Self.snapshot()) }
        await enabling.value
        await reader.resolve(0, value: Self.snapshot())
        await observing.value
        XCTAssertEqual(count, 1)
        XCTAssertTrue(model.changingProviders.isEmpty)
        XCTAssertNil(model.shown)
    }

    @MainActor
    func testFailedReadDropsUnmarkedEarlierMeasurements() async {
        let values = UsageFailureReader(value: Self.snapshot())
        let model = SubscriptionUsageModel(reader: { _, _, _ in try await values.read() })
        await model.load()
        XCTAssertNotNil(model.shown)
        await model.load()
        XCTAssertNil(model.shown)
        XCTAssertEqual(model.failureKey, "analytics_subscriptions_unavailable")
    }

    func testWordsPreserveUnknownWindowBankedZeroAndScopedFable() throws {
        let copy = try uiCopy()
        let login = DaemonData.SubscriptionUsageLogin(id: "claude-default", label: "Claude", defaultLogin: true,
            state: "observed", updatedAt: Self.now, windows: [
                .init(lane: "session", usedPermille: nil, remainingText: "100%"),
                .init(lane: "weekly", usedPermille: 400, remainingText: "60%", resetsAt: Self.now.addingTimeInterval(60)),
                .init(lane: "model", label: "Fable", usedPermille: 800, remainingText: "20%")], bankedResets: 0)
        let words = InsightsSubscriptionWords.login(login, provider: "claude", now: Self.now, copy: copy)
        XCTAssertEqual(words.windows.map(\.label), ["Session", "Weekly", "Fable"])
        XCTAssertEqual(words.windows.map(\.remaining), ["—", "60% remaining", "20% remaining"])
        XCTAssertEqual(words.windows.map(\.remainingPermille), [nil, 600, 200])
        XCTAssertEqual(words.banked, "0 available")
        XCTAssertNil(words.windows.first?.reset)
        XCTAssertNotNil(words.windows[1].reset)
    }

    func testPendingKeychainReadExplainsTheWaitWithoutRequestingAnotherPrompt() throws {
        let login = DaemonData.SubscriptionUsageLogin(id: "claude-default", label: "Claude", defaultLogin: true,
            state: "unknown", reason: "keychain_read_pending")
        let words = InsightsSubscriptionWords.login(login, provider: "claude", now: Self.now, copy: try uiCopy())
        XCTAssertEqual(words.state, "Waiting for Keychain access.")
        XCTAssertFalse(words.state?.contains("Refresh") ?? true)
        XCTAssertEqual(words.banked, "—")
    }

    func testAnExpiredWindowBecomesUnknownWithoutAFullAllowanceOrNewReset() throws {
        let login = DaemonData.SubscriptionUsageLogin(id: "login", label: "Saved subscription", state: "observed",
            stale: true, windows: [.init(lane: "weekly", usedPermille: 900, remainingText: "10%", resetsAt: Self.now)])
        let words = InsightsSubscriptionWords.login(login, provider: "codex", now: Self.now, copy: try uiCopy())
        XCTAssertEqual(words.windows.first?.remaining, "—")
        XCTAssertNil(words.windows.first?.reset)
        XCTAssertNil(words.windows.first?.remainingPermille)
        XCTAssertEqual(words.banked, "—")
        XCTAssertNotNil(words.state)
    }

    @MainActor
    func testSubscriptionCardRendersAtNarrowOverviewWidth() async throws {
        guard let target = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] else {
            throw XCTSkip("Opt-in native render")
        }
        let snapshot = DaemonData.SubscriptionUsage(providers: [
            .init(provider: "codex", enabled: true, logins: [.init(id: "codex-default", label: "Codex", defaultLogin: true,
                state: "observed", updatedAt: Date(), windows: [.init(lane: "weekly", usedPermille: 300, remainingText: "70%",
                    resetsAt: Date().addingTimeInterval(86400))], bankedResets: 2)]),
            .init(provider: "claude", enabled: true, logins: [.init(id: "claude-default", label: "Claude", defaultLogin: true,
                state: "observed", updatedAt: Date(), windows: [
                    .init(lane: "session", usedPermille: 200, remainingText: "80%", resetsAt: Date().addingTimeInterval(3600)),
                    .init(lane: "weekly", usedPermille: 450, remainingText: "55%", resetsAt: Date().addingTimeInterval(86400)),
                    .init(lane: "model", label: "Fable", usedPermille: 710, remainingText: "29%", resetsAt: Date().addingTimeInterval(86400))])])])
        let model = SubscriptionUsageModel(reader: { _, _, _ in snapshot }, writer: { _, _ in })
        await model.load()
        let sharedCopy = try uiCopy()
        XCTAssertFalse(sharedCopy["analytics_subscriptions_title"]?.isEmpty ?? true)
        _ = NSApplication.shared
        let view = InsightsSubscriptionsCard(model: model, copy: sharedCopy)
            .padding(16).frame(width: 408).background(Color(nsColor: .windowBackgroundColor))
        let hosting = NSHostingView(rootView: view)
        let size = hosting.fittingSize
        let bounds = NSRect(x: 0, y: 0, width: 408, height: max(600, size.height))
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let representation = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: representation)
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        let directory = URL(fileURLWithPath: target)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try png.write(to: directory.appendingPathComponent("insights-subscription-limits.png"))
        XCTAssertGreaterThan(png.count, 1000)
    }
}

private actor UsageReaderGate {
    private var requests: [(Bool, Bool)] = []
    private var continuations: [Int: CheckedContinuation<DaemonData.SubscriptionUsage, Error>] = [:]
    private var initial: DaemonData.SubscriptionUsage?
    init(initial: DaemonData.SubscriptionUsage? = nil) { self.initial = initial }
    func read(refresh: Bool, interactive: Bool) async throws -> DaemonData.SubscriptionUsage {
        let index = requests.count
        requests.append((refresh, interactive))
        if let initial { self.initial = nil; return initial }
        return try await withCheckedThrowingContinuation { continuations[index] = $0 }
    }
    func count() -> Int { requests.count }
    func flags(_ index: Int) -> [Bool] { [requests[index].0, requests[index].1] }
    func waitForCalls(_ count: Int) async -> Bool {
        for _ in 0..<10_000 {
            if requests.count >= count { return true }
            await Task.yield()
        }
        return false
    }
    func resolve(_ index: Int, value: DaemonData.SubscriptionUsage) { continuations.removeValue(forKey: index)?.resume(returning: value) }
}

private actor UsageRequestLog {
    private var values: [String] = []
    func append(_ value: String) { values.append(value) }
    func lines() -> [String] { values }
}

private actor UsageFailureReader {
    private var value: DaemonData.SubscriptionUsage?
    init(value: DaemonData.SubscriptionUsage) { self.value = value }
    func read() throws -> DaemonData.SubscriptionUsage {
        if let value { self.value = nil; return value }
        throw DaemonDataError.unreachable
    }
}

private actor UsageWriterGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private var started = false
    func write() async {
        started = true
        await withCheckedContinuation { continuation = $0 }
    }
    func waitForCall() async -> Bool {
        for _ in 0..<10_000 {
            if started { return true }
            await Task.yield()
        }
        return false
    }
    func resolve() { continuation?.resume(); continuation = nil }
}
