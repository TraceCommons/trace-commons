import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class FlagRefusingDaemon: DaemonCalling, Sendable {
    func call(_ method: String, params paramsJSON: String) -> String {
        #"{"id":1,"error":{"code":"unknown_method","message":"unknown-method"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
final class FeatureFlagIntegrationTests: XCTestCase {
    func testOlderDaemonDoesNotInventFeatureEnables() {
        XCTAssertThrowsError(try DaemonClient(daemon: FlagRefusingDaemon()).featureFlagsSnapshot())
    }
    @MainActor
    func testNoDaemonMeansDisabled() {
        let model = AppModel()
        XCTAssertFalse(model.featureIsEnabled("unknown"))
    }
    func testFeatureEventRequestsSnapshotRefresh() {
        XCTAssertEqual(DaemonEventParser.parse(#"{"event":"feature_flags_changed","data":{"generation":2}}"#), .featureFlagsChanged)
    }
    func testEmbeddedAndAttachedSnapshotsAgreeWithFreshFFI() throws {
        let dir = URL(fileURLWithPath: "/private/tmp/tc-flags-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let embedded = try TCDaemon(configDir: dir.path, settingsJSON: #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#)
        defer { embedded.shutdown() }
        let attached = try TCDaemon(attachingTo: dir.path)
        defer { attached.shutdown() }
        let a = try DaemonClient(daemon: embedded).featureFlagsSnapshot()
        let b = try DaemonClient(daemon: attached).featureFlagsSnapshot()
        XCTAssertEqual(a, b)
        XCTAssertFalse(a.isEnabled("unregistered-fixture"))
        let identity = try JSONDecoder().decode(BuildFeatureProfile.self, from: Data(XCTUnwrap(TCFeatureFlags.buildProfileJSON()).utf8))
        XCTAssertTrue(identity.agrees(environment: a.environment, digest: a.profileDigest))
    }
}
