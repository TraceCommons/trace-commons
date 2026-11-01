import Foundation
import XCTest
@testable import TCShellCore

/// `insights_submitted` as the daemon serves it. The answered shape is the
/// daemon's own recorded answer,
/// `tests/fixtures/insights-analytics/insights_submitted.json`, which the
/// contributor crate's `a_submitted_answer_for_shell_fixtures` test compares
/// to its live answer, so a wire change fails there first.
final class InsightsSubmittedWireTests: XCTestCase {
    static let fixture = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // TCShellCoreTests
        .deletingLastPathComponent()   // Tests
        .deletingLastPathComponent()   // macos
        .deletingLastPathComponent()   // <repo root>
        .appendingPathComponent("tests/fixtures/insights-analytics/insights_submitted.json")

    private func frame(_ result: String) -> String { #"{"id":0,"result":\#(result)}"# }

    static func recorded() throws -> String {
        try String(contentsOf: fixture, encoding: .utf8)
    }

    func testTheRecordedAnswerDecodesThroughTheLiveClient() async throws {
        let transport = FakeTransport(response: frame(try Self.recorded()))
        let answer = try await LiveDaemonClient(transport: transport).insightsSubmitted(weeks: 13)
        XCTAssertEqual(transport.calls.map(\.method), ["insights_submitted"])
        let params = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(transport.calls[0].params.utf8)) as? [String: Any])
        XCTAssertEqual(params["weeks"] as? Int, 13)
        XCTAssertEqual(params["tz"] as? Int, TimeZone.current.secondsFromGMT())
        XCTAssertTrue(answer.isAnswered)
        XCTAssertEqual(answer.weeks, 13)
        XCTAssertEqual(answer.windowStart, "2026-06-22T00:00:00+00:00")
        XCTAssertEqual(answer.pricing?.basis, "api_list")
        XCTAssertTrue(try XCTUnwrap(answer.pricing?.tables.first).tableId.hasPrefix("sha256:"))

        let totals = try XCTUnwrap(answer.totals)
        XCTAssertEqual(totals.submitted, 7)
        XCTAssertEqual(totals.matched, 3)
        XCTAssertEqual(totals.unmatched, 4)
        XCTAssertEqual(totals.creditFinal, 6.0)
        XCTAssertEqual(totals.creditPending, 4.0)
        XCTAssertEqual(totals.byStatus, ["accepted": 3, "pending": 1, "quarantined": 1, "rejected": 1,
                                         "withdrawn": 1, "other": 0])
        XCTAssertEqual(totals.byOutcomeReason, ["novelty_failed": 1, "policy_mismatch": 1])
        XCTAssertEqual(totals.unmatchedByReason, ["no_queue_entry": 2, "no_session_row": 1, "source_not_read": 1])

        let models = try XCTUnwrap(answer.byModel)
        XCTAssertEqual(models.map(\.label), ["claude-sonnet-5-5", "claude-opus-5-5"])
        let sonnet = models[0]
        XCTAssertEqual(sonnet.tool, "claude_code")
        XCTAssertEqual(sonnet.tally.submitted, 2)
        XCTAssertEqual(sonnet.tally.creditPending, 2.5)
        XCTAssertEqual(sonnet.spend.pricedSessions, 2)
        XCTAssertGreaterThan(sonnet.spend.usdMicrosKnown, 0)
        XCTAssertEqual(sonnet.edits.editsKeptKnown, 2)
        XCTAssertTrue(sonnet.edits.editsDeniedPartial)
        XCTAssertEqual(sonnet.tasks, ["docs": 1, "tests": 1])
        XCTAssertEqual(answer.byModelLeftOut, .init(unmatched: 4))

        XCTAssertEqual(answer.byTask?.map(\.key), ["bug_fix", "feature", "refactor", "tests", "docs",
                                                   "understand", "review", "setup"])
        XCTAssertEqual(answer.byTaskUnknown?.submitted, 4)
        let code = try XCTUnwrap(answer.code)
        XCTAssertEqual(code.files, .init(source: 1, test: 1, doc: 1))
        XCTAssertEqual(code.edits.editsKnown, 3)
        XCTAssertEqual(code.spend.pricedSessions, 3)
        XCTAssertEqual(answer.byWeek?.count, 13)
        XCTAssertEqual(answer.byWeek?.last?.weekStart, "2026-09-14")
        XCTAssertEqual(answer.byProject?.map(\.label), ["my-app", "other-app"])
        XCTAssertEqual(answer.leftOut?.neverFolder, 0)

        let traces = try XCTUnwrap(answer.traces)
        XCTAssertEqual(answer.tracesTotal, 7)
        XCTAssertEqual(traces.count, 7)
        let unmatched = traces[0]
        XCTAssertEqual(unmatched.status, "rejected")
        XCTAssertFalse(unmatched.matched)
        XCTAssertEqual(unmatched.unmatchedReason, "no_session_row")
        XCTAssertNil(unmatched.model)
        XCTAssertNil(unmatched.files, "unknown stays nil, never 0")
        let opus = try XCTUnwrap(traces.first { $0.model == "claude-opus-5-5" })
        XCTAssertEqual(opus.creditPointsFinal, 5.0)
        XCTAssertEqual(opus.task, "feature")
        XCTAssertEqual(opus.files?.source, 1)
        XCTAssertEqual(opus.editsKept, 1)
        XCTAssertNil(opus.editsDenied)
        XCTAssertNotNil(opus.usdMicros)
    }

    func testAllTimeSendsNoWeeks() async throws {
        let transport = FakeTransport(response: frame(#"{"enabled":false,"feed":"counter_pass"}"#))
        let answer = try await LiveDaemonClient(transport: transport).insightsSubmitted(weeks: nil)
        XCTAssertFalse(answer.enabled)
        XCTAssertFalse(answer.isAnswered)
        let params = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(transport.calls[0].params.utf8)) as? [String: Any])
        XCTAssertNil(params["weeks"])
    }

    func testTheUnreadableAnswerDecodes() async throws {
        let transport = FakeTransport(response: frame(
            #"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"history_unreadable"}"#))
        let answer = try await LiveDaemonClient(transport: transport).insightsSubmitted(weeks: 4)
        XCTAssertEqual(answer.readable, false)
        XCTAssertEqual(answer.reason, "history_unreadable")
        XCTAssertNil(answer.totals)
        XCTAssertFalse(answer.isAnswered)
    }

    /// A client that predates the method reads it as not served.
    func testTheDefaultIsNotAvailableYet() async throws {
        do {
            _ = try await SampleDaemonClient(.normalDay).insightsSubmitted(weeks: 4)
            XCTFail("the sample client answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .notAvailableYet(method: "insights_submitted"))
        }
    }

    /// The sample the tab shows, labelled, before anything is submitted:
    /// about 30 traces over six weeks on three models, and every aggregate
    /// agrees with its traces.
    func testTheSampleIsBelievableAndConsistent() throws {
        let sample = DaemonData.InsightsSubmitted.sample
        XCTAssertTrue(sample.isAnswered)
        let traces = try XCTUnwrap(sample.traces)
        XCTAssertEqual(traces.count, 30)
        XCTAssertEqual(sample.tracesTotal, 30)
        XCTAssertEqual(Set(traces.compactMap(\.model)), ["claude-opus-5-5", "claude-sonnet-5-5", "gpt-5.5"])
        XCTAssertEqual(traces.map(\.submittedAt), traces.map(\.submittedAt).sorted(by: >), "newest first")
        let totals = try XCTUnwrap(sample.totals)
        XCTAssertEqual(totals.submitted, 30)
        for status in ["accepted", "pending", "quarantined"] {
            XCTAssertGreaterThan(totals.byStatus[status] ?? 0, 0, status)
        }
        XCTAssertEqual(totals.byStatus.values.reduce(0, +), 30)
        XCTAssertEqual(Set(totals.byOutcomeReason.keys), ["duplicate", "novelty_failed", "policy_mismatch"])
        XCTAssertGreaterThan(totals.creditFinal, 0)
        XCTAssertGreaterThan(totals.creditPending, 0)
        XCTAssertEqual(sample.byWeek?.count, 6)
        XCTAssertEqual(sample.byWeek?.map(\.submitted).reduce(0, +), 30)
        XCTAssertTrue(sample.byWeek?.allSatisfy { $0.submitted > 0 } ?? false, "six weeks of history")
        XCTAssertEqual(sample.byModel?.map(\.tally.submitted).reduce(0, +), 30)
        XCTAssertEqual(sample.byTask?.count, 8)
        XCTAssertEqual(sample.byTask?.map(\.submitted).reduce(0, +), 30)
        XCTAssertEqual(sample.byProject?.map(\.submitted).reduce(0, +), 30)
        let files = try XCTUnwrap(sample.code?.files)
        for count in [files.source, files.test, files.doc, files.config] {
            XCTAssertGreaterThan(count, 0)
        }
        XCTAssertEqual(files.total, traces.compactMap(\.files?.total).reduce(0, +))
        XCTAssertEqual(sample.code?.spend.usdMicrosKnown, traces.compactMap(\.usdMicros).reduce(0, +))
        // It round-trips through the wire form.
        let data = try JSONEncoder().encode(sample)
        XCTAssertEqual(try JSONDecoder().decode(DaemonData.InsightsSubmitted.self, from: data), sample)
    }
}
