import Foundation
import XCTest
import SwiftUI
import AppKit
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class InsightsModelTests: XCTestCase {
    @MainActor
    func testFreshNavigationDoesNotAdvanceContributionGates() {
        let trace = AppModel()
        let navigation = MainWindowNavigation()
        // D-11: resting on Insights no longer defers services; the first
        // request starts them, once, and still advances no contribution gate.
        var starts = 0
        navigation.activateServicesIfNeeded { starts += 1 }
        XCTAssertEqual(starts, 1)
        XCTAssertEqual(trace.startup, .starting)
        XCTAssertFalse(trace.status.loggedIn)
        XCTAssertFalse(trace.isOnboardingComplete)
        XCTAssertFalse(trace.traceNavigationReady)
        navigation.activateServicesIfNeeded { starts += 1 }
        navigation.activateServicesIfNeeded { starts += 1 }
        XCTAssertEqual(starts, 1)
    }

    @MainActor
    func testClosingDiscardsLateResponseEvenWhenOperationIgnoresCancellation() async throws {
        let gate = ResponseGate()
        let model = InsightsModel(service: { request in try await gate.call(request) })
        model.open()
        await gate.waitForCall()
        model.close()
        try await gate.resolve("{\"type\":\"copy\",\"copy\":{\"title\":\"late\"}}")
        for _ in 0..<20 { await Task.yield() }
        XCTAssertTrue(model.copy.isEmpty)
        XCTAssertTrue(model.snapshots.isEmpty)
    }

    @MainActor
    func testInitialCopyThenListUsesOnlyLocalService() async throws {
        let recorder = Recorder()
        let model = InsightsModel(service: { request in try await recorder.call(request) })
        model.open()
        await model.loading?.value
        XCTAssertEqual(model.text("title"), "Insights")
        let operations = await recorder.operations
        XCTAssertEqual(operations, ["copy", "list"])
        XCTAssertEqual(model.snapshots.map(\.id), ["snap-1"])
        model.close()
    }

    /// A failed read shows the failure line, not blank words or an empty
    /// store that looks like no snapshots.
    @MainActor
    func testAFailedCopyReadSetsTheFailureLine() async {
        let model = InsightsModel(service: { _ in throw InsightsError.operationFailed })
        model.open()
        await model.loading?.value
        XCTAssertEqual(model.error, "insights-operation-failed")
        XCTAssertTrue(model.snapshots.isEmpty)
        model.close()
        XCTAssertNil(model.error)
    }

    /// A list read answered with another type is a failure, in the core's
    /// own words once the copy has been read.
    @MainActor
    func testAMismatchedListResponseSetsTheCoresFailureLine() async {
        let model = InsightsModel(service: { _ in
            let json = "{\"type\":\"copy\",\"copy\":{\"error\":\"Could not read Insights\"}}"
            return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
        })
        model.open()
        await model.loading?.value
        XCTAssertEqual(model.text("error"), "Could not read Insights")
        XCTAssertEqual(model.error, "Could not read Insights")
        XCTAssertTrue(model.snapshots.isEmpty)
        model.close()
    }

    /// A read that succeeds clears an earlier failure.
    @MainActor
    func testASuccessfulReadClearsAnEarlierFailure() async {
        let recorder = Recorder()
        let fail = FailOnce()
        let model = InsightsModel(service: { request in
            if await fail.next() { throw InsightsError.operationFailed }
            return try await recorder.call(request)
        })
        model.open()
        await model.loading?.value
        XCTAssertNotNil(model.error)
        model.close()
        model.open()
        await model.loading?.value
        XCTAssertNil(model.error)
        XCTAssertEqual(model.snapshots.map(\.id), ["snap-1"])
        model.close()
    }

    /// Reading the snapshots again lists only, once the copy is in hand,
    /// and picks up a snapshot saved since.
    @MainActor
    func testReloadSnapshotsListsAgainWithoutTheCopy() async {
        let recorder = Recorder()
        let model = InsightsModel(service: { request in try await recorder.call(request) })
        model.open()
        await model.loading?.value
        await recorder.add("snap-2")
        model.reloadSnapshots()
        await model.loading?.value
        let operations = await recorder.operations
        XCTAssertEqual(operations, ["copy", "list", "list"])
        XCTAssertEqual(model.snapshots.map(\.id), ["snap-1", "snap-2"])
        model.close()
        model.reloadSnapshots()
        XCTAssertNil(model.loading, "a closed screen reads nothing")
        let after = await recorder.operations
        XCTAssertEqual(after.count, 3)
    }
}

private actor FailOnce {
    private var failed = false
    func next() -> Bool { defer { failed = true }; return !failed }
}

private actor ResponseGate {
    private var continuation: CheckedContinuation<InsightsResponse, Error>?
    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        try await withCheckedThrowingContinuation { continuation = $0 }
    }
    func waitForCall() async {
        while continuation == nil { await Task.yield() }
    }
    func resolve(_ json: String) throws {
        continuation?.resume(returning: try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8)))
        continuation = nil
    }
}
private actor Recorder {
    var operations: [String] = []
    private var ids = ["snap-1"]
    func add(_ id: String) { ids.append(id) }
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        operations.append(request.operation.type)
        if request.operation.type == "copy" {
            let json = "{\"type\":\"copy\",\"copy\":{\"title\":\"Insights\"}}"
            return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
        }
        let list: [String: Any] = ["type": "list", "insights": ids.map { InsightsSessionsTests.snapshotJSON($0) }]
        return try JSONDecoder().decode(InsightsResponse.self, from: JSONSerialization.data(withJSONObject: list))
    }
}

/// The Overview tab's model and words: every figure is the core's, an
/// unknown figure is a dash, and nothing is re-sorted by value.
final class InsightsOverviewTests: XCTestCase {
    static let words: [String: String] = [
        "analytics_unavailable": "\u{2014}",
        "analytics_coverage_line": "Counts known for {k} of {n} traces \u{b7} {p} partial \u{b7} {u} unknown, not counted as zero",
        "analytics_feed_saved": "FEED_SAVED",
        "analytics_feed_comparisons_need_counter_pass": "NEEDS_T",
        "analytics_unknown_label": "UNKNOWN_LABEL",
        "analytics_source_codex": "CODEX_LINE",
        "analytics_reason_no_usage_counters": "NO_COUNTERS",
        "analytics_state_unknown": "STATE_UNKNOWN",
        "analytics_largest": "Largest: {t} tokens",
    ]

    static let overviewJSON = """
    {"type":"week_overview","overview":{
      "feed":"saved","generation":18446744073709551615,
      "week_start":"2026-09-07","week_end":"2026-09-13","tz":0,
      "coverage":{"known":1,"partial":1,"unknown":2,"reasons":{"no_usage_counters":2}},
      "undated_sessions":0,"sessions":4,
      "sources":[
        {"source":"claude_code","sessions":3,"tokens":null,"cache_share":null,
         "largest_session":null,"change":"needs_counter_pass","best_week":"needs_counter_pass"},
        {"source":"codex","sessions":1,"tokens":0,
         "cache_share":{"numerator":20,"denominator":50,"permille":400},
         "largest_session":{"session_ref":"abc","tokens":0},
         "change":"needs_counter_pass","best_week":"needs_counter_pass"}],
      "by_day":null,"codex_interval_tokens":0,
      "by_model":[{"label":"alpha","tokens":5},{"label":"zeta","tokens":900},{"label":null,"tokens":7000}],
      "by_tool":[{"source":"claude_code","tokens":null},{"source":"codex","tokens":0}],
      "by_project":"not_available_for_analyzed_files",
      "weeks":["2026-09-07"]}}
    """

    func overview() throws -> InsightsWeekOverview {
        try XCTUnwrap(JSONDecoder().decode(InsightsResponse.self, from: Data(Self.overviewJSON.utf8)).overview)
    }

    func testUnknownIsADashAndAMeasuredZeroStaysZero() throws {
        let week = try overview()
        let copy = Self.words
        XCTAssertEqual(InsightsOverviewWords.figure(week.sources[0].tokens, copy: copy), "\u{2014}")
        XCTAssertEqual(InsightsOverviewWords.figure(week.sources[1].tokens, copy: copy), "0")
        XCTAssertEqual(InsightsOverviewWords.share(week.sources[0].cache_share, copy: copy), "\u{2014}")
        XCTAssertEqual(InsightsOverviewWords.share(week.sources[1].cache_share, copy: copy), "40")
        // Feed S compares nothing: the change and the best week are dashes.
        XCTAssertEqual(InsightsOverviewWords.change(week.sources[1], copy: copy), "\u{2014}")
        XCTAssertEqual(InsightsOverviewWords.bestWeek(week.sources[1], copy: copy), "\u{2014}")
        XCTAssertEqual(InsightsOverviewWords.sourceLine("codex", copy: copy), "CODEX_LINE")
    }

    func testCoverageAndFeedLinesAreTheCoresWordsFilled() throws {
        let week = try overview()
        XCTAssertEqual(InsightsOverviewWords.coverageLine(week.coverage, copy: Self.words),
                       "Counts known for 1 of 4 traces \u{b7} 1 partial \u{b7} 2 unknown, not counted as zero")
        XCTAssertEqual(InsightsOverviewWords.feedLines(week.feed, copy: Self.words), ["FEED_SAVED", "NEEDS_T"])
        XCTAssertEqual(InsightsOverviewWords.feedLines("counter_pass", copy: Self.words), [""])
        XCTAssertEqual(InsightsOverviewWords.feedLines("counter_pass", copy: ["analytics_feed_counter_pass": "FEED_T"]),
                       ["FEED_T"])
        // The ledger feed's caption, as GTK and Windows draw it.
        XCTAssertEqual(InsightsOverviewWords.feedLines("ledger", copy: ["analytics_feed_ledger": "FEED_L"]),
                       ["FEED_L"])
        XCTAssertEqual(InsightsOverviewWords.reason("no_usage_counters", copy: Self.words), "NO_COUNTERS")
        XCTAssertEqual(InsightsOverviewWords.state("unknown", copy: Self.words), "STATE_UNKNOWN")
        XCTAssertEqual(InsightsOverviewWords.fill("Largest: {t} tokens", ["t": "9"]), "Largest: 9 tokens")
    }

    func testModelRowsKeepTheCoresOrderWhateverTheirFigures() throws {
        let week = try overview()
        let rows = InsightsOverviewWords.modelRows(week, copy: Self.words)
        XCTAssertEqual(rows.map(\.label), ["alpha", "zeta", "UNKNOWN_LABEL"])
    }

    @MainActor
    func testOverviewModelAsksForTheWeekInTheLocalOffsetAndDrillsDown() async throws {
        let recorder = OverviewRecorder()
        let model = InsightsOverviewModel(service: { try await recorder.call($0) })
        model.open()
        for _ in 0..<500 where model.busy || model.sessionsInputs == nil { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.overview?.week_start, "2026-09-07")
        var requests = await recorder.requests
        // The week, then the Sessions figure's breakdown, always read with
        // it (owner, 2026-10-22).
        XCTAssertEqual(requests.map(\.operation.type), ["week_overview", "card_inputs"])
        XCTAssertNil(requests[0].operation.week_start)
        XCTAssertEqual(requests[0].operation.tz, Int32(TimeZone.current.secondsFromGMT()))
        XCTAssertEqual(requests[1].operation.card, "sessions")
        XCTAssertEqual(requests[1].operation.week_start, "2026-09-07")
        XCTAssertEqual(model.sessionsInputs?.card, "sessions")
        XCTAssertNil(model.openCard, "the breakdown is not a figure's drill-down")

        model.showInputs("cache_share")
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        requests = await recorder.requests
        XCTAssertEqual(requests.last?.operation.type, "card_inputs")
        XCTAssertEqual(requests.last?.operation.card, "cache_share")
        XCTAssertEqual(requests.last?.operation.week_start, "2026-09-07")
        XCTAssertEqual(model.inputs?.card, "cache_share")
        XCTAssertEqual(model.inputs?.sessions.first?.tokens, nil)

        model.selectWeek("2026-08-31")
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        requests = await recorder.requests
        XCTAssertEqual(requests.last?.operation.week_start, "2026-08-31")
        XCTAssertNil(model.inputs, "a new week closes the old drill-down")

        await recorder.fail()
        model.showInputs("tokens")
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertNil(model.inputs)
        XCTAssertNotNil(model.overview, "a failed drill-down closes only itself")
        XCTAssertFalse(model.failed)
        model.reload()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertNil(model.overview, "a failed read never keeps the old figures")
        XCTAssertTrue(model.failed)
        model.close()
    }

    @MainActor
    func testOverviewOverTheRealCoreReadsAnAbsentStoreWithoutCreatingIt() async throws {
        let store = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let model = InsightsOverviewModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        model.open()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.failed)
        XCTAssertEqual(model.overview?.feed, "saved")
        XCTAssertEqual(model.overview?.sessions, 0)
        XCTAssertEqual(model.overview?.by_project, "not_available_for_analyzed_files")
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.close()
    }
}

private actor OverviewRecorder {
    var requests: [InsightsRequest] = []
    private var failing = false
    func fail() { failing = true }
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        requests.append(request)
        if failing { throw InsightsError.service("insights-operation-failed") }
        let json: String
        if request.operation.type == "card_inputs" {
            json = """
            {"type":"card_inputs","inputs":{"card":"\(request.operation.card ?? "")","feed":"saved",
             "generation":1,"week_start":"2026-09-07","tz":0,"sources":[],
             "coverage":{"known":0,"partial":0,"unknown":1,"reasons":{}},
             "sessions":[{"session_ref":"abc","source":"claude_code","tokens":null,"cache_share":null,
               "state":"unknown","reasons":["no_usage_counters"]}]}}
            """
        } else {
            json = InsightsOverviewTests.overviewJSON.replacingOccurrences(
                of: "\"2026-09-07\",\"week_end\"",
                with: "\"\(request.operation.week_start ?? "2026-09-07")\",\"week_end\"")
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}

/// Feed T in the window (owner decision D4, open): the model reads
/// `insights_week` through the daemon client and shows it only when the
/// daemon answers enabled and readable; anything else is feed S, with the
/// feed line naming which. The two are never mixed.
final class InsightsWeekFeedTests: XCTestCase {
    static let counted = """
    {"enabled": true, "feed": "counter_pass", "readable": true,
     "updated_at": "2026-10-08T09:59:12+00:00", "sessions_stored": 3,
     "iso_week": "2026-W41", "week_start": "2026-10-05",
     "comparable": false, "unavailable": "below_coverage_floor",
     "change_vs_last_week": [
       {"source": "claude_code", "permille": null, "unavailable": "below_coverage_floor"}],
     "overview": {"feed": "counter_pass"},
     "rollup": {"feed": "counter_pass", "week_start": "2026-10-05",
       "coverage": {"known": 1, "partial": 1, "unknown": 1,
                    "reasons": {"some_turns_unknown": 1, "no_usage_counters": 1}},
       "undated_sessions": 0,
       "sources": [{"source": "claude_code", "sessions": 3, "tokens": 37235,
                    "largest_session_tokens": 30000,
                    "cache_share": {"numerator": 24500, "denominator": 37000}}],
       "by_day": [{"date": "2026-10-05", "uncached": 15, "cache_read": 24500,
                   "cache_write": 12600, "output": 120}],
       "codex_interval_tokens": null,
       "by_model": [{"label": "claude-fixture-model", "tokens": 37235},
                    {"label": null, "tokens": 0}],
       "sessions": [{"source": "claude_code", "tokens": 37235, "state": "partial",
                     "reasons": ["some_turns_unknown"]}]}}
    """

    static func week(_ json: String) throws -> DaemonData.InsightsWeek {
        try DaemonDataDecoding.decoder().decode(DaemonData.InsightsWeek.self, from: Data(json.utf8))
    }

    @MainActor
    private func model(_ reader: InsightsModel.WeekReader?) -> InsightsModel {
        InsightsModel(service: { _ in throw InsightsError.invalidResponse }, weekReader: reader)
    }

    @MainActor
    func testAnEnabledReadableWeekShowsFeedT() async throws {
        let week = try Self.week(Self.counted)
        let model = model({ _ in week })
        await model.loadWeek()
        XCTAssertEqual(model.weekFeed, .counterPass)
        XCTAssertEqual(model.counterWeek?.rollup?.sources.first?.tokens, 37235)
        XCTAssertEqual(model.counterWeek?.rollup?.byModel.last?.label, nil)
        XCTAssertEqual(model.feedLineKey, InsightsModel.FeedLineKey.counterPass)
        XCTAssertNil(model.counterPassNoticeKey)
    }

    @MainActor
    func testSwitchedOffOrAnOlderDaemonShowsFeedSWithoutANotice() async throws {
        let off = try Self.week(#"{"enabled": false, "feed": "counter_pass"}"#)
        let readers: [InsightsModel.WeekReader?] = [
            nil,
            { _ in off },
            { _ in throw DaemonDataError.daemon(code: "unknown_method", message: "insights_week") },
            { _ in throw DaemonDataError.notAvailableYet(method: "insights_week") },
        ]
        for reader in readers {
            let model = model(reader)
            await model.loadWeek()
            XCTAssertEqual(model.weekFeed, .saved)
            XCTAssertNil(model.counterWeek)
            XCTAssertEqual(model.feedLineKey, InsightsModel.FeedLineKey.saved)
            XCTAssertNil(model.counterPassNoticeKey)
        }
    }

    @MainActor
    func testAFailedOrUnreadableReadShowsFeedSAndSaysCountingIsUnavailable() async throws {
        let unreadable = try Self.week(
            #"{"enabled": true, "feed": "counter_pass", "readable": false, "reason": "store_unreadable"}"#)
        let readers: [InsightsModel.WeekReader] = [
            { _ in unreadable },
            { _ in throw DaemonDataError.unreachable },
            { _ in throw DaemonDataError.undecodable(method: "insights_week") },
        ]
        for reader in readers {
            let model = model(reader)
            await model.loadWeek()
            XCTAssertEqual(model.weekFeed, .saved)
            XCTAssertNil(model.counterWeek)
            XCTAssertEqual(model.feedLineKey, InsightsModel.FeedLineKey.saved)
            XCTAssertEqual(model.counterPassNoticeKey, InsightsModel.FeedLineKey.counterPassUnavailable)
        }
    }

    @MainActor
    func testAWeekWithoutTheCoresOverviewIsNotDrawnAsFeedT() async throws {
        // An older daemon answers the rollup alone: the window cannot draw it
        // with the core's shapes, so it shows the saved feed and says so.
        let bare = try Self.week(Self.counted.replacingOccurrences(
            of: #""overview": {"feed": "counter_pass"},"#, with: ""))
        XCTAssertFalse(bare.showsCounterPass)
        let model = model({ _ in bare })
        await model.loadWeek()
        XCTAssertEqual(model.weekFeed, .saved)
        XCTAssertEqual(model.counterPassNoticeKey, InsightsModel.FeedLineKey.counterPassUnavailable)
    }

    @MainActor
    func testFeedTIsDroppedWholeWhenALaterReadFails() async throws {
        let week = try Self.week(Self.counted)
        let fail = FlipReader(week: week)
        let model = model({ iso in try await fail.read(iso) })
        await model.loadWeek()
        XCTAssertEqual(model.weekFeed, .counterPass)
        await fail.breakIt()
        await model.loadWeek(isoWeek: "2026-W40")
        XCTAssertEqual(model.weekFeed, .saved)
        XCTAssertNil(model.counterWeek, "never a T figure under the S feed line")
        let asked = await fail.asked
        XCTAssertEqual(asked, [nil, "2026-W40"])
    }

    @MainActor
    func testTheFeedLinesAreCoreCopy() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in [InsightsModel.FeedLineKey.saved, InsightsModel.FeedLineKey.counterPass,
                    InsightsModel.FeedLineKey.counterPassUnavailable] {
            XCTAssertFalse(copy[key, default: ""].isEmpty, key)
        }
    }
}

private actor FlipReader {
    let week: DaemonData.InsightsWeek
    private var broken = false
    private(set) var asked: [String?] = []
    init(week: DaemonData.InsightsWeek) { self.week = week }
    func breakIt() { broken = true }
    func read(_ iso: String?) throws -> DaemonData.InsightsWeek {
        asked.append(iso)
        if broken { throw DaemonDataError.unreachable }
        return week
    }
}

/// The Patterns tab's model and words: every figure is the core's, an absent
/// week is a gap and never a zero bar, files are letters, and no week is
/// compared under the saved feed.
final class InsightsPatternsTests: XCTestCase {
    static let words: [String: String] = [
        "analytics_unavailable": "\u{2014}",
        "analytics_pattern_repeated_reads_count": "{r} reads of {f} files",
        "analytics_pattern_retried_calls_count": "{c} calls",
        "analytics_pattern_edit_fail_edit_count": "{l} times",
        "analytics_pattern_long_context_line": "after {threshold}",
        "analytics_pattern_repeated_reads_headline_one": "{r} READ",
        "analytics_pattern_repeated_reads_headline_other": "{r} READS",
        "analytics_pattern_repeated_reads_count_under_count_one": "in {f} file",
        "analytics_pattern_repeated_reads_count_under_count_other": "in {f} files",
        "analytics_pattern_retried_calls_headline_one": "{c} CALL",
        "analytics_pattern_retried_calls_headline_other": "{c} CALLS",
        "analytics_pattern_retried_calls_count_under_count": "same tool and input",
        "analytics_pattern_edit_fail_edit_headline_one": "{l} TIME",
        "analytics_pattern_edit_fail_edit_headline_other": "{l} TIMES",
        "analytics_pattern_edit_fail_edit_count_under_count": "edited, failed, edited",
        "analytics_pattern_long_context_headline_one": "{n} TURN",
        "analytics_pattern_long_context_headline_other": "{n} TURNS",
        "analytics_estimate_from_result_size": "ABOUT",
        "analytics_from_counters": "COUNTERS",
        "analytics_inferred_from_order": "INFERRED",
        "analytics_claude_sessions_only": "Claude only: {k} of {n}.",
        "analytics_see_sessions": "Show {n} traces",
        "analytics_file_label": "File {letter} \u{b7} {ext}",
        "analytics_file_label_no_ext": "File {letter}",
    ]

    static let patternsJSON = """
    {"type":"patterns","patterns":{
      "feed":"saved","generation":7,"week_start":"2026-09-14","week_end":"2026-09-20","tz":0,
      "coverage":{"known":1,"partial":0,"unknown":0,"reasons":{}},
      "sessions":2,"claude_sessions":1,"claude_only":true,"long_context_threshold":200000,
      "cards":[
        {"kind":"repeated_reads","tokens":20000,"count":3,"files":2,"sessions":1,
         "basis":"estimate_from_result_size","inferred":false,
         "weeks":[{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":20000}],
         "change":null,"change_unavailable":"needs_counter_pass"},
        {"kind":"retried_calls","tokens":0,"count":0,"files":null,"sessions":0,
         "basis":"estimate_from_result_size","inferred":false,
         "weeks":[{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":0}],
         "change":null,"change_unavailable":"needs_counter_pass"},
        {"kind":"edit_fail_edit","tokens":null,"count":1,"files":null,"sessions":1,
         "basis":"estimate_from_result_size","inferred":true,
         "weeks":[{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":null}],
         "change":null,"change_unavailable":"needs_counter_pass"},
        {"kind":"long_context","tokens":530000,"count":2,"files":null,"sessions":1,
         "basis":"from_counters","inferred":false,
         "weeks":[{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":530000}],
         "change":null,"change_unavailable":"needs_counter_pass"}],
      "reread_files":[
        {"letter":"B","ext":".rs","reads":2,"after_shrink":1,"tokens":20000},
        {"letter":"A","ext":null,"reads":1,"after_shrink":0,"tokens":null}],
      "weeks":["2026-09-14"]}}
    """

    func patterns() throws -> InsightsWeekPatterns {
        try XCTUnwrap(JSONDecoder().decode(InsightsResponse.self, from: Data(Self.patternsJSON.utf8)).patterns)
    }

    func testCardsKeepTheCoresOrderAndTheirCountLinesAreTheCoresWordsFilled() throws {
        let week = try patterns()
        let copy = Self.words
        XCTAssertEqual(week.cards.map(\.kind), ["repeated_reads", "retried_calls", "edit_fail_edit", "long_context"])
        XCTAssertEqual(InsightsPatternsWords.shown(week.cards).map(\.kind), ["retried_calls"])
        XCTAssertEqual(InsightsPatternsWords.shown(week.cards).map { InsightsPatternsWords.countLine($0, copy: copy) },
                       ["0 calls"])
        // A measured zero stays zero; an unknown figure is the dash.
        XCTAssertEqual(InsightsOverviewWords.figure(week.cards[1].tokens, copy: copy), "0")
        XCTAssertEqual(InsightsOverviewWords.figure(week.cards[2].tokens, copy: copy), "\u{2014}")
    }

    func testDerivationLabelsNameTheBasisAndTheInferredCard() throws {
        let week = try patterns()
        XCTAssertEqual(InsightsPatternsWords.basisLines(week.cards[0], copy: Self.words), ["ABOUT"])
        XCTAssertEqual(InsightsPatternsWords.basisLines(week.cards[2], copy: Self.words), ["INFERRED", "ABOUT"])
        XCTAssertEqual(InsightsPatternsWords.basisLines(week.cards[3], copy: Self.words), ["COUNTERS"])
    }

    func testAnAbsentWeekIsAGapNeverAZeroBar() throws {
        let week = try patterns()
        let bars = InsightsPatternsWords.bars(week.cards[1])
        XCTAssertEqual(bars.map(\.week), ["2026-09-07", "2026-09-14"])
        XCTAssertNil(bars[0].tokens)
        XCTAssertEqual(bars[1].tokens, 0)
        XCTAssertEqual(InsightsPatternsWords.drawnBars(week.cards[1]).map(\.week), ["2026-09-14"])
        XCTAssertEqual(InsightsPatternsWords.drawnBars(week.cards[2]).count, 0)
    }

    /// Owner decision 2026-10-20: a card whose estimate rounds to 0 leads
    /// with its count and unit, as the core says, and the line beneath it
    /// drops the count; a token headline is unchanged, and an answer from
    /// before the core said leads with tokens.
    func testACardTheCoreLeadsWithItsCountShowsTheCountAndUnit() throws {
        let copy = Self.words
        func card(_ kind: String, tokens: String, count: UInt32, files: String = "null",
                  headline: String? = "count") throws -> InsightsPatternCard {
            let field = headline.map { "\"headline\":\"\($0)\"," } ?? ""
            let json = """
            {"kind":"\(kind)","tokens":\(tokens),"count":\(count),\(field)"files":\(files),"sessions":1,
             "basis":"estimate_from_result_size","inferred":false,"weeks":[],"change":250,"change_unavailable":null}
            """
            return try JSONDecoder().decode(InsightsPatternCard.self, from: Data(json.utf8))
        }
        let call = try card("retried_calls", tokens: "0", count: 1)
        XCTAssertEqual(InsightsPatternsWords.headline(call, copy: copy), "1 CALL")
        XCTAssertEqual(InsightsPatternsWords.countLine(call, copy: copy), "same tool and input")
        XCTAssertTrue(InsightsPatternsWords.leadsWithCount(call))

        // An answer without the field leads with tokens, even at 0.
        let older = try card("retried_calls", tokens: "0", count: 2, headline: nil)
        XCTAssertNil(older.headline)
        XCTAssertFalse(InsightsPatternsWords.leadsWithCount(older))
        XCTAssertEqual(InsightsPatternsWords.headline(older, copy: copy), "0")
        // A headline this build does not know leads with tokens.
        XCTAssertFalse(InsightsPatternsWords.leadsWithCount(try card("retried_calls", tokens: "0", count: 2,
                                                                     headline: "later")))
        // The count headline never carries the token change beside it.
        let view = try InsightsGlassConventionsTests.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(view.contains("Text(InsightsPatternsWords.headline(card, copy: copy)).insightsFigure()"))
        XCTAssertTrue(view.contains("if card.change != nil, !InsightsPatternsWords.leadsWithCount(card) {"))
    }

    func testACountHeadlineCardDrawsNoZeroOnItsCurrentWeekBar() throws {
        let copy = Self.words
        func card(headline: String, tokens: String, count: UInt32) throws -> InsightsPatternCard {
            let json = """
            {"kind":"retried_calls","tokens":\(tokens),"count":\(count),"headline":"\(headline)","files":null,"sessions":1,
             "basis":"estimate_from_result_size","inferred":false,
             "weeks":[{"week_start":"2026-10-05","tokens":12000},{"week_start":"2026-10-12","tokens":\(tokens)}],
             "change":null,"change_unavailable":null}
            """
            return try JSONDecoder().decode(InsightsPatternCard.self, from: Data(json.utf8))
        }
        // Tokens Some(0), count 1: the headline is the count, and no "0" sits above the bar.
        let counted = try card(headline: "count", tokens: "0", count: 1)
        XCTAssertTrue(InsightsPatternsWords.leadsWithCount(counted))
        XCTAssertNil(InsightsPatternsWords.currentWeekFigure(counted, copy: copy))
        // A token headline keeps the figure on the current week.
        let tokens = try card(headline: "tokens", tokens: "20000", count: 3)
        XCTAssertEqual(InsightsPatternsWords.currentWeekFigure(tokens, copy: copy),
                       InsightsOverviewWords.figure(20_000, copy: copy))
        // The view draws that text and nothing else on the current bar.
        let view = try InsightsGlassConventionsTests.text("Views/InsightsPatternsTab.swift")
        XCTAssertTrue(view.contains("InsightsPatternsWords.currentWeekFigure(card, copy: copy)"))
        XCTAssertFalse(view.contains("Text(figure(tokens)).insightsTick(current: true)"))
    }

    func testNoWeekIsComparedUnderTheSavedFeed() throws {
        let week = try patterns()
        for card in week.cards {
            XCTAssertEqual(InsightsPatternsWords.change(card, copy: Self.words), "\u{2014}")
        }
    }

    func testTheOtherHarnessLineIsFilled() throws {
        let week = try patterns()
        XCTAssertEqual(InsightsPatternsWords.shown(week.cards).map(\.kind), ["retried_calls"],
                       "repeated reads, edit after a failed command and long context are not shown")
        XCTAssertEqual(InsightsPatternsWords.claudeOnlyLine(week, copy: Self.words), "Claude only: 1 of 2.")
        XCTAssertEqual(InsightsPatternsWords.seeSessions(week.cards[0], copy: Self.words), "Show 1 traces")
    }

    @MainActor
    func testPatternsModelAsksForTheWeekInTheLocalOffsetAndListsSessions() async throws {
        let recorder = PatternsRecorder()
        let model = InsightsPatternsModel(service: { try await recorder.call($0) })
        model.open()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.patterns?.week_start, "2026-09-14")
        var requests = await recorder.requests
        XCTAssertEqual(requests.map(\.operation.type), ["patterns"])
        XCTAssertNil(requests[0].operation.week_start)
        XCTAssertNil(requests[0].operation.weeks, "the core's six bars by default")
        XCTAssertEqual(requests[0].operation.tz, Int32(TimeZone.current.secondsFromGMT()))

        model.showSessions("edit_fail_edit")
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        requests = await recorder.requests
        XCTAssertEqual(requests.last?.operation.type, "pattern_sessions")
        XCTAssertEqual(requests.last?.operation.pattern, "edit_fail_edit")
        XCTAssertEqual(requests.last?.operation.week_start, "2026-09-14")
        XCTAssertEqual(model.sessions?.pattern, "edit_fail_edit")
        XCTAssertNil(model.sessions?.sessions.first?.tokens)

        model.selectWeek("2026-09-07")
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        requests = await recorder.requests
        XCTAssertEqual(requests.last?.operation.week_start, "2026-09-07")
        XCTAssertNil(model.sessions, "a new week closes the old session list")

        await recorder.fail()
        model.reload()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertNil(model.patterns, "a failed read never keeps the old figures")
        XCTAssertTrue(model.failed)
        model.close()
    }

    @MainActor
    func testPatternsOverTheRealCoreReadAnAbsentStoreWithoutCreatingIt() async throws {
        let store = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let model = InsightsPatternsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        model.open()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.failed)
        XCTAssertEqual(model.patterns?.feed, "saved")
        XCTAssertEqual(model.patterns?.cards.count, 4)
        XCTAssertEqual(model.patterns?.cards.first?.weeks.count, 6)
        XCTAssertTrue(model.patterns?.cards.allSatisfy { $0.tokens == nil } ?? false)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.close()
    }
}

private actor PatternsRecorder {
    var requests: [InsightsRequest] = []
    private var failing = false
    func fail() { failing = true }
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        requests.append(request)
        if failing { throw InsightsError.service("insights-operation-failed") }
        let json: String
        if request.operation.type == "pattern_sessions" {
            json = """
            {"type":"pattern_sessions","pattern_sessions":{"pattern":"\(request.operation.pattern ?? "")",
             "feed":"saved","generation":7,"week_start":"2026-09-14","tz":0,
             "sessions":[{"session_ref":"abc","count":1,"tokens":null,"state":"known","reasons":[]}]}}
            """
        } else {
            json = InsightsPatternsTests.patternsJSON.replacingOccurrences(
                of: "\"2026-09-14\",\"week_end\"",
                with: "\"\(request.operation.week_start ?? "2026-09-14")\",\"week_end\"")
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}

/// A session row's words: a saved session is labelled by its first event,
/// harness and tokens, never by its snapshot ID.
final class InsightsSessionsTests: XCTestCase {
    static let words: [String: String] = [
        "analytics_unavailable": "\u{2014}",
        "claude_code": "CLAUDE",
        "codex": "CODEX",
        "analytics_session_label": "{date} {time} | {harness} | {t} tokens",
        "analytics_session_label_undated": "Undated | {harness} | {t} tokens",
    ]

    /// A saved snapshot as the core lists it. `earliest` is its first
    /// recorded event; `.some(nil)` is time evidence with no valid timestamp.
    static func snapshot(_ id: String, source: String = "claude_code",
                         earliest: String?? = .some("2026-10-03T14:05:09.123Z")) throws -> LocalInsight {
        try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(
            withJSONObject: snapshotJSON(id, source: source, earliest: earliest)))
    }

    static func snapshotJSON(_ id: String, source: String = "claude_code",
                             earliest: String?? = .some("2026-10-03T14:05:09.123Z")) -> [String: Any] {
        var json: [String: Any] = ["id": id, "source_format": source, "boundary": "local",
            "analyzed_at": "2026-10-08T09:00:00Z", "cost_unavailable_reason": "unknown",
            "report": ["schema_version": 1,
                       "provider": ["id": "local", "version": "1", "rubric_version": "1", "execution_mode": "local"],
                       "metrics": [], "evidence": []]]
        switch earliest {
        case nil: json["time_evidence"] = NSNull()
        case .some(nil):
            json["time_evidence"] = ["schema_version": 1, "valid_timestamps": 0, "earliest": NSNull(), "latest": NSNull()]
        case .some(let at?):
            json["time_evidence"] = ["schema_version": 1, "valid_timestamps": 2,
                                     "earliest": ["recorded_at": at, "event_refs": [["record_index": 1]],
                                                  "omitted_event_refs": 0],
                                     "latest": ["recorded_at": "2026-10-03T16:15:00Z",
                                                "event_refs": [["record_index": 9]], "omitted_event_refs": 0]]
        }
        return json
    }

    static let snapshotID = String(repeating: "ab", count: 32)

    func testASavedSessionIsLabelledByItsFirstEventHarnessAndTokens() throws {
        let copy = Self.words
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let insight = try Self.snapshot(Self.snapshotID)
        let label = InsightsSessionsWords.label(insight, tokens: 1_100_000, copy: copy, timeZone: utc)
        let first = try Date("2026-10-03T14:05:09.123Z", strategy: .iso8601.year().month().day()
            .time(includingFractionalSeconds: true))
        let date = first.formatted(Date.FormatStyle(timeZone: utc).month(.abbreviated).day())
        let time = first.formatted(Date.FormatStyle(timeZone: utc).hour().minute())
        XCTAssertEqual(label, date + " " + time + " | CLAUDE | "
                       + InsightsOverviewWords.figure(1_100_000, copy: copy) + " tokens")
        XCTAssertFalse(label.contains(Self.snapshotID), label)
        XCTAssertFalse(label.contains("2026-10-03"), "the date is formatted, not the wire value")
        // Read in the offset the app sends the core, so a late event lands
        // on the local day the drill header names.
        let east = try XCTUnwrap(TimeZone(secondsFromGMT: 10 * 3600))
        XCTAssertTrue(InsightsSessionsWords.label(insight, copy: copy, timeZone: east)
            .hasPrefix(first.formatted(Date.FormatStyle(timeZone: east).month(.abbreviated).day()) + " "))
        XCTAssertNotEqual(InsightsSessionsWords.label(insight, copy: copy, timeZone: east),
                          InsightsSessionsWords.label(insight, copy: copy, timeZone: utc))
        // A whole-second timestamp reads too.
        let whole = try Self.snapshot(Self.snapshotID, earliest: "2026-10-03T14:05:09Z")
        XCTAssertEqual(InsightsSessionsWords.label(whole, tokens: 1_100_000, copy: copy, timeZone: utc), label)
        // An unknown figure is the dash, never zero.
        XCTAssertTrue(InsightsSessionsWords.label(insight, copy: copy, timeZone: utc)
            .hasSuffix(" | CLAUDE | \u{2014} tokens"))
    }

    func testASessionWithNoRecordedTimeUsesTheUndatedLabel() throws {
        let copy = Self.words
        for insight in [try Self.snapshot("s-1", source: "codex", earliest: nil),
                        try Self.snapshot("s-1", source: "codex", earliest: .some(nil))] {
            XCTAssertEqual(InsightsSessionsWords.label(insight, copy: copy), "Undated | CODEX | \u{2014} tokens")
            XCTAssertEqual(InsightsSessionsWords.label(insight, tokens: 0, copy: copy),
                           "Undated | CODEX | 0 tokens", "a measured zero stays zero")
        }
    }

    func testADrillRowIsLabelledFromItsSnapshotOrShowsTheDash() throws {
        let copy = Self.words
        let snapshots = [try Self.snapshot(Self.snapshotID)]
        XCTAssertEqual(InsightsSessionsWords.rowLabel(Self.snapshotID, tokens: 42, snapshots: snapshots, copy: copy),
                       InsightsSessionsWords.label(snapshots[0], tokens: 42, copy: copy))
        let missing = String(repeating: "ef", count: 32)
        XCTAssertEqual(InsightsSessionsWords.rowLabel(missing, tokens: 42, snapshots: snapshots, copy: copy),
                       "\u{2014}", "an unmatched row shows the dash, never its ID")
        XCTAssertEqual(InsightsSessionsWords.rowLabel("", tokens: nil, snapshots: snapshots, copy: copy), "\u{2014}")
    }

    /// The label reads the core's own time evidence: a saved Codex rollout
    /// listed by the real core is dated by its first record, and one whose
    /// records carry no time is undated.
    func testTheRealCoresSavedListLabelsBothShapes() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let dated = root.appendingPathComponent("dated.jsonl")
        let undated = root.appendingPathComponent("undated.jsonl")
        try Data("""
        {"timestamp":"2026-10-03T14:05:09.123Z","type":"session_meta","payload":{}}
        {"timestamp":"2026-10-03T14:05:09.123Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"dated"}]}}
        {"timestamp":"2026-10-03T16:15:00.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"later"}]}}
        """.utf8).write(to: dated)
        try Data("""
        {"type":"session_meta","payload":{}}
        {"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"undated"}]}}
        """.utf8).write(to: undated)
        func local(_ operation: InsightsRequest.Operation) throws -> InsightsResponse {
            try TCInsights.call(.init(storeDirectory: store.path, operation: operation))
        }
        let one = try XCTUnwrap(local(.init("analyze", source: "codex", file: dated.path, save: true)).insight)
        let two = try XCTUnwrap(local(.init("analyze", source: "codex", file: undated.path, save: true)).insight)
        let listed = try XCTUnwrap(local(.init("list")).insights)
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let first = try Date("2026-10-03T14:05:09.123Z", strategy: .iso8601.year().month().day()
            .time(includingFractionalSeconds: true))
        let copy = Self.words
        let labelled = try XCTUnwrap(listed.first { $0.id == one.id })
        XCTAssertEqual(InsightsSessionsWords.label(labelled, copy: copy, timeZone: utc),
                       first.formatted(Date.FormatStyle(timeZone: utc).month(.abbreviated).day()) + " "
                       + first.formatted(Date.FormatStyle(timeZone: utc).hour().minute()) + " | CODEX | \u{2014} tokens")
        let bare = try XCTUnwrap(listed.first { $0.id == two.id })
        XCTAssertEqual(InsightsSessionsWords.label(bare, copy: copy), "Undated | CODEX | \u{2014} tokens")
    }

    @MainActor
    func testNoInsightsViewDrawsASnapshotID() throws {
        let overview = try MonitorNavigationTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertFalse(overview.contains("Text(session.session_ref)"))
        XCTAssertTrue(overview.contains("InsightsSessionsWords.rowLabel("))
    }
}

/// Feed T comparisons (owner decision D4, open): the daemon's week arrives in
/// the core's own shapes, its weekly figures go to the core unchanged, and
/// goals, the lever and the weekly summary card are the core's. Nothing is
/// compared in Swift, and nothing is shown under feed S.
final class InsightsComparisonsTests: XCTestCase {
    /// A real `insights_week` answer, recorded from the daemon's own test
    /// over the Claude fixture session (`daemon::insights_week`).
    static let daemonWeek = #"""
{"enabled":true,"feed":"counter_pass","readable":true,"updated_at":"2026-09-14T12:00:00+00:00","sessions_stored":1,"iso_week":"2026-W38","week_start":"2026-09-14","comparable":false,"unavailable":"below_coverage_floor","change_vs_last_week":[{"source":"claude_code","permille":null,"unavailable":"below_coverage_floor"}],"rollup":{"feed":"counter_pass","week_start":"2026-09-14","coverage":{"known":0,"partial":1,"unknown":0,"reasons":{"some_turns_unknown":1}},"undated_sessions":0,"sources":[{"source":"claude_code","sessions":1,"tokens":37235,"largest_session_tokens":37235,"cache_share":{"numerator":24500,"denominator":37115}}],"by_day":[{"date":"2026-09-14","uncached":15,"cache_read":24500,"cache_write":12600,"output":120},{"date":"2026-09-15","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-16","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-17","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-18","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-19","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-20","uncached":0,"cache_read":0,"cache_write":0,"output":0}],"codex_interval_tokens":null,"by_model":[{"label":"claude-fixture-model","tokens":37235}],"sessions":[{"cache_share":{"numerator":24500,"denominator":37115},"source":"claude_code","tokens":37235,"state":"partial","reasons":["some_turns_unknown"]}]},"overview":{"feed":"counter_pass","generation":0,"week_start":"2026-09-14","week_end":"2026-09-20","tz":0,"coverage":{"known":0,"partial":1,"unknown":0,"reasons":{"some_turns_unknown":1}},"undated_sessions":0,"sessions":1,"sources":[{"source":"claude_code","sessions":1,"tokens":37235,"cache_share":{"numerator":24500,"denominator":37115,"permille":660},"largest_session":{"tokens":37235},"change_permille":null,"change":"below_coverage_floor","best":null,"best_week":"below_coverage_floor"}],"by_day":[{"date":"2026-09-14","uncached":15,"cache_read":24500,"cache_write":12600,"output":120},{"date":"2026-09-15","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-16","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-17","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-18","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-19","uncached":0,"cache_read":0,"cache_write":0,"output":0},{"date":"2026-09-20","uncached":0,"cache_read":0,"cache_write":0,"output":0}],"codex_interval_tokens":null,"by_model":[{"label":"claude-fixture-model","tokens":37235}],"by_tool":[{"source":"claude_code","tokens":37235}],"by_project":"held_for_project_decision","weeks":["2026-09-14"]},"patterns":{"feed":"counter_pass","generation":0,"week_start":"2026-09-14","week_end":"2026-09-20","tz":0,"coverage":{"known":0,"partial":1,"unknown":0,"reasons":{"some_turns_unknown":1}},"sessions":1,"claude_sessions":1,"claude_only":false,"long_context_threshold":200000,"cards":[{"kind":"repeated_reads","tokens":0,"count":0,"files":0,"sessions":0,"basis":"estimate_from_result_size","inferred":false,"weeks":[{"week_start":"2026-08-10","tokens":null},{"week_start":"2026-08-17","tokens":null},{"week_start":"2026-08-24","tokens":null},{"week_start":"2026-08-31","tokens":null},{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":null}],"change":null,"change_unavailable":"below_coverage_floor"},{"kind":"retried_calls","tokens":0,"count":0,"files":null,"sessions":0,"basis":"estimate_from_result_size","inferred":false,"weeks":[{"week_start":"2026-08-10","tokens":null},{"week_start":"2026-08-17","tokens":null},{"week_start":"2026-08-24","tokens":null},{"week_start":"2026-08-31","tokens":null},{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":null}],"change":null,"change_unavailable":"below_coverage_floor"},{"kind":"edit_fail_edit","tokens":0,"count":0,"files":null,"sessions":0,"basis":"estimate_from_result_size","inferred":true,"weeks":[{"week_start":"2026-08-10","tokens":null},{"week_start":"2026-08-17","tokens":null},{"week_start":"2026-08-24","tokens":null},{"week_start":"2026-08-31","tokens":null},{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":null}],"change":null,"change_unavailable":"below_coverage_floor"},{"kind":"long_context","tokens":null,"count":0,"files":null,"sessions":0,"basis":"from_counters","inferred":false,"weeks":[{"week_start":"2026-08-10","tokens":null},{"week_start":"2026-08-17","tokens":null},{"week_start":"2026-08-24","tokens":null},{"week_start":"2026-08-31","tokens":null},{"week_start":"2026-09-07","tokens":null},{"week_start":"2026-09-14","tokens":null}],"change":null,"change_unavailable":"below_coverage_floor"}],"reread_files":[],"weeks":["2026-09-14"]},"history":[{"week_start":"2026-06-22","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-06-29","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-07-06","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-07-13","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-07-20","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-07-27","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-08-03","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-08-10","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-08-17","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-08-24","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-08-31","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-09-07","comparable":false,"tokens":{},"cache_share_permille":{},"patterns":{},"sessions":0,"pattern_counts":{},"reread_files":null,"past_threshold":null},{"week_start":"2026-09-14","comparable":false,"tokens":{"claude_code":37235},"cache_share_permille":{"claude_code":660},"patterns":{"repeated_reads":0,"retried_calls":0,"edit_fail_edit":0},"sessions":1,"pattern_counts":{"repeated_reads":0,"retried_calls":0,"edit_fail_edit":0,"long_context":0},"reread_files":0,"past_threshold":null}],"recap_card_enabled":true}
"""#

    static let words: [String: String] = [
        "analytics_unavailable": "\u{2014}",
        "analytics_change_down": "\u{25BC} {p}% vs last week",
        "analytics_change_up": "\u{25B2} {p}% vs last week",
        "analytics_best_week": "Your best week \u{b7} previous best {q}%",
        "analytics_goal_cache_share": "Input read from cache \u{2265} {p}%",
        "analytics_goal_weekly_tokens": "Weekly tokens under {t}",
        "analytics_goal_met": "Met",
        "analytics_goal_not_met": "Not met",
        "analytics_goal_down_from": "Down from {x} last week.",
        "analytics_goal_up_from": "Up from {x} last week.",
        "analytics_pattern_retried_calls": "RETRIED",
        "analytics_your_week": "Your week \u{b7} {range}",
        "analytics_recap_fewer": "{n} traces \u{b7} {p}% fewer tokens than last week",
        "analytics_recap_more": "{n} traces \u{b7} {p}% more tokens than last week",
        "analytics_recap_best_cache": "Best cache week yet: {p}%. Previous best {q}%.",
        "analytics_recap_goal_met": "Goal met: {goal}.",
        "analytics_recap_pattern_up": "{pattern} up {p}% on your usual week.",
        "analytics_recap_threshold": "{n} traces passed your {threshold} threshold.",
        "claude_code": "Claude Code",
    ]

    func week() throws -> DaemonData.InsightsWeek {
        try DaemonDataDecoding.decoder().decode(DaemonData.InsightsWeek.self, from: Data(Self.daemonWeek.utf8))
    }

    func testTheDaemonsWeekDecodesIntoTheCoresOwnShapes() throws {
        let week = try week()
        XCTAssertTrue(week.showsCounterPass)
        XCTAssertEqual(week.recapCardEnabled, true)
        let overview = try XCTUnwrap(week.coreOverview)
        XCTAssertEqual(overview.feed, "counter_pass")
        XCTAssertEqual(overview.by_project, "held_for_project_decision")
        let line = try XCTUnwrap(overview.sources.first)
        XCTAssertEqual(line.tokens, 37235)
        XCTAssertNil(line.change_permille)
        XCTAssertEqual(line.change, "below_coverage_floor")
        XCTAssertNil(line.best)
        XCTAssertNil(line.largest_session?.session_ref, "no session reference crosses")
        XCTAssertEqual(InsightsOverviewWords.change(line, copy: Self.words), "\u{2014}")
        let patterns = try XCTUnwrap(week.corePatterns)
        XCTAssertEqual(patterns.feed, "counter_pass")
        XCTAssertEqual(patterns.cards.count, 4)
        let history = try XCTUnwrap(week.coreHistory)
        XCTAssertEqual(history.count, 13)
        XCTAssertEqual(history.last?.tokens["claude_code"], 37235)
        XCTAssertEqual(history.last?.sessions, 1)
        XCTAssertNil(history.first?.reread_files)
        XCTAssertEqual(history.first?.tokens, [:], "no figure is absent, never zero")
    }

    func testTheWeeklyFiguresGoToTheCoreUnchanged() throws {
        let history = try XCTUnwrap(try week().coreHistory)
        let request = InsightsRequest(operation: .init("comparisons", tz: 0, counterWeeks: history,
                                                       recapCardEnabled: true))
        let sent = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]
        let operation = try XCTUnwrap(sent?["operation"] as? [String: Any])
        let weeks = try XCTUnwrap(operation["counter_weeks"] as? [[String: Any]])
        let original = try XCTUnwrap(
            (try JSONSerialization.jsonObject(with: Data(Self.daemonWeek.utf8)) as? [String: Any])?["history"]
                as? [[String: Any]])
        XCTAssertEqual(weeks.count, original.count)
        XCTAssertEqual(weeks.last?["week_start"] as? String, original.last?["week_start"] as? String)
        // Counts stay integers: a one is never sent as true.
        let encoded = String(decoding: try JSONEncoder().encode(history.last), as: UTF8.self)
        XCTAssertTrue(encoded.contains(#""sessions":1"#), encoded)
        XCTAssertEqual(operation["recap_card_enabled"] as? Bool, true)
    }

    @MainActor
    func testTheRealCoreComparesTheDaemonsWeeksWithoutCreatingAStore() async throws {
        let store = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let model = InsightsComparisonsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        let week = try week()
        model.load(counterWeeks: week.coreHistory, weekStart: week.weekStart, recapCardEnabled: true)
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.failed)
        let found = try XCTUnwrap(model.comparisons)
        XCTAssertEqual(found.feed, "counter_pass")
        XCTAssertEqual(found.week_start, "2026-09-14")
        XCTAssertEqual(found.goals, [])
        // One thin week: below the floor, so no lever.
        XCTAssertNil(found.lever.pick)
        XCTAssertEqual(found.lever.unavailable, "below_coverage_floor")
        XCTAssertNil(found.recap)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
    }

    @MainActor
    func testFeedSComparesNothingAndAsksTheCoreNothing() async throws {
        let recorder = ComparisonsRecorder()
        let model = InsightsComparisonsModel(service: { try await recorder.call($0) })
        model.load(counterWeeks: nil, weekStart: "2026-10-05", recapCardEnabled: true)
        model.notUseful(); model.addGoal(InsightsGoal(kind: "weekly_tokens_under", source: "codex", tokens: 5))
        for _ in 0..<20 { await Task.yield() }
        let requests = await recorder.requests
        XCTAssertTrue(requests.isEmpty)
        XCTAssertNil(model.comparisons)
    }

    @MainActor
    func testWritesCarryRuleIDsAndWeeksAndThenReread() async throws {
        let recorder = ComparisonsRecorder()
        let model = InsightsComparisonsModel(service: { try await recorder.call($0) })
        let history = try XCTUnwrap(try week().coreHistory)
        model.load(counterWeeks: history, weekStart: "2026-09-14", recapCardEnabled: false)
        for _ in 0..<500 where model.busy || model.comparisons == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        var requests = await recorder.requests
        XCTAssertEqual(requests.map(\.operation.type), ["comparisons"])
        XCTAssertEqual(requests[0].operation.counter_weeks, history)
        XCTAssertEqual(requests[0].operation.week_start, "2026-09-14")
        XCTAssertEqual(requests[0].operation.recap_card_enabled, false)

        model.notUseful()
        for _ in 0..<500 {
            if await recorder.requests.count >= 3 { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        requests = await recorder.requests
        XCTAssertEqual(requests[1].operation.type, "lever_feedback")
        XCTAssertEqual(requests[1].operation.kind, "repeated_reads")
        XCTAssertEqual(requests[1].operation.week_start, "2026-09-14")
        XCTAssertEqual(requests[1].operation.action, "not_useful")
        XCTAssertEqual(requests[2].operation.type, "comparisons", "a write is followed by a fresh read")

        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.openRecap(), "2026-09-07")
        for _ in 0..<500 {
            if await recorder.requests.count >= 4 { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        requests = await recorder.requests
        XCTAssertEqual(requests[3].operation.type, "recap_opened")
        XCTAssertEqual(requests[3].operation.week_start, "2026-09-07")
    }

    func testChangeAndBestWeekAreTheCoresFigures() throws {
        let json = """
        {"source":"claude_code","sessions":6,"tokens":800,"cache_share":null,"largest_session":null,
         "change_permille":-204,"change":null,"best":{"previous_best_permille":396,"is_new_best":true},
         "best_week":null}
        """
        let line = try JSONDecoder().decode(InsightsWeekSource.self, from: Data(json.utf8))
        XCTAssertEqual(InsightsOverviewWords.change(line, copy: Self.words), "\u{25BC} 20% vs last week")
        XCTAssertEqual(InsightsOverviewWords.changeLine(5, copy: Self.words), "\u{25B2} 1% vs last week")
        XCTAssertEqual(InsightsOverviewWords.bestWeek(line, copy: Self.words), "Your best week \u{b7} previous best 40%")
        XCTAssertEqual(InsightsOverviewWords.bestTick(line), 396)
    }

    func testTheWeekPickerAsksTheDaemonForAnISOWeek() {
        XCTAssertEqual(InsightsOverviewWords.isoWeek("2026-10-05"), "2026-W41")
        XCTAssertEqual(InsightsOverviewWords.isoWeek("2025-12-29"), "2026-W01")
        XCTAssertNil(InsightsOverviewWords.isoWeek("last week"))
    }

    func testGoalWordsAreTheCoresFilled() throws {
        let words = Self.words
        let share = InsightsGoal(kind: "cache_share_at_least", source: "claude_code", permille: 600)
        XCTAssertEqual(InsightsComparisonsWords.goal(share, copy: words),
                       "Claude Code \u{b7} Input read from cache \u{2265} 60%")
        XCTAssertEqual(InsightsComparisonsWords.goal(InsightsGoal(kind: "weekly_tokens_under", source: "claude_code",
                                                                  tokens: 50_000), copy: words),
                       "Claude Code \u{b7} Weekly tokens under 50K")
        XCTAssertEqual(InsightsComparisonsWords.goalKinds, ["cache_share_at_least", "weekly_tokens_under"],
                       "repeated reads and long context goals left Insights")
        XCTAssertEqual(InsightsComparisonsWords.newGoal(kind: "cache_share_at_least", source: "codex", number: "60"),
                       InsightsGoal(kind: "cache_share_at_least", source: "codex", permille: 600))
        XCTAssertNil(InsightsComparisonsWords.newGoal(kind: "cache_share_at_least", source: "codex", number: "101"))
        XCTAssertNil(InsightsComparisonsWords.newGoal(kind: "weekly_tokens_under", source: "codex", number: "0"))
        XCTAssertNil(InsightsComparisonsWords.newGoal(kind: "weekly_tokens_under", source: "codex", number: "lots"))
        XCTAssertEqual(InsightsComparisonsWords.newGoal(kind: "weekly_tokens_under", source: "codex", number: "9000"),
                       InsightsGoal(kind: "weekly_tokens_under", source: "codex", tokens: 9000))
        XCTAssertEqual(InsightsComparisonsWords.mark("met", copy: words), "Met")
        XCTAssertEqual(InsightsComparisonsWords.mark("not_met", copy: words), "Not met")
        XCTAssertEqual(InsightsComparisonsWords.mark("no_figure", copy: words), "\u{2014}")

        let view = try JSONDecoder().decode(InsightsGoalView.self, from: Data("""
        {"id":"goal-1","goal":{"kind":"weekly_tokens_under","source":"claude_code","tokens":900000},
         "figure":800000,"marks":{"marks":["no_figure","no_figure","no_figure","no_figure","not_met","met"],
         "change":{"direction":"down","from":1000000}},"unavailable":null}
        """.utf8))
        XCTAssertEqual(InsightsComparisonsWords.goalFigure(view, copy: words), "800K")
        XCTAssertEqual(InsightsComparisonsWords.goalChange(view, copy: words), "Down from 1M last week.")
    }

    func testTheLeverIsAnObservationInTheCoresWords() throws {
        let retried = try JSONDecoder().decode(InsightsLeverView.self, from: Data(
            #"{"kind":"retried_calls","tokens":120000,"count":3,"files":null,"ratio_permille":1500}"#.utf8))
        XCTAssertEqual(InsightsComparisonsWords.leverLines(retried, copy: Self.words), ["RETRIED", "120K"])
    }

    func testTheSummaryCardIsTheCoresItemsFilled() throws {
        let recap = try JSONDecoder().decode(InsightsRecap.self, from: Data("""
        {"week_start":"2026-09-28","week_end":"2026-10-04","sessions":6,
         "sources":[{"source":"claude_code","tokens":800000,"change_permille":-200},
                    {"source":"codex","tokens":10,"change_permille":null}],
         "items":[{"kind":"best_cache_week","source":"claude_code","permille":450,"previous_best_permille":300},
                  {"kind":"goal","id":"goal-1","goal":{"kind":"weekly_tokens_under","source":"claude_code","tokens":50000},"met":true},
                  {"kind":"pattern_up","pattern":"repeated_reads","up_permille":1500},
                  {"kind":"pattern_up","pattern":"retried_calls","up_permille":500},
                  {"kind":"from_a_newer_core"}],
         "past_threshold":null}
        """.utf8))
        let words = Self.words
        XCTAssertEqual(InsightsComparisonsWords.recapChange(recap.sources[0], sessions: recap.sessions, copy: words),
                       "6 traces \u{b7} 20% fewer tokens than last week")
        XCTAssertNil(InsightsComparisonsWords.recapChange(recap.sources[1], sessions: recap.sessions, copy: words),
                     "no comparable week: no line, never 0%")
        XCTAssertEqual(recap.items.compactMap { InsightsComparisonsWords.recapItem($0, copy: words) }, [
            "Best cache week yet: 45%. Previous best 30%.",
            "Goal met: Claude Code \u{b7} Weekly tokens under 50K.",
            "RETRIED up 50% on your usual week.",
        ])
        XCTAssertNil(InsightsComparisonsWords.recapThreshold(recap, copy: words),
                     "no threshold line until the user sets one")
        XCTAssertTrue(InsightsComparisonsWords.recapTitle(recap, copy: words).hasPrefix("Your week \u{b7} "))
    }
}

private actor ComparisonsRecorder {
    var requests: [InsightsRequest] = []
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        requests.append(request)
        let json: String
        switch request.operation.type {
        case "comparisons":
            json = """
            {"type":"comparisons","comparisons":{"feed":"counter_pass","week_start":"2026-09-14","goals":[],
             "lever":{"pick":{"kind":"repeated_reads","tokens":300000,"count":9,"files":4,"ratio_permille":3000},
                      "disabled":[],"unavailable":null},
             "recap":{"week_start":"2026-09-07","week_end":"2026-09-13","sessions":6,"sources":[],"items":[],
                      "past_threshold":null}}}
            """
        default:
            json = """
            {"type":"\(request.operation.type)","state":{"goals":[],"lever_dismissals":[],"lever_reenables":[],
             "recap_opened_week":null}}
            """
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}
