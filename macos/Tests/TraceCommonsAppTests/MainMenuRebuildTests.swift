import Combine
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The main menu is rebuilt only when what it shows changes.
///
/// SwiftUI rebuilds every scene and the whole main menu when the `App` body
/// or a `Commands` value's observed object changes. `AppModel` publishes on
/// every daemon status update, so observing it in either place kept the main
/// thread rebuilding menus, about one core while the app sat idle
/// (2026-11-11, 1.0.0).
@MainActor
final class MainMenuRebuildTests: XCTestCase {
    /// The app holds the model without observing it, and the menu commands
    /// observe only the Private AI item.
    func testNeitherTheAppNorItsCommandsObserveTheModel() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        XCTAssertFalse(main.contains("@StateObject private var model = AppModel()"))
        XCTAssertTrue(main.contains("@State private var model: AppModel"))
        let commands = try XCTUnwrap(main.range(of: "struct MonitorCommands: Commands {"))
        let end = try XCTUnwrap(main.range(of: "\n}\n", range: commands.upperBound..<main.endIndex))
        let body = main[commands.lowerBound..<end.lowerBound]
        XCTAssertFalse(body.contains(": AppModel"), "MonitorCommands observes the whole model again")
        XCTAssertTrue(body.contains("@ObservedObject var privateInference: PrivateInferenceMenuItem"))
    }

    /// A model change that leaves the switch as it was publishes nothing to
    /// the menu.
    func testAnUnrelatedModelChangeDoesNotRepublishTheItem() async throws {
        let model = AppModel()
        let item = PrivateInferenceMenuItem(model: model)
        var published = 0
        let watch = item.objectWillChange.sink { published += 1 }
        defer { watch.cancel() }
        for _ in 0..<20 { model.objectWillChange.send() }
        // The item reads the model on the next turn of the run loop.
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(published, 0)
        XCTAssertEqual(item.state, PrivateInferenceMenuItem.state(of: model))
    }

    /// Turning Private AI on or off does reach the menu: the item
    /// republishes, and its label is the switch's own for the new state.
    /// Without this, an item that never refreshed would pass the test above.
    func testTurningPrivateAIOnOrOffRepublishesTheItem() async throws {
        let model = AppModel()
        guard let copy = model.privateInferenceCopy else {
            return XCTFail("the core's Private AI words did not load")
        }
        let item = PrivateInferenceMenuItem(model: model)
        var published = 0
        let watch = item.objectWillChange.sink { published += 1 }
        defer { watch.cancel() }

        for on in [true, false] {
            let before = published
            model.setDaemonSettingsForTesting(try Self.settings(privateInference: on))
            // The item reads the model on the next turn of the run loop.
            try await Task.sleep(for: .milliseconds(100))
            XCTAssertGreaterThan(published, before, "the menu was not told Private AI turned \(on ? "on" : "off")")
            XCTAssertEqual(item.state?.on, on)
            XCTAssertEqual(item.state?.label, PrivateInferenceTray.label(on: on, copy: copy))
        }
    }

    /// A settings view with Private AI on or off, decoded the way the
    /// daemon's answer is.
    private static func settings(privateInference on: Bool) throws -> DaemonSettingsView {
        try DaemonClient(daemon: FixedDaemon(response: """
            {"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,
            "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
            "max_uploads_per_day":100,"near_ai_configured":false,
            "claude_root_configured":true,"codex_root_configured":true,
            "private_inference":\(on),"private_inference_offer_seen":true,
            "private_inference_state":{"state":"\(on ? "running" : "off")","port":null}}}
            """)).settings()
    }
}

/// Answers every call with one frame.
private final class FixedDaemon: DaemonCalling {
    let response: String
    init(response: String) { self.response = response }
    func call(_ method: String, params paramsJSON: String) -> String { response }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
