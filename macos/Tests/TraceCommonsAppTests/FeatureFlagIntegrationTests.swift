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
    @MainActor
    func testGapEventsReplaceStaleEnabledSnapshotWithDaemonDecision() async throws {
        let dir = URL(fileURLWithPath: "/private/tmp/tc-fgap-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = AppModel()
        defer { model.shutdown() }
        let started = expectation(description: "daemon running")
        model.startDaemon(at: dir.path, settingsJSON: #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#) { _ in started.fulfill() }
        await fulfillment(of: [started], timeout: 10)
        for _ in 0..<100 {
            if model.featureFlags != nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        let current = try XCTUnwrap(model.featureFlags)
        // Simulate an enabled snapshot left from an earlier daemon instance after
        // losing a disable/expiry update. The authoritative daemon has it off.
        let stale: [String: Any] = [
            "schema_version": 1, "environment": current.environment,
            "profile_digest": current.profileDigest, "instance_id": UUID().uuidString,
            "generation": 1, "sampled_at": current.sampledAt, "source": "remote",
            "revision": 1, "expires_at": "2026-11-05T00:00:00Z", "fresh": true,
            "flags": ["missed-disable-fixture": ["enabled": true, "pending": true,
                "capability_blocked": false, "timing": "live"]]
        ]
        let encoded = try JSONSerialization.data(withJSONObject: stale)
        for event in [DaemonEvent.resyncRequired, .lagged(skipped: 1)] {
            model.featureFlags = try JSONDecoder().decode(FeatureFlagSnapshot.self, from: encoded)
            XCTAssertTrue(model.featureIsEnabled("missed-disable-fixture"))
            model.handle(event: event)
            for _ in 0..<100 {
                if !model.featureIsEnabled("missed-disable-fixture") { break }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTAssertFalse(model.featureIsEnabled("missed-disable-fixture"), "gap recovery must reread flags for \(event)")
        }
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
