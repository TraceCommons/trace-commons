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

    func testDelegatedEvidencePreservesPopulationAndParentWithoutAddingTotals() throws {
        let json = #"""
        {"enabled":true,"usage":{"sessions_observed":2,"delegated_sessions_observed":3,"delegated_sessions_unknown":1,"unclassified_sessions_observed":4,"unclassified_sessions_unknown":2,"skills":[
          {"name":"review","agent":"opencode","population":"direct","invocations":1,"loads":0,"sessions":1},
          {"name":"review","agent":"opencode","population":"delegated","invocations":2,"loads":0,"sessions":1,"parent_session_refs":["parent"],"evidence":[{"session_ref":"child","parent_session_ref":"parent"}]}]}}
        """#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertEqual(snapshot.usage.sessionsObserved, 2)
        XCTAssertEqual(snapshot.usage.delegatedSessionsObserved, 3)
        XCTAssertEqual(snapshot.usage.delegatedSessionsUnknown, 1)
        XCTAssertEqual(snapshot.usage.unclassifiedSessionsObserved, 4)
        XCTAssertEqual(snapshot.usage.unclassifiedSessionsUnknown, 2)
        let rows = snapshot.usage.skills
        XCTAssertNotEqual(rows[0].identity, rows[1].identity)
        XCTAssertEqual(rows[1].evidence.first?.parentSessionRef, "parent")
        XCTAssertEqual(rows[1].evidence.first?.sessionRef, "child")
        XCTAssertEqual(rows[1].sessions, 1)
        XCTAssertEqual(try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONEncoder().encode(snapshot)), snapshot)
    }

    func testOlderUsageDoesNotClaimDelegatedCoverage() throws {
        let json = #"{"usage":{"skills":[{"name":"review","agent":"codex","invocations":0,"loads":1,"sessions":1}]}}"#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertNil(snapshot.usage.delegatedSessionsObserved)
        XCTAssertNil(snapshot.usage.delegatedSessionsUnknown)
        XCTAssertEqual(snapshot.usage.skills.first?.population, "direct")
        XCTAssertEqual(snapshot.usage.skills.first?.evidence, [])
    }

    func testEvidenceClientPreservesOpaqueChildAndParentReferences() async throws {
        let transport = FakeTransport(response: #"{"id":0,"result":{"enabled":true,"readable":true,"session":{"source":"claude_code"}}}"#)
        let client = LiveDaemonClient(transport: transport)
        for reference in ["c42.storetag", "parent-storetag"] {
            _ = try await client.skillEvidence(sessionRef: reference)
        }
        XCTAssertEqual(transport.calls.map(\.method), ["insights_session_drill", "insights_session_drill"])
        let parameters = try transport.calls.map { try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.params.utf8)) as? [String: Any]) }
        XCTAssertEqual(parameters[0]["session_ref"] as? String, "c42.storetag")
        XCTAssertEqual(parameters[1]["session_ref"] as? String, "parent-storetag")
    }

    func testOpenCodeEvidenceUsesSkillOnlyDrill() async throws {
        let transport = FakeTransport(response: #"{"id":0,"result":{"enabled":true,"readable":true,"agent":"opencode","population":"unknown","skill_usage":{"coverage":"explicit_invocations","partial":true,"skills":[{"name":"review","invocations":2,"loads":0}]}}}"#)
        let result = try await LiveDaemonClient(transport: transport).skillEvidence(sessionRef: "os.opaque")
        XCTAssertEqual(transport.calls.map(\.method), ["insights_skills"])
        let parameters = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(transport.calls[0].params.utf8)) as? [String: Any])
        XCTAssertEqual(parameters["session_ref"] as? String, "os.opaque")
        XCTAssertNil(result.session)
        XCTAssertEqual(result.population, "unknown")
        XCTAssertEqual(result.skillUsage?.skills.first?.invocations, 2)
        XCTAssertEqual(result.skillUsage?.partial, true)
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
