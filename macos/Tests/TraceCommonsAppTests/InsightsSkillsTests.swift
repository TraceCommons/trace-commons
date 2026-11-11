import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsTests: XCTestCase {
    func testInventorySearchFiltersAndUsageSort() {
        let model = InsightsSkillsModel()
        let alpha = DaemonData.InstalledSkill(id: "a", name: "Alpha", description: "Reviews code", agent: "codex", scope: "global", location: "hidden", sha256: "a")
        let beta = DaemonData.InstalledSkill(id: "b", name: "Beta", description: "Builds", agent: "claude_code", scope: "global", location: "hidden", sha256: "b")
        let snapshot = DaemonData.SkillsSnapshot(inventory: .init(skills: [beta, alpha]), usage: .init(skills: [
            .init(name: "Beta", agent: "claude_code", invocations: 10, loads: 0, sessions: 3,
                  windows: [.init(days: 30, invocations: 10, loads: 0, sessions: 3)])
        ]))
        XCTAssertEqual(model.inventorySkills(in: snapshot, matching: "").map(\.id), ["a", "b"])
        XCTAssertEqual(model.inventorySkills(in: snapshot, matching: "reviews").map(\.id), ["a"])
        XCTAssertTrue(model.inventorySkills(in: snapshot, matching: "hidden").isEmpty)
        XCTAssertEqual(model.inventorySkills(in: snapshot, matching: "", agent: "codex").map(\.id), ["a"])
        XCTAssertEqual(model.inventorySkills(in: snapshot, matching: "", sort: "sessions").map(\.id), ["b", "a"])
        XCTAssertEqual(model.inventorySkills(in: snapshot, matching: "", sort: "sessions", days: 7).map(\.id), ["a", "b"])
    }

    func testAmbiguousInstallationsDoNotReceiveNameOnlyCounts() {
        let model = InsightsSkillsModel()
        let first = DaemonData.InstalledSkill(id: "a", name: "Review", description: "", agent: "codex", scope: "global", location: "a", sha256: "a")
        let second = DaemonData.InstalledSkill(id: "b", name: "Review", description: "", agent: "codex", scope: "project", location: "b", sha256: "b")
        let usage = DaemonData.SkillsUsage(enabled: true, lastAnalysisAt: "2026-11-11T00:00:00Z", partial: false,
            skills: [.init(name: "Review", agent: "codex", invocations: 0, loads: 2, sessions: 1)],
            coverage: [.init(agent: "codex", status: "loads_only")])
        let snapshot = DaemonData.SkillsSnapshot(inventory: .init(skills: [first, second]), usage: usage)
        XCTAssertNil(model.usage(for: first, in: snapshot))
        XCTAssertEqual(model.usageState(for: first, in: snapshot), "skills_usage_unattributed")
        XCTAssertEqual(model.usageState(for: first, in: .init(inventory: .init(skills: [first]))), "skills_usage_unavailable")
    }

    func testSharedSkillKeepsConsumerUsageSeparateAndWithholdsCollisions() {
        let model = InsightsSkillsModel()
        let shared = DaemonData.InstalledSkill(id: "shared", name: "Review", description: "", agent: "shared", scope: "global", location: "shared", sha256: "a")
        let competing = DaemonData.InstalledSkill(id: "private", name: "Review", description: "", agent: "codex", scope: "global", location: "private", sha256: "b")
        let usage = DaemonData.SkillsUsage(enabled: true, lastAnalysisAt: "2026-11-11T00:00:00Z", partial: false,
            skills: [
                .init(name: "Review", agent: "codex", invocations: 0, loads: 2, sessions: 1, sessionRefs: ["same"], windows: [.init(days: 30, invocations: 0, loads: 2, sessions: 1)]),
                .init(name: "Review", agent: "claude_code", invocations: 1, loads: 0, sessions: 1, sessionRefs: ["same"], windows: [.init(days: 30, invocations: 1, loads: 0, sessions: 1)])
            ], coverage: [.init(agent: "codex", status: "loads_only"), .init(agent: "claude_code", status: "explicit_invocations")])
        let snapshot = DaemonData.SkillsSnapshot(inventory: .init(skills: [shared]), usage: usage)
        let rows = model.usageRows(for: shared, in: snapshot)
        XCTAssertEqual(rows.map(\.agent), ["claude_code", "codex"])
        XCTAssertEqual(rows.map(\.sessions), [1, 1])
        XCTAssertNil(model.usage(for: shared, in: snapshot), "Never sum potentially overlapping trace totals")
        XCTAssertNotEqual(model.usageState(for: shared, in: snapshot), "skills_usage_unavailable")
        let collision = DaemonData.SkillsSnapshot(inventory: .init(skills: [shared, competing]), usage: usage)
        XCTAssertTrue(model.usageRows(for: shared, in: collision).isEmpty)
        XCTAssertTrue(model.usageRows(for: competing, in: collision).isEmpty)
        XCTAssertEqual(model.usageState(for: shared, in: collision), "skills_usage_unattributed")
    }

    func testDefinitionAndEvidenceReadersUseDaemonResults() async {
        let model = InsightsSkillsModel(definitionReader: { id in
            XCTAssertEqual(id, "local-id")
            return try JSONDecoder().decode(DaemonData.SkillDefinition.self,
                from: Data(##"{"skill_id":"local-id","status":"available","definition":"# Local definition"}"##.utf8))
        }, evidenceReader: { reference in
            XCTAssertEqual(reference, "t1.42")
            return try JSONDecoder().decode(DaemonData.SkillEvidence.self,
                from: Data(#"{"enabled":true,"readable":true,"session":{"source":"codex","date":"2026-11-11","turns":null,"tokens":42}}"#.utf8))
        })
        await model.loadDefinition("local-id")
        XCTAssertEqual(model.definition?.definition, "# Local definition")
        XCTAssertFalse(model.definitionFailure)
        await model.loadEvidence("t1.42")
        XCTAssertEqual(model.evidence?.session?.tokens, 42)
        XCTAssertFalse(model.evidenceFailure)
    }

    func testUnavailableDefinitionAndRevokedEvidenceStayUnavailable() async {
        let model = InsightsSkillsModel(definitionReader: { id in
            try JSONDecoder().decode(DaemonData.SkillDefinition.self,
                from: Data(#"{"skill_id":"local-id","status":"unavailable"}"#.utf8))
        }, evidenceReader: { _ in
            try JSONDecoder().decode(DaemonData.SkillEvidence.self, from: Data(#"{"enabled":false}"#.utf8))
        })
        await model.loadDefinition("local-id")
        XCTAssertTrue(model.definitionFailure)
        XCTAssertNil(model.definition?.definition)
        await model.loadEvidence("t1.42")
        XCTAssertTrue(model.evidenceFailure)
    }

    private actor DelayedDefinition {
        var waiting: CheckedContinuation<Void, Never>?
        var reads = 0
        func read(_ id: String) async throws -> DaemonData.SkillDefinition {
            reads += 1
            let first = reads == 1
            if first { await withCheckedContinuation { waiting = $0 } }
            let body = first ? "old" : "new"
            return try JSONDecoder().decode(DaemonData.SkillDefinition.self,
                from: JSONSerialization.data(withJSONObject: ["skill_id": id, "status": "available", "definition": body]))
        }
        func hasWaiter() -> Bool { waiting != nil }
        func release() { waiting?.resume(); waiting = nil }
    }

    func testLateSameIdentityDefinitionCannotRestoreClearedSelection() async {
        let replies = DelayedDefinition()
        let model = InsightsSkillsModel(definitionReader: { try await replies.read($0) })
        let old = Task { await model.loadDefinition("same-id") }
        while !(await replies.hasWaiter()) { await Task.yield() }
        model.invalidateDetails()
        await model.loadDefinition("same-id")
        XCTAssertEqual(model.definition?.definition, "new")
        await replies.release()
        await old.value
        XCTAssertEqual(model.definition?.definition, "new")
    }

    func testInvalidationDiscardsInFlightDefinitionWithoutReplacement() async {
        let replies = DelayedDefinition()
        let model = InsightsSkillsModel(definitionReader: { try await replies.read($0) })
        let old = Task { await model.loadDefinition("same-id") }
        while !(await replies.hasWaiter()) { await Task.yield() }
        model.invalidateDetails()
        await replies.release()
        await old.value
        XCTAssertNil(model.definition)
    }

    private actor Replies {
        var reads = 0
        func next() throws -> DaemonData.SkillsSnapshot {
            reads += 1
            if reads > 1 { throw DaemonDataError.notAvailableYet(method: "insights_skills") }
            return .init(enabled: true, followedPublishers: ["publisher"])
        }
    }

    private actor ObservedReads {
        private(set) var refreshes: [Bool] = []
        func read(refresh: Bool) -> DaemonData.SkillsSnapshot {
            refreshes.append(refresh)
            return .init(enabled: true)
        }
    }

    func testAutomaticOpeningAndPeriodicReadsDoNotForceCatalogRefresh() async throws {
        let reads = ObservedReads()
        let model = InsightsSkillsModel(reader: { refresh in await reads.read(refresh: refresh) },
                                        idlePollInterval: .milliseconds(1))
        let observation = Task { await model.observe() }
        for _ in 0 ..< 1_000 {
            if await reads.refreshes.count >= 2 { break }
            try await Task.sleep(for: .milliseconds(1))
        }
        observation.cancel()
        await observation.value
        let automatic = await reads.refreshes
        XCTAssertGreaterThanOrEqual(automatic.count, 2, "Opening and a periodic read must both be exercised")
        XCTAssertTrue(automatic.allSatisfy { !$0 }, "Automatic reads must retain the backend six-hour throttle")
        await model.load(refresh: true)
        let manual = await reads.refreshes
        XCTAssertEqual(manual.last, true, "The explicit Refresh action can still force a refresh")
    }

    private actor DelayedRead {
        private(set) var count = 0
        private var continuation: CheckedContinuation<Void, Never>?
        func read() async -> DaemonData.SkillsSnapshot {
            count += 1
            if count == 1 {
                // Intentionally ignores task cancellation until the IPC reply arrives.
                await withCheckedContinuation { continuation = $0 }
                return .init(enabled: true, followedPublishers: ["stale"])
            }
            return .init(enabled: true, followedPublishers: ["fresh"])
        }
        func release() { continuation?.resume(); continuation = nil }
    }

    func testReactivatedObserverWaitsForCancelledReadThenReadsPromptly() async {
        let reads = DelayedRead()
        let model = InsightsSkillsModel(reader: { _ in await reads.read() })
        let first = Task { await model.observe() }
        for _ in 0 ..< 1_000 {
            if await reads.count == 1 { break }
            try? await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertTrue(model.busy)
        first.cancel()
        let replacement = Task { await model.observe() }
        // Let the replacement encounter the occupied lane before releasing IPC.
        try? await Task.sleep(for: .milliseconds(20))
        await reads.release()
        for _ in 0 ..< 1_000 {
            if model.shown?.followedPublishers == ["fresh"] { break }
            try? await Task.sleep(for: .milliseconds(1))
        }
        replacement.cancel()
        await first.value
        await replacement.value
        XCTAssertEqual(model.shown?.followedPublishers, ["fresh"], "Reactivation must not wait for the five-minute poll")
        let count = await reads.count
        XCTAssertEqual(count, 2)
        XCTAssertFalse(model.busy)
    }

    func testRefreshFailurePreservesLastSuccessfulSnapshot() async {
        let replies = Replies()
        let model = InsightsSkillsModel(reader: { _ in try await replies.next() })
        await model.load(refresh: true)
        XCTAssertEqual(model.shown?.followedPublishers, ["publisher"])
        await model.load(refresh: true)
        XCTAssertEqual(model.shown?.followedPublishers, ["publisher"])
        XCTAssertEqual(model.failureKey, "skills_refresh_failed")
        XCTAssertFalse(model.busy)
    }

    func testFollowUsesConfirmedSnapshotAndSerializesMutations() async {
        let gate = FollowGate()
        let model = InsightsSkillsModel(writer: { publisher, followed in
            await gate.write(publisher, followed: followed)
        })
        let first = Task { await model.setFollow(publisherID: "a", followed: true) }
        while await gate.count == 0 { await Task.yield() }
        XCTAssertNil(model.shown, "No optimistic follow state")
        await model.setFollow(publisherID: "b", followed: true)
        await gate.release()
        await first.value
        let writes = await gate.count
        XCTAssertEqual(writes, 1)
        XCTAssertEqual(model.shown?.followedPublishers, ["a"])
        XCTAssertFalse(model.busy)
    }

    func testLoadsOnlyDoesNotPresentAnUnknownInvocationCountAsZero() {
        let model = InsightsSkillsModel(copy: ["skills_invocations": "{count} invocations",
            "skills_no_invocations": "No observed invocations", "skills_loads": "{count} loads",
            "skills_sessions": "{count} sessions"])
        let row = DaemonData.SkillsUsageRow(name: "review", agent: "codex", invocations: 0, loads: 3, sessions: 2)
        XCTAssertEqual(model.usageSummary(row, coverage: [.init(agent: "codex", status: "loads_only")]),
                       "3 loads · 2 sessions")
        XCTAssertEqual(model.usageSummary(row, coverage: [.init(agent: "codex", status: "explicit_invocations")]),
                       "No observed invocations · 3 loads · 2 sessions")
    }

    func testExplicitInvocationCoverageDoesNotPresentUnobservedClaudeLoadsAsZero() {
        let model = InsightsSkillsModel(copy: ["skills_invocations": "{count} invocations",
            "skills_loads": "{count} loads", "skills_sessions": "{count} traces"])
        let row = DaemonData.SkillsUsageRow(name: "review", agent: "claude_code", invocations: 3, loads: 0, sessions: 2)
        XCTAssertEqual(model.usageSummary(row, coverage: [.init(agent: "claude_code", status: "explicit_invocations")]),
                       "3 invocations · 2 traces")
    }

    func testSkillsSectionsKeepTheirOrderAndSharedCopyKeys() {
        XCTAssertEqual(SkillsSection.allCases, [.inventory, .usage, .catalog, .following])
        XCTAssertEqual(SkillsSection.allCases.map(\.copyKey),
                       ["skills_tab_inventory", "skills_tab_usage", "skills_tab_catalog", "skills_tab_following"])
    }

    func testTimestampsUseTheNativeLocalizedDateFormat() throws {
        let model = InsightsSkillsModel()
        let date = try Date("2026-11-05T18:00:00Z", strategy: .iso8601)
        XCTAssertEqual(model.timestamp("2026-11-05T18:00:00Z"), date.formatted(date: .abbreviated, time: .shortened))
        XCTAssertEqual(model.timestamp("2026-11-05T18:00:00.000Z"), date.formatted(date: .abbreviated, time: .shortened))
        XCTAssertEqual(model.timestamp("unknown"), "unknown")
    }

    func testMatchingNeedsRepositoryAndPathInsteadOfAnIdenticalName() throws {
        let json = #"""
        {"enabled":true,"inventory":{"skills":[{"id":"local","name":"review","description":"","agent":"codex","scope":"user","location":"local","sha256":"hash"}],"partial":false,"coverage":[]},"catalog":{"publishers":[],"skills":[{"id":"remote","publisher_id":"p","name":"review","description":"","repository":"owner/repo","path":"skills/review","revision":"main","sha256":"hash","source_url":"https://example.com/source","instructions_url":"https://example.com/docs"}]}}
        """#
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(json.utf8))
        let model = InsightsSkillsModel()
        XCTAssertNil(model.catalogMatch(for: snapshot.inventory.skills[0], in: snapshot))
    }
}

private actor FollowGate {
    private(set) var count = 0
    private var continuation: CheckedContinuation<Void, Never>?
    func write(_ publisher: String, followed: Bool) async -> DaemonData.SkillsSnapshot {
        count += 1
        await withCheckedContinuation { continuation = $0 }
        return .init(enabled: true, followedPublishers: followed ? [publisher] : [])
    }
    func release() { continuation?.resume(); continuation = nil }
}
