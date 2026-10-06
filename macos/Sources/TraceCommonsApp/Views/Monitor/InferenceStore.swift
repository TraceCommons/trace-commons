#if DEBUG
import Foundation
import Observation
import TCBridge
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
    /// `tool_destinations`: its per-tool counts (K14) are the core's call
    /// totals for the window, which one page of calls is not.
    private(set) var destinations: DaemonData.ToolDestinations?
    /// The Private AI switch, as the daemon's settings echo it. Only a read
    /// or a write the core's rule confirmed is kept here: a write that was
    /// not confirmed never moves it, so the switch never reads as on before
    /// the daemon said so.
    private(set) var privateAI: DaemonData.PrivateAISwitch?
    /// A switch write is in flight; the switch takes no other.
    private(set) var privateAIBusy = false
    /// The core's words for a write that was not confirmed
    /// (`PrivateInferenceCopy.writeUnconfirmed`); cleared by the next
    /// confirmed write or by the person.
    private(set) var privateAIRefusal: String?
    /// Bumped when a switch write starts and when it answers. A switch read
    /// that started under an older value answers about the switch before
    /// that write, and is dropped rather than drawn over its result.
    private var privateAIWrites = 0
    /// The last read that failed, by method; cleared when it next succeeds.
    private(set) var failures: [String: DaemonDataError] = [:]

    /// One page: the tab shows the newest calls, not the whole ledger.
    static let pageSize = 50

    /// The app's live client (`AppModel.daemonData`), attached by the
    /// window when the daemon starts; nil while it is not running.
    private(set) var client: (any DaemonDataClient)?
    /// Bumped by each attach; a read that started against an older client
    /// is dropped when it answers, so it never writes over the new one.
    private var generation = 0
    /// True while no client is attached because the daemon is still
    /// starting (set by `attach`). `run` then reads nothing and the screen
    /// stays loading: start-up is not the core being down. Any other nil
    /// client fails as an unreachable core.
    private(set) var awaiting = false

    init(client: (any DaemonDataClient)?) {
        self.client = client
    }

    /// Follows a new client (or none): nothing read from the old one is
    /// drawn as current, so the tab is loading until the new one is read.
    func attach(_ client: (any DaemonDataClient)?, awaiting: Bool = false) {
        self.client = client
        self.awaiting = awaiting && client == nil
        generation += 1
        harnesses = nil
        calls = nil
        summary = nil
        destinations = nil
        privateAI = nil
        privateAIRefusal = nil
        // A write to the old client is dropped when it answers, so it must
        // not hold the new client's switch.
        privateAIBusy = false
        failures = [:]
    }

    /// Loads, then follows the event stream for as long as the calling task
    /// runs (a view's `.task`), as `TracesStore.run()` does. A new call
    /// rereads the calls page; a status change or resync rereads it all.
    /// With no client the core is down: every read says so.
    func run() async {
        // The daemon is still starting: nothing to read yet, and not down.
        guard !awaiting else { return }
        await load()
        guard let client else { return }
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .inferenceCallAdded:
                async let calls: Void = loadCalls()
                async let destinations: Void = loadDestinations()
                _ = await (calls, destinations)
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
        async let destinations: Void = loadDestinations()
        async let privateAI: Void = loadPrivateAI()
        _ = await (harnesses, calls, summary, destinations, privateAI)
    }

    private func loadPrivateAI() async {
        let startedAt = beginPrivateAIRead()
        await read("private_ai", { try await $0.privateAI() }) { self.landPrivateAIRead($0, startedAt: startedAt) }
    }

    /// The write count a switch read starts under.
    func beginPrivateAIRead() -> Int { privateAIWrites }

    /// A switch read's answer, kept only when no write started or answered
    /// since the read began: an older read never undoes a confirmed write.
    /// A failed read (nil) keeps the last value.
    func landPrivateAIRead(_ value: DaemonData.PrivateAISwitch?, startedAt: Int) {
        guard let value, startedAt == privateAIWrites else { return }
        privateAI = value
    }

    /// Turns Private AI on or off. The reply is taken only when the core's
    /// rule `check` (`TCPrivateInference.writeConfirmed`) confirms it;
    /// otherwise, or when the write fails, `unconfirmed` is shown and the
    /// switch is read again, so it stands where the daemon has it. A write
    /// that threw is recorded under `set_private_ai`.
    func setPrivateAI(
        on: Bool, unconfirmed: String?,
        check: (Bool?, Bool?, Bool?) -> Bool = TCPrivateInference.writeConfirmed
    ) async {
        guard !privateAIBusy else { return }
        guard let client else {
            failures["set_private_ai"] = .unreachable
            privateAIRefusal = unconfirmed
            return
        }
        let mine = generation
        privateAIBusy = true
        privateAIWrites += 1
        let result: Result<DaemonData.PrivateAISwitch, DaemonDataError>
        do {
            result = .success(try await client.setPrivateAI(on: on))
        } catch {
            result = .failure(error as? DaemonDataError ?? .undecodable(method: "set_private_ai"))
        }
        guard mine == generation else { return }
        privateAIBusy = false
        privateAIWrites += 1
        switch result {
        case .success(let reply) where Self.confirmed(reply, requested: on, check: check):
            failures["set_private_ai"] = nil
            privateAIRefusal = nil
            privateAI = reply
        case .success:
            failures["set_private_ai"] = nil
            privateAIRefusal = unconfirmed
            await loadPrivateAI()
        case .failure(let error):
            failures["set_private_ai"] = error
            privateAIRefusal = unconfirmed
            await loadPrivateAI()
        }
    }

    /// Puts the refusal away. It does not retry the write.
    func dismissPrivateAIRefusal() {
        privateAIRefusal = nil
    }

    /// Whether a write's reply confirms it, by the core's rule `check`,
    /// asked (requested, echoed marker, echoed switch).
    static func confirmed(
        _ reply: DaemonData.PrivateAISwitch, requested: Bool, check: (Bool?, Bool?, Bool?) -> Bool
    ) -> Bool {
        check(requested, reply.offerSeen, reply.on)
    }

    private func loadDestinations() async {
        await read("tool_destinations", { try await $0.toolDestinations() }) { self.destinations = $0 }
    }

    private func loadHarnesses() async {
        await read("harness_list", { try await $0.harnessList() }) { if let value = $0 { self.harnesses = value } }
    }

    func loadCalls() async {
        await read("inference_calls", { try await $0.inferenceCalls(limit: Self.pageSize, cursor: nil) }) {
            if let value = $0 { self.calls = value }
        }
    }

    private func loadSummary() async {
        await read("inference_summary", { try await $0.inferenceSummary() }) { if let value = $0 { self.summary = value } }
    }

    /// One read, recording its failure by method, then `apply` with its
    /// value (nil when it failed or is not there). A provisional method the
    /// live client does not have yet is not a failure to show: it is simply
    /// not there. A read answered after another client was attached
    /// changes nothing.
    private func read<T: Sendable>(
        _ method: String, _ call: @Sendable (any DaemonDataClient) async throws -> T, apply: (T?) -> Void
    ) async {
        guard let client else {
            failures[method] = .unreachable
            apply(nil)
            return
        }
        let mine = generation
        let result: Result<T, DaemonDataError>
        do {
            result = .success(try await call(client))
        } catch {
            result = .failure(error as? DaemonDataError ?? .undecodable(method: method))
        }
        guard mine == generation else { return }
        switch result {
        case .success(let value):
            failures[method] = nil
            apply(value)
        case .failure(.notAvailableYet):
            failures[method] = nil
            apply(nil)
        case .failure(let error):
            failures[method] = error
            apply(nil)
        }
    }
}
#endif
