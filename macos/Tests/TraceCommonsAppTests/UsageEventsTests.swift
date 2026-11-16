import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The anonymous weekly usage counts (#1920, approved by the owner
/// 2026-11-16) as the app sends them: which screen or action reports which
/// event, that a report never waits and never surfaces, and the
/// `share_usage_data` switch in Settings > General.
@MainActor
final class UsageEventsTests: XCTestCase {
    // MARK: Main tabs

    func test_eachMainTabReportsItsOwnEvent() {
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .home), .tabHomeOpened)
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .inference), .tabPrivateAiOpened)
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .traces), .tabTracesOpened)
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .insights), .tabInsightsOpened)
    }

    /// A change of tab reports the tab now drawn, once; no change of what is
    /// drawn reports nothing.
    func test_aTabChangeReportsTheTabNowDrawn() {
        for old in MonitorWindowView.Tab.allCases {
            for new in MonitorWindowView.Tab.allCases {
                let event = MonitorWindowView.usageEvent(tabChangedFrom: old, to: new, requiresOnboarding: false)
                XCTAssertEqual(event, old == new ? nil : MonitorWindowView.usageEvent(for: new), "\(old) -> \(new)")
            }
        }
    }

    /// The window's opening counts the tab it opens on, once, and only once
    /// the window knows what it draws.
    func test_theOpeningCountsTheDrawnTabOnceItIsKnown() {
        func opening(reported: Bool = false, known: Bool = true, daemon: Bool = true, lands: Bool = false,
                     tab: MonitorWindowView.Tab = .insights, onboarding: Bool = false) -> UsageEvent? {
            MonitorWindowView.openingUsageEvent(reported: reported, onboardingKnown: known, hasDaemon: daemon,
                                                destinationLandsNow: lands, tab: tab, requiresOnboarding: onboarding)
        }
        XCTAssertEqual(opening(), .tabInsightsOpened)
        XCTAssertEqual(opening(tab: .home), .tabHomeOpened)
        XCTAssertEqual(opening(onboarding: true), .tabPrivateAiOpened, "the drawn tab, not the restored one")
        XCTAssertNil(opening(reported: true), "an opening counted twice")
        XCTAssertNil(opening(known: false), "counted before the core said what is drawn")
        XCTAssertNil(opening(daemon: false), "counted with no daemon to report to")
        XCTAssertNil(opening(lands: true), "counted the tab a landing destination is about to replace")
    }

    /// While onboarding is required Inference is drawn whatever is picked, so
    /// a change of the picked tab opens nothing.
    func test_aTabChangeDuringOnboardingReportsNothing() {
        for old in MonitorWindowView.Tab.allCases {
            for new in MonitorWindowView.Tab.allCases {
                XCTAssertNil(MonitorWindowView.usageEvent(tabChangedFrom: old, to: new, requiresOnboarding: true))
            }
        }
    }

    /// The window reports on a tab change and on opening, from the drawn
    /// tab, and nowhere else.
    func test_theWindowReportsOnOpeningAndOnATabChange() throws {
        let window = try SettingsParityTests.text("Views/MonitorWindowView.swift")
        let change = try XCTUnwrap(window.range(of: ".onChange(of: tab) { old, picked in\n"))
        let changeBody = window[change.upperBound...].prefix(800)
        XCTAssertTrue(changeBody.contains("Self.usageEvent(tabChangedFrom: old, to: picked,"))
        XCTAssertTrue(changeBody.contains("requiresOnboarding: model.requiresOnboarding) {\n"))
        XCTAssertTrue(changeBody.contains("model.recordUsage(event)"))
        XCTAssertTrue(changeBody.contains("if !openingReported {\n                reportOpening()"),
                      "a destination landing as the window opens is counted twice")
        // The opening is reported once the window knows what it draws, after
        // the handlers that land a waiting destination.
        let opening = try XCTUnwrap(window.range(of:
            ".onChange(of: model.onboardingKnown && model.daemonData != nil, initial: true) { _, _ in\n"
                + "            reportOpening()\n"))
        let requests = try XCTUnwrap(window.range(of: ".onChange(of: navigation.requests, initial: true)"))
        let known = try XCTUnwrap(window.range(of: ".onChange(of: model.onboardingKnown) { _, _ in consumePending() }"))
        XCTAssertLessThan(requests.lowerBound, opening.lowerBound)
        XCTAssertLessThan(known.lowerBound, opening.lowerBound)
        XCTAssertFalse(window.contains(".onAppear {\n            model.recordUsage"), "an opening counted before it is known")
        XCTAssertTrue(window.contains("recordUsage: { model.recordUsage($0) })"), "Insights is given no reporter")
        XCTAssertEqual(window.components(separatedBy: "model.recordUsage(").count - 1, 3)
    }

    // MARK: Insights sub-views

    func test_eachInsightsSubViewReportsItsOwnEvent() {
        XCTAssertEqual(InsightsView.usageEvent(for: .overview), .insightsOverviewOpened)
        XCTAssertEqual(InsightsView.usageEvent(for: .patterns), .insightsBreakdownOpened)
        XCTAssertEqual(InsightsView.usageEvent(for: .benchmarks), .insightsBenchmarksOpened)
        XCTAssertNil(InsightsView.usageEvent(for: .skills), "Skills is on no list")
    }

    /// Insights reports the sub-view drawn when it opens and each one picked
    /// after, from `visibleTab`, never from a redraw.
    func test_insightsReportsTheDrawnSubView() throws {
        let insights = try SettingsParityTests.text("Views/InsightsView.swift")
        XCTAssertTrue(insights.contains(
            ".onChange(of: visibleTab, initial: true) { _, shown in\n"
                + "            if let event = Self.usageEvent(for: shown) { recordUsage?(event) }\n"))
        XCTAssertEqual(insights.components(separatedBy: "recordUsage?(").count - 1, 1)
    }

    // MARK: Sending

    /// A report goes to the daemon as `record_usage_event {event}`.
    func test_aReportIsSentToTheDaemon() async throws {
        let daemon = UsageDaemon(answer: #"{"id":1,"result":{"recorded":true}}"#)
        let model = AppModel()
        model.setLiveDataForTesting(LiveDaemonClient(transport: daemon))
        let report = try XCTUnwrap(model.recordUsage(.tabTracesOpened))
        await report.value
        XCTAssertEqual(daemon.calls.map(\.method), ["record_usage_event"])
        XCTAssertEqual(daemon.calls.map(\.params), [#"{"event":"tab_traces_opened"}"#])
    }

    /// Not counted, refused, or no daemon at all: nothing throws, nothing
    /// is said, nothing the app draws changes.
    func test_aReportThatIsNotCountedSaysNothing() async throws {
        for answer in [#"{"id":1,"result":{"recorded":false}}"#,
                       #"{"id":1,"error":{"code":"bad_params","message":"usage-event-unknown"}}"#,
                       "not a frame"] {
            let daemon = UsageDaemon(answer: answer)
            let model = AppModel()
            model.setLiveDataForTesting(LiveDaemonClient(transport: daemon))
            await model.recordUsage(.insightsOverviewOpened)?.value
            XCTAssertEqual(daemon.calls.count, 1, answer)
            XCTAssertNil(model.shareUsageDataRefusal, answer)
        }
        XCTAssertNil(AppModel().recordUsage(.tabHomeOpened), "a report with no daemon went somewhere")
    }

    /// Finishing the first run reports `setup_finished`, once.
    func test_finishingTheFirstRunReportsSetupFinished() async throws {
        let sent = expectation(description: "setup_finished sent")
        let daemon = UsageDaemon(answer: #"{"id":1,"result":{"recorded":true}}"#, onCall: { sent.fulfill() })
        let model = AppModel()
        model.setLiveDataForTesting(LiveDaemonClient(transport: daemon))
        model.firstRunFinished(notice: nil)
        await fulfillment(of: [sent], timeout: 10)
        XCTAssertEqual(daemon.calls.map(\.params), [#"{"event":"setup_finished"}"#])
    }

    // MARK: The switch

    static func frame(_ value: Bool?) -> String {
        let key = value.map { "\"share_usage_data\":\($0)," } ?? ""
        return """
        {"id":1,"result":{\(key)"quiescence_secs":45,"digest_interval_secs":3600,
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
        "max_uploads_per_day":100,"near_ai_configured":false,
        "claude_root_configured":true,"codex_root_configured":true}}
        """
    }

    static let refused = #"{"id":1,"error":{"code":"unavailable","message":"settings-save-failed"}}"#

    static func settings(_ value: Bool?) throws -> DaemonSettingsView {
        try DaemonClient(daemon: UsageDaemon(answer: frame(value))).settings()
    }

    func test_theSettingsViewReadsTheKeyAndAbsenceIsNotOff() throws {
        XCTAssertEqual(try Self.settings(true).shareUsageData, true)
        XCTAssertEqual(try Self.settings(false).shareUsageData, false)
        XCTAssertNil(try Self.settings(nil).shareUsageData, "an older daemon's silence read as an answer")
    }

    /// A confirmed write moves the switch, both ways, and sends only its key.
    func test_aConfirmedWriteMovesTheSwitchAndSendsOnlyItsKey() async throws {
        for on in [false, true] {
            let daemon = UsageDaemon(answer: Self.frame(on))
            let model = AppModel()
            model.setDaemonSettingsForTesting(try Self.settings(!on))
            model.setClientForTesting(DaemonClient(daemon: daemon))
            await model.setShareUsageData(on)
            XCTAssertEqual(model.daemonSettings?.shareUsageData, on)
            XCTAssertNil(model.shareUsageDataRefusal)
            XCTAssertFalse(model.shareUsageDataBusy)
            XCTAssertEqual(daemon.calls.first?.method, "set_settings")
            XCTAssertEqual(daemon.calls.first?.params, "{\"share_usage_data\":\(on)}")
        }
    }

    /// A reply carrying the other value is where the daemon stands: kept,
    /// and refused.
    func test_anEchoOfTheOtherValueIsARefusal() async throws {
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(true))
        model.setClientForTesting(DaemonClient(daemon: UsageDaemon(answer: Self.frame(true))))
        await model.setShareUsageData(false)
        XCTAssertEqual(model.daemonSettings?.shareUsageData, true)
        XCTAssertNotNil(model.shareUsageDataRefusal)
        XCTAssertEqual(model.shareUsageDataRefusal, MonitorWords.table?.requestFailed)
    }

    /// A failed write never claims the asked value.
    func test_aFailedWriteKeepsTheDaemonsValueAndSaysSo() async throws {
        for read in [Self.frame(true), Self.refused] {
            let model = AppModel()
            model.setDaemonSettingsForTesting(try Self.settings(true))
            model.setClientForTesting(DaemonClient(daemon: UsageDaemon(write: Self.refused, read: read)))
            await model.setShareUsageData(false)
            XCTAssertEqual(model.daemonSettings?.shareUsageData, true)
            XCTAssertNotNil(model.shareUsageDataRefusal)
            XCTAssertFalse(model.shareUsageDataBusy)
        }
    }

    /// The switch sits in General, is drawn only from a value the daemon
    /// reported, takes no second write in flight, and its words are marked
    /// as drafts nobody has approved.
    func test_theSwitchIsDrawnFromItsConfirmedValueInGeneral() throws {
        let general = try SettingsParityTests.text("Views/Settings/GeneralSection.swift")
        XCTAssertEqual(general.components(separatedBy: "UsageCountsSettingsCard()").count - 1, 1)
        let card = try SettingsParityTests.text("Views/Settings/UsageCountsSettingsCard.swift")
        XCTAssertTrue(card.contains("if let on = model.daemonSettings?.shareUsageData {"))
        XCTAssertTrue(card.contains("Task { await model.setShareUsageData(on) }"))
        XCTAssertTrue(card.contains(".disabled(model.shareUsageDataBusy)"))
        XCTAssertTrue(card.contains("model.shareUsageDataRefusal"))
        XCTAssertTrue(card.contains("// COPY: pending owner approval"))
        XCTAssertTrue(SettingsSection.general.availableBeforeOnboarding,
                      "the off switch for counts that start at install waits for first run")
    }
}

/// Answers `set_settings` with `write` and every other call with `read`,
/// recording each call; `onCall` runs after each one is recorded.
private final class UsageDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []
    private let write: String
    private let read: String
    private let onCall: (@Sendable () -> Void)?

    init(answer: String, onCall: (@Sendable () -> Void)? = nil) {
        write = answer
        read = answer
        self.onCall = onCall
    }

    init(write: String, read: String) {
        self.write = write
        self.read = read
        onCall = nil
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
        onCall?()
        return method == "set_settings" ? write : read
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
