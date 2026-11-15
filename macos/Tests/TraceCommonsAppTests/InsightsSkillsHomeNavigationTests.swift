import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsHomeNavigationTests: XCTestCase {
    private let now = ISO8601DateFormatter().date(from: "2026-11-15T12:00:00Z")!
    private let catalogSkill = DaemonData.CatalogSkill(id: "catalog-review", publisherID: "creator", name: "review", description: "", repository: "creator/skills", path: "skills/review", revision: "main", sha256: String(repeating: "b", count: 64), sourceURL: "https://example.com/review", instructionsURL: "https://example.com/install")

    private var snapshot: DaemonData.SkillsSnapshot {
        .init(enabled: true, catalog: .init(skills: [catalogSkill]), usage: .init(enabled: true,
            skills: [.init(name: "review", agent: "claude_code", invocations: 0, loads: 0, sessions: 0,
                           windows: [.init(days: 7, invocations: 0, loads: 0, sessions: 0)])],
            coverage: [.init(agent: "claude_code", status: "explicit_invocations")]))
    }

    func testHomeIsTheDefaultSkillsDestination() {
        XCTAssertEqual(SkillsSection.defaultSection, .home)
        XCTAssertEqual(SkillsSection.allCases.first, .home)
    }

    func testFeedActionsResolveExactCatalogAndUsageTargets() throws {
        let catalog = try XCTUnwrap(SkillsHomeRoute.resolve(.catalog(id: catalogSkill.id), in: snapshot))
        XCTAssertEqual(catalog.section, .catalog)
        XCTAssertEqual(catalog.catalogID, catalogSkill.id)
        let usageID = "claude_code\u{0}review"
        let usage = try XCTUnwrap(SkillsHomeRoute.resolve(.usage(id: usageID), in: snapshot))
        XCTAssertEqual(usage.section, .usage)
        XCTAssertEqual(usage.usageID, usageID)
        XCTAssertEqual(SkillsHomeRoute.resolve(.index, in: snapshot)?.section, .inventory)
        XCTAssertEqual(SkillsHomeRoute.resolve(.browse, in: snapshot)?.section, .catalog)
    }

    func testUpdateNavigationKeepsTheOutdatedInstallationStatus() throws {
        let source = DaemonData.SkillsSource(repository: catalogSkill.repository, path: catalogSkill.path)
        let current = DaemonData.InstalledSkill(id: "first-current", name: "review", description: "", agent: "claude_code", scope: "global", location: "current", sha256: catalogSkill.sha256, source: source)
        let outdated = DaemonData.InstalledSkill(id: "second-outdated", name: "review", description: "", agent: "opencode", scope: "global", location: "outdated", sha256: String(repeating: "a", count: 64), source: source)
        let snapshot = DaemonData.SkillsSnapshot(enabled: true, inventory: .init(skills: [current, outdated]),
            catalog: .init(skills: [catalogSkill]), updates: [.init(installedID: current.id, status: "up_to_date", catalogSkillID: catalogSkill.id),
                .init(installedID: outdated.id, status: "update_available", catalogSkillID: catalogSkill.id)])
        let route = try XCTUnwrap(SkillsHomeRoute.resolve(.catalog(id: catalogSkill.id, installedID: outdated.id), in: snapshot))
        XCTAssertEqual(route.installedID, outdated.id)
        XCTAssertEqual(SkillsHomeRoute.catalogUpdate(catalogID: catalogSkill.id, installedID: route.installedID, in: snapshot)?.status, "update_available")
        XCTAssertEqual(SkillsHomeRoute.catalogUpdate(catalogID: catalogSkill.id, installedID: nil, in: snapshot)?.status, "up_to_date")
        XCTAssertNil(SkillsHomeRoute.resolve(.catalog(id: catalogSkill.id, installedID: "missing"), in: snapshot))
        let wrongSource = DaemonData.InstalledSkill(id: outdated.id, name: "review", description: "", agent: "opencode", scope: "global", location: "outdated", sha256: outdated.sha256, source: .init(repository: "other/skills", path: "review"))
        let changed = DaemonData.SkillsSnapshot(enabled: true, inventory: .init(skills: [wrongSource]), catalog: snapshot.catalog, updates: snapshot.updates)
        XCTAssertNil(SkillsHomeRoute.resolve(.catalog(id: catalogSkill.id, installedID: outdated.id), in: changed))
        XCTAssertNil(SkillsHomeRoute.catalogUpdate(catalogID: catalogSkill.id, installedID: outdated.id, in: changed))
    }

    func testStaleOrRevokedFeedTargetsDoNotOpenOtherSkills() {
        XCTAssertNil(SkillsHomeRoute.resolve(.catalog(id: "missing"), in: snapshot))
        XCTAssertNil(SkillsHomeRoute.resolve(.usage(id: "claude_code\u{0}missing"), in: snapshot))
        XCTAssertNil(SkillsHomeRoute.resolve(.browse, in: .init(enabled: false)))
        XCTAssertNil(SkillsHomeRoute.resolve(.index, in: .init(enabled: true, readable: false)))
        let unsupported = DaemonData.SkillsSnapshot(enabled: true, usage: .init(enabled: true,
            skills: snapshot.usage.skills, coverage: [.init(agent: "claude_code", status: "loads_only")]))
        XCTAssertNil(SkillsHomeRoute.resolve(.usage(id: "claude_code\u{0}review"), in: unsupported))
    }

    func testExplicitZeroUsageTargetStaysSelectableWithoutAddingZerosToNormalRanking() {
        let usageID = "claude_code\u{0}review"
        XCTAssertTrue(InsightsSkillsUsage.ranked(snapshot.usage, days: 7, now: now).isEmpty)
        let selected = InsightsSkillsUsage.ranked(snapshot.usage, days: 7, now: now, includingID: usageID)
        XCTAssertEqual(selected.map(\.id), [usageID])
        XCTAssertEqual(selected.first?.uses, 0)
        XCTAssertTrue(selected.first?.daily.isEmpty == true, "Missing daily evidence is not filled with invented zero buckets")
        XCTAssertTrue(InsightsSkillsUsage.ranked(snapshot.usage, days: 7, now: now, includingID: "missing").isEmpty)
    }

    func testHomeFeedRendersAtPaneAndWideWidths() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let model = InsightsSkillsModel(copy: copy)
        let publisher = DaemonData.SkillsPublisher(id: "creator", name: "Obra", repository: "creator/skills", skillsRoot: "skills", instructionsURL: "https://example.com/install")
        let event = DaemonData.SkillsFeedEvent(id: "creator-update", publisherID: publisher.id, skillID: catalogSkill.id, kind: "updated", observedAt: "2026-11-15T11:00:00Z")
        let feed = InsightsSkillsHome.Snapshot(
            updates: [.init(id: "update-review", groupID: "installed-review", installedID: "installed-review", name: "review", agents: ["claude_code", "codex"], catalogSkill: catalogSkill, checkedAt: now)],
            trends: [.init(id: "claude_code\u{0}review", name: "systematic-debugging", agent: "claude_code", previousUses: 2, currentUses: 7, direction: .more, periodEnd: now),
                     .init(id: "opencode\u{0}planning", name: "planning", agent: "opencode", previousUses: 4, currentUses: 0, direction: .less, periodEnd: now)],
            creators: [.init(id: event.id, event: event, skill: catalogSkill, publisher: publisher, observedAt: now)])
        for key in ["skills_home_updates", "skills_home_trends", "skills_home_creators", "skills_home_view", "skills_home_trend_period"] {
            XCTAssertFalse(model.text(key).isEmpty, key)
        }
        for width: CGFloat in [408, 800] {
            let content = InsightsSkillsHomeView(feed: feed, model: model, open: { _ in })
                .insightsSurface()
                .frame(width: width)
                .fixedSize(horizontal: false, vertical: true)
                .padding(16)
                .background(Color(red: 0.18, green: 0.19, blue: 0.23))
                .environment(\.colorScheme, .dark)
            let renderer = ImageRenderer(content: content)
            var bitmap: NSBitmapImageRep?
            renderer.render(rasterizationScale: 2) { size, paint in
                let pixelsWide = Int((size.width * 2).rounded(.up))
                let pixelsHigh = Int((size.height * 2).rounded(.up))
                guard let space = CGColorSpace(name: CGColorSpace.sRGB),
                      let context = CGContext(data: nil, width: pixelsWide, height: pixelsHigh, bitsPerComponent: 8,
                                              bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
                context.scaleBy(x: 2, y: 2)
                paint(context)
                bitmap = context.makeImage().map(NSBitmapImageRep.init(cgImage:))
            }
            let image = try XCTUnwrap(bitmap)
            XCTAssertEqual(image.pixelsWide, Int((width + 32) * 2))
            XCTAssertGreaterThan(image.pixelsHigh, 500)
            let png = try XCTUnwrap(image.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(png.count, 10_000)
            if let output = ProcessInfo.processInfo.environment["TC_SKILLS_HOME_RENDER_OUT"] {
                let url = URL(fileURLWithPath: output)
                let target = width == 408 ? url : url.deletingPathExtension().appendingPathExtension("wide.png")
                try png.write(to: target)
            }
        }
    }
}
