import AppKit
import Combine
import Foundation
import Observation
import TCShellCore

extension DaemonClient {
    func claudeDesktopStatus() throws -> ClaudeDesktopStatus {
        try call("claude_desktop_status", as: ClaudeDesktopStatus.self).withoutEnvironment()
    }

    func claudeDesktopConfigure(model: String, backgroundModel: String?) throws -> ClaudeDesktopStatus {
        var params: [String: Any] = ["model": model]
        if let backgroundModel { params["background_model"] = backgroundModel }
        return try call("claude_desktop_configure", params: params, as: ClaudeDesktopStatus.self)
    }

    func claudeDesktopPreset() throws -> ClaudeDesktopStatus {
        try call("claude_desktop_preset", as: ClaudeDesktopStatus.self)
    }

    func claudeDesktopDisconnect() throws -> ClaudeDesktopStatus {
        try call("claude_desktop_disconnect", as: ClaudeDesktopStatus.self).withoutEnvironment()
    }
}

/// Owned by the Settings tools row, never AppModel. Environment text exists
/// only while its setup form is open and is copied only by the Copy action.
@MainActor @Observable
final class ClaudeDesktopSetupModel {
    static let featureID = "claude_desktop_private_ai"

    private(set) var status: ClaudeDesktopStatus?
    private(set) var busy = false
    private(set) var failed = false
    private(set) var isPresented = false
    private(set) var environment: String?
    private(set) var catalogue: ManagedModelCatalogue?
    private(set) var catalogueBusy = false
    var selectedModel = ""
    var selectedBackgroundModel = ""

    @ObservationIgnored private let client: DaemonClient
    @ObservationIgnored private let features: AppModel
    @ObservationIgnored private var featureSubscription: AnyCancellable?
    private(set) var featureEnabled = false
    @ObservationIgnored private var presentation = UUID()
    @ObservationIgnored private var catalogueRead = UUID()

    init(client: DaemonClient, features: AppModel) {
        self.client = client
        self.features = features
        featureSubscription = features.$featureFlags.sink { [weak self] snapshot in
            // AppModel is MainActor and publishes synchronously in willSet.
            // Read the incoming decision so disable clears the preset before
            // the next SwiftUI render or detached response can run.
            MainActor.assumeIsolated {
                self?.featureDecisionChanged(snapshot?.isEnabled(Self.featureID) ?? false)
            }
        }
    }

    private var isEnabled: Bool {
        featureEnabled && features.featureIsEnabled(Self.featureID)
    }

    private func featureDecisionChanged(_ enabled: Bool) {
        featureEnabled = enabled
        guard !enabled else { return }
        close()
        status = nil
        failed = false
    }

    var resolvedModel: String? {
        selectedModel.isEmpty ? catalogue?.default : selectedModel
    }

    var canSave: Bool {
        guard isEnabled, isPresented, !busy, !catalogueBusy, status?.canConfigure == true,
              let resolvedModel else { return false }
        return catalogue?.models.contains { $0.id == resolvedModel } == true
    }

    var modelNames: [String: String] { ManagedSurface.modelNames(catalogue) }

    func readCachedCatalogue(_ catalogue: ManagedModelCatalogue?) {
        guard isEnabled, !isPresented, let catalogue else { return }
        self.catalogue = catalogue
    }

    func open() {
        guard isEnabled, status != nil, !busy else { return }
        presentation = UUID()
        isPresented = true
        environment = nil
        failed = false
        selectedModel = status?.model ?? ""
        selectedBackgroundModel = status?.backgroundModel ?? ""
    }

    func close() {
        isPresented = false
        environment = nil
        presentation = UUID()
        catalogueRead = UUID()
        catalogueBusy = false
    }

    /// Called only from the visible row's task. One request at a time,
    /// including writes; the daemon bounds its credential/catalogue reads.
    func refresh() async {
        guard isEnabled, !busy else { return }
        busy = true
        let token = presentation
        defer { busy = false }
        let client = client
        let result = await Task.detached(priority: .utility) {
            try? client.claudeDesktopStatus()
        }.value
        guard !Task.isCancelled, isEnabled, presentation == token else { return }
        guard let result else {
            close()
            status = nil
            return
        }
        accept(result)
    }

    /// The catalogue loader is injected by the view from AppModel's
    /// existing path. Late callbacks from a dismissed form are ignored.
    func loadCatalogue(using load: (@escaping @MainActor (ManagedModelCatalogue?) -> Void) -> Void) {
        guard isEnabled, isPresented, !catalogueBusy else { return }
        catalogueBusy = true
        let token = UUID()
        catalogueRead = token
        load { [weak self] catalogue in
            guard let self, self.isEnabled, self.isPresented, self.catalogueRead == token else { return }
            self.catalogueBusy = false
            self.catalogue = catalogue
            if catalogue == nil { self.failed = true }
            self.reconcileBackground()
        }
    }

    func chooseModel(_ value: String) {
        guard isEnabled else { return }
        selectedModel = value
        environment = nil
        reconcileBackground()
    }

    func chooseBackground(_ value: String) {
        guard isEnabled else { return }
        selectedBackgroundModel = value
        environment = nil
    }

    private func reconcileBackground() {
        selectedBackgroundModel = ManagedSurface.reconciledBackground(
            selectedBackgroundModel.isEmpty ? nil : selectedBackgroundModel,
            main: resolvedModel, in: catalogue) ?? ""
    }

    func save() async {
        guard canSave, let model = resolvedModel else { return }
        let background = selectedBackgroundModel.isEmpty ? nil : selectedBackgroundModel
        await requestEnvironment { try $0.claudeDesktopConfigure(model: model, backgroundModel: background) }
    }

    /// Fetch an existing route's preset only after an explicit Copy action
    /// in a newly opened form. Opening or polling never exports secrets.
    func copyEnvironment(write: @MainActor (String) -> Void = ClaudeDesktopSetupModel.writeClipboard) async {
        guard isEnabled, isPresented, !busy else { return }
        let token = presentation
        if environment == nil {
            guard selectionsMatchStatus else { return }
            await requestEnvironment { try $0.claudeDesktopPreset() }
        }
        guard isEnabled, isPresented, presentation == token, let environment else { return }
        write(environment)
    }

    var canCopy: Bool {
        isEnabled && isPresented && !busy && (environment != nil || selectionsMatchStatus)
    }

    private var selectionsMatchStatus: Bool {
        guard let status, [.ready, .observed].contains(status.state) else { return false }
        return resolvedModel == status.model
            && (selectedBackgroundModel.isEmpty ? nil : selectedBackgroundModel) == status.backgroundModel
    }

    func disconnect() async {
        guard isEnabled, !busy, status?.canDisconnect == true else { return }
        environment = nil
        presentation = UUID()
        let token = presentation
        busy = true
        failed = false
        defer { busy = false }
        let client = client
        let result = await Task.detached(priority: .userInitiated) {
            try? client.claudeDesktopDisconnect()
        }.value
        guard !Task.isCancelled, isEnabled, presentation == token else { return }
        if let result { accept(result) }
        else { failed = true }
    }

    private func requestEnvironment(
        _ operation: @escaping @Sendable (DaemonClient) throws -> ClaudeDesktopStatus
    ) async {
        guard isEnabled, isPresented, !busy else { return }
        busy = true
        failed = false
        environment = nil
        let token = presentation
        let client = client
        defer { busy = false }
        let result = await Task.detached(priority: .userInitiated) { try? operation(client) }.value
        guard !Task.isCancelled, isEnabled, isPresented, presentation == token else { return }
        guard let result else { failed = true; return }
        accept(result)
        selectedModel = result.model ?? selectedModel
        selectedBackgroundModel = result.backgroundModel ?? ""
        environment = result.environment
        if environment == nil { failed = true }
    }

    private func accept(_ result: ClaudeDesktopStatus) {
        let next = result.withoutEnvironment()
        if status?.state != next.state || status?.model != next.model
            || status?.backgroundModel != next.backgroundModel {
            environment = nil
        }
        status = next
    }

    private static func writeClipboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}
