import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsHomeTests: XCTestCase {
    private let now = ISO8601DateFormatter().date(from: "2026-11-15T12:00:00Z")!
    private var calendar: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(secondsFromGMT: 0)!; return c }
    private let contentHash = String(repeating: "a", count: 64)
    private var publisher: DaemonData.SkillsPublisher { .init(id: "maker", name: "Maker", repository: "maker/skills", skillsRoot: "skills", instructionsURL: "https://example.com") }
    private var catalog: DaemonData.CatalogSkill { .init(id: "upstream", publisherID: "maker", name: "review", description: "", repository: "maker/skills", path: "skills/review/SKILL.md", revision: "main", sha256: String(repeating: "b", count: 64), sourceURL: "https://example.com/source", instructionsURL: "https://example.com") }
    private func installed(_ id: String = "local", agent: String = "claude_code") -> DaemonData.InstalledSkill {
        .init(id: id, name: "review", description: "", agent: agent, scope: "global", location: "/skills/\(id)", sha256: contentHash, source: .init(repository: "maker/skills", path: "skills/review/SKILL.md"))
    }
    private func row(_ population: String = "direct", previous: Int = 2, current: Int = 5) throws -> DaemonData.SkillsUsageRow {
        let value: [String: Any] = ["name":"review", "agent":"claude_code", "population":population, "invocations":previous+current, "loads":0, "sessions":1,
            "windows":[["days":30,"invocations":previous+current,"loads":0,"sessions":1]],
            "daily":[["day":"2026-10-30","invocations":1,"loads":0,"sessions":1], ["day":"2026-11-03","invocations":previous,"loads":0,"sessions":1], ["day":"2026-11-10","invocations":current,"loads":0,"sessions":1]]]
        return try JSONDecoder().decode(DaemonData.SkillsUsageRow.self, from: JSONSerialization.data(withJSONObject: value))
    }
    private func snapshot(rows: [DaemonData.SkillsUsageRow] = [], partial: Bool = false, followed: [String] = ["maker"], feed: [DaemonData.SkillsFeedEvent] = [], updates: [DaemonData.SkillsUpdate] = [], skills: [DaemonData.InstalledSkill]? = nil, status: String = "current", enabled: Bool = true, analysis: Bool = true, partialAgents: [DaemonData.SkillsPartialAgent]? = nil) -> DaemonData.SkillsSnapshot {
        .init(enabled: enabled, inventory: .init(skills: skills ?? [installed()], partial: false),
              catalog: .init(publishers: [publisher], skills: [catalog], checkedAt: "2026-11-15T11:00:00Z", publisherStates: [.init(publisherID:"maker",status:status,checkedAt:"2026-11-15T11:00:00Z")]),
              followedPublishers: followed, updates: updates, feed: feed,
              usage: .init(enabled: analysis, lastAnalysisAt:"2026-11-15T11:00:00Z",earliestEvidence:"2026-10-30T00:00:00Z",partial:partial,skills:rows,coverage:[.init(agent:"claude_code",status:"explicit_invocations")],partialAgents:partialAgents))
    }
    private func project(_ s: DaemonData.SkillsSnapshot) -> InsightsSkillsHome.Snapshot {
        InsightsSkillsHome.make(s, model: InsightsSkillsModel(), now: now, calendar: calendar)
    }
    func testUsedUpdatesRequireVerifiedSourceAndCurrentPublisher() throws {
        let update = DaemonData.SkillsUpdate(installedID:"local",status:"update_available",catalogSkillID:"upstream")
        XCTAssertEqual(project(snapshot(rows:[try row()],updates:[update])).updates.map(\.installedID), ["local"])
        XCTAssertTrue(project(snapshot(updates:[update])).updates.isEmpty)
        XCTAssertTrue(project(snapshot(rows:[try row()],updates:[update],status:"unknown")).updates.isEmpty)
        let wrong = DaemonData.SkillsUpdate(installedID:"local",status:"locally_modified",catalogSkillID:"upstream")
        XCTAssertTrue(project(snapshot(rows:[try row()],updates:[wrong])).updates.isEmpty)
    }
    func testMatchingInstalledCopiesProduceOnePersonalizedUpdate() throws {
        let updates = ["a","b"].map { DaemonData.SkillsUpdate(installedID:$0,status:"update_available",catalogSkillID:"upstream") }
        let result = project(snapshot(rows:[try row()],updates:updates,skills:[installed("a"),installed("b",agent:"opencode")]))
        XCTAssertEqual(result.updates.count,1)
        XCTAssertEqual(result.updates[0].agents,["claude_code","opencode"])
    }
    func testTrendsCompareCompletedWeeksAndCombineDisjointPopulations() throws {
        let trends = project(snapshot(rows:[try row(previous:2,current:5),try row("delegated",previous:1,current:2)])).trends
        XCTAssertEqual(trends.count,1)
        XCTAssertEqual(trends[0].previousUses,3)
        XCTAssertEqual(trends[0].currentUses,7)
        XCTAssertEqual(trends[0].direction,.more)
        XCTAssertEqual(project(snapshot(rows:[try row(previous:4,current:1)])).trends.first?.direction,.less)
        XCTAssertEqual(project(snapshot(rows:[try row(previous:0,current:1)])).trends.first?.direction,.started)
        XCTAssertTrue(project(snapshot(rows:[try row(previous:2,current:2)])).trends.isEmpty)
    }
    func testPartialHistoryUnknownPopulationsAndForeignAgentHistoryCannotCreateTrends() throws {
        XCTAssertTrue(project(snapshot(rows:[try row()],partial:true)).trends.isEmpty)
        XCTAssertTrue(project(snapshot(rows:[try row(),try row("unknown")])).trends.isEmpty)
        var raw = try JSONSerialization.jsonObject(with: JSONEncoder().encode(try row())) as! [String:Any]
        raw["daily"] = [["day":"2026-11-10","invocations":5,"loads":0,"sessions":1]]
        let recent = try JSONDecoder().decode(DaemonData.SkillsUsageRow.self, from: JSONSerialization.data(withJSONObject:raw))
        XCTAssertTrue(project(snapshot(rows:[recent])).trends.isEmpty,"Global oldest evidence cannot stand in for this agent's history")
    }
    func testCreatorChangesRequireFollowKnownTargetsAndValidRecentDates() {
        let events = [
            DaemonData.SkillsFeedEvent(id:"good",publisherID:"maker",skillID:"upstream",kind:"updated",observedAt:"2026-11-14T10:00:00Z"),
            .init(id:"future",publisherID:"maker",skillID:"upstream",kind:"added",observedAt:"2026-11-16T10:00:00Z"),
            .init(id:"bad",publisherID:"maker",skillID:"upstream",kind:"updated",observedAt:"invalid"),
            .init(id:"unknown",publisherID:"maker",skillID:"missing",kind:"updated",observedAt:"2026-11-14T10:00:00Z")]
        XCTAssertEqual(project(snapshot(feed:events)).creators.map(\.id),["good"])
        XCTAssertTrue(project(snapshot(followed:[],feed:events)).creators.isEmpty)
        XCTAssertEqual(project(snapshot(feed:events,analysis:false)).creators.count,1)
        XCTAssertTrue(project(snapshot(feed:events,enabled:false)).isEmpty)
    }
    func testTodayFutureAndMalformedDailyBucketsDoNotCreateTrends() throws {
        let base = try JSONSerialization.jsonObject(with: JSONEncoder().encode(try row())) as! [String:Any]
        for badDay in ["2026-11-16", "2026-02-30", "invalid"] {
            var value = base
            var daily = value["daily"] as! [[String:Any]]
            daily.append(["day":badDay,"invocations":100,"loads":0,"sessions":1])
            value["daily"] = daily
            let bad = try JSONDecoder().decode(DaemonData.SkillsUsageRow.self, from: JSONSerialization.data(withJSONObject:value))
            XCTAssertTrue(project(snapshot(rows:[bad])).trends.isEmpty)
        }
        var value = base
        var daily = value["daily"] as! [[String:Any]]
        daily.append(["day":"2026-11-15","invocations":100,"loads":0,"sessions":1])
        value["daily"] = daily
        let today = try JSONDecoder().decode(DaemonData.SkillsUsageRow.self, from: JSONSerialization.data(withJSONObject:value))
        XCTAssertEqual(project(snapshot(rows:[today])).trends.first?.currentUses,5)
    }
    func testCreatorChangesKeepNewestPerSkillAndOmitOldEvents() {
        let events = [
            DaemonData.SkillsFeedEvent(id:"old",publisherID:"maker",skillID:"upstream",kind:"added",observedAt:"2026-09-14T10:00:00Z"),
            .init(id:"earlier",publisherID:"maker",skillID:"upstream",kind:"updated",observedAt:"2026-11-13T10:00:00Z"),
            .init(id:"latest",publisherID:"maker",skillID:"upstream",kind:"updated",observedAt:"2026-11-14T10:00:00Z")]
        XCTAssertEqual(project(snapshot(feed:events)).creators.map(\.id),["latest"])
    }

    // MARK: The feed's order and periods

    private func date(_ value: String) -> Date { ISO8601DateFormatter().date(from: value)! }
    private func update(_ id: String, catalogID: String = "upstream", checked: String) -> InsightsSkillsHome.Update {
        let skill = catalogSkill(catalogID)
        return .init(id: "update:" + id, groupID: id, installedID: id, name: id, agents: ["claude_code"], catalogSkill: skill, checkedAt: date(checked))
    }
    private func trend(_ name: String, previous: Int, current: Int) -> InsightsSkillsHome.Trend {
        .init(id: "claude_code\u{0}" + name, name: name, agent: "claude_code", previousUses: previous, currentUses: current,
              direction: current > previous ? .more : .less, periodEnd: date("2026-11-15T00:00:00Z"))
    }
    /// Each change has its own publisher unless one is named: two from one
    /// publisher in a period are a roll-up.
    private func change(_ id: String, catalogID: String, observed: String, from publisherID: String? = nil) -> InsightsSkillsHome.CreatorUpdate {
        let skill = catalogSkill(catalogID)
        let maker = DaemonData.SkillsPublisher(id: publisherID ?? "maker-" + id, name: "Maker", repository: "maker/skills", skillsRoot: "skills", instructionsURL: "https://example.com")
        return .init(id: id, event: .init(id: id, publisherID: maker.id, skillID: catalogID, kind: "updated", observedAt: observed),
                     skill: skill, publisher: maker, observedAt: date(observed))
    }
    private func catalogSkill(_ id: String) -> DaemonData.CatalogSkill {
        .init(id: id, publisherID: "maker", name: "review", description: "", repository: "maker/skills", path: "skills/review/SKILL.md", revision: "main", sha256: String(repeating: "b", count: 64), sourceURL: "https://example.com/source", instructionsURL: "https://example.com")
    }
    private func ids(_ groups: [InsightsSkillsHome.Group]) -> [[String]] { groups.map { $0.stories.map(\.id) } }

    func testTimelineIsOneNewestFirstColumnUnderItsPeriods() {
        // 2026-11-15 is a Sunday: its ISO week began on Monday the 9th.
        let feed = InsightsSkillsHome.Snapshot(
            updates: [update("a", checked: "2026-11-15T11:00:00Z")],
            trends: [trend("big", previous: 1, current: 9), trend("small", previous: 3, current: 2)],
            creators: [change("noon", catalogID: "one", observed: "2026-11-15T11:30:00Z"),
                       change("morning", catalogID: "two", observed: "2026-11-15T08:00:00Z"),
                       change("yesterday", catalogID: "three", observed: "2026-11-14T23:59:59Z"),
                       change("monday", catalogID: "four", observed: "2026-11-09T00:00:00Z"),
                       change("last-week", catalogID: "five", observed: "2026-11-08T23:59:59Z")])
        let groups = feed.timeline(now: now, calendar: calendar)
        XCTAssertEqual(groups.map(\.period), [.today, .thisWeek, .earlier])
        XCTAssertEqual(ids(groups), [
            ["creator:noon", "update:a", "creator:morning", "trend:claude_code\u{0}big", "trend:claude_code\u{0}small"],
            ["creator:yesterday", "creator:monday"],
            ["creator:last-week"]])
        XCTAssertEqual(Set(groups.flatMap(\.stories).map(\.id)).count, 8, "Story ids are unique across kinds")
    }

    func testTimelineLeavesOutEmptyPeriodsAndAnEmptyFeed() {
        XCTAssertTrue(InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: []).timeline(now: now, calendar: calendar).isEmpty)
        let old = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [change("old", catalogID: "one", observed: "2026-10-20T10:00:00Z")])
        XCTAssertEqual(old.timeline(now: now, calendar: calendar).map(\.period), [.earlier])
        // On a Monday nothing can be earlier this week.
        let monday = date("2026-11-09T12:00:00Z")
        let sunday = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [change("sunday", catalogID: "one", observed: "2026-11-08T22:00:00Z")])
        XCTAssertEqual(sunday.timeline(now: monday, calendar: calendar).map(\.period), [.earlier])
    }

    func testAvailableUpdatesAndUsageChangesAreTodayWheneverTheyWereChecked() {
        // Just after midnight the last catalog check was yesterday. The
        // update is still what is true now, so it is not filed as old news.
        let night = date("2026-11-16T02:00:00Z")
        let feed = InsightsSkillsHome.Snapshot(
            updates: [update("late", catalogID: "one", checked: "2026-11-15T23:00:00Z"), update("later", catalogID: "two", checked: "2026-11-15T23:30:00Z")],
            trends: [trend("big", previous: 1, current: 9)],
            creators: [change("tonight", catalogID: "three", observed: "2026-11-16T01:00:00Z"),
                       change("yesterday", catalogID: "four", observed: "2026-11-15T23:45:00Z")])
        let groups = feed.timeline(now: night, calendar: calendar)
        XCTAssertEqual(groups.map(\.period), [.today, .earlier])
        XCTAssertEqual(ids(groups), [["creator:tonight", "update:late", "update:later", "trend:claude_code\u{0}big"], ["creator:yesterday"]])
    }

    func testACatalogChangeForASkillWithAnUpdateIsNotToldTwice() {
        let feed = InsightsSkillsHome.Snapshot(
            updates: [update("a", catalogID: "same", checked: "2026-11-15T11:00:00Z")], trends: [],
            creators: [change("same-news", catalogID: "same", observed: "2026-11-14T10:00:00Z"),
                       change("other", catalogID: "other", observed: "2026-11-14T09:00:00Z")])
        XCTAssertEqual(ids(feed.timeline(now: now, calendar: calendar)), [["update:a"], ["creator:other"]])
        XCTAssertEqual(feed.creators.count, 2, "The projection itself is unchanged")
    }

    func testTimelineFollowsTheLocalDayNotUTC() {
        var tokyo = Calendar(identifier: .gregorian)
        tokyo.timeZone = TimeZone(identifier: "Asia/Tokyo")!
        // 12:00Z is 21:00 in Tokyo; that local day began at 15:00Z the day before.
        let feed = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [
            change("local-today", catalogID: "one", observed: "2026-11-14T16:00:00Z"),
            change("local-yesterday", catalogID: "two", observed: "2026-11-14T14:00:00Z")])
        XCTAssertEqual(ids(feed.timeline(now: now, calendar: tokyo)), [["creator:local-today"], ["creator:local-yesterday"]])
        XCTAssertEqual(feed.timeline(now: now, calendar: tokyo).map(\.period), [.today, .thisWeek])
    }

    // MARK: Roll-ups, the hero, the chart and the week

    func testSeveralChangesFromOnePublisherInAPeriodAreOneRollup() throws {
        let feed = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [
            change("a", catalogID: "one", observed: "2026-11-14T10:00:00Z", from: "obra"),
            change("b", catalogID: "two", observed: "2026-11-13T10:00:00Z", from: "obra"),
            change("c", catalogID: "three", observed: "2026-11-14T12:00:00Z", from: "obra"),
            change("alone", catalogID: "four", observed: "2026-11-14T11:00:00Z", from: "other"),
            // The same publisher in another period is its own story.
            change("old", catalogID: "five", observed: "2026-11-02T10:00:00Z", from: "obra")])
        let groups = feed.timeline(now: now, calendar: calendar)
        XCTAssertEqual(ids(groups), [["rollup:thisWeek:obra", "creator:alone"], ["creator:old"]])
        guard case .rollup(let rollup) = try XCTUnwrap(groups.first?.stories.first) else { return XCTFail("Expected a roll-up") }
        XCTAssertEqual(rollup.changes.map(\.id), ["c", "a", "b"], "Newest first, at the newest change's place in the feed")
        XCTAssertEqual(rollup.publisher.id, "obra")
        // The grouping is by dictionary; the order must not depend on it.
        for _ in 0 ..< 20 { XCTAssertEqual(ids(feed.timeline(now: now, calendar: calendar)), ids(groups)) }
    }

    func testARollupCountsOnlyChangesNotAlreadyToldAsAnUpdate() {
        let feed = InsightsSkillsHome.Snapshot(
            updates: [update("mine", catalogID: "one", checked: "2026-11-15T11:00:00Z")], trends: [],
            creators: [change("a", catalogID: "one", observed: "2026-11-14T10:00:00Z", from: "obra"),
                       change("b", catalogID: "two", observed: "2026-11-14T09:00:00Z", from: "obra")])
        XCTAssertEqual(ids(feed.timeline(now: now, calendar: calendar)), [["update:mine"], ["creator:b"]], "One change left is a story, not a roll-up of one")
    }

    func testTheHeroIsTheFirstStoryTodayWhenItHasACover() {
        typealias Home = InsightsSkillsHome.Snapshot
        func hero(_ feed: Home) -> String? { Home.heroID(in: feed.timeline(now: now, calendar: calendar)) }
        let anUpdate = update("a", checked: "2026-11-15T11:00:00Z")
        XCTAssertEqual(hero(Home(updates: [anUpdate], trends: [trend("big", previous: 1, current: 9)], creators: [])), "update:a")
        XCTAssertEqual(hero(Home(updates: [anUpdate], trends: [], creators: [change("noon", catalogID: "one", observed: "2026-11-15T11:30:00Z")])),
                       "creator:noon", "The newest story today, whichever kind")
        XCTAssertNil(hero(Home(updates: [], trends: [trend("big", previous: 1, current: 9)], creators: [])), "A usage change has no cover")
        XCTAssertNil(hero(Home(updates: [], trends: [], creators: [change("old", catalogID: "one", observed: "2026-11-12T10:00:00Z")])),
                     "Nothing today: no story is promoted from an older period")
        XCTAssertNil(hero(Home(updates: [], trends: [], creators: [
            change("a", catalogID: "one", observed: "2026-11-15T10:00:00Z", from: "obra"),
            change("b", catalogID: "two", observed: "2026-11-15T09:00:00Z", from: "obra")])), "A roll-up is not drawn tall")
        XCTAssertNil(hero(Home(updates: [], trends: [], creators: [])))
        let week = [InsightsSkillsHome.WeekSkill(id: "claude_code\u{0}review", name: "review", agent: "claude_code", uses: 3)]
        XCTAssertNil(hero(Home(updates: [], trends: [], creators: [], week: week)))
    }

    func testAUsageStoryCarriesItsFourteenCompletedDays() throws {
        // row(): 2 uses on 11-03 and 5 on 11-10; the delegated row adds 1 and
        // 2 on the same days. 10-30 is before the fourteen days and today
        // (11-15) is not complete, so neither is drawn.
        let story = try XCTUnwrap(project(snapshot(rows: [try row(previous: 2, current: 5), try row("delegated", previous: 1, current: 2)])).trends.first)
        XCTAssertEqual(story.daily, [0, 0, 3, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0])
        XCTAssertEqual(story.daily.prefix(7).reduce(0, +), story.previousUses)
        XCTAssertEqual(story.daily.suffix(7).reduce(0, +), story.currentUses)
        XCTAssertEqual(story.bars.map(\.uses), story.daily)
        XCTAssertEqual(story.bars.map(\.recent), Array(repeating: false, count: 7) + Array(repeating: true, count: 7))
        XCTAssertEqual(story.bars.map(\.id), Array(0 ..< 14))
        // A story without daily history draws no chart, never a flat one.
        XCTAssertTrue(trend("bare", previous: 1, current: 2).bars.isEmpty)
        var short = trend("short", previous: 1, current: 2)
        short.daily = [1, 2, 3]
        XCTAssertTrue(short.bars.isEmpty)
    }

    func testYourWeekIsTheThreeMostUsedSkillsAndOpensThisWeek() throws {
        func used(_ name: String, _ uses: Int, agent: String = "claude_code") throws -> DaemonData.SkillsUsageRow {
            // Today's bucket (11-15) and one from eight days back are not in
            // the seven completed days; the rolling window counts both.
            let value: [String: Any] = ["name": name, "agent": agent, "population": "direct", "invocations": uses + 200, "loads": 0, "sessions": 1,
                                        "windows": [["days": 7, "invocations": uses + 100, "loads": 0, "sessions": 1]],
                                        "daily": [["day": "2026-11-07", "invocations": 100, "loads": 0, "sessions": 1],
                                                  ["day": "2026-11-08", "invocations": uses, "loads": 0, "sessions": 1],
                                                  ["day": "2026-11-15", "invocations": 100, "loads": 0, "sessions": 1]]]
            return try JSONDecoder().decode(DaemonData.SkillsUsageRow.self, from: JSONSerialization.data(withJSONObject: value))
        }
        let rows = [try used("a", 2), try used("b", 9), try used("c", 5), try used("d", 1), try used("idle", 0),
                    try used("uncounted", 50, agent: "codex")]
        let feed = project(snapshot(rows: rows))
        XCTAssertEqual(feed.week.map(\.name), ["b", "c", "a"], "Codex has no counted uses in this fixture's coverage")
        XCTAssertEqual(feed.week.map(\.uses), [9, 5, 2])
        XCTAssertFalse(feed.isEmpty)
        let groups = feed.timeline(now: now, calendar: calendar)
        XCTAssertEqual(ids(groups), [["week"]])
        XCTAssertEqual(groups.map(\.period), [.thisWeek])
        // It leads This week, above that week's other stories.
        let more = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [change("c", catalogID: "one", observed: "2026-11-13T10:00:00Z")], week: feed.week)
        XCTAssertEqual(ids(more.timeline(now: now, calendar: calendar)), [["week", "creator:c"]])
        // A partial history would pass a short count off as the week.
        XCTAssertTrue(project(snapshot(rows: rows, partial: true)).week.isEmpty)
        XCTAssertTrue(project(snapshot(rows: rows, analysis: false)).week.isEmpty)
        XCTAssertTrue(project(snapshot(rows: [try used("idle", 0)])).week.isEmpty)
        // The same seven days as a usage story's last week, so one skill
        // never shows two totals in one feed.
        let both = project(snapshot(rows: [try row(previous: 2, current: 5)]))
        XCTAssertEqual(both.week.map(\.uses), [try XCTUnwrap(both.trends.first).currentUses])
        XCTAssertEqual(both.week.first?.id, both.trends.first?.id)
    }

    // MARK: Recently active publishers

    /// A server-catalog snapshot with `count` publishers, each with `skills`
    /// catalog skills, all named by the daemon as recently active.
    private func serverSnapshot(publishers count: Int = 2, skills: Int = 2, source: String? = "server",
                                signals: (Int) -> DaemonData.SkillsRepositorySignals? = { .init(stars: 1_000 * ($0 + 1), commits7d: 5 + $0, commits28d: 20) },
                                named: [DaemonData.SkillsActivePublisher]? = nil) -> DaemonData.SkillsSnapshot {
        let publishers = (0 ..< count).map { DaemonData.SkillsPublisher(id: "gh.p\($0)", name: "owner\($0)/skills", repository: "owner\($0)/skills", skillsRoot: "", instructionsURL: "https://example.com", url: "https://github.com/owner\($0)/skills", sourceKind: "list") }
        let catalogSkills = publishers.flatMap { publisher in
            (0 ..< skills).map { DaemonData.CatalogSkill(id: "\(publisher.id):s\($0)/SKILL.md", publisherID: publisher.id, name: "s\($0)", description: "", repository: publisher.repository, path: "s\($0)/SKILL.md", revision: "main", sha256: String(repeating: "c", count: 64), sourceURL: "https://example.com/source", instructionsURL: "https://example.com") }
        }
        let states = publishers.enumerated().map { DaemonData.SkillsPublisherState(publisherID: $0.element.id, status: "current", checkedAt: "2026-11-15T11:00:00Z", signals: signals($0.offset)) }
        let active = named ?? publishers.map { publisher in .init(publisherID: publisher.id, skillIDs: catalogSkills.filter { $0.publisherID == publisher.id }.map(\.id)) }
        return .init(enabled: true, catalog: .init(publishers: publishers, skills: catalogSkills, publisherStates: states, source: source, activePublishers: active))
    }

    func testRecentlyActivePublishersAreDrawnOnlyFromAVerifiedServerCatalog() {
        let active = project(serverSnapshot()).active
        XCTAssertEqual(active.map(\.publisher.id), ["gh.p0", "gh.p1"], "The daemon's order is kept")
        XCTAssertEqual(active.map(\.stars), [1_000, 2_000])
        XCTAssertEqual(active.map(\.commits), [5, 6])
        XCTAssertEqual(active.first?.skills.map(\.id), ["gh.p0:s0/SKILL.md", "gh.p0:s1/SKILL.md"])
        // The four fixed publishers fetched from GitHub are not a verified
        // catalog, whatever the snapshot says beside them; nor is a daemon
        // that does not say where its catalog came from.
        XCTAssertTrue(project(serverSnapshot(source: "github")).active.isEmpty)
        XCTAssertTrue(project(serverSnapshot(source: nil)).active.isEmpty)
        XCTAssertTrue(project(serverSnapshot(source: "Server")).active.isEmpty)
        XCTAssertTrue(project(serverSnapshot(named: [])).active.isEmpty)
        XCTAssertTrue(project(serverSnapshot(named: [])).isEmpty, "With nothing to say, Home is its empty state and draws no card")
        XCTAssertFalse(project(serverSnapshot()).isEmpty)
    }

    func testAnActivePublisherNeedsItsSignalsAndASkillTheSnapshotCarries() {
        // Unknown is not zero: no stars or no commit count, no row.
        XCTAssertEqual(project(serverSnapshot { $0 == 0 ? nil : .init(stars: 50, commits7d: 2, commits28d: 2) }).active.map(\.publisher.id), ["gh.p1"])
        XCTAssertEqual(project(serverSnapshot { $0 == 0 ? .init(stars: nil, commits7d: 2, commits28d: 2) : .init(stars: 50, commits7d: 2, commits28d: 2) }).active.map(\.publisher.id), ["gh.p1"])
        XCTAssertEqual(project(serverSnapshot { $0 == 0 ? .init(stars: 50) : .init(stars: 50, commits7d: 2, commits28d: 2) }).active.map(\.publisher.id), ["gh.p1"])
        XCTAssertEqual(project(serverSnapshot { $0 == 0 ? .init(stars: 50, commits7d: 0, commits28d: 9) : .init(stars: 50, commits7d: 2, commits28d: 2) }).active.map(\.publisher.id), ["gh.p1"])
        XCTAssertTrue(project(serverSnapshot { _ in .init(stars: -1, commits7d: 2, commits28d: 2) }).active.isEmpty)
        // Ids that name nothing here, or another publisher's skill, are dropped;
        // a publisher left with no skill is not drawn.
        let named: [DaemonData.SkillsActivePublisher] = [
            .init(publisherID: "gh.p0", skillIDs: ["missing", "gh.p1:s0/SKILL.md", "gh.p0:s1/SKILL.md", "gh.p0:s1/SKILL.md"]),
            .init(publisherID: "gh.unknown", skillIDs: ["gh.p0:s0/SKILL.md"]),
            .init(publisherID: "gh.p1", skillIDs: ["missing"]),
            .init(publisherID: "gh.p0", skillIDs: ["gh.p0:s0/SKILL.md"])]
        let active = project(serverSnapshot(named: named)).active
        XCTAssertEqual(active.map(\.publisher.id), ["gh.p0"], "A publisher is drawn once")
        XCTAssertEqual(active.first?.skills.map(\.id), ["gh.p0:s1/SKILL.md"])
    }

    func testAtMostThreeActivePublishersWithThreeSkillsEach() {
        let active = project(serverSnapshot(publishers: 5, skills: 5)).active
        XCTAssertEqual(active.map(\.publisher.id), ["gh.p0", "gh.p1", "gh.p2"])
        XCTAssertEqual(active.map(\.skills.count), [3, 3, 3])
    }

    func testActivePublishersAreOneCardInThisWeekAfterTheRecap() throws {
        let active = project(serverSnapshot()).active
        let alone = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [], active: active)
        XCTAssertEqual(ids(alone.timeline(now: now, calendar: calendar)), [["active"]])
        XCTAssertEqual(alone.timeline(now: now, calendar: calendar).map(\.period), [.thisWeek])
        XCTAssertNil(InsightsSkillsHome.Snapshot.heroID(in: alone.timeline(now: now, calendar: calendar)))
        let week = [InsightsSkillsHome.WeekSkill(id: "claude_code\u{0}review", name: "review", agent: "claude_code", uses: 3)]
        let full = InsightsSkillsHome.Snapshot(updates: [], trends: [], creators: [change("c", catalogID: "one", observed: "2026-11-13T10:00:00Z")], week: week, active: active)
        XCTAssertEqual(ids(full.timeline(now: now, calendar: calendar)), [["week", "active", "creator:c"]])
        // The skills it names open exactly those catalog entries.
        let snapshot = serverSnapshot()
        for skill in active.flatMap(\.skills) {
            XCTAssertEqual(SkillsHomeRoute.resolve(.catalog(id: skill.id), in: snapshot)?.catalogID, skill.id)
        }
    }

    func testStarCountsAreShortAndNeverRoundedUp() {
        XCTAssertEqual(InsightsSkillsHome.Active.short(0), "0")
        XCTAssertEqual(InsightsSkillsHome.Active.short(999), "999")
        XCTAssertEqual(InsightsSkillsHome.Active.short(1_000), "1K")
        XCTAssertEqual(InsightsSkillsHome.Active.short(1_949), "1.9K")
        XCTAssertEqual(InsightsSkillsHome.Active.short(12_400), "12K")
        XCTAssertEqual(InsightsSkillsHome.Active.short(99_999), "99K")
        XCTAssertEqual(InsightsSkillsHome.Active.short(1_250_000), "1.2M")
    }

    func testReduceMotionTurnsEveryMovementOff() {
        let still = SkillsHomeMotion(reduce: true)
        XCTAssertEqual(still, .still)
        XCTAssertFalse(still.growsBars)
        XCTAssertFalse(still.lifts)
        XCTAssertFalse(still.scrolls)
        XCTAssertFalse(still.rollsNumbers)
        XCTAssertNil(still.expand)
        XCTAssertNil(still.hover)
        XCTAssertNil(still.grow)
        let moving = SkillsHomeMotion(reduce: false)
        XCTAssertTrue(moving.growsBars && moving.lifts && moving.scrolls && moving.rollsNumbers)
        XCTAssertNotNil(moving.expand)
        XCTAssertNotNil(moving.hover)
        XCTAssertNotNil(moving.grow)
    }

    // MARK: Partial history, by agent and day

    /// row(): uses on 11-03 and 11-10, so it is both a usage story and the
    /// week, today being 11-15. Partial history comes from the daemon.
    private func stories(_ gaps: [DaemonData.SkillsPartialAgent]?, partial: Bool = true) throws -> (week: Bool, trend: Bool) {
        let feed = project(snapshot(rows: [try row()], partial: partial, partialAgents: gaps))
        return (!feed.week.isEmpty, !feed.trends.isEmpty)
    }

    func testAnotherAgentsGapDoesNotHoldBackClaudeCodeStories() throws {
        let only = try stories([.init(agent: "codex", days: ["2026-11-10"], undated: 3), .init(agent: "opencode", days: [], undated: 1)])
        XCTAssertTrue(only.week)
        XCTAssertTrue(only.trend)
        let none = try stories([])
        XCTAssertTrue(none.week)
        XCTAssertTrue(none.trend)
    }

    func testAClaudeCodeGapInsideAStorysDaysHoldsItBack() throws {
        // Inside both: the last seven completed days.
        let recent = try stories([.init(agent: "claude_code", days: ["2026-11-10"], undated: 0)])
        XCTAssertFalse(recent.week)
        XCTAssertFalse(recent.trend)
        // Inside the fourteen a usage story compares, before the week.
        let earlier = try stories([.init(agent: "claude_code", days: ["2026-11-02"], undated: 0)])
        XCTAssertTrue(earlier.week)
        XCTAssertFalse(earlier.trend)
        // A session that cannot be dated is inside every story's days.
        let undated = try stories([.init(agent: "claude_code", days: [], undated: 1)])
        XCTAssertFalse(undated.week)
        XCTAssertFalse(undated.trend)
        // A day that cannot be read is not taken to be outside.
        let unreadable = try stories([.init(agent: "claude_code", days: ["11/10/2026"], undated: 0)])
        XCTAssertFalse(unreadable.week)
        XCTAssertFalse(unreadable.trend)
        // An agent is named only for a gap: named with no place, it is
        // taken to be everywhere.
        let unplaced = try stories([.init(agent: "claude_code", days: [], undated: 0)])
        XCTAssertFalse(unplaced.week)
        XCTAssertFalse(unplaced.trend)
    }

    func testAClaudeCodeGapOutsideAStorysDaysLeavesItAlone() throws {
        // Older than both windows, and today, which neither counts.
        let outside = try stories([.init(agent: "claude_code", days: ["2026-10-20", "2026-11-15"], undated: 0)])
        XCTAssertTrue(outside.week)
        XCTAssertTrue(outside.trend)
    }

    func testWithoutPerAgentGapsAnyPartialHistoryHoldsStoriesBack() throws {
        let older = try stories(nil)
        XCTAssertFalse(older.week)
        XCTAssertFalse(older.trend)
        let complete = try stories(nil, partial: false)
        XCTAssertTrue(complete.week)
        XCTAssertTrue(complete.trend)
    }

    func testPerAgentGapsDecodeAndAreOptional() throws {
        let with = try JSONDecoder().decode(DaemonData.SkillsUsage.self, from: Data(#"{"partial":true,"partial_agents":[{"agent":"codex","days":["2026-11-10"],"undated":2}]}"#.utf8))
        XCTAssertEqual(with.partialAgents, [.init(agent: "codex", days: ["2026-11-10"], undated: 2)])
        let without = try JSONDecoder().decode(DaemonData.SkillsUsage.self, from: Data(#"{"partial":true}"#.utf8))
        XCTAssertNil(without.partialAgents)
        let sparse = try JSONDecoder().decode(DaemonData.SkillsUsage.self, from: Data(#"{"partial_agents":[{"agent":"codex"}]}"#.utf8))
        XCTAssertEqual(sparse.partialAgents, [.init(agent: "codex", days: [], undated: 0)])
        let again = try JSONDecoder().decode(DaemonData.SkillsUsage.self, from: JSONEncoder().encode(with))
        XCTAssertEqual(again, with)
    }

    func testYesterdayAnalysisCannotTreatTheUnfinishedDayAsComplete() throws {
        let original = snapshot(rows:[try row()])
        var raw = try JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as! [String:Any]
        var usage = raw["usage"] as! [String:Any]
        usage["last_analysis_at"] = "2026-11-14T13:00:00Z"
        raw["usage"] = usage
        let old = try JSONDecoder().decode(DaemonData.SkillsSnapshot.self, from: JSONSerialization.data(withJSONObject:raw))
        XCTAssertTrue(project(old).trends.isEmpty)
    }

}
