import Foundation
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The Inference tab's per-call tokens and the ledger-feed switch behind
/// them. A call's four counters are drawn as the daemon sent them, in a
/// fixed order, never added together; a null counter is the dash and a
/// measured zero stays zero. The switch moves only on a write the daemon
/// confirmed, and is not drawn for a daemon that does not report it.
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

    /// The switch sits after the calls it governs and before the account
    /// cards, and is not drawn while the daemon has not reported it.
    func test_theSwitchSitsBetweenTheCallsAndTheAccount() throws {
        let views = try MonitorNavigationTests.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(views.contains(
            "                ledgerSections\n                InsightsLedgerFeedSwitch(store: store)\n"
                + "                InferenceAccountSection(store: store)\n"))
        XCTAssertTrue(views.contains("if let on = store.ledgerFeed {"))
    }

    // MARK: Store

    func test_aConfirmedWriteMovesTheSwitchAndRereadsTheCalls() async {
        let transport = LedgerFeedTransport(settings: #"{"insights_ledger_feed":false}"#,
                                            write: #"{"insights_ledger_feed":true}"#)
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(store.ledgerFeed, false)
        transport.reset()
        await store.setLedgerFeed(true)
        XCTAssertEqual(store.ledgerFeed, true)
        XCTAssertNil(store.ledgerFeedRefusal)
        XCTAssertFalse(store.ledgerFeedBusy)
        XCTAssertEqual(transport.calls.map(\.method), ["set_settings", "inference_calls"])
        XCTAssertEqual(transport.calls.first?.params, #"{"insights_ledger_feed":true}"#)
    }

    func test_aRefusedWriteKeepsTheSwitchAndSaysSo() async {
        let transport = LedgerFeedTransport(settings: #"{"insights_ledger_feed":false}"#, write: nil)
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        await store.setLedgerFeed(true)
        XCTAssertEqual(store.ledgerFeed, false)
        XCTAssertNotNil(store.ledgerFeedRefusal)
        XCTAssertEqual(store.ledgerFeedRefusal, MonitorWords.table?.requestFailed)
        XCTAssertFalse(store.ledgerFeedBusy)
    }

    /// A reply that does not echo what was asked is not a confirmation: the
    /// switch stands where the daemon has it.
    func test_anUnconfirmedReplyKeepsTheDaemonsValue() async {
        let transport = LedgerFeedTransport(settings: #"{"insights_ledger_feed":false}"#,
                                            write: #"{"insights_ledger_feed":false}"#)
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        await store.setLedgerFeed(true)
        XCTAssertEqual(store.ledgerFeed, false)
        XCTAssertEqual(store.ledgerFeedRefusal, MonitorWords.table?.requestFailed)
        let noEcho = LedgerFeedTransport(settings: #"{"insights_ledger_feed":false}"#, write: "{}")
        let other = InferenceStore(client: LiveDaemonClient(transport: noEcho))
        await other.load()
        await other.setLedgerFeed(true)
        XCTAssertEqual(other.ledgerFeed, false)
        XCTAssertEqual(other.ledgerFeedRefusal, MonitorWords.table?.requestFailed)
    }

    /// An older daemon omits the key: nil, never drawn as off.
    func test_aDaemonWithoutTheKeyLeavesTheSwitchUnknown() async {
        let store = InferenceStore(client: LiveDaemonClient(transport: LedgerFeedTransport(settings: "{}", write: nil)))
        await store.load()
        XCTAssertNil(store.ledgerFeed)
    }

    func test_attachForgetsTheSwitch() async {
        let transport = LedgerFeedTransport(settings: #"{"insights_ledger_feed":true}"#, write: nil)
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        await store.setLedgerFeed(false)
        XCTAssertEqual(store.ledgerFeed, true)
        XCTAssertNotNil(store.ledgerFeedRefusal)
        store.attach(nil)
        XCTAssertNil(store.ledgerFeed)
        XCTAssertNil(store.ledgerFeedRefusal)
        XCTAssertFalse(store.ledgerFeedBusy)
    }

    func test_theSampleClientReadsTheSwitch() async {
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(store.ledgerFeed, false)
    }
}

/// Answers from the normal day's recorded replies, with `get_settings` and
/// `set_settings` scripted. A nil `write` refuses the write. Records every
/// call by method and params.
private final class LedgerFeedTransport: DaemonTransport, @unchecked Sendable {
    private let settings: String
    private let write: String?
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []

    init(settings: String, write: String?) {
        self.settings = settings
        self.write = write
    }

    var calls: [(method: String, params: String)] { lock.withLock { recorded } }
    func reset() { lock.withLock { recorded = [] } }

    func call(_ method: String, params: String) -> String {
        lock.withLock { recorded.append((method, params)) }
        switch method {
        case "get_settings":
            return #"{"id":0,"result":\#(settings)}"#
        case "set_settings":
            guard let write else {
                return #"{"id":0,"error":{"code":"internal","message":"settings-write-failed"}}"#
            }
            return #"{"id":0,"result":\#(write)}"#
        default:
            guard let reply = SampleDaemonData.reply(method, in: .normalDay) else {
                return #"{"id":0,"error":{"code":"unknown_method","message":"\#(method)"}}"#
            }
            return #"{"id":0,"result":\#(reply)}"#
        }
    }
}
