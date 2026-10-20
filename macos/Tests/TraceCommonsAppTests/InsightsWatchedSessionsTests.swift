import Foundation
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// The Sessions tab over watched folders (feed T, owner decisions
/// 2026-10-20): while the daemon's counting is on and readable the tab is a
/// lazily loaded feed of every watched trace, newest first, each row its
/// folder, time, tool, turns, tokens and a sparkline; a row expands its drill
/// in place. Otherwise it is the saved snapshots, exactly as before, with the
/// same unavailable notice Overview shows.
final class InsightsWatchedSessionsTests: XCTestCase {
    static let words: [String: String] = InsightsSessionsTests.words.merging([
        "analytics_watched_label": "{folder} | {date} {time} | {harness} | {n} turns",
        "analytics_watched_label_no_turns": "{folder} | {date} {time} | {harness}",
        "analytics_watched_label_undated": "{folder} | Undated | {harness}",
        "analytics_folder_unknown": "UNKNOWN FOLDER",
        "analytics_feed_saved": "SAVED FEED",
        "analytics_feed_counter_pass": "WATCHED FEED",
        "insights_feed_counter_pass_unavailable": "COUNTING UNAVAILABLE",
        "analytics_session_row_tokens": "{t} TOKENS",
        "analytics_sparkline_peak": "PEAK {t} AT {n}",
        "analytics_sessions_loading_more": "LOADING",
        "analytics_sessions_retry": "RETRY",
        "analytics_sessions_page_failed": "PAGEFAILED",
    ]) { $1 }

    /// `insights_sessions` as the daemon answers it (see the daemon's
    /// `insights_sessions` tests): newest first, opaque references.
    static let listJSON = """
    {"enabled":true,"feed":"counter_pass","readable":true,"updated_at":"2026-10-19T12:00:00+00:00",
     "sessions_stored":3,"truncated":false,"next":null,"sessions":[
      {"session_ref":"t7","source":"claude_code","started_at":"2026-10-19T14:02:00+00:00","turns":119,
       "tokens":1234567,"sparkline":{"points":[10,4000,20],"peak_tokens":4000,"peak_turn":9},
       "folder":"trace-commons"},
      {"session_ref":"t5","source":"codex","started_at":"2026-10-18T09:30:00+00:00","turns":null,
       "tokens":70,"sparkline":null,"folder":null},
      {"session_ref":"t2","source":"claude_code","started_at":null,"turns":4,"tokens":null,"sparkline":null,
       "folder":"ironclaw"}]}
    """

    /// A page of `count` rows `t<first>` down, with `next` as given.
    static func pageJSON(first: Int, count: Int, next: String?, stored: Int = 7) -> String {
        let rows = (0..<count).map { i in
            #"{"session_ref":"t\#(first - i)","source":"claude_code","started_at":"2026-10-1\#(9 - i % 9)T10:00:00+00:00","turns":3,"tokens":30,"sparkline":{"points":[10,10,10],"peak_tokens":10,"peak_turn":1},"folder":"f"}"#
        }
        let nextJSON = next.map { "\"\($0)\"" } ?? "null"
        return #"{"enabled":true,"feed":"counter_pass","readable":true,"sessions_stored":\#(stored),"#
            + #""truncated":\#(next != nil),"next":\#(nextJSON),"sessions":[\#(rows.joined(separator: ","))]}"#
    }

    static func drillJSON(_ reference: String) -> String {
        InsightsSessionsTests.claudeJSON
            .replacingOccurrences(of: "\"feed\":\"saved\",\"session_ref\":\"snap-1\"",
                                  with: "\"feed\":\"counter_pass\",\"session_ref\":\"\(reference)\"")
            .replacingOccurrences(of: "{\"type\":\"session_drill\",\"session\":",
                                  with: "{\"enabled\":true,\"feed\":\"counter_pass\",\"readable\":true,\"session\":")
    }

    static func list(_ json: String = listJSON) throws -> DaemonData.InsightsSessions {
        try JSONDecoder().decode(DaemonData.InsightsSessions.self, from: Data(json.utf8))
    }

    func testTheDaemonsListDecodesAndShowsOnlyWhenReadable() throws {
        let listed = try Self.list()
        XCTAssertTrue(listed.showsCounterPass)
        XCTAssertNil(listed.next)
        XCTAssertEqual(listed.sessions?.map(\.sessionRef), ["t7", "t5", "t2"])
        XCTAssertEqual(listed.sessions?.first?.folder, "trace-commons")
        XCTAssertEqual(listed.sessions?.first?.tokens, 1_234_567)
        XCTAssertEqual(listed.sessions?.first?.sparkline,
                       DaemonData.InsightsSessionSparkline(points: [10, 4000, 20], peakTokens: 4000, peakTurn: 9))
        XCTAssertNil(listed.sessions?[1].folder)
        XCTAssertNil(listed.sessions?[1].turns)
        XCTAssertNil(listed.sessions?[1].sparkline)
        XCTAssertNil(listed.sessions?[2].tokens)
        for json in [
            #"{"enabled":false,"feed":"counter_pass"}"#,
            #"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"key_unavailable"}"#,
        ] {
            XCTAssertFalse(try Self.list(json).showsCounterPass, json)
        }
        // A daemon that predates paging and row figures still decodes: one
        // page, no tokens, no sparkline.
        let older = try Self.list("""
        {"enabled":true,"feed":"counter_pass","readable":true,"sessions_stored":1,"truncated":false,"sessions":[
         {"session_ref":"t1","source":"claude_code","started_at":null,"turns":2,"folder":null}]}
        """)
        XCTAssertTrue(older.showsCounterPass)
        XCTAssertNil(older.next)
        XCTAssertNil(older.sessions?.first?.tokens)
        XCTAssertNil(older.sessions?.first?.sparkline)
        let paged = try Self.list(Self.pageJSON(first: 7, count: 2, next: "p1.0.5"))
        XCTAssertEqual(paged.next, "p1.0.5")
        let drill = try JSONDecoder().decode(DaemonData.InsightsSessionDrillAnswer.self,
                                             from: Data(Self.drillJSON("t7").utf8))
        XCTAssertEqual(drill.coreDrill?.session_ref, "t7")
        XCTAssertEqual(drill.coreDrill?.feed, "counter_pass")
    }

    func testAWatchedTraceIsLabelledFolderTimeToolAndTurns() throws {
        let rows = try XCTUnwrap(Self.list().sessions)
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let copy = Self.words
        let first = try Date("2026-10-19T14:02:00Z", strategy: .iso8601)
        let day = first.formatted(Date.FormatStyle(timeZone: utc).weekday(.abbreviated).month(.abbreviated).day())
        let time = first.formatted(Date.FormatStyle(timeZone: utc).hour().minute())
        XCTAssertEqual(InsightsSessionsWords.watchedLabel(rows[0], copy: copy, timeZone: utc),
                       "trace-commons | \(day) \(time) | CLAUDE | 119 turns")
        // Codex records no turns; a folder no longer known reads the core's word.
        let codex = try Date("2026-10-18T09:30:00Z", strategy: .iso8601)
        XCTAssertEqual(InsightsSessionsWords.watchedLabel(rows[1], copy: copy, timeZone: utc),
                       "UNKNOWN FOLDER | "
                       + codex.formatted(Date.FormatStyle(timeZone: utc).weekday(.abbreviated).month(.abbreviated).day())
                       + " " + codex.formatted(Date.FormatStyle(timeZone: utc).hour().minute()) + " | CODEX")
        XCTAssertEqual(InsightsSessionsWords.watchedLabel(rows[2], copy: copy, timeZone: utc),
                       "ironclaw | Undated | CLAUDE")
    }

    /// A row's tokens read the core's figure; a row with no counts says so
    /// once, with no sparkline; a Codex row has its total and no sparkline.
    func testARowsTokensAndSparklineInTheCoresWords() throws {
        let rows = try XCTUnwrap(Self.list().sessions)
        let copy = Self.words
        let big = InsightsOverviewWords.figure(1_234_567, copy: copy)
        XCTAssertEqual(InsightsSessionsWords.rowTokens(rows[0], copy: copy), "\(big) TOKENS")
        XCTAssertEqual(InsightsSessionsWords.rowTokens(rows[1], copy: copy),
                       "\(InsightsOverviewWords.figure(70, copy: copy)) TOKENS")
        XCTAssertEqual(InsightsSessionsWords.rowTokens(rows[2], copy: copy), "NO COUNTERS")
        XCTAssertEqual(InsightsSessionsWords.sparklineSummary(rows[0], copy: copy),
                       "PEAK \(InsightsOverviewWords.figure(4000, copy: copy)) AT 9")
        XCTAssertNil(InsightsSessionsWords.sparklineSummary(rows[1], copy: copy))
        XCTAssertNil(InsightsSessionsWords.sparklineSummary(rows[2], copy: copy))
    }

    /// VoiceOver reads a row as one element: its label, its tokens and its
    /// sparkline's peak in words. Never its reference.
    func testARowReadsAsOneElementWithTheSparklineInWords() throws {
        let rows = try XCTUnwrap(Self.list().sessions)
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let copy = Self.words
        let label = InsightsSessionsWords.rowAccessibilityLabel(rows[0], copy: copy, timeZone: utc)
        XCTAssertEqual(label, [InsightsSessionsWords.watchedLabel(rows[0], copy: copy, timeZone: utc),
                               InsightsSessionsWords.rowTokens(rows[0], copy: copy),
                               "PEAK \(InsightsOverviewWords.figure(4000, copy: copy)) AT 9"].joined(separator: ", "))
        XCTAssertEqual(InsightsSessionsWords.rowAccessibilityLabel(rows[2], copy: copy, timeZone: utc),
                       "ironclaw | Undated | CLAUDE, NO COUNTERS")
        for row in rows {
            let read = InsightsSessionsWords.rowAccessibilityLabel(row, copy: copy, timeZone: utc)
            XCTAssertFalse(read.contains(row.sessionRef), read)
        }
    }

    /// Up and down move between rows and stop at either end; with none
    /// focused, either key starts at the first.
    func testTheKeysMoveBetweenRowsAndStopAtTheEnds() {
        let refs = ["t7", "t5", "t2"]
        XCTAssertEqual(InsightsSessionsWords.moved(nil, by: 1, in: refs), "t7")
        XCTAssertEqual(InsightsSessionsWords.moved(nil, by: -1, in: refs), "t7")
        XCTAssertEqual(InsightsSessionsWords.moved("t7", by: 1, in: refs), "t5")
        XCTAssertEqual(InsightsSessionsWords.moved("t2", by: 1, in: refs), "t2")
        XCTAssertEqual(InsightsSessionsWords.moved("t5", by: -1, in: refs), "t7")
        XCTAssertEqual(InsightsSessionsWords.moved("t7", by: -1, in: refs), "t7")
        XCTAssertEqual(InsightsSessionsWords.moved("gone", by: 1, in: refs), "t7")
        XCTAssertNil(InsightsSessionsWords.moved(nil, by: 1, in: []))
    }

    /// The sparkline spans the box: oldest on the left, 0 at the bottom, the
    /// largest point at the top; one point is a level line mid-row.
    func testTheSparklineScalesToItsLargestPoint() {
        let box = CGSize(width: 90, height: 20)
        XCTAssertEqual(InsightsSparkline.positions([], in: box), [])
        XCTAssertEqual(InsightsSparkline.positions([5], in: box), [CGPoint(x: 45, y: 0)])
        XCTAssertEqual(InsightsSparkline.positions([0, 50, 100, 25], in: box),
                       [CGPoint(x: 0, y: 20), CGPoint(x: 30, y: 10), CGPoint(x: 60, y: 0), CGPoint(x: 90, y: 15)])
        XCTAssertEqual(InsightsSparkline.positions([0, 0], in: box), [CGPoint(x: 0, y: 20), CGPoint(x: 90, y: 20)],
                       "all zero is a flat line at the bottom, never a division by zero")
    }

    @MainActor
    func testSessionsSwitchToTheWatchedFeedExpandOneRowAndBack() async throws {
        let daemon = WatchedRecorder()
        let saved = SavedRecorder()
        let model = InsightsSessionsModel(
            service: { try await saved.call($0) },
            watchedReader: { try await daemon.list($0) },
            watchedDrill: { try await daemon.drill($0) })
        model.open()
        model.sync(snapshotIDs: ["snap-1"])
        await settle(model)
        XCTAssertNil(model.watched)
        XCTAssertEqual(model.drill?.session_ref, "snap-1")

        model.showWatched(true)
        await settle(model)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t7", "t5", "t2"], "watched traces only")
        XCTAssertNil(model.selected, "no row is expanded until one is clicked")
        XCTAssertNil(model.drill)
        XCTAssertNil(model.noticeKey)
        var drills = await daemon.drills
        XCTAssertEqual(drills, [], "no drill is read for a row nobody opened")

        // The saved list changing reads nothing while watched traces show.
        model.sync(snapshotIDs: ["snap-0", "snap-1"])
        XCTAssertNil(model.selected)
        let savedReads = await saved.count
        XCTAssertEqual(savedReads, 1)

        model.toggle("t7")
        await settle(model)
        XCTAssertEqual(model.selected, "t7")
        XCTAssertEqual(model.drill?.session_ref, "t7")
        XCTAssertEqual(model.drill?.feed, "counter_pass")

        // Another row expands in its place: one at a time.
        model.toggle("t5")
        await settle(model)
        XCTAssertEqual(model.selected, "t5")
        XCTAssertEqual(model.drill?.session_ref, "t5")
        drills = await daemon.drills
        XCTAssertEqual(drills, ["t7", "t5"])

        // The list read again keeps the open row while it is listed.
        model.showWatched(true)
        await settle(model)
        XCTAssertEqual(model.selected, "t5")
        drills = await daemon.drills
        XCTAssertEqual(drills, ["t7", "t5"], "an open row is not read again")

        // Clicking the open row again closes it.
        model.toggle("t5")
        await settle(model)
        XCTAssertNil(model.selected)
        XCTAssertNil(model.drill)

        // Counting off: back to the saved snapshots, newest first.
        model.showWatched(false)
        await settle(model)
        XCTAssertNil(model.watched)
        XCTAssertEqual(model.selected, "snap-0")
        XCTAssertEqual(model.drill?.session_ref, "snap-0")
        model.close()
    }

    /// The feed reads the next page only when asked (its last rows drawn),
    /// one read at a time, never repeats a row, and stops at the end.
    @MainActor
    func testTheFeedPagesOnDemandAndStopsAtTheEnd() async throws {
        let daemon = WatchedRecorder(pages: [
            nil: Self.pageJSON(first: 9, count: 3, next: "c1"),
            "c1": Self.pageJSON(first: 6, count: 3, next: "c2"),
            // One row repeated across pages is listed once.
            "c2": Self.pageJSON(first: 4, count: 2, next: nil),
        ])
        let model = InsightsSessionsModel(
            service: { try await SavedRecorder().call($0) },
            watchedReader: { try await daemon.list($0) },
            watchedDrill: { try await daemon.drill($0) })
        model.open()
        model.showWatched(true)
        await settle(model)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7"])
        XCTAssertFalse(model.atEnd)
        var reads = await daemon.reads
        XCTAssertEqual(reads, [nil], "one page until more is asked for")

        model.loadMore()
        model.loadMore()
        XCTAssertTrue(model.loadingMore)
        await settle(model)
        XCTAssertFalse(model.loadingMore)
        reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1"], "one read at a time")
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7", "t6", "t5", "t4"])

        model.loadMore()
        await settle(model)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7", "t6", "t5", "t4", "t3"],
                       "a repeated row is listed once")
        XCTAssertTrue(model.atEnd)

        model.loadMore()
        await settle(model)
        reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", "c2"], "nothing is read past the end")

        // Reading the list again starts from the first page.
        model.reload()
        await settle(model)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7"])
        XCTAssertFalse(model.atEnd)
        model.close()
    }

    /// A row made again since its page was read (its file grew) drills to
    /// `insights-session-unknown`: the feed reads its first page again
    /// rather than leave a dead row open.
    @MainActor
    func testAnUnknownReferenceReadsTheFirstPageAgain() async throws {
        let daemon = WatchedRecorder()
        let model = InsightsSessionsModel(
            service: { try await SavedRecorder().call($0) },
            watchedReader: { try await daemon.list($0) },
            watchedDrill: { try await daemon.drill($0) })
        model.open()
        model.showWatched(true)
        await settle(model)
        await daemon.failDrills(with: .daemon(code: "bad_params", message: "insights-session-unknown"))
        model.toggle("t5")
        await settle(model)
        let reads = await daemon.reads
        XCTAssertEqual(reads, [nil, nil], "the first page is read again")
        XCTAssertNil(model.selected)
        XCTAssertNil(model.drill)
        model.close()
    }

    @MainActor
    func testAnUnreadableWatchedListFallsBackToSavedWithOverviewsNotice() async throws {
        for (answer, notice) in [
            (Answer.json(#"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"store_unreadable"}"#),
             "insights_feed_counter_pass_unavailable"),
            (Answer.json(#"{"enabled":false,"feed":"counter_pass"}"#), nil),
            (Answer.error(DaemonDataError.daemon(code: "unknown_method", message: "unknown_method")), nil),
            (Answer.error(DaemonDataError.unreachable), "insights_feed_counter_pass_unavailable"),
        ] {
            let daemon = WatchedRecorder(answer: answer)
            let saved = SavedRecorder()
            let model = InsightsSessionsModel(
                service: { try await saved.call($0) },
                watchedReader: { try await daemon.list($0) },
                watchedDrill: { try await daemon.drill($0) })
            model.open()
            model.sync(snapshotIDs: ["snap-1"])
            model.showWatched(true)
            await settle(model)
            XCTAssertNil(model.watched, "\(answer)")
            XCTAssertEqual(model.noticeKey, notice, "\(answer)")
            XCTAssertEqual(model.selected, "snap-1", "\(answer)")
            XCTAssertEqual(model.drill?.session_ref, "snap-1", "\(answer)")
            model.close()
        }
    }

    @MainActor
    func testAFailedWatchedDrillKeepsNoOtherTracesFigures() async throws {
        let daemon = WatchedRecorder()
        let model = InsightsSessionsModel(
            service: { try await SavedRecorder().call($0) },
            watchedReader: { try await daemon.list($0) },
            watchedDrill: { try await daemon.drill($0) })
        model.open()
        model.showWatched(true)
        await settle(model)
        model.toggle("t7")
        await settle(model)
        XCTAssertEqual(model.drill?.session_ref, "t7")
        await daemon.failDrills(with: .unreachable)
        model.toggle("t5")
        await settle(model)
        XCTAssertNil(model.drill)
        XCTAssertTrue(model.failed)
        XCTAssertEqual(model.selected, "t5", "the row stays open, saying it could not be read")
        model.close()
    }

    @MainActor
    private static func failingFeed(_ daemon: WatchedRecorder) -> InsightsSessionsModel {
        InsightsSessionsModel(
            service: { try await SavedRecorder().call($0) },
            watchedReader: { try await daemon.list($0) },
            watchedDrill: { try await daemon.drill($0) },
            retryBackoff: .milliseconds(1))
    }

    private static let retryPages: [String?: String] = [
        nil: pageJSON(first: 9, count: 3, next: "c1"),
        "c1": pageJSON(first: 6, count: 3, next: nil),
    ]

    /// A failed page does not end paging: Try again reads the page that
    /// failed, and the rows read so far stay.
    @MainActor
    func testARetryAfterAFailedPageLoadsTheNextPage() async throws {
        let daemon = WatchedRecorder(pages: Self.retryPages)
        await daemon.failPage("c1", times: 1)
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore()
        await settle(model)
        XCTAssertTrue(model.pageFailed)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7"], "the rows read so far stay")
        model.retry()
        await settle(model)
        XCTAssertFalse(model.pageFailed)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7", "t6", "t5", "t4"])
        XCTAssertTrue(model.atEnd)
        model.close()
    }

    /// The retry asks for the same cursor and does not read page one again.
    @MainActor
    func testARetryAsksForTheSameCursorNotThePageOne() async throws {
        let daemon = WatchedRecorder(pages: Self.retryPages)
        await daemon.failPage("c1", times: 1)
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore()
        await settle(model)
        model.retry()
        await settle(model)
        let reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", "c1"])
        model.close()
    }

    /// The last row appearing again after a failure retries once on its own;
    /// a second failure waits for the control. Nothing loops on errors.
    @MainActor
    func testAFailingPageIsNotRetriedInATightLoop() async throws {
        let daemon = WatchedRecorder(pages: Self.retryPages)
        await daemon.failPage("c1", times: 1000)
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore()
        await settle(model)
        var reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1"], "a failure alone reads nothing more")
        try? await Task.sleep(for: .milliseconds(50))
        reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1"], "no retry without the last row appearing")

        model.loadMore()   // the last row appears again
        await settle(model)
        reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", "c1"], "one automatic retry")
        XCTAssertTrue(model.pageFailed)

        for _ in 0..<20 { model.loadMore() }
        await settle(model)
        try? await Task.sleep(for: .milliseconds(50))
        reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", "c1"], "a second failure waits for the control")

        model.retry()   // the control always asks
        await settle(model)
        reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", "c1", "c1"])
        model.close()
    }

    /// A page read after the automatic retry succeeds gives the retry back.
    @MainActor
    func testAnAutomaticRetryRecoversScrolling() async throws {
        let daemon = WatchedRecorder(pages: Self.retryPages)
        await daemon.failPage("c1", times: 1)
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore(); await settle(model)
        XCTAssertTrue(model.pageFailed)
        model.loadMore(); await settle(model)
        XCTAssertFalse(model.pageFailed)
        XCTAssertEqual(model.watched?.count, 6)
        model.close()
    }

    /// A stale drill (the row's file grew, or the store dropped it) keeps the
    /// rows read so far: the stale row goes, and a fresh first page merges in
    /// above the rest.
    @MainActor
    func testAnUnknownReferenceKeepsTheLoadedRowsAndMergesAFreshFirstPage() async throws {
        let daemon = WatchedRecorder(pages: Self.retryPages)
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore(); await settle(model)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7", "t6", "t5", "t4"])
        await daemon.setPage(nil, Self.pageJSON(first: 10, count: 3, next: "c1"))
        await daemon.failDrills(with: .daemon(code: "bad_params", message: "insights-session-unknown"))
        model.toggle("t5")
        await settle(model)
        XCTAssertNil(model.selected)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t10", "t9", "t8", "t7", "t6", "t4"],
                       "the stale row is gone, the rest stay, the fresh page leads")
        XCTAssertTrue(model.atEnd, "the loaded tail's end is kept")
        model.close()
    }

    /// A page that reports another `updated_at` or `sessions_stored` than
    /// the first page did makes the feed read the first page again and merge
    /// it above the loaded rows.
    @MainActor
    func testAChangedStoreStampMergesAFreshFirstPage() async throws {
        let daemon = WatchedRecorder(pages: [
            nil: Self.pageJSON(first: 9, count: 3, next: "c1"),
            "c1": Self.pageJSON(first: 6, count: 3, next: nil, stored: 8),
        ])
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        await daemon.setPage(nil, Self.pageJSON(first: 10, count: 3, next: "c1", stored: 8))
        model.loadMore()
        await settle(model)
        let reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", nil], "the first page is read again, not the cursor")
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t10", "t9", "t8", "t7", "t6", "t5", "t4"])
        XCTAssertTrue(model.atEnd)
        // The stamp is now the new one: the end is not re-read.
        model.loadMore()
        await settle(model)
        let after = await daemon.reads
        XCTAssertEqual(after, [nil, "c1", nil])
        model.close()
    }

    /// A cursor the store no longer accepts (it started again) reads the
    /// first page and replaces the list: the old references name other rows.
    @MainActor
    func testARefusedCursorReadsTheFirstPageAgain() async throws {
        let daemon = WatchedRecorder(pages: Self.retryPages)
        await daemon.failPage("c1", times: 1,
                              error: .daemon(code: "bad_params", message: "insights-sessions-cursor-invalid"))
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore()
        await settle(model)
        let reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", nil])
        XCTAssertFalse(model.pageFailed, "a refused cursor is not a failed page")
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7"])
        model.close()
    }

    /// A page whose rows are all listed already adds nothing for the last
    /// row to appear against: the next page is asked for anyway.
    @MainActor
    func testAPageOfKnownRowsDoesNotStallPaging() async throws {
        let daemon = WatchedRecorder(pages: [
            nil: Self.pageJSON(first: 9, count: 3, next: "c1"),
            "c1": Self.pageJSON(first: 8, count: 2, next: "c2"),
            "c2": Self.pageJSON(first: 6, count: 2, next: nil),
        ])
        let model = Self.failingFeed(daemon)
        model.open(); model.showWatched(true)
        await settle(model)
        model.loadMore()
        await settle(model)
        let reads = await daemon.reads
        XCTAssertEqual(reads, [nil, "c1", "c2"])
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t9", "t8", "t7", "t6", "t5"])
        XCTAssertTrue(model.atEnd)
        model.close()
    }

    @MainActor
    func testTheTabDrawsTheFeedAndTheFeedLine() throws {
        let tab = try MonitorNavigationTests.text("Views/InsightsSessionsTab.swift")
        XCTAssertTrue(tab.contains("LazyVStack"), "drawn lazily")
        XCTAssertTrue(tab.contains("InsightsSessionsWords.rowAccessibilityLabel("))
        XCTAssertTrue(tab.contains("InsightsSparkline(points:"))
        XCTAssertTrue(tab.contains("model.loadMore()"))
        XCTAssertTrue(tab.contains("model.retry()"), "the foot offers a retry")
        XCTAssertTrue(tab.contains("analytics_sessions_retry"))
        XCTAssertTrue(tab.contains("analytics_sessions_page_failed"), "a sentence, not a bare dash")
        XCTAssertTrue(tab.contains("model.toggle("))
        XCTAssertTrue(tab.contains("InsightsSessionsWords.feedLines("))
        XCTAssertFalse(tab.contains("watchedChoices"), "no picker over watched traces")
        XCTAssertFalse(tab.contains("info: [text(\"analytics_feed_saved\")]"), "the feed line follows the drill")
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("sessionsModel.showWatched(week != nil)"))
        XCTAssertTrue(view.contains("client.insightsSessions(before: cursor, limit: InsightsSessionsModel.pageSize)"))
    }

    func testTheFeedLineFollowsTheDrillsFeed() throws {
        let copy = Self.words
        XCTAssertEqual(InsightsSessionsWords.feedLines("counter_pass", notice: "N", copy: copy), ["WATCHED FEED"])
        XCTAssertEqual(InsightsSessionsWords.feedLines("saved", notice: nil, copy: copy), ["SAVED FEED"])
        XCTAssertEqual(InsightsSessionsWords.feedLines("saved", notice: "COUNTING UNAVAILABLE", copy: copy),
                       ["SAVED FEED", "COUNTING UNAVAILABLE"])
    }

    /// The row's words are the core's: each key the feed reads is in the
    /// real copy table.
    func testTheFeedsWordsAreCoreCopy() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in ["analytics_session_row_tokens", "analytics_sparkline_peak", "analytics_sessions_loading_more",
                    "analytics_reason_no_usage_counters", "analytics_watched_label"] {
            XCTAssertFalse(copy[key, default: ""].isEmpty, key)
        }
        XCTAssertTrue(copy["analytics_sparkline_peak", default: ""].contains("{t}"))
        XCTAssertTrue(copy["analytics_sparkline_peak", default: ""].contains("{n}"))
    }

    @MainActor
    private func settle(_ model: InsightsSessionsModel) async {
        for _ in 0..<500 {
            await Task.yield()
            if !model.busy { try? await Task.sleep(for: .milliseconds(5)); if !model.busy { return } }
            try? await Task.sleep(for: .milliseconds(10))
        }
    }
}

enum Answer: CustomStringConvertible, Sendable {
    case json(String)
    case error(DaemonDataError)
    var description: String {
        switch self {
        case .json(let json): return json
        case .error(let error): return "\(error)"
        }
    }
}

private actor WatchedRecorder {
    let answer: Answer
    private(set) var pages: [String?: String]
    func setPage(_ cursor: String?, _ json: String) { pages[cursor] = json }
    var drills: [String] = []
    var reads: [String?] = []
    private var failure: DaemonDataError?
    /// Cursors whose next read throws, and how many more times.
    private var pageFailures: [String: (times: Int, error: DaemonDataError)] = [:]
    func failPage(_ cursor: String, times: Int, error: DaemonDataError = .unreachable) {
        pageFailures[cursor] = (times, error)
    }
    init(answer: Answer = .json(InsightsWatchedSessionsTests.listJSON), pages: [String?: String] = [:]) {
        self.answer = answer; self.pages = pages
    }
    func failDrills(with error: DaemonDataError) { failure = error }
    func list(_ cursor: String?) throws -> DaemonData.InsightsSessions {
        reads.append(cursor)
        if let cursor, let failing = pageFailures[cursor], failing.times > 0 {
            pageFailures[cursor] = (failing.times - 1, failing.error)
            throw failing.error
        }
        if let page = pages[cursor] { return try InsightsWatchedSessionsTests.list(page) }
        switch answer {
        case .json(let json): return try InsightsWatchedSessionsTests.list(json)
        case .error(let error): throw error
        }
    }
    func drill(_ reference: String) throws -> DaemonData.InsightsSessionDrillAnswer {
        drills.append(reference)
        if let failure { throw failure }
        return try JSONDecoder().decode(DaemonData.InsightsSessionDrillAnswer.self,
                                        from: Data(InsightsWatchedSessionsTests.drillJSON(reference).utf8))
    }
}

private actor SavedRecorder {
    var count = 0
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        count += 1
        let reference = request.operation.snapshot_id ?? ""
        let json = InsightsSessionsTests.claudeJSON
            .replacingOccurrences(of: "\"session_ref\":\"snap-1\"", with: "\"session_ref\":\"\(reference)\"")
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}
