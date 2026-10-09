import Foundation
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The menu-bar glance: today's routed calls per tool from the proxy
/// ledger (`insights_glance`). Only an enabled, readable, fresh answer
/// with rows is drawn; off, unreadable, stale, a failure and an older
/// daemon all draw no card, never a zero, and none of them stales the
/// popover.
@MainActor
final class MenuBarGlanceTests: XCTestCase {
    // `glanceOff` and `glanceUnreadable` are `handle_glance`'s two fixed
    // answers (`insights_glance.rs`), which `DaemonDataKeyCoverageTests`
    // reads from a running daemon. `glanceData`, `glanceStale` and `tipLit`
    // are recorded verbatim by `RUSTFLAGS="-D warnings" cargo test -p
    // trace-commons-contributor --lib a_recorded_ -- --nocapture`
    // (`a_recorded_glance_for_the_shells`); never written by hand.
    static let glanceOff = #"{"enabled":false,"feed":"ledger"}"#
    static let glanceUnreadable = #"{"enabled":true,"feed":"ledger","readable":false}"#
    static let glanceData = #"{"enabled":true,"feed":"ledger","readable":true,"updated_at":"2026-10-08T10:00:00+00:00","stale":false,"date":"2026-10-08","tools":[{"tool":"claude-code","calls":2,"known_calls":1,"tokens":1050,"cache_share":{"numerator":800,"denominator":1000,"permille":800}},{"tool":"codex","calls":1,"known_calls":1,"tokens":1025,"cache_share":{"numerator":600,"denominator":1000,"permille":600}},{"tool":"unknown","calls":1,"known_calls":0,"tokens":null,"cache_share":null}],"coverage":{"calls":4,"known":2,"unknown":2,"unreadable_rows":0},"context_tip":{"state":"held"}}"#
    static let glanceStale = #"{"enabled":true,"feed":"ledger","readable":true,"updated_at":"2026-10-08T09:49:59+00:00","stale":true,"date":"2026-10-08","tools":[{"tool":"claude-code","calls":1,"known_calls":1,"tokens":40,"cache_share":null}],"coverage":{"calls":1,"known":1,"unknown":0,"unreadable_rows":2},"context_tip":{"state":"held"}}"#
    static let tipLit = #"{"state":"lit","context":180000,"threshold":200000}"#

    static func decode(_ json: String) throws -> DaemonData.InsightsGlance {
        try JSONDecoder().decode(DaemonData.InsightsGlance.self, from: Data(json.utf8))
    }

    /// Templates that are only their holes, so a test reads the filled
    /// figures without depending on the core's wording.
    static let words: [String: String] = [
        "analytics_unavailable": "DASH",
        "analytics_state_partial": "PARTIAL",
        "analytics_glance_line": "{tokens}|{p}",
        "analytics_glance_tokens_only": "{tokens}|ONLY",
        "analytics_tip_context": "CTX {ctx}",
        "analytics_tip_threshold": "TH {threshold}",
    ]

    // MARK: State mapping

    func test_aGlanceThatIsNotServedOrFailsIsUnavailable() {
        let failures: [any Error] = [
            DaemonDataError.daemon(code: "unknown_method", message: "insights_glance"),
            DaemonDataError.notAvailableYet(method: "insights_glance"),
            DaemonDataError.undecodable(method: "insights_glance"),
            DaemonDataError.unreachable,
        ]
        for error in failures {
            XCTAssertEqual(MenuPanelData.glance(.failure(error)), .unavailable, "\(error)")
        }
    }

    func test_eachShapeMapsToItsState() throws {
        XCTAssertEqual(MenuPanelData.glance(.success(try Self.decode(Self.glanceOff))), .off)
        XCTAssertEqual(MenuPanelData.glance(.success(try Self.decode(Self.glanceUnreadable))), .unreadable)
        XCTAssertEqual(MenuPanelData.glance(.success(try Self.decode(Self.glanceStale))), .stale)
        let data = try Self.decode(Self.glanceData)
        XCTAssertEqual(MenuPanelData.glance(.success(data)), .data(data))
    }

    /// A readable answer without its rows or coverage is not drawn.
    func test_aReadableAnswerWithoutRowsIsUnreadable() throws {
        let noTools = Self.glanceData.replacingOccurrences(
            of: #""tools":["#, with: #""dropped":["#)
        XCTAssertEqual(MenuPanelData.glance(.success(try Self.decode(noTools))), .unreadable)
        let noCoverage = Self.glanceData.replacingOccurrences(of: #""coverage":"#, with: #""dropped":"#)
        XCTAssertEqual(MenuPanelData.glance(.success(try Self.decode(noCoverage))), .unreadable)
    }

    /// A missing `stale` is stale: an absent signal is never fresh.
    func test_aMissingStaleKeyIsStale() throws {
        let noStale = Self.glanceData.replacingOccurrences(of: #""stale":false,"#, with: "")
        XCTAssertNotEqual(noStale, Self.glanceData)
        XCTAssertEqual(MenuPanelData.glance(.success(try Self.decode(noStale))), .stale)
    }

    /// The card draws only data with rows, and only while the popover is
    /// current.
    func test_theCardDrawsOnlyFreshDataWithRows() throws {
        let data = try Self.decode(Self.glanceData)
        XCTAssertEqual(MenuPanelData.glanceToDraw(.data(data), stale: false), data)
        XCTAssertNil(MenuPanelData.glanceToDraw(.data(data), stale: true))
        for state: MenuPanelData.GlanceState in [.notRead, .off, .unavailable, .unreadable, .stale] {
            XCTAssertNil(MenuPanelData.glanceToDraw(state, stale: false), "\(state)")
        }
        let empty = try Self.decode(Self.glanceData.replacing(
            #/"tools":\[.*\],"coverage"/#, with: #""tools":[],"coverage""#))
        XCTAssertEqual(empty.tools, [])
        XCTAssertNil(MenuPanelData.glanceToDraw(.data(empty), stale: false))
    }

    // MARK: The store never stales the popover

    func test_anOlderDaemonLeavesThePopoverCurrent() async {
        let store = MenuPanelStore(client: LiveDaemonClient(transport: GlanceRefusingTransport(.normalDay)))
        await store.load()
        XCTAssertFalse(store.stale, "a daemon without insights_glance must not stale the popover")
        XCTAssertEqual(store.glance, .unavailable)
    }

    func test_theSampleClientDrawsNoGlanceAndStaysCurrent() async {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertFalse(store.stale)
        XCTAssertEqual(store.glance, .unavailable)
    }

    func test_aServedGlanceIsReadAndAttachForgetsIt() async throws {
        let transport = GlanceRefusingTransport(.normalDay, glance: Self.glanceData)
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertFalse(store.stale)
        XCTAssertEqual(store.glance, .data(try Self.decode(Self.glanceData)))
        XCTAssertEqual(transport.glanceParams, [#"{"tz":\#(TimeZone.current.secondsFromGMT())}"#])
        store.attach(SampleDaemonClient(.normalDay))
        XCTAssertEqual(store.glance, .notRead)
    }

    func test_noClientIsUnavailable() async {
        let store = MenuPanelStore(client: nil)
        await store.load()
        XCTAssertEqual(store.glance, .unavailable)
    }

    // MARK: Words

    func test_aRowsFigureIsTheCoresWordsFilled() throws {
        let tools = try XCTUnwrap(try Self.decode(Self.glanceData).tools)
        let words = Self.words
        XCTAssertEqual(InsightsGlanceWords.figureLine(tools[0], copy: words),
                       "\(InsightsOverviewWords.figure(1050, copy: words))|80")
        XCTAssertEqual(InsightsGlanceWords.figureLine(tools[1], copy: words),
                       "\(InsightsOverviewWords.figure(1025, copy: words))|60")
        // Unknown tokens are the dash, never zero.
        XCTAssertEqual(InsightsGlanceWords.figureLine(tools[2], copy: words), "DASH")
        // Known tokens with no cache share use the tokens-only words.
        let staleRow = try XCTUnwrap(try Self.decode(Self.glanceStale).tools?.first)
        XCTAssertEqual(InsightsGlanceWords.figureLine(staleRow, copy: words), "40|ONLY")
    }

    func test_aRowsTextKeepsTheRecentRowsJoiner() throws {
        let tools = try XCTUnwrap(try Self.decode(Self.glanceData).tools)
        XCTAssertEqual(InsightsGlanceWords.rowText(tools[2], copy: Self.words), "— · DASH")
        XCTAssertTrue(InsightsGlanceWords.rowText(tools[0], copy: Self.words).hasPrefix("Claude Code · "))
    }

    /// Partial only when some, not all and not none, of the calls are known.
    func test_partialIsSomeButNotAllCallsKnown() throws {
        let tools = try XCTUnwrap(try Self.decode(Self.glanceData).tools)
        XCTAssertEqual(InsightsGlanceWords.partial(tools[0], copy: Self.words), "PARTIAL")
        XCTAssertNil(InsightsGlanceWords.partial(tools[1], copy: Self.words))
        XCTAssertNil(InsightsGlanceWords.partial(tools[2], copy: Self.words))
    }

    func test_unreadableRowsMarkTheDayPartial() throws {
        XCTAssertNil(InsightsGlanceWords.dayPartial(try Self.decode(Self.glanceData), copy: Self.words))
        XCTAssertEqual(InsightsGlanceWords.dayPartial(try Self.decode(Self.glanceStale), copy: Self.words), "PARTIAL")
    }

    /// An unknown figure is read out as unknown, not as the dash.
    func test_anUnknownFigureIsReadAsUnknown() throws {
        let tools = try XCTUnwrap(try Self.decode(Self.glanceData).tools)
        XCTAssertEqual(InsightsGlanceWords.accessibilityValue(tools[2]), MonitorWords.unknown)
        XCTAssertNil(InsightsGlanceWords.accessibilityValue(tools[0]))
    }

    /// The tip draws only when lit; held (today's only state) draws none.
    func test_theTipDrawsOnlyWhenLit() throws {
        XCTAssertNil(InsightsGlanceWords.tip(try Self.decode(Self.glanceData), copy: Self.words))
        let lit = try Self.decode(Self.glanceData.replacingOccurrences(of: #"{"state":"held"}"#, with: Self.tipLit))
        XCTAssertEqual(InsightsGlanceWords.tip(lit, copy: Self.words),
                       "CTX \(InsightsOverviewWords.figure(180_000, copy: Self.words)) "
                           + "TH \(InsightsOverviewWords.figure(200_000, copy: Self.words))")
        let quiet = try Self.decode(Self.glanceData.replacingOccurrences(
            of: #"{"state":"held"}"#, with: #"{"state":"quiet","context":180000,"threshold":200000}"#))
        XCTAssertNil(InsightsGlanceWords.tip(quiet, copy: Self.words))
    }

    /// The words the card reads exist in the core's table.
    func test_theCardsWordsAreInTheCoresTable() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in ["analytics_today", "analytics_glance_routed_only", "analytics_glance_line",
                    "analytics_glance_tokens_only", "analytics_unavailable", "analytics_state_partial",
                    "analytics_tip_context", "analytics_tip_threshold", "analytics_mark_a11y_tip"] {
            XCTAssertFalse((copy[key] ?? "").isEmpty, key)
        }
    }

    // MARK: Source rules

    func test_theCardHasNoMoneyNoSumAndNoMute() throws {
        let card = try MonitorNavigationTests.text("Views/Monitor/InsightsGlanceCard.swift").lowercased()
        for forbidden in ["cost", "priced", "money", "total", "analytics_tip_mute", "reduce("] {
            XCTAssertFalse(card.contains(forbidden), "the glance card contains \(forbidden)")
        }
        XCTAssertTrue(card.contains("tcinsights.copy()"), "the card reads the core's words")
    }

    func test_thePanelDrawsTheGlanceBetweenTheGraphAndRecent() throws {
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("            graph\n            glance\n            recent\n"))
        XCTAssertTrue(panel.contains("MenuPanelData.glanceToDraw(store.glance, stale: store.stale)"))
        // The interval re-read runs beside the pinned opening read.
        let follow = try XCTUnwrap(panel.range(of: ".task { await store.followGlance() }"))
        let pinned = try XCTUnwrap(panel.range(
            of: "        .task { await store.load() }\n        .onAppear { model.refreshAll() }\n"))
        XCTAssertLessThan(follow.lowerBound, pinned.lowerBound)
    }

    /// The popover only reads the glance: neither the card nor the store
    /// behind it turns the ledger feed on or off.
    func test_thePopoverNeverWritesTheLedgerFeed() throws {
        for file in ["Views/Monitor/InsightsGlanceCard.swift", "Views/Monitor/MenuPanelStore.swift"] {
            let source = try MonitorNavigationTests.text(file)
            for forbidden in ["setInsightsLedgerFeed(", "insights_ledger_feed"] {
                XCTAssertFalse(source.contains(forbidden), "\(file) contains \(forbidden)")
            }
        }
    }

    func test_usageChangedReReadsTheGlance() throws {
        let store = try MonitorNavigationTests.text("Views/Monitor/MenuPanelStore.swift")
        XCTAssertTrue(store.contains("case .usageChanged:\n                await loadGlance()"))
    }
}

/// Answers from a sample set's recorded replies; `insights_glance` is
/// refused as `unknown_method`, as an older daemon refuses it, unless a
/// glance is given.
private final class GlanceRefusingTransport: DaemonTransport, @unchecked Sendable {
    private let set: SampleDaemonClient.SampleSet
    private let glance: String?
    private let lock = NSLock()
    private var params: [String] = []

    init(_ set: SampleDaemonClient.SampleSet, glance: String? = nil) {
        self.set = set
        self.glance = glance
    }

    var glanceParams: [String] { lock.withLock { params } }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == "insights_glance" {
            lock.withLock { params.append(paramsJSON) }
            guard let glance else {
                return #"{"id":0,"error":{"code":"unknown_method","message":"insights_glance"}}"#
            }
            return #"{"id":0,"result":\#(glance)}"#
        }
        guard let reply = SampleDaemonData.reply(method, in: set) else {
            return #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
        }
        return #"{"id":0,"result":\#(reply)}"#
    }
}
