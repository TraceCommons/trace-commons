import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R9 of #1173: Home and History draw only what the core reports, against
/// C1's sample sets.
@MainActor
final class HomeTests: XCTestCase {
    private func store(_ set: SampleDaemonClient.SampleSet) async -> HomeStore {
        let store = HomeStore(client: SampleDaemonClient(set))
        await store.load()
        return store
    }

    /// Every sample set loads; the core-down set records failures instead of
    /// drawing zeros.
    func test_theStoreLoadsEverySampleSet() async {
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let store = await store(set)
            XCTAssertNotNil(store.history, "\(set)")
            XCTAssertNotNil(store.rollup, "\(set)")
            XCTAssertTrue(store.failures.isEmpty, "\(set): \(store.failures)")
        }
    }

    /// History is newest first, and a row with no date sorts last.
    func test_historyIsNewestFirst() async throws {
        let loaded = await store(.normalDay)
        let rows = try XCTUnwrap(loaded.history)
        let dates = rows.compactMap(\.submittedAt)
        XCTAssertEqual(dates, dates.sorted(by: >))

        let json = #"[{"submission_id":"a","submitted_at":null},{"submission_id":"b","submitted_at":"2026-09-30T09:00:00Z"}]"#
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let undated = try decoder.decode([DaemonData.HistoryRow].self, from: Data(json.utf8))
        XCTAssertEqual(HomeStore.newestFirst(undated).map(\.submissionId), ["b", "a"])
    }

    /// Unknown is a dash; zero is a number.
    func test_countsAreDashedOnlyWhenUnknown() {
        XCTAssertEqual(HomeFormat.count(nil), "—")
        XCTAssertEqual(HomeFormat.count(0), "0")
        XCTAssertEqual(HomeFormat.points(nil), "—")
    }

    /// Pending credit has a condition only when the commons stated one (D6);
    /// an unknown posture has none, so no pending figure is drawn.
    func test_pendingCreditNeedsTheCommonsCondition() async {
        let known = await store(.normalDay)
        XCTAssertNotNil(HomeFormat.pendingCondition(known.credit))
        let unknown = await store(.empty)
        XCTAssertNil(HomeFormat.pendingCondition(unknown.credit))
        XCTAssertNil(HomeFormat.pendingCondition(nil))
    }

    /// Not recorded is never said as approved; armed uses the core's label
    /// for contributing automatically.
    func test_provenanceNeverClaimsAnApprovalThatWasNotRecorded() {
        XCTAssertEqual(HomeFormat.provenance(.notRecorded), MonitorWords.unrecorded)
        XCTAssertNotEqual(HomeFormat.provenance(.notRecorded), HomeFormat.provenance(.personApproved))
        XCTAssertEqual(HomeFormat.provenance(.armed), ProjectCopy.modeChoiceLabel(.autoUpload))
    }

    /// The sample's undated-provenance row reads as not recorded.
    func test_theSampleRowThatPredatesProvenanceReadsUnrecorded() async throws {
        let loaded = await store(.normalDay)
        let rows = try XCTUnwrap(loaded.history)
        XCTAssertTrue(rows.contains { $0.provenance == .notRecorded })
        let row = try XCTUnwrap(rows.first { $0.provenance == .notRecorded })
        XCTAssertEqual(HomeFormat.provenance(row.provenance), MonitorWords.unrecorded)
    }

    /// Held for review is waiting, never drawn as rejected or done.
    func test_heldForReviewIsNotRejected() {
        XCTAssertEqual(HomeFormat.tone("accepted"), .on)
        XCTAssertEqual(HomeFormat.tone("quarantined"), .ask)
        XCTAssertNotEqual(HomeFormat.tone("quarantined"), .outside)
        XCTAssertEqual(HomeFormat.tone("withdrawn"), .neutral)
    }

    /// A row shows only final credit; a pending figure needs its condition.
    func test_rowsShowOnlyFinalCredit() async throws {
        let loaded = await store(.normalDay)
        let rows = try XCTUnwrap(loaded.history)
        for row in rows {
            if let figure = HomeFormat.credit(row) {
                XCTAssertEqual(figure, HomeFormat.points(row.creditPointsFinal))
            } else {
                XCTAssertTrue((row.creditPointsFinal ?? 0) <= 0, row.submissionId)
            }
        }
    }
}
