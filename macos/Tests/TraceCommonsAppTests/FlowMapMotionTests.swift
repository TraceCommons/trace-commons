import SwiftUI
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// The flow map at rest: one timeline for what moves (the arcs' dashes
/// and the library), and a still layer that is not drawn again on its
/// ticks.
final class FlowMapMotionTests: XCTestCase {
    /// Dashes run at 30 frames a second, as they always have; the library
    /// alone at its own rate; nothing moves under Reduce Motion or when
    /// there is nothing to move.
    @MainActor
    func test_restingMotion() {
        let dashes = FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: true, library: false)
        XCTAssertEqual(dashes.interval, 1 / 30)
        XCTAssertFalse(dashes.paused)

        let both = FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: true, library: true)
        XCTAssertEqual(both.interval, 1 / 30)
        XCTAssertFalse(both.paused)

        let library = FlowMapView.restingMotion(reduceMotion: false, flows: false, healthy: true, library: true)
        XCTAssertEqual(library.interval, GlassLibraryStation.frameInterval)
        XCTAssertFalse(library.paused)

        // Unhealthy flows are drawn still: only the library turns.
        let unhealthy = FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: false, library: true)
        XCTAssertEqual(unhealthy.interval, GlassLibraryStation.frameInterval)
        XCTAssertFalse(unhealthy.paused)

        XCTAssertTrue(FlowMapView.restingMotion(reduceMotion: false, flows: false, healthy: true, library: false).paused)
        XCTAssertTrue(FlowMapView.restingMotion(reduceMotion: false, flows: true, healthy: false, library: false).paused)
        XCTAssertTrue(FlowMapView.restingMotion(reduceMotion: true, flows: true, healthy: true, library: true).paused)
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
