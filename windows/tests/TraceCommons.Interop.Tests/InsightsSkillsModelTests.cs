using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;
using static TraceCommons.Interop.Tests.SkillsFixtures;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The rest of the Mac's macos/Tests/TraceCommonsAppTests/InsightsSkillsTests.swift,
/// test for test, against the Windows model.
/// </summary>
public class InsightsSkillsModelTests
{
    [Fact]
    public void DirectAndDelegatedUsageRemainSeparateForOneInstallation()
    {
        var model = new InsightsSkillsModel();
        InstalledSkill skill = Skill("local", "review", "opencode");
        SkillsUsageRow direct = Row("review", "opencode", 1, 0, 1);
        SkillsUsageRow delegated = Row("review", "opencode", 2, 0, 1, population: "delegated",
            evidence: new[] { new SkillsUsageEvidence("child", "parent"), new SkillsUsageEvidence("child", "parent") });
        SkillsSnapshot snapshot = Snapshot(inventory: Inventory(new[] { skill }),
            usage: Usage(enabled: true, skills: new[] { direct, delegated }, coverage: new[] { Coverage("opencode", "explicit_invocations") }));
        Assert.Equal(2, model.UsageRows(skill, snapshot).Count);
        Assert.Null(model.Usage(skill, snapshot)); // Do not combine direct and delegated trace totals.
        Assert.Equal(new[] { new SkillsUsageEvidence("child", "parent") }, InsightsSkillsModel.EvidenceLinks(delegated));
        Assert.Equal(1, delegated.Sessions); // Parent context is not another observed trace.
    }

    [Fact]
    public void PartialOpenCodeCoverageCannotClaimNoUsage()
    {
        var model = new InsightsSkillsModel();
        InstalledSkill skill = Skill("local", "review", "opencode");
        foreach (string status in new[] { "partial", "pending", "unknown" })
        {
            SkillsSnapshot snapshot = Snapshot(inventory: Inventory(new[] { skill }),
                usage: Usage(enabled: true, lastAnalysisAt: "today", partial: false, coverage: new[] { Coverage("opencode", status) }));
            Assert.Equal("skills_usage_pending", model.UsageState(skill, snapshot));
        }
    }

    [Fact]
    public async Task OpenCodeSkillEvidenceDoesNotRequireTokenDrill()
    {
        var model = new InsightsSkillsModel(evidenceReader: _ => Task.FromResult(SkillEvidence.Parse(JsonDocument.Parse(
            """{"enabled":true,"readable":true,"agent":"opencode","population":"unknown","skill_usage":{"coverage":"explicit_invocations","partial":true,"skills":[{"name":"review","invocations":1,"loads":0}]}}""").RootElement)!));
        await model.LoadEvidenceAsync("os.opaque");
        Assert.False(model.EvidenceFailure);
        Assert.Null(model.Evidence?.Session);
        Assert.Equal("review", model.Evidence?.SkillUsage?.Skills.FirstOrDefault()?.Name);
    }

    [Fact]
    public void InventorySearchFiltersAndUsageSort()
    {
        var model = new InsightsSkillsModel();
        InstalledSkill alpha = new("a", "Alpha", "Reviews code", "codex", "global", "hidden", "a", null);
        InstalledSkill beta = new("b", "Beta", "Builds", "claude_code", "global", "hidden", "b", null);
        SkillsSnapshot snapshot = Snapshot(inventory: Inventory(new[] { beta, alpha }), usage: Usage(skills: new[]
        {
            Row("Beta", "claude_code", 10, 0, 3, windows: new[] { Window(30, 10, 0, 3) }),
        }));
        Assert.Equal(new[] { "a", "b" }, model.InventorySkills(snapshot, "").Select(s => s.Id));
        Assert.Equal(new[] { "a" }, model.InventorySkills(snapshot, "reviews").Select(s => s.Id));
        Assert.Empty(model.InventorySkills(snapshot, "hidden"));
        Assert.Equal(new[] { "a" }, model.InventorySkills(snapshot, "", agent: "codex").Select(s => s.Id));
        Assert.Equal(new[] { "b", "a" }, model.InventorySkills(snapshot, "", sort: "sessions").Select(s => s.Id));
        Assert.Equal(new[] { "a", "b" }, model.InventorySkills(snapshot, "", sort: "sessions", days: 7).Select(s => s.Id));
    }

    [Fact]
    public void AmbiguousInstallationsDoNotReceiveNameOnlyCounts()
    {
        var model = new InsightsSkillsModel();
        InstalledSkill first = Skill("a", "Review", "codex", location: "a", sha256: "a");
        InstalledSkill second = Skill("b", "Review", "codex", scope: "project", location: "b", sha256: "b");
        SkillsUsage usage = Usage(enabled: true, lastAnalysisAt: "2026-11-11T00:00:00Z", partial: false,
            skills: new[] { Row("Review", "codex", 0, 2, 1) }, coverage: new[] { Coverage("codex", "loads_only") });
        SkillsSnapshot snapshot = Snapshot(inventory: Inventory(new[] { first, second }), usage: usage);
        Assert.Null(model.Usage(first, snapshot));
        Assert.Equal("skills_usage_unattributed", model.UsageState(first, snapshot));
        Assert.Equal("skills_usage_unavailable", model.UsageState(first, Snapshot(inventory: Inventory(new[] { first }))));
    }

    [Fact]
    public void SharedSkillKeepsConsumerUsageSeparateAndWithholdsCollisions()
    {
        var model = new InsightsSkillsModel();
        InstalledSkill shared = Skill("shared", "Review", "shared", location: "shared", sha256: "a");
        InstalledSkill competing = Skill("private", "Review", "codex", location: "private", sha256: "b");
        SkillsUsage usage = Usage(enabled: true, lastAnalysisAt: "2026-11-11T00:00:00Z", partial: false, skills: new[]
        {
            Row("Review", "codex", 0, 2, 1, sessionRefs: new[] { "same" }, windows: new[] { Window(30, 0, 2, 1) }),
            Row("Review", "claude_code", 1, 0, 1, sessionRefs: new[] { "same" }, windows: new[] { Window(30, 1, 0, 1) }),
        }, coverage: new[] { Coverage("codex", "loads_only"), Coverage("claude_code", "explicit_invocations") });
        SkillsSnapshot snapshot = Snapshot(inventory: Inventory(new[] { shared }), usage: usage);
        IReadOnlyList<SkillsUsageRow> rows = model.UsageRows(shared, snapshot);
        Assert.Equal(new[] { "claude_code", "codex" }, rows.Select(r => r.Agent));
        Assert.Equal(new[] { 1, 1 }, rows.Select(r => r.Sessions));
        Assert.Null(model.Usage(shared, snapshot)); // Never sum potentially overlapping trace totals.
        Assert.NotEqual("skills_usage_unavailable", model.UsageState(shared, snapshot));
        SkillsSnapshot collision = Snapshot(inventory: Inventory(new[] { shared, competing }), usage: usage);
        Assert.Empty(model.UsageRows(shared, collision));
        Assert.Empty(model.UsageRows(competing, collision));
        Assert.Equal("skills_usage_unattributed", model.UsageState(shared, collision));
    }

    [Fact]
    public async Task DefinitionAndEvidenceReadersUseDaemonResults()
    {
        var asked = new List<string>();
        var model = new InsightsSkillsModel(definitionReader: id =>
        {
            asked.Add(id);
            return Task.FromResult(SkillDefinition.Parse(JsonDocument.Parse(
                """{"skill_id":"local-id","status":"available","definition":"# Local definition"}""").RootElement)!);
        }, evidenceReader: reference =>
        {
            asked.Add(reference);
            return Task.FromResult(SkillEvidence.Parse(JsonDocument.Parse(
                """{"enabled":true,"readable":true,"session":{"source":"codex","date":"2026-11-11","turns":null,"tokens":42}}""").RootElement)!);
        });
        await model.LoadDefinitionAsync("local-id");
        Assert.Equal("# Local definition", model.Definition?.Definition);
        Assert.False(model.DefinitionFailure);
        await model.LoadEvidenceAsync("t1.42");
        Assert.Equal(42UL, model.Evidence?.Session?.Tokens);
        Assert.False(model.EvidenceFailure);
        Assert.Equal(new[] { "local-id", "t1.42" }, asked);
    }

    [Fact]
    public async Task UnavailableDefinitionAndRevokedEvidenceStayUnavailable()
    {
        var model = new InsightsSkillsModel(
            definitionReader: _ => Task.FromResult(SkillDefinition.Parse(JsonDocument.Parse(
                """{"skill_id":"local-id","status":"unavailable"}""").RootElement)!),
            evidenceReader: _ => Task.FromResult(SkillEvidence.Parse(JsonDocument.Parse("""{"enabled":false}""").RootElement)!));
        await model.LoadDefinitionAsync("local-id");
        Assert.True(model.DefinitionFailure);
        Assert.Null(model.Definition?.Definition);
        await model.LoadEvidenceAsync("t1.42");
        Assert.True(model.EvidenceFailure);
    }

    [Fact]
    public async Task ClosedEvidenceReadsAsUnavailableUntilANewReadStarts()
    {
        // invalidateDetails() (InsightsSkillsModel.swift:44-49): the definition
        // is not failed, but a closed evidence panel is unavailable.
        var model = new InsightsSkillsModel(evidenceReader: _ => Task.FromResult(SkillEvidence.Parse(JsonDocument.Parse(
            """{"enabled":true,"readable":true,"session":{"source":"codex"}}""").RootElement)!));
        await model.LoadEvidenceAsync("t1.42");
        Assert.False(model.EvidenceFailure);
        model.InvalidateDetails();
        Assert.Null(model.Evidence);
        Assert.True(model.EvidenceFailure);
        Assert.False(model.DefinitionFailure);
        Assert.Null(model.Definition);
    }

    /// <summary>The Mac's <c>DelayedDefinition</c>: the first read waits until released and answers "old", later ones "new".</summary>
    private sealed class DelayedDefinition
    {
        private readonly TaskCompletionSource _release = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private int _reads;
        public bool HasWaiter => Volatile.Read(ref _reads) >= 1;

        public async Task<SkillDefinition> Read(string id)
        {
            bool first = Interlocked.Increment(ref _reads) == 1;
            if (first) await _release.Task;
            return new SkillDefinition(id, "available", first ? "old" : "new");
        }

        public void Release() => _release.TrySetResult();
    }

    [Fact]
    public async Task LateSameIdentityDefinitionCannotRestoreClearedSelection()
    {
        var replies = new DelayedDefinition();
        var model = new InsightsSkillsModel(definitionReader: replies.Read);
        Task old = model.LoadDefinitionAsync("same-id");
        Assert.True(replies.HasWaiter);
        model.InvalidateDetails();
        await model.LoadDefinitionAsync("same-id");
        Assert.Equal("new", model.Definition?.Definition);
        replies.Release();
        await old;
        Assert.Equal("new", model.Definition?.Definition);
    }

    [Fact]
    public async Task InvalidationDiscardsInFlightDefinitionWithoutReplacement()
    {
        var replies = new DelayedDefinition();
        var model = new InsightsSkillsModel(definitionReader: replies.Read);
        Task old = model.LoadDefinitionAsync("same-id");
        Assert.True(replies.HasWaiter);
        model.InvalidateDetails();
        replies.Release();
        await old;
        Assert.Null(model.Definition);
    }

    private sealed class ObservedReads
    {
        private readonly object _gate = new();
        private readonly List<bool> _refreshes = new();

        public bool[] Refreshes
        {
            get { lock (_gate) return _refreshes.ToArray(); }
        }

        public Task<SkillsSnapshot> Read(bool refresh)
        {
            lock (_gate) _refreshes.Add(refresh);
            return Task.FromResult(Snapshot(enabled: true));
        }
    }

    [Fact]
    public async Task AutomaticOpeningAndPeriodicReadsDoNotForceCatalogRefresh()
    {
        var reads = new ObservedReads();
        var model = new InsightsSkillsModel(reads.Read, idlePollInterval: TimeSpan.FromMilliseconds(1));
        using var cancel = new CancellationTokenSource();
        Task observation = model.ObserveAsync(cancel.Token);
        for (int i = 0; i < 1_000 && reads.Refreshes.Length < 2; i++) await Task.Delay(1);
        cancel.Cancel();
        await observation;
        bool[] automatic = reads.Refreshes;
        Assert.True(automatic.Length >= 2, "Opening and a periodic read must both be exercised");
        Assert.All(automatic, refresh => Assert.False(refresh)); // Automatic reads keep the daemon's six-hour throttle.
        await model.LoadAsync(refresh: true);
        Assert.True(reads.Refreshes[^1]); // The explicit Refresh action can still force a refresh.
    }

    /// <summary>The Mac's <c>DelayedRead</c>: the first read ignores cancellation until released.</summary>
    private sealed class DelayedRead
    {
        private readonly TaskCompletionSource _release = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private int _count;
        public int Count => Volatile.Read(ref _count);

        public async Task<SkillsSnapshot> Read(bool refresh)
        {
            if (Interlocked.Increment(ref _count) == 1)
            {
                await _release.Task;
                return Snapshot(enabled: true, followed: new[] { "stale" });
            }

            return Snapshot(enabled: true, followed: new[] { "fresh" });
        }

        public void Release() => _release.TrySetResult();
    }

    [Fact]
    public async Task ReactivatedObserverWaitsForCancelledReadThenReadsPromptly()
    {
        var reads = new DelayedRead();
        var model = new InsightsSkillsModel(reads.Read);
        using var first = new CancellationTokenSource();
        Task firstObserver = model.ObserveAsync(first.Token);
        for (int i = 0; i < 1_000 && reads.Count != 1; i++) await Task.Delay(1);
        Assert.True(model.Busy);
        first.Cancel();
        using var second = new CancellationTokenSource();
        Task replacement = model.ObserveAsync(second.Token);
        // Let the replacement meet the occupied lane before the reply arrives.
        await Task.Delay(20);
        reads.Release();
        for (int i = 0; i < 1_000 && !(model.Shown?.FollowedPublishers.SequenceEqual(new[] { "fresh" }) ?? false); i++) await Task.Delay(1);
        second.Cancel();
        await firstObserver;
        await replacement;
        Assert.Equal(new[] { "fresh" }, model.Shown?.FollowedPublishers); // Reactivation must not wait for the five-minute poll.
        Assert.Equal(2, reads.Count);
        Assert.False(model.Busy);
    }

    [Fact]
    public async Task RefreshFailurePreservesLastSuccessfulSnapshot()
    {
        int reads = 0;
        var model = new InsightsSkillsModel(_ => ++reads > 1
            ? Task.FromException<SkillsSnapshot>(new InvalidOperationException("insights_skills not available yet"))
            : Task.FromResult(Snapshot(enabled: true, followed: new[] { "publisher" })));
        await model.LoadAsync(refresh: true);
        Assert.Equal(new[] { "publisher" }, model.Shown?.FollowedPublishers);
        await model.LoadAsync(refresh: true);
        Assert.Equal(new[] { "publisher" }, model.Shown?.FollowedPublishers);
        Assert.Equal("skills_refresh_failed", model.FailureKey);
        Assert.False(model.Busy);
    }

    [Fact]
    public async Task UnreadableResponseClearsPreviouslyVisibleSkills()
    {
        int reads = 0;
        var model = new InsightsSkillsModel(_ => Task.FromResult(++reads == 1
            ? Snapshot(enabled: true, followed: new[] { "old" })
            : Snapshot(enabled: true, readable: false)));
        await model.LoadAsync();
        Assert.Equal(new[] { "old" }, model.Shown?.FollowedPublishers);
        await model.LoadAsync();
        Assert.Null(model.Shown); // An authoritative unreadable answer may reflect a revoked scope.
        Assert.Equal("skills_unavailable", model.FailureKey);
    }

    [Fact]
    public async Task FollowUsesConfirmedSnapshotAndSerializesMutations()
    {
        var gate = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        int writes = 0;
        var model = new InsightsSkillsModel(_ => Task.FromResult(Snapshot(enabled: true)), async (publisher, followed) =>
        {
            Interlocked.Increment(ref writes);
            await gate.Task;
            return Snapshot(enabled: true, followed: followed ? new[] { publisher } : Array.Empty<string>());
        });
        await model.LoadAsync();
        Task first = model.SetFollowAsync("a", true);
        Assert.Equal(1, writes);
        Assert.Empty(model.Shown!.FollowedPublishers); // No optimistic follow state.
        await model.SetFollowAsync("b", true);
        gate.SetResult();
        await first;
        Assert.Equal(1, writes);
        Assert.Equal(new[] { "a" }, model.Shown!.FollowedPublishers);
        Assert.False(model.Busy);
    }

    private static readonly Dictionary<string, string> SummaryCopy = new()
    {
        ["skills_invocations"] = "{count} invocations", ["skills_no_invocations"] = "No observed invocations",
        ["skills_loads"] = "{count} loads", ["skills_sessions"] = "{count} traces",
    };

    [Fact]
    public void LoadsOnlyDoesNotPresentAnUnknownInvocationCountAsZero()
    {
        var model = new InsightsSkillsModel(copy: key => SummaryCopy.TryGetValue(key, out string? text) ? text : null);
        SkillsUsageRow row = Row("review", "codex", 0, 3, 2);
        Assert.Equal("3 loads · 2 traces", model.UsageSummary(row, new[] { Coverage("codex", "loads_only") }));
        Assert.Equal("No observed invocations · 3 loads · 2 traces", model.UsageSummary(row, new[] { Coverage("codex", "explicit_invocations") }));
    }

    [Fact]
    public void ExplicitInvocationCoverageDoesNotPresentUnobservedClaudeLoadsAsZero()
    {
        var model = new InsightsSkillsModel(copy: key => key == "skills_no_invocations" ? null
            : SummaryCopy.TryGetValue(key, out string? text) ? text : null);
        SkillsUsageRow row = Row("review", "claude_code", 3, 0, 2);
        Assert.Equal("3 invocations · 2 traces", model.UsageSummary(row, new[] { Coverage("claude_code", "explicit_invocations") }));
    }

    [Fact]
    public void SkillsSectionsKeepTheirOrderAndSharedCopyKeys()
    {
        Assert.Equal(new[] { "home", "inventory", "usage", "catalog", "following" }, InsightsSkillsModel.Sections);
        Assert.Equal(new[] { "skills_tab_home", "skills_tab_inventory", "skills_tab_usage", "skills_tab_catalog", "skills_tab_following" },
            InsightsSkillsModel.Sections.Select(InsightsSkillsModel.SectionCopyKey));
    }

    [Fact]
    public void TimestampsUseOneLocalizedDateFormatWithOrWithoutFractions()
    {
        string whole = InsightsSkillsModel.Timestamp("2026-11-05T18:00:00Z");
        Assert.Equal(whole, InsightsSkillsModel.Timestamp("2026-11-05T18:00:00.000Z"));
        Assert.NotEqual("2026-11-05T18:00:00Z", whole);
        Assert.Contains("2026", whole, StringComparison.Ordinal);
        Assert.Equal("unknown", InsightsSkillsModel.Timestamp("unknown"));
    }

    [Fact]
    public void MatchingNeedsRepositoryAndPathInsteadOfAnIdenticalName()
    {
        SkillsSnapshot snapshot = Decode("""
        {"enabled":true,"inventory":{"skills":[{"id":"local","name":"review","description":"","agent":"codex","scope":"user","location":"local","sha256":"hash"}],"partial":false,"coverage":[]},"catalog":{"publishers":[],"skills":[{"id":"remote","publisher_id":"p","name":"review","description":"","repository":"owner/repo","path":"skills/review","revision":"main","sha256":"hash","source_url":"https://example.com/source","instructions_url":"https://example.com/docs"}]}}
        """);
        Assert.Null(InsightsSkillsModel.CatalogMatch(snapshot.Inventory.Skills[0], snapshot));
    }
}
