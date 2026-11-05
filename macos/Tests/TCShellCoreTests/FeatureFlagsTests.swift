import Foundation
import XCTest
@testable import TCShellCore
final class FeatureFlagsTests: XCTestCase {
    func testSnapshotIsDaemonPolicyAndMissingFlagsAreOff() throws {
        let snapshot = try JSONDecoder().decode(FeatureFlagSnapshot.self, from: Data(Self.json.utf8))
        XCTAssertTrue(snapshot.isEnabled("fixture"))
        XCTAssertFalse(snapshot.isEnabled("unknown"))
        XCTAssertFalse(snapshot.acceptsSuccessor(snapshot))
        let newer = try JSONDecoder().decode(FeatureFlagSnapshot.self, from: Data(Self.json.replacingOccurrences(of: "\"generation\":1", with: "\"generation\":2").utf8))
        XCTAssertTrue(snapshot.acceptsSuccessor(newer))
        XCTAssertFalse(newer.acceptsSuccessor(snapshot))
    }
    func testMalformedSnapshotCannotEnableFlags() {
        for json in ["{}", Self.json.replacingOccurrences(of: "\"schema_version\":1", with: "\"schema_version\":2"), Self.json.replacingOccurrences(of: "\"enabled\":true", with: "\"enabled\":\"true\"")] {
            XCTAssertThrowsError(try JSONDecoder().decode(FeatureFlagSnapshot.self, from: Data(json.utf8)))
        }
    }
    func testBuildProfileRequiresExactAgreement() throws {
        let profile = try JSONDecoder().decode(BuildFeatureProfile.self, from: Data("{\"schema_version\":1,\"environment\":\"staging\",\"profile_digest\":\"\(Self.digest)\"}".utf8))
        XCTAssertTrue(profile.agrees(environment: "staging", digest: Self.digest))
        XCTAssertFalse(profile.agrees(environment: "testing", digest: Self.digest))
        XCTAssertFalse(profile.agrees(environment: "staging", digest: String(repeating: "b", count: 64)))
        XCTAssertFalse(profile.agrees(environment: nil, digest: nil))
    }
    static let digest = String(repeating: "a", count: 64)
    static let json = """
    {"schema_version":1,"environment":"staging","profile_digest":"\(digest)","instance_id":"63D037AB-023D-434D-9AD1-048C468AA6EB","generation":1,"sampled_at":"2026-11-05T00:00:00Z","source":"remote","revision":1,"expires_at":"2026-11-05T01:00:00Z","fresh":true,"flags":{"fixture":{"enabled":true,"pending":true,"capability_blocked":false,"timing":"live"}}}
    """
}
