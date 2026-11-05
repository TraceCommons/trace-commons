import Foundation
import os
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private func waitForDesktopConfigure(_ started: DispatchSemaphore) -> Bool {
    started.wait(timeout: .now() + 3) == .success
}

private final class DesktopDaemon: DaemonCalling {
    struct Call: Sendable {
        let method: String
        let params: String
    }
    private struct State: Sendable {
        var calls: [Call] = []
        var response: String
    }
    private let state: OSAllocatedUnfairLock<State>
    private let beforeResponse: @Sendable (String) -> Void

    init(state: String = "disconnected", model: String? = nil,
         beforeResponse: @escaping @Sendable (String) -> Void = { _ in }) {
        self.state = OSAllocatedUnfairLock(initialState: State(response: Self.frame(state: state, model: model)))
        self.beforeResponse = beforeResponse
    }

    var calls: [Call] { state.withLock { $0.calls } }
    func answer(state next: String, model: String? = nil, background: String? = nil, environment: String? = nil) {
        state.withLock { $0.response = Self.frame(state: next, model: model, background: background, environment: environment) }
    }
    func call(_ method: String, params: String) -> String {
        let response = state.withLock {
            $0.calls.append(Call(method: method, params: params))
            return $0.response
        }
        beforeResponse(method)
        return response
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }

    static func frame(state: String, model: String? = nil, background: String? = nil, environment: String? = nil) -> String {
        var result: [String: Any] = [
            "state": state, "observed": state == "observed",
            "copy": Dictionary(uniqueKeysWithValues: ClaudeDesktopStatus.copyKeys.map { ($0, $0) }),
        ]
        if let model { result["model"] = model }
        if let background { result["background_model"] = background }
        if let environment { result["environment"] = environment }
        return String(decoding: try! JSONSerialization.data(withJSONObject: ["id": 1, "result": result]), as: UTF8.self)
    }
}

final class ClaudeDesktopClientTests: XCTestCase {
    func testDedicatedMethodsSendOnlyDesktopContractParams() throws {
        let daemon = DesktopDaemon()
        let client = DaemonClient(daemon: daemon)
        _ = try client.claudeDesktopStatus()
        _ = try client.claudeDesktopConfigure(model: "main", backgroundModel: "background")
        _ = try client.claudeDesktopConfigure(model: "main", backgroundModel: nil)
        _ = try client.claudeDesktopPreset()
        _ = try client.claudeDesktopDisconnect()
        XCTAssertEqual(daemon.calls.map(\.method), [
            "claude_desktop_status", "claude_desktop_configure", "claude_desktop_configure",
            "claude_desktop_preset", "claude_desktop_disconnect",
        ])
        let params = try daemon.calls.map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.params.utf8)) as? [String: String])
        }
        XCTAssertEqual(params, [[:], ["model": "main", "background_model": "background"], ["model": "main"], [:], [:]])
    }

    func testStatusAndDisconnectDropUnexpectedCredentialText() throws {
        let daemon = DesktopDaemon()
        daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
        let client = DaemonClient(daemon: daemon)
        XCTAssertNil(try client.claudeDesktopStatus().environment)
        XCTAssertNil(try client.claudeDesktopDisconnect().environment)
        XCTAssertEqual(try client.claudeDesktopPreset().environment, "sensitive-preset")
    }
}

@MainActor
final class ClaudeDesktopSetupTests: XCTestCase {
    private func catalogue() throws -> ManagedModelCatalogue {
        try JSONDecoder().decode(ManagedModelCatalogue.self, from: Data(#"{"models":[{"id":"main","name":"Main label","tier":"near_ai"},{"id":"background","name":"Background label","tier":"near_ai"},{"id":"other","tier":"incognito"}],"default":"main","background":{"main":{"default":"background","set":0}},"background_sets":[["main","background"]]}"#.utf8))
    }

    private func open(_ setup: ClaudeDesktopSetupModel) async throws {
        await setup.refresh()
        setup.open()
        let catalogue = try catalogue()
        setup.loadCatalogue { done in done(catalogue) }
    }

    func testAbsentMissingAndOffFeatureDecisionsCreateNoRowControllerOrIPC() async throws {
        let daemon = DesktopDaemon()
        let client = DaemonClient(daemon: daemon)
        let app = AppModel()
        app.setClientForTesting(client)
        let snapshots: [FeatureFlagSnapshot?] = [
            nil, try DesktopFeatureFixture.snapshot(enabled: nil), try DesktopFeatureFixture.snapshot(enabled: false),
        ]
        for snapshot in snapshots {
            app.featureFlags = snapshot
            XCTAssertFalse(app.featureIsEnabled(ClaudeDesktopSetupModel.featureID))
            XCTAssertNil(app.makeClaudeDesktopSetup())
            // Even a retained/stale caller cannot issue Desktop IPC while
            // the current authoritative feature snapshot is absent or off.
            let setup = ClaudeDesktopSetupModel(client: client, features: app)
            await setup.refresh()
            setup.open()
            setup.loadCatalogue { _ in XCTFail("feature-off must not request a catalogue") }
            await setup.save()
            await setup.copyEnvironment { _ in XCTFail("feature-off must not copy") }
            await setup.disconnect()
            XCTAssertNil(setup.status)
            XCTAssertFalse(setup.isPresented)
        }
        XCTAssertTrue(daemon.calls.isEmpty)
    }

    func testLiveDisableSynchronouslyClosesSheetClearsPresetAndAllowsReenable() async throws {
        let daemon = DesktopDaemon(state: "ready", model: "main")
        let app = try DesktopFeatureFixture.app(client: DaemonClient(daemon: daemon), enabled: true)
        let setup = try XCTUnwrap(app.makeClaudeDesktopSetup())
        try await open(setup)
        daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
        await setup.save()
        XCTAssertNotNil(setup.environment)

        app.featureFlags = try DesktopFeatureFixture.snapshot(enabled: false, generation: 2)
        // No render/run-loop turn is needed to discard the credential.
        XCTAssertNil(setup.environment)
        XCTAssertNil(setup.status)
        XCTAssertFalse(setup.isPresented)
        XCTAssertFalse(setup.canSave)
        XCTAssertFalse(setup.canCopy)
        let before = daemon.calls.count
        await setup.refresh()
        await setup.copyEnvironment { _ in XCTFail("disabled feature copied") }
        await setup.disconnect()
        XCTAssertEqual(daemon.calls.count, before)

        app.featureFlags = try DesktopFeatureFixture.snapshot(enabled: true, generation: 3)
        try await open(setup)
        XCTAssertTrue(setup.isPresented)
        XCTAssertTrue(setup.canSave)
        XCTAssertNil(setup.environment)
        await setup.save()
        XCTAssertEqual(setup.environment, "sensitive-preset")

        app.featureFlags = nil
        XCTAssertNil(setup.environment)
        XCTAssertFalse(setup.isPresented)
    }

    func testLiveDisableDiscardsLateConfigureAndPresetEvenAfterReenable() async throws {
        for blocked in ["claude_desktop_configure", "claude_desktop_preset"] {
            let started = DispatchSemaphore(value: 0)
            let release = DispatchSemaphore(value: 0)
            let daemon = DesktopDaemon(state: "ready", model: "main") { method in
                if method == blocked {
                    started.signal()
                    release.wait()
                }
            }
            let app = try DesktopFeatureFixture.app(client: DaemonClient(daemon: daemon), enabled: true)
            let setup = try XCTUnwrap(app.makeClaudeDesktopSetup())
            try await open(setup)
            daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
            var copied: [String] = []
            let requesting = Task {
                if blocked == "claude_desktop_configure" { await setup.save() }
                else { await setup.copyEnvironment { copied.append($0) } }
            }
            let began = await Task.detached { waitForDesktopConfigure(started) }.value
            XCTAssertTrue(began)
            app.featureFlags = try DesktopFeatureFixture.snapshot(enabled: false, generation: 2)
            XCTAssertFalse(setup.isPresented)
            XCTAssertNil(setup.environment)
            app.featureFlags = try DesktopFeatureFixture.snapshot(enabled: true, generation: 3)
            release.signal()
            await requesting.value
            XCTAssertTrue(copied.isEmpty)
            XCTAssertNil(setup.environment)
            XCTAssertNil(setup.status)
            XCTAssertFalse(setup.isPresented)
            XCTAssertFalse(setup.busy)
        }
    }

    func testSaveUsesConcreteCatalogueDefaultWithoutClipboardWrite() async throws {
        let daemon = DesktopDaemon()
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
        try await open(setup)
        XCTAssertTrue(setup.canSave)
        XCTAssertNil(setup.environment)
        daemon.answer(state: "ready", model: "main", background: "background", environment: "sensitive-preset")
        await setup.save()
        XCTAssertEqual(setup.status?.state, .ready)
        XCTAssertFalse(setup.status?.observed ?? true)
        XCTAssertNil(setup.status?.environment)
        XCTAssertEqual(setup.environment, "sensitive-preset")
        XCTAssertEqual(setup.modelNames["main"], "Main label")
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(try XCTUnwrap(daemon.calls.last).params.utf8)) as? [String: String])
        XCTAssertEqual(params, ["model": "main"])
        var writes: [String] = []
        await setup.copyEnvironment { writes.append($0) }
        XCTAssertEqual(writes, ["sensitive-preset"])
        XCTAssertFalse(daemon.calls.contains { $0.method == "claude_desktop_preset" })
        setup.close()
        XCTAssertNil(setup.environment)
        await setup.copyEnvironment { writes.append($0) }
        XCTAssertEqual(writes.count, 1)
    }

    func testOpeningExistingConnectionDoesNotExportPresetUntilCopy() async throws {
        let daemon = DesktopDaemon(state: "ready", model: "main")
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
        try await open(setup)
        XCTAssertEqual(daemon.calls.map(\.method), ["claude_desktop_status"])
        XCTAssertNil(setup.environment)
        daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
        var writes: [String] = []
        await setup.copyEnvironment { writes.append($0) }
        XCTAssertEqual(writes, ["sensitive-preset"])
        XCTAssertEqual(daemon.calls.last?.method, "claude_desktop_preset")
        setup.chooseModel("other")
        XCTAssertNil(setup.environment)
        XCTAssertFalse(setup.canCopy)
    }

    func testStatusChangeAndDisconnectClearPreset() async throws {
        let daemon = DesktopDaemon()
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
        try await open(setup)
        daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
        await setup.save()
        daemon.answer(state: "observed", model: "main")
        await setup.refresh()
        XCTAssertNil(setup.environment)
        XCTAssertEqual(setup.status?.state, .observed)
        daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
        await setup.copyEnvironment { _ in }
        XCTAssertNotNil(setup.environment)
        daemon.answer(state: "disconnected")
        await setup.disconnect()
        XCTAssertNil(setup.environment)
        XCTAssertEqual(setup.status?.state, .disconnected)
        XCTAssertEqual(daemon.calls.last?.method, "claude_desktop_disconnect")
    }

    func testBlockedStateAndUnreadableCatalogueCannotSave() async throws {
        for state in ["sign_in_required", "disabled", "new_state"] {
            let daemon = DesktopDaemon(state: state)
            let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
            try await open(setup)
            XCTAssertFalse(setup.canSave)
            await setup.save()
            XCTAssertFalse(daemon.calls.contains { $0.method == "claude_desktop_configure" })
        }
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: DesktopDaemon()))
        await setup.refresh()
        setup.open()
        setup.loadCatalogue { $0(nil) }
        XCTAssertFalse(setup.canSave)
        XCTAssertTrue(setup.failed)
    }

    func testRejectedConfigureCanChooseCompatibleModelAndRetryInSameSheet() async throws {
        let daemon = DesktopDaemon()
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
        try await open(setup)
        setup.chooseModel("other")
        daemon.answer(state: "unavailable")
        await setup.save()
        XCTAssertEqual(setup.status?.state, .unavailable)
        XCTAssertNil(setup.status?.model)
        XCTAssertTrue(setup.failed)
        XCTAssertNil(setup.environment)
        XCTAssertTrue(setup.isPresented)
        XCTAssertTrue(setup.status?.canConfigure == true)
        XCTAssertTrue(setup.canSave)
        XCTAssertFalse(setup.canCopy)

        setup.chooseModel("main")
        daemon.answer(state: "ready", model: "main", background: "background", environment: "sensitive-preset")
        await setup.save()
        XCTAssertEqual(setup.status?.state, .ready)
        XCTAssertEqual(setup.status?.model, "main")
        XCTAssertEqual(setup.environment, "sensitive-preset")
        XCTAssertFalse(setup.failed)
        let configured = daemon.calls.filter { $0.method == "claude_desktop_configure" }
        XCTAssertEqual(configured.count, 2)
        let models = try configured.map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.params.utf8)) as? [String: String])["model"]
        }
        XCTAssertEqual(models, ["other", "main"])
    }

    func testUnavailableWithoutModelCanDisconnectAndUnknownStateCannot() async throws {
        let daemon = DesktopDaemon(state: "unavailable")
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
        await setup.refresh()
        XCTAssertTrue(setup.status?.canDisconnect == true)
        daemon.answer(state: "disconnected")
        await setup.disconnect()
        XCTAssertEqual(setup.status?.state, .disconnected)
        XCTAssertEqual(daemon.calls.last?.method, "claude_desktop_disconnect")

        let unknownDaemon = DesktopDaemon(state: "future_state", model: "main")
        let unknown = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: unknownDaemon))
        try await open(unknown)
        await unknown.disconnect()
        await unknown.save()
        XCTAssertEqual(unknownDaemon.calls.map(\.method), ["claude_desktop_status"])
    }

    func testDismissedFormIgnoresLateCatalogueRead() async throws {
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: DesktopDaemon()))
        await setup.refresh()
        setup.open()
        var complete: (@MainActor (ManagedModelCatalogue?) -> Void)?
        setup.loadCatalogue { complete = $0 }
        setup.close()
        complete?(try catalogue())
        XCTAssertNil(setup.catalogue)
        XCTAssertNil(setup.environment)
    }

    func testModelChoiceUsesDaemonBackgroundAllowlist() async throws {
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: DesktopDaemon()))
        try await open(setup)
        setup.chooseBackground("other")
        setup.chooseModel("main")
        XCTAssertEqual(setup.selectedBackgroundModel, "")
        setup.chooseBackground("background")
        setup.chooseModel("main")
        XCTAssertEqual(setup.selectedBackgroundModel, "background")
    }

    func testRepeatedSaveIsDisabledAndDismissalDiscardsLateEnvironment() async throws {
        let started = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        let daemon = DesktopDaemon { method in
            if method == "claude_desktop_configure" {
                started.signal()
                release.wait()
            }
        }
        let setup = try DesktopFeatureFixture.enabledSetup(client: DaemonClient(daemon: daemon))
        try await open(setup)
        daemon.answer(state: "ready", model: "main", environment: "sensitive-preset")
        let saving = Task { await setup.save() }
        let began = await Task.detached { waitForDesktopConfigure(started) }.value
        XCTAssertTrue(began)
        XCTAssertTrue(setup.busy)
        XCTAssertFalse(setup.canSave)
        await setup.save()
        XCTAssertEqual(daemon.calls.filter { $0.method == "claude_desktop_configure" }.count, 1)
        setup.close()
        release.signal()
        await saving.value
        XCTAssertNil(setup.environment)
        XCTAssertFalse(setup.isPresented)
        XCTAssertFalse(setup.busy)
    }
}
