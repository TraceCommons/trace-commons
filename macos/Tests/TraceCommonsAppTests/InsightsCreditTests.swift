import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// Where NEAR AI credit went: the sample answer decodes, and the tables'
/// words and money read as the owner asked.
final class InsightsCreditTests: XCTestCase {
    private let copy = [
        "analytics_task_bug_fix": "Bug fix", "analytics_task_refactor": "Refactor",
        "analytics_task_other": "Other", "analytics_credit_tool_codex": "Codex",
        "analytics_credit_tool_claude_code": "Claude Code", "analytics_credit_other_project": "Other folders",
        "analytics_unavailable": "—",
    ]

    func testTheSampleAnswerDecodes() async throws {
        let credit = try await SampleDaemonClient(.normalDay).insightsCredit()
        XCTAssertEqual(credit.ledger, true)
        XCTAssertEqual(credit.total?.usdMicros, 18_420_000)
        XCTAssertEqual(credit.total?.unknownTraces, 2)
        XCTAssertEqual(credit.byFolder?.last?.key, nil)
        let off = try await SampleDaemonClient(.empty).insightsCredit()
        XCTAssertFalse(off.enabled)
    }

    func testMoneyReadsInDollarsAndNeverRoundsToZero() {
        XCTAssertEqual(InsightsCreditWords.money(18_420_000), "$18.42")
        XCTAssertEqual(InsightsCreditWords.money(0), "$0.00")
        XCTAssertEqual(InsightsCreditWords.money(4_000), "<$0.01")
    }

    /// The rules' no_edits and other share the Other row; a missing folder
    /// is Other folders; rows lead with the most spent.
    func testRowsMergeSharedLabelsAndSortBySpend() {
        typealias Group = DaemonData.InsightsCredit.CreditGroup
        let tasks = InsightsCreditWords.rows([
            Group(key: "no_edits", usdMicros: 1_000_000, traces: 1, unknownTraces: 0),
            Group(key: "refactor", usdMicros: 2_000_000, traces: 2, unknownTraces: 1),
            Group(key: "other", usdMicros: 1_500_000, traces: 3, unknownTraces: 0),
        ], .task, copy: copy)
        XCTAssertEqual(tasks.map(\.label), ["Other", "Refactor"])
        XCTAssertEqual(tasks[0].group.usdMicros, 2_500_000)
        XCTAssertEqual(tasks[0].group.traces, 4)
        XCTAssertEqual(InsightsCreditWords.label(nil, .project, copy: copy), "Other folders")
        XCTAssertEqual(InsightsCreditWords.label("codex", .tool, copy: copy), "Codex")
    }

    /// An answer that is off, unreadable or without the ledger is never
    /// drawn as figures.
    @MainActor
    func testOnlyALedgerReadIsReadable() async throws {
        let model = InsightsCreditModel()
        await model.load(SampleDaemonClient(.unknownCounts))
        XCTAssertNil(model.readable)
        XCTAssertTrue(model.shown)
        await model.load(SampleDaemonClient(.normalDay))
        XCTAssertNotNil(model.readable)
    }
}
