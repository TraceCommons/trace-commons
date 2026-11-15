import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsIndexTests: XCTestCase {
    private func skill(_ id: String, agent: String = "codex", hash: String = String(repeating: "a", count: 64),
                       source: DaemonData.SkillsSource? = nil) -> DaemonData.InstalledSkill {
        .init(id: id, name: "review", description: "Review code", agent: agent, scope: "global",
              location: "/skills/\(id)", sha256: hash, source: source)
    }

    func testIdenticalDefinitionsAcrossAgentsAppearOnceWithAllLocationsAndAgents() {
        let copies = [skill("codex"), skill("claude", agent: "claude_code"), skill("open", agent: "opencode")]
        let groups = InsightsSkillsIndex.groups(copies)
        XCTAssertEqual(groups.count, 1)
        XCTAssertEqual(groups[0].agents, ["claude_code", "codex", "opencode"])
        XCTAssertEqual(Set(groups[0].installations.map(\.location)), Set(copies.map(\.location)))
        XCTAssertEqual(groups[0].id, InsightsSkillsIndex.groups(Array(copies.reversed()))[0].id)
    }

    func testSameNameDifferentContentOrUnknownContentRemainSeparate() {
        XCTAssertEqual(InsightsSkillsIndex.groups([skill("a"), skill("b", agent: "claude_code", hash: String(repeating: "b", count: 64))]).count, 2)
        for hash in ["", "invalid", String(repeating: "z", count: 64)] {
            XCTAssertEqual(InsightsSkillsIndex.groups([skill("a", hash: hash), skill("b", agent: "claude_code", hash: hash)]).count, 2)
        }
    }

    func testConflictingKnownOriginsDoNotMergeDespiteIdenticalContent() {
        let a = skill("a", source: .init(repository: "one/repo", path: "review"))
        let b = skill("b", agent: "claude_code", source: .init(repository: "two/repo", path: "review"))
        XCTAssertEqual(InsightsSkillsIndex.groups([a, b, skill("unknown", agent: "opencode")]).count, 3)
        XCTAssertEqual(InsightsSkillsIndex.groups([a, skill("b", agent: "claude_code", source: a.source)]).count, 1)
    }

    func testFilteringKeepsTheMatchingGroupAndAllItsAgentCopies() {
        let copies = [skill("codex"), skill("claude", agent: "claude_code"), skill("different", hash: String(repeating: "b", count: 64))]
        let groups = InsightsSkillsIndex.groups(copies, matching: ["claude"])
        XCTAssertEqual(groups.count, 1)
        XCTAssertEqual(groups[0].agents, ["claude_code", "codex"])
        XCTAssertTrue(InsightsSkillsIndex.groups(copies, matching: []).isEmpty)
    }

    func testDefinitionUsesAnAvailableCopyWithoutChangingGroupIdentity() throws {
        let copies = [skill("a-cached"), skill("z-available", agent: "claude_code")]
        let cached = try JSONDecoder().decode(DaemonData.SkillInstallationDetails.self,
            from: Data(#"{"aliases":[],"availability":"cached","definition_status":"valid"}"#.utf8))
        let available = try JSONDecoder().decode(DaemonData.SkillInstallationDetails.self,
            from: Data(#"{"aliases":[],"availability":"available","definition_status":"valid"}"#.utf8))
        let snapshot = DaemonData.SkillsSnapshot(inventory: .init(skills: copies,
            details: ["a-cached": cached, "z-available": available]))
        let group = InsightsSkillsIndex.groups(copies)[0]
        XCTAssertEqual(group.id, "a-cached")
        XCTAssertEqual(group.definitionInstallation(in: snapshot).id, "z-available")
        let unavailable = DaemonData.SkillsSnapshot(inventory: .init(skills: copies))
        XCTAssertEqual(group.definitionInstallation(in: unavailable).id, "a-cached")
        XCTAssertEqual(group.id, "a-cached")
    }

    func testGroupUsageDoesNotDuplicateRowsOrSumTraceTotals() {
        let copies = [skill("codex"), skill("claude", agent: "claude_code")]
        let snapshot = DaemonData.SkillsSnapshot(inventory: .init(skills: copies), usage: .init(enabled: true,
            skills: [.init(name: "review", agent: "codex", invocations: 0, loads: 2, sessions: 1),
                     .init(name: "review", agent: "claude_code", invocations: 3, loads: 0, sessions: 1)]))
        let rows = InsightsSkillsIndex.usageRows(InsightsSkillsIndex.groups(copies)[0], snapshot: snapshot, model: InsightsSkillsModel())
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows.map(\.sessions), [1, 1])
        // Shared roots overlap per-agent name attribution; keep withholding it.
        let ambiguous = DaemonData.SkillsSnapshot(inventory: .init(skills: copies + [skill("shared", agent: "shared")]), usage: snapshot.usage)
        XCTAssertTrue(InsightsSkillsIndex.usageRows(InsightsSkillsIndex.groups(ambiguous.inventory.skills)[0], snapshot: ambiguous, model: InsightsSkillsModel()).isEmpty)
    }
}
