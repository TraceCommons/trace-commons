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

    func testTheSettingsRowReadsTheStatusBeforeItHasAnythingToDraw() async throws {
        let methods = try await statusReads(hosting: ClaudeDesktopSettingsRow(closeLabel: "Close"))
        XCTAssertTrue(methods.contains("claude_desktop_status"), "\(methods)")
    }

    func testTheTabCardReadsTheStatusBeforeItHasAnythingToDraw() async throws {
        let methods = try await statusReads(hosting: ClaudeDesktopSetupPrompt(onOpenSettings: {}))
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
