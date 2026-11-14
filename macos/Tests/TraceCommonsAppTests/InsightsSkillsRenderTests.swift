import AppKit
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Save the native Skills pane at its narrow/default and comfortable widths
/// with TC_INSIGHTS_RENDER_DIR. Data here exercises identity and coverage.
@MainActor
final class InsightsSkillsRenderTests: XCTestCase {
    func testSkillsRendersInventoryUsageCatalogAndPublisherFeed() async throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let snapshot = Self.snapshot(copy: copy)
        let required = SkillsSection.allCases.map(\.copyKey) + ["skills_title", "skills_inventory", "skills_usage", "skills_catalog", "skills_publishers",
            "skills_feed", "skills_partial_catalog", "skills_partial_feed", "skills_status_complete", "skills_status_current", "skills_status_update_available",
            "skills_feed_updated", "skills_scope_global", "skills_agent_codex", "skills_coverage_loads_only",
            "skills_population_direct", "skills_population_delegated", "skills_evidence_links", "skills_evidence_parent",
            "skills_coverage_explicit_invocations", "skills_coverage_unsupported", "skills_search", "skills_search_empty"]
        for key in required { XCTAssertFalse((copy[key] ?? "").isEmpty, "Missing shared copy: \(key)") }
        let model = InsightsSkillsModel(reader: { _ in snapshot }, copy: copy)
        await model.load()
        for section in SkillsSection.allCases {
            XCTAssertTrue(Self.ready(model, section: section), "Missing fixture content for \(section)")
            for width: CGFloat in [440, 900] {
                let png = try await InsightsGetStartedRenderTests.render(
                    AnyView(InsightsSkillsTab(model: model, initialSection: section)), width: width, height: 2000,
                    ready: { Self.ready(model, section: section) })
                XCTAssertGreaterThan(png.count, 20_000, "\(section) at \(width)")
                if let directory = ProcessInfo.processInfo.environment["TC_INSIGHTS_RENDER_DIR"] {
                    try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
                    try png.write(to: URL(fileURLWithPath: directory)
                        .appendingPathComponent("insights-skills-\(section.rawValue)-\(Int(width)).png"))
                }
            }
        }
    }

    private static func ready(_ model: InsightsSkillsModel, section: SkillsSection) -> Bool {
        guard let snapshot = model.shown, snapshot.enabled, !model.busy,
              !model.text(section.copyKey).isEmpty else { return false }
        switch section {
        case .inventory:
            return snapshot.inventory.skills.contains { $0.id == "installed-1" }
                && snapshot.updates.contains { $0.status == "update_available" }
        case .usage:
            return snapshot.usage.skills.contains { $0.loads == 4 }
                && snapshot.usage.coverage.contains { $0.status == "loads_only" }
        case .catalog:
            return snapshot.catalog.publishers.contains { $0.id == "superpowers" }
                && snapshot.catalog.skills.contains { $0.id == "catalog-1" }
        case .following:
            return snapshot.followedPublishers.contains("superpowers")
                && snapshot.feed.contains { $0.kind == "updated" }
        }
    }

    static func snapshot(copy: [String: String]) -> DaemonData.SkillsSnapshot {
        let instructions = "https://github.com/obra/superpowers#installation"
        return .init(enabled: true,
            inventory: .init(skills: [.init(
                id: "installed-1", name: "test-driven-development", description: "Start with the behavior to verify.",
                agent: "codex", scope: "global", location: "~/.agents/skills/test-driven-development/SKILL.md",
                sha256: String(repeating: "a", count: 64), source: .init(repository: "obra/superpowers", path: "skills/test-driven-development/SKILL.md"))],
                partial: true, coverage: [.init(location: "~/.agents/skills", status: "complete")]),
            catalog: .init(publishers: [.init(id: "superpowers", name: "Superpowers", repository: "obra/superpowers",
                skillsRoot: "skills", instructionsURL: instructions)],
                skills: [.init(id: "catalog-1", publisherID: "superpowers", name: "test-driven-development",
                    description: "Start with the behavior to verify.", repository: "obra/superpowers",
                    path: "skills/test-driven-development/SKILL.md", revision: String(repeating: "b", count: 40), sha256: String(repeating: "c", count: 64),
                    sourceURL: "https://github.com/obra/superpowers/tree/main/skills/test-driven-development",
                    instructionsURL: instructions)], checkedAt: "2026-11-05T18:00:00Z",
                publisherStates: [.init(publisherID: "superpowers", status: "current")], partial: true),
            followedPublishers: ["superpowers"],
            updates: [.init(installedID: "installed-1", status: "update_available", catalogSkillID: "catalog-1")],
            feed: [.init(id: "feed-1", publisherID: "superpowers", skillID: "catalog-1", kind: "updated",
                         observedAt: "2026-11-05T18:00:00Z")], feedPartial: true,
            usage: .init(sessionsObserved: 12, sessionsUnknown: 3, partial: true,
                skills: [.init(name: "test-driven-development", agent: "codex", invocations: 0, loads: 4,
                    sessions: 3, lastSeen: "2026-11-05T17:42:00Z"),
                    .init(name: "test-driven-development", agent: "claude_code", invocations: 2, loads: 0, sessions: 1,
                          population: "delegated", evidence: [.init(sessionRef: "child", parentSessionRef: "parent")]),
                    .init(name: "review", agent: "opencode", invocations: 1, loads: 0, sessions: 1, population: "unknown")],
                coverage: [.init(agent: "claude_code", status: "explicit_invocations"),
                           .init(agent: "codex", status: "loads_only"),
                           .init(agent: "opencode", status: "explicit_invocations")], delegatedSessionsObserved: 2, delegatedSessionsUnknown: 1, unclassifiedSessionsObserved: 1, unclassifiedSessionsUnknown: 1), copy: copy)
    }
}
