import SwiftUI
import TCBridge
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Balance tile's 24-hour change and its arrival flourish. Every figure
/// is synthetic.
@MainActor
final class BalanceArrivalHomeTests: XCTestCase {
    private func tile(_ json: String) throws -> (value: String, beside: String?, detail: String?, detailInk: GlassRGBA?) {
        let model = AppModel()
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        return HomeFormat.balance(BalanceStatus.parse(fromJSON: json), copy: copy, calls: model.balanceCalls)
    }

    /// Up in the status green, down in the status red, as the Credit tile's
    /// change is drawn; the words are the core's.
    func test_theChangeSitsUnderTheFigureUpOrDown() throws {
        let up = try tile("""
            {"state":"known","scale":9,"remaining_nanos":8200000000,
             "change_24h_nanos":3200000000,"change_24h_text":"+$3.20 in the last 24 hours"}
            """)
        XCTAssertEqual(up.value, "$8.20")
        XCTAssertEqual(up.detail, "+$3.20 in the last 24 hours")
        XCTAssertEqual(up.detailInk, GlassTokens.Color.statusOnText)
        let down = try tile("""
            {"state":"known","scale":9,"remaining_nanos":4000000000,
             "change_24h_nanos":-1000000000,"change_24h_text":"-$1.00 in the last 24 hours"}
            """)
        XCTAssertEqual(down.detail, "-$1.00 in the last 24 hours")
        XCTAssertEqual(down.detailInk, GlassTokens.Color.statusOutsideText)
    }

    /// No reading from a day ago: nothing under the figure, never "$0.00".
    func test_anUnknownChangeDrawsNothing() throws {
        let unknown = try tile("""
            {"state":"known","scale":9,"remaining_nanos":8200000000,
             "change_24h_nanos":null,"change_24h_text":null}
            """)
        XCTAssertEqual(unknown.value, "$8.20")
        XCTAssertNil(unknown.detail)
        XCTAssertNil(unknown.detailInk)
        // A line with no figure behind it is not drawn either.
        let orphan = try tile("""
            {"state":"known","scale":9,"remaining_nanos":1,"change_24h_text":"+$1.00 in the last 24 hours"}
            """)
        XCTAssertNil(orphan.detail)
    }

    /// The flourish is one self-contained view, marked as a placeholder, and
    /// it reads Reduce Motion.
    func test_theFlourishIsOneSwappablePlaceholderThatReadsReduceMotion() throws {
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(
                "Views/Monitor/BalanceArrivalFlourish.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains("Placeholder"))
        XCTAssertTrue(source.contains("accessibilityReduceMotion"))
        XCTAssertTrue(source.contains("BalanceArrival.effect("))
        let home = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(
                "Views/Monitor/HomeViews.swift"),
            encoding: .utf8)
        XCTAssertTrue(home.contains("BalanceArrivalFlourish("))
    }
}
