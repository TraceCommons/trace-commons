import XCTest
@testable import TCDesign

final class RefreshMotionTests: XCTestCase {
    @MainActor
    func testAnImmediateAnswerStillTurnsTheRefreshIconOnce() {
        let period = GlassTokens.Motion.spin
        XCTAssertEqual(GlassRefreshButton.angle(elapsed: 0, isRefreshing: false, reduceMotion: false), 0)
        XCTAssertEqual(GlassRefreshButton.angle(elapsed: period / 2, isRefreshing: false, reduceMotion: false), 180)
        XCTAssertEqual(GlassRefreshButton.angle(elapsed: period, isRefreshing: false, reduceMotion: false), 360)
        XCTAssertEqual(GlassRefreshButton.angle(elapsed: period * 3, isRefreshing: false, reduceMotion: false), 360)
        XCTAssertTrue(GlassRefreshButton.animates(isRefreshing: false, completingClick: true, reduceMotion: false))
        XCTAssertFalse(GlassRefreshButton.animates(isRefreshing: false, completingClick: false, reduceMotion: false))
    }

    @MainActor
    func testTheIconKeepsTurningUntilTheRequestEnds() {
        let period = GlassTokens.Motion.spin
        XCTAssertEqual(GlassRefreshButton.angle(elapsed: period * 2.5, isRefreshing: true, reduceMotion: false), 180,
                       accuracy: 0.0001)
        XCTAssertTrue(GlassRefreshButton.animates(isRefreshing: true, completingClick: false, reduceMotion: false))
    }

    @MainActor
    func testReduceMotionKeepsTheRefreshIconStill() {
        XCTAssertEqual(GlassRefreshButton.angle(elapsed: 0.3, isRefreshing: true, reduceMotion: true), 0)
        XCTAssertFalse(GlassRefreshButton.animates(isRefreshing: true, completingClick: true, reduceMotion: true))
    }
}
