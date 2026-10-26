import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class ManagedToolSetupTests: XCTestCase {
    private func snapshot(capability: Bool?, installed: [String] = []) throws -> ManagedSnapshot {
        var capabilities: [String: Any] = ["managed_launch": true, "terminal_launch": true,
                                            "near_ai_signin": false, "terminal_destination": "Terminal"]
        capabilities["tool_setup"] = capability
        let value: [String: Any] = [
            "revision": 0, "accounts": [], "defaults": [], "generations": [:], "sessions": [],
            "capabilities": capabilities,
            "tools": ManagedSurface.supportedTools.map {
                ["tool": $0, "installed": installed.contains($0), "install_url": "https://example.com/" + $0] as [String: Any]
            }
        ]
        return try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
    }

    func testOlderCapabilitiesKeepGuideFallbackWithoutGuessingTerminalSupport() throws {
        let old = try snapshot(capability: nil)
        XCTAssertNil(old.capabilities.toolSetup)
        XCTAssertEqual(old.missingInstallTools.map(\.tool), ManagedSurface.supportedTools)
        let source = try GlassSurfaceRulesTests.text("Views/ManagedSessionsView.swift")
        XCTAssertTrue(source.contains("if model.managedSnapshot?.capabilities.toolSetup == true {"))
        XCTAssertTrue(source.contains("Button(label) { model.setupManagedTool(tool.tool, action: labelKey) }"))
        XCTAssertTrue(source.contains("} else if let url = tool.url {\n            Link(label, destination: url)"))
    }

    func testMissingInstallButtonsStayAlongsideEachInstalledTool() throws {
        for installed in ManagedSurface.supportedTools {
            let value = try snapshot(capability: true, installed: [installed])
            XCTAssertEqual(value.missingInstallTools.map(\.tool), ManagedSurface.supportedTools.filter { $0 != installed })
        }
        let source = try GlassSurfaceRulesTests.text("Views/ManagedSessionsView.swift")
        XCTAssertTrue(source.contains("ForEach(snapshot.signInStartTools, id: \\.self)"))
        XCTAssertTrue(source.contains("ForEach(snapshot.missingInstallTools)"))
        XCTAssertTrue(source.contains("if !snapshot.nearAiSignedIn, !snapshot.missingInstallTools.isEmpty {\n                    setupOnlyButtons(snapshot)"))
    }

    func testSetupBeforeSignInSendsOnlyNamedToolAndAction() async throws {
        for tool in ManagedSurface.supportedTools {
            for action in ["install", "update"] {
                let daemon = ToolSetupDaemon(reply: #"{"opened":true}"#)
                let model = AppModel()
                model.setClientForTesting(DaemonClient(daemon: daemon))
                model.managedSnapshot = try snapshot(capability: true)
                XCTAssertFalse(model.managedSnapshot!.nearAiSignedIn)
                XCTAssertTrue(model.managedSnapshot!.accounts.isEmpty)
                model.setupManagedTool(tool, action: action)
                try await waitForSetup(model)
                XCTAssertEqual(daemon.requests.filter { $0.method == "managed_tool_setup" }.map(\.params),
                               [["tool": tool, "action": action]])
                XCTAssertFalse(daemon.requests.contains { $0.method == "managed_terminal_launch" })
                XCTAssertNil(model.managedErrorKey)
            }
        }
    }

    func testUnsupportedCapabilityAndUnknownEnumsSendNothing() throws {
        for capability in [nil, false] as [Bool?] {
            let daemon = ToolSetupDaemon(reply: #"{"opened":true}"#)
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: daemon))
            model.managedSnapshot = try snapshot(capability: capability)
            model.setupManagedTool("codex", action: "install")
            XCTAssertTrue(daemon.requests.isEmpty)
        }
        let daemon = ToolSetupDaemon(reply: #"{"opened":true}"#)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try snapshot(capability: true)
        model.setupManagedTool("codex; other", action: "install")
        model.setupManagedTool("codex", action: "install; other")
        XCTAssertTrue(daemon.requests.isEmpty)
    }

    func testUnconfirmedAndRefusedSetupRetainAttemptedToolAndActionLocation() async throws {
        for response in [#"{"opened":false}"#, #"{}"#, nil] as [String?] {
            let daemon = ToolSetupDaemon(reply: response)
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: daemon))
            model.managedSnapshot = try snapshot(capability: true)
            model.setupManagedTool("codex", action: "update")
            try await waitForSetup(model)
            XCTAssertNotNil(model.managedErrorKey)
            XCTAssertEqual(model.managedErrorTool, "codex")
            XCTAssertEqual(model.managedErrorMethod, "managed_tool_setup")
            XCTAssertTrue(ManagedSessionsSection.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: true))
            XCTAssertFalse(ManagedSessionsSection.failureOnSignInCard(method: model.managedErrorMethod, signInCardShown: false))
        }
    }

    private func waitForSetup(_ model: AppModel) async throws {
        for _ in 0..<300 where model.managedBusy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.managedBusy)
    }
}

private final class ToolSetupDaemon: DaemonCalling, @unchecked Sendable {
    struct Request { let method: String; let params: [String: String] }
    private let lock = NSLock()
    private var sent: [Request] = []
    private let reply: String?
    init(reply: String?) { self.reply = reply }
    var requests: [Request] { lock.withLock { sent } }

    func call(_ method: String, params paramsJSON: String) -> String {
        let params = (try? JSONDecoder().decode([String: String].self, from: Data(paramsJSON.utf8))) ?? [:]
        lock.withLock { sent.append(Request(method: method, params: params)) }
        if method == "managed_tool_setup" {
            return reply.map { #"{"id":1,"result":\#($0)}"# }
                ?? #"{"id":1,"error":{"code":"unavailable","message":"managed-terminal-not-found"}}"#
        }
        return #"{"id":1,"result":{"revision":0,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":true,"tool_setup":true}}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
