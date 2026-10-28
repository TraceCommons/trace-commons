import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Subscriptions card's model and its Settings switch (PR #1608 design,
/// first slice). The card reads the held snapshot when it opens and asks
/// for a probe only on Refresh; it follows the probe until no login says
/// `refreshing`; an older daemon's `unknown_method` draws no card; the
/// switch is drawn and moved only from the daemon's own value.
@MainActor
final class SubscriptionUsageModelTests: XCTestCase {
    static let off = DaemonData.SubscriptionUsage(enabled: false, logins: nil)

    static func on(refreshing: Bool, permille: Int = 420) -> DaemonData.SubscriptionUsage {
        .init(enabled: true, logins: [
            .init(provider: "codex", state: "observed", reason: nil, refreshing: refreshing, source: "cli",
                  updatedAt: "2026-10-24T15:02:11+00:00", stale: false, plan: "plus", limitReached: false,
                  windows: [.init(lane: "session", label: nil, durationMins: 300, usedPermille: permille,
                                  resetsAt: nil)],
                  credits: nil, resetCredits: nil),
        ])
    }

    // MARK: Reading

    func test_openingReadsTheSnapshotWithoutAskingForAProbe() async {
        let daemon = ScriptedUsage([.success(Self.on(refreshing: false))])
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.open()
        await daemon.waitForCalls(1)
        await Task.yield()
        XCTAssertEqual(daemon.refreshes, [false])
        XCTAssertTrue(model.shown)
        XCTAssertFalse(model.refreshing)
    }

    func test_theSwitchOffDrawsNoCard() async {
        let daemon = ScriptedUsage([.success(Self.off)])
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.open()
        await daemon.waitForCalls(1)
        await Task.yield()
        XCTAssertNotNil(model.usage)
        XCTAssertFalse(model.shown)
    }

    /// A daemon built without the feature refuses the method: no card,
    /// never an error or an empty one.
    func test_anOlderDaemonDrawsNoCard() async {
        let daemon = ScriptedUsage([.success(Self.on(refreshing: false)),
                                    .failure(DaemonDataError.daemon(code: "unknown_method", message: "x"))])
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.open()
        await daemon.waitForCalls(1)
        await Task.yield()
        XCTAssertTrue(model.shown)
        model.refresh()
        await model.waitForRefresh()
        XCTAssertNil(model.usage)
        XCTAssertFalse(model.shown)
    }

    /// Any other failed read keeps what is held.
    func test_aFailedReadKeepsTheHeldReading() async {
        let daemon = ScriptedUsage([.success(Self.on(refreshing: false)),
                                    .failure(DaemonDataError.unreachable)])
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.open()
        await daemon.waitForCalls(1)
        await Task.yield()
        model.refresh()
        await model.waitForRefresh()
        XCTAssertEqual(model.usage, Self.on(refreshing: false))
        XCTAssertFalse(model.refreshing)
    }

    // MARK: Refresh

    /// Refresh sends `refresh: true` once, then plain reads until the probe
    /// is done, and keeps the probe's answer.
    func test_refreshFollowsTheProbeUntilItIsDone() async {
        let daemon = ScriptedUsage([
            .success(Self.on(refreshing: true)), .success(Self.on(refreshing: true)),
            .success(Self.on(refreshing: false, permille: 600)),
        ])
        let pauses = PauseLog()
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { await pauses.record($0) })
        model.refresh()
        XCTAssertTrue(model.refreshing)
        await model.waitForRefresh()
        XCTAssertEqual(daemon.refreshes, [true, false, false])
        let count = await pauses.count
        XCTAssertEqual(count, 2)
        XCTAssertFalse(model.refreshing)
        XCTAssertEqual(model.usage?.logins?.first?.windows?.first?.usedPermille, 600)
    }

    /// A probe the daemon skipped (the 60-second floor, Codex open) answers
    /// `refreshing: false` at once: one call, no follow-up reads.
    func test_aSkippedProbeIsOneCall() async {
        let daemon = ScriptedUsage([.success(Self.on(refreshing: false))])
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.refresh()
        await model.waitForRefresh()
        XCTAssertEqual(daemon.refreshes, [true])
    }

    /// The follow-up is bounded: a daemon that never finishes is read on
    /// the schedule and then left alone. Nothing polls in the background.
    func test_followingIsBounded() async {
        let daemon = ScriptedUsage(Array(repeating: .success(Self.on(refreshing: true)), count: 40))
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.refresh()
        await model.waitForRefresh()
        XCTAssertEqual(daemon.refreshes.count, 1 + SubscriptionUsageModel.pollSchedule.count)
        XCTAssertEqual(daemon.refreshes.filter { $0 }.count, 1, "only the press asks for a probe")
        XCTAssertFalse(model.asking)
        let total = SubscriptionUsageModel.pollSchedule.reduce(Duration.zero, +)
        XCTAssertGreaterThan(total, .seconds(30),
                             "the schedule outlasts the probe's 20-second budget and 4-second shutdown")
        XCTAssertLessThanOrEqual(total, .seconds(40))
    }

    /// A second press while one is followed sends nothing.
    func test_aSecondPressIsIgnored() async {
        let daemon = ScriptedUsage([.success(Self.on(refreshing: false))])
        let model = SubscriptionUsageModel(reader: daemon.read, pause: { _ in })
        model.refresh()
        model.refresh()
        await model.waitForRefresh()
        XCTAssertEqual(daemon.refreshes, [true])
    }

    func test_withoutADaemonNothingIsAsked() async {
        let model = SubscriptionUsageModel(reader: nil)
        model.open()
        model.refresh()
        await model.waitForRefresh()
        XCTAssertFalse(model.shown)
        XCTAssertFalse(model.refreshing)
    }

    // MARK: The design sample

    /// `TRACE_COMMONS_INSIGHTS_SAMPLE=1`: ChatGPT Plus, the 5-hour window
    /// 42% with about three hours to go, Weekly 13% with about four days,
    /// and one reset credit.
    func test_theSampleReadsAsTheDesignReviewExpects() throws {
        let now = Date()
        let usage = InsightsSampleWeek.subscriptionUsage(now: now)
        var words: [String: String] = [:]
        for key in SubscriptionUsageWords.Key.all { words[key] = key }
        words[SubscriptionUsageWords.Key.used] = "{n}% used"
        words[SubscriptionUsageWords.Key.resetCredits] = "{n} reset credits"
        let row = try XCTUnwrap(SubscriptionUsageWords.rows(usage, copy: words, now: now).first)
        XCTAssertEqual(row.title, SubscriptionUsageWords.Key.plan("plus"))
        XCTAssertEqual(row.meters.map(\.name),
                       [SubscriptionUsageWords.Key.window5h, SubscriptionUsageWords.Key.windowWeekly])
        XCTAssertEqual(row.meters.map(\.used), ["42% used", "13% used"])
        XCTAssertTrue(row.meters.allSatisfy { $0.resets != nil }, "no sample reset is already past")
        XCTAssertEqual(row.credits, [SubscriptionUsageWords.Key.resetCreditOne])
        XCTAssertEqual(row.status, [])
        XCTAssertNotNil(row.updated)
    }

    // MARK: The Settings switch

    static func frame(_ subscription: String?) -> String {
        let key = subscription.map { "\"subscription_usage\":\($0)," } ?? ""
        return """
        {"id":1,"result":{\(key)"quiescence_secs":45,"digest_interval_secs":3600,
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
        "max_uploads_per_day":100,"near_ai_configured":false,
        "claude_root_configured":true,"codex_root_configured":true}}
        """
    }

    static let refused = #"{"id":1,"error":{"code":"bad_params","message":"unknown-setting"}}"#

    static func settings(_ subscription: String?) throws -> DaemonSettingsView {
        try DaemonClient(daemon: SettingsDaemon(write: frame(subscription), read: frame(subscription))).settings()
    }

    /// Absent from a daemon built without the feature: no switch, never an
    /// "off".
    func test_theSettingsViewReadsTheSwitchAndAbsenceIsNotOff() throws {
        XCTAssertEqual(try Self.settings(#"{"codex":true}"#).subscriptionUsage?.codex, true)
        XCTAssertEqual(try Self.settings(#"{"codex":false,"claude":false}"#).subscriptionUsage?.codex, false)
        XCTAssertNil(try Self.settings(nil).subscriptionUsage)
    }

    func test_aConfirmedWriteMovesTheSwitchAndSendsOnlyItsKey() async throws {
        let daemon = SettingsDaemon(write: Self.frame(#"{"codex":true}"#), read: Self.frame(#"{"codex":false}"#))
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(#"{"codex":false}"#))
        model.setClientForTesting(DaemonClient(daemon: daemon))
        await model.setSubscriptionUsageCodex(true)
        XCTAssertEqual(model.daemonSettings?.subscriptionUsage?.codex, true)
        XCTAssertNil(model.subscriptionUsageRefusal)
        XCTAssertFalse(model.subscriptionUsageBusy)
        XCTAssertEqual(daemon.calls.first?.method, "set_settings")
        XCTAssertEqual(daemon.calls.first?.params, #"{"subscription_usage":{"codex":true}}"#)
    }

    func test_anEchoOfTheOtherValueIsARefusal() async throws {
        let daemon = SettingsDaemon(write: Self.frame(#"{"codex":false}"#), read: Self.frame(#"{"codex":false}"#))
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(#"{"codex":false}"#))
        model.setClientForTesting(DaemonClient(daemon: daemon))
        await model.setSubscriptionUsageCodex(true)
        XCTAssertEqual(model.daemonSettings?.subscriptionUsage?.codex, false)
        XCTAssertEqual(model.subscriptionUsageRefusal, MonitorWords.table?.requestFailed)
    }

    /// A daemon built without the feature refuses the key: the switch keeps
    /// the daemon's value and says the write failed.
    func test_aRefusedWriteKeepsTheDaemonsValueAndSaysSo() async throws {
        let model = AppModel()
        model.setDaemonSettingsForTesting(try Self.settings(#"{"codex":false}"#))
        model.setClientForTesting(DaemonClient(daemon: SettingsDaemon(write: Self.refused,
                                                                      read: Self.frame(#"{"codex":false}"#))))
        await model.setSubscriptionUsageCodex(true)
        XCTAssertEqual(model.daemonSettings?.subscriptionUsage?.codex, false)
        XCTAssertNotNil(model.subscriptionUsageRefusal)
        XCTAssertFalse(model.subscriptionUsageBusy)
    }

    // MARK: Source rules

    /// The card and the switch author no sentence: every word is the
    /// core's, through the Insights copy table.
    func test_theCardAndSwitchAuthorNoSentence() throws {
        for file in ["Views/SubscriptionUsageCard.swift", "Views/Settings/SubscriptionUsageSettingsCard.swift"] {
            let source = try SettingsParityTests.text(file)
            for needle in ["Text(\"", "Toggle(\"", "GlassEyebrowCard(\"", "GlassRefreshButton(\""] {
                XCTAssertFalse(source.contains(needle), "\(file) authors \(needle)")
            }
        }
        let settings = try SettingsParityTests.text("Views/Settings/SubscriptionUsageSettingsCard.swift")
        XCTAssertTrue(settings.contains("TCInsights.copy()"))
        XCTAssertTrue(settings.contains("model.daemonSettings?.subscriptionUsage?.codex"))
        XCTAssertTrue(settings.contains(".disabled(model.subscriptionUsageBusy)"))
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertEqual(content.components(separatedBy: "SubscriptionUsageSettingsCard()").count - 1, 1)
        let overview = try SettingsParityTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertTrue(overview.contains("if let subscriptions, subscriptions.shown {"))
    }

    /// The core sends every word the card and the switch read, so none is
    /// drawn as an empty string.
    func test_theCoreSendsEveryWord() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in SubscriptionUsageWords.Key.all {
            XCTAssertFalse((copy[key] ?? "").isEmpty, "the core does not send \(key)")
        }
    }
}

/// Answers `subscription_usage` from a script, in order; the last answer
/// repeats. Records each call's `refresh`.
private final class ScriptedUsage: @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [Result<DaemonData.SubscriptionUsage, Error>]
    private var sent: [Bool] = []

    init(_ answers: [Result<DaemonData.SubscriptionUsage, Error>]) { self.answers = answers }

    var refreshes: [Bool] { lock.withLock { sent } }

    func read(_ refresh: Bool) async throws -> DaemonData.SubscriptionUsage {
        let answer = lock.withLock {
            sent.append(refresh)
            return answers.count > 1 ? answers.removeFirst() : answers[0]
        }
        return try answer.get()
    }

    func waitForCalls(_ count: Int) async {
        for _ in 0..<200 where refreshes.count < count { await Task.yield() }
    }
}

private actor PauseLog {
    private(set) var count = 0
    func record(_: Duration) { count += 1 }
}

/// Answers `set_settings` with `write` and every other call with `read`.
private final class SettingsDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []
    let write: String
    let read: String

    init(write: String, read: String) {
        self.write = write
        self.read = read
    }

    var calls: [(method: String, params: String)] { lock.withLock { recorded } }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.withLock { recorded.append((method: method, params: paramsJSON)) }
        return method == "set_settings" ? write : read
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
