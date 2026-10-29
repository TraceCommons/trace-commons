import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Insights switches in Settings (Ron's #1287 review, adopted by the
/// owner): "Count tokens in watched folders" (`insights_counter_pass`, on
/// by default and always on since 2026-10-29, so Settings no longer draws
/// it) and "Weekly summary" (`insights_recap_card_enabled`) are daemon
/// settings; the "Higher-than-usual card" is the core's `lever_card`
/// switch. Each reads its confirmed value, moves only on a confirmed echo,
/// and says the core's request-failed words when a write is refused.
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

    // MARK: The lever card switch (the core's `lever_card`)

    static func comparisons(cardEnabled: Bool?) -> InsightsResponse {
        let field = cardEnabled.map { ",\"card_enabled\":\($0)" } ?? ""
        return decode("""
        {"type":"comparisons","comparisons":{"feed":"saved","week_start":"2026-10-05","goals":[],
        "lever":{"pick":null,"disabled":[],"unavailable":"needs_counter_pass"\(field)},"recap":null}}
        """)
    }

    static func leverCard(off: Bool) -> InsightsResponse {
        decode("""
        {"type":"lever_card","state":{"goals":[],"lever_dismissals":[],"lever_reenables":[],
        "lever_card_off":\(off),"recap_opened_week":null}}
        """)
    }

    static func decode(_ json: String) -> InsightsResponse {
        // swiftlint:disable:next force_try
        try! JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }

    func test_theLeverSwitchReadsTheCoresValueWithoutCounterWeeks() async throws {
        let core = LeverCore([Self.comparisons(cardEnabled: false)])
        let lever = InsightsLeverCardSwitch(service: core.call)
        await lever.load()
        XCTAssertEqual(lever.enabled, false)
        let sent = try XCTUnwrap(core.requests.first)
        XCTAssertEqual(sent.operation.type, "comparisons")
        XCTAssertNil(sent.operation.counter_weeks, "Settings sends no weeks")
    }

    /// A core that does not report the switch draws none, never an "off".
    func test_anUnreportedLeverValueDrawsNoSwitch() async {
        let lever = InsightsLeverCardSwitch(service: LeverCore([Self.comparisons(cardEnabled: nil)]).call)
        await lever.load()
        XCTAssertNil(lever.enabled)
    }

    func test_aConfirmedLeverWriteMovesTheSwitch() async throws {
        let core = LeverCore([Self.comparisons(cardEnabled: false), Self.leverCard(off: false)])
        let lever = InsightsLeverCardSwitch(service: core.call)
        await lever.load()
        await lever.set(true)
        XCTAssertEqual(lever.enabled, true)
        XCTAssertNil(lever.refusal)
        XCTAssertFalse(lever.busy)
        let write = try XCTUnwrap(core.requests.last)
        XCTAssertEqual(write.operation.type, "lever_card")
        XCTAssertEqual(write.operation.enabled, true)
    }

    /// The core answered with the switch still off: that is where it
    /// stands, and the request-failed words say so.
    func test_aLeverEchoOfTheOtherValueIsARefusal() async {
        let core = LeverCore([Self.comparisons(cardEnabled: false), Self.leverCard(off: true),
                              Self.comparisons(cardEnabled: false)])
        let lever = InsightsLeverCardSwitch(service: core.call)
        await lever.load()
        await lever.set(true)
        XCTAssertEqual(lever.enabled, false)
        XCTAssertEqual(lever.refusal, MonitorWords.table?.requestFailed)
        XCTAssertNotNil(lever.refusal)
    }

    /// A failed write reads the core back and never draws what was asked.
    func test_aFailedLeverWriteReadsBackAndSaysSo() async {
        let core = LeverCore([Self.comparisons(cardEnabled: false), nil, Self.comparisons(cardEnabled: false)])
        let lever = InsightsLeverCardSwitch(service: core.call)
        await lever.load()
        await lever.set(true)
        XCTAssertEqual(lever.enabled, false)
        XCTAssertNotNil(lever.refusal)
        XCTAssertFalse(lever.busy)
    }

    func test_aConfirmedLeverWriteClearsAnEarlierRefusal() async {
        let core = LeverCore([Self.comparisons(cardEnabled: false), nil, Self.comparisons(cardEnabled: false),
                              Self.leverCard(off: false)])
        let lever = InsightsLeverCardSwitch(service: core.call)
        await lever.load()
        await lever.set(true)
        XCTAssertNotNil(lever.refusal)
        await lever.set(true)
        XCTAssertNil(lever.refusal)
        XCTAssertEqual(lever.enabled, true)
    }

    /// The real core, through the router the card uses, against a scratch
    /// store: `comparisons` with no weeks answers the switch, and a
    /// `lever_card` reply carries `lever_card_off`, both in shapes the
    /// bridge decodes. A decode slip here would leave the switch undrawn.
    func test_theRealCoreAnswersTheLeverSwitchRoundTrip() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        let router = InsightsServiceRouter(selection: .custom(root.path))
        let lever = InsightsLeverCardSwitch(service: { try await router.call($0) })

        await lever.load()
        XCTAssertEqual(lever.enabled, true, "a fresh store reads the card as on")
        let off = try await router.call(.init(operation: .init("lever_card", enabled: false)))
        XCTAssertEqual(off.state?.lever_card_off, true)
        await lever.load()
        XCTAssertEqual(lever.enabled, false)
        await lever.set(true)
        XCTAssertEqual(lever.enabled, true)
        XCTAssertNil(lever.refusal)
        await lever.load()
        XCTAssertEqual(lever.enabled, true, "the write reached the store")
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
        XCTAssertTrue(card.contains("if let on = lever.enabled {"))
        for key in ["analytics_setting_recap_card", "analytics_setting_lever"] {
            XCTAssertTrue(card.contains("text(\"\(key)\")"), "\(key) is not the label")
        }
        XCTAssertTrue(card.contains("Task { await model.setInsightsSwitch(.recapCard, on) }"))
        XCTAssertTrue(card.contains("Task { await lever.set(on) }"))
        XCTAssertTrue(card.contains(".disabled(model.insightsSwitchBusy.contains(.recapCard))"))
        XCTAssertTrue(card.contains(".disabled(lever.busy)"))
        XCTAssertTrue(card.contains("model.insightsSwitchRefusal[.recapCard]"))
        XCTAssertTrue(card.contains("lever.refusal"))
        // "Hide" on the summary card writes the daemon directly, so the
        // card re-reads both owners whenever it appears.
        XCTAssertTrue(card.contains("model.refreshSettings()"))
        XCTAssertTrue(card.contains("await lever.load()"))
        XCTAssertEqual(card.components(separatedBy: "GlassToggleStyle(.settings)").count - 1, 2)
        // No headed card without a switch in it (an older daemon and core).
        XCTAssertTrue(card.contains(
            "if model.daemonSettings?.insightsRecapCardEnabled != nil\n"
                + "                || lever.enabled != nil {"))

        // Drawn under Tools, after its card, inside the modal's gate.
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertEqual(content.components(separatedBy: "InsightsSettingsCard()").count - 1, 1)
        XCTAssertTrue(content.contains("case .tools:\n                ToolsSection()\n"))
        let tools = try XCTUnwrap(content.range(of: "case .tools:"))
        let next = try XCTUnwrap(content.range(of: "case .privateAI:"))
        let drawn = try XCTUnwrap(content.range(of: "InsightsSettingsCard()"))
        XCTAssertTrue(tools.upperBound < drawn.lowerBound && drawn.upperBound < next.lowerBound)
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

/// The core's analytics service, answering in order; `nil` throws.
private final class LeverCore: @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [InsightsResponse?]
    private var sent: [InsightsRequest] = []

    init(_ answers: [InsightsResponse?]) { self.answers = answers }

    var requests: [InsightsRequest] {
        lock.lock()
        defer { lock.unlock() }
        return sent
    }

    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        let answer: InsightsResponse? = lock.withLock {
            sent.append(request)
            return answers.isEmpty ? nil : answers.removeFirst()
        }
        guard let answer else { throw InsightsError.invalidResponse }
        return answer
    }
}
