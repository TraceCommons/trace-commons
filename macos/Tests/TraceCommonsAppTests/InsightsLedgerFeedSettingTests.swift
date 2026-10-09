import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Insights ledger feed (`insights_ledger_feed`) is on by default
/// (owner ruling, 2026-10-09), so the Inference tab no longer draws a switch
/// for it; Settings' Tools section offers the one way to turn it off. The
/// switch reads the daemon's value, moves only on a write the daemon
/// confirmed, and is not drawn for a daemon that does not report it.
@MainActor
final class InsightsLedgerFeedSettingTests: XCTestCase {
    /// `get_settings` and `set_settings` both answer the settings object.
    static func frame(_ feed: Bool?) -> String {
        let key = feed.map { "\"insights_ledger_feed\":\($0)," } ?? ""
        return """
        {"id":1,"result":{\(key)"quiescence_secs":45,"digest_interval_secs":3600,
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
        "max_uploads_per_day":100,"near_ai_configured":false,
        "claude_root_configured":true,"codex_root_configured":true}}
        """
    }

    static let refused = #"{"id":1,"error":{"code":"unavailable","message":"settings-save-failed"}}"#

    static func settings(_ feed: Bool?) throws -> DaemonSettingsView {
        try DaemonClient(daemon: LedgerFeedDaemon(write: frame(feed), read: frame(feed))).settings()
    }

    // MARK: Decoding

    func test_theSettingsViewReadsTheFeedAndAbsenceIsNotOff() throws {
        XCTAssertEqual(try Self.settings(true).insightsLedgerFeed, true)
        XCTAssertEqual(try Self.settings(false).insightsLedgerFeed, false)
        XCTAssertNil(try Self.settings(nil).insightsLedgerFeed, "an older daemon's silence read as an answer")
    }

    // MARK: The model's write

    func test_aConfirmedWriteMovesTheSwitchAndSendsOnlyTheKey() async throws {
        let daemon = LedgerFeedDaemon(write: Self.frame(false), read: Self.frame(true))
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(true))
        model.setClientForTesting(DaemonClient(daemon: daemon))
        await model.setInsightsLedgerFeed(false)
        XCTAssertEqual(model.daemonSettings?.insightsLedgerFeed, false)
        XCTAssertNil(model.insightsLedgerFeedRefusal)
        XCTAssertFalse(model.insightsLedgerFeedBusy)
        XCTAssertEqual(daemon.calls.first?.method, "set_settings")
        XCTAssertEqual(daemon.calls.first?.params, #"{"insights_ledger_feed":false}"#)
    }

    /// The daemon answered with its own value: that is where the switch
    /// stands, and the request-failed words say it was not what was asked.
    func test_anEchoOfTheOtherValueIsARefusal() async throws {
        let daemon = LedgerFeedDaemon(write: Self.frame(true), read: Self.frame(true))
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(true))
        model.setClientForTesting(DaemonClient(daemon: daemon))
        await model.setInsightsLedgerFeed(false)
        XCTAssertEqual(model.daemonSettings?.insightsLedgerFeed, true)
        XCTAssertEqual(model.insightsLedgerFeedRefusal, MonitorWords.table?.requestFailed)
        XCTAssertNotNil(model.insightsLedgerFeedRefusal)
        XCTAssertFalse(model.insightsLedgerFeedBusy)
    }

    /// A reply with no value confirms nothing.
    func test_anAnswerWithoutTheKeyIsARefusal() async throws {
        let daemon = LedgerFeedDaemon(write: Self.frame(nil), read: Self.frame(true))
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(true))
        model.setClientForTesting(DaemonClient(daemon: daemon))
        await model.setInsightsLedgerFeed(false)
        XCTAssertEqual(model.insightsLedgerFeedRefusal, MonitorWords.table?.requestFailed)
        XCTAssertNotEqual(model.daemonSettings?.insightsLedgerFeed, false)
    }

    /// A failed write reads back where the daemon stands and never draws
    /// what was asked; a failed read-back keeps the last confirmed value.
    func test_aFailedWriteKeepsTheDaemonsValueAndSaysSo() async throws {
        for read in [Self.frame(true), Self.refused] {
            let daemon = LedgerFeedDaemon(write: Self.refused, read: read)
            let model = AppModel()
            model.setDaemonSettingsForTesting(try Self.settings(true))
            model.setClientForTesting(DaemonClient(daemon: daemon))
            await model.setInsightsLedgerFeed(false)
            XCTAssertEqual(model.daemonSettings?.insightsLedgerFeed, true)
            XCTAssertEqual(model.insightsLedgerFeedRefusal, MonitorWords.table?.requestFailed)
            XCTAssertNotNil(model.insightsLedgerFeedRefusal)
            XCTAssertFalse(model.insightsLedgerFeedBusy)
        }
    }

    /// The next confirmed write clears the refusal.
    func test_aConfirmedWriteClearsAnEarlierRefusal() async throws {
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(true))
        model.setClientForTesting(DaemonClient(daemon: LedgerFeedDaemon(write: Self.refused, read: Self.refused)))
        await model.setInsightsLedgerFeed(false)
        XCTAssertNotNil(model.insightsLedgerFeedRefusal)
        model.setClientForTesting(DaemonClient(daemon: LedgerFeedDaemon(write: Self.frame(false), read: Self.frame(false))))
        await model.setInsightsLedgerFeed(false)
        XCTAssertNil(model.insightsLedgerFeedRefusal)
        XCTAssertEqual(model.daemonSettings?.insightsLedgerFeed, false)
    }

    // MARK: Source rules

    /// The switch is in Settings' Tools section, beside the routing it
    /// counts, drawn only from the daemon's own value, labelled in the
    /// core's words, and takes no second write while one is in flight.
    func test_theToolsSectionDrawsTheSwitchFromTheDaemonsValue() throws {
        let tools = try SettingsParityTests.text("Views/Settings/ToolsSection.swift")
        XCTAssertTrue(tools.contains("if let feed = settings.insightsLedgerFeed {"))
        XCTAssertTrue(tools.contains(#"InsightsOverviewWords.text("analytics_setting_ledger_feed", Self.insightsCopy)"#))
        XCTAssertTrue(tools.contains(#"InsightsOverviewWords.text("analytics_feed_ledger", Self.insightsCopy)"#))
        XCTAssertTrue(tools.contains("Task { await model.setInsightsLedgerFeed(on) }"))
        XCTAssertTrue(tools.contains(".disabled(model.insightsLedgerFeedBusy)"))
        XCTAssertTrue(tools.contains("if let refusal = model.insightsLedgerFeedRefusal {"))
        XCTAssertTrue(tools.contains("private static let insightsCopy = TCInsights.copy() ?? [:]"))
    }

    /// The Inference tab draws the tokens and no switch for them; nothing
    /// there reads or writes the setting any more.
    func test_theInferenceTabHasNoLedgerFeedSwitch() throws {
        let views = try MonitorNavigationTests.text("Views/Monitor/InferenceViews.swift")
        let store = try MonitorNavigationTests.text("Views/Monitor/InferenceStore.swift")
        XCTAssertFalse(views.contains("InsightsLedgerFeedSwitch"))
        XCTAssertFalse(views.contains("analytics_setting_ledger_feed"))
        for needle in ["ledgerFeed", "setInsightsLedgerFeed(", "insightsLedgerFeed"] {
            XCTAssertFalse(views.contains(needle), "InferenceViews still holds \(needle)")
            XCTAssertFalse(store.contains(needle), "InferenceStore still holds \(needle)")
        }
        // The token line still reads the core's analytics words.
        XCTAssertTrue(views.contains("private static let insightsCopy = TCInsights.copy() ?? [:]"))
    }

    /// Tools is a write section, so it waits for first run (R-43).
    func test_theSwitchIsNotAWriteSurfaceBeforeOnboarding() {
        XCTAssertFalse(SettingsSection.tools.availableBeforeOnboarding)
    }
}

/// Answers `set_settings` with `write` and every other call with `read`,
/// and records what was sent. Locked: the model's write runs on a detached
/// task while the test reads `calls`.
private final class LedgerFeedDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []
    let write: String
    let read: String

    init(write: String, read: String) {
        self.write = write
        self.read = read
    }

    var calls: [(method: String, params: String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        recorded.append((method: method, params: paramsJSON))
        lock.unlock()
        return method == "set_settings" ? write : read
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
