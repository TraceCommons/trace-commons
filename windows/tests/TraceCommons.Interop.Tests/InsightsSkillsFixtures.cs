using System;
using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Swift wire types' memberwise initialisers with their defaults
/// (macos/Sources/TCShellCore/DataContract/SkillsModels.swift), so a ported
/// Mac test reads as its Swift does: an inventory and a usage block are
/// partial unless a test says otherwise.
/// </summary>
internal static class SkillsFixtures
{
    public static SkillsSnapshot Decode(string json) =>
        SkillsSnapshot.Parse(JsonDocument.Parse(json).RootElement) ?? throw new InvalidOperationException("undecodable");

    public static SkillsSnapshot Snapshot(bool enabled = false, bool readable = true, bool refreshing = false, string? error = null,
        SkillsInventory? inventory = null, SkillsUsage? usage = null, string[]? followed = null) =>
        SkillsSnapshot.Empty(enabled) with
        {
            Readable = readable, Refreshing = refreshing, Error = error,
            Inventory = inventory ?? SkillsInventory.Empty, Usage = usage ?? SkillsUsage.Empty, FollowedPublishers = followed ?? Array.Empty<string>(),
        };

    public static InstalledSkill Skill(string id, string name, string agent, string scope = "global", string location = "local",
        string sha256 = "hash", SkillsSource? source = null) =>
        new(id, name, "", agent, scope, location, sha256, source);

    public static SkillsInventory Inventory(IReadOnlyList<InstalledSkill> skills, bool partial = true,
        IReadOnlyDictionary<string, SkillInstallationDetails>? details = null) =>
        new(skills, partial, null, Array.Empty<SkillsInventoryCoverage>(),
            details ?? new Dictionary<string, SkillInstallationDetails>());

    public static SkillsUsageRow Row(string name, string agent, int invocations, int loads, int sessions,
        string population = "direct", IReadOnlyList<SkillsUsageWindow>? windows = null, IReadOnlyList<SkillsUsageDay>? daily = null,
        int undated = 0, IReadOnlyList<SkillsUsageEvidence>? evidence = null, IReadOnlyList<string>? sessionRefs = null) =>
        new(name, agent, invocations, loads, sessions, null, sessionRefs ?? Array.Empty<string>(),
            windows ?? Array.Empty<SkillsUsageWindow>(), daily ?? Array.Empty<SkillsUsageDay>(), undated, population,
            evidence ?? Array.Empty<SkillsUsageEvidence>(), Array.Empty<string>());

    public static SkillsUsage Usage(bool enabled = false, string? lastAnalysisAt = null, string? earliestEvidence = null,
        bool partial = true, IReadOnlyList<SkillsUsageRow>? skills = null, SkillsUsageCoverage[]? coverage = null) =>
        new(enabled, lastAnalysisAt, earliestEvidence, 0, 0, 0, null, null, null, null, partial,
            skills ?? Array.Empty<SkillsUsageRow>(), coverage ?? Array.Empty<SkillsUsageCoverage>(), null);

    public static SkillsUsageCoverage Coverage(string agent, string status) => new(agent, status);

    public static SkillsUsageWindow Window(int days, int invocations, int loads, int sessions) => new(days, invocations, loads, sessions);
}
