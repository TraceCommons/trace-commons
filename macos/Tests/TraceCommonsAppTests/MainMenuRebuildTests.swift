import Combine
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
}
