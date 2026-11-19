import Foundation
import TCShellCore
extension AppModel {
    func refreshFeatureFlags() {
        guard let client = featureFlagClient else { featureFlags = nil; return }
        featureFlagReadGeneration &+= 1
        let requestGeneration = featureFlagReadGeneration
        Task.detached(priority: .utility) {
            let read = Result { try client.featureFlagsSnapshot() }
            let snapshot = try? read.get()
            switch read {
            case .success(let s): FeatureBreadcrumb.flagsRead(claudeDesktop: s.isEnabled("claude_desktop_private_ai"), source: s.source.rawValue)
            case .failure(let error): FeatureBreadcrumb.flagsFailed(error)
            }
            await MainActor.run {
                guard self.featureFlagClient === client, self.featureFlagReadGeneration == requestGeneration else { return }
                guard let snapshot else { self.featureFlags = nil; return }
                if let old = self.featureFlags, !old.acceptsSuccessor(snapshot) { return }
                self.featureFlags = snapshot
            }
        }
    }
    func refreshFeatureFlagsAfterWake() {
        guard let client = featureFlagClient else { return }
        Task.detached(priority: .utility) { try? client.requestFeatureFlagsRefresh() }
        refreshFeatureFlags()
    }
    func featureIsEnabled(_ id: String) -> Bool { featureFlags?.isEnabled(id) ?? false }
}
