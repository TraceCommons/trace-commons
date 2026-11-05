import XCTest
@testable import TCShellCore

final class SkillsWireTests: XCTestCase {
    func testSnapshotDecodesIdentityAndUsageWithoutInferringFromNames() throws {
        let json = #"""
        {"enabled":true,"inventory":{"skills":[{"id":"local","name":"review","description":"Local","agent":"codex","scope":"user","location":"/skills/review","sha256":"abc","source":{"repository":"owner/repo","path":"skills/review","revision":"main"}}],"partial":true,"coverage":[{"location":"/skills","status":"read"}]},"catalog":{"publishers":[{"id":"owner","name":"Owner","repository":"owner/repo","skills_root":"skills","instructions_url":"https://example.com/docs"}],"skills":[{"id":"upstream","publisher_id":"owner","name":"review","description":"Remote","repository":"owner/repo","path":"skills/review","revision":"main","sha256":"def","source_url":"https://example.com/source","instructions_url":"https://example.com/docs"}],"checked_at":"today"},"followed_publishers":["owner"],"updates":[{"installed_id":"local","status":"changed","catalog_skill_id":"upstream"}],"feed":[{"id":"event","publisher_id":"owner","skill_id":"upstream","kind":"changed","observed_at":"today"}],"usage":{"sessions_observed":3,"partial":true,"skills":[{"name":"review","agent":"codex","invocations":2,"loads":1,"sessions":2,"last_seen":"today","session_refs":["session-hash"]}]},"copy":{"skills_title":"Skills"},"future":true}
        """#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertEqual(snapshot.inventory.skills.first?.source?.repository, "owner/repo")
        XCTAssertEqual(snapshot.catalog.skills.first?.publisherID, "owner")
        XCTAssertEqual(snapshot.updates.first?.catalogSkillID, "upstream")
        XCTAssertEqual(snapshot.usage.skills.first?.sessionRefs, ["session-hash"])
        XCTAssertTrue(snapshot.usage.partial)
        XCTAssertEqual(snapshot.followedPublishers, ["owner"])
    }

    func testDisabledOlderShapeHasConservativeDefaults() throws {
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(#"{"enabled":false}"#.utf8))
        XCTAssertFalse(snapshot.enabled)
        XCTAssertTrue(snapshot.inventory.partial)
        XCTAssertTrue(snapshot.usage.partial)
        XCTAssertTrue(snapshot.catalog.skills.isEmpty)
        XCTAssertFalse(snapshot.catalog.partial)
        XCTAssertFalse(snapshot.feedPartial)
    }

    func testBoundedCatalogAndFeedRetainPartialFlags() throws {
        let json = #"{"enabled":true,"catalog":{"partial":true},"feed_partial":true}"#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertTrue(snapshot.catalog.partial)
        XCTAssertTrue(snapshot.feedPartial)
        let roundTrip = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONEncoder().encode(snapshot))
        XCTAssertEqual(roundTrip, snapshot)
    }

    func testLiveMethodsSendExactRefreshAndFollowParameters() async throws {
        let transport = FakeTransport(response: #"{"id":0,"result":{"enabled":false}}"#)
        let client = LiveDaemonClient(transport: transport)
        _ = try await client.insightsSkills(refresh: true)
        _ = try await client.setSkillsFollow(publisherID: "owner/repo", followed: false)
        XCTAssertEqual(transport.calls.map(\.method), ["insights_skills", "insights_skills_follow"])
        let first = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(transport.calls[0].params.utf8)) as? [String: Any])
        let second = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(transport.calls[1].params.utf8)) as? [String: Any])
        XCTAssertEqual(first["refresh"] as? Bool, true)
        XCTAssertEqual(second["publisher_id"] as? String, "owner/repo")
        XCTAssertEqual(second["followed"] as? Bool, false)
    }

    func testOlderClientsRemainUnavailable() async {
        do {
            _ = try await SampleDaemonClient(.normalDay).insightsSkills(refresh: false)
            XCTFail("An older client must not claim an empty inventory")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .notAvailableYet(method: "insights_skills"))
        }
    }
}
