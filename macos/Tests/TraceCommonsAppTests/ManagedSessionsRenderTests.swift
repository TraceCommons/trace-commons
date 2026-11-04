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
        let resumeCapable = model.managedSnapshot?.capabilities.resume == true
        XCTAssertEqual(rows.map { ManagedSurface.rowButtons($0, terminalLaunch: true, resumeCapable: resumeCapable) }, [[], [], [.resume]])
        XCTAssertEqual(rows.map { ManagedSurface.rowMenuItems($0, terminalLaunch: true, resumeCapable: resumeCapable) }, [[.show, .stop], [], [.startAgain, .dismiss]])
        XCTAssertEqual(ManagedSurface.rowButtons(rows[2], terminalLaunch: true, resumeCapable: true), [.resume])
        XCTAssertEqual(ManagedSurface.rowMenuItems(rows[2], terminalLaunch: true, resumeCapable: true), [.startAgain, .dismiss])
        XCTAssertNotNil(rows[2].resumedFrom)
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 640), to: directory.appendingPathComponent("managed-session-actions.png"))
        try render(ManagedStopSheet(session: rows[0], onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-stop.png"))
    }

    /// Every row state: what each row draws (one visible action at most,
    /// the rest in the menu), then a render at a wide and a narrow width to
    /// PNG for review (TRACE_COMMONS_SCREENSHOT_DIR), off screen.
    @MainActor
    func testSessionRowStatesDrawOneActionAndRenderWideAndNarrow() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let fixture = try Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: fixture) as? [String: Any])
        let base = try XCTUnwrap((value["sessions"] as? [[String: Any]])?.first)
        func row(_ id: Int, _ state: String, _ edit: (inout [String: Any]) -> Void = { _ in }) -> [String: Any] {
            var r = base
            r["id"] = "00000000-0000-0000-0000-00000000010\(id)"
            r["state"] = state
            r["model"] = "z-ai/glm-5.3-flash-with-a-long-id"
            r["project_label"] = "VolumeFi Invoice data"
            r["can_focus"] = false; r["can_stop"] = false; r["can_start_again"] = false; r["can_resume"] = false
            edit(&r)
            return r
        }
        value["capabilities"] = (value["capabilities"] as? [String: Any] ?? [:]).merging(["resume": true, "terminal_launch": true]) { $1 }
        value["sessions"] = [
            row(1, "running") { $0["can_focus"] = true; $0["can_stop"] = true },
            row(2, "starting"),
            row(3, "exited") { $0["can_resume"] = true; $0["can_start_again"] = true; $0["exit_code"] = 0 },
            row(4, "failed") { $0["can_resume"] = true; $0["exit_code"] = 1 },
            row(5, "exited") { $0["can_start_again"] = true },
            row(6, "failed"),
            row(7, "something-new"),
        ]
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
        let rows = try XCTUnwrap(model.managedSnapshot?.sessions)
        XCTAssertEqual(rows.count, 7)
        // What the row view draws for each state, from the same inputs it
        // passes (`ManagedSessionActions.drawn`): one visible action at
        // most, the rest in the menu. With the config directory unknown the
        // menu has no Copy resume command; known, it has one exactly where a
        // resumable ended row is.
        let expected: [(buttons: [ManagedSessionAction], menu: [ManagedSessionAction], menuKnown: [ManagedSessionAction])] = [
            ([], [.show, .stop], [.show, .stop]),
            ([], [], []),
            ([.resume], [.startAgain, .dismiss], [.startAgain, .copyResumeCommand, .dismiss]),
            ([.resume], [.dismiss], [.copyResumeCommand, .dismiss]),
            ([.startAgain], [.dismiss], [.dismiss]),
            ([.dismiss], [], []),
            ([], [], []),
        ]
        for (index, session) in rows.enumerated() {
            let unknown = ManagedSessionActions.drawn(session, terminalLaunch: true, resumeCapable: true, configDirectoryKnown: false)
            let known = ManagedSessionActions.drawn(session, terminalLaunch: true, resumeCapable: true, configDirectoryKnown: true)
            XCTAssertLessThanOrEqual(known.0.count, 1, "row \(index + 1) draws more than one visible action")
            XCTAssertEqual(unknown.0, expected[index].buttons, "row \(index + 1) buttons")
            XCTAssertEqual(unknown.1, expected[index].menu, "row \(index + 1) menu")
            XCTAssertEqual(known.1, expected[index].menuKnown, "row \(index + 1) menu with the config directory known")
        }
        // A resumable row without a terminal keeps Dismiss as its link and
        // Copy resume command in the menu.
        let noTerminal = ManagedSessionActions.drawn(rows[2], terminalLaunch: false, resumeCapable: true, configDirectoryKnown: true)
        XCTAssertEqual(noTerminal.0, [.dismiss])
        XCTAssertEqual(noTerminal.1, [.copyResumeCommand])
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 900, height: 900), to: directory.appendingPathComponent("rows-wide.png"))
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 420, height: 1200), to: directory.appendingPathComponent("rows-narrow.png"))
    }

    /// The row layout itself: one line when everything fits (900 pt), two
    /// when it does not (420 pt) with the text wrapped, never cut, and the
    /// trailing views no wider than the row.
    @MainActor
    func testSessionRowLayoutWrapsToTwoLinesInsteadOfCutting() throws {
        func height(_ width: CGFloat, trailing: CGFloat = 200) -> CGFloat {
            let view = SessionRowLayout(spacing: 8, lineSpacing: 6) {
                Color.clear.frame(width: 40, height: 40)
                VStack(alignment: .leading, spacing: 1) {
                    Text("VolumeFi Invoice data").font(.system(size: 13, weight: .semibold))
                    Text("Claude Code \u{00B7} Personal \u{00B7} Subscription \u{00B7} z-ai/glm-5.3-flash-with-a-long-id \u{00B7} Exit code 0 \u{00B7} Resumed \u{00B7} Continues 2:14 PM \u{00B7} 3 min ago")
                        .font(.system(size: 11))
                }
                .fixedSize(horizontal: false, vertical: true)
                Color.clear.frame(width: trailing, height: 28)
            }
            let controller = NSHostingController(rootView: view)
            return controller.sizeThatFits(in: CGSize(width: width, height: 10_000)).height
        }
        let wide = height(1_000)
        let narrow = height(420)
        XCTAssertEqual(wide, 40, accuracy: 1, "one line: the tile's height")
        // Two lines: the wrapped text beside the tile, then the trailing row.
        XCTAssertGreaterThan(narrow, wide + 28, "the trailing views sit on a line of their own")
        // The caption wraps in full: a narrower row is taller, never shorter.
        XCTAssertGreaterThanOrEqual(height(300), narrow)
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
