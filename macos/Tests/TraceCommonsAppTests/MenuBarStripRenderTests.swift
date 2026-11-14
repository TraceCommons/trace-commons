import AppKit
import TCBridge
import TCDesign
import XCTest
@testable import TraceCommonsApp

/// The menu-bar strip is rasterized only when what it draws changes.
///
/// The label observes the whole `AppModel`, so its body runs on every model
/// publish. Each run used to draw the strip through `ImageRenderer` twice
/// (dark and light) and hand the status item a new image, whatever had
/// changed.
@MainActor
final class MenuBarStripRenderTests: XCTestCase {
    private func inputs(badge: Int? = nil) -> MenuBarStripInputs {
        MenuBarStripInputs(
            traces: [GlassDayColumn(id: "a", up: 1, down: 2)], privateAI: nil,
            condition: .live, badge: badge, mark: .none, scale: 2, graph: true)
    }

    /// Equal inputs draw once and return the same image; a change to what is
    /// drawn draws again.
    func testEqualInputsDrawOnceAndAChangeDrawsAgain() {
        let cache = MenuBarStripImageCache()
        var renders = 0
        let render: (MenuBarStripInputs) -> NSImage = { _ in
            renders += 1
            return NSImage(size: NSSize(width: 1, height: 1))
        }
        let first = cache.image(for: inputs(), render: render)
        let second = cache.image(for: inputs(), render: render)
        XCTAssertEqual(renders, 1, "equal inputs were drawn again")
        XCTAssertTrue(first === second, "equal inputs handed the status item a new image")
        _ = cache.image(for: inputs(badge: 3), render: render)
        XCTAssertEqual(renders, 2, "a new badge was not drawn")
        _ = cache.image(for: inputs(badge: 3), render: render)
        XCTAssertEqual(renders, 2)
    }

    /// A model publish that changes nothing the strip draws leaves its
    /// inputs equal; one that does changes them.
    func testOnlyWhatTheStripDrawsChangesItsInputs() throws {
        let model = AppModel()
        let store = MenuPanelStore(client: nil)
        let before = MenuBarStripInputs.of(model: model, store: store, scale: 2)
        model.setDaemonSettingsForTesting(try MainMenuRebuildTests.settings(privateInference: true))
        XCTAssertEqual(MenuBarStripInputs.of(model: model, store: store, scale: 2), before)
        // Decisions owed draw the badge, whatever the reads behind the bars.
        model.setStatusForTesting(DaemonStatus(
            schemaVersion: "1.1", loggedIn: true, tenantID: "fixture", consentScopes: [], paused: false,
            queueDepth: 3, decisionsOwed: 3, nextDigestAt: nil,
            health: DaemonHealth(lastErrorLabel: nil, since: nil)))
        XCTAssertNotEqual(MenuBarStripInputs.of(model: model, store: store, scale: 2), before)
        XCTAssertNotEqual(MenuBarStripInputs.of(model: model, store: store, scale: 3), before)
    }

    /// The label draws through the cache, not straight through the renderer.
    func testTheLabelDrawsThroughTheCache() throws {
        let source = PreviewSheetTests.code(try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift"))
        let label = try XCTUnwrap(source.range(of: "struct MenuBarStripLabel: View {"))
        let end = try XCTUnwrap(source.range(of: "\n}\n", range: label.upperBound..<source.endIndex))
        let body = source[label.lowerBound..<end.lowerBound]
        XCTAssertTrue(body.contains("cache.image(for: inputs"))
        XCTAssertEqual(body.components(separatedBy: "ImageRenderer(").count - 1, 1)
    }
}
