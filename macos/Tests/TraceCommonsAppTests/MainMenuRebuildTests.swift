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
    /// The app and its Monitor scene hold the model without observing it,
    /// and the menu commands observe only the Private AI item. Comment lines
    /// are left out, so prose that names the model is not taken for code.
    func testNeitherTheAppNorItsCommandsObserveTheModel() throws {
        let main = PreviewSheetTests.code(try MonitorNavigationTests.text("TraceCommonsAppMain.swift"))
        XCTAssertFalse(main.contains("@StateObject private var model = AppModel()"))
        XCTAssertTrue(main.contains("@State private var model: AppModel"))
        let commands = try XCTUnwrap(main.range(of: "struct MonitorCommands: Commands {"))
        let end = try XCTUnwrap(main.range(of: "\n}\n", range: commands.upperBound..<main.endIndex))
        let body = main[commands.lowerBound..<end.lowerBound]
        XCTAssertFalse(body.contains("AppModel"), "MonitorCommands names the model again")
        XCTAssertTrue(body.contains("@ObservedObject var privateInference: PrivateInferenceMenuItem"))
        let scene = try XCTUnwrap(main.range(of: "struct MonitorScene: Scene {"))
        let sceneEnd = try XCTUnwrap(main.range(of: "\n}\n", range: scene.upperBound..<main.endIndex))
        let sceneBody = main[scene.lowerBound..<sceneEnd.lowerBound]
        XCTAssertTrue(sceneBody.contains("    let model: AppModel"), "MonitorScene observes the model")
        XCTAssertFalse(sceneBody.contains("@ObservedObject var model"))
        XCTAssertFalse(sceneBody.contains("@StateObject"))
    }

    /// The command-line tool's item is rebuilt when its status changes: the
    /// menu commands observe its controller themselves. A View observing it
    /// inside a CommandGroup is not known to rebuild the menu's item.
    func testTheMenuCommandsObserveTheCliToolController() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        let commands = try XCTUnwrap(main.range(of: "struct MonitorCommands: Commands {"))
        let end = try XCTUnwrap(main.range(of: "\n}\n", range: commands.upperBound..<main.endIndex))
        let body = main[commands.lowerBound..<end.lowerBound]
        XCTAssertTrue(body.contains("@ObservedObject var cliTool: CliToolController"))
        XCTAssertTrue(body.contains("CliToolMenuItem(controller: cliTool)"))
    }

    /// A model change that leaves the switch as it was publishes nothing to
    /// the menu.
    func testAnUnrelatedModelChangeDoesNotRepublishTheItem() throws {
        let model = AppModel()
        let item = PrivateInferenceMenuItem(model: model)
        var published = 0
        let watch = item.objectWillChange.sink { published += 1 }
        defer { watch.cancel() }
        for startup in [AppModel.Startup.running, .refused("r"), .starting] {
            model.setStartupForTesting(startup)
        }
        XCTAssertEqual(model.startup, .starting)
        XCTAssertEqual(published, 0)
        XCTAssertEqual(item.state, PrivateInferenceMenuItem.state(of: model))
    }

    /// Turning Private AI on or off does reach the menu, at once: the item
    /// republishes, its label is the switch's own for the new state, and a
    /// turn-off in flight disables it. Without this, an item that never
    /// refreshed would pass the test above.
    func testTurningPrivateAIOnOrOffRepublishesTheItem() throws {
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
            XCTAssertGreaterThan(published, before, "the menu was not told Private AI turned \(on ? "on" : "off")")
            XCTAssertEqual(item.state?.on, on)
            XCTAssertEqual(item.state?.label, PrivateInferenceTray.label(on: on, copy: copy))
            XCTAssertEqual(item.state?.disabled, false)
        }

        model.setDaemonSettingsForTesting(try Self.settings(privateInference: true))
        model.setPrivateInferenceBusyForTesting(true)
        XCTAssertEqual(item.state?.disabled, true, "a turn-off in flight left the item enabled")
        model.setPrivateInferenceBusyForTesting(false)
        XCTAssertEqual(item.state?.disabled, false)
        // Only the writing direction is ever unavailable.
        model.setDaemonSettingsForTesting(try Self.settings(privateInference: false))
        model.setPrivateInferenceBusyForTesting(true)
        XCTAssertEqual(item.state?.disabled, false)
    }

    /// A settings view with Private AI on or off, decoded the way the
    /// daemon's answer is.
    static func settings(privateInference on: Bool) throws -> DaemonSettingsView {
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
