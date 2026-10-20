import AppKit
import SwiftUI
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Private AI tab's managed states that lived in the saved-accounts
/// card and must outlive it (#1490 review): a failed launch, stop or
/// dismiss with its update links; no terminal; no supported tool; and the
/// list not read yet, with Retry. Each is pinned by what decides it and by
/// the words the view draws for it.
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

    /// The fixture has a terminal and every tool installed: nothing to say.
    func test_aHealthySnapshotSaysNothing() throws {
        XCTAssertEqual(ManagedSessionsSection.statusItems(try Self.fixture(), errorKey: nil), [])
    }

    /// (a) A failed request is said whatever else holds, signed in or not,
    /// and the view draws the failure lines and the update links.
    func test_aFailedRequestIsSaid() throws {
        XCTAssertEqual(ManagedSessionsSection.statusItems(try Self.fixture(), errorKey: "unsupported_version"),
                       [.failure("unsupported_version")])
        let signedOut = try Self.fixture { value in
            var capabilities = value["capabilities"] as? [String: Any] ?? [:]
            capabilities["near_ai_signin"] = false
            value["capabilities"] = capabilities
        }
        XCTAssertEqual(ManagedSessionsSection.statusItems(signedOut, errorKey: "action_failed"), [.failure("action_failed")])
        let source = try Self.source()
        for needle in [
            "GlassAlert(model.managedText(\"action_failed\"))",
            "if key == \"unsupported_version\" {",
            "updateRecoveryTools(attemptedTool: model.managedErrorTool)",
            "ManagedInstallLink(tool: availability, copy: model.managedText, labelKey: \"update\", includeToolName: true)",
        ] {
            XCTAssertTrue(source.contains(needle), "ManagedSessionsView.swift lacks \(needle)")
        }
    }

    /// (b) No terminal to launch in: the notice, exactly when the NEAR AI
    /// card's tool buttons are left out.
    func test_noTerminalIsSaid() throws {
        let snapshot = try Self.fixture { value in
            var capabilities = value["capabilities"] as? [String: Any] ?? [:]
            capabilities["terminal_launch"] = false
            value["capabilities"] = capabilities
        }
        XCTAssertEqual(ManagedSessionsSection.statusItems(snapshot, errorKey: nil), [.noTerminal])
        XCTAssertTrue(try Self.source().contains(
            "GlassNotice(tone: .ask, title: model.managedText(\"terminal_unavailable\")) { EmptyView() }"))
    }

    /// (c) No supported tool installed: the install list.
    func test_noToolInstalledOffersInstall() throws {
        let snapshot = try Self.fixture { value in
            value["tools"] = ManagedSurface.supportedTools.map {
                ["tool": $0, "installed": false, "install_url": "https://example.com/" + $0] as [String: Any]
            }
        }
        XCTAssertEqual(ManagedSessionsSection.statusItems(snapshot, errorKey: nil), [.installTools])
        XCTAssertTrue(try Self.source().contains(
            "ManagedToolInstallation(tools: snapshot.tools ?? [], copy: model.managedText, retry: model.refreshManagedSessions)"))
    }

    /// (d) The list not read yet: the sessions card says it is connecting,
    /// with Retry; a failure that came with it is said too.
    func test_anUnreadListSaysConnectingWithRetry() throws {
        XCTAssertEqual(ManagedSessionsSection.statusItems(nil, errorKey: nil), [.connecting])
        XCTAssertEqual(ManagedSessionsSection.statusItems(nil, errorKey: "action_failed"), [.connecting, .failure("action_failed")])
        let source = try Self.source()
        for needle in [
            "subtitle: [model.managedText(\"connecting\")]",
            "Label(model.managedText(\"retry\"), systemImage: \"arrow.clockwise\")",
        ] {
            XCTAssertTrue(source.contains(needle), "ManagedSessionsView.swift lacks \(needle)")
        }
    }

    /// The states are drawn under the NEAR AI card and above the sessions,
    /// outside the signed-in branch, so a signed-out tab still shows them.
    func test_theStatusIsDrawnBetweenTheSignInAndTheSessions() throws {
        let source = try Self.source()
        let body = try XCTUnwrap(source.range(of: "    var body: some View {"))
        let rest = String(source[body.upperBound...].prefix(900))
        let signIn = try XCTUnwrap(rest.range(of: "signInCard(snapshot)"))
        let status = try XCTUnwrap(rest.range(of: "statusBlock"))
        let sessions = try XCTUnwrap(rest.range(of: "sessionsCard("))
        XCTAssertLessThan(signIn.lowerBound, status.lowerBound)
        XCTAssertLessThan(status.lowerBound, sessions.lowerBound)
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
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-failure.png"))
        model.managedErrorKey = nil
        model.managedSnapshot = try Self.fixture { value in
            var capabilities = value["capabilities"] as? [String: Any] ?? [:]
            capabilities["terminal_launch"] = false
            value["capabilities"] = capabilities
            value["tools"] = ManagedSurface.supportedTools.map {
                ["tool": $0, "installed": false, "install_url": "https://example.com/" + $0] as [String: Any]
            }
        }
        try render(ManagedSessionsSection().environmentObject(model), to: directory.appendingPathComponent("managed-status-no-terminal-no-tools.png"))
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
