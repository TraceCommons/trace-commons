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
        XCTAssertTrue(model.busy)
        model.close()
        try await gate.resolve("{\"type\":\"copy\",\"copy\":{\"title\":\"late\"}}")
        for _ in 0..<20 { await Task.yield() }
        XCTAssertFalse(model.busy)
        XCTAssertTrue(model.copy.isEmpty)
        XCTAssertNil(model.error)
        XCTAssertTrue(model.snapshots.isEmpty)
    }

    @MainActor
    func testInitialCopyThenListUsesOnlyLocalService() async throws {
        let recorder = Recorder()
        let model = InsightsModel(service: { request in try await recorder.call(request) })
        model.open()
        for _ in 0..<500 where model.busy || model.episodeBusy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.text("title"), "Insights")
        let operations = await recorder.operations
        XCTAssertEqual(Set(operations), ["copy", "episode_list", "list", "summary"])
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertTrue(model.snapshots.isEmpty)
        model.close()
    }
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
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        operations.append(request.operation.type)
        if request.operation.type == "summary" {
            return try TCInsights.call(.init(
                storeDirectory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
                operation: .init("summary")))
        }
        let json = switch request.operation.type {
        case "copy": "{\"type\":\"copy\",\"copy\":{\"title\":\"Insights\"}}"
        case "episode_list": "{\"type\":\"episode_list\",\"episodes\":[]}"
        default: "{\"type\":\"list\",\"insights\":[]}"
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}

extension InsightsModelTests {
    @MainActor
    func testActualModelKeepsAnalysisEphemeralAndSavesOnlyOnAction() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let file = root.appendingPathComponent("fixture.jsonl")
        try Data("{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"hello\"}\n".utf8).write(to: file)
        let model = InsightsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        func settle() async throws {
            for _ in 0..<500 {
                if !model.busy && !model.episodeBusy { return }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTFail("Local operation did not finish")
        }
        model.open(); try await settle()
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.analyze(file: file, source: "trajectory"); try await settle()
        XCTAssertNotNil(model.selected)
        XCTAssertFalse(model.selectedIsSaved)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.save(); try await settle()
        XCTAssertTrue(model.selectedIsSaved)
        XCTAssertEqual(model.snapshots.count, 1)
        XCTAssertEqual(model.summary?.saved_snapshots, 1)
        XCTAssertEqual(model.summary?.user_reported.unassessed_snapshots, 1)
        model.annotate(category: "docs", outcome: "accepted"); try await settle()
        XCTAssertEqual(model.selected?.manual_annotation?.provenance, "user_reported")
        XCTAssertEqual(model.assessmentCategory, "docs")
        XCTAssertEqual(model.assessmentOutcome, "accepted")
        XCTAssertEqual(model.summary?.user_reported.assessed_snapshots, 1)
        XCTAssertEqual(model.summary?.user_reported.unassessed_snapshots, 0)
        XCTAssertEqual(model.summary?.user_reported.outcomes.first { $0.outcome == "accepted" }?.snapshots, 1)
        let savedID = try XCTUnwrap(model.selected?.id)
        model.analyze(file: file, source: "trajectory"); try await settle()
        XCTAssertEqual(model.assessmentCategory, "unknown")
        XCTAssertEqual(model.assessmentOutcome, "unknown")
        model.explain(savedID); try await settle()
        XCTAssertEqual(model.assessmentCategory, "docs", "Explain must populate the editable assessment")
        XCTAssertEqual(model.assessmentOutcome, "accepted")
        let detail = InsightDetail(insight: try XCTUnwrap(model.selected), copy: model.copy)
        let renderer = ImageRenderer(content: detail.padding(20).frame(width: 700).background(Color.white))
        let image = try XCTUnwrap(renderer.nsImage)
        XCTAssertGreaterThan(image.size.height, 100)
        if let path = ProcessInfo.processInfo.environment["TC_INSIGHTS_RENDER_PATH"] {
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: path))
        }
        _ = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("clear_annotation", id: try XCTUnwrap(model.selected?.id))))
        model.refresh(); try await settle()
        XCTAssertNil(model.selected?.manual_annotation, "Refresh must replace the selected saved detail after external edits")
        XCTAssertEqual(model.assessmentCategory, "unknown")
        XCTAssertEqual(model.assessmentOutcome, "unknown")
        model.annotate(category: "tests", outcome: "partial"); try await settle()
        model.clearAnnotation(); try await settle()
        XCTAssertEqual(model.summary?.user_reported.unassessed_snapshots, 1)
        XCTAssertEqual(model.assessmentCategory, "unknown")
        XCTAssertEqual(model.assessmentOutcome, "unknown")
        let oldID = model.selected?.id
        let original = try String(contentsOf: file, encoding: .utf8)
        try Data(original.replacingOccurrences(of: "hello", with: "changed").utf8).write(to: file)
        model.analyze(file: file, source: "trajectory"); try await settle()
        model.save(); try await settle()
        XCTAssertEqual(model.snapshots.count, 1, "Reimport must remove the replaced cached snapshot")
        XCTAssertNotEqual(model.selected?.id, oldID)
        XCTAssertNil(model.selected?.manual_annotation)
        model.delete(); try await settle()
        XCTAssertNil(model.selected)
        XCTAssertTrue(model.snapshots.isEmpty)
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertNil(model.summary?.snapshot_analysis_range)
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
        model.close()
    }
}

/// The Overview tab's model and words: every figure is the core's, an
/// unknown figure is a dash, and nothing is re-sorted by value.
final class InsightsOverviewTests: XCTestCase {
    static let words: [String: String] = [
        "analytics_unavailable": "\u{2014}",
        "analytics_coverage_line": "Usage known for {k} of {n} sessions \u{b7} {p} partial \u{b7} {u} unknown, not counted as zero",
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
                       "Usage known for 1 of 4 sessions \u{b7} 1 partial \u{b7} 2 unknown, not counted as zero")
        XCTAssertEqual(InsightsOverviewWords.feedLines(week.feed, copy: Self.words), ["FEED_SAVED", "NEEDS_T"])
        XCTAssertEqual(InsightsOverviewWords.feedLines("counter_pass", copy: Self.words), [])
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
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.overview?.week_start, "2026-09-07")
        var requests = await recorder.requests
        XCTAssertEqual(requests.map(\.operation.type), ["week_overview"])
        XCTAssertNil(requests[0].operation.week_start)
        XCTAssertEqual(requests[0].operation.tz, Int32(TimeZone.current.secondsFromGMT()))

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
        "analytics_pattern_repeated_reads": "REPEATED",
        "analytics_pattern_repeated_reads_count": "{r} reads of {f} files",
        "analytics_pattern_retried_calls_count": "{c} calls",
        "analytics_pattern_edit_fail_edit_count": "{l} times",
        "analytics_pattern_long_context_line": "after {threshold}",
        "analytics_estimate_from_result_size": "ABOUT",
        "analytics_from_counters": "COUNTERS",
        "analytics_inferred_from_order": "INFERRED",
        "analytics_claude_sessions_only": "Claude only: {k} of {n}.",
        "analytics_see_sessions": "See {n} sessions",
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
        let lines = week.cards.map { InsightsPatternsWords.countLine($0, threshold: week.long_context_threshold, copy: copy) }
        XCTAssertEqual(lines, ["3 reads of 2 files", "0 calls", "1 times",
                              "after " + InsightsOverviewWords.figure(200_000, copy: copy)])
        XCTAssertEqual(InsightsPatternsWords.title("repeated_reads", copy: copy), "REPEATED")
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

    func testNoWeekIsComparedUnderTheSavedFeed() throws {
        let week = try patterns()
        for card in week.cards {
            XCTAssertEqual(InsightsPatternsWords.change(card, copy: Self.words), "\u{2014}")
        }
    }

    func testFilesAreLettersWithTheirExtensionAndTheOtherHarnessLineIsFilled() throws {
        let week = try patterns()
        XCTAssertEqual(week.reread_files.map { InsightsPatternsWords.fileLabel($0, copy: Self.words) },
                       ["File B \u{b7} .rs", "File A"])
        XCTAssertEqual(InsightsPatternsWords.claudeOnlyLine(week, copy: Self.words), "Claude only: 1 of 2.")
        XCTAssertEqual(InsightsPatternsWords.seeSessions(week.cards[0], copy: Self.words), "See 1 sessions")
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
