import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Insights switches in Settings (Ron's #1287 review, adopted by the
/// owner): "Count tokens in watched folders" (`insights_counter_pass`, on
/// by default and always on since 2026-10-29, so Settings no longer draws
/// it) and "Weekly summary" (`insights_recap_card_enabled`) are daemon
/// settings; the "Higher-than-usual card" switch is gone (product owner,
/// 2026-10-29, #1753). Each reads its confirmed value, moves only on a
/// confirmed echo, and says the core's request-failed words when a write
/// is refused.
@MainActor
final class InsightsSettingsSwitchesTests: XCTestCase {
    static func frame(_ values: [String: Bool]) -> String {
        let keys = values.sorted { $0.key < $1.key }.map { "\"\($0.key)\":\($0.value)," }.joined()
        return """
        {"id":1,"result":{\(keys)"quiescence_secs":45,"digest_interval_secs":3600,
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
        "max_uploads_per_day":100,"near_ai_configured":false,
        "claude_root_configured":true,"codex_root_configured":true}}
        """
    }

    static let refused = #"{"id":1,"error":{"code":"unavailable","message":"settings-save-failed"}}"#

    static func settings(_ values: [String: Bool]) throws -> DaemonSettingsView {
        try DaemonClient(daemon: SwitchDaemon(write: frame(values), read: frame(values))).settings()
    }

    // MARK: Decoding

    func test_theSettingsViewReadsBothKeysAndAbsenceIsNotOff() throws {
        let both = try Self.settings(["insights_counter_pass": true, "insights_recap_card_enabled": false])
        XCTAssertEqual(both.insightsCounterPass, true)
        XCTAssertEqual(both.insightsRecapCardEnabled, false)
        let neither = try Self.settings([:])
        XCTAssertNil(neither.insightsCounterPass, "an older daemon's silence read as an answer")
        XCTAssertNil(neither.insightsRecapCardEnabled, "an older daemon's silence read as an answer")
    }

    // MARK: The daemon switches

    func test_aConfirmedWriteMovesEachSwitchAndSendsOnlyItsKey() async throws {
        for which in [AppModel.InsightsDaemonSwitch.counterPass, .recapCard] {
            let key = which.rawValue
            let daemon = SwitchDaemon(write: Self.frame([key: true]), read: Self.frame([key: false]))
            let model = AppModel()
            model.setDaemonSettingsForTesting(try Self.settings([key: false]))
            model.setClientForTesting(DaemonClient(daemon: daemon))
            await model.setInsightsSwitch(which, true)
            XCTAssertEqual(model.daemonSettings.flatMap(which.value), true, key)
            XCTAssertNil(model.insightsSwitchRefusal[which], key)
            XCTAssertFalse(model.insightsSwitchBusy.contains(which), key)
            XCTAssertEqual(daemon.calls.first?.method, "set_settings")
            XCTAssertEqual(daemon.calls.first?.params, "{\"\(key)\":true}")
        }
    }

    func test_anEchoOfTheOtherValueIsARefusal() async throws {
        for which in [AppModel.InsightsDaemonSwitch.counterPass, .recapCard] {
            let key = which.rawValue
            let daemon = SwitchDaemon(write: Self.frame([key: false]), read: Self.frame([key: false]))
            let model = AppModel()
            model.setDaemonSettingsForTesting(try Self.settings([key: false]))
            model.setClientForTesting(DaemonClient(daemon: daemon))
            await model.setInsightsSwitch(which, true)
            XCTAssertEqual(model.daemonSettings.flatMap(which.value), false, key)
            XCTAssertEqual(model.insightsSwitchRefusal[which], MonitorWords.table?.requestFailed, key)
            XCTAssertNotNil(model.insightsSwitchRefusal[which], key)
        }
    }

    func test_aFailedWriteKeepsTheDaemonsValueAndSaysSo() async throws {
        for which in [AppModel.InsightsDaemonSwitch.counterPass, .recapCard] {
            let key = which.rawValue
            for read in [Self.frame([key: false]), Self.refused] {
                let model = AppModel()
                model.setDaemonSettingsForTesting(try Self.settings([key: false]))
                model.setClientForTesting(DaemonClient(daemon: SwitchDaemon(write: Self.refused, read: read)))
                await model.setInsightsSwitch(which, true)
                XCTAssertEqual(model.daemonSettings.flatMap(which.value), false, key)
                XCTAssertNotNil(model.insightsSwitchRefusal[which], key)
                XCTAssertFalse(model.insightsSwitchBusy.contains(which), key)
            }
        }
    }

    /// A refusal on one switch is said under that switch only.
    func test_aRefusalStaysWithItsSwitch() async throws {
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(["insights_counter_pass": false]))
        model.setClientForTesting(DaemonClient(daemon: SwitchDaemon(write: Self.refused, read: Self.refused)))
        await model.setInsightsSwitch(.counterPass, true)
        XCTAssertNotNil(model.insightsSwitchRefusal[.counterPass])
        XCTAssertNil(model.insightsSwitchRefusal[.recapCard])
        XCTAssertNil(model.insightsLedgerFeedRefusal)
    }

    // MARK: Source rules

    static func card() throws -> String {
        try SettingsParityTests.text("Views/Settings/InsightsSettingsCard.swift")
    }

    /// The three switches are one card in Tools, a section that waits for
    /// first run (R-43), each drawn only from a value its owner reported,
    /// labelled in the core's words, and taking no second write in flight.
    func test_theCardDrawsEachSwitchFromItsConfirmedValue() throws {
        let card = try Self.card()
        // Counting is always on (product owner, 2026-10-29): no switch.
        XCTAssertFalse(card.contains("insightsCounterPass"))
        XCTAssertFalse(card.contains("analytics_setting_counter_pass"))
        XCTAssertTrue(card.contains("if let on = settings.insightsRecapCardEnabled {"))
        XCTAssertTrue(card.contains("text(\"analytics_setting_recap_card\")"), "not the label")
        // The higher-than-usual card's switch is gone (product owner,
        // 2026-10-29, #1753), with the core's operation behind it.
        XCTAssertFalse(card.contains("lever"))
        XCTAssertFalse(card.contains("analytics_setting_lever"))
        XCTAssertNil(TCInsights.copy()?["analytics_setting_lever"])
        XCTAssertTrue(card.contains("Task { await model.setInsightsSwitch(.recapCard, on) }"))
        XCTAssertTrue(card.contains(".disabled(model.insightsSwitchBusy.contains(.recapCard))"))
        XCTAssertTrue(card.contains("model.insightsSwitchRefusal[.recapCard]"))
        // "Hide" on the summary card writes the daemon directly, so the
        // card re-reads the setting whenever it appears.
        XCTAssertTrue(card.contains("model.refreshSettings()"))
        XCTAssertEqual(card.components(separatedBy: "GlassToggleStyle(.settings)").count - 1, 1)
        // No headed card without a switch in it (an older daemon).
        XCTAssertTrue(card.contains("if model.daemonSettings?.insightsRecapCardEnabled != nil {"))

        // Its own section since the Settings IA review (D13), inside the
        // modal's gate: it waits for first run, as Tools did.
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertEqual(content.components(separatedBy: "InsightsSection()").count - 1, 1)
        XCTAssertTrue(content.contains("case .insights: InsightsSection()\n"))
        XCTAssertTrue(content.contains("case .tools: ToolsSection()\n"))
        XCTAssertFalse(content.contains("InsightsSettingsCard()"), "the card is drawn outside its section")
        XCTAssertTrue(card.contains("struct InsightsSection: View {\n    var body: some View {\n        InsightsSettingsCard()\n        SubscriptionUsageSettingsSection()"))
        XCTAssertFalse(SettingsSection.insights.availableBeforeOnboarding)
        XCTAssertFalse(SettingsSection.tools.availableBeforeOnboarding)
    }

    /// The card authors no sentence: every word is the core's.
    func test_theCardAuthorsNoSentence() throws {
        let card = try Self.card()
        for needle in ["Text(\"", "Toggle(\"", "GlassEyebrowCard(\""] {
            XCTAssertFalse(card.contains(needle), "the card authors \(needle)")
        }
    }

    /// The core carries the counter switch's label (Ron's new string).
    func test_theCoreNamesTheCounterSwitch() {
        XCTAssertEqual(TCInsights.copy()?["analytics_setting_counter_pass"], "Count tokens in watched folders")
    }
}

/// Answers `set_settings` with `write` and every other call with `read`.
private final class SwitchDaemon: DaemonCalling, @unchecked Sendable {
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
