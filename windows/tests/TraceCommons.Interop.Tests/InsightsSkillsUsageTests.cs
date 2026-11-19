using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;
using static TraceCommons.Interop.Tests.SkillsFixtures;

namespace TraceCommons.Interop.Tests;

/// <summary>macos/Tests/TraceCommonsAppTests/InsightsSkillsUsageTests.swift, against the Windows Usage section.</summary>
public class InsightsSkillsUsageTests
{
    private static readonly DateTimeOffset Now = DateTimeOffset.Parse("2026-11-14T12:00:00Z");
    private static readonly TimeZoneInfo Utc = TimeZoneInfo.Utc;

    private static SkillsUsageRow UseRow(string population = "direct", int uses = 2, int loads = 0, int undated = 0, bool windows = true) =>
        Row("review", "claude_code", uses, loads, 1, population: population, undated: undated,
            windows: windows ? new[] { Window(7, 0, 0, 0), Window(30, uses, loads, 1) } : Array.Empty<SkillsUsageWindow>());

    private static readonly SkillsUsageRow History = Row("history", "claude_code", 1, 0, 1,
        windows: new[] { Window(7, 0, 0, 0), Window(30, 0, 0, 0) }, daily: new[] { new SkillsUsageDay("2026-08-01", 1, 0, 1) });

    private static SkillInstallationDetails Details(string availability = "available", string definition = "valid") =>
        new(Array.Empty<string>(), availability, definition);

    private static readonly InstalledSkill Local = Skill("local", "review", "claude_code", scope: "user", sha256: "a");

    private static SkillsSnapshot UsageSnapshot(SkillsUsageRow[]? rows = null, bool partial = false,
        string status = "explicit_invocations", string earliest = "2026-07-01T00:00:00Z") =>
        Snapshot(
            inventory: Inventory(new[] { Local }, partial: false, details: new Dictionary<string, SkillInstallationDetails> { ["local"] = Details() }),
            usage: Usage(enabled: true, lastAnalysisAt: "2026-11-14T12:00:00Z", earliestEvidence: earliest, partial: partial,
                skills: (rows ?? Array.Empty<SkillsUsageRow>()).Append(History).ToArray(), coverage: new[] { Coverage("claude_code", status) }));

    private static IReadOnlyList<InstalledSkill> Removal(SkillsSnapshot snapshot, int days = 30) =>
        InsightsSkillsUsage.RemovalCandidates(snapshot, days, new InsightsSkillsModel(), Now, Utc);

    [Fact]
    public void EmptyStateNeverCallsDisabledOrUnknownDataUnused()
    {
        Assert.Equal("skills_analysis_off", InsightsSkillsUsage.EmptyState(SkillsUsage.Empty, 30));
        Assert.Equal("skills_usage_unavailable", InsightsSkillsUsage.EmptyState(UsageSnapshot(status: "loads_only").Usage, 30));
        Assert.Equal("skills_usage_pending", InsightsSkillsUsage.EmptyState(UsageSnapshot(partial: true).Usage, 30));
        Assert.Equal("skills_usage_empty", InsightsSkillsUsage.EmptyState(UsageSnapshot(new[] { UseRow(uses: 0) }).Usage, 30));
    }

    [Fact]
    public void MissingWindowsAndUndatedOrUnclassifiedUsesDoNotClaimZero()
    {
        foreach (SkillsUsageRow row in new[] { UseRow(windows: false), UseRow("unknown", uses: 1), UseRow(uses: 0, undated: 1) })
        {
            Assert.Equal("skills_usage_pending", InsightsSkillsUsage.EmptyState(UsageSnapshot(new[] { row }).Usage, 30));
        }

        Assert.Equal("skills_usage_pending", InsightsSkillsUsage.EmptyState(UsageSnapshot(new[] { UseRow(uses: 0) }).Usage, 90));
    }

    [Fact]
    public void RankingCombinesUsesButNeverLoadsOrSessionCounts()
    {
        SkillsUsage usage = UsageSnapshot(new[] { UseRow(uses: 2, loads: 9), UseRow("delegated", uses: 3), UseRow("unknown", uses: 99) }).Usage;
        IReadOnlyList<InsightsSkillsUsage.Skill> ranked = InsightsSkillsUsage.Ranked(usage, 30, Now, Utc);
        InsightsSkillsUsage.Skill only = Assert.Single(ranked);
        Assert.Equal(5, only.Uses);
        Assert.Equal(2, only.Sources.Count);
        Assert.Empty(InsightsSkillsUsage.Ranked(usage, 7, Now, Utc));
        Assert.Empty(InsightsSkillsUsage.Ranked(UsageSnapshot(new[] { UseRow() }, status: "loads_only").Usage, 30, Now, Utc));
    }

    [Fact]
    public void DailyChartAddsDisjointPopulationCountsAndFiltersOldDates()
    {
        var rows = JsonDocument.Parse("""
        [{"name":"review","agent":"claude_code","population":"direct","invocations":2,"loads":0,"sessions":1,"windows":[{"days":7,"invocations":2,"loads":0,"sessions":1}],"daily":[{"day":"2026-11-13","invocations":2,"loads":0,"sessions":1},{"day":"2026-10-01","invocations":50,"loads":0,"sessions":1},{"day":"2026-11-07","invocations":50,"loads":0,"sessions":1}]},
        {"name":"review","agent":"claude_code","population":"delegated","invocations":3,"loads":0,"sessions":1,"windows":[{"days":7,"invocations":3,"loads":0,"sessions":1}],"daily":[{"day":"2026-11-13","invocations":3,"loads":0,"sessions":1}]}]
        """).RootElement.EnumerateArray().Select(e => Decode("{\"usage\":{\"skills\":[" + e.GetRawText() + "]}}").Usage.Skills[0]).ToArray();
        IReadOnlyList<InsightsSkillsUsage.Skill> ranked = InsightsSkillsUsage.Ranked(UsageSnapshot(rows).Usage, 7, Now, Utc);
        Assert.Equal(new[] { new InsightsSkillsUsage.Day("2026-11-13", 5) }, ranked[0].Daily);
    }

    [Fact]
    public void RemovalRequiresCompleteInvocationCoverageForEntirePeriod()
    {
        Assert.Equal(new[] { "local" }, Removal(UsageSnapshot()).Select(s => s.Id));
        foreach (SkillsSnapshot unavailable in new[]
                 {
                     UsageSnapshot(partial: true), UsageSnapshot(status: "loads_only"), UsageSnapshot(status: "pending"),
                     UsageSnapshot(earliest: "2026-11-13T00:00:00Z"), UsageSnapshot(new[] { UseRow(uses: 0, undated: 1) }),
                     UsageSnapshot(new[] { UseRow("unknown", uses: 0) }), UsageSnapshot(new[] { UseRow(uses: 0, windows: false) }),
                     UsageSnapshot(new[] { UseRow(uses: 0, loads: 1) }), UsageSnapshot(new[] { UseRow("delegated", uses: 2) }),
                 })
        {
            Assert.Empty(Removal(unavailable));
        }

        Assert.Single(Removal(UsageSnapshot(new[] { UseRow(uses: 2) }), days: 7));
    }

    [Fact]
    public void IncompleteInventoryAndUnavailableInstallationAreWithheld()
    {
        SkillsSnapshot original = UsageSnapshot();
        // The Mac's test varies the availability; an installation whose
        // definition is not valid is withheld by the same guard
        // (InsightsSkillsUsage.swift:81), so it is pinned beside them.
        foreach (SkillInstallationDetails details in new[]
                 {
                     Details("cached"), Details("unknown"), Details("disabled"), Details("missing"),
                     Details(definition: "invalid"), Details(definition: "unknown"),
                 })
        {
            SkillsSnapshot changed = original with
            {
                Inventory = Inventory(original.Inventory.Skills, partial: false,
                    details: new Dictionary<string, SkillInstallationDetails> { ["local"] = details }),
            };
            Assert.Empty(Removal(changed));
        }

        SkillsSnapshot truncated = original with { Inventory = original.Inventory with { Partial = true } };
        Assert.Empty(Removal(truncated));
    }

    [Fact]
    public void UnrelatedAgentHistoryCannotQualifyAnUnusedSkill()
    {
        SkillsSnapshot original = UsageSnapshot();
        SkillsUsageRow codex = Row("history", "codex", 0, 1, 1, daily: History.Daily);
        SkillsUsage usage = Usage(enabled: true, lastAnalysisAt: original.Usage.LastAnalysisAt, earliestEvidence: original.Usage.EarliestEvidence,
            partial: false, skills: new[] { codex },
            coverage: new[] { Coverage("claude_code", "explicit_invocations"), Coverage("codex", "loads_only") });
        Assert.Empty(Removal(Snapshot(inventory: original.Inventory, usage: usage)));
    }

    [Fact]
    public void CollidingAndSharedInstallationsAreNotRemovalCandidates()
    {
        SkillsSnapshot original = UsageSnapshot();
        InstalledSkill duplicate = Skill("other", "review", "claude_code", scope: "project", location: "elsewhere", sha256: "b");
        // The Mac builds these inventories partial by default, which withholds
        // them on its own; here they are complete and available, so only the
        // collision and the shared root withhold them.
        var available = new Dictionary<string, SkillInstallationDetails> { ["local"] = Details(), ["other"] = Details(), ["shared"] = Details() };
        Assert.Empty(Removal(Snapshot(inventory: Inventory(original.Inventory.Skills.Append(duplicate).ToArray(), partial: false, details: available),
            usage: original.Usage)));
        InstalledSkill shared = Skill("shared", "review", "shared", scope: "user", sha256: "b");
        Assert.Empty(Removal(Snapshot(inventory: Inventory(new[] { shared }, partial: false, details: available), usage: original.Usage)));
    }
}
