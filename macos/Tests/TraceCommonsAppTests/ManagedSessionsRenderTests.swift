import AppKit
import TCBridge
import SwiftUI
import TCBridge
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
        let snapshot = try XCTUnwrap(model.managedSnapshot)
        XCTAssertEqual(snapshot.terminalChoices.count, 2)
        try render(ManagedTerminalPicker(snapshot: snapshot).environmentObject(model), size: CGSize(width: 520, height: 80), to: directory.appendingPathComponent("managed-terminal-choice.png"))
        let account = try XCTUnwrap(model.managedSnapshot?.accounts.first)
        try render(ManagedRenameSheet(account: account, onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-rename.png"))
        try render(ManagedRemoveSheet(account: account, onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-remove.png"))
    }

    @MainActor
    func testMissingAndExportOnlyToolsKeepRecoveryVisible() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        var fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))) as? [String: Any])
        let copy = try XCTUnwrap(TCDiscovery.managedCopyJSON())
        fixture["copy"] = try JSONSerialization.jsonObject(with: Data(copy.utf8))
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        for installed in [[], ["opencode"], ["codex"]] {
            fixture["tools"] = ManagedSurface.supportedTools.map { ["tool": $0, "installed": installed.contains($0), "install_url": "https://example.com/" + $0] as [String: Any] }
            let model = AppModel()
            let snapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: fixture))
            model.managedSnapshot = snapshot
            XCTAssertFalse(snapshot.canLaunch(tool: "claude"))
            XCTAssertEqual(ManagedSessionsSection.offersLaunch(snapshot), installed.contains("codex"))
            XCTAssertEqual(snapshot.needsToolInstallation, installed.isEmpty)
            try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 900), to: directory.appendingPathComponent("availability-\(installed.first ?? "none").png"))
        }
    }

    /// The NEAR AI sign-in card both ways: Start buttons with a sign-in,
    /// the sign-in line and button without one.
    @MainActor
    func testTheSignInCardRendersSignedInAndSignedOut() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = try String(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"), encoding: .utf8)
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(fixture.utf8))
        XCTAssertEqual(model.managedSnapshot?.nearAiSignedIn, true)
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 560), to: directory.appendingPathComponent("managed-sign-in.png"))
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 360, height: 640), to: directory.appendingPathComponent("managed-sign-in-compact.png"))
        let signedOut = fixture.replacingOccurrences(of: #""near_ai_signin": true"#, with: #""near_ai_signin": false"#)
        XCTAssertNotEqual(signedOut, fixture)
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(signedOut.utf8))
        XCTAssertEqual(model.managedSnapshot?.nearAiSignedIn, false)
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 560), to: directory.appendingPathComponent("managed-sign-in-needed.png"))
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

    /// #1408, as the launch dialog (owner, 2026-10-19): the explanation,
    /// the folder row and Open {tool}, for each tool, and its artwork at
    /// rest, as Reduce Motion shows it.
    @MainActor
    func testTheFolderExplainerRendersForEachStartButton() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json")))
        XCTAssertEqual(model.managedSnapshot?.explainsFolderChoice, true)
        XCTAssertEqual(model.managedText("folder_choice"), "")
        XCTAssertEqual(model.managedText("home_folder"), "Home folder")
        XCTAssertEqual(model.managedText("clear_folder"), "Use home")
        XCTAssertEqual(model.managedText("pick_folder"), "Pick a folder as context")
        XCTAssertEqual(ManagedFolderExplainerSheet.openLabel(tool: "claude", model: model), "Open Claude Code")
        XCTAssertEqual(ManagedFolderExplainerSheet.openLabel(tool: "codex", model: model), "Open Codex")
        XCTAssertTrue(ManagedFolderHandoff.startsAtRest(reduceMotion: true))
        XCTAssertFalse(ManagedFolderHandoff.startsAtRest(reduceMotion: false))
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        for tool in ManagedSurface.signInTools {
            try render(ManagedFolderExplainerSheet(tool: tool, onCancel: {}, onOpen: { _ in }).environmentObject(model), size: CGSize(width: 520, height: 400), to: directory.appendingPathComponent("managed-folder-explainer-\(tool).png"))
        }
        try render(ManagedFolderHandoff(tool: .claudeCode, atRest: true), size: CGSize(width: 320, height: 160), to: directory.appendingPathComponent("managed-folder-handoff-rest.png"))
    }

    /// #1407: a running row the daemon can show and stop, a running row it
    /// cannot, an ended row that can resume and start again, and the stop
    /// confirmation, rendered off screen.
    @MainActor
    func testSessionRowActionsAndStopConfirmationRender() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = try Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: fixture) as? [String: Any])
        var sessions = try XCTUnwrap(value["sessions"] as? [[String: Any]])
        var owned = sessions[0]
        owned["can_focus"] = true
        owned["can_stop"] = true
        owned["started_at"] = "2026-10-14T09:30:00.123456789Z"
        var cli = sessions[0]
        cli["id"] = "00000000-0000-0000-0000-000000000004"
        var ended = sessions[0]
        ended["id"] = "00000000-0000-0000-0000-000000000005"
        ended["state"] = "exited"
        ended["exit_code"] = 0
        ended["can_start_again"] = true
        ended["can_resume"] = true
        ended["resumed_from"] = "2026-10-14T09:30:00.123456789Z"
        var capabilities = try XCTUnwrap(value["capabilities"] as? [String: Any])
        capabilities["resume"] = true
        value["capabilities"] = capabilities
        sessions = [owned, cli, ended]
        value["sessions"] = sessions
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
        let rows = try XCTUnwrap(model.managedSnapshot?.sessions)
        XCTAssertEqual(rows.map { ManagedSurface.sessionActions($0, terminalLaunch: true, resumeCapable: model.managedSnapshot?.capabilities.resume == true) }, [[.show, .stop], [], [.resume, .startAgain, .dismiss]])
        XCTAssertEqual(ManagedSurface.rowButtons(rows[2], terminalLaunch: true, resumeCapable: true), [.resume, .dismiss])
        XCTAssertEqual(ManagedSurface.rowMenuItems(rows[2], terminalLaunch: true, resumeCapable: true), [.startAgain])
        XCTAssertNotNil(rows[2].resumedFrom)
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 640), to: directory.appendingPathComponent("managed-session-actions.png"))
        try render(ManagedStopSheet(session: rows[0], onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-stop.png"))
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
    func testUnreadHarnessListWithoutManagedRoutesDrawsNoMap() throws {
        let scene = FlowMapScene.privateAI(.none, destinationLabel: "Private AI", privateAI: "off", sentence: { _ in nil }, state: { _ in .unknown }).addingManagedRoutes(nil)
        XCTAssertFalse(scene.drawsMap)
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
