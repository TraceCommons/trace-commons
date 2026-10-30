import AppKit
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
import os
@testable import TraceCommonsApp

final class ManagedFolderLaunchTests: XCTestCase {
    @MainActor
    func testLookupAndResetSendOnlyFolderAndTool() async throws {
        for reset in [false, true] {
            let daemon = FolderSettingsDaemon()
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: daemon))
            var result: ManagedFolderSettings?
            var finished = false
            model.managedFolderSettings(tool: "codex", cwd: "/project", reset: reset) { result = $0; finished = true }
            for _ in 0..<300 where !finished { try await Task.sleep(for: .milliseconds(10)) }
            XCTAssertTrue(finished)
            XCTAssertEqual(daemon.method, reset ? "managed_folder_settings_reset" : "managed_folder_settings_get")
            XCTAssertEqual(daemon.params, ["tool": "codex", "cwd": "/project"])
            XCTAssertEqual(result?.model, "selected-model")
            XCTAssertEqual(result?.collectTraces, true)
        }
    }

    @MainActor
    func testLookupWithNoFolderSendsNoCwd() async throws {
        let daemon = FolderSettingsDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        var finished = false
        model.managedFolderSettings(tool: "codex", cwd: nil) { _ in finished = true }
        for _ in 0..<300 where !finished { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(finished)
        XCTAssertEqual(daemon.params, ["tool": "codex"])
    }

    @MainActor
    func testRememberedUnavailableModelCannotBeLaunchedUntilChanged() throws {
        let value = try JSONDecoder().decode(ManagedFolderSettings.self, from: Data(#"{"tool":"codex","model":"removed-model","collect_traces":true,"cli_options":"","remembered":true,"revision":3}"#.utf8))
        var draft = ManagedFolderDraft()
        let token = draft.begin(tool: "codex", cwd: "/project")
        draft.complete(value, token: token)
        XCTAssertFalse(ManagedFolderOptions.modelIsAvailable(draft, tool: "codex", connection: "near_ai_signin", catalogue: nil))
        draft.model = nil
        XCTAssertTrue(ManagedFolderOptions.modelIsAvailable(draft, tool: "codex", connection: "near_ai_signin", catalogue: nil))
    }

    @MainActor
    func testFolderScreenShowsCollectionOptionsForAllThreeAgents() throws {
        _ = NSApplication.shared
        let model = AppModel()
        XCTAssertEqual(model.managedText("collect_managed_trace"), "Collect trace for review")
        for tool in ManagedSurface.supportedTools {
            var draft = ManagedFolderDraft()
            draft.useDefaults(tool: tool, cwd: "/project", cliOptions: "--custom")
            draft.collectTraces = true
            let view = NSHostingView(rootView: ManagedFolderOptions(tool: tool, connection: "near_ai_signin",
                draft: .constant(draft), catalogue: .constant(nil), supportsFolderSettings: true, retry: {}, reset: {})
                .environmentObject(model).padding(20).frame(width: 520)
                .background(Color(nsColor: .windowBackgroundColor)))
            let size = view.fittingSize
            XCTAssertGreaterThan(size.height, 120)
            view.frame = CGRect(origin: .zero, size: size)
            view.layoutSubtreeIfNeeded()
            let rep = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: rep)
            let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(png.count, 1000)
            if let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] {
                try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
                try png.write(to: URL(fileURLWithPath: path).appendingPathComponent("folder-options-\(tool).png"))
            }
        }
    }
}

private final class FolderSettingsDaemon: DaemonCalling {
    private let state = OSAllocatedUnfairLock(initialState: (method: "", params: [String: String]()))
    var method: String { state.withLock { $0.method } }
    var params: [String: String] { state.withLock { $0.params } }
    func call(_ method: String, params paramsJSON: String) -> String {
        let params = (try? JSONDecoder().decode([String: String].self, from: Data(paramsJSON.utf8))) ?? [:]
        state.withLock { $0 = (method, params) }
        return #"{"id":1,"result":{"tool":"codex","model":"selected-model","collect_traces":true,"cli_options":"--custom","remembered":true,"revision":3}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
