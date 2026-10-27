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
    private var observationSleep: Task<Void, Never>?
    private let activePollInterval: Duration
    private(set) var codexEnabled: Bool?
    private(set) var shown: DaemonData.SubscriptionUsage?
    private(set) var failureKey: String?
    private(set) var unsupported = false
    private(set) var changingProviders = Set<String>()

    var supportedProviders: [DaemonData.SubscriptionUsageProvider] {
        (shown?.providers ?? []).filter { $0.provider == "codex" && $0.enabled }
    }
    var isVisible: Bool { reader != nil && !unsupported && codexEnabled == true }
    var isRefreshing: Bool {
        reading != nil || supportedProviders.contains(where: \.refreshing)
    }
    var canChange: Bool { writer != nil }

    init(reader: Reader? = nil, writer: Writer? = nil, activePollInterval: Duration = .seconds(2)) {
        self.reader = reader
        self.writer = writer
        self.activePollInterval = activePollInterval
    }

    /// The view's task owns polling and cancellation. Only an explicit button
    /// passes interactive:true; opening Overview cannot prompt for Keychain.
    func observe() async {
        let token = UUID()
        observationSleep?.cancel()
        observation = token
        generation = UUID()
        clearQueuedRefresh()
        await withTaskCancellationHandler {
            defer { endObservation(token) }
            while !Task.isCancelled, observation == token {
                await load()
                guard !Task.isCancelled, observation == token else { return }
                if unsupported || reader == nil { return }
                let delay: Duration = isRefreshing ? activePollInterval : .seconds(60)
                let sleeper = Task<Void, Never> { _ = try? await Task.sleep(for: delay) }
                observationSleep = sleeper
                await sleeper.value
                guard observation == token else { return }
                observationSleep = nil
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
        observationSleep?.cancel()
        observationSleep = nil
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
                observationSleep?.cancel()
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
            codexEnabled = value.providers.first(where: { $0.provider == "codex" })?.enabled
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
        guard codexEnabled == true, !isRefreshing else { return }
        let token = generation
        Task { [weak self] in
            guard let self, self.generation == token else { return }
            await self.load(refresh: true, interactive: true, provider: "codex")
            // Resume status polling immediately after an explicit request,
            // even when the observer was in its minute-long idle wait.
            self.observationSleep?.cancel()
        }
    }

    /// Hide disabled data immediately, and invalidate any read from before the
    /// user's choice. The daemon remains authoritative when the write returns.
    func setEnabled(_ provider: String, on: Bool) async {
        guard let writer, provider == "codex",
              changingProviders.isEmpty else { return }
        let startedObservation = observation
        generation = UUID()
        clearQueuedRefresh()
        changingProviders.insert(provider)
        if !on { codexEnabled = false }
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
            // Consent only permits a later explicit Refresh. Reconcile the
            // saved permission without launching any credential operation.
            await load()
            observationSleep?.cancel()
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
                codexEnabled = actual.providers.first(where: { $0.provider == "codex" })?.enabled
            }
            observationSleep?.cancel()
        }
    }
}
