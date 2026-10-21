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
        "Views/InsightsModelsCard.swift",
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

        let reads = try XCTUnwrap(source.range(of: #"Button(text("analytics_lever_show_reads")) { showReads() }"#
                                                 + "\n                            .buttonStyle(GlassButtonStyle(.glass, small: true))"))
        let notUseful = try XCTUnwrap(source.range(of: #"Button(text("analytics_lever_not_useful")) { comparisons.notUseful() }"#
                                                     + "\n                        .buttonStyle(GlassButtonStyle(.link))"))
        XCTAssertLessThan(reads.lowerBound, notUseful.lowerBound, "Not useful comes after Show reads")
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
        for file in ["Views/InsightsOverviewTab.swift", "Views/InsightsPatternsTab.swift",
                     "Views/InsightsSessionsTab.swift"] {
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

    /// The week picker sits on the right of its row on Overview and
    /// Patterns, and Patterns' scope line ("Claude Code traces only") is a
    /// card of its own on the left (owner, 2026-10-19).
    func test_theWeekPickerIsOnTheRightAndTheScopeIsACard() throws {
        for (file, leading) in [("Views/InsightsPatternsTab.swift", "InsightsScopeCard(source: \"claude_code\", line: line)"),
                                ("Views/InsightsOverviewTab.swift", "drillButton(\"sessions\") {")] {
            let source = try Self.text(file)
            let header = try XCTUnwrap(source.range(of: "private func header("), file)
            let picker = try XCTUnwrap(source.range(of: "InsightsWeekSelect(", range: header.upperBound..<source.endIndex))
            let first = try XCTUnwrap(source.range(of: leading, range: header.upperBound..<source.endIndex))
            XCTAssertLessThan(first.lowerBound, picker.lowerBound, "\(file): the week picker is on the left")
        }
        let glass = try Self.text("Views/InsightsGlass.swift")
        let card = try XCTUnwrap(glass.range(of: "struct InsightsScopeCard: View {"))
        XCTAssertTrue(glass[card.upperBound...].prefix(600).contains("GlassCard(quiet: true, flush: true) {"))
    }

    /// The Models card (spec 2026-10-20 section 8) is drawn only from
    /// answers the daemon gave, so an older daemon or counting off hides it,
    /// never greys it; each row is one VoiceOver element with the row's
    /// whole sentence, and its "i" stays a control of its own.
    func test_theModelsCardIsHiddenNotGreyedAndEachRowIsOneElement() throws {
        let patterns = try Self.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(patterns.contains("if let answers = models?.answers {\n"
            + "                    InsightsModelsCard(answers: answers, copy: copy)"))
        let card = try Self.text("Views/InsightsModelsCard.swift")
        let row = try XCTUnwrap(card.range(of: "struct InsightsModelsRow: View {"))
        let rowBody = card[row.upperBound...]
        XCTAssertTrue(rowBody.contains(".accessibilityElement(children: .ignore)\n"
            + "            .accessibilityLabel(InsightsModelsWords.accessibilityLabel(row, copy: copy))"))
        XCTAssertTrue(rowBody.contains("InsightsInfo(InsightsModelsWords.title(row, copy: copy),"))
        for greyed in [".disabled(", ".opacity(", "textTertiary"] {
            XCTAssertFalse(card.contains(greyed), "the Models card draws \(greyed)")
        }
        let view = try Self.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("{ try await client.insightsModelAnswers() }"))
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
