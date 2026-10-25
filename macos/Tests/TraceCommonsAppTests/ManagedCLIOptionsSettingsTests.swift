import AppKit
import SwiftUI
import TCShellCore
import XCTest
import TCBridge
import os
@testable import TraceCommonsApp

final class ManagedCLIOptionsSettingsTests: XCTestCase {
    @MainActor
    func testDraftStaysInModelAcrossSnapshotRefresh() throws {
        let model = AppModel()
        model.managedSnapshot = try snapshot(options: "--model old")
        model.managedCLIOptionsDrafts.edit(tool: "codex", options: "--model new")
        model.managedSnapshot = try snapshot(options: "--model external")
        XCTAssertEqual(model.managedCLIOptionsDrafts.value(tool: "codex", saved: model.managedSnapshot?.cliOptions), "--model new")
    }

    @MainActor
    func testSaveConfirmsTheExactOptionsAndKeepsOtherAgentDrafts() async throws {
        let model = AppModel()
        let daemon = CLIOptionsDaemon(reply: .echo)
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try snapshot(options: "--model old")
        model.managedCLIOptionsDrafts.edit(tool: "codex", options: "-c 'model_reasoning_effort=high'")
        model.managedCLIOptionsDrafts.edit(tool: "claude", options: "--model sonnet")
        model.saveManagedCLIOptions(tool: "codex")
        try await waitForSave(model)
        XCTAssertEqual(daemon.sent?["tool"], "codex")
        XCTAssertEqual(daemon.sent?["options"], "-c 'model_reasoning_effort=high'")
        XCTAssertEqual(model.managedSnapshot?.cliOptions?["codex"], "-c 'model_reasoning_effort=high'")
        XCTAssertFalse(model.managedCLIOptionsDrafts.isChanged(tool: "codex", saved: model.managedSnapshot?.cliOptions))
        XCTAssertTrue(model.managedCLIOptionsDrafts.isChanged(tool: "claude", saved: model.managedSnapshot?.cliOptions))
        XCTAssertNil(model.managedErrorKey)
    }

    @MainActor
    func testRefusedOrUnconfirmedSaveRetainsTheDraft() async throws {
        for reply in [CLIOptionsDaemon.Reply.refuse, .mismatch] {
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: CLIOptionsDaemon(reply: reply)))
            model.managedSnapshot = try snapshot(options: "--model old")
            model.managedCLIOptionsDrafts.edit(tool: "codex", options: "--model new")
            model.saveManagedCLIOptions(tool: "codex")
            try await waitForSave(model)
            XCTAssertEqual(model.managedSnapshot?.cliOptions?["codex"], "--model old")
            XCTAssertEqual(model.managedCLIOptionsDrafts.value(tool: "codex", saved: model.managedSnapshot?.cliOptions), "--model new")
            XCTAssertNotNil(model.managedErrorKey)
            XCTAssertEqual(model.managedErrorMethod, "managed_cli_options_set")
            XCTAssertEqual(model.managedErrorTool, "codex")
        }
    }

    @MainActor
    func testOlderInFlightSnapshotCannotUndoAConfirmedSave() async throws {
        let model = AppModel()
        let oldJSON = #"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":false},"cli_options":{"codex":"--model old"}}"#
        let daemon = CLIOptionsDaemon(reply: .echo, parkedSnapshot: oldJSON)
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.managedSnapshot = try snapshot(options: "--model old")
        model.refreshManagedSessions()
        for _ in 0..<300 where !daemon.snapshotWaiting { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(daemon.snapshotWaiting)
        defer { daemon.releaseSnapshot() }
        model.managedCLIOptionsDrafts.edit(tool: "codex", options: "--model new")
        model.saveManagedCLIOptions(tool: "codex")
        try await waitForSave(model)
        daemon.releaseSnapshot()
        for _ in 0..<300 where !model.answeredReads.contains("managed_snapshot") { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(model.answeredReads.contains("managed_snapshot"))
        XCTAssertEqual(model.managedSnapshot?.revision, 2)
        XCTAssertEqual(model.managedSnapshot?.cliOptions?["codex"], "--model new")
    }

    @MainActor
    private func waitForSave(_ model: AppModel) async throws {
        for _ in 0..<300 where model.managedBusy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.managedBusy)
    }

    @MainActor
    func testOptionsCardRendersWithoutStartingAnAgent() throws {
        _ = NSApplication.shared
        let model = AppModel()
        model.managedSnapshot = try snapshot(options: "-c model_reasoning_effort=high")
        let view = NSHostingView(rootView: ManagedCLIOptionsSection().environmentObject(model).padding(20).frame(width: 640))
        let size = view.fittingSize
        XCTAssertGreaterThan(size.height, 150)
        view.frame = CGRect(origin: .zero, size: size)
        view.layoutSubtreeIfNeeded()
        let rep = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: rep)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        if let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] {
            try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
            try png.write(to: URL(fileURLWithPath: path).appendingPathComponent("managed-cli-options.png"))
        }
    }

    private func snapshot(options: String) throws -> ManagedSnapshot {
        let optionsJSON = String(decoding: try JSONEncoder().encode(["claude": "--model opus", "codex": options, "opencode": ""]), as: UTF8.self)
        return try JSONDecoder().decode(ManagedSnapshot.self, from: Data((#"{"revision":1,"accounts":[],"defaults":[],"generations":{},"sessions":[],"capabilities":{"managed_launch":true,"terminal_launch":false},"cli_options":"# + optionsJSON + "}").utf8))
    }
}

private final class CLIOptionsDaemon: DaemonCalling {
    enum Reply: Sendable { case echo, refuse, mismatch }
    private let reply: Reply
    private let parkedSnapshot: String?
    private let snapshotCalls = OSAllocatedUnfairLock(initialState: 0)
    private let gate = DispatchSemaphore(value: 0)
    var snapshotWaiting: Bool { snapshotCalls.withLock { $0 > 0 } }
    func releaseSnapshot() { gate.signal() }
    private let captured = OSAllocatedUnfairLock<[String: String]?>(initialState: nil)
    var sent: [String: String]? { captured.withLock { $0 } }
    init(reply: Reply, parkedSnapshot: String? = nil) { self.reply = reply; self.parkedSnapshot = parkedSnapshot }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == "managed_snapshot", let parkedSnapshot {
            let first = snapshotCalls.withLock { calls in calls += 1; return calls == 1 }
            if first {
                gate.wait()
                return #"{"id":1,"result":"# + parkedSnapshot + "}"
            }
        }
        guard method == "managed_cli_options_set" else {
            return #"{"id":1,"error":{"code":"unavailable","message":"test-refusal"}}"#
        }
        let params = (try? JSONDecoder().decode([String: String].self, from: Data(paramsJSON.utf8))) ?? [:]
        captured.withLock { $0 = params }
        switch reply {
        case .echo:
            var response: [String: Any] = params
            response["revision"] = 2
            let data = try! JSONSerialization.data(withJSONObject: response)
            return #"{"id":1,"result":"# + String(decoding: data, as: UTF8.self) + "}"
        case .refuse: return #"{"id":1,"error":{"code":"managed-cli-options-invalid","message":"managed-cli-options-invalid"}}"#
        case .mismatch: return #"{"id":1,"result":{"tool":"codex","options":"other"}}"#
        }
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
