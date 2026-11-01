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

    /// #1754 review, the view-only findings: Settings reads NEAR AI only
    /// while there is a favorite; the card draws from the stored list with
    /// no state of its own; a star or removal refreshes the settings every
    /// window reads and re-reads the catalogue instead of recomputing it;
    /// and a refused star's line does not outlive the account it was for.
    func testFavoritesViewsFollowTheReview() throws {
        func text(_ rel: String) throws -> String {
            try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
        }
        let section = try text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(section.contains(".task(id: hasFavorites)"), "the catalogue read is keyed on having a favorite")
        let card = try text("Views/Settings/ManagedFavoritesCard.swift")
        XCTAssertFalse(card.contains("@State private var stored"))
        XCTAssertFalse(card.contains("withFavorites"))
        XCTAssertTrue(card.contains("model.daemonSettings?.managedFavoriteModels"))
        let options = try text("Views/ManagedFolderOptions.swift")
        XCTAssertFalse(options.contains("withFavorites"))
        // A new account or folder redraws the fields, and `.onDisappear`
        // clears a refused star's line; an `.onChange(of: draft.phase)`
        // inside the fields never saw a change (#1754 review).
        XCTAssertFalse(options.contains(".onChange(of: draft.phase)"))
        XCTAssertTrue(options.contains(".onDisappear { favoriteError = nil }"))
        // A star shows the setter's answer at once; the read again that
        // follows belongs to the sheet, which drops another tool's answer.
        XCTAssertTrue(options.contains("catalogue = catalogue?.projecting(storedFavorites: favorites)"))
        XCTAssertFalse(options.contains("model.managedModelCatalogue("))
        // Settings follow the daemon's `status_changed`, not a second read.
        let app = try text("AppModel.swift")
        XCTAssertFalse(app.contains("if favorites != nil { self.refreshSettings() }"))
    }

    /// #1754 review: a failed catalogue read no longer holds Launch with
    /// no word of why. While the read is pending a saved model is held as
    /// before; once it has failed, Launch goes, and the daemon refuses a
    /// model NEAR AI does not list (`managed-model-unavailable`).
    @MainActor
    func testAFailedCatalogueReadDoesNotHoldLaunch() {
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        draft.model = "a/one"
        XCTAssertFalse(ManagedFolderOptions.launchModelReady(draft, tool: "claude", connection: "near_ai", catalogue: nil, catalogueFailed: false))
        XCTAssertTrue(ManagedFolderOptions.launchModelReady(draft, tool: "claude", connection: "near_ai", catalogue: nil, catalogueFailed: true))
        // OpenCode's typed model still needs text, read or not.
        draft.useDefaults(tool: "opencode", cwd: "/project", cliOptions: "")
        XCTAssertFalse(ManagedFolderOptions.launchModelReady(draft, tool: "opencode", connection: "api_key", catalogue: nil, catalogueFailed: true))
    }

    /// Launch is held for a model the catalogue does not list, and while
    /// there is no catalogue to check it against; the default always goes.
    @MainActor
    func testLaunchNeverNamesAModelTheCatalogueDoesNotList() throws {
        let catalogue = try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"a/one"}],"default":"a/one"}"#.utf8))
        var draft = ManagedFolderDraft()
        draft.useDefaults(tool: "claude", cwd: "/project", cliOptions: "")
        for connection in ["near_ai", "near_ai_signin"] {
            draft.model = nil
            XCTAssertTrue(ManagedFolderOptions.modelIsAvailable(draft, tool: "claude", connection: connection, catalogue: catalogue))
            XCTAssertTrue(ManagedFolderOptions.modelIsAvailable(draft, tool: "claude", connection: connection, catalogue: nil))
            draft.model = "a/one"
            XCTAssertTrue(ManagedFolderOptions.modelIsAvailable(draft, tool: "claude", connection: connection, catalogue: catalogue))
            XCTAssertFalse(ManagedFolderOptions.modelIsAvailable(draft, tool: "claude", connection: connection, catalogue: nil))
            draft.model = "gone/model"
            XCTAssertFalse(ManagedFolderOptions.modelIsAvailable(draft, tool: "claude", connection: connection, catalogue: catalogue))
        }
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
                draft: .constant(draft), catalogue: .constant(nil), refreshCatalogue: {}, supportsFolderSettings: true, retry: {}, reset: {})
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
