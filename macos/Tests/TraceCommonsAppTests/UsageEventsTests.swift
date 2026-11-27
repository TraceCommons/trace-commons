import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The anonymous weekly usage counts (#1920, approved by the owner
/// 2026-11-16) as the app sends them: which screen or action reports which
/// event, and that a report never waits and never surfaces.
@MainActor
final class UsageEventsTests: XCTestCase {
    // MARK: Main tabs

    func test_eachMainTabReportsItsOwnEvent() {
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .home), .tabHomeOpened)
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .inference), .tabPrivateAiOpened)
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .traces), .tabTracesOpened)
        XCTAssertEqual(MonitorWindowView.usageEvent(for: .insights), .tabInsightsOpened)
    }

    /// `reportDrawnTab` as the window runs it: the reducer, and the tab it
    /// remembers as last reported. `land` is `consumePending`, which sets the
    /// tab and clears the destination; `tabChanged` is `onChange(of: tab)`;
    /// `readinessChanged` the readiness handler.
    @MainActor private struct Window {
        var tab: MonitorWindowView.Tab
        var destination: MonitorWindowView.Tab?
        var ready = true
        var requiresOnboarding = false
        var lastReported: MonitorWindowView.Tab?
        var sent: [UsageEvent] = []

        mutating func report() {
            guard let event = MonitorWindowView.usageReport(
                lastReported: lastReported, ready: ready, destinationLandsNow: destination != nil,
                tab: tab, requiresOnboarding: requiresOnboarding) else { return }
            lastReported = MonitorWindowView.shownTab(tab, requiresOnboarding: requiresOnboarding)
            sent.append(event)
        }

        mutating func land() {
            guard let destination else { return }
            tab = destination
            self.destination = nil
        }

        mutating func pick(_ picked: MonitorWindowView.Tab) {
            let changed = picked != tab
            tab = picked
            if changed { report() }
        }
    }

    /// A destination landing as the window opens counts once, as the tab it
    /// lands on, whichever handler runs first.
    func test_aDestinationLandingAsTheWindowOpensCountsOnce() {
        // The destination lands first, the readiness handler reports, and
        // the tab's change handler runs after both.
        var landedFirst = Window(tab: .home, destination: .traces)
        landedFirst.land()
        landedFirst.report()
        landedFirst.report()
        XCTAssertEqual(landedFirst.sent, [.tabTracesOpened])

        // The readiness handler runs while the destination still waits, then
        // the destination lands and the tab's change handler reports.
        var reportedFirst = Window(tab: .home, destination: .traces)
        reportedFirst.report()
        XCTAssertEqual(reportedFirst.sent, [], "counted the tab a landing destination is about to replace")
        reportedFirst.land()
        reportedFirst.report()
        XCTAssertEqual(reportedFirst.sent, [.tabTracesOpened])
    }

    /// The tab the window opens on counts once; each pick of another tab
    /// counts once; a redraw or a pick of the drawn tab counts nothing.
    func test_theOpeningAndEachPickCountOnce() {
        var window = Window(tab: .insights)
        window.report()
        window.report()
        window.pick(.insights)
        XCTAssertEqual(window.sent, [.tabInsightsOpened])
        window.pick(.home)
        window.pick(.inference)
        window.pick(.traces)
        window.pick(.insights)
        XCTAssertEqual(window.sent, [.tabInsightsOpened, .tabHomeOpened, .tabPrivateAiOpened, .tabTracesOpened,
                                     .tabInsightsOpened])
    }

    /// Nothing counts before the window knows what it draws; the opening
    /// counts once it does.
    func test_nothingCountsBeforeTheWindowIsReady() {
        var window = Window(tab: .home, ready: false)
        window.report()
        window.pick(.traces)
        XCTAssertEqual(window.sent, [])
        window.ready = true
        window.report()
        XCTAssertEqual(window.sent, [.tabTracesOpened])
    }

    /// While onboarding is required Inference is drawn whatever is picked:
    /// it counts once, as Private AI, and a pick changes nothing.
    func test_onlyInferenceCountsDuringOnboarding() {
        var window = Window(tab: .home, requiresOnboarding: true)
        window.report()
        for tab in MonitorWindowView.Tab.allCases { window.pick(tab) }
        XCTAssertEqual(window.sent, [.tabPrivateAiOpened])
    }

    /// The window runs the reducer from the readiness handler, after the
    /// handlers that land a waiting destination, and from the tab's change
    /// handler; nowhere else.
    func test_theWindowReportsFromTheReducerOnly() throws {
        let window = try SettingsParityTests.text("Views/MonitorWindowView.swift")
        let change = try XCTUnwrap(window.range(of: ".onChange(of: tab) { _, picked in\n"))
        XCTAssertTrue(window[change.upperBound...].hasPrefix("            reportDrawnTab()\n"))
        let ready = try XCTUnwrap(window.range(of:
            ".onChange(of: model.onboardingKnown && model.daemonData != nil, initial: true) { _, _ in\n"
                + "            reportDrawnTab()\n"))
        let requests = try XCTUnwrap(window.range(of: ".onChange(of: navigation.requests, initial: true)"))
        let known = try XCTUnwrap(window.range(of: ".onChange(of: model.onboardingKnown) { _, _ in consumePending() }"))
        XCTAssertLessThan(requests.lowerBound, ready.lowerBound)
        XCTAssertLessThan(known.lowerBound, ready.lowerBound)
        XCTAssertEqual(window.components(separatedBy: "reportDrawnTab()").count - 1, 3, "a third caller")
        XCTAssertTrue(window.contains("lastReportedTab = Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding)"))
        XCTAssertTrue(window.contains("recordUsage: { model.recordUsage($0) })"), "Insights is given no reporter")
        XCTAssertEqual(window.components(separatedBy: "model.recordUsage(").count - 1, 2)
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
