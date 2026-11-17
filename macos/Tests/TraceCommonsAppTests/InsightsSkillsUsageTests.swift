import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsUsageTests: XCTestCase {
    private func row(_ population: String = "direct", uses: Int = 2, loads: Int = 0,
                     undated: Int = 0, windows: Bool = true) -> DaemonData.SkillsUsageRow {
        .init(name: "review", agent: "claude_code", invocations: uses, loads: loads, sessions: 1,
              windows: windows ? [.init(days: 7, invocations: 0, loads: 0, sessions: 0),
                                  .init(days: 30, invocations: uses, loads: loads, sessions: 1)] : [],
              undatedEvents: undated, population: population)
    }
    private let now = ISO8601DateFormatter().date(from: "2026-11-14T12:00:00Z")!
    private var history: DaemonData.SkillsUsageRow {
        try! JSONDecoder().decode(DaemonData.SkillsUsageRow.self, from: Data(#"{"name":"history","agent":"claude_code","invocations":1,"loads":0,"sessions":1,"windows":[{"days":7,"invocations":0,"loads":0,"sessions":0},{"days":30,"invocations":0,"loads":0,"sessions":0}],"daily":[{"day":"2026-08-01","invocations":1,"loads":0,"sessions":1}]}"#.utf8))
    }
    private var available: DaemonData.SkillInstallationDetails {
        try! JSONDecoder().decode(DaemonData.SkillInstallationDetails.self,
            from: Data(#"{"aliases":[],"availability":"available","definition_status":"valid"}"#.utf8))
    }
    private func snapshot(_ rows: [DaemonData.SkillsUsageRow] = [], partial: Bool = false,
                          status: String = "explicit_invocations", earliest: String = "2026-07-01T00:00:00Z") -> DaemonData.SkillsSnapshot {
        .init(inventory: .init(skills: [.init(id: "local", name: "review", description: "", agent: "claude_code", scope: "user", location: "local", sha256: "a")], partial: false, details: ["local": available]),
              usage: .init(enabled: true, lastAnalysisAt: "2026-11-14T12:00:00Z", earliestEvidence: earliest, partial: partial,
                           skills: rows + [history], coverage: [.init(agent: "claude_code", status: status)]))
    }
    func testEmptyStateNeverCallsDisabledOrUnknownDataUnused() {
        XCTAssertEqual(InsightsSkillsUsage.emptyState(.init(), days: 30), "skills_analysis_off")
        XCTAssertEqual(InsightsSkillsUsage.emptyState(snapshot(status: "loads_only").usage, days: 30), "skills_usage_unavailable")
        XCTAssertEqual(InsightsSkillsUsage.emptyState(snapshot(partial: true).usage, days: 30), "skills_usage_pending")
        XCTAssertEqual(InsightsSkillsUsage.emptyState(snapshot([row(uses: 0)]).usage, days: 30), "skills_usage_empty")
    }
    func testMissingWindowsAndUndatedOrUnclassifiedUsesDoNotClaimZero() {
        for row in [row(windows: false), row("unknown", uses: 1), row(uses: 0, undated: 1)] {
            XCTAssertEqual(InsightsSkillsUsage.emptyState(snapshot([row]).usage, days: 30), "skills_usage_pending")
        }
        XCTAssertEqual(InsightsSkillsUsage.emptyState(snapshot([row(uses: 0)]).usage, days: 90), "skills_usage_pending")
    }
    func testRankingCombinesUsesButNeverLoadsOrSessionCounts() {
        let usage = snapshot([row(uses: 2, loads: 9), row("delegated", uses: 3), row("unknown", uses: 99)]).usage
        let ranked = InsightsSkillsUsage.ranked(usage, days: 30)
        XCTAssertEqual(ranked.count, 1)
        XCTAssertEqual(ranked.first?.uses, 5)
        XCTAssertEqual(ranked.first?.sources.count, 2)
        XCTAssertTrue(InsightsSkillsUsage.ranked(usage, days: 7).isEmpty)
        XCTAssertTrue(InsightsSkillsUsage.ranked(snapshot([row()], status: "loads_only").usage, days: 30).isEmpty)
    }
    func testDailyChartAddsDisjointPopulationCountsAndFiltersOldDates() throws {
        let rows = try JSONDecoder().decode([DaemonData.SkillsUsageRow].self, from: Data(#"""
        [{"name":"review","agent":"claude_code","population":"direct","invocations":2,"loads":0,"sessions":1,"windows":[{"days":7,"invocations":2,"loads":0,"sessions":1}],"daily":[{"day":"2026-11-13","invocations":2,"loads":0,"sessions":1},{"day":"2026-10-01","invocations":50,"loads":0,"sessions":1},{"day":"2026-11-07","invocations":50,"loads":0,"sessions":1}]},
        {"name":"review","agent":"claude_code","population":"delegated","invocations":3,"loads":0,"sessions":1,"windows":[{"days":7,"invocations":3,"loads":0,"sessions":1}],"daily":[{"day":"2026-11-13","invocations":3,"loads":0,"sessions":1}]}]
        """#.utf8))
        let ranked = InsightsSkillsUsage.ranked(snapshot(rows).usage, days: 7,
            now: ISO8601DateFormatter().date(from: "2026-11-14T12:00:00Z")!)
        XCTAssertEqual(ranked.first?.daily, [.init(id: "2026-11-13", uses: 5)])
    }
    func testRemovalRequiresCompleteInvocationCoverageForEntirePeriod() {
        let model = InsightsSkillsModel()
        XCTAssertEqual(InsightsSkillsUsage.removalCandidates(snapshot(), days: 30, model: model, now: now).map(\.id), ["local"])
        for unavailable in [snapshot(partial: true), snapshot(status: "loads_only"), snapshot(status: "pending"),
                            snapshot(earliest: "2026-11-13T00:00:00Z"), snapshot([row(uses: 0, undated: 1)]),
                            snapshot([row("unknown", uses: 0)]), snapshot([row(uses: 0, windows: false)]),
                            snapshot([row(uses: 0, loads: 1)]), snapshot([row("delegated", uses: 2)])] {
            XCTAssertTrue(InsightsSkillsUsage.removalCandidates(unavailable, days: 30, model: model, now: now).isEmpty)
        }
        XCTAssertEqual(InsightsSkillsUsage.removalCandidates(snapshot([row(uses: 2)]), days: 7, model: model, now: now).count, 1)
    }
    func testIncompleteInventoryAndUnavailableInstallationAreWithheld() throws {
        let original = snapshot()
        let model = InsightsSkillsModel()
        for availability in ["cached", "unknown", "disabled", "missing"] {
            let details = try JSONDecoder().decode(DaemonData.SkillInstallationDetails.self,
                from: Data("{\"aliases\":[],\"availability\":\"\(availability)\",\"definition_status\":\"valid\"}".utf8))
            let changed = DaemonData.SkillsSnapshot(inventory: .init(skills: original.inventory.skills, partial: false,
                details: ["local": details]), usage: original.usage)
            XCTAssertTrue(InsightsSkillsUsage.removalCandidates(changed, days: 30, model: model, now: now).isEmpty)
        }
        let truncated = DaemonData.SkillsSnapshot(inventory: .init(skills: original.inventory.skills,
            partial: true, details: original.inventory.details), usage: original.usage)
        XCTAssertTrue(InsightsSkillsUsage.removalCandidates(truncated, days: 30, model: model, now: now).isEmpty)
    }
    func testUnrelatedAgentHistoryCannotQualifyAnUnusedSkill() {
        let original = snapshot()
        let codex = DaemonData.SkillsUsageRow(name: "history", agent: "codex", invocations: 0, loads: 1,
            sessions: 1, daily: history.daily)
        let usage = DaemonData.SkillsUsage(enabled: true, lastAnalysisAt: original.usage.lastAnalysisAt,
            earliestEvidence: original.usage.earliestEvidence, partial: false, skills: [codex],
            coverage: [.init(agent: "claude_code", status: "explicit_invocations"), .init(agent: "codex", status: "loads_only")])
        XCTAssertTrue(InsightsSkillsUsage.removalCandidates(.init(inventory: original.inventory, usage: usage),
            days: 30, model: InsightsSkillsModel(), now: now).isEmpty)
    }
    func testCollidingAndSharedInstallationsAreNotRemovalCandidates() {
        let model = InsightsSkillsModel()
        let original = snapshot()
        let duplicate = DaemonData.InstalledSkill(id: "other", name: "review", description: "", agent: "claude_code", scope: "project", location: "elsewhere", sha256: "b")
        let collision = DaemonData.SkillsSnapshot(inventory: .init(skills: original.inventory.skills + [duplicate]), usage: original.usage)
        XCTAssertTrue(InsightsSkillsUsage.removalCandidates(collision, days: 30, model: model, now: now).isEmpty)
        let shared = DaemonData.InstalledSkill(id: "shared", name: "review", description: "", agent: "shared", scope: "user", location: "local", sha256: "b")
        XCTAssertTrue(InsightsSkillsUsage.removalCandidates(.init(inventory: .init(skills: [shared]), usage: original.usage), days: 30, model: model, now: now).isEmpty)
    }
}
