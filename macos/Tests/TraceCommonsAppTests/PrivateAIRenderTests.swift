import AppKit
import SwiftUI
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Inference tab's Private AI page and its inspector, rendered at the
/// compact left pane's and the inspector's widths (P24, P25) from the sample
/// daemon. Set `TRACE_COMMONS_SCREENSHOT_DIR` to keep the images.
final class PrivateAIRenderTests: XCTestCase {
    @MainActor
    func test_thePageAndTheInspectorRenderAtTheirPaneWidths() async throws {
        let model = AppModel()
        model.setStartupForTesting(.running)
        // The three persistent default connections belong to Settings.
        let rows = ["claude", "codex", "opencode"].map { id in
            ["id": id, "name": id == "claude" ? "Claude Code" : id == "codex" ? "Codex" : "OpenCode",
             "installed": true, "connected": false, "connect_command": "", "state": "not_connected",
             "can_connect": true, "can_disconnect": false] as [String: Any]
        }
        let wire: [String: Any] = [
            "harnesses": rows, "catalog_present": false, "destination_port": 8471,
            "destination_credentialed": true,
            "activity": ["readable": false, "window_hours": 24, "families": []],
            "spend": ["known": false]
        ]
        let data = try JSONSerialization.data(withJSONObject: wire)
        let harnesses = HarnessSurface.list(fromJSON: String(decoding: data, as: UTF8.self))
        XCTAssertEqual(harnesses.harnesses.map(\.id), ["claude", "codex", "opencode"])
        model.setHarnessesForTesting(harnesses)
        XCTAssertNotNil(model.privateInferenceCopy, "the page draws nothing without the core's words")
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:))
            ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer {
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil {
                try? FileManager.default.removeItem(at: directory)
            }
        }
        try render(
            ScrollView { InferenceAccountSection(store: store) }.environmentObject(model),
            size: CGSize(width: 360, height: 1400), to: directory.appendingPathComponent("private-ai-page.png"))
        try render(
            PrivateAIInspectorView(store: store)
                .environmentObject(model),
            size: CGSize(width: 300, height: 640), to: directory.appendingPathComponent("private-ai-inspector.png"))
        try render(
            ScrollView { PrivateAISettingsPanels(store: store) }.environmentObject(model),
            size: CGSize(width: 520, height: 1400), to: directory.appendingPathComponent("private-ai-settings.png"))
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        _ = NSApplication.shared
        let hosting = NSHostingView(rootView: view.padding(12).frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor)))
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let rep = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: rep)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: url)
    }
}
