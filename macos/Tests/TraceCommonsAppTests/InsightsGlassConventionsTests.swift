import XCTest

/// The Insights screens, the menu-bar glance and the Inference token line
/// follow the flat glass theme's conventions (#1318): glass button styles,
/// never a stock one, and no literal white or black ink, which does not
/// follow the theme or the appearance.
final class InsightsGlassConventionsTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    static let files = [
        "Views/InsightsView.swift",
        "Views/InsightsOverviewTab.swift",
        "Views/InsightsPatternsTab.swift",
        "Views/InsightsSessionsTab.swift",
        "Views/Monitor/InsightsGlanceCard.swift",
        "Views/Monitor/InferenceViews.swift",
    ]

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    func test_noStockButtonStyles() throws {
        for file in Self.files {
            let source = try Self.text(file)
            for stock in [".buttonStyle(.link)", ".buttonStyle(.borderless)", ".buttonStyle(.bordered)",
                          ".buttonStyle(.borderedProminent)"] {
                XCTAssertFalse(source.contains(stock), "\(file) uses \(stock)")
            }
        }
    }

    func test_seeSessionsIsAGlassLink() throws {
        let source = try Self.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(source.contains(
            "if model.sessions?.pattern == card.kind { model.hideSessions() } else { model.showSessions(card.kind) }\n"
                + "                }\n"
                + "                .buttonStyle(GlassButtonStyle(.link))"))
    }

    /// A tab that cannot be used is hidden, not drawn greyed (Ron's #1287
    /// review): no Spend tab and no "Later" chip (owner decision D5, no
    /// money in Insights).
    func test_noSpendTabOrLaterChip() throws {
        for file in Self.files {
            let source = try Self.text(file)
            XCTAssertFalse(source.contains("analytics_tab_spend"), "\(file) draws the Spend tab")
            XCTAssertFalse(source.contains("analytics_later"), "\(file) draws the Later chip")
        }
    }

    func test_noLiteralWhiteOrBlackInk() throws {
        for file in Self.files {
            let source = try Self.text(file)
            for literal in ["(.white)", "(.black)", "Color.white", "Color.black", "Color.primary",
                            "Color.secondary", "Color(red:", "Color(white:", "Material"] {
                XCTAssertFalse(source.contains(literal), "\(file) contains \(literal)")
            }
        }
    }
}
