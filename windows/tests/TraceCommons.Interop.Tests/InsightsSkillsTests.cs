using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Windows Skills tab against the Mac's own tests
/// (macos/Tests/TraceCommonsAppTests/InsightsSkills*Tests.swift and
/// macos/Tests/TCShellCoreTests/SkillsWireTests.swift): the same fixtures and
/// the same expected values, so the two shells decide the same things.
/// </summary>
public class InsightsSkillsTests
{
    private static readonly TimeZoneInfo Utc = TimeZoneInfo.Utc;
    private static readonly DateTimeOffset Now = DateTimeOffset.Parse("2026-11-15T12:00:00Z");
    private static readonly string ContentHash = new('a', 64);

    private static SkillsSnapshot Parse(string json) =>
        SkillsSnapshot.Parse(JsonDocument.Parse(json).RootElement) ?? throw new InvalidOperationException("undecodable");

    // MARK: Wire (SkillsWireTests.swift)

    [Fact]
    public void SnapshotDecodesIdentityAndUsageWithoutInferringFromNames()
    {
        const string json = """
        {"enabled":true,"inventory":{"skills":[{"id":"local","name":"review","description":"Local","agent":"codex","scope":"user","location":"/skills/review","sha256":"abc","source":{"repository":"owner/repo","path":"skills/review","revision":"main"}}],"partial":true,"coverage":[{"location":"/skills","status":"read"}]},"catalog":{"publishers":[{"id":"owner","name":"Owner","repository":"owner/repo","skills_root":"skills","instructions_url":"https://example.com/docs"}],"skills":[{"id":"upstream","publisher_id":"owner","name":"review","description":"Remote","repository":"owner/repo","path":"skills/review","revision":"main","sha256":"def","source_url":"https://example.com/source","instructions_url":"https://example.com/docs"}],"checked_at":"today"},"followed_publishers":["owner"],"updates":[{"installed_id":"local","status":"changed","catalog_skill_id":"upstream"}],"feed":[{"id":"event","publisher_id":"owner","skill_id":"upstream","kind":"changed","observed_at":"today"}],"usage":{"sessions_observed":3,"partial":true,"skills":[{"name":"review","agent":"codex","invocations":2,"loads":1,"sessions":2,"last_seen":"today","session_refs":["session-hash"]}]},"copy":{"skills_title":"Skills"},"future":true}
        """;
        SkillsSnapshot snapshot = Parse(json);
        Assert.Equal("owner/repo", snapshot.Inventory.Skills[0].Source?.Repository);
        Assert.Equal("owner", snapshot.Catalog.Skills[0].PublisherId);
        Assert.Equal("upstream", snapshot.Updates[0].CatalogSkillId);
        Assert.Equal(new[] { "session-hash" }, snapshot.Usage.Skills[0].SessionRefs);
        Assert.True(snapshot.Usage.Partial);
        Assert.Equal(new[] { "owner" }, snapshot.FollowedPublishers);
        Assert.Equal("Skills", snapshot.Copy["skills_title"]);
    }

    [Fact]
    public void DelegatedEvidenceKeepsPopulationAndParentWithoutAddingTotals()
    {
        SkillsSnapshot snapshot = Parse("""
        {"enabled":true,"usage":{"sessions_observed":2,"delegated_sessions_observed":3,"delegated_sessions_unknown":1,"unclassified_sessions_observed":4,"unclassified_sessions_unknown":2,"skills":[
          {"name":"review","agent":"opencode","population":"direct","invocations":1,"loads":0,"sessions":1},
          {"name":"review","agent":"opencode","population":"delegated","invocations":2,"loads":0,"sessions":1,"parent_session_refs":["parent"],"evidence":[{"session_ref":"child","parent_session_ref":"parent"}]}]}}
        """);
        Assert.Equal((2, 3, 1, 4, 2), (snapshot.Usage.SessionsObserved, snapshot.Usage.DelegatedSessionsObserved,
            snapshot.Usage.DelegatedSessionsUnknown, snapshot.Usage.UnclassifiedSessionsObserved, snapshot.Usage.UnclassifiedSessionsUnknown));
        Assert.NotEqual(snapshot.Usage.Skills[0].Identity, snapshot.Usage.Skills[1].Identity);
        Assert.Equal("parent", snapshot.Usage.Skills[1].Evidence[0].ParentSessionRef);
        Assert.Equal("child", snapshot.Usage.Skills[1].Evidence[0].SessionRef);
    }

    [Fact]
    public void OlderUsageDoesNotClaimDelegatedCoverage()
    {
        SkillsSnapshot snapshot = Parse("""{"usage":{"skills":[{"name":"review","agent":"codex","invocations":0,"loads":1,"sessions":1}]}}""");
        Assert.Null(snapshot.Usage.DelegatedSessionsObserved);
        Assert.Equal("direct", snapshot.Usage.Skills[0].Population);
        Assert.Empty(snapshot.Usage.Skills[0].Evidence);
        Assert.False(snapshot.Enabled);
        Assert.True(snapshot.Readable);
    }

    [Fact]
    public void AMissingRequiredFieldIsUndecodableAndAMalformedFactIsUnknown()
    {
        // A catalog skill without its repository fails the whole answer, as
        // Swift's `decode` does; a malformed licence or signal is unknown.
        Assert.Null(SkillsSnapshot.Parse(JsonDocument.Parse(
            """{"catalog":{"skills":[{"id":"a","publisher_id":"p","name":"n","description":"","path":"x","revision":"r","sha256":"s","source_url":"u","instructions_url":"i"}]}}""").RootElement));
        SkillsSnapshot snapshot = Parse("""
        {"catalog":{"skills":[{"id":"a","publisher_id":"p","name":"n","description":"","repository":"o/r","path":"x","revision":"r","sha256":"s","source_url":"u","instructions_url":"i","license":7,"checks":{"frontmatter":1}}],
          "publisher_states":[{"publisher_id":"p","status":"current","signals":"busy"}],"active_publishers":"none"},"suggestions":[]}
        """);
        Assert.Null(snapshot.Catalog.Skills[0].License);
        Assert.Null(snapshot.Catalog.Skills[0].Checks);
        Assert.Null(snapshot.Catalog.PublisherStates[0].Signals);
        Assert.Empty(snapshot.Catalog.ActivePublishers);
        Assert.False(snapshot.Suggestions.Available);
    }

    [Fact]
    public async Task EvidenceUsesTheSkillsReadForAnOsReferenceAndTheDrillOtherwise()
    {
        var calls = new List<(string Method, string Params)>();
        Task<DaemonResponse> Call(string method, string parameters)
        {
            calls.Add((method, parameters));
            return Task.FromResult(DaemonResponse.Parse("""{"id":0,"result":{"enabled":true,"readable":true,"session":{"source":"claude_code"}}}"""));
        }

        foreach (string reference in new[] { "c42.storetag", "os.abc" })
        {
            SkillEvidence evidence = await InsightsSkillsModel.EvidenceAsync(Call, reference);
            Assert.Equal("claude_code", evidence.Session?.Source);
        }

        Assert.Equal(new[] { "insights_session_drill", "insights_skills" }, calls.Select(c => c.Method));
        JsonElement drill = JsonDocument.Parse(calls[0].Params).RootElement;
        Assert.Equal("c42.storetag", drill.GetProperty("session_ref").GetString());
        Assert.True(drill.TryGetProperty("tz", out _));
        Assert.False(JsonDocument.Parse(calls[1].Params).RootElement.TryGetProperty("tz", out _));
    }

    // MARK: The tab and the flag (InsightsSkillsFlagTests.swift)

    [Fact]
    public void TheSkillsTabFollowsTheFlagInTheMacsPlace()
    {
        Assert.Equal(new[] { InsightsViewModel.OverviewTab, InsightsViewModel.PatternsTab, InsightsViewModel.BenchmarksTab,
            InsightsViewModel.SessionsTab, InsightsViewModel.AnalyzeTab }, InsightsViewModel.ShownTabs(false));
        Assert.Equal(new[] { InsightsViewModel.OverviewTab, InsightsViewModel.PatternsTab, InsightsViewModel.BenchmarksTab,
            InsightsViewModel.SkillsTab, InsightsViewModel.SessionsTab, InsightsViewModel.AnalyzeTab }, InsightsViewModel.ShownTabs(true));
    }

    private sealed class CountingReader
    {
        private int _count;
        public int Count => Volatile.Read(ref _count);

        public Task<SkillsSnapshot> Read(bool refresh)
        {
            Interlocked.Increment(ref _count);
            return Task.FromResult(SkillsSnapshot.Empty(enabled: true));
        }
    }

    private static InsightsViewModel ViewModel(CountingReader reader) =>
        new(new SampleInsights(), skills: copy => new InsightsSkillsModel(reader.Read, copy: copy, idlePollInterval: TimeSpan.FromMilliseconds(5)));

    [Fact]
    public async Task AMissingOrOffFlagNeverStartsSkillsReadsEvenWhenSkillsIsAsked()
    {
        var reader = new CountingReader();
        using InsightsViewModel model = ViewModel(reader);
        await model.SelectTabAsync(InsightsViewModel.SkillsTab);
        await Task.Delay(60);
        Assert.Equal(0, reader.Count);
        Assert.Equal(InsightsViewModel.OverviewTab, model.ShownTab);
        Assert.Null(model.Skills.Shown);
    }

    [Fact]
    public async Task TurningTheFlagOffStopsReadsAndMovesTheSelectionToOverview()
    {
        var reader = new CountingReader();
        using InsightsViewModel model = ViewModel(reader);
        await model.SetSkillsEnabled(true);
        await model.SelectTabAsync(InsightsViewModel.SkillsTab);
        for (int i = 0; i < 200 && reader.Count == 0; i++) await Task.Delay(5);
        Assert.True(reader.Count > 0, "The selected Skills tab reads once its flag is on");

        await model.SetSkillsEnabled(false);
        int stopped = reader.Count;
        await Task.Delay(60);
        Assert.Equal(stopped, reader.Count);
        Assert.Equal(InsightsViewModel.OverviewTab, model.ShownTab);

        // Turned on again without choosing Skills, the old selection does not reopen it.
        await model.SetSkillsEnabled(true);
        await Task.Delay(60);
        Assert.Equal(stopped, reader.Count);
        Assert.Equal(InsightsViewModel.OverviewTab, model.ShownTab);
    }

    [Fact]
    public async Task StoppedClocksStopSkillsReadsAndRunningClocksResumeThem()
    {
        var reader = new CountingReader();
        using InsightsViewModel model = ViewModel(reader);
        await model.SetSkillsEnabled(true);
        model.SetClocksRunning(false);
        await model.SelectTabAsync(InsightsViewModel.SkillsTab);
        await Task.Delay(60);
        Assert.Equal(0, reader.Count);
        model.SetClocksRunning(true);
        for (int i = 0; i < 200 && reader.Count == 0; i++) await Task.Delay(5);
        Assert.True(reader.Count > 0);
        model.SetClocksRunning(false);
        int stopped = reader.Count;
        await Task.Delay(60);
        Assert.Equal(stopped, reader.Count);
    }

    // MARK: The model (InsightsSkillsFollowTests.swift, InsightsSkillsRecoveryTests.swift)

    private static SkillsSnapshot Followed(params string[] publishers) =>
        SkillsSnapshot.Empty(enabled: true) with { FollowedPublishers = publishers };

    [Fact]
    public async Task AnUnconfirmedReplyIsAFailureEvenWithoutADaemonError()
    {
        var model = new InsightsSkillsModel(_ => Task.FromResult(Followed()), (_, _) => Task.FromResult(Followed()));
        await model.LoadAsync();
        await model.SetFollowAsync("new", true);
        Assert.Empty(model.Shown!.FollowedPublishers);
        Assert.Equal("new", model.FollowFailurePublisherId);
        Assert.Equal("skills_follow_failed", model.FailureKey);
    }

    [Fact]
    public async Task ATransportFailureKeepsTheConfirmedFollowAndNamesTheCreator()
    {
        var model = new InsightsSkillsModel(_ => Task.FromResult(Followed("old")),
            (_, _) => Task.FromException<SkillsSnapshot>(new InvalidOperationException("offline")));
        await model.LoadAsync();
        await model.SetFollowAsync("old", false);
        Assert.Equal(new[] { "old" }, model.Shown!.FollowedPublishers);
        Assert.Equal("old", model.FollowFailurePublisherId);
        Assert.Equal("skills_follow_failed", model.FailureKey);
        Assert.Null(model.PendingFollowPublisherId);
        Assert.True(model.CanFollow);
    }

    [Fact]
    public async Task AConfirmedFollowStandsDespiteAnUnrelatedCatalogRefreshError()
    {
        var model = new InsightsSkillsModel(_ => Task.FromResult(Followed()),
            (_, _) => Task.FromResult(Followed("new") with { Error = "insights-skills-refresh-failed" }));
        await model.LoadAsync();
        await model.SetFollowAsync("new", true);
        Assert.Equal(new[] { "new" }, model.Shown!.FollowedPublishers);
        Assert.Null(model.FollowFailurePublisherId);
        Assert.Equal("skills_refresh_failed", model.FailureKey);
    }

    [Fact]
    public async Task PendingBelongsToOneCreatorAndTheLaneRefusesASecondWrite()
    {
        var reply = new TaskCompletionSource<SkillsSnapshot>();
        var writes = new List<string>();
        var model = new InsightsSkillsModel(_ => Task.FromResult(Followed("old")), (publisher, _) =>
        {
            writes.Add(publisher);
            return reply.Task;
        });
        await model.LoadAsync();
        Task first = model.SetFollowAsync("new", true);
        Assert.Equal("new", model.PendingFollowPublisherId);
        Assert.False(model.CanFollow);
        Assert.Equal(new[] { "old" }, model.Shown!.FollowedPublishers);
        await model.SetFollowAsync("other", true);
        await model.LoadAsync(refresh: true);
        reply.SetResult(Followed("old", "new"));
        await first;
        Assert.Equal(new[] { "old", "new" }, model.Shown.FollowedPublishers);
        Assert.Equal(new[] { "new" }, writes);
        Assert.Equal(1, model.ReadCount);
    }

    [Fact]
    public async Task AnUnreadableAnswerHidesTheSnapshotAndAFailedReadKeepsIt()
    {
        int calls = 0;
        var model = new InsightsSkillsModel(_ => ++calls switch
        {
            1 => Task.FromResult(SkillsSnapshot.Empty(enabled: true)),
            2 => Task.FromException<SkillsSnapshot>(new InvalidOperationException()),
            _ => Task.FromResult(SkillsSnapshot.Empty(enabled: true) with { Readable = false }),
        });
        await model.LoadAsync();
        Assert.NotNull(model.Shown);
        await model.LoadAsync();
        Assert.NotNull(model.Shown);
        Assert.Equal("skills_refresh_failed", model.FailureKey);
        await model.LoadAsync();
        Assert.Null(model.Shown);
        Assert.Equal("skills_unavailable", model.FailureKey);
    }

    [Fact]
    public async Task ADefinitionIsReadOncePerIdUntilTheDetailsAreDropped()
    {
        var reply = new TaskCompletionSource<SkillDefinition>();
        var asked = new List<string>();
        var model = new InsightsSkillsModel(definitionReader: id =>
        {
            asked.Add(id);
            return reply.Task;
        });
        // A view asks on every redraw while the read is in flight: one read goes out.
        Task[] reads = { model.LoadDefinitionAsync("a"), model.LoadDefinitionAsync("a"), model.LoadDefinitionAsync("a") };
        Assert.Equal(new[] { "a" }, asked);
        reply.SetResult(new SkillDefinition("a", "available", "body"));
        await Task.WhenAll(reads);
        await model.LoadDefinitionAsync("a");
        Assert.Equal(new[] { "a" }, asked);
        Assert.Equal("body", model.Definition?.Definition);
        Assert.False(model.DefinitionFailure);

        // Dropped details (a new snapshot, another section) read it again.
        model.InvalidateDetails();
        await model.LoadDefinitionAsync("a");
        Assert.Equal(new[] { "a", "a" }, asked);
    }

    [Fact]
    public void ThePollsAreTheMacs()
    {
        Assert.Equal(TimeSpan.FromMinutes(5), InsightsSkillsModel.IdlePollInterval);
        Assert.Equal(TimeSpan.FromSeconds(2), InsightsSkillsModel.RefreshingPollInterval);
        Assert.Equal(new[] { 2.0, 4, 8, 16, 30, 30 }, Enumerable.Range(0, 6).Select(a => InsightsSkillsModel.RetryDelay(a).TotalSeconds));
        Assert.Equal(2.0, InsightsSkillsModel.RetryDelay(-3).TotalSeconds);
    }

    // MARK: Index (InsightsSkillsIndexTests.swift)

    private static InstalledSkill Installed(string id, string agent = "claude_code", string? hash = null, SkillsSource? source = null, string name = "review") =>
        new(id, name, "", agent, "global", "/skills/" + id, hash ?? ContentHash, source ?? new SkillsSource("maker/skills", "skills/review/SKILL.md", null));

    [Fact]
    public void ContentIdentityGroupsCopiesButNeverConflictingOriginsOrBadHashes()
    {
        var groups = InsightsSkillsIndex.Groups(new[]
        {
            Installed("b", "opencode"), Installed("a"),
            Installed("c", hash: "short"), Installed("d", hash: "short"),
            Installed("e", hash: new string('c', 64), source: new SkillsSource("one/x", "p", null)),
            Installed("f", hash: new string('c', 64), source: new SkillsSource("two/x", "p", null)),
        });
        Assert.Equal(new[] { "a", "c", "d", "e", "f" }, groups.Select(g => g.Id));
        Assert.Equal(new[] { "claude_code", "opencode" }, groups[0].Agents);
        var ordered = InsightsSkillsIndex.Groups(new[] { Installed("a"), Installed("x", hash: "1") }, new[] { "x", "a" });
        Assert.Equal(new[] { "x", "a" }, ordered.Select(g => g.Id));
    }

    // MARK: Home (InsightsSkillsHomeTests.swift)

    private static JsonObject RowJson(string population = "direct", int previous = 2, int current = 5, string name = "review") => new()
    {
        ["name"] = name, ["agent"] = "claude_code", ["population"] = population, ["invocations"] = previous + current,
        ["loads"] = 0, ["sessions"] = 1,
        ["windows"] = new JsonArray(new JsonObject { ["days"] = 30, ["invocations"] = previous + current, ["loads"] = 0, ["sessions"] = 1 }),
        ["daily"] = new JsonArray(
            new JsonObject { ["day"] = "2026-10-30", ["invocations"] = 1, ["loads"] = 0, ["sessions"] = 1 },
            new JsonObject { ["day"] = "2026-11-03", ["invocations"] = previous, ["loads"] = 0, ["sessions"] = 1 },
            new JsonObject { ["day"] = "2026-11-10", ["invocations"] = current, ["loads"] = 0, ["sessions"] = 1 }),
    };

    private static SkillsSnapshot HomeSnapshot(IEnumerable<JsonObject>? rows = null, bool partial = false, string[]? followed = null,
        JsonArray? feed = null, JsonArray? updates = null, string status = "current", bool enabled = true, bool analysis = true,
        string checkedAt = "2026-11-15T11:00:00Z")
    {
        var root = new JsonObject
        {
            ["enabled"] = enabled,
            ["inventory"] = new JsonObject
            {
                ["partial"] = false,
                ["skills"] = new JsonArray(new JsonObject
                {
                    ["id"] = "local", ["name"] = "review", ["description"] = "", ["agent"] = "claude_code", ["scope"] = "global",
                    ["location"] = "/skills/local", ["sha256"] = ContentHash,
                    ["source"] = new JsonObject { ["repository"] = "maker/skills", ["path"] = "skills/review/SKILL.md" },
                }),
            },
            ["catalog"] = new JsonObject
            {
                ["publishers"] = new JsonArray(new JsonObject
                {
                    ["id"] = "maker", ["name"] = "Maker", ["repository"] = "maker/skills", ["skills_root"] = "skills", ["instructions_url"] = "https://example.com",
                }),
                ["skills"] = new JsonArray(new JsonObject
                {
                    ["id"] = "upstream", ["publisher_id"] = "maker", ["name"] = "review", ["description"] = "", ["repository"] = "maker/skills",
                    ["path"] = "skills/review/SKILL.md", ["revision"] = "main", ["sha256"] = new string('b', 64),
                    ["source_url"] = "https://example.com/source", ["instructions_url"] = "https://example.com",
                }),
                ["checked_at"] = "2026-11-15T11:00:00Z",
                ["publisher_states"] = new JsonArray(new JsonObject { ["publisher_id"] = "maker", ["status"] = status, ["checked_at"] = checkedAt }),
            },
            ["followed_publishers"] = new JsonArray((followed ?? new[] { "maker" }).Select(p => (JsonNode)JsonValue.Create(p)!).ToArray()),
            ["updates"] = updates ?? new JsonArray(),
            ["feed"] = feed ?? new JsonArray(),
            ["usage"] = new JsonObject
            {
                ["enabled"] = analysis, ["last_analysis_at"] = "2026-11-15T11:00:00Z", ["earliest_evidence"] = "2026-10-30T00:00:00Z",
                ["partial"] = partial, ["skills"] = new JsonArray((rows ?? Array.Empty<JsonObject>()).Select(r => (JsonNode)r).ToArray()),
                ["coverage"] = new JsonArray(new JsonObject { ["agent"] = "claude_code", ["status"] = "explicit_invocations" }),
            },
        };
        return Parse(root.ToJsonString());
    }

    private static InsightsSkillsHome.Snapshot Project(SkillsSnapshot snapshot) =>
        InsightsSkillsHome.Make(snapshot, new InsightsSkillsModel(), Now, Utc);

    [Fact]
    public void RankingContextUsesCatalogSourceIdentityAndExplicitUsage()
    {
        InsightsSkillsHome.Snapshot home = Project(HomeSnapshot(new[] { RowJson() }));
        Assert.Equal(new[] { "upstream" }, home.Context.InstalledSkillIds);
        Assert.Equal(new[] { "upstream" }, home.Context.UsedSkillIds);
        Assert.Equal(new[] { "maker" }, home.Context.FollowedPublisherIds);
        Assert.Equal(new[] { "upstream" }, home.Context.CatalogIdsByUsageId["claude_code\0review"]);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson() }, analysis: false)).Context.UsedSkillIds);
    }

    [Fact]
    public void TrendsCompareCompletedWeeksAndCombineDisjointPopulations()
    {
        var trends = Project(HomeSnapshot(new[] { RowJson(previous: 2, current: 5), RowJson("delegated", 1, 2) })).Trends;
        Assert.Single(trends);
        Assert.Equal((3, 7, InsightsSkillsHome.Direction.More), (trends[0].PreviousUses, trends[0].CurrentUses, trends[0].Direction));
        Assert.Equal(InsightsSkillsHome.Direction.Less, Project(HomeSnapshot(new[] { RowJson(previous: 4, current: 1) })).Trends[0].Direction);
        Assert.Equal(InsightsSkillsHome.Direction.Started, Project(HomeSnapshot(new[] { RowJson(previous: 0, current: 1) })).Trends[0].Direction);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson(previous: 2, current: 2) })).Trends);
        // Fourteen bars, the last seven the recent ones.
        IReadOnlyList<InsightsSkillsHome.TrendBar> bars = trends[0].Bars;
        Assert.Equal(14, bars.Count);
        Assert.Equal(7, bars.Count(b => b.Recent));
        Assert.Equal(3, bars[2].Uses);
        Assert.Equal(7, bars[9].Uses);
    }

    [Fact]
    public void PartialHistoryAndUnknownPopulationsTellNoTrend()
    {
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson() }, partial: true)).Trends);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson(), RowJson("unknown") })).Trends);
        foreach (string badDay in new[] { "2026-11-16", "2026-02-30", "invalid" })
        {
            JsonObject row = RowJson();
            ((JsonArray)row["daily"]!).Add(new JsonObject { ["day"] = badDay, ["invocations"] = 100, ["loads"] = 0, ["sessions"] = 1 });
            Assert.Empty(Project(HomeSnapshot(new[] { row })).Trends);
        }

        JsonObject today = RowJson();
        ((JsonArray)today["daily"]!).Add(new JsonObject { ["day"] = "2026-11-15", ["invocations"] = 100, ["loads"] = 0, ["sessions"] = 1 });
        Assert.Equal(5, Project(HomeSnapshot(new[] { today })).Trends[0].CurrentUses);
    }

    [Fact]
    public void UsedUpdatesNeedAVerifiedSourceAndACurrentPublisher()
    {
        JsonArray Update(string status) => new(new JsonObject { ["installed_id"] = "local", ["status"] = status, ["catalog_skill_id"] = "upstream" });
        Assert.Equal(new[] { "local" }, Project(HomeSnapshot(new[] { RowJson() }, updates: Update("update_available"))).Updates.Select(u => u.InstalledId));
        Assert.Empty(Project(HomeSnapshot(updates: Update("update_available"))).Updates);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson() }, updates: Update("update_available"), status: "unknown")).Updates);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson() }, updates: Update("locally_modified"))).Updates);
    }

    [Fact]
    public void AnUpdateComesFromACheckLessThanSixHoursOld()
    {
        // now - checkedAt < 6 h (InsightsSkillsHome.swift:321), and never ahead of now.
        JsonArray Update() => new(new JsonObject { ["installed_id"] = "local", ["status"] = "update_available", ["catalog_skill_id"] = "upstream" });
        Assert.Single(Project(HomeSnapshot(new[] { RowJson() }, updates: Update(), checkedAt: "2026-11-15T06:00:01Z")).Updates);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson() }, updates: Update(), checkedAt: "2026-11-15T06:00:00Z")).Updates);
        Assert.Empty(Project(HomeSnapshot(new[] { RowJson() }, updates: Update(), checkedAt: "2026-11-15T12:00:01Z")).Updates);
    }

    [Fact]
    public void CreatorChangesNeedAFollowAKnownTargetAndARecentValidDate()
    {
        JsonObject Event(string id, string skill, string kind, string observed) =>
            new() { ["id"] = id, ["publisher_id"] = "maker", ["skill_id"] = skill, ["kind"] = kind, ["observed_at"] = observed };
        JsonArray events() => new(Event("good", "upstream", "updated", "2026-11-14T10:00:00Z"),
            Event("future", "upstream", "added", "2026-11-16T10:00:00Z"), Event("bad", "upstream", "updated", "invalid"),
            Event("unknown", "missing", "updated", "2026-11-14T10:00:00Z"));
        Assert.Equal(new[] { "good" }, Project(HomeSnapshot(feed: events())).Creators.Select(c => c.Id));
        Assert.Empty(Project(HomeSnapshot(followed: Array.Empty<string>(), feed: events())).Creators);
        Assert.Single(Project(HomeSnapshot(feed: events(), analysis: false)).Creators);
        Assert.True(Project(HomeSnapshot(feed: events(), enabled: false)).IsEmpty);
        JsonArray newest = new(Event("old", "upstream", "added", "2026-09-14T10:00:00Z"),
            Event("earlier", "upstream", "updated", "2026-11-13T10:00:00Z"), Event("latest", "upstream", "updated", "2026-11-14T10:00:00Z"));
        Assert.Equal(new[] { "latest" }, Project(HomeSnapshot(feed: newest)).Creators.Select(c => c.Id));
    }

    private static CatalogSkill CatalogSkill(string id) => new(id, "maker", "review", "", "maker/skills", "skills/review/SKILL.md", "main",
        new string('b', 64), "https://example.com/source", "https://example.com");

    private static InsightsSkillsHome.Update Update(string id, string catalogId = "upstream", string checkedAt = "2026-11-15T11:00:00Z") =>
        new("update:" + id, id, id, id, new[] { "claude_code" }, CatalogSkill(catalogId), DateTimeOffset.Parse(checkedAt));

    private static InsightsSkillsHome.Trend Trend(string name, int previous, int current) =>
        new("claude_code\0" + name, name, "claude_code", previous, current,
            current > previous ? InsightsSkillsHome.Direction.More : InsightsSkillsHome.Direction.Less,
            DateTimeOffset.Parse("2026-11-15T00:00:00Z"), Array.Empty<int>());

    private static InsightsSkillsHome.CreatorUpdate Change(string id, string catalogId, string observed, string? publisherId = null)
    {
        var maker = new SkillsPublisher(publisherId ?? "maker-" + id, "Maker", "maker/skills", "skills", "https://example.com", null, null);
        return new InsightsSkillsHome.CreatorUpdate(id, new SkillsFeedEvent(id, maker.Id, catalogId, "updated", observed),
            CatalogSkill(catalogId), maker, DateTimeOffset.Parse(observed));
    }

    private static InsightsSkillsHome.Snapshot Feed(InsightsSkillsHome.Update[] updates, InsightsSkillsHome.Trend[] trends,
        InsightsSkillsHome.CreatorUpdate[] creators) =>
        InsightsSkillsHome.Snapshot.None() with { Updates = updates, Trends = trends, Creators = creators };

    private static string[][] Ids(IReadOnlyList<InsightsSkillsHome.Group> groups) =>
        groups.Select(g => g.Stories.Select(s => s.Id).ToArray()).ToArray();

    [Fact]
    public void TheTimelineIsOneNewestFirstColumnUnderItsPeriods()
    {
        // 2026-11-15 is a Sunday: its ISO week began on Monday the 9th.
        var feed = Feed(new[] { Update("a") }, new[] { Trend("big", 1, 9), Trend("small", 3, 2) }, new[]
        {
            Change("noon", "one", "2026-11-15T11:30:00Z"), Change("morning", "two", "2026-11-15T08:00:00Z"),
            Change("yesterday", "three", "2026-11-14T23:59:59Z"), Change("monday", "four", "2026-11-09T00:00:00Z"),
            Change("last-week", "five", "2026-11-08T23:59:59Z"),
        });
        var groups = feed.Timeline(Now, Utc);
        Assert.Equal(new[] { InsightsSkillsHome.Period.Today, InsightsSkillsHome.Period.ThisWeek, InsightsSkillsHome.Period.Earlier },
            groups.Select(g => g.Period));
        Assert.Equal(new[]
        {
            new[] { "creator:noon", "update:a", "creator:morning", "trend:claude_code\0big", "trend:claude_code\0small" },
            new[] { "creator:yesterday", "creator:monday" },
            new[] { "creator:last-week" },
        }, Ids(groups));
    }

    [Fact]
    public void UpdatesAndUsageChangesAreTodayWheneverTheyWereChecked()
    {
        var night = DateTimeOffset.Parse("2026-11-16T02:00:00Z");
        var feed = Feed(new[] { Update("late", "one", "2026-11-15T23:00:00Z"), Update("later", "two", "2026-11-15T23:30:00Z") },
            new[] { Trend("big", 1, 9) },
            new[] { Change("tonight", "three", "2026-11-16T01:00:00Z"), Change("yesterday", "four", "2026-11-15T23:45:00Z") });
        Assert.Equal(new[]
        {
            new[] { "creator:tonight", "update:late", "update:later", "trend:claude_code\0big" },
            new[] { "creator:yesterday" },
        }, Ids(feed.Timeline(night, Utc)));
    }

    [Fact]
    public void ACatalogChangeForASkillWithAnUpdateIsNotToldTwiceAndTwoChangesRollUp()
    {
        var feed = Feed(new[] { Update("a", "same") }, Array.Empty<InsightsSkillsHome.Trend>(),
            new[] { Change("same-news", "same", "2026-11-14T10:00:00Z"), Change("other", "other", "2026-11-14T09:00:00Z") });
        Assert.Equal(new[] { new[] { "update:a" }, new[] { "creator:other" } }, Ids(feed.Timeline(Now, Utc)));
        var rolled = Feed(Array.Empty<InsightsSkillsHome.Update>(), Array.Empty<InsightsSkillsHome.Trend>(),
            new[] { Change("x", "one", "2026-11-14T10:00:00Z", "maker"), Change("y", "two", "2026-11-14T11:00:00Z", "maker") });
        var only = Assert.Single(Assert.Single(rolled.Timeline(Now, Utc)).Stories);
        var rollup = Assert.IsType<InsightsSkillsHome.Story.RollupStory>(only);
        Assert.Equal("rollup:thisWeek:maker", rollup.Id);
        Assert.Equal(new[] { "y", "x" }, rollup.Item.Changes.Select(c => c.Id));
    }

    [Fact]
    public void WeakHighVolumeRowsDoNotHideAMeaningfulTrend()
    {
        var rows = Enumerable.Range(0, 12).Select(i => RowJson(previous: 1000, current: 1010, name: "weak-" + i))
            .Append(RowJson(previous: 2, current: 5, name: "meaningful"));
        var stories = Project(HomeSnapshot(rows)).RankedStories(Now, Utc);
        Assert.Contains(stories, s => s.Id == "trend:claude_code\0meaningful");
        Assert.True(stories.Count <= 12);
    }

    [Fact]
    public void ASmallOrRelativelyWeakChangeIsNotRanked()
    {
        // Ranking keeps a usage change of at least 2 uses and 25% (InsightsSkillsHomeRanking.swift:50).
        var feed = Feed(Array.Empty<InsightsSkillsHome.Update>(), new[]
        {
            Trend("one-more", 3, 4) with { PeriodEnd = DateTimeOffset.Parse("2026-11-15T00:00:00Z") },
            Trend("small-share", 100, 110),
            Trend("doubled", 2, 4),
        }, Array.Empty<InsightsSkillsHome.CreatorUpdate>());
        Assert.Equal(new[] { "trend:claude_code\0doubled" }, feed.RankedStories(Now, Utc).Select(s => s.Id));
    }

    [Fact]
    public void ActiveCountsAreCutNeverRoundedUp()
    {
        Assert.Equal(new[] { "0", "999", "1K", "1.9K", "12K", "100K", "1M", "1.2M" },
            new[] { -4, 999, 1_000, 1_949, 12_345, 100_000, 1_000_000, 1_250_000 }.Select(InsightsSkillsHome.Active.Short));
    }

    [Fact]
    public void HomeRoutesResolveAgainstTheCurrentSnapshot()
    {
        SkillsSnapshot snapshot = HomeSnapshot(new[] { RowJson() });
        Assert.Equal(new SkillsHomeRoute("usage", UsageId: "claude_code\0review"), SkillsHomeRoute.Usage("claude_code\0review", snapshot));
        Assert.Null(SkillsHomeRoute.Usage("codex\0review", snapshot));
        Assert.Equal(new SkillsHomeRoute("catalog", CatalogId: "upstream", InstalledId: "local"), SkillsHomeRoute.Catalog("upstream", "local", snapshot));
        Assert.Null(SkillsHomeRoute.Catalog("upstream", "elsewhere", snapshot));
        Assert.Null(SkillsHomeRoute.Catalog("missing", null, snapshot));
        Assert.Null(SkillsHomeRoute.Index(snapshot with { Enabled = false }));
    }

    // MARK: Usage (InsightsSkillsUsageTests.swift)

    [Fact]
    public void UsageRanksInvocationsAcrossPopulationsAndLeavesOutTheFirstPartialDay()
    {
        JsonObject direct = RowJson(previous: 2, current: 5), delegated = RowJson("delegated", 1, 2);
        SkillsSnapshot snapshot = HomeSnapshot(new[] { direct, delegated });
        var ranked = InsightsSkillsUsage.Ranked(snapshot.Usage, 30, Now, Utc);
        InsightsSkillsUsage.Skill skill = Assert.Single(ranked);
        Assert.Equal(10, skill.Uses);
        // 30 days before noon on the 15th is noon on 16 October: the 17th is the first whole day.
        Assert.Equal(new[] { "2026-10-30", "2026-11-03", "2026-11-10" }, skill.Daily.Select(d => d.Id));
        Assert.Equal(new[] { 2, 3, 7 }, skill.Daily.Select(d => d.Uses));
        Assert.Empty(InsightsSkillsUsage.Ranked(snapshot.Usage, 14, Now, Utc));
        Assert.Equal("skills_usage_empty", InsightsSkillsUsage.EmptyState(snapshot.Usage, 30));
        Assert.Equal("skills_usage_pending", InsightsSkillsUsage.EmptyState(snapshot.Usage, 7));
        Assert.Equal("skills_analysis_off", InsightsSkillsUsage.EmptyState(HomeSnapshot(analysis: false).Usage, 30));
    }

    // MARK: Cover art (InsightsSkillsCoverTests.swift)

    [Fact]
    public void TheCoverHashIsFnv1aWithTheMacsVectors()
    {
        Assert.Equal(0xcbf2_9ce4_8422_2325UL, SkillCoverArt.Fnv1a(""));
        Assert.Equal(0xaf63_dc4c_8601_ec8cUL, SkillCoverArt.Fnv1a("a"));
        Assert.Equal(0x8594_4171_f739_67e8UL, SkillCoverArt.Fnv1a("foobar"));
        Assert.Equal(0xefd0_1f60_ba99_2926UL, SkillCoverArt.Seed(""));
        Assert.Equal(0x82a2_a958_a9be_ce5bUL, SkillCoverArt.Seed("a"));
    }

    [Fact]
    public void TheSameSkillHasTheMacsCover()
    {
        SkillCoverArt.Spec review = SkillCoverArt.Spec.For("catalog-review", "creator");
        Assert.Equal(5, review.Palette);
        Assert.Equal(SkillCoverArt.Pattern.Dots, review.Pattern);
        Assert.Equal(253.0 / 255, review.AnchorX, 12);
        Assert.Equal(140.0 / 255, review.AnchorY, 12);
        Assert.Equal(0.12 + 56.0 / 63 * 0.12, review.Step, 12);
        Assert.Equal(143.0 / 255 * 70 - 35, review.Angle, 12);
        Assert.False(review.Flipped);
        SkillCoverArt.Spec brainstorming = SkillCoverArt.Spec.For("catalog-brainstorming", "creator");
        Assert.Equal(SkillCoverArt.Pattern.Rings, brainstorming.Pattern);
        Assert.Equal(review.Palette, brainstorming.Palette);
        SkillCoverArt.Spec elsewhere = SkillCoverArt.Spec.For("catalog-review", "other");
        Assert.Equal((0, SkillCoverArt.Pattern.Rings), (elsewhere.Palette, elsewhere.Pattern));
        Assert.NotEqual(SkillCoverArt.Spec.For("ab", "c"), SkillCoverArt.Spec.For("a", "bc"));
        Assert.Equal(4, Enumerable.Range(0, 200).Select(i => SkillCoverArt.Spec.For("skill-" + i, "p").Pattern).Distinct().Count());
        Assert.Equal(8, Enumerable.Range(0, 200).Select(i => SkillCoverArt.Spec.For("s", "publisher-" + i).Palette).Distinct().Count());
    }

    [Fact]
    public void WhiteInitialsHoldContrastOnEveryPaletteUnderThePattern()
    {
        foreach ((uint from, uint to) in SkillCoverArt.Palettes)
        {
            foreach (uint stop in new[] { from, to })
            {
                Assert.True(SkillCoverArt.ContrastWithWhite(stop) >= 4.5, stop.ToString("X6"));
                uint Lighten(int shift)
                {
                    double c = ((stop >> shift) & 0xFF) / 255.0;
                    return (uint)Math.Floor((c + (1 - c) * SkillCoverArt.InkOpacity) * 255) << shift;
                }

                Assert.True(SkillCoverArt.ContrastWithWhite(Lighten(16) | Lighten(8) | Lighten(0)) >= 4.5, stop.ToString("X6"));
            }
        }

        Assert.Equal(1, SkillCoverArt.ContrastWithWhite(0xFFFFFF), 9);
        Assert.Equal(21, SkillCoverArt.ContrastWithWhite(0x000000), 9);
    }

    [Fact]
    public void InitialsComeFromThePublishersWords()
    {
        Assert.Equal(new[] { "OB", "TC", "CS", "X", "" },
            new[] { "Obra", "Trace Commons Team", "creator/skills", "x", " -- " }.Select(SkillCoverArt.Initials));
    }

    [Fact]
    public void ThePatternMarksStayInsideTheReachAndUseTheMacsStep()
    {
        SkillCoverArt.Spec review = SkillCoverArt.Spec.For("catalog-review", "creator");
        SkillCoverArt.Mark[] dots = SkillCoverArt.Marks(review, 96, 64);
        double step = 64 * review.Step;
        Assert.All(dots, d => Assert.Equal(step * 0.45, d.Width, 9));
        Assert.Equal(step * (review.AnchorX - 1), dots[0].X, 9);
        Assert.Equal(step * (review.AnchorY - 1), dots[0].Y, 9);
        Assert.DoesNotContain(SkillCoverArt.Marks(review with { Step = 0.12 }, 0, 0), d => d.Width > 8);
    }

    [Fact]
    public void TheHomeMotionNumbersAreTheMacs()
    {
        Assert.Equal((1.5, 0.15), (SkillsHomeMotion.HoverLift, SkillsHomeMotion.HoverSeconds));
        Assert.Equal((0.6, 0.985), (SkillsHomeMotion.ScrollOpacity, SkillsHomeMotion.ScrollScale));
        Assert.Equal((0.02, 0.5), (SkillsHomeMotion.GrowFrom, SkillsHomeMotion.GrowSeconds));
        Assert.Equal(0.25, SkillsHomeMotion.ExpandSeconds);
        Assert.Equal((560.0, 96.0, 40.0), (SkillsHomeMotion.ColumnMaximum, SkillsHomeMotion.HeroBand, SkillsHomeMotion.MarkSize));
        Assert.False(SkillsHomeMotion.Still.GrowsBars || SkillsHomeMotion.Still.Lifts || SkillsHomeMotion.Still.Scrolls
            || SkillsHomeMotion.Still.RollsNumbers);
        var moving = new SkillsHomeMotion(false);
        Assert.True(moving.GrowsBars && moving.Lifts && moving.Scrolls && moving.RollsNumbers);
        Assert.Equal(0, SkillsHomeMotion.Snappy(0));
        Assert.Equal(1, SkillsHomeMotion.Snappy(SkillsHomeMotion.SnappySettling));
        Assert.True(SkillsHomeMotion.EaseOut(0.5) > 0.5);
    }

    [Theory]
    // Printed by SwiftUI itself on macOS (a throwaway probe, 2026-11-19):
    // Spring.snappy(duration: 0.25).value(target: 1, time: t); the same
    // probe printed response 0.25, dampingRatio 0.85 and a settlingDuration
    // of 0.4688122063285237.
    [InlineData(0.025, 0.138210353)]
    [InlineData(0.05, 0.388102446)]
    [InlineData(0.1, 0.786369170)]
    [InlineData(0.15, 0.956446594)]
    [InlineData(0.2, 1.001616993)]
    [InlineData(0.25, 1.006019961)]
    [InlineData(0.3, 1.003072604)]
    [InlineData(0.4, 1.000154774)]
    public void TheExpandAndRollFollowSwiftUIsSnappySpring(double seconds, double want)
    {
        Assert.Equal(want, SkillsHomeMotion.Snappy(seconds), 6);
        Assert.Equal((0.25, 0.85, 0.4688122063285237), (SkillsHomeMotion.SnappyResponse, SkillsHomeMotion.SnappyDamping, SkillsHomeMotion.SnappySettling));
    }
}
