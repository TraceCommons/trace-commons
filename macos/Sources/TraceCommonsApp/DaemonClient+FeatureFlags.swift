import Foundation
import TCBridge
import TCShellCore
extension DaemonClient {
    func featureFlagsSnapshot() throws -> FeatureFlagSnapshot {
        let snapshot = try call("feature_flags_snapshot", as: FeatureFlagSnapshot.self)
        guard let json = TCFeatureFlags.buildProfileJSON(),
              let identity = try? JSONDecoder().decode(BuildFeatureProfile.self, from: Data(json.utf8)),
              identity.agrees(environment: snapshot.environment, digest: snapshot.profileDigest)
        else { throw Failure(code: "unavailable", message: "feature-profile-mismatch") }
        return snapshot
    }
    func requestFeatureFlagsRefresh() throws {
        struct Reply: Decodable { let requested: Bool }
        _ = try call("feature_flags_refresh", as: Reply.self)
    }
}
