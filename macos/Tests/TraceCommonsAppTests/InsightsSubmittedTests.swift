import XCTest

import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// Submitted (product owner, 2026-10-29): every trace sent, from
/// `insights_submitted`, in the core's words and History's.
@MainActor
final class InsightsSubmittedTests: XCTestCase {
    private func words() throws -> [String: String] { try XCTUnwrap(TCInsights.copy()) }
    private func labels() throws -> PublicRunCopy {
        try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
    }

    func testTilesReadTheFixturesTotals() throws {
        let answer = try InsightsGetStartedRenderTests.submittedFixture()
        let tiles = InsightsSubmittedWords.tiles(answer, copy: try words())
        XCTAssertEqual(tiles.map(\.id), ["submitted", "accepted", "credit", "model"])
        XCTAssertEqual(tiles.map(\.value), ["7", "3", "6.0", "claude-opus-5-5"])
        XCTAssertEqual(tiles[1].note, "43% of traces sent")
        XCTAssertEqual(tiles[2].note, "4.0 pending")
        XCTAssertEqual(tiles[3].note, "5.0 credit received")
    }

    func testTopEarningModelIsTheDashUntilOneEarns() throws {
        let copy = try words()
        let pending = SubmittedSample.build([try XCTUnwrap(DaemonData.InsightsSubmitted.sample.traces?.first)])
        XCTAssertNil(InsightsSubmittedWords.topModel(pending))
        let tile = try XCTUnwrap(InsightsSubmittedWords.tiles(pending, copy: copy).last)
        XCTAssertEqual(tile.value, "—")
        XCTAssertEqual(tile.note, copy["analytics_sub_tile_top_model_none"])
    }

    func testByModelLeadsWithCreditThenTracesSent() throws {
        let answer = try InsightsGetStartedRenderTests.submittedFixture()
        // The daemon sends Sonnet first (more sent); Opus earned more.
        XCTAssertEqual(answer.byModel?.map(\.label), ["claude-sonnet-5-5", "claude-opus-5-5"])
        XCTAssertEqual(InsightsSubmittedWords.models(answer).map(\.label), ["claude-opus-5-5", "claude-sonnet-5-5"])
        let sample = DaemonData.InsightsSubmitted.sample
        let credits = InsightsSubmittedWords.models(sample).map(\.tally.creditFinal)
        XCTAssertEqual(credits, credits.sorted(by: >))
    }

    func testWhyNothingWasEarnedReadsHistorysWords() throws {
        let answer = try InsightsGetStartedRenderTests.submittedFixture()
        let reasons = InsightsSubmittedWords.reasons(answer, labels: try labels())
        XCTAssertEqual(Set(reasons.map(\.label)), ["Needs Train consent", "Too similar"])
        XCTAssertEqual(reasons.filter(\.needsConsent).map(\.id), ["policy_mismatch"])
        XCTAssertTrue(InsightsSubmittedWords.reasons(answer, labels: nil).isEmpty, "never the raw label")
    }

    func testStatusesAreHistorysWithItsTones() throws {
        let labels = try labels()
        XCTAssertEqual(InsightsSubmittedWords.status("pending", labels: labels)?.label,
                       labels.historyStatusLabel(for: "submitted"))
        XCTAssertEqual(InsightsSubmittedWords.status("pending", labels: labels)?.tone, .ask)
        XCTAssertEqual(InsightsSubmittedWords.status("accepted", labels: labels)?.tone, .on)
        XCTAssertEqual(InsightsSubmittedWords.status("other", labels: labels)?.label,
                       labels.contributionStatusUnavailable)
        XCTAssertNil(InsightsSubmittedWords.status("accepted", labels: nil))
    }

    /// Day 1: one trace fills every section with its own figures.
    func testOneTraceFillsEverySection() throws {
        let copy = try words()
        let answer = SubmittedSample.build([try InsightsGetStartedRenderTests.oneTrace()])
        XCTAssertEqual(InsightsSubmittedWords.intro(1, copy: copy), "From the one trace you sent")
        XCTAssertEqual(InsightsSubmittedWords.tiles(answer, copy: copy).map(\.value).first, "1")
        XCTAssertEqual(InsightsSubmittedWords.models(answer).count, 1)
        XCTAssertEqual(InsightsSubmittedWords.taskRows(answer).count, 1)
        XCTAssertTrue(InsightsSubmittedWords.hasCode(answer.code))
        XCTAssertEqual(answer.byWeek?.filter { $0.submitted > 0 }.count, 1)
        XCTAssertEqual(answer.byProject?.count, 1)
        XCTAssertEqual(InsightsSubmittedWords.percent(1, of: 1), 100)
        XCTAssertEqual(InsightsSubmittedWords.percent(1, of: 300), 1)
        XCTAssertEqual(InsightsSubmittedWords.percent(299, of: 300), 99)
    }

    func testEditsKeptAndSpendShowTheDashWhenUnknown() throws {
        let copy = try words()
        XCTAssertEqual(InsightsSubmittedWords.kept(.init(editsKnown: 0, editsKeptKnown: 0), copy: copy), "—")
        XCTAssertEqual(InsightsSubmittedWords.kept(.init(editsKnown: 14, editsKeptKnown: 12), copy: copy), "12 of 14")
        XCTAssertEqual(InsightsSubmittedWords.spend(.init(usdMicrosKnown: 0, pricedSessions: 0), copy: copy), "—")
        XCTAssertEqual(InsightsSubmittedWords.spend(.init(usdMicrosKnown: 2_500_000, pricedSessions: 1), copy: copy),
                       "$2.50")
    }

    func testEverySubmittedWordIsTheCores() throws {
        let copy = try words()
        for key in ["analytics_tab_submitted", "analytics_sub_scope", "analytics_sub_reasons_title",
                    "analytics_sub_models_title", "analytics_sub_tasks_title", "analytics_sub_code_title",
                    "analytics_sub_weeks_title", "analytics_sub_projects_title", "analytics_sub_list_title",
                    "analytics_by_model_open_submitted"] {
            XCTAssertFalse((copy[key] ?? "").isEmpty, key)
        }
        for kind in InsightsSubmittedWords.fileKinds(.init(source: 1, test: 1, doc: 1, config: 1), copy: copy) {
            XCTAssertFalse(kind.label.isEmpty, kind.id)
        }
    }
}
