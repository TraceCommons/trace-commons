import Foundation
import XCTest
@testable import TCShellCore

/// `insights_model_answers` as the daemon serves it. The answered shape is
/// the daemon's own recorded answer,
/// `tests/fixtures/insights-analytics/insights_model_answers.json`, which the
/// contributor crate's `a_model_answers_answer_for_shell_fixtures` test
/// compares to its live answer, so a wire change fails there first. The off
/// and unreadable shapes are the handler's own.
final class InsightsModelAnswersWireTests: XCTestCase {
    static let fixture = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // TCShellCoreTests
        .deletingLastPathComponent()   // Tests
        .deletingLastPathComponent()   // macos
        .deletingLastPathComponent()   // <repo root>
        .appendingPathComponent("tests/fixtures/insights-analytics/insights_model_answers.json")

    private func frame(_ result: String) -> String { #"{"id":0,"result":\#(result)}"# }

    static func recorded() throws -> String {
        try String(contentsOf: fixture, encoding: .utf8)
    }

    func testTheRecordedAnswerDecodesThroughTheLiveClient() async throws {
        let transport = FakeTransport(response: frame(try Self.recorded()))
        let answers = try await LiveDaemonClient(transport: transport).insightsModelAnswers()
        XCTAssertEqual(transport.calls.map(\.method), ["insights_model_answers"])
        XCTAssertEqual(transport.calls.map(\.params), ["{}"], "the method takes no params")
        XCTAssertTrue(answers.showsCounterPass)
        XCTAssertEqual(answers.updatedAt, "2026-09-20T12:00:00+00:00")
        XCTAssertEqual(answers.windowStart, "2026-06-21T12:00:00+00:00")
        XCTAssertEqual(answers.windowEnd, "2026-09-20T12:00:00+00:00")
        XCTAssertEqual(answers.pricing?.basis, "api_list")
        let table = try XCTUnwrap(answers.pricing?.tables.first)
        XCTAssertTrue(table.tableId.hasPrefix("sha256:"))
        XCTAssertEqual(table.version, 1)

        let rows = try XCTUnwrap(answers.rows)
        XCTAssertEqual(rows.map(\.question), [
            "bug_fix_rejected", "refactor_rejected", "tests_rejected", "docs_cost",
            "rejected_across_tasks", "finish_time",
        ], "six rows, in the daemon's order")

        // Docs cost: answered at API prices, with the NEAR AI charge beside
        // the leader's figure and on the row, never added to either.
        let docs = rows[3]
        XCTAssertEqual(docs.state, "answered")
        XCTAssertNil(docs.basis)
        let leader = try XCTUnwrap(docs.leader)
        XCTAssertEqual(leader.label, "claude-sonnet-5-5")
        XCTAssertEqual(leader.sessions, 5)
        XCTAssertNil(leader.edits, "unknown stays nil, never 0")
        XCTAssertEqual(leader.figure, DaemonData.InsightsModelFigureValue(kind: "usd_micros", value: 1_900))
        XCTAssertNil(leader.interruptsPerSessionPermille)
        let next = try XCTUnwrap(docs.next)
        XCTAssertEqual(next.label, "claude-opus-5-5")
        XCTAssertEqual(next.figure.value, 85_400)
        XCTAssertNil(next.nearAiCharge)
        let charge = try XCTUnwrap(docs.nearAiCharge)
        XCTAssertEqual(charge.basis, "near_ai_ledger")
        XCTAssertEqual(charge.median, DaemonData.InsightsModelFigureValue(kind: "usd_micros", value: 33_750))
        XCTAssertEqual(charge.routedSessions, 7)
        XCTAssertEqual(charge.chargeUnknown, 1)
        // The leader's charge counts only the sessions NEAR AI proved it
        // served; the row's counts every Docs session sent through NEAR AI.
        let leaderCharge = try XCTUnwrap(leader.nearAiCharge)
        XCTAssertEqual(leaderCharge.median, DaemonData.InsightsModelFigureValue(kind: "usd_micros", value: 30_000))
        XCTAssertEqual(leaderCharge.routedSessions, 6)
        XCTAssertEqual(leaderCharge.chargeUnknown, 1)
        XCTAssertEqual(docs.leftOut.routed, 7)
        XCTAssertEqual(docs.sessionsByModel, [
            DaemonData.InsightsModelCount(label: "claude-opus-5-5", sessions: 5),
            DaemonData.InsightsModelCount(label: "claude-sonnet-5-5", sessions: 5),
        ], "pairs decode in the daemon's order")

        // Rows short of sessions name no model and keep their counts.
        let across = rows[4]
        XCTAssertEqual(across.state, "not_enough_sessions")
        XCTAssertNil(across.leader)
        XCTAssertNil(across.next)
        XCTAssertEqual(across.sessionsByModel.map(\.sessions), [1, 5, 12])
        XCTAssertEqual(across.leftOut.tooFewEdits, 17)
        let finish = rows[5]
        XCTAssertEqual(finish.editMode, "prompting")
        XCTAssertEqual(finish.leftOut.unknownFacts, 18)
        XCTAssertEqual(finish.leftOut.reasons.map(\.reason), [
            "unknown_facts", "automated", "delegated", "capped", "routed", "reviewer", "no_majority",
            "unpriced", "too_few_sessions", "too_few_edits", "mix_too_different",
        ])
        for row in rows where row.question != "docs_cost" {
            XCTAssertNil(row.nearAiCharge, row.question)
        }
    }

    /// Every left-out reason the daemon sends is one the contract reads: a
    /// renamed or added reason fails here, not as a silent zero.
    func testEveryLeftOutKeyIsDeclared() throws {
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(try Self.recorded().utf8)) as? [String: Any])
        let rows = try XCTUnwrap(object["rows"] as? [[String: Any]])
        let sent = Set(try XCTUnwrap(rows.first?["left_out"] as? [String: Any]).keys)
        let answers = try DaemonDataDecoding.decoder().decode(
            DaemonData.InsightsModelAnswers.self, from: Data(try Self.recorded().utf8))
        let read = Set(try XCTUnwrap(answers.rows?.first).leftOut.reasons.map(\.reason))
        XCTAssertEqual(sent, read)
    }

    func testTheOffAndUnreadableShapesDrawNoCard() throws {
        let decoder = DaemonDataDecoding.decoder()
        let off = try decoder.decode(DaemonData.InsightsModelAnswers.self,
                                     from: Data(#"{"enabled":false,"feed":"counter_pass"}"#.utf8))
        XCTAssertFalse(off.enabled)
        XCTAssertNil(off.readable)
        XCTAssertNil(off.rows)
        XCTAssertFalse(off.showsCounterPass)
        let unreadable = try decoder.decode(
            DaemonData.InsightsModelAnswers.self,
            from: Data(#"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"key_unavailable"}"#.utf8))
        XCTAssertEqual(unreadable.readable, false)
        XCTAssertEqual(unreadable.reason, "key_unavailable")
        XCTAssertNil(unreadable.rows, "an unreadable store has no rows, not empty ones")
        XCTAssertFalse(unreadable.showsCounterPass)
    }

    /// The hand-written sample answers in the daemon's shape: six rows,
    /// every state but `not_priced`, on the sets with traces; off for a
    /// fresh install; unreadable where counts are unknown.
    func testTheSampleSetsAnswerInTheDaemonsShape() async throws {
        let answered = try await SampleDaemonClient(.normalDay).insightsModelAnswers()
        XCTAssertTrue(answered.showsCounterPass)
        let rows = try XCTUnwrap(answered.rows)
        XCTAssertEqual(rows.map(\.question), [
            "bug_fix_rejected", "refactor_rejected", "tests_rejected", "docs_cost",
            "rejected_across_tasks", "finish_time",
        ])
        XCTAssertEqual(Set(rows.map(\.state)), ["answered", "no_clear_difference", "not_enough_sessions"])
        XCTAssertEqual(Set(rows.compactMap(\.basis)), ["rejected", "undone"])
        XCTAssertNotNil(rows[3].nearAiCharge)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(
            with: Data(try XCTUnwrap(SampleDaemonClient(.normalDay).json(for: "insights_model_answers")).utf8))
            as? [String: Any])
        XCTAssertEqual(object["_sample"] as? String, "hand-written")
        let off = try await SampleDaemonClient(.empty).insightsModelAnswers()
        XCTAssertFalse(off.showsCounterPass)
        let unreadable = try await SampleDaemonClient(.unknownCounts).insightsModelAnswers()
        XCTAssertEqual(unreadable.readable, false)
        XCTAssertFalse(unreadable.showsCounterPass)
    }
}
