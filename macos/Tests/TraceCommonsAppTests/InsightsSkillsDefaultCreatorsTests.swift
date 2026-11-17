import AppKit
import Foundation
import SwiftUI
import TCBridge
import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsDefaultCreatorsTests: XCTestCase {
    private let now = ISO8601DateFormatter().date(from: "2026-11-15T12:00:00Z")!

    private func snapshot(defaults: [String] = ["obra"], followed: [String] = [],
                          homeFeed: [DaemonData.SkillsFeedEvent] = [], feed: [DaemonData.SkillsFeedEvent] = [],
                          skills: Bool = true, analysis: Bool = true) throws -> DaemonData.SkillsSnapshot {
        let publisher = DaemonData.SkillsPublisher(id: "obra", name: "Obra", repository: "obra/superpowers",
                                                 skillsRoot: "skills", instructionsURL: "https://github.com/obra/superpowers")
        let skill = DaemonData.CatalogSkill(id: "review", publisherID: "obra", name: "review", description: "Review changes.",
                                          repository: publisher.repository, path: "skills/review/SKILL.md", revision: "main",
                                          sha256: String(repeating: "a", count: 64), sourceURL: publisher.instructionsURL,
                                          instructionsURL: publisher.instructionsURL)
        let base = DaemonData.SkillsSnapshot(enabled: true, catalog: .init(publishers: [publisher], skills: skills ? [skill] : []),
                                            followedPublishers: followed, feed: feed, usage: .init(enabled: analysis, partial: true))
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(base)) as? [String: Any])
        json["default_publishers"] = defaults
        json["home_feed"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(homeFeed))
        return try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONSerialization.data(withJSONObject: json))
    }

    private func event(_ id: String = "recorded", publisher: String = "obra", skill: String = "review") -> DaemonData.SkillsFeedEvent {
        .init(id: id, publisherID: publisher, skillID: skill, kind: "updated", observedAt: "2026-11-14T10:00:00Z")
    }

    private func project(_ snapshot: DaemonData.SkillsSnapshot) -> InsightsSkillsHome.Snapshot {
        InsightsSkillsHome.make(snapshot, model: InsightsSkillsModel(), now: now)
    }

    func testDefaultCreatorBaselineHasContentWithoutInventingActivityOrFollowing() throws {
        let snapshot = try snapshot()
        let home = project(snapshot)
        XCTAssertFalse(home.isEmpty, "A catalog baseline offers its default creators before any events exist")
        XCTAssertEqual(home.defaultCreators.map(\.id), ["obra"])
        XCTAssertEqual(home.defaultCreators.first?.skills.map(\.id), ["review"])
        XCTAssertTrue(home.timeline(now: now).isEmpty, "Introductions have no invented activity timestamp")
        XCTAssertTrue(home.creators.isEmpty)
        XCTAssertTrue(home.trends.isEmpty)
        XCTAssertTrue(home.week.isEmpty)
        XCTAssertTrue(snapshot.followedPublishers.isEmpty)
        XCTAssertTrue(snapshot.feed.isEmpty)
    }

    func testDefaultHomeNewsUsesOnlyRecordedEventsForKnownDefaultTargets() throws {
        let snapshot = try snapshot(homeFeed: [event(), event("other", publisher: "other"), event("missing", skill: "absent")])
        XCTAssertEqual(project(snapshot).creators.map(\.id), ["recorded"])
        XCTAssertTrue(snapshot.feed.isEmpty, "Following does not inherit default Home news")
    }

    func testExplicitFollowsPersonalizeMixedHomeAndKeepFollowingFeed() throws {
        let snapshot = try snapshot(followed: ["obra"], homeFeed: [event("default")], feed: [event("followed")])
        XCTAssertEqual(project(snapshot).creators.map(\.id), ["followed"])
        XCTAssertEqual(project(snapshot).defaultCreators.map(\.id), ["obra"])
        XCTAssertEqual(snapshot.feed.map(\.id), ["followed"])
    }

    func testFollowingOneCreatorKeepsOtherDefaultNewsOnHome() throws {
        // The follow can name another creator even when this fixture only has
        // the curated creator's catalog. Home must still retain discovery.
        let snapshot = try snapshot(followed: ["other"], homeFeed: [event()])
        XCTAssertEqual(project(snapshot).creators.map(\.id), ["recorded"])
        XCTAssertEqual(project(snapshot).defaultCreators.map(\.id), ["obra"])
        XCTAssertTrue(snapshot.feed.isEmpty)
    }

    func testCreatorBurstDoesNotCrowdOutAnotherPublisherBeforeRanking() throws {
        var raw = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(try snapshot(defaults: ["obra", "slack"]))) as? [String: Any])
        var catalog = try XCTUnwrap(raw["catalog"] as? [String: Any])
        var otherPublisher = try XCTUnwrap((catalog["publishers"] as? [[String: Any]])?.first)
        otherPublisher["id"] = "slack"
        var publishers = try XCTUnwrap(catalog["publishers"] as? [[String: Any]])
        publishers.append(otherPublisher)
        catalog["publishers"] = publishers
        let seed = try XCTUnwrap((catalog["skills"] as? [[String: Any]])?.first)
        var skills = (0..<20).map { index in
            var row = seed
            row["id"] = "burst-\(index)"
            return row
        }
        var other = seed
        other["id"] = "slack-skill"
        other["publisher_id"] = "slack"
        skills.append(other)
        catalog["skills"] = skills
        raw["catalog"] = catalog
        let burst = (0..<20).map { event("burst-\($0)", skill: "burst-\($0)") }
        raw["home_feed"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(burst + [event("slack-news", publisher: "slack", skill: "slack-skill")]))
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONSerialization.data(withJSONObject: raw))
        let candidates = project(snapshot).creators
        XCTAssertTrue(candidates.contains { $0.publisher.id == "slack" })
        XCTAssertLessThanOrEqual(candidates.filter { $0.publisher.id == "obra" }.count, 8)
    }

    func testMissingCatalogAndDisabledAnalysisDoNotInventDefaultContent() throws {
        XCTAssertTrue(project(try snapshot(skills: false)).isEmpty)
        XCTAssertTrue(project(try snapshot(defaults: ["unknown"])).isEmpty)
        XCTAssertTrue(project(try snapshot(homeFeed: [event()], analysis: false)).isEmpty)
    }

    func testOlderSnapshotsDecodeWithoutImplicitDefaults() throws {
        let old = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(#"{"enabled":true}"#.utf8))
        XCTAssertTrue(old.defaultPublishers.isEmpty)
        XCTAssertTrue(old.homeFeed.isEmpty)
        XCTAssertFalse(old.homeFeedPartial)
        XCTAssertTrue(project(old).isEmpty)
        let current = try snapshot(homeFeed: [event()])
        let roundtrip = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONEncoder().encode(current))
        XCTAssertEqual(roundtrip.defaultPublishers, ["obra"])
        XCTAssertEqual(roundtrip.homeFeed.map(\.id), ["recorded"])
        XCTAssertTrue(roundtrip.followedPublishers.isEmpty)
    }

    func testDefaultCardsDeduplicateAndBoundRealSkills() throws {
        var raw = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(try snapshot(defaults: ["obra", "obra", "missing"]))) as? [String: Any])
        var catalog = try XCTUnwrap(raw["catalog"] as? [String: Any])
        let seed = try XCTUnwrap((catalog["skills"] as? [[String: Any]])?.first)
        var skills: [[String: Any]] = (0..<5).map { index in
            var row = seed
            row["id"] = "skill-\(index)"
            row["name"] = "skill-\(index)"
            return row
        }
        skills.append(skills[0])
        var foreign = seed
        foreign["id"] = "foreign"
        foreign["repository"] = "another/repository"
        skills.insert(foreign, at: 0)
        catalog["skills"] = skills
        raw["catalog"] = catalog
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONSerialization.data(withJSONObject: raw))
        let cards = project(snapshot).defaultCreators
        XCTAssertEqual(cards.map(\.id), ["obra"])
        XCTAssertEqual(cards[0].skills.map(\.id), ["skill-0", "skill-1", "skill-2"])
        for skill in cards[0].skills {
            XCTAssertEqual(SkillsHomeRoute.resolve(.catalog(id: skill.id), in: snapshot)?.catalogID, skill.id)
        }
        for gate in ["enabled", "readable"] {
            var closed = raw
            closed[gate] = false
            XCTAssertTrue(project(try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONSerialization.data(withJSONObject: closed))).isEmpty)
        }
    }

    func testFollowActionKeepsDiscoveryAfterConfirmedWrite() async throws {
        let initial = try snapshot()
        let confirmed = try snapshot(followed: ["obra"], feed: [event()])
        let calls = FollowCalls()
        let model = InsightsSkillsModel(reader: { _ in initial }, writer: { publisher, followed in
            await calls.record(publisher, followed)
            return confirmed
        })
        await model.load()
        XCTAssertEqual(project(try XCTUnwrap(model.shown)).defaultCreators.map(\.id), ["obra"])
        await model.setFollow(publisherID: "obra", followed: true)
        let requests = await calls.values
        XCTAssertEqual(requests, ["obra:true"])
        let shown = try XCTUnwrap(model.shown)
        XCTAssertEqual(shown.followedPublishers, ["obra"])
        XCTAssertEqual(project(shown).defaultCreators.map(\.id), ["obra"])
        XCTAssertEqual(project(shown).creators.map(\.id), ["recorded"])
    }

    func testDefaultCreatorCardsRenderWithSharedCopy() async throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["skills_home_creators"], "Creators")
        let snapshot = try snapshot()
        let model = InsightsSkillsModel(reader: { _ in snapshot }, writer: { _, _ in snapshot }, copy: copy)
        await model.load()
        for width: CGFloat in [440, 900] {
            let png = try await InsightsGetStartedRenderTests.render(
                AnyView(ScrollView { InsightsSkillsHomeView(feed: project(snapshot), model: model, now: now, motion: .still, open: { _ in }).insightsSurface() }),
                width: width, height: 560, ready: { model.canFollow && !model.text("skills_home_creators").isEmpty })
            XCTAssertGreaterThan(png.count, 10_000)
            if let directory = ProcessInfo.processInfo.environment["TC_SKILLS_DEFAULT_RENDER_DIR"] {
                try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
                try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("default-creators-\(Int(width)).png"))
            }
        }
        if let path = ProcessInfo.processInfo.environment["TC_SKILLS_DEFAULT_REPLAY"],
           let directory = ProcessInfo.processInfo.environment["TC_SKILLS_DEFAULT_RENDER_DIR"] {
            let retained = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(contentsOf: URL(fileURLWithPath: path)))
            let retainedModel = InsightsSkillsModel(reader: { _ in retained }, writer: { _, _ in retained }, copy: copy)
            await retainedModel.load()
            let home = InsightsSkillsHome.make(retained, model: retainedModel)
            for width: CGFloat in [440, 900] {
                let png = try await InsightsGetStartedRenderTests.render(
                    AnyView(ScrollView { InsightsSkillsHomeView(feed: home, model: retainedModel, motion: .still, open: { _ in }).insightsSurface() }),
                    width: width, height: 1800, ready: { retainedModel.canFollow && !home.isEmpty })
                try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("retained-home-\(Int(width)).png"))
            }
        }
    }

    /// Opt-in replay of current retained data. Coverage varies by agent and
    /// date; ordinary fixtures separately prove incomplete windows fail closed.
    func testRetainedSnapshotProjection() throws {
        guard let path = ProcessInfo.processInfo.environment["TC_SKILLS_DEFAULT_REPLAY"] else {
            throw XCTSkip("Set TC_SKILLS_DEFAULT_REPLAY to replay the retained local snapshot")
        }
        let snapshot = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: Data(contentsOf: URL(fileURLWithPath: path)))
        let home = InsightsSkillsHome.make(snapshot, model: InsightsSkillsModel())
        XCTAssertEqual(home.defaultCreators.map(\.id), ["obra", "poteto", "slack"])
        XCTAssertTrue(home.defaultCreators.allSatisfy { !$0.skills.isEmpty && $0.skills.count <= 3 })
        XCTAssertFalse(home.creators.isEmpty)
        XCTAssertFalse(home.isEmpty)
        XCTAssertTrue(snapshot.followedPublishers.isEmpty)
        XCTAssertTrue(snapshot.feed.isEmpty)
        let recorded = Set(snapshot.homeFeed.map(\.id))
        XCTAssertTrue(home.creators.allSatisfy { recorded.contains($0.id) })
        XCTAssertTrue(home.week.allSatisfy { $0.uses > 0 })
        let stories = home.rankedStories()
        XCTAssertLessThanOrEqual(stories.count, 12)
        XCTAssertEqual(Set(stories.map(\.id)).count, stories.count)
        print("Ranked retained Home: " + stories.map(\.id).joined(separator: ", "))
        print("Retained Home replay: \(home.defaultCreators.count) creators, \(home.defaultCreators.flatMap(\.skills).count) catalog examples, \(home.creators.count) real changes; explicit follows remain empty")
    }
}

private actor FollowCalls {
    var values: [String] = []
    func record(_ id: String, _ followed: Bool) { values.append("\(id):\(followed)") }
}
