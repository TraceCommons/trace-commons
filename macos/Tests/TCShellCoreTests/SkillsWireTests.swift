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

    func testServerCatalogFactsAndSuggestionsDecode() throws {
        let json = #"""
        {"enabled":true,"catalog":{"source":"server","revision":1763251200,"issued_at":"2026-11-16T00:00:00Z","expires_at":"2026-11-30T00:00:00Z","total_skills":2,
          "publisher_states":[{"publisher_id":"gh.acme.skills","status":"current","checked_at":"2026-11-16T00:00:00Z","signals":{"stars":40,"repository_checked_at":"2026-11-16T00:00:00Z","commits_7d":1,"commits_28d":3,"activity_checked_at":"2026-11-16T00:00:00Z"}},{"publisher_id":"obra","status":"current","signals":{"stars":"many"}}],
          "publishers":[{"id":"gh.acme.skills","name":"acme/skills","repository":"acme/skills","skills_root":"","instructions_url":"https://github.com/acme/skills","url":"https://github.com/acme/skills","source_kind":"marketplace"}],
          "skills":[{"id":"gh.acme.skills:lint/SKILL.md","publisher_id":"gh.acme.skills","name":"lint","description":"Lints.","repository":"acme/skills","path":"lint/SKILL.md","revision":"0123456789abcdef0123456789abcdef01234567","sha256":"aa","source_url":"https://github.com/acme/skills/blob/x/lint/SKILL.md","instructions_url":"https://github.com/acme/skills",
            "license":{"status":"declared","spdx":"MIT"},"first_seen_at":"2026-11-01T00:00:00Z","last_changed_at":"2026-11-10T00:00:00Z","checks_version":2,
            "checks":{"frontmatter":"valid","document_bytes":10,"directory_bytes":30,"directory_files":3,"bundled":{"scripts":1,"executables":1,"binaries":0},"patterns":["sudo"],"inactive":false}}]},
         "suggestions":{"available":true,"opted_in":false,"pending":2,"sent_total":0,"last_sent_at":null}}
        """#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertEqual(snapshot.catalog.source, "server")
        XCTAssertEqual(snapshot.catalog.revision, 1763251200)
        XCTAssertEqual(snapshot.catalog.totalSkills, 2)
        XCTAssertEqual(snapshot.catalog.publisherStates.first?.signals, DaemonData.SkillsRepositorySignals(stars: 40, repositoryCheckedAt: "2026-11-16T00:00:00Z", commits7d: 1, commits28d: 3, activityCheckedAt: "2026-11-16T00:00:00Z"))
        XCTAssertNil(snapshot.catalog.publisherStates.last?.signals, "a malformed signal is unknown")
        XCTAssertEqual(snapshot.catalog.publishers.first?.sourceKind, "marketplace")
        XCTAssertEqual(snapshot.catalog.publishers.first?.url, "https://github.com/acme/skills")
        let skill = try XCTUnwrap(snapshot.catalog.skills.first)
        XCTAssertEqual(skill.license, DaemonData.SkillsLicense(status: "declared", spdx: "MIT"))
        XCTAssertEqual(skill.checks?.bundled.executables, 1)
        XCTAssertEqual(skill.checks?.patterns, ["sudo"])
        XCTAssertEqual(skill.checksVersion, 2)
        XCTAssertEqual(skill.firstSeenAt, "2026-11-01T00:00:00Z")
        XCTAssertTrue(snapshot.suggestions.available)
        XCTAssertFalse(snapshot.suggestions.optedIn)
        XCTAssertEqual(snapshot.suggestions.pending, 2)
        XCTAssertEqual(try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONEncoder().encode(snapshot)), snapshot)
    }

    func testGithubCatalogWithoutFactsAndMalformedFactsDecodeAsUnknown() throws {
        let json = #"""
        {"enabled":true,"catalog":{"publishers":[{"id":"obra","name":"Obra","repository":"obra/superpowers","skills_root":"skills","instructions_url":"https://x"}],
          "skills":[{"id":"obra:skills/a/SKILL.md","publisher_id":"obra","name":"a","description":"A","repository":"obra/superpowers","path":"skills/a/SKILL.md","revision":"r","sha256":"s","source_url":"u","instructions_url":"i","checks":{"frontmatter":7},"license":"MIT"}]}}
        """#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertNil(snapshot.catalog.source)
        XCTAssertNil(snapshot.catalog.publishers.first?.sourceKind)
        XCTAssertNil(snapshot.catalog.skills.first?.checks)
        XCTAssertNil(snapshot.catalog.skills.first?.license)
        XCTAssertFalse(snapshot.suggestions.available)
    }

    func testRecentlyActivePublishersDecodeAndAMalformedListIsEmpty() throws {
        let json = #"{"enabled":true,"catalog":{"source":"server","trending":[{"publisher_id":"gh.acme.skills","skill_ids":["gh.acme.skills:lint/SKILL.md","gh.acme.skills:plan/SKILL.md"]},{"publisher_id":"obra","skill_ids":[]}]}}"#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        XCTAssertEqual(snapshot.catalog.trending.map(\.publisherID), ["gh.acme.skills", "obra"])
        XCTAssertEqual(snapshot.catalog.trending.first?.skillIDs, ["gh.acme.skills:lint/SKILL.md", "gh.acme.skills:plan/SKILL.md"])
        XCTAssertEqual(try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONEncoder().encode(snapshot)), snapshot)

        // Absent (an older daemon), null (no verified catalog) and malformed
        // all read as none, and never fail the snapshot.
        for body in [#"{"source":"github"}"#, #"{"source":"github","trending":null}"#,
                     #"{"source":"server","trending":"soon"}"#, #"{"source":"server","trending":[{"publisher_id":7}]}"#,
                     #"{"source":"server","trending":[{"publisher_id":"a","skill_ids":["x"]},{"skill_ids":["y"]}]}"#] {
            let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(#"{"enabled":true,"catalog":"# .utf8) + Data(body.utf8) + Data("}".utf8))
            XCTAssertTrue(snapshot.enabled, body)
            XCTAssertTrue(snapshot.catalog.trending.isEmpty, body)
        }
        XCTAssertTrue(DaemonData.SkillsCatalog().trending.isEmpty)
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
