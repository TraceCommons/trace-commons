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

    func testKnownZeroChangeIsVisibleWithoutGainOrLossColor() throws {
        let zero = try tile("""
            {"state":"known","scale":9,"remaining_nanos":8200000000,
             "change_24h_nanos":0,"change_24h_text":"$0.00 in the last 24 hours"}
            """)
        XCTAssertEqual(zero.detail, "$0.00 in the last 24 hours")
        XCTAssertNil(zero.detailInk)
    }

    func testSubcentChangeUsesTheCoreFormattingPrecisionAndStaysNeutral() throws {
        for nanos in [-9_000_000, 9_000_000] {
            let tiny = try tile("""
                {"state":"known","scale":9,"remaining_nanos":8200000000,
                 "change_24h_nanos":\(nanos),"change_24h_direction":0,"change_24h_text":"$0.00 in the last 24 hours"}
                """)
            XCTAssertEqual(tiny.detail, "$0.00 in the last 24 hours")
            XCTAssertNil(tiny.detailInk)
        }
        let alternateScale = try tile("""
            {"state":"known","scale":3,"remaining_nanos":8200,
             "change_24h_nanos":20,"change_24h_direction":1,"change_24h_text":"+$0.02 in the last 24 hours"}
            """)
        XCTAssertEqual(alternateScale.detailInk, GlassTokens.Color.statusOnText)
    }

    func testUnavailableHistoryDrawsNothingWithoutInventingZero() throws {
        let status = BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"remaining_nanos":8200000000,
             "change_24h_nanos":null,"change_24h_text":null,
             "change_24h_unavailable_text":"24-hour balance change unavailable"}
            """)
        let model = AppModel()
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let rendered = HomeFormat.balance(status, copy: copy, calls: model.balanceCalls)
        XCTAssertNil(rendered.detail)
        XCTAssertNil(rendered.detailInk)
        XCTAssertNil(status.change24hNanos)
    }

    /// The flourish is one self-contained view: gold coins burst from the
    /// tile (owner, 2026-10-27, in place of the green edge), and it reads
    /// Reduce Motion.
    func test_theFlourishBurstsCoinsAndReadsReduceMotion() throws {
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(
                "Views/Monitor/BalanceArrivalFlourish.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains(".glassCoinBurst(start: burst, still: reduceMotion, trailingRoom: Self.edgeRoom)"))
        XCTAssertFalse(source.contains("strokeBorder"), "the lit edge is back")
        XCTAssertFalse(source.contains("statusOnText"), "the green edge is back")
        XCTAssertTrue(source.contains("accessibilityReduceMotion"))
        XCTAssertTrue(source.contains("BalanceArrival.effect("))
        let home = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(
                "Views/Monitor/HomeViews.swift"),
            encoding: .utf8)
        XCTAssertTrue(home.contains("BalanceArrivalFlourish("))
    }

    /// A balance refresh mid-count neither cancels the count nor the coins:
    /// the count has a task of its own, keyed on the count and not on the
    /// balance, and the coins clear on a `.task(id: burst)`, not on a task
    /// the check begins.
    func test_aRefreshMidCountLeavesTheCountAndCoinsToFinish() throws {
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(
                "Views/Monitor/BalanceArrivalFlourish.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains(".task(id: count) {"))
        XCTAssertTrue(source.contains(".task(id: burst) {"))
        XCTAssertTrue(source.contains("count = Count(from: from, to: to)"))
        XCTAssertFalse(source.contains("Task {"), "no unstructured task")
        XCTAssertFalse(source.contains("burst == mine"), "no Date check by hand")
        // The check task starts the count and returns; it does not run it.
        let check = try XCTUnwrap(source.range(of: "private func check() {"))
        XCTAssertFalse(source[check.upperBound...].contains("Task.sleep"))
        XCTAssertEqual(BalanceArrivalFlourish<EmptyView>.coinsStay(reduceMotion: true), .milliseconds(1600))
        XCTAssertEqual(BalanceArrivalFlourish<EmptyView>.coinsStay(reduceMotion: false),
                       .seconds(GlassCoinBurst.duration))
    }
}
