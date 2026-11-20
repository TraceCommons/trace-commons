import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Settings row and the Private AI tab card read Claude Desktop's
/// status from their own polling task. That task must run while nothing is
/// drawn yet: a `.task` hung on a `Group` whose only content waits on the
/// task's own result has no view to attach to, so it never starts and the
/// row and card never appear (staging 0.24.64, 2026-11-19).
@MainActor
final class ClaudeDesktopPollingTests: XCTestCase {
    /// As the Private AI tab and the Settings tools card place it: one
    /// entry in a spaced VStack between siblings. Hosted alone, an empty
    /// Group still runs its .task; in a list it contributes no view and
    /// its .task never runs (staging 0.24.65).
    static func inAList(_ view: some View) -> some View {
        VStack(spacing: 12) {
            Text("before")
            view
            Text("after")
        }
    }

    /// The Settings tools card: the row in a stack that polls.
    struct SettingsCard: View {
        @State private var setup: ClaudeDesktopSetupModel?
        var body: some View {
            ClaudeDesktopPollingTests.inAList(ClaudeDesktopSettingsRow(setup: setup, closeLabel: "Close"))
                .claudeDesktopPolling($setup, every: .seconds(5), readsModelNames: true)
        }
    }

    /// The Private AI tab: the card in a stack that polls.
    struct Tab: View {
        @State private var setup: ClaudeDesktopSetupModel?
        var body: some View {
            ClaudeDesktopPollingTests.inAList(ClaudeDesktopSetupPrompt(setup: setup, onOpenSettings: {}))
                .claudeDesktopPolling($setup, every: .seconds(15))
        }
    }

    private func statusReads(hosting view: some View) async throws -> [String] {
        let daemon = PollingDaemon()
        let app = try DesktopFeatureFixture.app(client: DaemonClient(daemon: daemon), enabled: true)
        app.setStartupForTesting(.running)
        XCTAssertTrue(app.featureIsEnabled(ClaudeDesktopSetupModel.featureID))
        _ = NSApplication.shared
        let bounds = NSRect(x: 0, y: 0, width: 600, height: 400)
        let hosting = NSHostingView(rootView: view.environmentObject(app).frame(width: 600, height: 400))
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        hosting.frame = bounds
        window.contentView = hosting
        defer { window.close() }
        for _ in 0..<100 where !daemon.methods.contains("claude_desktop_status") {
            hosting.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        return daemon.methods
    }

    /// The app draws Settings and the Private AI tab before its first flag
    /// read lands, so the flag turns on after the row and card are on
    /// screen. Their polling must start then too (staging 0.24.65: the
    /// log shows the flag read as on, and no status read ever).
    private func statusReadsAfterTheFlagTurnsOn(hosting view: some View) async throws -> [String] {
        let daemon = PollingDaemon()
        let app = try DesktopFeatureFixture.app(client: DaemonClient(daemon: daemon), enabled: nil)
        app.setStartupForTesting(.running)
        XCTAssertFalse(app.featureIsEnabled(ClaudeDesktopSetupModel.featureID))
        _ = NSApplication.shared
        let bounds = NSRect(x: 0, y: 0, width: 600, height: 400)
        let hosting = NSHostingView(rootView: view.environmentObject(app).frame(width: 600, height: 400))
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        hosting.frame = bounds
        window.contentView = hosting
        defer { window.close() }
        for _ in 0..<10 {
            hosting.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTAssertFalse(daemon.methods.contains("claude_desktop_status"))
        app.featureFlags = try DesktopFeatureFixture.snapshot(enabled: true, generation: 2)
        for _ in 0..<100 where !daemon.methods.contains("claude_desktop_status") {
            hosting.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        return daemon.methods
    }

    func testTheSettingsRowStartsPollingWhenTheFlagTurnsOnLater() async throws {
        let methods = try await statusReadsAfterTheFlagTurnsOn(hosting: SettingsCard())
        XCTAssertTrue(methods.contains("claude_desktop_status"), "\(methods)")
    }

    func testTheTabCardStartsPollingWhenTheFlagTurnsOnLater() async throws {
        let methods = try await statusReadsAfterTheFlagTurnsOn(hosting: Tab())
        XCTAssertTrue(methods.contains("claude_desktop_status"), "\(methods)")
    }

    func testTheSettingsRowReadsTheStatusBeforeItHasAnythingToDraw() async throws {
        let methods = try await statusReads(hosting: SettingsCard())
        XCTAssertTrue(methods.contains("claude_desktop_status"), "\(methods)")
    }

    func testTheTabCardReadsTheStatusBeforeItHasAnythingToDraw() async throws {
        let methods = try await statusReads(hosting: Tab())
        XCTAssertTrue(methods.contains("claude_desktop_status"), "\(methods)")
    }
}

private final class PollingDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var calls: [String] = []
    var methods: [String] { lock.withLock { calls } }

    func call(_ method: String, params: String) -> String {
        lock.withLock { calls.append(method) }
        guard method == "claude_desktop_status" else { return #"{"id":1,"result":{}}"# }
        let copy = Dictionary(uniqueKeysWithValues: (ClaudeDesktopStatus.copyKeys + ClaudeDesktopStatus.promptCopyKeys).map { ($0, $0) })
        let result: [String: Any] = ["state": "disconnected", "observed": false, "copy": copy]
        let frame = try! JSONSerialization.data(withJSONObject: ["id": 1, "result": result])
        return String(decoding: frame, as: UTF8.self)
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
