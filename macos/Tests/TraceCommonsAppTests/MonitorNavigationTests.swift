@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

@MainActor
final class MonitorNavigationTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The Monitor's three stores read the app's live client, re-attached
    /// whenever the daemon restarts; sample data is debug-only and opt-in.
    func test_theMonitorUsesTheLiveClient() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("DaemonDataWiring.sample(choice.set)"), "a store is built on sample data")
        for store in ["traces", "inference", "home"] {
            XCTAssertTrue(window.contains("\(store).attach(client)"), "\(store) is never attached to the live client")
        }
        XCTAssertTrue(window.contains(".task(id: model.liveData.map(ObjectIdentifier.init))"))
        // The function's body: from its signature to the `#endif` that
        // closes its debug-only branch.
        let sample = try XCTUnwrap(window.range(of: "static func sampleClient()"))
        let end = try XCTUnwrap(window.range(of: "#endif", range: sample.upperBound ..< window.endIndex))
        let body = window[sample.lowerBound ..< end.upperBound]
        XCTAssertFalse(body.dropFirst().contains("static func"), "the scan ran past sampleClient()")
        XCTAssertTrue(body.contains("#if DEBUG"), "sample data must be debug-only")
        XCTAssertTrue(body.contains("TRACE_COMMONS_SAMPLE"))
        XCTAssertTrue(body.contains("#else\n        return nil"), "a release build has no sample client")
    }

    // MARK: No client: the core is down, never empty and healthy

    func test_withNoClientTheTracesStoreIsCoreDown() async {
        let store = TracesStore(client: nil)
        await store.run()
        XCTAssertEqual(store.phase, .failed(.unreachable))
        XCTAssertNil(store.decisionsOwed)
    }

    func test_withNoClientTheHomeStoreIsCoreDown() async {
        let store = HomeStore(client: nil)
        await store.run()
        XCTAssertEqual(store.failures["status"], .unreachable)
        XCTAssertNil(store.status)
        XCTAssertEqual(HomeFormat.watchingState(store), .coreDown)
        XCTAssertEqual(MissionFormat.count(store.missions), "—")
    }

    func test_withNoClientTheInferenceStoreIsCoreDown() async {
        let store = InferenceStore(client: nil)
        await store.run()
        XCTAssertEqual(store.failures["inference_calls"], .unreachable)
        XCTAssertEqual(store.failures["harness_list"], .unreachable)
        XCTAssertNil(store.calls)
    }

    // MARK: Attaching a new client: loading again, nothing from the old one as current

    func test_attachResetsEachStoreToLoading() async {
        let traces = TracesStore(client: SampleDaemonClient(.normalDay))
        await traces.load()
        XCTAssertEqual(traces.phase, .loaded)
        XCTAssertNotNil(traces.decisionsOwed)
        XCTAssertFalse(traces.tree.allSessions.isEmpty)
        traces.attach(SampleDaemonClient(.busyQueue))
        XCTAssertEqual(traces.phase, .loading)
        XCTAssertNil(traces.decisionsOwed, "the old daemon's count is not drawn as the new one's")
        XCTAssertTrue(traces.tree.tools.isEmpty, "the old daemon's folders are not drawn or acted on")
        XCTAssertTrue(traces.tree.allSessions.isEmpty)

        let home = HomeStore(client: SampleDaemonClient(.coreDown))
        await home.load()
        XCTAssertNotNil(home.failures["status"])
        home.attach(SampleDaemonClient(.normalDay))
        XCTAssertTrue(home.failures.isEmpty)
        XCTAssertNil(home.status)
        XCTAssertEqual(HomeFormat.watchingState(home), .loading)

        let inference = InferenceStore(client: SampleDaemonClient(.normalDay))
        await inference.load()
        XCTAssertNotNil(inference.calls)
        inference.attach(nil)
        XCTAssertNil(inference.calls)
        XCTAssertTrue(inference.failures.isEmpty)
        await inference.run()
        XCTAssertEqual(inference.failures["inference_calls"], .unreachable)
    }

    // MARK: A read the old client answers after attach is dropped

    /// A daemon restart while Home is reading: the old client's answers
    /// arrive after the new one was attached, and must not be drawn as its.
    func test_theHomeStoreDropsAReadTheOldClientAnswersAfterAttach() async {
        // Five of Home's six reads reach the daemon; missions is provisional.
        let entered = expectation(description: "every read reached the old daemon")
        entered.expectedFulfillmentCount = 5
        let gate = GatedTransport(.normalDay, entered: entered)
        let store = HomeStore(client: LiveDaemonClient(transport: gate))
        let loading = Task { await store.load() }
        await fulfillment(of: [entered], timeout: 10)
        store.attach(nil)
        gate.open()
        await loading.value
        XCTAssertNil(store.status, "the old daemon's status is drawn after a restart")
        XCTAssertNil(store.destinations)
        XCTAssertNil(store.history)
        XCTAssertNil(store.rollup)
        XCTAssertNil(store.credit)
        XCTAssertTrue(store.failures.isEmpty, "an old read's outcome is recorded against the new client")
    }

    func test_theInferenceStoreDropsAReadTheOldClientAnswersAfterAttach() async {
        // Three of its four reads reach the daemon; the summary is provisional.
        let entered = expectation(description: "every read reached the old daemon")
        entered.expectedFulfillmentCount = 3
        let gate = GatedTransport(.normalDay, entered: entered)
        let store = InferenceStore(client: LiveDaemonClient(transport: gate))
        let loading = Task { await store.load() }
        await fulfillment(of: [entered], timeout: 10)
        store.attach(nil)
        gate.open()
        await loading.value
        XCTAssertNil(store.calls, "the old daemon's calls are drawn after a restart")
        XCTAssertNil(store.harnesses)
        XCTAssertNil(store.destinations)
        XCTAssertTrue(store.failures.isEmpty, "an old read's outcome is recorded against the new client")
    }

    // MARK: Traces: nothing the old daemon said survives an attach

    /// After a daemon restart the old daemon's refusals, notices and undo
    /// offers are not drawn on the new one's tab, and Undo never reaches it.
    func test_attachClearsTheOldDaemonsNoticesAndUndo() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let id = try XCTUnwrap(store.tree.allSessions.first?.entryId)
        await store.perform(.contribute, on: id)
        XCTAssertNotNil(store.lastContributed)
        // The sample core refuses the cancel, so a refusal is kept.
        await store.perform(.undoContribute, on: id)
        XCTAssertNotNil(store.actionError)
        await store.perform(.keep, on: id)
        XCTAssertEqual(store.lastKept, id)
        await store.setSource(.codex, .watch(path: ""))
        XCTAssertFalse(store.writeErrors.isEmpty)
        let folder = TracesTree.FolderNode(id: "p1", label: "docs", mode: .ask, offerableModes: [.ask, .ignore], sessions: [])
        await store.setFolderMode(folder, .ignore, promised: 3)
        XCTAssertNotNil(store.folderNotice)

        store.attach(SampleDaemonClient(.busyQueue))
        XCTAssertNil(store.lastContributed, "the old daemon's Undo would send cancel to the new one")
        XCTAssertNil(store.lastKept)
        XCTAssertNil(store.actionError)
        XCTAssertTrue(store.writeErrors.isEmpty)
        XCTAssertNil(store.folderNotice)
        XCTAssertTrue(store.acting.isEmpty)
        XCTAssertTrue(store.writing.isEmpty)
    }

    /// A gated old client, a write started on it, then a restart (attach)
    /// before the old daemon answers.
    private func staleWrite(
        _ write: @escaping @MainActor (TracesStore) async -> Void
    ) async -> TracesStore {
        let entered = expectation(description: "the write reached the old daemon")
        entered.assertForOverFulfill = false
        let gate = GatedTransport(.normalDay, entered: entered)
        let store = TracesStore(client: LiveDaemonClient(transport: gate))
        let writing = Task { await write(store) }
        await fulfillment(of: [entered], timeout: 10)
        store.attach(nil)
        gate.open()
        await writing.value
        return store
    }

    func test_aReviewActionTheOldDaemonRefusesAfterAttachIsDropped() async {
        // `cancel` is refused by the sample transport.
        let store = await staleWrite { await $0.perform(.undoContribute, on: "e1") }
        XCTAssertNil(store.actionError, "the old daemon's refusal is drawn on the new one's tab")
        XCTAssertTrue(store.acting.isEmpty)
        XCTAssertEqual(store.phase, .loading, "a stale answer moved the new client's tab")
    }

    func test_aReviewActionTheOldDaemonTakesAfterAttachIsDropped() async {
        let store = await staleWrite { await $0.perform(.keep, on: "e1") }
        XCTAssertNil(store.lastKept, "the old daemon's undo is offered against the new one")
        XCTAssertNil(store.actionError)
        XCTAssertEqual(store.phase, .loading)
    }

    func test_aSourceWriteTheOldDaemonRefusesAfterAttachIsDropped() async {
        // `set_settings` is refused by the sample transport.
        let store = await staleWrite { await $0.setSource(.codex, .off) }
        XCTAssertTrue(store.writeErrors.isEmpty, "the old daemon's refusal is drawn beside the new one's row")
        XCTAssertTrue(store.writing.isEmpty)
        XCTAssertEqual(store.phase, .loading)
    }

    func test_aFolderWriteTheOldDaemonAnswersAfterAttachIsDropped() async {
        let folder = TracesTree.FolderNode(id: "p1", label: "docs", mode: .ask, offerableModes: [.ask, .ignore], sessions: [])
        let store = await staleWrite { await $0.setFolderMode(folder, .ignore, promised: 3) }
        XCTAssertNil(store.folderNotice, "the old daemon's notice is drawn on the new one's tab")
        XCTAssertTrue(store.writeErrors.isEmpty)
        XCTAssertTrue(store.writing.isEmpty)
        XCTAssertEqual(store.phase, .loading)
    }

    // MARK: The live client's provisional methods (notAvailableYet)

    /// The live client throws `notAvailableYet` for the provisional methods
    /// (Zaki's C3). The stores draw those as absent, never as a failure
    /// that hides the rest and never as zero or healthy.
    func test_theInferenceStoreDrawsAProvisionalSummaryAsAbsent() async {
        let store = InferenceStore(client: LiveDaemonClient(transport: SampleTransport(.normalDay)))
        await store.load()
        XCTAssertNil(store.summary)
        XCTAssertNil(store.failures["inference_summary"])
        XCTAssertNotNil(store.calls, "the calls stand without the summary")
    }

    func test_theHomeStoreDrawsAProvisionalCatalogueAsAbsent() async {
        let store = HomeStore(client: LiveDaemonClient(transport: SampleTransport(.normalDay)))
        await store.load()
        XCTAssertNil(store.missions)
        XCTAssertNil(store.failures["mission_catalogue"])
        XCTAssertEqual(MissionFormat.count(store.missions), "—", "an unread catalogue is a dash, never 0")
        XCTAssertNotNil(store.status)
    }

    func test_theTracesStoreLoadsOverTheLiveClient() async {
        let store = TracesStore(client: LiveDaemonClient(transport: SampleTransport(.normalDay)))
        await store.load()
        XCTAssertEqual(store.phase, .loaded)
    }
}

/// Answers `tc_call` from a sample set's daemon-shaped replies, as the
/// daemon would, so a store reads through the real `LiveDaemonClient`.
private final class SampleTransport: DaemonTransport, @unchecked Sendable {
    let set: SampleDaemonClient.SampleSet

    init(_ set: SampleDaemonClient.SampleSet) {
        self.set = set
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        SampleDaemonData.reply(method, in: set).map { #"{"id":0,"result":\#($0)}"# }
            ?? #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}

/// A `SampleTransport` whose every call blocks (on the live client's work
/// queue, not the main actor) until `open()`, so a test can attach a new
/// client while the old one's reads are still outstanding.
private final class GatedTransport: DaemonTransport, @unchecked Sendable {
    private let inner: SampleTransport
    private let entered: XCTestExpectation
    private let gate = DispatchSemaphore(value: 0)

    init(_ set: SampleDaemonClient.SampleSet, entered: XCTestExpectation) {
        inner = SampleTransport(set)
        self.entered = entered
    }

    /// Lets every waiting call, and every later one, through.
    func open() {
        gate.signal()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        entered.fulfill()
        gate.wait()
        // Pass the opening on to the next waiting call.
        gate.signal()
        return inner.call(method, params: paramsJSON)
    }
}
