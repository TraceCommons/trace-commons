import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Tests exercise the shipped snapshot decoder and AppModel gate with the
/// actual build identity, rather than bypassing Desktop's feature decision.
@MainActor
enum DesktopFeatureFixture {
    static func snapshot(enabled: Bool?, generation: UInt64 = 1) throws -> FeatureFlagSnapshot {
        let profile = try JSONDecoder().decode(BuildFeatureProfile.self,
            from: Data(try XCTUnwrap(TCFeatureFlags.buildProfileJSON()).utf8))
        var flags: [String: Any] = [:]
        if let enabled {
            flags[ClaudeDesktopSetupModel.featureID] = [
                "enabled": enabled, "pending": enabled, "capability_blocked": false, "timing": "live",
            ]
        }
        let object: [String: Any] = [
            "schema_version": 1, "environment": profile.environment, "profile_digest": profile.profileDigest,
            "instance_id": "edc1ac4a-798f-4f19-b186-3587122523be", "generation": generation,
            "sampled_at": ISO8601DateFormatter().string(from: Date()), "source": "remote", "revision": generation,
            "fresh": true, "flags": flags,
        ]
        return try JSONDecoder().decode(FeatureFlagSnapshot.self,
            from: JSONSerialization.data(withJSONObject: object))
    }

    static func app(client: DaemonClient, enabled: Bool?) throws -> AppModel {
        let app = AppModel()
        app.setClientForTesting(client)
        app.featureFlags = try snapshot(enabled: enabled)
        return app
    }

    static func enabledSetup(client: DaemonClient) throws -> ClaudeDesktopSetupModel {
        let app = try app(client: client, enabled: true)
        return try XCTUnwrap(app.makeClaudeDesktopSetup())
    }
}
