import Foundation
import Observation
import TCShellCore

/// Current account limits remain separate from analytics storage and its week.
/// The only credential-capable operations are injected daemon calls.
@Observable @MainActor
final class SubscriptionUsageModel {
    typealias Reader = @Sendable (Bool, Bool, String?) async throws -> DaemonData.SubscriptionUsage
    typealias Writer = @Sendable (String, Bool) async throws -> Void

    private let reader: Reader?
    private let writer: Writer?
    private var generation = UUID()
    private var reading: UUID?
    private var readingGeneration: UUID?
    private var queuedRefresh = false
    private var queuedInteractive = false
    private var queuedProvider: String?
    private var observation: UUID?
    private(set) var shown: DaemonData.SubscriptionUsage?
    private(set) var failureKey: String?
    private(set) var unsupported = false
    private(set) var changingProviders = Set<String>()

    var isVisible: Bool { reader != nil && !unsupported && (shown != nil || failureKey != nil) }
    var isRefreshing: Bool {
        reading != nil || shown?.providers.contains(where: { $0.enabled && $0.refreshing }) == true
    }
    var canChange: Bool { writer != nil }

    init(reader: Reader? = nil, writer: Writer? = nil) {
        self.reader = reader
        self.writer = writer
    }

    /// The view's task owns polling and cancellation. Only an explicit button
    /// passes interactive:true; opening Overview cannot prompt for Keychain.
    func observe() async {
        let token = UUID()
        observation = token
        generation = UUID()
        clearQueuedRefresh()
        await withTaskCancellationHandler {
            defer { endObservation(token) }
            while !Task.isCancelled, observation == token {
                await load()
                if unsupported || reader == nil { return }
                do {
                    try await Task.sleep(for: .seconds(isRefreshing ? 2 : 60))
                } catch { return }
            }
        } onCancel: {
            // A transport may ignore cancellation while waiting on IPC. End
            // the view's authority without waiting for that reply to arrive.
            Task { @MainActor [weak self] in self?.endObservation(token) }
        }
    }

    private func endObservation(_ token: UUID) {
        guard observation == token else { return }
        observation = nil
        generation = UUID()
        reading = nil
        readingGeneration = nil
        clearQueuedRefresh()
    }

    private func clearQueuedRefresh() {
        queuedRefresh = false
        queuedInteractive = false
        queuedProvider = nil
    }

    /// Refreshes coalesce while a current read is in flight. A read invalidated
    /// by a setting change may be superseded without awaiting its old reply.
    func load(refresh: Bool = false, interactive: Bool = false, provider: String? = nil) async {
        guard let reader, changingProviders.isEmpty, !Task.isCancelled else { return }
        if reading != nil, readingGeneration == generation {
            if refresh {
                queuedProvider = queuedRefresh && queuedProvider != provider ? nil : provider
            }
            queuedRefresh = queuedRefresh || refresh
            queuedInteractive = queuedInteractive || (refresh && interactive)
            return
        }
        let request = UUID()
        let token = generation
        reading = request
        readingGeneration = token
        defer {
            if reading == request {
                reading = nil
                readingGeneration = nil
                let pending = queuedRefresh
                let prompt = queuedInteractive
                let selected = queuedProvider
                clearQueuedRefresh()
                if pending, generation == token, !Task.isCancelled {
                    Task { [weak self] in
                        guard let self, self.generation == token else { return }
                        await self.load(refresh: true, interactive: prompt, provider: selected)
                    }
                }
            }
        }
        do {
            let value = try await reader(refresh, refresh && interactive, provider)
            guard generation == token, !Task.isCancelled else { return }
            guard value.schemaVersion == "trace_commons.subscription_usage.v1" else {
                shown = nil
                failureKey = "analytics_subscriptions_unavailable"
                return
            }
            shown = value
            unsupported = false
            failureKey = nil
        } catch {
            guard generation == token, !Task.isCancelled else { return }
            shown = nil
            unsupported = (error as? DaemonDataError)?.isNotServed == true
            failureKey = unsupported ? nil : "analytics_subscriptions_unavailable"
        }
    }

    func refresh() {
        guard !isRefreshing else { return }
        let token = generation
        Task { [weak self] in
            guard let self, self.generation == token else { return }
            await self.load(refresh: true, interactive: true)
        }
    }

    /// Hide disabled data immediately, and invalidate any read from before the
    /// user's choice. The daemon remains authoritative when the write returns.
    func setEnabled(_ provider: String, on: Bool) async {
        guard let writer, provider == "codex" || provider == "claude",
              changingProviders.isEmpty else { return }
        let startedObservation = observation
        generation = UUID()
        clearQueuedRefresh()
        changingProviders.insert(provider)
        if !on, let shown {
            self.shown = DaemonData.SubscriptionUsage(schemaVersion: shown.schemaVersion, providers: shown.providers.map { row in
                row.provider == provider
                    ? DaemonData.SubscriptionUsageProvider(provider: provider, enabled: false)
                    : row
            }, truncated: shown.truncated)
        }
        defer { changingProviders.remove(provider) }
        do {
            try await writer(provider, on)
            guard !Task.isCancelled,
                  startedObservation == nil || observation == startedObservation else { return }
            changingProviders.remove(provider)
            generation = UUID()
            await load(refresh: on, interactive: on, provider: provider)
        } catch {
            guard !Task.isCancelled,
                  startedObservation == nil || observation == startedObservation else { return }
            changingProviders.remove(provider)
            generation = UUID()
            failureKey = "analytics_subscriptions_change_failed"
            // Reconcile actual state after a refused write without prompting.
            let token = generation
            if let reader, let actual = try? await reader(false, false, nil), generation == token {
                shown = actual
            }
        }
    }
}
