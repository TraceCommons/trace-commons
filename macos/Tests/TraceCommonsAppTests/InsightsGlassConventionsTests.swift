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

    /// Decline buttons are links, after the card's primary or glass button
    /// (Ron's #1287 review: action first).
    func test_declineButtonsAreLinksAfterTheAction() throws {
        let source = try Self.text("Views/InsightsOverviewTab.swift")
        let recap = try XCTUnwrap(source.range(of: #"Button(text("analytics_recap_open")) { openRecap() }"#))
        let recapStyle = try XCTUnwrap(source.range(of: ".buttonStyle(GlassButtonStyle(.primary, small: true))",
                                                    range: recap.upperBound ..< source.endIndex))
        let hide = try XCTUnwrap(source.range(of: #"Button(text("analytics_recap_turn_off")) { turnOffRecap() }"#
                                                + "\n                        .buttonStyle(GlassButtonStyle(.link))"))
        XCTAssertLessThan(recapStyle.lowerBound, hide.lowerBound, "Hide comes after View week")
    }

    /// Every chart is plotted on the chart surface, never straight on glass,
    /// and its series are the chart tokens, which clear 3:1 on that surface
    /// (`ChartSeriesContrastTests`): never a text ink, never a dimmed one
    /// (direction B, 2026-10-15; Ron's #1287 review asked for 3:1).
    func test_chartsPlotOnTheChartSurfaceInTheSeriesTokens() throws {
        let glass = try Self.text("Views/InsightsGlass.swift")
        XCTAssertTrue(glass.contains(".fill(GlassTokens.Color.chartSurface.color)"))
        for token in ["chartSeries1", "chartSeries2", "chartSeries3", "chartSeries4"] {
            XCTAssertTrue(glass.contains("GlassTokens.Color.\(token)"), "InsightsSeries has no \(token)")
        }
        // Tokens by day moved from Overview to its own view on Details,
        // and every usage card draws the same bars (owner, 2026-10-28).
        for file in ["Views/InsightsTokensByDay.swift", "Views/InsightsUsageCards.swift",
                     "Views/InsightsPatternsTab.swift", "Views/InsightsSessionsTab.swift"] {
            let source = try Self.text(file)
            XCTAssertTrue(source.contains(".insightsPlot()"), "\(file) plots straight on glass")
            XCTAssertFalse(source.contains("accentText.opacity("), "\(file) dims accentText for a series")
            XCTAssertFalse(source.contains("range: [GlassColor."), "\(file) draws a series in a text ink")
        }
    }

    /// A container inside a card's body (a chart's well, a quiet card, a
    /// row card) is not inset as far as the body's text (owner,
    /// 2026-10-19): the body marks itself, and each container gives the
    /// body's extra inset back, so its edges line up with the card's title.
    func test_inlineContainersAreNotInsetAsFarAsTheText() throws {
        let glass = try Self.text("Views/InsightsGlass.swift")
        XCTAssertTrue(glass.contains("""
                padding(.horizontal, InsightsLayout.bodyInset)
                    .environment(\\.insightsInCardBody, true)
        """), "the card body does not mark what is in it")
        XCTAssertTrue(glass.contains("content.padding(.horizontal, inBody ? -InsightsLayout.bodyInset : 0)"),
                      "a container in a body keeps the text's inset")
        for opening in ["func insightsPlot() -> some View {", "func insightsCard() -> some View {",
                        "func insightsRowCard() -> some View {"] {
            let start = try XCTUnwrap(glass.range(of: opening), opening)
            let end = try XCTUnwrap(glass.range(of: "\n    }\n", range: start.upperBound..<glass.endIndex))
            XCTAssertTrue(glass[start.upperBound..<end.lowerBound].contains(".modifier(InsightsInlineContainer())"),
                          "\(opening) is inset as far as the text")
        }
        // A quiet card resets the mark for what it holds, so a container
        // inside it is not pulled out twice.
        XCTAssertEqual(glass.components(separatedBy: "environment(\\.insightsInCardBody, false)").count - 1, 2)
    }

    /// The week picker has its row to itself, on the right (owner,
    /// 2026-10-19), on Patterns. Under it is one section (owner,
    /// 2026-10-20): the scope line ("Claude Code traces only"), over the
    /// coverage line. Overview's picker shares its row with the coverage
    /// line (product owner, 2026-10-29: the Sessions figure left Overview).
    func test_theWeekPickerHasItsRowAndTheSectionIsUnderIt() throws {
        let overview = try Self.text("Views/InsightsOverviewTab.swift")
        let start = try XCTUnwrap(overview.range(of: "private func weekPicker("))
        let row = String(overview[start.upperBound...].prefix(600))
        let coverageLine = try XCTUnwrap(row.range(of: "InsightsCoverageLine("))
        let picker = try XCTUnwrap(row.range(of: "InsightsWeekSelect("))
        XCTAssertLessThan(coverageLine.lowerBound, picker.lowerBound, "the picker is on the right")
        for (file, leading) in [("Views/InsightsPatternsTab.swift", "InsightsScopeLine(source: \"claude_code\", line: line)")] {
            let source = try Self.text(file)
            let start = try XCTUnwrap(source.range(of: "private func header("), file)
            let end = try XCTUnwrap(source.range(of: "\n    }\n", range: start.upperBound..<source.endIndex), file)
            let body = String(source[start.upperBound..<end.lowerBound])
            XCTAssertTrue(body.contains("HStack(spacing: 0) {\n                Spacer(minLength: 0)\n                InsightsWeekSelect("),
                          "\(file): the week picker has its row to itself, on the right")
            let picker = try XCTUnwrap(body.range(of: "InsightsWeekSelect("), file)
            let first = try XCTUnwrap(body.range(of: leading), file)
            let coverage = try XCTUnwrap(body.range(of: "InsightsCoverageLine("), file)
            let section = try XCTUnwrap(body.range(of: ".insightsSection()"), file)
            XCTAssertLessThan(picker.lowerBound, first.lowerBound, "\(file): the section is under the picker")
            XCTAssertLessThan(first.lowerBound, coverage.lowerBound, file)
            XCTAssertLessThan(coverage.lowerBound, section.lowerBound, "\(file): one section holds both")
        }
        XCTAssertFalse(try Self.text("Views/InsightsGlass.swift").contains("struct InsightsScopeCard"),
                       "the scope is a card of its own again")
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
