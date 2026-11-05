import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Opt-in render QA using the current daemon's own copy and a safe dummy
/// preset. No product sentences or real credentials are embedded here.
final class ClaudeDesktopRenderTests: XCTestCase {
    @MainActor
    func test_toolsRowAndSetupSheetAtRegularAndNarrowWidths() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard let output = environment["TRACE_COMMONS_SCREENSHOT_DIR"],
              let payload = environment["TRACE_COMMONS_DESKTOP_RENDER_PAYLOAD"] else {
            throw XCTSkip("desktop-render-payload-not-supplied")
        }
        let directory = URL(fileURLWithPath: output, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: payload))) as? [String: Any])
        let result = try XCTUnwrap(frame["result"] as? [String: Any])
        let main = try XCTUnwrap(result["model"] as? String)
        let background = try XCTUnwrap(result["background_model"] as? String)
        let preset = try XCTUnwrap(result["environment"] as? String)
        XCTAssertEqual(preset.components(separatedBy: "\n").count, 16, "render fixture must exercise the full preset height")
        let catalogue: [String: Any] = [
            "models": [main, background].map { id in
                ["id": id, "name": id.components(separatedBy: "/").last ?? id, "tier": "near_ai"]
            },
            "default": main, "tier_order": ["near_ai", "external", "incognito"],
            "background": [main: ["default": background, "set": 0]],
            "background_sets": [[main, background]],
        ]

        for state in ["disconnected", "ready"] {
            var snapshot = result
            snapshot["state"] = state
            snapshot["observed"] = false
            if state == "disconnected" {
                snapshot.removeValue(forKey: "model")
                snapshot.removeValue(forKey: "background_model")
                snapshot.removeValue(forKey: "environment")
            }
            let daemon = DesktopRenderDaemon(result: snapshot, catalogue: catalogue)
            let client = DaemonClient(daemon: daemon)
            let app = AppModel()
            app.setStartupForTesting(.running)
            app.setClientForTesting(client)
            let setup = ClaudeDesktopSetupModel(client: client)
            await setup.refresh()
            let decodedCatalogue = try JSONDecoder().decode(ManagedModelCatalogue.self,
                from: JSONSerialization.data(withJSONObject: catalogue))
            setup.readCachedCatalogue(decodedCatalogue)
            setup.open()
            setup.loadCatalogue { $0(decodedCatalogue) }
            if state == "ready" {
                await setup.save()
                XCTAssertEqual(setup.environment, preset)
            }
            let status = try XCTUnwrap(setup.status)
            XCTAssertFalse(MonitorWords.close.isEmpty)
            for (widthName, width) in [("regular", CGFloat(804)), ("narrow", CGFloat(474))] {
                for (themeName, theme) in [("light", ColorScheme.light), ("dark", ColorScheme.dark)] {
                    let stem = "claude-desktop-\(state)-\(widthName)-\(themeName)"
                    try await render(
                        GlassCard { ClaudeDesktopToolRow(setup: setup, status: status) },
                        size: CGSize(width: width, height: 260), theme: theme,
                        to: directory.appendingPathComponent(stem + "-row.png"))
                    try await render(
                        ClaudeDesktopSetupSheet(setup: setup, closeLabel: MonitorWords.close)
                            .environmentObject(app),
                        size: CGSize(width: width, height: 1080), theme: theme,
                        to: directory.appendingPathComponent(stem + "-sheet.png"))
                }
            }
            setup.close()
        }
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, theme: ColorScheme, to url: URL) async throws {
        _ = NSApplication.shared
        let root = view.padding(12).frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor))
            .environment(\.colorScheme, theme)
        let hosting = NSHostingView(rootView: root)
        let bounds = NSRect(origin: .zero, size: size)
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: theme == .dark ? .darkAqua : .aqua)
        hosting.frame = bounds
        window.contentView = hosting
        defer { window.close() }
        // The sheet's real onAppear catalogue request uses AppModel's
        // detached IPC path; let its mock response land before capturing.
        try await Task.sleep(for: .milliseconds(60))
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let rep = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: rep)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: url)
    }
}

private final class DesktopRenderDaemon: DaemonCalling {
    private let response: String
    private let catalogue: String

    init(result: [String: Any], catalogue: [String: Any]) {
        self.response = String(decoding: try! JSONSerialization.data(withJSONObject: ["id": 1, "result": result]), as: UTF8.self)
        self.catalogue = String(decoding: try! JSONSerialization.data(withJSONObject: ["id": 1, "result": catalogue]), as: UTF8.self)
    }

    func call(_ method: String, params: String) -> String {
        method == "managed_model_catalogue" ? catalogue : response
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
