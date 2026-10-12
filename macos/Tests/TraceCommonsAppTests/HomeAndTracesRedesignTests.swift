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

    /// The tile's sign and tone: green up, red down, no tone for no change.
    func test_creditDeltaIsSignedAndToned() throws {
        let words = try XCTUnwrap(MonitorWords.table?.homeHistory)
        let up = try XCTUnwrap(HomeFormat.creditDelta(3, words: words))
        XCTAssertTrue(up.text.hasPrefix("+3"))
        XCTAssertEqual(up.ink, GlassTokens.Color.statusOnText)
        let down = try XCTUnwrap(HomeFormat.creditDelta(-2.5, words: words))
        XCTAssertTrue(down.text.contains("2.5"))
        XCTAssertEqual(down.ink, GlassTokens.Color.statusOutsideText)
        XCTAssertEqual(HomeFormat.creditDelta(0, words: words)?.text, words.creditDeltaNone)
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
}
