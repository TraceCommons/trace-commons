import SwiftUI
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// The flow map at rest: the arcs' dashes and the library each on their
/// own timeline, and a still layer that is not drawn again on their ticks.
final class FlowMapMotionTests: XCTestCase {
    /// Dashes run while flows are live and healthy, the library turns
    /// while there is one, each independently; nothing moves under Reduce
    /// Motion or when there is nothing to move.
    @MainActor
    func test_restingMotion() {
        let dashes = FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: true, library: false)
        XCTAssertTrue(dashes.dashes)
        XCTAssertFalse(dashes.turns)

        let both = FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: true, library: true)
        XCTAssertTrue(both.dashes)
        XCTAssertTrue(both.turns)

        let library = FlowMapView.restingMotion(reduceMotion: false, flows: false, healthy: true, library: true)
        XCTAssertFalse(library.dashes)
        XCTAssertTrue(library.turns)

        // Unhealthy flows are drawn still: only the library turns.
        let unhealthy = FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: false, library: true)
        XCTAssertFalse(unhealthy.dashes)
        XCTAssertTrue(unhealthy.turns)

        let still = FlowMapView.restingMotion(reduceMotion: true, flows: true, healthy: true, library: true)
        XCTAssertFalse(still.dashes)
        XCTAssertFalse(still.turns)
    }

    /// The dashes keep their 30 frames a second; the library turns slower
    /// than that, so it is no longer drawn at the dashes' rate.
    @MainActor
    func test_libraryTurnsSlowerThanTheDashes() {
        XCTAssertEqual(FlowMapView.dashInterval, 1 / 30)
        XCTAssertGreaterThan(GlassLibraryStation.frameInterval, FlowMapView.dashInterval)
    }

    /// The still layer compares by its key alone: a new drawing closure
    /// with the same key is the same layer, and SwiftUI keeps its drawing.
    @MainActor
    func test_stillLayerComparesByKey() {
        let a = FlowMapStillLayer(key: 1) { _ in }
        let b = FlowMapStillLayer(key: 1) { _ in }
        let c = FlowMapStillLayer(key: 2) { _ in }
        XCTAssertTrue(a == b)
        XCTAssertFalse(a == c)
    }

    /// The map's geometry is part of the still layer's key: a zoom or a
    /// resize draws the nodes again, the same field does not.
    func test_geometryEquality() {
        let size = CGSize(width: 580, height: 760)
        XCTAssertEqual(FlowMapGeometry(size: size, zoom: 1), FlowMapGeometry(size: size, zoom: 1))
        XCTAssertNotEqual(FlowMapGeometry(size: size, zoom: 1), FlowMapGeometry(size: size, zoom: 1.25))
        XCTAssertNotEqual(FlowMapGeometry(size: size, zoom: 1), FlowMapGeometry(size: CGSize(width: 600, height: 760), zoom: 1))
    }
}
