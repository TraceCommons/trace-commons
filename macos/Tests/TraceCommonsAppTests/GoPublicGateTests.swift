import XCTest

@testable import TraceCommonsApp

final class GoPublicGateTests: XCTestCase {
    func test_goPublicNeedsAnAcknowledgementAndAHandle() {
        XCTAssertFalse(GoPublicGate.canGoPublic(acknowledged: false, handle: "zaki", busy: false))
        XCTAssertFalse(GoPublicGate.canGoPublic(acknowledged: true, handle: "   ", busy: false))
        XCTAssertFalse(GoPublicGate.canGoPublic(acknowledged: true, handle: "zaki", busy: true))
        XCTAssertTrue(GoPublicGate.canGoPublic(acknowledged: true, handle: "zaki", busy: false))
    }
}
