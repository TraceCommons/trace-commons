import AppKit
import TCBridge
import SwiftUI
import TCShellCore
import XCTest
@testable import TraceCommonsApp

final class ManagedSessionsRenderTests: XCTestCase {
    @MainActor
    func testSavedAccountsAndCliSessionRenderWithoutStartingANativeTool() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json")))
        XCTAssertEqual(model.managedSnapshot?.sessions.first?.accountLabel, "Personal")
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 480), to: directory.appendingPathComponent("managed-sessions.png"))
        // The compact left pane (360pt with its 12pt padding), where the
        // header buttons once broke per syllable (V1).
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 360, height: 480), to: directory.appendingPathComponent("managed-sessions-compact.png"))
        try render(ManagedAccountSheet(onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 360), to: directory.appendingPathComponent("managed-add-account.png"))
        try render(ManagedLaunchSheet(onClose: {}).environmentObject(model), size: CGSize(width: 580, height: 300), to: directory.appendingPathComponent("managed-launch.png"))
        let account = try XCTUnwrap(model.managedSnapshot?.accounts.first)
        try render(ManagedRenameSheet(account: account, onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-rename.png"))
        try render(ManagedRemoveSheet(account: account, onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-remove.png"))
    }

    @MainActor
    func testOpenCodeLaunchAndCollectionControlsRender() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = try Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: fixture) as? [String: Any])
        value["accounts"] = [["id": "opencode-test", "tool": "opencode", "connection": "api_key", "label": "Work", "auth_state": "ready"]]
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(ManagedLaunchSheet(onClose: {}, accountID: "opencode-test").environmentObject(model), size: CGSize(width: 580, height: 580), to: directory.appendingPathComponent("managed-opencode-launch.png"))
    }

    @MainActor
    func testInstallationAndOpenCodeOnlySetupRender() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = try Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: fixture) as? [String: Any])
        let copyJSON = try XCTUnwrap(TCDiscovery.managedCopyJSON())
        value["copy"] = try JSONSerialization.jsonObject(with: Data(copyJSON.utf8))
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        for installed in [false, true] {
            value["tools"] = ["claude", "codex", "opencode"].map { tool in
                ["tool": tool, "installed": installed && tool == "opencode", "install_url": "https://opencode.ai/docs/"] as [String: Any]
            }
            let model = AppModel()
            model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
            if installed {
                try render(ManagedAccountSheet(onClose: {}).environmentObject(model), size: CGSize(width: 580, height: 480), to: directory.appendingPathComponent("managed-opencode-only-setup.png"))
            } else {
                try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 780), to: directory.appendingPathComponent("managed-install-tools.png"))
            }
        }
    }

    @MainActor
    func testManagedOpenCodeMapRender() throws {
        let copy = try XCTUnwrap(TCDiscovery.managedCopyJSON())
        let wire = "{\"tools\":[],\"managed_copy\":\(copy),\"managed_sessions\":[{\"id\":\"map-session\",\"tool\":\"opencode\",\"connection\":\"api_key\",\"provider\":\"openai\",\"model\":\"gpt-4.1\",\"state\":\"running\",\"collection_state\":\"pending\"}]}"
        let destinations = try JSONDecoder().decode(DaemonData.ToolDestinations.self, from: Data(wire.utf8))
        let scene = FlowMapScene.privateAI(.none, destinationLabel: "Private AI", privateAI: "off", sentence: { _ in nil }, state: { _ in .unknown }).addingManagedRoutes(destinations)
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(FlowMapView(scene: scene, legend: [], zoomable: false, accessibilityName: "Managed sessions"), size: CGSize(width: 580, height: 760), to: directory.appendingPathComponent("managed-opencode-map.png"))
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        _ = NSApplication.shared
        let hosting = NSHostingView(rootView: view.padding(20).frame(width: size.width, height: size.height).background(Color(nsColor: .windowBackgroundColor)))
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
