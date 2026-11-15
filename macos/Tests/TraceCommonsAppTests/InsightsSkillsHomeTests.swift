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
    private func snapshot(rows: [DaemonData.SkillsUsageRow] = [], partial: Bool = false, followed: [String] = ["maker"], feed: [DaemonData.SkillsFeedEvent] = [], updates: [DaemonData.SkillsUpdate] = [], skills: [DaemonData.InstalledSkill]? = nil, status: String = "current", enabled: Bool = true, analysis: Bool = true) -> DaemonData.SkillsSnapshot {
        .init(enabled: enabled, inventory: .init(skills: skills ?? [installed()], partial: false),
              catalog: .init(publishers: [publisher], skills: [catalog], checkedAt: "2026-11-15T11:00:00Z", publisherStates: [.init(publisherID:"maker",status:status,checkedAt:"2026-11-15T11:00:00Z")]),
              followedPublishers: followed, updates: updates, feed: feed,
              usage: .init(enabled: analysis, lastAnalysisAt:"2026-11-15T11:00:00Z",earliestEvidence:"2026-10-30T00:00:00Z",partial:partial,skills:rows,coverage:[.init(agent:"claude_code",status:"explicit_invocations")]))
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
    private func change(_ id: String, catalogID: String, observed: String) -> InsightsSkillsHome.CreatorUpdate {
        let skill = catalogSkill(catalogID)
        return .init(id: id, event: .init(id: id, publisherID: "maker", skillID: catalogID, kind: "updated", observedAt: observed),
                     skill: skill, publisher: publisher, observedAt: date(observed))
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
