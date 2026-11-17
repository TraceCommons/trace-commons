import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsHomeRankingTests: XCTestCase {
    private typealias Home = InsightsSkillsHome
    private let now = ISO8601DateFormatter().date(from: "2026-11-17T12:00:00Z")!
    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar
    }
    private func date(_ value: String) -> Date { ISO8601DateFormatter().date(from: value)! }
    private func skill(_ id: String, publisher: String = "maker", name: String = "review") -> DaemonData.CatalogSkill {
        .init(id: id, publisherID: publisher, name: name, description: "", repository: publisher + "/skills",
              path: "skills/" + id + "/SKILL.md", revision: "main", sha256: String(repeating: "b", count: 64),
              sourceURL: "https://example.com/source", instructionsURL: "https://example.com")
    }
    private func change(_ id: String, skillID: String? = nil, publisher: String = "maker", observed: String = "2026-11-17T11:30:00Z") -> Home.CreatorUpdate {
        let publisher = DaemonData.SkillsPublisher(id: publisher, name: publisher, repository: publisher + "/skills", skillsRoot: "skills", instructionsURL: "https://example.com")
        return .init(id: id, event: .init(id: id, publisherID: publisher.id, skillID: skillID ?? id, kind: "updated", observedAt: observed),
                     skill: skill(skillID ?? id, publisher: publisher.id), publisher: publisher, observedAt: date(observed))
    }
    private func update(_ id: String, skillID: String? = nil) -> Home.Update {
        .init(id: "update:" + id, groupID: id, installedID: id, name: id, agents: ["claude_code"],
              catalogSkill: skill(skillID ?? id), checkedAt: date("2026-11-17T10:00:00Z"))
    }
    private func trend(_ id: String, previous: Int = 2, current: Int = 5, agent: String = "claude_code") -> Home.Trend {
        .init(id: agent + "\u{0}" + id, name: id, agent: agent, previousUses: previous, currentUses: current,
              direction: current > previous ? (previous == 0 ? .started : .more) : .less,
              periodEnd: date("2026-11-17T00:00:00Z"))
    }
    private func ranked(_ snapshot: Home.Snapshot) -> [Home.Story] { snapshot.rankedStories(now: now, calendar: calendar) }

    // A newer unrelated catalog observation must not hide an actionable update.
    func testVerifiedUsedUpdateLeadsNewerDiscoveryAndKeepsObservedTime() throws {
        let news = change("discovery", publisher: "other")
        let feed = Home.Snapshot(updates: [update("mine")], trends: [trend("mine")], creators: [news],
                                 context: .init(usedSkillIDs: ["mine"]))
        let stories = ranked(feed)
        XCTAssertEqual(stories.first?.id, "update:mine")
        XCTAssertEqual(Home.Snapshot.rankedHeroID(in: stories), "update:mine")
        guard case .creator(let retained) = try XCTUnwrap(stories.first { $0.id == "creator:discovery" }) else { return XCTFail("Missing creator story") }
        XCTAssertEqual(retained.observedAt, date("2026-11-17T11:30:00Z"))
    }

    // Returning every chronological candidate would expose tiny changes as insights.
    func testOneEventAndNegligibleRelativeChangesAreNotStories() {
        let feed = Home.Snapshot(updates: [], trends: [trend("first", previous: 0, current: 1), trend("tiny", previous: 10, current: 11),
                                                     trend("flat", previous: 1_000, current: 1_002), trend("meaningful")], creators: [])
        XCTAssertEqual(ranked(feed).map(\.id), ["trend:claude_code\u{0}meaningful"])
    }

    // Unbounded use volume would bury an exact local skill match.
    func testExactUsedIdentityOutweighsVolumeWithoutMatchingAnIdenticalNameFromAnotherAgent() {
        let local = trend("review")
        let unrelated = trend("review", previous: 0, current: Int.max, agent: "opencode")
        let feed = Home.Snapshot(updates: [], trends: [unrelated, local], creators: [],
                                 context: .init(usedSkillIDs: ["catalog-review"], catalogIDsByUsageID: [local.id: ["catalog-review"]]))
        XCTAssertEqual(ranked(feed).first?.id, "trend:claude_code\u{0}review")
    }

    // Following customizes the mixed feed without erasing discovery.
    func testFollowedCreatorOutranksEqualDiscoveryButBothRemain() {
        let feed = Home.Snapshot(updates: [], trends: [], creators: [change("a", publisher: "discovery"), change("z", publisher: "followed")],
                                 context: .init(followedPublisherIDs: ["followed"]))
        XCTAssertEqual(ranked(feed).map(\.id), ["creator:z", "creator:a"])
    }

    // Similar display names must never grant installed-skill relevance.
    func testInstalledIdentityBoostsOnlyTheExactCatalogSkill() {
        let feed = Home.Snapshot(updates: [], trends: [], creators: [change("a", skillID: "lookalike", publisher: "other"), change("z", skillID: "installed")],
                                 context: .init(installedSkillIDs: ["installed"]))
        XCTAssertEqual(ranked(feed).first?.id, "creator:z")
    }

    // A large batch cannot push the other publisher out of the first few cards.
    func testPublisherBatchVolumeDoesNotOutrankPersonalUsageOrCrowdOutOtherCreators() {
        let burst = (0..<30).map { change("burst-\($0)", publisher: "prolific") }
        let feed = Home.Snapshot(updates: [], trends: [trend("local")], creators: burst + [change("other", publisher: "other")],
                                 context: .init(followedPublisherIDs: ["prolific"]))
        let stories = ranked(feed)
        XCTAssertEqual(stories.first?.id, "trend:claude_code\u{0}local")
        XCTAssertTrue(stories.prefix(3).contains { $0.id == "creator:other" })
        XCTAssertEqual(stories.filter { if case .rollup = $0 { return true }; return false }.count, 1)
    }

    // The same topic should not occupy consecutive cards while another topic is available.
    func testTopicAndSourceDiversityBreaksRepeatedCreatorAndUsageRuns() {
        let local = trend("review")
        let feed = Home.Snapshot(updates: [update("mine", skillID: "review")], trends: [local, trend("other")],
                                 creators: [change("review", publisher: "followed")],
                                 context: .init(usedSkillIDs: ["review"], followedPublisherIDs: ["followed"], catalogIDsByUsageID: [local.id: ["review"]]))
        let stories = ranked(feed)
        XCTAssertEqual(stories.first?.id, "update:mine")
        XCTAssertEqual(stories.dropFirst().first?.id, "trend:claude_code\u{0}other")
    }

    // A local-usage burst must leave room for real discovery near the top.
    func testRepeatedRelevantUsageAllowsAnotherSourceWithoutStrictAlternation() {
        let changes = (0..<6).map { trend("local-\($0)") }
        let exactIDs = Set((0..<6).map { "catalog-\($0)" })
        let identities = Dictionary(uniqueKeysWithValues: changes.enumerated().map { index, item in
            (item.id, Set(["catalog-\(index)"]))
        })
        let feed = Home.Snapshot(updates: [], trends: changes, creators: [change("discovery", publisher: "other")],
                                 context: .init(usedSkillIDs: exactIDs, catalogIDsByUsageID: identities))
        let stories = ranked(feed)
        XCTAssertEqual(stories.first?.id, "trend:claude_code\u{0}local-0")
        XCTAssertTrue(stories.prefix(3).contains { $0.id == "creator:discovery" })
        XCTAssertEqual(stories.count, 7, "Diversity reorders supported stories rather than dropping a source")
    }

    // Ranking must use real age, not the Today group assigned by timeline.
    func testFreshDiscoveryCanBeatOldFollowedNewsAndInvalidAgesAreExcluded() {
        let feed = Home.Snapshot(updates: [], trends: [], creators: [change("fresh", publisher: "new"),
            change("old-followed", publisher: "followed", observed: "2026-10-20T11:30:00Z"),
            change("expired", publisher: "expired", observed: "2026-09-01T11:30:00Z"),
            change("future", publisher: "future", observed: "2026-11-18T11:30:00Z")], context: .init(followedPublisherIDs: ["followed"]))
        XCTAssertEqual(ranked(feed).map(\.id), ["creator:fresh", "creator:old-followed"])
    }

    // Duplicate evidence across periods or updates must not make duplicate cards.
    func testDuplicateSkillNewsIsSuppressedBeforeRollup() {
        let feed = Home.Snapshot(updates: [update("mine", skillID: "installed"), update("copy", skillID: "installed")], trends: [], creators: [
            change("same-as-update", skillID: "installed"), change("new", skillID: "one", observed: "2026-11-17T11:00:00Z"),
            change("old", skillID: "one", observed: "2026-11-09T11:00:00Z")])
        let stories = ranked(feed)
        XCTAssertEqual(stories.count, 2)
        XCTAssertTrue(stories.contains { $0.id == "creator:new" })
        XCTAssertFalse(stories.contains { $0.id == "creator:old" })
    }

    // Input iteration order must not determine equal scores or the bounded output.
    func testStableTiesAndTwelveStoryLimit() {
        let changes = (0..<25).map { change(String(format: "%02d", $0), publisher: "publisher-\($0)") }
        let first = ranked(Home.Snapshot(updates: [], trends: [], creators: changes)).map(\.id)
        let reversed = ranked(Home.Snapshot(updates: [], trends: [], creators: changes.reversed())).map(\.id)
        XCTAssertEqual(first, reversed)
        XCTAssertEqual(first.count, 12)
        XCTAssertEqual(first.first, "creator:00")
        XCTAssertEqual(Set(first).count, first.count)
    }

    // Missing evidence must leave an empty result and cannot manufacture a hero.
    func testEmptyAndUsageOnlyFeedsDoNotForceUnsupportedCardsOrCover() {
        XCTAssertTrue(ranked(Home.Snapshot(updates: [], trends: [], creators: [])).isEmpty)
        let stories = ranked(Home.Snapshot(updates: [], trends: [trend("only")], creators: []))
        XCTAssertEqual(stories.map(\.id), ["trend:claude_code\u{0}only"])
        XCTAssertNil(Home.Snapshot.rankedHeroID(in: stories))
    }
}
