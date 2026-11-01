import AppKit
import SwiftUI
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Private AI tab's managed states that lived in the saved-accounts
/// card and must outlive it (#1490 review), drawn only while they apply and
/// inside the cards already there: a failed launch, stop or dismiss with
/// its update links, where it was asked; no terminal and no supported tool,
/// in place of the NEAR AI card's tool buttons; and the list not read yet,
/// with Retry. Each is pinned by what decides it and the words drawn for it.
@MainActor
final class ManagedStatusTests: XCTestCase {
    private static var repository: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
    }

    private static func fixture(_ edit: (inout [String: Any]) -> Void = { _ in }) throws -> ManagedSnapshot {
        let data = try Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        edit(&value)
        return try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
    }

    private static func source() throws -> String {
        try GlassSurfaceRulesTests.text("Views/ManagedSessionsView.swift")
    }

    /// The fixture has a terminal and every tool installed: the tool
    /// buttons, and nothing else.
    func test_aHealthySnapshotDrawsTheToolButtons() throws {
        XCTAssertEqual(ManagedSessionsSection.toolArea(try Self.fixture()), .buttons)
    }

    /// (a) A failed launch is said on the NEAR AI card, under the buttons
    /// that started it; a failed stop, dismiss, start again or resume, or a launch
    /// with no NEAR AI card drawn, on the sessions card. The lines and the
    /// update links are drawn either way.
    func test_aFailureIsSaidWhereItWasAsked() throws {
        XCTAssertTrue(ManagedSessionsSection.failureOnSignInCard(method: "managed_launch_prepare", signInCardShown: true))
        XCTAssertTrue(ManagedSessionsSection.failureOnSignInCard(method: "managed_tool_setup", signInCardShown: true))
        XCTAssertFalse(ManagedSessionsSection.failureOnSignInCard(method: "managed_tool_setup", signInCardShown: false))
        XCTAssertFalse(ManagedSessionsSection.failureOnSignInCard(method: "managed_launch_prepare", signInCardShown: false))
        for method in ["managed_session_stop", "managed_session_dismiss", "managed_session_start_again", "managed_session_resume", nil] {
            XCTAssertFalse(ManagedSessionsSection.failureOnSignInCard(method: method, signInCardShown: true), method ?? "nil")
        }
        let source = try Self.source()
        for needle in [
            "GlassAlert(model.managedText(\"action_failed\"))",
            "if key == \"unsupported_version\" {",
            "updateRecoveryTools(attemptedTool: model.managedErrorTool)",
            "ManagedInstallLink(tool: availability, copy: model.managedText, labelKey: \"update\", includeToolName: true)",
            "Self.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: true) {\n                actionFailure(key)",
            "if let failure { actionFailure(failure) }",
        ] {
            XCTAssertTrue(source.contains(needle), "ManagedSessionsView.swift lacks \(needle)")
        }
        // The model records which request failed.
        let app = try GlassSurfaceRulesTests.text("AppModel.swift")
        XCTAssertTrue(app.contains("self.managedErrorMethod = method"))
    }

    /// (b) No terminal to launch in: the notice, in place of the tool
    /// buttons, exactly when they are left out.
    func test_noTerminalIsSaidInPlaceOfTheButtons() throws {
        let snapshot = try Self.fixture { value in
            var capabilities = value["capabilities"] as? [String: Any] ?? [:]
            capabilities["terminal_launch"] = false
            value["capabilities"] = capabilities
        }
        XCTAssertEqual(ManagedSessionsSection.toolArea(snapshot), .noTerminal)
        XCTAssertTrue(try Self.source().contains(
            "case .noTerminal:\n                GlassNotice(tone: .ask, title: model.managedText(\"terminal_unavailable\")) { EmptyView() }"))
    }

    /// (c) No supported tool installed: Install for each, in place of the
    /// tool buttons.
    func test_noToolInstalledOffersInstallInPlaceOfTheButtons() throws {
        let snapshot = try Self.fixture { value in
            value["tools"] = ManagedSurface.supportedTools.map {
                ["tool": $0, "installed": false, "install_url": "https://example.com/" + $0] as [String: Any]
            }
        }
        XCTAssertEqual(ManagedSessionsSection.toolArea(snapshot), .install)
        let source = try Self.source()
        for needle in [
            "ManagedNote(text: model.managedText(\"tools_empty\"))",
            "ManagedInstallLink(tool: tool, copy: model.managedText, includeToolName: true)",
        ] {
            XCTAssertTrue(source.contains(needle), "ManagedSessionsView.swift lacks \(needle)")
        }
    }

    /// (d) The list not read yet: the sessions card says it is connecting,
    /// with Retry as the card's own re-read icon, the size the balance
    /// card's is (owner, 2026-10-22), in the core's words even before the
    /// daemon has sent its table.
    func test_anUnreadListSaysConnectingWithRetry() throws {
        let source = try Self.source()
        for needle in [
            "} else {\n                connectingCard\n            }",
            "subtitle: [model.managedText(\"connecting\")]",
            "refresh: .init(model.managedText(\"retry\"), isDisabled: model.managedBusy) {",
        ] {
            XCTAssertTrue(source.contains(needle), "ManagedSessionsView.swift lacks \(needle)")
        }
        XCTAssertFalse(source.contains("Label(model.managedText(\"retry\")"), "Retry is a capsule again")
        let model = AppModel()
        XCTAssertNil(model.managedSnapshot)
        for key in ["title", "connecting", "retry"] {
            XCTAssertFalse(model.managedText(key).isEmpty, "\(key) has no words before the first read")
        }
    }

    /// No card of its own: the states live in the NEAR AI and sessions
    /// cards, and the saved-accounts card stays gone.
    func test_noNewCardAndNoSavedAccounts() throws {
        let source = try Self.source()
        for gone in ["GlassCard {", "accountsCard", "accounts_title", "ManagedToolInstallation"] {
            XCTAssertFalse(source.contains(gone), "ManagedSessionsView.swift draws \(gone)")
        }
    }

    /// Each state renders off screen in the copy the daemon sent, the
    /// unread one with the words kept from the last read.
    func test_eachStateRenders() throws {
        let model = AppModel()
        model.managedSnapshot = try Self.fixture()
        XCTAssertFalse(model.managedText("connecting").isEmpty)
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        model.managedErrorKey = "unsupported_version"
        model.managedErrorTool = "claude"
        model.managedErrorMethod = "managed_launch_prepare"
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-launch-failure.png"))
        model.managedErrorKey = "action_failed"
        model.managedErrorMethod = "managed_session_stop"
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-stop-failure.png"))
        model.managedErrorKey = nil
        model.managedErrorMethod = nil
        model.managedSnapshot = try Self.fixture { value in
            value["tools"] = ManagedSurface.supportedTools.map {
                ["tool": $0, "installed": false, "install_url": "https://example.com/" + $0] as [String: Any]
            }
        }
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-no-tools.png"))
        model.managedSnapshot = try Self.fixture { value in
            var capabilities = value["capabilities"] as? [String: Any] ?? [:]
            capabilities["terminal_launch"] = false
            value["capabilities"] = capabilities
        }
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-no-terminal.png"))
        model.managedSnapshot = nil
        XCTAssertFalse(model.managedText("connecting").isEmpty, "the words outlive the snapshot")
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-connecting.png"))
    }

    private func render<V: View>(_ view: V, to url: URL) throws {
        _ = NSApplication.shared
        let size = CGSize(width: 720, height: 900)
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
        try XCTUnwrap(rep.representation(using: .png, properties: [:])).write(to: url)
    }
}
