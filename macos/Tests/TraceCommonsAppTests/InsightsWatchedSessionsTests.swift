import Foundation
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// The Sessions tab over watched folders (feed T, owner decisions
/// 2026-10-20): while the daemon's counting is on and readable the picker
/// lists watched traces only, labelled folder, time, tool and turns, and each
/// drills through the daemon; otherwise it is the saved snapshots, exactly as
/// before, with the same unavailable notice Overview shows.
final class InsightsWatchedSessionsTests: XCTestCase {
    static let words: [String: String] = InsightsSessionsTests.words.merging([
        "analytics_watched_label": "{folder} | {date} {time} | {harness} | {n} turns",
        "analytics_watched_label_no_turns": "{folder} | {date} {time} | {harness}",
        "analytics_watched_label_undated": "{folder} | Undated | {harness}",
        "analytics_folder_unknown": "UNKNOWN FOLDER",
        "analytics_feed_saved": "SAVED FEED",
        "analytics_feed_counter_pass": "WATCHED FEED",
        "insights_feed_counter_pass_unavailable": "COUNTING UNAVAILABLE",
    ]) { $1 }

    /// `insights_sessions` as the daemon answers it (see the daemon's
    /// `insights_sessions` tests): newest first, opaque references.
    static let listJSON = """
    {"enabled":true,"feed":"counter_pass","readable":true,"updated_at":"2026-10-19T12:00:00+00:00",
     "sessions_stored":3,"truncated":false,"sessions":[
      {"session_ref":"t7","source":"claude_code","started_at":"2026-10-19T14:02:00+00:00","turns":119,
       "folder":"trace-commons"},
      {"session_ref":"t5","source":"codex","started_at":"2026-10-18T09:30:00+00:00","turns":null,
       "folder":null},
      {"session_ref":"t2","source":"claude_code","started_at":null,"turns":4,"folder":"ironclaw"}]}
    """

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
        XCTAssertEqual(listed.sessions?.map(\.sessionRef), ["t7", "t5", "t2"])
        XCTAssertEqual(listed.sessions?.first?.folder, "trace-commons")
        XCTAssertNil(listed.sessions?[1].folder)
        XCTAssertNil(listed.sessions?[1].turns)
        for json in [
            #"{"enabled":false,"feed":"counter_pass"}"#,
            #"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"key_unavailable"}"#,
        ] {
            XCTAssertFalse(try Self.list(json).showsCounterPass, json)
        }
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
        let choices = InsightsSessionsWords.watchedChoices(rows, copy: copy)
        XCTAssertEqual(choices.map(\.value), ["t7", "t5", "t2"], "valued by the opaque reference, in order")
        for choice in choices { XCTAssertFalse(choice.title.contains(choice.value), choice.title) }
    }

    @MainActor
    func testSessionsSwitchToWatchedTracesAndBack() async throws {
        let daemon = WatchedRecorder()
        let saved = SavedRecorder()
        let model = InsightsSessionsModel(
            service: { try await saved.call($0) },
            watchedReader: { try await daemon.list() },
            watchedDrill: { try await daemon.drill($0) })
        model.open()
        model.sync(snapshotIDs: ["snap-1"])
        await settle(model)
        XCTAssertNil(model.watched)
        XCTAssertEqual(model.drill?.session_ref, "snap-1")

        model.showWatched(true)
        await settle(model)
        XCTAssertEqual(model.watched?.map(\.sessionRef), ["t7", "t5", "t2"], "watched traces only")
        XCTAssertEqual(model.selected, "t7", "the newest watched trace first")
        XCTAssertEqual(model.drill?.session_ref, "t7")
        XCTAssertEqual(model.drill?.feed, "counter_pass")
        XCTAssertNil(model.noticeKey)
        var drills = await daemon.drills
        XCTAssertEqual(drills, ["t7"])

        // The saved list changing reads nothing while watched traces show.
        model.sync(snapshotIDs: ["snap-0", "snap-1"])
        XCTAssertEqual(model.selected, "t7")
        let savedReads = await saved.count
        XCTAssertEqual(savedReads, 1)

        model.select("t5")
        await settle(model)
        XCTAssertEqual(model.drill?.session_ref, "t5")
        drills = await daemon.drills
        XCTAssertEqual(drills, ["t7", "t5"])

        // The week read again keeps the picked trace while it is listed.
        model.showWatched(true)
        await settle(model)
        XCTAssertEqual(model.selected, "t5")

        // Counting off: back to the saved snapshots, newest first.
        model.showWatched(false)
        await settle(model)
        XCTAssertNil(model.watched)
        XCTAssertEqual(model.selected, "snap-0")
        XCTAssertEqual(model.drill?.session_ref, "snap-0")
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
                watchedReader: { try await daemon.list() },
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
            watchedReader: { try await daemon.list() },
            watchedDrill: { try await daemon.drill($0) })
        model.open()
        model.showWatched(true)
        await settle(model)
        XCTAssertEqual(model.drill?.session_ref, "t7")
        await daemon.failDrills()
        model.select("t5")
        await settle(model)
        XCTAssertNil(model.drill)
        XCTAssertTrue(model.failed)
        model.close()
    }

    @MainActor
    /// The first trace shown is the newest dated one, even from a daemon
    /// that lists an undated trace first; with none dated, the first.
    func testTheDefaultTraceIsTheNewestDatedOne() throws {
        let rows = try XCTUnwrap(Self.list().sessions)
        XCTAssertEqual(InsightsSessionsWords.defaultWatched(rows)?.sessionRef, "t7")
        let undatedFirst = [rows[2], rows[0], rows[1]]
        XCTAssertEqual(InsightsSessionsWords.defaultWatched(undatedFirst)?.sessionRef, "t7")
        XCTAssertEqual(InsightsSessionsWords.defaultWatched([rows[2]])?.sessionRef, "t2")
        XCTAssertNil(InsightsSessionsWords.defaultWatched([]))
        let model = try MonitorNavigationTests.text("InsightsModel.swift")
        XCTAssertTrue(model.contains("} else if let newest = InsightsSessionsWords.defaultWatched(rows) {"))
    }

    @MainActor
    func testTheTabDrawsWatchedChoicesAndTheFeedLine() throws {
        let tab = try MonitorNavigationTests.text("Views/InsightsSessionsTab.swift")
        XCTAssertTrue(tab.contains("InsightsSessionsWords.watchedChoices("))
        XCTAssertTrue(tab.contains("InsightsSessionsWords.feedLines("))
        XCTAssertFalse(tab.contains("info: [text(\"analytics_feed_saved\")]"), "the feed line follows the drill")
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("sessionsModel.showWatched(week != nil)"))
    }

    func testTheFeedLineFollowsTheDrillsFeed() throws {
        let copy = Self.words
        XCTAssertEqual(InsightsSessionsWords.feedLines("counter_pass", notice: "N", copy: copy), ["WATCHED FEED"])
        XCTAssertEqual(InsightsSessionsWords.feedLines("saved", notice: nil, copy: copy), ["SAVED FEED"])
        XCTAssertEqual(InsightsSessionsWords.feedLines("saved", notice: "COUNTING UNAVAILABLE", copy: copy),
                       ["SAVED FEED", "COUNTING UNAVAILABLE"])
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
    var drills: [String] = []
    private var failing = false
    init(answer: Answer = .json(InsightsWatchedSessionsTests.listJSON)) { self.answer = answer }
    func failDrills() { failing = true }
    func list() throws -> DaemonData.InsightsSessions {
        switch answer {
        case .json(let json): return try InsightsWatchedSessionsTests.list(json)
        case .error(let error): throw error
        }
    }
    func drill(_ reference: String) throws -> DaemonData.InsightsSessionDrillAnswer {
        drills.append(reference)
        if failing { throw DaemonDataError.unreachable }
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
