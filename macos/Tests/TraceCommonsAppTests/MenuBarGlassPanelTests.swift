import TCBridge
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
        XCTAssertEqual(MenuPanelData.rollup("notify_only"), .ask)
        XCTAssertEqual(MenuPanelData.rollup("auto_upload"), .armed)
        XCTAssertEqual(MenuPanelData.rollup("ignore"), .never)
        XCTAssertEqual(MenuPanelData.rollup("mixed"), .mixed)
        XCTAssertEqual(MenuPanelData.rollup("a-mode-from-a-later-daemon"), .none)
        XCTAssertEqual(MenuPanelData.rollup(nil), .none)
    }

    /// The pill reads the daemon's roll-up from `status` (#1208), in the
    /// core's words, with the partial line only when the status says so.
    func test_thePillReadsTheCoresContributionMode() async throws {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let status = try XCTUnwrap(store.status)
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        let rollup = MenuPanelData.rollup(status.contributionMode)
        XCTAssertNotEqual(rollup, .none, "the recorded status carries contribution_mode")
        XCTAssertEqual(MenuPanelData.modeValue(rollup, mode: status.contributionMode, copy: copy),
                       rollup == .mixed ? copy.mixed : copy.choice(for: status.contributionMode)?.label)
        XCTAssertEqual(MenuPanelData.modeValue(.none, mode: nil, copy: copy), "—")
        XCTAssertEqual(MenuPanelData.modeValue(.ask, mode: "notify_only", copy: nil), "—")
        XCTAssertEqual(MenuPanelData.partialLine("auto_upload", status: status, copy: copy),
                       status.contributionModePartial == true ? copy.autoPartial : nil)
        XCTAssertNil(MenuPanelData.partialLine("notify_only", status: status, copy: copy))
        // The panel never works the roll-up out from the folders.
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertFalse(source.contains("projects.map(\\.mode)"))
        XCTAssertTrue(source.contains("store.status?.contributionMode"))
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
    }

    /// An outside call carries its proof label unless it was verified; a
    /// routed call is not recent activity.
    func test_anOutsideCallCarriesItsProofLabel() throws {
        let decode = { (json: String) in
            try DaemonDataDecoding.decoder().decode(DaemonData.InferenceCall.self, from: Data(json.utf8))
        }
        let unproven = try decode(#"{"id":1,"at":"2026-09-30T09:00:00Z","tool":"codex","family":"openai","model":"m","route":"outside","cost":null,"proof":"unattested"}"#)
        let verified = try decode(#"{"id":2,"at":"2026-09-30T08:00:00Z","tool":"codex","family":"openai","model":"m","route":"outside","cost":null,"proof":"verified"}"#)
        let routed = try decode(#"{"id":3,"at":"2026-09-30T10:00:00Z","tool":"codex","family":"openai","model":"m","route":"routed","cost":null,"proof":"unattested"}"#)
        let rows = MenuPanelData.recent(pending: [], history: [], calls: [unproven, verified, routed], statusLabel: { _ in nil })
        XCTAssertEqual(rows.map(\.id), ["call:1", "call:2"])
        XCTAssertEqual(rows.first?.trailing, InferenceWords.proof(.unattested))
        XCTAssertNil(rows.last?.trailing)
    }

    // MARK: Live data and the strip

    /// With no client, after a failed read, or once the event stream ends,
    /// the store's data is stale; a full read clears it.
    func test_theStoreMarksItsDataStale() async {
        let none = MenuPanelStore(client: nil)
        await none.load()
        XCTAssertTrue(none.stale)

        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        XCTAssertTrue(store.stale, "nothing read yet")
        await store.load()
        XCTAssertFalse(store.stale)
        store.attach(SampleDaemonClient(.coreDown))
        XCTAssertTrue(store.stale)
        await store.load()
        XCTAssertTrue(store.stale, "a failed read is stale, never the last values as current")
    }

    /// The strip is never live unless the daemon runs, the data is current
    /// and the core is healthy with a known count.
    func test_theStripIsDownWhenTheCoreIs() {
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 2, unhealthy: false, paused: false, available: false, stale: false), .unavailable)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 2, unhealthy: false, paused: false, available: true, stale: true), .unavailable)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 2, unhealthy: true, paused: false, available: true, stale: false), .attention)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: nil, unhealthy: false, paused: false, available: true, stale: false), .attention)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 0, unhealthy: false, paused: true, available: true, stale: false), .paused)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 0, unhealthy: false, paused: false, available: true, stale: false), .live)
    }

    /// The popover's store is the app's live client, never sample data,
    /// outside tests and previews.
    func test_thePanelUsesTheLiveClient() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
        let main = try String(contentsOf: root.appendingPathComponent("TraceCommonsAppMain.swift"), encoding: .utf8)
        XCTAssertTrue(main.contains("MenuPanelStore(client: nil)"))
        XCTAssertFalse(main.contains("MenuPanelStore(client: MonitorWindowView.dataClient())"))
        let panel = try String(contentsOf: root.appendingPathComponent("Views/Monitor/MenuBarGlassPanel.swift"), encoding: .utf8)
        XCTAssertTrue(panel.contains("store.attach(model.daemonData)"))
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

    /// The popover's words come from the core's table, not from Swift.
    func test_theWordsComeFromTheCore() {
        XCTAssertNotNil(MonitorWords.table)
        XCTAssertEqual(MenuWords.on, MonitorWords.table?.on)
        XCTAssertEqual(MenuBarGlassPanel.modeCaption, MonitorWords.table?.contributionMode)
        XCTAssertFalse(MenuWords.quit.isEmpty)
    }
}
