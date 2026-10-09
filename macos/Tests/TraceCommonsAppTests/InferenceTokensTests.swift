import Foundation
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Inference tab's per-call tokens. A call's four counters are drawn
/// as the daemon sent them, in a fixed order, never added together; a null
/// counter is the dash and a measured zero stays zero. The ledger feed
/// behind them is on by default and has no switch on this tab; its off
/// switch is in Settings (`InsightsLedgerFeedSettingTests`).
@MainActor
final class InferenceTokensTests: XCTestCase {
    // Recorded verbatim by `RUSTFLAGS="-D warnings" cargo test -p
    // trace-commons-contributor --lib a_recorded_ -- --nocapture`
    // (`a_recorded_tokened_call_for_the_shells`); never written by hand.
    static let knownCall = #"{"id":1,"at":"2027-01-15T08:00:00+00:00","tool":"unknown","family":"openai","model":"Qwen/Qwen3.6-27B-FP8","route":"unknown","cost":{"known":true,"priced_micros":12300},"proof":"unrecorded","tokens":{"input":1000,"cache_read":600,"cache_write":0,"output":25}}"#
    static let doubtfulCall = #"{"id":2,"at":"2027-01-15T08:00:01+00:00","tool":"unknown","family":"openai","model":"Qwen/Qwen3.6-27B-FP8","route":"unknown","cost":{"known":true,"priced_micros":12300},"proof":"unrecorded","tokens":{"input":null,"cache_read":null,"cache_write":null,"output":3}}"#

    /// Labels that are only markers, so a test reads the figures without
    /// depending on the core's wording.
    static let words: [String: String] = [
        "analytics_unavailable": "DASH",
        "metric_input_tokens": "IN",
        "analytics_series_cache_read": "CR",
        "analytics_series_cache_write": "CW",
        "metric_output_tokens": "OUT",
    ]

    static func tokens(_ json: String) throws -> DaemonData.InferenceCallTokens {
        let call = try DaemonDataDecoding.decoder().decode(DaemonData.InferenceCall.self, from: Data(json.utf8))
        return try XCTUnwrap(call.tokens)
    }

    // MARK: Words

    func test_theLineIsTheFourCountersInAFixedOrder() throws {
        let words = Self.words
        let fig = { (n: UInt64) in InsightsOverviewWords.figure(n, copy: words) }
        XCTAssertEqual(InferenceTokenWords.line(try Self.tokens(Self.knownCall), copy: words),
                       "IN \(fig(1000)) · CR \(fig(600)) · CW 0 · OUT \(fig(25))")
    }

    /// Each null counter is its own dash; the known one beside it stands.
    func test_aNullCounterIsTheDashAndZeroStaysZero() throws {
        XCTAssertEqual(InferenceTokenWords.line(try Self.tokens(Self.doubtfulCall), copy: Self.words),
                       "IN DASH · CR DASH · CW DASH · OUT 3")
        // `cache_write: 0` is a measured zero, not unknown.
        XCTAssertTrue(InferenceTokenWords.line(try Self.tokens(Self.knownCall), copy: Self.words).contains("CW 0"))
    }

    /// Nothing is added together: no figure is the sum of the counters, and
    /// the line holds exactly the four pairs.
    func test_theCountersAreNeverSummed() throws {
        let line = InferenceTokenWords.line(try Self.tokens(Self.knownCall), copy: Self.words)
        // Sums whose compact figure differs from every counter's ("1.6K",
        // "625"); 1025 would read "1K", the same as the input.
        for sum: UInt64 in [1625, 1600, 625] {
            XCTAssertFalse(line.contains(InsightsOverviewWords.figure(sum, copy: Self.words)), "\(sum) in \(line)")
        }
        XCTAssertEqual(line.components(separatedBy: " · ").count, 4)
    }

    /// The read-out says unknown where the line draws the dash.
    func test_theAccessibilityLineSaysUnknownForTheDash() throws {
        let line = InferenceTokenWords.accessibilityLine(try Self.tokens(Self.doubtfulCall), copy: Self.words)
        let unknown = MonitorWords.unknown
        XCTAssertEqual(line, "IN \(unknown) · CR \(unknown) · CW \(unknown) · OUT 3")
        XCTAssertFalse(line.contains("DASH"))
    }

    func test_theWordsAreInTheCoresTable() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in ["metric_input_tokens", "analytics_series_cache_read", "analytics_series_cache_write",
                    "metric_output_tokens", "analytics_unavailable", "analytics_setting_ledger_feed",
                    "analytics_feed_ledger"] {
            XCTAssertFalse((copy[key] ?? "").isEmpty, key)
        }
    }

    // MARK: Reload on appear

    /// The recorded `inference_calls` page with every call's `tokens` taken
    /// out, as a daemon sends it once the ledger feed is turned off.
    static func callsWithoutTokens(_ client: SampleDaemonClient) throws -> String {
        let recorded = try XCTUnwrap(client.json(for: "inference_calls"))
        var page = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(recorded.utf8)) as? [String: Any])
        let calls = try XCTUnwrap(page["calls"] as? [[String: Any]])
        page["calls"] = calls.map { call in call.filter { $0.key != "tokens" } }
        return String(decoding: try JSONSerialization.data(withJSONObject: page), as: UTF8.self)
    }

    /// Turning the ledger feed off in Settings leaves the calls already
    /// loaded holding their tokens; coming back to the tab re-reads the
    /// page, so the lines go as soon as the daemon stops sending them, not
    /// when the next call arrives.
    func test_aReloadOnAppearDropsTokensTheDaemonNoLongerSends() async throws {
        let client = SampleDaemonClient(.normalDay)
        let store = InferenceStore(client: client)
        await store.load()
        let loaded = try XCTUnwrap(store.calls?.calls)
        XCTAssertTrue(loaded.contains { $0.tokens != nil }, "the recorded page carries tokens")

        client.replaceReply("inference_calls", with: try Self.callsWithoutTokens(client))
        XCTAssertTrue(store.calls?.calls.contains { $0.tokens != nil } ?? false, "nothing re-reads before the tab appears")

        await store.appeared()
        let reloaded = try XCTUnwrap(store.calls?.calls)
        XCTAssertEqual(reloaded.count, loaded.count)
        XCTAssertTrue(reloaded.allSatisfy { $0.tokens == nil }, "a token line outlived the feed")
        XCTAssertNil(store.failures["inference_calls"])
    }

    /// The tab calls the reload each time it appears, beside the settings
    /// refresh that re-reads the ledger feed's switch, and again each time
    /// that switch moves: Settings is its own window, so the tab can stay
    /// in view while the feed is turned off there. It writes nothing.
    func test_theTabReloadsTheStoreWhenItAppears() throws {
        let views = try MonitorNavigationTests.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(views.contains(".task(id: model.daemonSettings?.insightsLedgerFeed) { await store.appeared() }"))
        XCTAssertFalse(views.contains(".task { await store.appeared() }"), "a reload that misses the switch moving")
        XCTAssertTrue(views.contains(".onAppear { model.refreshAll() }"))
        let store = try MonitorNavigationTests.text("Views/Monitor/InferenceStore.swift")
        let start = try XCTUnwrap(store.range(of: "func appeared() async {"))
        let body = store[start.upperBound...].prefix { $0 != "}" }
        XCTAssertFalse(body.contains("set"), "the reload on appear writes: \(body)")
    }

    /// While the daemon is still starting there is nothing to read, and
    /// start-up is not the core being down: the tab appearing then reads
    /// nothing and records no failure, as `run()` does.
    func test_appearingWhileTheDaemonStartsRecordsNoFailure() async {
        let store = InferenceStore(client: nil)
        store.attach(nil, awaiting: true)
        await store.appeared()
        XCTAssertTrue(store.failures.isEmpty, "start-up drawn as the core down: \(store.failures)")
        XCTAssertTrue(store.awaiting)
    }

    // MARK: Source rules

    func test_aCallRowDrawsTokensOnlyWhenTheCallCarriesThem() throws {
        let views = try MonitorNavigationTests.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(views.contains("if let tokens = call.tokens {"))
        XCTAssertTrue(views.contains("private static let insightsCopy = TCInsights.copy() ?? [:]"))
    }

    /// The totals and the per-model card add no tokens.
    func test_theTotalsReadNoTokens() throws {
        let views = try MonitorNavigationTests.text("Views/Monitor/InferenceViews.swift")
        let start = try XCTUnwrap(views.range(of: "// MARK: Totals"))
        let end = try XCTUnwrap(views.range(of: "// MARK: Calls"))
        XCTAssertFalse(views[start.upperBound ..< end.lowerBound].contains("tokens"))
    }
}
