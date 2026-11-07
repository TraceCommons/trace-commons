import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-11-07 ruling: Home's cards carry no background icons.
/// The large faded watermark each small card drew from 2026-10-20 (the
/// hourglass, the seal, the lock, the dollar sign and the card) is gone,
/// and the stat card no longer offers a watermark at all.
@MainActor
final class DesignRulings20261107Tests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    func test_homesCardsCarryNoBackgroundIcons() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertFalse(home.contains("glassCardWatermark"), "a Home card's watermark is back")
        XCTAssertFalse(home.contains("iconPlacement"), "a Home tile's icon is back")
        XCTAssertFalse(home.contains("HomeStatIcons"), "Home's tile icons are back")
        // Every Home tile is drawn without a symbol.
        let tile = try XCTUnwrap(home.range(of: "private struct HomeStatTile: View {"))
        let end = try XCTUnwrap(home.range(of: "\n}\n", range: tile.upperBound..<home.endIndex))
        XCTAssertFalse(home[tile.upperBound..<end.lowerBound].contains("systemImage"), "a Home tile has an icon again")
        let card = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/StatCard.swift"),
            encoding: .utf8)
        XCTAssertFalse(card.contains("GlassCardWatermark"), "the stat card's watermark is back")
        XCTAssertFalse(card.contains("IconPlacement"), "the stat card's watermark placement is back")
    }
}
