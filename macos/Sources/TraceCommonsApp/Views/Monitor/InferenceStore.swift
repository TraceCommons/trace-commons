#if DEBUG
import Foundation
import Observation
import TCShellCore

/// The map's Private AI view and the Inference tab's data (R8 of #1173),
/// read through `DaemonDataClient`.
///
/// Each read stands alone: a tool list the daemon could not read leaves
/// the calls standing, and the other way round. A failed read keeps the
/// last good value rather than drawing nothing, as `TracesStore` does.
@MainActor
@Observable
final class InferenceStore {
    /// `harness_list`: the tools and whether each sends calls here.
    private(set) var harnesses: HarnessList?
    /// The newest page of `inference_calls`.
    private(set) var calls: DaemonData.InferenceCallPage?
    /// The per-model summary. PROVISIONAL (Zaki's C3): the live client
    /// throws `notAvailableYet`, and the tab then shows the calls alone.
    private(set) var summary: DaemonData.InferenceSummary?
    /// The last read that failed, by method; cleared when it next succeeds.
    private(set) var failures: [String: DaemonDataError] = [:]

    /// One page: the tab shows the newest calls, not the whole ledger.
    static let pageSize = 50

    let client: any DaemonDataClient

    init(client: any DaemonDataClient) {
        self.client = client
    }

    /// Loads, then follows the event stream for as long as the calling task
    /// runs (a view's `.task`), as `TracesStore.run()` does. A new call
    /// rereads the calls page; a status change or resync rereads it all.
    func run() async {
        await load()
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .inferenceCallAdded:
                await loadCalls()
            case .snapshot, .statusChanged, .resyncRequired:
                await load()
            case .queueChanged, .digestDue, .previewReady, .unknown:
                break
            }
        }
    }

    func load() async {
        async let harnesses: Void = loadHarnesses()
        async let calls: Void = loadCalls()
        async let summary: Void = loadSummary()
        _ = await (harnesses, calls, summary)
    }

    private func loadHarnesses() async {
        if let value = await read("harness_list", { try await $0.harnessList() }) { harnesses = value }
    }

    func loadCalls() async {
        if let value = await read("inference_calls", { try await $0.inferenceCalls(limit: Self.pageSize, cursor: nil) }) {
            calls = value
        }
    }

    private func loadSummary() async {
        if let value = await read("inference_summary", { try await $0.inferenceSummary() }) { summary = value }
    }

    /// One read, recording its failure by method. A provisional method the
    /// live client does not have yet is not a failure to show: it is simply
    /// not there.
    private func read<T: Sendable>(
        _ method: String, _ call: @Sendable (any DaemonDataClient) async throws -> T
    ) async -> T? {
        do {
            let value = try await call(client)
            failures[method] = nil
            return value
        } catch let error as DaemonDataError {
            if case .notAvailableYet = error {
                failures[method] = nil
            } else {
                failures[method] = error
            }
            return nil
        } catch {
            failures[method] = .undecodable(method: method)
            return nil
        }
    }
}
#endif
