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

    func testHeadlinePlacesTheNameAsPlainCharactersSetStrong() {
        let name = "**review** [x](https://example.com) {name}"
        let line = InsightsSkillsHomeView.headline("{name} has an update", name: name)
        XCTAssertEqual(String(line.characters), name + " has an update", "Catalog text is never parsed as markup or as a template")
        let strong = line.runs.filter { $0.inlinePresentationIntent == .stronglyEmphasized }
            .map { String(line[$0.range].characters) }
        XCTAssertEqual(strong, [name])
        XCTAssertEqual(String(InsightsSkillsHomeView.headline("You used {name} more", name: "review").characters), "You used review more")
        XCTAssertEqual(String(InsightsSkillsHomeView.headline("", name: "review").characters), "review", "Missing copy still names the skill")
    }

    func testHomeFeedRendersAtPaneAndWideWidths() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let model = InsightsSkillsModel(copy: copy)
        let publisher = DaemonData.SkillsPublisher(id: "creator", name: "Obra", repository: "creator/skills", skillsRoot: "skills", instructionsURL: "https://example.com/install")
        // A catalog change for the skill with the update would be left out as
        // the same news, so the creator stories are about other skills.
        let described = DaemonData.CatalogSkill(id: "catalog-brainstorming", publisherID: publisher.id, name: "brainstorming", description: "Explores intent, requirements and design before any creative work, one question at a time.", repository: "creator/skills", path: "skills/brainstorming", revision: "main", sha256: String(repeating: "c", count: 64), sourceURL: "https://example.com/brainstorming", instructionsURL: "https://example.com/install")
        let added = DaemonData.CatalogSkill(id: "catalog-worktrees", publisherID: publisher.id, name: "using-git-worktrees", description: "", repository: "creator/skills", path: "skills/using-git-worktrees", revision: "main", sha256: String(repeating: "d", count: 64), sourceURL: "https://example.com/worktrees", instructionsURL: "https://example.com/install")
        let event = DaemonData.SkillsFeedEvent(id: "creator-update", publisherID: publisher.id, skillID: described.id, kind: "updated", observedAt: "2026-11-15T09:00:00Z")
        let earlier = DaemonData.SkillsFeedEvent(id: "creator-add", publisherID: publisher.id, skillID: added.id, kind: "added", observedAt: "2026-11-03T09:00:00Z")
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        // The same stories as the first feed render, plus what the newer
        // cards need: daily history, the week's top skills, and three
        // changes from a second publisher this week (a roll-up).
        let second = DaemonData.SkillsPublisher(id: "anthropic-skills", name: "Example Labs", repository: "example/skills", skillsRoot: "skills", instructionsURL: "https://example.com/install")
        let rolled = ["pdf", "spreadsheets", "frontend-design"].enumerated().map { index, name in
            let skill = DaemonData.CatalogSkill(id: "catalog-" + name, publisherID: second.id, name: name, description: "", repository: second.repository, path: "skills/" + name, revision: "main", sha256: String(repeating: "e", count: 64), sourceURL: "https://example.com/" + name, instructionsURL: "https://example.com/install")
            let observed = now.addingTimeInterval(-Double(2 * 86_400 + index * 3600))
            return InsightsSkillsHome.CreatorUpdate(id: "rolled-" + name, event: .init(id: "rolled-" + name, publisherID: second.id, skillID: skill.id, kind: index == 0 ? "added" : "updated", observedAt: "2026-11-13T12:00:00Z"), skill: skill, publisher: second, observedAt: observed)
        }
        let feed = InsightsSkillsHome.Snapshot(
            updates: [.init(id: "update-review", groupID: "installed-review", installedID: "installed-review", name: "review", agents: ["claude_code", "codex"], catalogSkill: catalogSkill, checkedAt: now, publisherName: publisher.name)],
            trends: [.init(id: "claude_code\u{0}review", name: "systematic-debugging", agent: "claude_code", previousUses: 2, currentUses: 7, direction: .more, periodEnd: now,
                           daily: [0, 1, 0, 0, 1, 0, 0, 1, 0, 2, 1, 0, 2, 1]),
                     .init(id: "opencode\u{0}planning", name: "planning", agent: "opencode", previousUses: 4, currentUses: 0, direction: .less, periodEnd: now,
                           daily: [1, 0, 2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
                     // No use the week before, and an agent with no logo artwork.
                     .init(id: "cline\u{0}writing-plans", name: "writing-plans", agent: "cline", previousUses: 0, currentUses: 3, direction: .started, periodEnd: now,
                           daily: [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0])],
            creators: [.init(id: event.id, event: event, skill: described, publisher: publisher, observedAt: now.addingTimeInterval(-3 * 3600)),
                       .init(id: earlier.id, event: earlier, skill: added, publisher: publisher, observedAt: now.addingTimeInterval(-12 * 86_400 - 3 * 3600))] + rolled,
            week: [.init(id: "claude_code\u{0}review", name: "systematic-debugging", agent: "claude_code", uses: 7),
                   .init(id: "claude_code\u{0}brainstorming", name: "brainstorming", agent: "claude_code", uses: 4),
                   .init(id: "cline\u{0}writing-plans", name: "writing-plans", agent: "cline", uses: 3)])
        let groups = feed.timeline(now: now, calendar: utc)
        XCTAssertEqual(groups.map(\.period), [.today, .thisWeek, .earlier])
        XCTAssertEqual(groups.map { $0.stories.map(\.id) }, [
            ["update-review", "creator:creator-update", "trend:claude_code\u{0}review", "trend:opencode\u{0}planning", "trend:cline\u{0}writing-plans"],
            ["week", "rollup:thisWeek:anthropic-skills"],
            ["creator:creator-add"]])
        XCTAssertEqual(InsightsSkillsHome.Snapshot.heroID(in: groups), "update-review")
        // Swift reads these through the FFI dylib: a stale one draws blanks.
        for key in ["skills_home_today", "skills_home_this_week", "skills_home_earlier",
                    "skills_home_story_update", "skills_home_story_more", "skills_home_story_less",
                    "skills_home_story_added", "skills_home_story_updated",
                    "skills_home_compare_up", "skills_home_compare_down", "skills_home_week", "skills_home_rollup",
                    "skills_home_view", "skills_home_trend_period",
                    // Other tables' words the feed reuses.
                    "skills_invocations", "skills_inspect", "skills_uses", "skills_usage_period_label", "skills_usage_chart_accessibility"] {
            XCTAssertFalse(model.text(key).isEmpty, key)
        }
        for period in InsightsSkillsHome.Period.allCases {
            XCTAssertFalse(model.text(period.copyKey).isEmpty, period.copyKey)
        }
        // TC_SKILLS_HOME_RENDER_SCHEME=light draws the light appearance.
        let light = ProcessInfo.processInfo.environment["TC_SKILLS_HOME_RENDER_SCHEME"] == "light"
        let saved = NSApplication.shared.appearance
        NSApplication.shared.appearance = NSAppearance(named: light ? .aqua : .darkAqua)
        defer { NSApplication.shared.appearance = saved }
        for width: CGFloat in [408, 800] {
            // One frame is drawn, so the feed is drawn still: with motion on,
            // a chart's first frame is its bars before they rise.
            let content = InsightsSkillsHomeView(feed: feed, model: model, now: now, calendar: utc, motion: .still, open: { _ in })
                .insightsSurface()
                .frame(width: width)
                .fixedSize(horizontal: false, vertical: true)
                .padding(16)
                .background(light ? Color(red: 0.93, green: 0.93, blue: 0.95) : Color(red: 0.18, green: 0.19, blue: 0.23))
                .environment(\.colorScheme, light ? .light : .dark)
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
            XCTAssertGreaterThan(image.pixelsHigh, 1_500)
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
