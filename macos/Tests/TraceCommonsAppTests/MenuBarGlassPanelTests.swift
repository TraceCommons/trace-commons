import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R13 of #1173: the menu-bar popover from the handoff, under its rules.
@MainActor
final class MenuBarGlassPanelTests: XCTestCase {
    // MARK: Mode roll-up

    /// One mode for every folder reads as that mode; any difference is
    /// Mixed; no folders is unknown.
    func test_theModePillRollsUpTheFolders() {
        XCTAssertEqual(MenuPanelData.rollup([.ask, .ask]), .ask)
        XCTAssertEqual(MenuPanelData.rollup([.autoUpload]), .armed)
        XCTAssertEqual(MenuPanelData.rollup([.ignore, .ignore]), .never)
        XCTAssertEqual(MenuPanelData.rollup([.ask, .autoUpload]), .mixed)
        XCTAssertEqual(MenuPanelData.rollup([.ask, .ignore, .autoUpload]), .mixed)
        XCTAssertEqual(MenuPanelData.rollup([]), .none)
    }

    // MARK: Day graph

    /// Contributions go up and kept sessions go down, on the day they
    /// happened; the newest day is the last column; older days fall off.
    func test_theGraphCountsPerDayEndingToday() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        let day: TimeInterval = 86_400
        let columns = MenuPanelData.days(
            shared: [now, now - 60, now - day, now - 40 * day],
            kept: [now - 2 * day], ending: now, count: 36, calendar: calendar)
        XCTAssertEqual(columns.count, 36)
        XCTAssertEqual(columns.last?.up, 2)
        XCTAssertEqual(columns[34].up, 1)
        XCTAssertEqual(columns[33].down, 1)
        XCTAssertEqual(columns.reduce(0) { $0 + $1.up }, 3, "a day outside the window is not drawn")
    }

    // MARK: Flagged and recent

    /// Flagged counts the queue's attention reasons only.
    func test_flaggedCountsNothingMatchedAndTrimmed() async throws {
        // A real sample entry, with only its second-look reasons varied.
        let pending = try await SampleDaemonClient(.normalDay).listPending(projectId: nil)
        let sample = try XCTUnwrap(pending.first)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        func entry(_ id: String, _ reasons: [String]) throws -> DaemonData.QueueEntry {
            var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoder.encode(sample)) as? [String: Any])
            object["entry_id"] = id
            object["second_look"] = reasons
            return try decoder.decode(DaemonData.QueueEntry.self, from: JSONSerialization.data(withJSONObject: object))
        }
        let entries = try [
            entry("a", ["nothing-matched"]), entry("b", ["trimmed-to-fit"]),
            entry("c", ["looks-unsure"]), entry("d", []),
        ]
        XCTAssertEqual(MenuPanelData.flagged(entries), 2)
    }

    /// Recent activity is newest first, at most three, and an outside call
    /// carries its proof label unless it was verified.
    func test_recentActivityIsNewestFirst() async throws {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let rows = MenuPanelData.recent(
            pending: store.pending, history: store.history, calls: store.calls, statusLabel: { _ in nil })
        XCTAssertLessThanOrEqual(rows.count, 3)
        XCTAssertEqual(rows.map(\.at), rows.map(\.at).sorted(by: >))
        for row in rows where row.kind == .call {
            XCTAssertNotNil(row.trailing)
        }
    }

    // MARK: Badge

    func test_theBadgeCountsDecisionsOwedOnly() {
        XCTAssertEqual(MenuPanelStatus.badge(9), 9)
        XCTAssertNil(MenuPanelStatus.badge(0))
        XCTAssertNil(MenuPanelStatus.badge(nil))
    }

    // MARK: Rules

    /// Nothing is sent from the popover, and nothing is armed or turned on
    /// from it: its only writes are the shipping menu's pause, resume and
    /// Private AI off; the mode overrides are disabled.
    func test_thePopoverMakesOnlyTheShippingMenusWrites() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // No credit figure of any kind: no projected or pending credit.
        for forbidden in ["setProjectMode", "approve(", "applyPrivateInference(true)", "setPrivateAI(",
                          "creditPoints", "creditPending", "creditFinal", "creditRange", "commonsCreditSummary"] {
            XCTAssertFalse(source.contains(forbidden), "the popover contains \(forbidden)")
        }
        XCTAssertTrue(source.contains("modeOptions"))
        XCTAssertTrue(source.contains(".disabled(true)"), "the mode overrides must stay disabled until the core has them")
    }
}
