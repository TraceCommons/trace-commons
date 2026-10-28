import Foundation
import Observation
import TCShellCore

/// Account limits remain separate from analytics storage and its week.
/// The only credential-capable operations are injected daemon calls.
@Observable @MainActor
final class SubscriptionUsageModel {
    typealias Reader = @Sendable (Bool, Bool, String?, String?) async throws -> DaemonData.SubscriptionUsage
    typealias Writer = @Sendable (String, Bool) async throws -> Void

    private let reader: Reader?
    private let writer: Writer?
    private var generation = UUID()
    private var reading: UUID?
    private var readingGeneration: UUID?
    private var queuedRefresh = false
    private var queuedInteractive = false
    private var queuedProvider: String?
    private var queuedLogin: String?
    private var refreshingProvider: String?
    private var refreshingLogin: String?
    private var observation: UUID?
    private var observationSleep: Task<Void, Never>?
    private let activePollInterval: Duration
    private(set) var codexEnabled: Bool?
    private(set) var claudeEnabled: Bool?
    private var claudeAvailable = false
    private(set) var shown: DaemonData.SubscriptionUsage?
    private(set) var failureKey: String?
    private(set) var unsupported = false
    private(set) var changingProviders = Set<String>()

    var supportedProviders: [DaemonData.SubscriptionUsageProvider] {
        (shown?.providers ?? []).filter {
            ($0.provider == "codex" || $0.provider == "claude") && $0.enabled
                && ($0.provider == "codex" || !$0.logins.isEmpty)
        }
    }
    var isVisible: Bool { reader != nil && !unsupported && (codexEnabled == true || claudeAvailable) }
    var isRefreshing: Bool {
        reading != nil || supportedProviders.contains { $0.refreshing || $0.logins.contains { $0.refreshing == true } }
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
        refreshingProvider = nil
        refreshingLogin = nil
        clearQueuedRefresh()
    }

    private func clearQueuedRefresh() {
        queuedRefresh = false
        queuedInteractive = false
        queuedProvider = nil
        queuedLogin = nil
    }

    /// Refreshes coalesce while a current read is in flight. A read invalidated
    /// by a setting change may be superseded without awaiting its old reply.
    func load(refresh: Bool = false, interactive: Bool = false, provider: String? = nil, login: String? = nil) async {
        guard let reader, changingProviders.isEmpty, !Task.isCancelled else { return }
        if reading != nil, readingGeneration == generation {
            if refresh {
                // Keep the exact scope; coalescing must never fan out.
                queuedProvider = provider
                queuedLogin = login
                queuedInteractive = interactive
            }
            queuedRefresh = queuedRefresh || refresh
            return
        }
        let request = UUID()
        let token = generation
        reading = request
        readingGeneration = token
        refreshingProvider = refresh ? (provider ?? "codex") : nil
        refreshingLogin = refresh ? login : nil
        defer {
            if reading == request {
                reading = nil
                readingGeneration = nil
                refreshingProvider = nil
                refreshingLogin = nil
                observationSleep?.cancel()
                let pending = queuedRefresh
                let prompt = queuedInteractive
                let selected = queuedProvider
                let selectedLogin = queuedLogin
                clearQueuedRefresh()
                if pending, generation == token, !Task.isCancelled {
                    Task { [weak self] in
                        guard let self, self.generation == token else { return }
                        await self.load(refresh: true, interactive: prompt, provider: selected, login: selectedLogin)
                    }
                }
            }
        }
        do {
            let value = try await reader(refresh, refresh && interactive, provider, login)
            guard generation == token, !Task.isCancelled else { return }
            guard value.schemaVersion == "trace_commons.subscription_usage.v1" else {
                shown = nil
                failureKey = "analytics_subscriptions_unavailable"
                return
            }
            shown = value
            reconcile(value)
            unsupported = false
            failureKey = nil
        } catch {
            guard generation == token, !Task.isCancelled else { return }
            shown = nil
            unsupported = (error as? DaemonDataError)?.isNotServed == true
            failureKey = unsupported ? nil : "analytics_subscriptions_unavailable"
        }
    }

    /// The default action retains Codex-only behavior. Claude always
    /// requires the exact local login selected by its row.
    func refresh(provider: String = "codex", login: String? = nil) {
        guard canRefresh(provider: provider, login: login) else { return }
        let token = generation
        Task { [weak self] in
            guard let self, self.generation == token else { return }
            self.observationSleep?.cancel()
            await self.load(refresh: true, interactive: true, provider: provider, login: login)
            // Wake polling while the worker awaits a Keychain decision.
            self.observationSleep?.cancel()
        }
    }

    func canRefresh(provider: String, login: String?) -> Bool {
        guard changingProviders.isEmpty,
              !supportedProviders.contains(where: { $0.refreshing || $0.logins.contains { $0.refreshing == true } }),
              refreshingProvider == nil, !queuedRefresh,
              let row = supportedProviders.first(where: { $0.provider == provider }) else { return false }
        if provider == "claude" { return login.map { id in row.logins.contains { $0.id == id } } ?? false }
        return provider == "codex" && (login == nil || row.logins.contains { $0.id == login })
    }

    func isRefreshing(provider: String, login: String?) -> Bool {
        if refreshingProvider == provider && refreshingLogin == login { return true }
        if queuedRefresh && queuedProvider == provider && queuedLogin == login { return true }
        guard let row = supportedProviders.first(where: { $0.provider == provider }) else { return false }
        guard let login else { return row.refreshing }
        guard let entry = row.logins.first(where: { $0.id == login }) else { return false }
        return entry.refreshing ?? row.refreshing
    }

    private func reconcile(_ value: DaemonData.SubscriptionUsage) {
        codexEnabled = value.providers.first(where: { $0.provider == "codex" })?.enabled
        let claude = value.providers.first(where: { $0.provider == "claude" })
        claudeEnabled = claude?.enabled
        claudeAvailable = claude.map { $0.enabled && !$0.logins.isEmpty } ?? false
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
        if !on, provider == "codex" { codexEnabled = false }
        if !on, provider == "claude" { claudeEnabled = false; claudeAvailable = false }
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
            if let reader, let actual = try? await reader(false, false, nil, nil), generation == token {
                shown = actual
                reconcile(actual)
            }
            observationSleep?.cancel()
        }
    }
}
