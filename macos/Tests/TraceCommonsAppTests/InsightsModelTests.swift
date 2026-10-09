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

/// The Sessions tab's model and words: one saved session turn by turn, its
/// markers lettered by the core, an unknown turn drawn as a gap, and a Codex
/// session read as not recorded.
final class InsightsSessionsTests: XCTestCase {
    static let words: [String: String] = [
        "analytics_unavailable": "\u{2014}",
        "claude_code": "CLAUDE",
        "codex": "CODEX",
        "analytics_session_header": "{date} | {harness} | {n} turns | {t} tokens | {span} between",
        "analytics_turn": "Turn {n}",
        "analytics_codex_not_recorded": "NOT RECORDED",
        "analytics_reason_no_usage_counters": "NO COUNTERS",
        "analytics_reason_source_unsupported": "UNSUPPORTED",
        "analytics_marker_cache_rewrite": "Rewritten after {m} min",
        "analytics_marker_cache_rewrite_detail": "Turn {t} wrote {x}",
        "analytics_marker_crossed": "Passed {threshold}",
        "analytics_marker_crossed_detail": "LONG RANGE",
        "analytics_marker_shrank": "Shrank at {t}",
        "analytics_marker_inferred": "INFERRED",
        "analytics_marker_reread": "File {letter} again",
        "analytics_marker_reread_detail": "NO EDIT",
        "analytics_marker_from_tool_calls": "TOOL ORDER",
        "analytics_from_counters": "COUNTERS",
        "analytics_file_label": "File {letter} \u{b7} {ext}",
        "analytics_file_label_no_ext": "File {letter}",
    ]

    static let claudeJSON = """
    {"type":"session_drill","session":{
      "feed":"saved","session_ref":"snap-1","source":"claude_code","date":"2026-10-03","tz":0,
      "turns":3,"tokens":1100000,"span_secs":7800,"state":"partial","reasons":["some_turns_unknown"],
      "long_context_threshold":200000,
      "series":[
        {"ordinal":1,"uncached":1000,"cache_read":0,"cache_write":60000,"output":100,"context":61000},
        {"ordinal":2,"uncached":null,"cache_read":null,"cache_write":null,"output":null,"context":null},
        {"ordinal":3,"uncached":0,"cache_read":0,"cache_write":0,"output":0,"context":0}],
      "series_unavailable":null,
      "markers":[
        {"letter":"A","turn_ordinal":2,"kind":"cache_written_again","basis":"inferred_from_counters",
         "pause_minutes":14,"cache_write":64000,"context_from":null,"context":null,"file_letter":null,"file_ext":null},
        {"letter":"B","turn_ordinal":2,"kind":"re_read","basis":"from_tool_calls",
         "pause_minutes":null,"cache_write":null,"context_from":null,"context":null,"file_letter":"A","file_ext":".rs"},
        {"letter":"C","turn_ordinal":3,"kind":"context_shrank","basis":"inferred_from_counters",
         "pause_minutes":null,"cache_write":null,"context_from":90000,"context":1000,"file_letter":null,"file_ext":null},
        {"letter":"D","turn_ordinal":3,"kind":"crossed_long_context","basis":"from_counters",
         "pause_minutes":null,"cache_write":null,"context_from":null,"context":210000,"file_letter":null,"file_ext":null}]}}
    """

    static let codexJSON = """
    {"type":"session_drill","session":{
      "feed":"saved","session_ref":"snap-2","source":"codex","date":null,"tz":0,
      "turns":null,"tokens":null,"span_secs":null,"state":"unknown","reasons":["no_usage_counters"],
      "long_context_threshold":200000,"series":null,"series_unavailable":"not_recorded","markers":[]}}
    """

    func drill(_ json: String = claudeJSON) throws -> InsightsSessionDrill {
        try XCTUnwrap(JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8)).session)
    }

    func testTheHeaderIsTheCoresTemplateFilledAndAnUnknownIsTheDash() throws {
        let copy = Self.words
        let header = InsightsSessionsWords.header(try drill(), copy: copy)
        XCTAssertTrue(header.contains(" | CLAUDE | 3 turns | "), header)
        XCTAssertTrue(header.contains(InsightsOverviewWords.figure(1_100_000, copy: copy) + " tokens"), header)
        XCTAssertFalse(header.contains("{"), header)
        XCTAssertFalse(header.contains("2026-10-03"), "the date is formatted, not the wire value")
        let codex = InsightsSessionsWords.header(try drill(Self.codexJSON), copy: copy)
        XCTAssertEqual(codex, "\u{2014} | CODEX | \u{2014} turns | \u{2014} tokens | \u{2014} between")
    }

    func testAnUnknownTurnIsAGapAndNeverAZeroBar() throws {
        let segments = InsightsSessionsWords.segments(try drill(), copy: Self.words)
        XCTAssertFalse(segments.contains { $0.turn == 2 }, "an unknown turn draws nothing")
        // A measured zero is still a bar.
        XCTAssertEqual(segments.filter { $0.turn == 3 }.map(\.tokens), [0, 0, 0])
        XCTAssertEqual(segments.filter { $0.turn == 1 }.map(\.tokens), [0, 1000, 60000])
    }

    func testMarkerCardsAreTheCoresWordsWithTheirDerivationLabels() throws {
        let markers = try drill().markers
        let copy = Self.words
        let cards = markers.map { InsightsSessionsWords.markerLines($0, threshold: 200_000, copy: copy) }
        XCTAssertEqual(cards[0], ["Rewritten after 14 min",
                                  "Turn 2 wrote " + InsightsOverviewWords.figure(64_000, copy: copy), "INFERRED"])
        XCTAssertEqual(cards[1], ["File A again", "NO EDIT", "File A \u{b7} .rs", "TOOL ORDER"])
        XCTAssertEqual(cards[2], ["Shrank at 3", "INFERRED"])
        XCTAssertEqual(cards[3], ["Passed " + InsightsOverviewWords.figure(200_000, copy: copy),
                                  "LONG RANGE", "COUNTERS"])
        XCTAssertEqual(markers.map(\.letter), ["A", "B", "C", "D"])
    }

    func testACodexSessionReadsNotRecordedAndDrawsNoChart() throws {
        let codex = try drill(Self.codexJSON)
        XCTAssertEqual(InsightsSessionsWords.unavailableLine(codex, copy: Self.words), "NOT RECORDED")
        XCTAssertTrue(InsightsSessionsWords.segments(codex, copy: Self.words).isEmpty)
        XCTAssertNil(InsightsSessionsWords.unavailableLine(try drill(), copy: Self.words))
    }

    @MainActor
    func testTheSessionsModelFollowsTheSavedListAndDropsAFailedRead() async throws {
        let recorder = SessionsRecorder()
        let model = InsightsSessionsModel(service: { try await recorder.call($0) })
        model.open()
        model.sync(snapshotIDs: [])
        XCTAssertNil(model.selected)
        var requests = await recorder.requests
        XCTAssertTrue(requests.isEmpty, "no session, no read")

        model.sync(snapshotIDs: ["snap-1", "snap-2"])
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.selected, "snap-1", "the newest saved session first")
        XCTAssertEqual(model.drill?.session_ref, "snap-1")
        requests = await recorder.requests
        XCTAssertEqual(requests.map(\.operation.type), ["session_drill"])
        XCTAssertEqual(requests[0].operation.snapshot_id, "snap-1")
        XCTAssertEqual(requests[0].operation.tz, Int32(TimeZone.current.secondsFromGMT()))

        // The list changing without dropping the selection reads nothing.
        model.sync(snapshotIDs: ["snap-0", "snap-1", "snap-2"])
        XCTAssertEqual(model.selected, "snap-1")
        requests = await recorder.requests
        XCTAssertEqual(requests.count, 1)

        model.select("snap-2")
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.drill?.series_unavailable, "not_recorded")

        // A deleted selection moves to the newest left.
        model.sync(snapshotIDs: ["snap-1"])
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.selected, "snap-1")

        await recorder.fail()
        model.reload()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertNil(model.drill, "a failed read never keeps the old session")
        XCTAssertTrue(model.failed)

        model.sync(snapshotIDs: [])
        XCTAssertNil(model.selected)
        XCTAssertNil(model.drill)
        model.close()
    }

    @MainActor
    func testAnUnknownSnapshotOverTheRealCoreFailsWithoutCreatingTheStore() async throws {
        let store = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let model = InsightsSessionsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        model.open()
        model.sync(snapshotIDs: ["missing"])
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(model.failed)
        XCTAssertNil(model.drill)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.close()
    }
}

private actor SessionsRecorder {
    var requests: [InsightsRequest] = []
    private var failing = false
    func fail() { failing = true }
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        requests.append(request)
        if failing { throw InsightsError.service("insights-operation-failed") }
        let json = request.operation.snapshot_id == "snap-2"
            ? InsightsSessionsTests.codexJSON : InsightsSessionsTests.claudeJSON
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
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
        "analytics_goal_repeated_reads": "Repeated reads under {t}",
        "analytics_goal_weekly_tokens": "Weekly tokens under {t}",
        "analytics_goal_met": "Met",
        "analytics_goal_not_met": "Not met",
        "analytics_goal_down_from": "Down from {x} last week.",
        "analytics_goal_up_from": "Up from {x} last week.",
        "analytics_lever_line": "{f} files were read again {r} times, about {t} tokens.",
        "analytics_pattern_retried_calls": "RETRIED",
        "analytics_pattern_repeated_reads": "REPEATED",
        "analytics_your_week": "Your week \u{b7} {range}",
        "analytics_recap_fewer": "{n} sessions \u{b7} {p}% fewer tokens than last week",
        "analytics_recap_more": "{n} sessions \u{b7} {p}% more tokens than last week",
        "analytics_recap_best_cache": "Best cache week yet: {p}%. Previous best {q}%.",
        "analytics_recap_goal_met": "Goal met: {goal}.",
        "analytics_recap_pattern_up": "{pattern} went up {p}% against your usual week.",
        "analytics_recap_threshold": "{n} sessions passed your {threshold} threshold.",
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
        model.notUseful(); model.addGoal(InsightsGoal(kind: "repeated_reads_under", tokens: 5))
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
        XCTAssertEqual(InsightsComparisonsWords.goal(InsightsGoal(kind: "repeated_reads_under", tokens: 50_000),
                                                     copy: words), "Repeated reads under 50K")
        XCTAssertEqual(InsightsComparisonsWords.newGoal(kind: "cache_share_at_least", source: "codex", number: "60"),
                       InsightsGoal(kind: "cache_share_at_least", source: "codex", permille: 600))
        XCTAssertNil(InsightsComparisonsWords.newGoal(kind: "cache_share_at_least", source: "codex", number: "101"))
        XCTAssertNil(InsightsComparisonsWords.newGoal(kind: "repeated_reads_under", source: "codex", number: "0"))
        XCTAssertNil(InsightsComparisonsWords.newGoal(kind: "repeated_reads_under", source: "codex", number: "lots"))
        XCTAssertEqual(InsightsComparisonsWords.newGoal(kind: "repeated_reads_under", source: "codex", number: "9000"),
                       InsightsGoal(kind: "repeated_reads_under", tokens: 9000))
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
        let reads = try JSONDecoder().decode(InsightsLeverView.self, from: Data(
            #"{"kind":"repeated_reads","tokens":300000,"count":9,"files":4,"ratio_permille":3000}"#.utf8))
        XCTAssertEqual(InsightsComparisonsWords.leverLines(reads, copy: Self.words),
                       ["4 files were read again 9 times, about 300K tokens."])
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
                  {"kind":"goal","id":"goal-1","goal":{"kind":"repeated_reads_under","tokens":50000},"met":true},
                  {"kind":"pattern_up","pattern":"repeated_reads","up_permille":1500},
                  {"kind":"from_a_newer_core"}],
         "past_threshold":null}
        """.utf8))
        let words = Self.words
        XCTAssertEqual(InsightsComparisonsWords.recapChange(recap.sources[0], sessions: recap.sessions, copy: words),
                       "6 sessions \u{b7} 20% fewer tokens than last week")
        XCTAssertNil(InsightsComparisonsWords.recapChange(recap.sources[1], sessions: recap.sessions, copy: words),
                     "no comparable week: no line, never 0%")
        XCTAssertEqual(recap.items.compactMap { InsightsComparisonsWords.recapItem($0, copy: words) }, [
            "Best cache week yet: 45%. Previous best 30%.",
            "Goal met: Repeated reads under 50K.",
            "REPEATED went up 150% against your usual week.",
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
