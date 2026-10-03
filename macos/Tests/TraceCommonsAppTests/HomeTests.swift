@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R9 of #1173: Home and History draw only what the core reports, against
/// C1's sample sets.
@MainActor
final class HomeTests: XCTestCase {
    /// History says a submission in its own core words, waiting to be
    /// scored, and not the shared "Submitted" label; every other status
    /// keeps the shared label, and an unlabelled one its raw value.
    func test_historySaysASubmissionAsWaitingToBeScored() throws {
        let words = try XCTUnwrap(MonitorWords.table)
        let shared: (String) -> String? = { $0 == "submitted" ? "Submitted" : ($0 == "accepted" ? "Accepted into the commons" : nil) }
        XCTAssertEqual(HomeFormat.statusWord("submitted", table: words, fallback: shared), words.historySubmitted)
        XCTAssertNotEqual(words.historySubmitted, "Submitted")
        XCTAssertEqual(HomeFormat.statusWord("accepted", table: words, fallback: shared), "Accepted into the commons")
        XCTAssertEqual(HomeFormat.statusWord("purged", table: words, fallback: shared), "purged")
        // Before the core's words load, the shared label stands in.
        XCTAssertEqual(HomeFormat.statusWord("submitted", table: nil, fallback: shared), "Submitted")
    }

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

    /// A recorded reply, decoded after `edit` is applied to its JSON object
    /// (the K2 recordings carry an unknown commons posture).
    private func recorded<T: Decodable>(
        _ type: T.Type, _ method: String, _ set: SampleDaemonClient.SampleSet = .normalDay,
        edit: (inout [String: Any]) -> Void = { _ in }
    ) throws -> T {
        let reply = try XCTUnwrap(SampleDaemonData.reply(method, in: set))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        edit(&object)
        return try DaemonDataDecoding.decoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }

    /// Pending credit has a condition only when the commons stated one (D6);
    /// an unknown posture has none, so no pending figure is drawn.
    func test_pendingCreditNeedsTheCommonsCondition() async throws {
        let known = try recorded(DaemonData.CommonsCreditSummary.self, "commons_credit_summary") {
            $0["posture_state"] = "known"
            $0["commons_settlement"] = "pending_review"
            $0["commons_settlement_explanation"] = "Scored when the commons reviews it."
        }
        XCTAssertTrue(known.postureKnown)
        XCTAssertEqual(HomeFormat.pendingCondition(known), "Scored when the commons reviews it.")
        let unknown = await store(.normalDay)
        XCTAssertNil(HomeFormat.pendingCondition(unknown.credit))
        XCTAssertNil(HomeFormat.pendingCondition(nil))
        // Credit is never shown without the core's not-currency sentence.
        XCTAssertFalse(MonitorWords.creditNotCurrency.isEmpty)
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

    /// A row shows only final credit, zero included; a pending figure
    /// needs its condition, and an unscored row shows none.
    func test_rowsShowOnlyFinalCredit() async throws {
        let loaded = await store(.normalDay)
        let rows = try XCTUnwrap(loaded.history)
        for row in rows {
            if let figure = HomeFormat.credit(row) {
                XCTAssertEqual(figure, HomeFormat.points(row.creditPointsFinal))
            } else {
                XCTAssertNil(row.creditPointsFinal, row.submissionId)
            }
        }
        let decode = { (json: String) in
            try DaemonDataDecoding.decoder().decode(DaemonData.HistoryRow.self, from: Data(json.utf8))
        }
        let zero = try decode(#"{"submission_id":"z","credit_points_final":0}"#)
        XCTAssertEqual(HomeFormat.credit(zero), HomeFormat.points(0))
        XCTAssertNil(HomeFormat.credit(try decode(#"{"submission_id":"u","credit_points_final":null}"#)))
    }

    /// A row's day is month and day, compact or not: a weekday alone would
    /// read a months-old row as this week. In full it adds the uploaded
    /// size and when a withdrawal was seen.
    func test_aRowsDayIsAlwaysMonthAndDay() throws {
        let row = try DaemonDataDecoding.decoder().decode(DaemonData.HistoryRow.self, from: Data(
            #"{"submission_id":"a","submitted_at":"2026-03-14T15:00:00Z","source":"claude-code","uploaded_bytes":48213,"revoked_at":"2026-09-30T09:00:00Z"}"#.utf8))
        let date = try XCTUnwrap(row.submittedAt)
        let day = date.formatted(.dateTime.month(.abbreviated).day())
        XCTAssertTrue(HomeFormat.meta(row, compact: true).hasPrefix(day))
        XCTAssertFalse(HomeFormat.meta(row, compact: true).contains(MonitorWords.withdrawn))
        let full = HomeFormat.meta(row, compact: false)
        XCTAssertTrue(full.hasPrefix(day))
        XCTAssertTrue(full.contains(ByteCountFormatter.string(fromByteCount: 48213, countStyle: .file)), full)
        XCTAssertTrue(full.contains(MonitorWords.withdrawn), full)
    }

    /// A full page of History says how many of the total it shows, in the
    /// core's words; a short page says nothing.
    func test_aFullHistoryPageSaysItIsCapped() throws {
        let rollup = try recorded(DaemonData.HistoryRollup.self, "history_rollup") {
            $0["all_time"] = ["submitted": 10, "accepted": 200, "quarantined": 3, "withdrawn": 1, "other": 0]
        }
        XCTAssertNil(HomeFormat.cap(HomeStore.historyLimit - 1, rollup: rollup))
        let words = try XCTUnwrap(MonitorWords.table)
        XCTAssertEqual(HomeFormat.cap(HomeStore.historyLimit, rollup: rollup),
                       words.historyCap(shown: HomeStore.historyLimit, total: 214))
        XCTAssertTrue(try XCTUnwrap(HomeFormat.cap(HomeStore.historyLimit, rollup: rollup)).contains("214"))
        XCTAssertEqual(HomeFormat.cap(HomeStore.historyLimit, rollup: nil),
                       words.historyCap(shown: HomeStore.historyLimit, total: nil))
    }

    /// Watching counts the tools the core reads, unset ones included, and
    /// a signed-out or unhealthy core is never drawn as watching.
    func test_watchingFollowsTheCore() async throws {
        let loaded = await store(.normalDay)
        let destinations = try XCTUnwrap(loaded.destinations)
        let watched = destinations.tools.filter { $0.sessions?.watch == "watched" }.count
        XCTAssertGreaterThan(watched, 0)
        XCTAssertEqual(HomeFormat.watching(loaded.status, destinations: destinations), .watching(watched))
        XCTAssertEqual(HomeFormat.watchingState(loaded), .ready)

        let unsetButRead = try recorded(DaemonData.ToolDestinations.self, "tool_destinations")
        XCTAssertEqual(unsetButRead.watchedCount, watched)

        let signedOut = try recorded(DaemonData.Status.self, "status") { $0["logged_in"] = false }
        XCTAssertEqual(HomeFormat.watching(signedOut, destinations: destinations), .signedOut)
        let unhealthy = try recorded(DaemonData.Status.self, "status") {
            $0["health"] = ["last_error_label": "queue-full", "since": NSNull()]
        }
        XCTAssertEqual(HomeFormat.watching(unhealthy, destinations: destinations), .unhealthy("queue-full"))
    }

    /// A status read that fails after a good one leaves Home unknown, never
    /// the last state drawn as current, and the waiting count a dash.
    func test_aFailedStatusReadIsNeverDrawnAsWatching() async {
        let transport = FlakyStatusTransport(.normalDay)
        let store = HomeStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(HomeFormat.watchingState(store), .ready)
        transport.breaksStatus = true
        await store.load()
        XCTAssertNil(store.status)
        XCTAssertEqual(HomeFormat.watchingState(store), .unknown)
        XCTAssertEqual(HomeFormat.count(store.status?.decisionsOwed), "—")
    }

    /// The stack-wide rule on Home: a core that does not answer reads as
    /// core down, never as watching, and a status with no paused field reads
    /// as unknown.
    func test_aCoreThatIsDownIsNeverDrawnAsWatching() async {
        let down = await store(.coreDown)
        let state = HomeFormat.watchingState(down)
        XCTAssertEqual(state, .coreDown)
        XCTAssertFalse(state.isHealthy)
    }

    /// Home's words come from the core's table, not from Swift.
    func test_homeWordsComeFromTheCore() {
        XCTAssertNotNil(MonitorWords.table)
        XCTAssertEqual(MonitorWords.history, MonitorWords.table?.history)
        XCTAssertFalse(MonitorWords.unrecorded.isEmpty)
    }
}

/// Answers from a sample set's recorded replies; `status` stops decoding
/// once `breaksStatus` is set.
private final class FlakyStatusTransport: DaemonTransport, @unchecked Sendable {
    private let set: SampleDaemonClient.SampleSet
    private let lock = NSLock()
    private var broken = false

    init(_ set: SampleDaemonClient.SampleSet) {
        self.set = set
    }

    var breaksStatus: Bool {
        get { lock.withLock { broken } }
        set { lock.withLock { broken = newValue } }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == "status" && breaksStatus { return #"{"id":0,"result":{"logged_in":"not a bool"}}"# }
        guard let reply = SampleDaemonData.reply(method, in: set) else {
            return #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
        }
        return #"{"id":0,"result":\#(reply)}"#
    }
}
