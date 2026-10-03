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
        let sample = try XCTUnwrap(window.range(of: "static func sampleClient()"))
        let body = window[sample.lowerBound...].prefix(600)
        XCTAssertTrue(body.contains("#if DEBUG"), "sample data must be debug-only")
        XCTAssertTrue(body.contains("TRACE_COMMONS_SAMPLE"))
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
        traces.attach(SampleDaemonClient(.busyQueue))
        XCTAssertEqual(traces.phase, .loading)
        XCTAssertNil(traces.decisionsOwed, "the old daemon's count is not drawn as the new one's")

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
