import Foundation
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-10-12 round: Traces search, the Credit tile's
/// day-over-day change, and the Uncontributed tile.
@MainActor
final class HomeAndTracesRedesignTests: XCTestCase {
    private func entry(_ id: Int, _ project: String, title: String? = nil, extra: [String: Any] = [:]) throws
        -> DaemonData.QueueEntry
    {
        var fields: [String: Any] = [
            "entry_id": String(format: "00000000-0000-4000-8000-%012d", id),
            "project_id": project, "project_label": project,
            "title": title.map { $0 as Any } ?? NSNull(),
        ]
        fields.merge(extra) { _, new in new }
        return try recordedEntry(fields)
    }

    /// A folder whose name matches keeps every session; otherwise only the
    /// matching sessions stay, and a folder left with none is dropped.
    func test_searchNarrowsTheTreeToWhatMatches() throws {
        let entries = [
            try entry(1, "payments-api", title: "Fix refund rounding"),
            try entry(2, "payments-api", title: "Add webhook retries"),
            try entry(3, "dotfiles", title: "Tidy zsh prompt"),
            try entry(4, "dotfiles", title: "Refund nothing"),
        ]
        let tree = TracesTree.build(entries: entries, projects: [], settings: nil, scansWhenUnset: [])

        XCTAssertEqual(tree.matching("   "), tree, "a blank search is the whole tree")

        let byFolder = tree.matching("PAYMENTS")
        XCTAssertEqual(byFolder.folders.map(\.id), ["payments-api"])
        XCTAssertEqual(byFolder.folders.first?.sessions.count, 2)

        let byTitle = tree.matching("refund")
        XCTAssertEqual(Set(byTitle.folders.map(\.id)), ["payments-api", "dotfiles"])
        XCTAssertEqual(byTitle.folders.flatMap(\.sessions).compactMap(\.title).sorted(),
                       ["Fix refund rounding", "Refund nothing"])

        XCTAssertTrue(tree.matching("nothing-like-this").folders.isEmpty)
    }

    /// The change is against the newest earlier day this Mac saw; with no
    /// earlier day it is unknown, never zero; old days are let go.
    func test_creditTotalsGiveTheChangeSinceTheLastDaySeen() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "UTC"))
        func day(_ text: String) throws -> Date { try XCTUnwrap(ISO8601DateFormatter().date(from: text + "T12:00:00Z")) }

        var totals = CreditDailyTotals()
        totals.record(10, on: try day("2026-10-09"), calendar: calendar)
        XCTAssertNil(totals.delta(on: try day("2026-10-09"), calendar: calendar), "no earlier day")

        // Two days later (none seen between): against the 9th.
        totals.record(14.5, on: try day("2026-10-11"), calendar: calendar)
        XCTAssertEqual(totals.delta(on: try day("2026-10-11"), calendar: calendar), 4.5)
        // A later read the same day replaces that day's total.
        totals.record(9, on: try day("2026-10-11"), calendar: calendar)
        XCTAssertEqual(totals.delta(on: try day("2026-10-11"), calendar: calendar), -1)
        XCTAssertNil(totals.delta(on: try day("2026-10-12"), calendar: calendar), "today not read yet")

        for offset in 0 ..< 40 {
            totals.record(Double(offset), on: try day("2026-10-12").addingTimeInterval(Double(offset) * 86_400),
                          calendar: calendar)
        }
        XCTAssertEqual(totals.totals.count, CreditDailyTotals.keptDays)
    }

    /// The tile's sign and tone: green up, red down, and no line at all for
    /// no change (owner, 2026-10-16).
    func test_creditDeltaIsSignedAndToned() throws {
        let words = try XCTUnwrap(MonitorWords.table?.homeHistory)
        let up = try XCTUnwrap(HomeFormat.creditDelta(3, words: words))
        XCTAssertTrue(up.text.hasPrefix("+3"))
        XCTAssertEqual(up.ink, GlassTokens.Color.statusOnText)
        let down = try XCTUnwrap(HomeFormat.creditDelta(-2.5, words: words))
        XCTAssertTrue(down.text.contains("2.5"))
        XCTAssertEqual(down.ink, GlassTokens.Color.statusOutsideText)
        XCTAssertNil(HomeFormat.creditDelta(0, words: words))
        XCTAssertNil(HomeFormat.creditDelta(0.04, words: words))
        XCTAssertNil(HomeFormat.creditDelta(nil, words: words))
        XCTAssertEqual(HomeFormat.creditTotal(nil), "—")
    }

    /// Uncontributed counts what has not left this Mac, and adds up only
    /// the estimates the core says to draw.
    func test_uncontributedCountsWhatWaitsAndItsDrawnEstimate() throws {
        let estimate: (Double, Double, Bool) -> [String: Any] = { low, high, drawn in
            ["credit_estimate": ["low": low, "high": high, "calibration": "uncalibrated", "basis": "published",
                                 "drawn": drawn]]
        }
        var a = estimate(1, 3, true)
        a["size_bytes"] = 1_000
        a["state"] = "pending"
        var b = estimate(10, 20, false)
        b["size_bytes"] = 2_000
        b["state"] = "pending"
        var gone = estimate(5, 5, true)
        gone["state"] = "uploaded"
        gone["size_bytes"] = 9_000
        let waiting = HomeUncontributed([
            try entry(1, "x", extra: a), try entry(2, "x", extra: b), try entry(3, "x", extra: gone),
        ])
        XCTAssertEqual(waiting.count, 2)
        XCTAssertEqual(waiting.bytes, 3_000)
        XCTAssertEqual(waiting.low, 1)
        XCTAssertEqual(waiting.high, 3)

        let none = HomeUncontributed([try entry(4, "x", extra: estimate(1, 2, false))])
        XCTAssertNil(none.low, "no drawn estimate is no estimate, never zero")
    }

    /// Dollars replace points on the tile (owner, 2026-10-16): the core's
    /// finished machine-wide line, drawn as given, the same one the menu
    /// bar draws.
    func test_uncontributedTileDrawsTheDollarLineInPlaceOfPoints() throws {
        guard let words = MonitorWords.table?.homeHistory else {
            throw XCTSkip("core copy unavailable")
        }
        var a: [String: Any] = ["credit_estimate": ["low": 1, "high": 3, "calibration": "uncalibrated",
                                                    "basis": "published", "drawn": true]]
        a["state"] = "pending"
        let line = "About $9 to $26 of inference in traces on this Mac"
        let dollars = HomeUncontributed([try entry(1, "x", extra: a)], settlement: try settlement(line))
        XCTAssertEqual(dollars.detail(words), line)

        let points = HomeUncontributed([try entry(1, "x", extra: a)], settlement: nil)
        XCTAssertEqual(points.detail(words), HomeUncontributed([try entry(1, "x", extra: a)]).detail(words),
                       "without a dollar estimate the points line stays")
        XCTAssertNotEqual(points.detail(words), line)

        let blank = HomeUncontributed([try entry(1, "x", extra: a)], settlement: try settlement("  "))
        XCTAssertEqual(blank.detail(words), points.detail(words), "an empty line is no line")

        let nothingQueued = HomeUncontributed([], settlement: try settlement(line))
        XCTAssertEqual(nothingQueued.detail(words), line,
                       "the dollar line covers traces on this Mac, queued or not")
        XCTAssertNil(HomeUncontributed([], settlement: nil).detail(words))
    }

    private func settlement(_ text: String) throws -> DaemonData.SettlementEstimate {
        let json: [String: Any] = [
            "low_usd_micros": 9_000_000, "high_usd_micros": 26_000_000, "up_to": false,
            "known": 127, "unknown": 4, "table_version": "s1", "estimated_at": "2026-10-16T16:00:00Z",
            "text": text,
        ]
        return try JSONDecoder().decode(DaemonData.SettlementEstimate.self,
                                        from: JSONSerialization.data(withJSONObject: json))
    }
}

/// The opening prompt, split into what was typed and what a tool wrapped
/// in a tag (owner, 2026-10-12).
final class PromptQuoteTests: XCTestCase {
    @MainActor
    func test_typedTextAndTaggedBlocksAreSplit() {
        let prompt = #"<pasted_content id="2cfe">Use the MCP to import</pasted_content> then rebuild"#
        XCTAssertEqual(PromptQuote.parts(prompt), [
            .tagged(name: "pasted_content", body: "Use the MCP to import"),
            .typed("then rebuild"),
        ])
        XCTAssertEqual(PromptQuote.parts("rebuild and launch"), [.typed("rebuild and launch")])
        // A tag left open runs to the end, as a prompt cut short leaves it.
        XCTAssertEqual(PromptQuote.parts("look <pasted_content id=\"x\">half a paste"), [
            .typed("look"), .tagged(name: "pasted_content", body: "half a paste"),
        ])
        XCTAssertEqual(PromptQuote.words("pasted_content"), "Pasted content")
        XCTAssertEqual(PromptQuote.icon("pasted_content"), "doc.on.clipboard")
    }
}

/// The map's node names are cut to 14 characters, the last an ellipsis
/// (owner, 2026-10-12).
final class FlowMapLabelTests: XCTestCase {
    @MainActor
    func test_namesAreCutToFourteenCharacters() {
        XCTAssertEqual(FlowMapView.shortLabel("build-run-latest-design"), "build-run-lat\u{2026}")
        XCTAssertEqual(FlowMapView.shortLabel("build-run-latest-design").count, 14)
        XCTAssertEqual(FlowMapView.shortLabel("trace-commons"), "trace-commons")
        XCTAssertEqual(FlowMapView.shortLabel("fourteen-chars"), "fourteen-chars")
    }
}
